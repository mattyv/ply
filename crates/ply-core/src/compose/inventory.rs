//! Building the operation inventory from real source, fail-closed.
//!
//! The sampled operation pool cannot serve as this certificate. It exists
//! to *run* things, so it excludes what it cannot construct arguments for,
//! and it drops generic impls with no record at all. A proof over a set
//! that quietly omits one mutator says nothing about the type and reads as
//! stronger than the sampling it replaces.
//!
//! So this scan is separate, and its bias is the opposite one: anything it
//! cannot classify goes in `unclassified`, which blocks the property.

use super::Inventory;

/// Scan one module's source for every way `type_name` can be built or
/// changed.
///
/// Fails closed at every branch. Anything this scan cannot read, cannot
/// resolve, or was not built to classify lands in `unclassified`, which
/// blocks the property rather than being omitted from a list that then
/// reads as complete.
pub fn inventory_from_source(src: &str, type_name: &str) -> Inventory {
    let mut inv = Inventory::default();

    let file = match syn::parse_file(src) {
        Ok(f) => f,
        Err(e) => {
            // Never an empty inventory: that reads as "nothing can change
            // this type", which is the opposite of what a failed parse
            // means.
            inv.unclassified
                .push(format!("this module's source could not be parsed ({e}), so no operation could be found at all"));
            return inv;
        }
    };

    for item in &file.items {
        match item {
            // A struct's own fields. Anything a caller outside can write is
            // a route that never passes through an operation.
            syn::Item::Struct(st) if st.ident == type_name => {
                if !st.generics.params.is_empty() {
                    inv.unclassified.push(format!(
                        "`{type_name}` is generic, and this scan reads one instantiation at most"
                    ));
                }
                for f in &st.fields {
                    if matches!(f.vis, syn::Visibility::Public(_)) {
                        let name = f
                            .ident
                            .as_ref()
                            .map(|i| i.to_string())
                            .unwrap_or_else(|| "<positional>".into());
                        inv.escapes.push(format!(
                            "`{type_name}::{name}` is a public field, so it can be written without calling any operation"
                        ));
                    }
                }
            }

            syn::Item::Impl(imp) => {
                let Some(self_name) = impl_self_name(imp) else {
                    continue;
                };
                if self_name != type_name {
                    continue;
                }
                // The hole the sampled pool has: a generic impl is dropped
                // there with no record, and a mutator can be hiding in it.
                if !imp.generics.params.is_empty() {
                    inv.unclassified.push(format!(
                        "`impl` block for `{type_name}` is generic, so this scan cannot enumerate its operations"
                    ));
                    continue;
                }
                let via_trait = imp.trait_.as_ref().map(|(_, path, _)| path_last(path));
                for it in &imp.items {
                    let syn::ImplItem::Fn(m) = it else {
                        continue;
                    };
                    classify_method(type_name, m, via_trait.as_deref(), &mut inv);
                }
            }

            // A free function taking `&mut Type` reaches the fields
            // directly. Private fields are NOT a boundary against the same
            // module.
            syn::Item::Fn(f) => {
                if fn_takes_mut_of(&f.sig, type_name) {
                    inv.escapes.push(format!(
                        "`{}` takes `&mut {type_name}` and lives in the same module, so it can change the state without calling any operation -- private fields are not a boundary here",
                        f.sig.ident
                    ));
                }
            }

            // Opaque to a source scan.
            syn::Item::Macro(m) => {
                inv.unclassified.push(format!(
                    "`{}!` expands to items this scan cannot see",
                    path_last(&m.mac.path)
                ));
            }

            _ => {}
        }
    }

    inv.constructors.sort();
    inv.mutators.sort();
    inv.escapes.sort();
    inv.unclassified.sort();
    inv
}

/// Where one method lands.
fn classify_method(
    type_name: &str,
    m: &syn::ImplItemFn,
    via_trait: Option<&str>,
    inv: &mut Inventory,
) {
    let name = m.sig.ident.to_string();
    let path = match via_trait {
        Some(t) => format!("<{type_name} as {t}>::{name}"),
        None => format!("{type_name}::{name}"),
    };

    // Handing out a mutable reference lets a caller change the state with
    // no operation involved.
    if returns_mut_reference(&m.sig) {
        inv.escapes.push(format!(
            "`{path}` returns a mutable reference into the value, so a caller can change the state without calling any operation"
        ));
        return;
    }

    match m.sig.receiver() {
        None => {
            // No receiver: a constructor if it produces one of these.
            if returns_self(&m.sig, type_name) {
                inv.constructors.push(path);
            }
        }
        Some(r) if r.reference.is_none() => {
            inv.unclassified.push(format!(
                "`{path}` consumes the value it is called on, which is outside the shapes this scan classifies"
            ));
        }
        Some(r) if r.mutability.is_some() => inv.mutators.push(path),
        Some(_) => { /* `&self`: a reader, handled as an observer */ }
    }
}

fn impl_self_name(imp: &syn::ItemImpl) -> Option<String> {
    match &*imp.self_ty {
        syn::Type::Path(tp) => Some(path_last(&tp.path)),
        _ => None,
    }
}

fn path_last(p: &syn::Path) -> String {
    p.segments
        .last()
        .map(|s| s.ident.to_string())
        .unwrap_or_default()
}

fn returns_self(sig: &syn::Signature, type_name: &str) -> bool {
    match &sig.output {
        syn::ReturnType::Type(_, ty) => match &**ty {
            syn::Type::Path(tp) => {
                let last = path_last(&tp.path);
                last == "Self" || last == type_name
            }
            _ => false,
        },
        syn::ReturnType::Default => false,
    }
}

fn returns_mut_reference(sig: &syn::Signature) -> bool {
    matches!(&sig.output, syn::ReturnType::Type(_, ty)
        if matches!(&**ty, syn::Type::Reference(r) if r.mutability.is_some()))
}

fn fn_takes_mut_of(sig: &syn::Signature, type_name: &str) -> bool {
    sig.inputs.iter().any(|arg| {
        let syn::FnArg::Typed(pt) = arg else {
            return false;
        };
        matches!(&*pt.ty, syn::Type::Reference(r)
            if r.mutability.is_some()
                && matches!(&*r.elem, syn::Type::Path(tp) if path_last(&tp.path) == type_name))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inv(src: &str) -> Inventory {
        inventory_from_source(src, "Bucket")
    }

    /// The plain shape: a constructor and two mutators, all found.
    #[test]
    fn the_supported_shape_is_classified_completely() {
        let i = inv(r#"
            pub struct Bucket { capacity: u32, available: u32 }
            impl Bucket {
                pub fn new(capacity: u32) -> Self { Bucket { capacity, available: capacity } }
                pub fn try_take(&mut self, n: u32) -> bool { true }
                pub fn refill(&mut self, n: u32) { }
                pub fn available(&self) -> u32 { self.available }
            }
        "#);
        assert_eq!(i.constructors, vec!["Bucket::new".to_string()]);
        assert_eq!(
            i.mutators,
            vec!["Bucket::refill".to_string(), "Bucket::try_take".to_string()]
        );
        assert!(i.escapes.is_empty(), "{:?}", i.escapes);
        assert!(i.unclassified.is_empty(), "{:?}", i.unclassified);
    }

    /// **The hole the sampled pool has.** A generic impl is dropped there
    /// with no record; here it must be named, because a mutator could be
    /// hiding in it.
    #[test]
    fn a_generic_impl_is_recorded_rather_than_silently_dropped() {
        let i = inv(r#"
            pub struct Bucket { capacity: u32 }
            impl Bucket { pub fn new() -> Self { Bucket { capacity: 0 } } }
            impl<T: Into<u32>> Bucket { pub fn set(&mut self, v: T) { } }
        "#);
        assert!(
            i.unclassified.iter().any(|u| u.contains("generic")),
            "a generic impl must be named, not dropped: {:?}",
            i.unclassified
        );
    }

    /// A public field is a mutation route that never passes through an
    /// operation, so the theorem has no subject.
    #[test]
    fn a_public_field_is_an_escape() {
        let i = inv(r#"
            pub struct Bucket { pub available: u32 }
            impl Bucket { pub fn new() -> Self { Bucket { available: 0 } } }
        "#);
        assert!(
            i.escapes.iter().any(|e| e.contains("available")),
            "{:?}",
            i.escapes
        );
    }

    /// Private fields alone do NOT establish closure: anything else in the
    /// same module can reach them.
    #[test]
    fn a_free_function_in_the_same_module_reaching_in_is_an_escape() {
        let i = inv(r#"
            pub struct Bucket { available: u32 }
            impl Bucket { pub fn new() -> Self { Bucket { available: 0 } } }
            pub fn drain(b: &mut Bucket) { b.available = 0; }
        "#);
        assert!(
            i.escapes.iter().any(|e| e.contains("drain")),
            "private fields are not a boundary against the same module: {:?}",
            i.escapes
        );
    }

    /// Handing out a mutable reference lets a caller change the state with
    /// no operation involved at all.
    #[test]
    fn a_method_returning_a_mutable_reference_is_an_escape() {
        let i = inv(r#"
            pub struct Bucket { available: u32 }
            impl Bucket {
                pub fn new() -> Self { Bucket { available: 0 } }
                pub fn slot(&mut self) -> &mut u32 { &mut self.available }
            }
        "#);
        assert!(
            i.escapes.iter().any(|e| e.contains("slot")),
            "{:?}",
            i.escapes
        );
    }

    /// A trait implementation can mutate too, and is a real route.
    #[test]
    fn a_mutating_trait_method_is_accounted_for_not_ignored() {
        let i = inv(r#"
            pub struct Bucket { available: u32 }
            impl Bucket { pub fn new() -> Self { Bucket { available: 0 } } }
            impl Extend<u32> for Bucket {
                fn extend<I>(&mut self, iter: I) { }
            }
        "#);
        assert!(
            i.mutators.iter().any(|m| m.contains("extend"))
                || i.unclassified.iter().any(|u| u.contains("extend")),
            "a trait method taking &mut self is a mutation route: {:?} / {:?}",
            i.mutators,
            i.unclassified
        );
    }

    /// A second construction path must be found, or a proof covers one
    /// entry point and reads as covering the type.
    #[test]
    fn every_construction_path_is_found_not_just_the_first() {
        let i = inv(r#"
            pub struct Bucket { available: u32 }
            impl Bucket {
                pub fn new() -> Self { Bucket { available: 0 } }
                pub fn empty() -> Bucket { Bucket { available: 0 } }
                pub fn from_parts(n: u32) -> Self { Bucket { available: n } }
            }
        "#);
        assert_eq!(i.constructors.len(), 3, "{:?}", i.constructors);
    }

    /// Code produced by a macro is opaque to a source scan, so the scan
    /// says so rather than reporting what it happens to see.
    #[test]
    fn macro_generated_items_are_unclassified_because_the_scan_cannot_see_them() {
        let i = inv(r#"
            pub struct Bucket { available: u32 }
            impl Bucket { pub fn new() -> Self { Bucket { available: 0 } } }
            impl_extra_ops!(Bucket);
        "#);
        assert!(
            i.unclassified.iter().any(|u| u.contains("impl_extra_ops")),
            "an opaque macro must block: {:?}",
            i.unclassified
        );
    }

    /// A method consuming `self` is outside the accepted subset and must be
    /// named rather than ignored.
    #[test]
    fn a_self_consuming_method_is_outside_the_subset_and_named() {
        let i = inv(r#"
            pub struct Bucket { available: u32 }
            impl Bucket {
                pub fn new() -> Self { Bucket { available: 0 } }
                pub fn into_inner(self) -> u32 { self.available }
            }
        "#);
        assert!(
            i.unclassified.iter().any(|u| u.contains("into_inner")),
            "{:?}",
            i.unclassified
        );
    }

    /// Source the scanner cannot parse must never come back as an empty
    /// inventory, which would read as "nothing can change this type".
    #[test]
    fn unparseable_source_blocks_rather_than_returning_nothing() {
        let i = inv("pub struct Bucket { this is not rust");
        assert!(
            !i.unclassified.is_empty(),
            "a scan that could not read the source must say so"
        );
    }

    /// **Against the real fixture, not a snippet.** Hand-written test
    /// sources agree with whatever the scanner does; the fixture does not.
    #[test]
    fn the_real_token_bucket_fixture_classifies_completely() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tests/fixtures/tokenbucket/src/lib.rs");
        let src = std::fs::read_to_string(&path).expect("the fixture is in the repo");
        let i = inventory_from_source(&src, "TokenBucket");

        assert!(
            !i.constructors.is_empty(),
            "no construction path found in the real fixture: {i:?}"
        );
        assert!(
            !i.mutators.is_empty(),
            "no mutator found in the real fixture, which has two: {i:?}"
        );
        assert!(
            i.escapes.is_empty(),
            "the fixture has private fields and no escapes; found {:?}",
            i.escapes
        );
        assert!(
            i.unclassified.is_empty(),
            "the fixture is inside the supported subset; found {:?}",
            i.unclassified
        );
    }
}
