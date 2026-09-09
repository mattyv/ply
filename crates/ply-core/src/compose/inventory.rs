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
//!
//! **It was not, in fact, fail-closed until 2026-09-09.** Review handed it
//! sixteen small modules that compile and that build or write the value,
//! and fifteen came back as a complete inventory with nothing blocked --
//! an impl in an inline module or a function body, a field the rest of the
//! crate can write, a field writable through a shared reference, a free
//! function that constructs one, an alias, a raw pointer, a mutable
//! global. The cause was structural: the walk recognised four kinds of item
//! and ignored the rest, and ignoring is the one thing this scan must never
//! do. The default is now `unclassified`, and only items that provably
//! cannot reach the value are passed over in silence.
//!
//! Two rules carry most of the weight, and both over-report on purpose --
//! over-reporting blocks, which is the safe direction:
//!
//! - Anything in this module that writes a field by one of the type's own
//!   field names is an escape, wherever it lives. Private fields are not a
//!   boundary inside the module that declares them, and a function need
//!   not mention the type to reach one (`GLOBAL.available = 0`).
//! - A field whose type can be written through a shared reference makes
//!   `&self` no boundary either, so the type's readers stop being readers.

use std::collections::BTreeSet;

use super::Inventory;

/// Types whose contents can be changed through a shared reference, so a
/// `&self` method is no guarantee the value did not move.
const INTERIOR_MUTABILITY: &[&str] = &[
    "Cell",
    "RefCell",
    "UnsafeCell",
    "OnceCell",
    "Mutex",
    "RwLock",
    "AtomicBool",
    "AtomicI8",
    "AtomicI16",
    "AtomicI32",
    "AtomicI64",
    "AtomicIsize",
    "AtomicU8",
    "AtomicU16",
    "AtomicU32",
    "AtomicU64",
    "AtomicUsize",
    "AtomicPtr",
];

/// Scan one module's source for every way `type_name` can be built or
/// changed.
///
/// Fails closed: anything this scan cannot read, cannot resolve, or was not
/// built to classify lands in `unclassified`, which blocks the property
/// rather than being omitted from a list that then reads as complete.
pub fn inventory_from_source(src: &str, type_name: &str) -> Inventory {
    let mut inv = Inventory::default();

    let file = match syn::parse_file(src) {
        Ok(f) => f,
        Err(e) => {
            // Never an empty inventory: that reads as "nothing can change
            // this type", which is the opposite of what a failed parse
            // means.
            inv.unclassified.push(format!(
                "this module's source could not be parsed ({e}), so no operation could be \
                 found at all"
            ));
            return inv;
        }
    };

    // The type's own field names, needed before the walk: a write to any of
    // them from anywhere in this module is a route around the operations.
    let fields = declared_fields(&file.items, type_name);
    let mut scan = Scan {
        type_name,
        fields,
        inv: &mut inv,
    };
    scan.items(&file.items);

    inv.constructors.sort();
    inv.constructors.dedup();
    inv.mutators.sort();
    inv.mutators.dedup();
    inv.escapes.sort();
    inv.escapes.dedup();
    inv.unclassified.sort();
    inv.unclassified.dedup();
    inv
}

struct Scan<'a> {
    type_name: &'a str,
    fields: BTreeSet<String>,
    inv: &'a mut Inventory,
}

impl Scan<'_> {
    fn items(&mut self, items: &[syn::Item]) {
        for item in items {
            self.item(item);
        }
    }

    fn item(&mut self, item: &syn::Item) {
        let type_name = self.type_name;
        match item {
            syn::Item::Struct(st) => {
                if st.ident == type_name {
                    self.own_struct(st);
                } else {
                    self.other_type_holding_it(&st.ident.to_string(), st.fields.iter());
                }
            }

            // Every variant of a visible enum can be written by name, so
            // there is no operation to route a change through.
            syn::Item::Enum(en) => {
                if en.ident == type_name {
                    if !en.generics.params.is_empty() {
                        self.unclassified(format!(
                            "`{type_name}` is generic, and this scan reads one instantiation \
                             at most"
                        ));
                    }
                    self.escape(format!(
                        "`{type_name}` is an enum, so any code that can see it can write a \
                         whole new value by naming a variant, without calling any operation"
                    ));
                } else {
                    for v in &en.variants {
                        self.other_type_holding_it(&en.ident.to_string(), v.fields.iter());
                    }
                }
            }

            syn::Item::Union(u) if u.ident == type_name => self.unclassified(format!(
                "`{type_name}` is a union, whose fields overlap in memory -- outside the \
                 shapes this scan classifies"
            )),

            syn::Item::Impl(imp) => self.impl_block(imp),

            syn::Item::Fn(f) => {
                self.free_fn(&f.sig);
                self.body_writes_a_field(&f.sig.ident.to_string(), &f.block);
                // An item written inside a function body is still an item.
                for st in &f.block.stmts {
                    if let syn::Stmt::Item(inner) = st {
                        self.item(inner);
                    }
                }
            }

            // A module written out here is part of the same source, and
            // privacy is no boundary between them. One that is not is a
            // file this scan has not read.
            syn::Item::Mod(m) => match &m.content {
                Some((_, items)) => self.items(items),
                None => self.unclassified(format!(
                    "`mod {};` is a separate file this scan has not read, and an operation on \
                     `{type_name}` can live in it",
                    m.ident
                )),
            },

            // Another name for the type; operations may be written against
            // it, and this scan matches on names.
            syn::Item::Type(t) if mentions_type(&t.ty, type_name) => self.unclassified(format!(
                "`type {} = ..` is another name for `{type_name}`, and this scan matches names \
                 rather than resolving them, so operations written against it are invisible here",
                t.ident
            )),

            syn::Item::Static(s) if mentions_type(&s.ty, type_name) => {
                if matches!(s.mutability, syn::StaticMutability::Mut(_)) {
                    self.escape(format!(
                        "`static mut {}` holds a `{type_name}` that any code in this module can \
                         write, without calling any operation",
                        s.ident
                    ));
                }
            }

            // Opaque to a source scan.
            syn::Item::Macro(m) => self.unclassified(format!(
                "`{}!` expands to items this scan cannot see",
                path_last(&m.mac.path)
            )),

            // Cannot reach a value: an import, a name, an immutable
            // constant, a trait's declaration.
            syn::Item::Use(_)
            | syn::Item::ExternCrate(_)
            | syn::Item::Const(_)
            | syn::Item::Trait(_)
            | syn::Item::TraitAlias(_)
            | syn::Item::Static(_)
            | syn::Item::Type(_)
            | syn::Item::Union(_) => {}

            // **The default is to block.** Ignoring what it did not
            // recognise is how this scan reported fifteen mutation routes as
            // a clean inventory.
            other => self.unclassified(format!(
                "this module contains a `{}` that this scan was not built to classify, so the \
                 list of ways to change `{type_name}` is not known to be complete",
                item_kind(other)
            )),
        }
    }

    fn own_struct(&mut self, st: &syn::ItemStruct) {
        let type_name = self.type_name;
        if !st.generics.params.is_empty() {
            self.unclassified(format!(
                "`{type_name}` is generic, and this scan reads one instantiation at most"
            ));
        }
        for (i, f) in st.fields.iter().enumerate() {
            let name = f
                .ident
                .as_ref()
                .map(|i| i.to_string())
                .unwrap_or_else(|| i.to_string());
            // Anything but fully private is writable from outside the type.
            // `pub(crate)` and `pub(super)` are not a boundary either, and
            // reading only `pub` was one of the fifteen.
            if !matches!(f.vis, syn::Visibility::Inherited) {
                self.escape(format!(
                    "`{type_name}::{name}` is visible outside the type, so it can be written \
                     without calling any operation"
                ));
            }
            if let Some(container) = interior_mutability(&f.ty) {
                self.escape(format!(
                    "`{type_name}::{name}` is a `{container}`, whose contents can be changed \
                     through a shared reference -- so a `&self` method is not a promise the \
                     value did not move, and the readers this property rests on stop being \
                     readers"
                ));
            }
        }
    }

    /// A different type with a field holding ours. Anyone who can reach
    /// that field can reach the value inside it.
    fn other_type_holding_it<'f>(
        &mut self,
        owner: &str,
        fields: impl Iterator<Item = &'f syn::Field>,
    ) {
        let type_name = self.type_name;
        for (i, f) in fields.enumerate() {
            if !mentions_type(&f.ty, type_name) {
                continue;
            }
            if matches!(f.vis, syn::Visibility::Inherited) {
                continue;
            }
            let name = f
                .ident
                .as_ref()
                .map(|i| i.to_string())
                .unwrap_or_else(|| i.to_string());
            self.escape(format!(
                "`{owner}::{name}` holds a `{type_name}` and is visible outside `{owner}`, so \
                 a caller can reach the value and change it without calling any operation"
            ));
        }
    }

    fn impl_block(&mut self, imp: &syn::ItemImpl) {
        let type_name = self.type_name;
        let is_ours = matches!(&*imp.self_ty,
            syn::Type::Path(tp) if path_last(&tp.path) == type_name);

        if !is_ours {
            if mentions_type(&imp.self_ty, type_name) {
                // `impl .. for &mut Bucket`, `impl .. for Vec<Bucket>`: the
                // methods act on the value and this scan cannot say how.
                self.unclassified(format!(
                    "an `impl` block is written for a type built out of `{type_name}` rather \
                     than for `{type_name}` itself, and its methods can change the value"
                ));
            }
            // Another type's methods can still take ours by mutable
            // reference, and private fields are no boundary in one module.
            for it in &imp.items {
                if let syn::ImplItem::Fn(m) = it {
                    self.free_fn(&m.sig);
                    self.body_writes_a_field(&m.sig.ident.to_string(), &m.block);
                }
            }
            return;
        }

        if !imp.generics.params.is_empty() {
            // The hole the sampled pool has: a generic impl is dropped
            // there with no record, and a mutator can be hiding in it.
            self.unclassified(format!(
                "`impl` block for `{type_name}` is generic, so this scan cannot enumerate its \
                 operations"
            ));
            return;
        }
        let via_trait = imp.trait_.as_ref().map(|(_, path, _)| path_last(path));
        for it in &imp.items {
            let syn::ImplItem::Fn(m) = it else {
                continue;
            };
            self.method(m, via_trait.as_deref());
        }
    }

    /// Where one of the type's own methods lands.
    fn method(&mut self, m: &syn::ImplItemFn, via_trait: Option<&str>) {
        let type_name = self.type_name;
        let name = m.sig.ident.to_string();
        let path = match via_trait {
            Some(t) => format!("<{type_name} as {t}>::{name}"),
            None => format!("{type_name}::{name}"),
        };

        // Handing out a mutable reference lets a caller change the state
        // with no operation involved -- at any depth, not only at the top:
        // `Option<&mut u32>` was being classified as an ordinary mutator.
        if returns_something_mutable(&m.sig) {
            self.escape(format!(
                "`{path}` hands out a mutable reference into the value, so a caller can change \
                 the state afterwards without calling any operation"
            ));
            return;
        }

        match m.sig.receiver() {
            None => {
                if returns_self(&m.sig, type_name) {
                    self.inv.constructors.push(path);
                } else {
                    // An associated function that is not a constructor can
                    // still reach a value it is handed.
                    self.free_fn(&m.sig);
                    self.body_writes_a_field(&name, &m.block);
                }
            }
            // A receiver written out in full -- `self: &mut Self` -- sets
            // no shorthand flags, so it read as consuming the value and was
            // blocked with a reason that was simply untrue. Its type says
            // what it is.
            Some(r) if r.reference.is_none() => match &*r.ty {
                syn::Type::Reference(tr) if tr.mutability.is_some() => self.inv.mutators.push(path),
                syn::Type::Reference(_) => {}
                _ => self.unclassified(format!(
                    "`{path}` consumes the value it is called on, which is outside the shapes \
                     this scan classifies"
                )),
            },
            Some(r) if r.mutability.is_some() => self.inv.mutators.push(path),
            // `&self`: a reader. Whether that is a boundary depends on the
            // fields, which `own_struct` has already ruled on.
            Some(_) => {}
        }
    }

    /// A function that is not one of the type's operations, judged by what
    /// its signature lets it do to the value.
    fn free_fn(&mut self, sig: &syn::Signature) {
        let type_name = self.type_name;
        let name = sig.ident.to_string();
        if returns_self(sig, type_name) {
            // A construction path that is not a method is still a
            // construction path, and the theorem starts at one.
            self.inv.constructors.push(name.clone());
        }
        for arg in &sig.inputs {
            let syn::FnArg::Typed(pt) = arg else {
                continue;
            };
            if !mentions_type(&pt.ty, type_name) {
                continue;
            }
            match reach(&pt.ty, type_name) {
                Reach::SharedOnly => {}
                Reach::Mutable(how) => self.escape(format!(
                    "`{name}` takes `{type_name}` {how} and lives in this module, so it can \
                     change the state without calling any operation -- private fields are not \
                     a boundary here"
                )),
                Reach::Unknown => self.unclassified(format!(
                    "`{name}` takes `{type_name}` in a shape this scan cannot judge, so \
                     whether it can change the state is unknown"
                )),
            }
        }
    }

    /// A write to one of the type's own field names, anywhere in this
    /// module. A function need not mention the type to reach one.
    fn body_writes_a_field(&mut self, owner: &str, block: &syn::Block) {
        let type_name = self.type_name;
        let mut found = BTreeSet::new();
        collect_field_writes(block, &self.fields, &mut found);
        for f in found {
            self.escape(format!(
                "`{owner}` writes the field `{f}` that `{type_name}` declares, so it changes \
                 the state without calling any operation -- private fields are not a boundary \
                 inside the module that declares them"
            ));
        }
    }

    fn escape(&mut self, s: String) {
        self.inv.escapes.push(s);
    }

    fn unclassified(&mut self, s: String) {
        self.inv.unclassified.push(s);
    }
}

/// What a signature lets a function do to a value of the type.
enum Reach {
    /// It can read it and nothing more.
    SharedOnly,
    Mutable(&'static str),
    Unknown,
}

fn reach(ty: &syn::Type, type_name: &str) -> Reach {
    match ty {
        syn::Type::Reference(r) => {
            let inner_is_it = matches!(&*r.elem,
                syn::Type::Path(tp) if path_last(&tp.path) == type_name);
            match (r.mutability.is_some(), inner_is_it) {
                (true, true) => Reach::Mutable("by mutable reference"),
                (false, true) => Reach::SharedOnly,
                (true, _) => Reach::Mutable("inside a mutable reference"),
                (false, _) => Reach::Unknown,
            }
        }
        syn::Type::Ptr(p) if p.mutability.is_some() => Reach::Mutable("as a raw pointer"),
        syn::Type::Path(tp) if path_last(&tp.path) == type_name => Reach::Mutable("by value"),
        _ => Reach::Unknown,
    }
}

/// Every field name the type declares, whichever shape it is declared in.
fn declared_fields(items: &[syn::Item], type_name: &str) -> BTreeSet<String> {
    fn add(out: &mut BTreeSet<String>, fields: &syn::Fields) {
        for f in fields {
            if let Some(i) = &f.ident {
                out.insert(i.to_string());
            }
        }
    }
    let mut out = BTreeSet::new();
    for item in items {
        match item {
            syn::Item::Struct(st) if st.ident == type_name => add(&mut out, &st.fields),
            syn::Item::Enum(en) if en.ident == type_name => {
                for v in &en.variants {
                    add(&mut out, &v.fields);
                }
            }
            syn::Item::Mod(m) => {
                if let Some((_, inner)) = &m.content {
                    out.extend(declared_fields(inner, type_name));
                }
            }
            _ => {}
        }
    }
    out
}

/// Every one of `names` that this block assigns to as a field.
fn collect_field_writes(block: &syn::Block, names: &BTreeSet<String>, out: &mut BTreeSet<String>) {
    use syn::visit::Visit;
    struct V<'a> {
        names: &'a BTreeSet<String>,
        out: &'a mut BTreeSet<String>,
    }
    impl V<'_> {
        fn note(&mut self, target: &syn::Expr) {
            if let syn::Expr::Field(syn::ExprField {
                member: syn::Member::Named(name),
                ..
            }) = target
            {
                let name = name.to_string();
                if self.names.contains(&name) {
                    self.out.insert(name);
                }
            }
        }
    }
    impl<'ast> Visit<'ast> for V<'_> {
        fn visit_expr_assign(&mut self, e: &'ast syn::ExprAssign) {
            self.note(&e.left);
            syn::visit::visit_expr_assign(self, e);
        }
        /// `b.available -= n` is a write and is not an assignment node.
        /// Reading only assignment nodes missed every compound operator.
        fn visit_expr_binary(&mut self, e: &'ast syn::ExprBinary) {
            use syn::BinOp::*;
            if matches!(
                e.op,
                AddAssign(_)
                    | SubAssign(_)
                    | MulAssign(_)
                    | DivAssign(_)
                    | RemAssign(_)
                    | BitXorAssign(_)
                    | BitAndAssign(_)
                    | BitOrAssign(_)
                    | ShlAssign(_)
                    | ShrAssign(_)
            ) {
                self.note(&e.left);
            }
            syn::visit::visit_expr_binary(self, e);
        }
    }
    V { names, out }.visit_block(block);
}

/// Whether this type mentions `type_name` anywhere inside it, however
/// deeply -- `&mut [T]`, `Option<&mut T>`, `*mut T`.
fn mentions_type(ty: &syn::Type, type_name: &str) -> bool {
    struct V<'a> {
        name: &'a str,
        hit: bool,
    }
    impl<'ast> syn::visit::Visit<'ast> for V<'_> {
        fn visit_path(&mut self, p: &'ast syn::Path) {
            if path_last(p) == self.name {
                self.hit = true;
            }
            syn::visit::visit_path(self, p);
        }
    }
    let mut v = V {
        name: type_name,
        hit: false,
    };
    syn::visit::Visit::visit_type(&mut v, ty);
    v.hit
}

/// The interior-mutability container this field's type is built from, if
/// any.
fn interior_mutability(ty: &syn::Type) -> Option<&'static str> {
    struct V {
        hit: Option<&'static str>,
    }
    impl<'ast> syn::visit::Visit<'ast> for V {
        fn visit_path(&mut self, p: &'ast syn::Path) {
            let last = path_last(p);
            if let Some(c) = INTERIOR_MUTABILITY.iter().find(|c| **c == last) {
                self.hit = Some(c);
            }
            syn::visit::visit_path(self, p);
        }
    }
    let mut v = V { hit: None };
    syn::visit::Visit::visit_type(&mut v, ty);
    v.hit
}

/// Whether the return type hands out a mutable reference at any depth.
fn returns_something_mutable(sig: &syn::Signature) -> bool {
    let syn::ReturnType::Type(_, ty) = &sig.output else {
        return false;
    };
    struct V {
        hit: bool,
    }
    impl<'ast> syn::visit::Visit<'ast> for V {
        fn visit_type_reference(&mut self, r: &'ast syn::TypeReference) {
            if r.mutability.is_some() {
                self.hit = true;
            }
            syn::visit::visit_type_reference(self, r);
        }
    }
    let mut v = V { hit: false };
    syn::visit::Visit::visit_type(&mut v, ty);
    v.hit
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

/// A name for the kind of item, for a message a reader can act on.
fn item_kind(item: &syn::Item) -> &'static str {
    match item {
        syn::Item::ForeignMod(_) => "extern block",
        syn::Item::Verbatim(_) => "piece of source it could not parse",
        _ => "item",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inv(src: &str) -> Inventory {
        inventory_from_source(src, "Bucket")
    }

    /// **The invariant this scan exists to satisfy, and did not.** Review on
    /// 2026-09-09 found fourteen of these sixteen coming back as a clean,
    /// complete inventory: nothing to build on, nothing to change it
    /// through, nothing unclassified. Every one is real Rust that compiles
    /// and that constructs or writes the value.
    ///
    /// A route the scan cannot classify has to *block*. It does not have to
    /// be classified correctly -- naming it in `unclassified` is the honest
    /// answer and the one that stops a property being claimed.
    #[test]
    fn no_route_that_builds_or_changes_the_value_comes_back_as_a_clean_inventory() {
        // Each: a name for the report, the module, and what accounting for
        // it looks like -- either the operation appears under that name, or
        // the route is blocked. Classifying beats blocking where the scan
        // can manage it, so the two are distinguished rather than lumped.
        const BLOCKED: &str = "";
        let routes: &[(&str, &str, &str)] = &[
            (
                "a field the rest of the crate can write",
                r#"pub struct Bucket { pub(crate) available: u32 }
                   impl Bucket { pub fn new() -> Self { Bucket { available: 0 } } }"#,
                BLOCKED,
            ),
            (
                "an impl in a module written inline here",
                r#"pub struct Bucket { available: u32 }
                   impl Bucket { pub fn new() -> Self { Bucket { available: 0 } } }
                   mod extra {
                       impl super::Bucket { pub fn drain(&mut self) { self.available = 0; } }
                   }"#,
                "Bucket::drain",
            ),
            (
                "a method on another type in the same module taking it by mutable reference",
                r#"pub struct Bucket { available: u32 }
                   impl Bucket { pub fn new() -> Self { Bucket { available: 0 } } }
                   pub struct Pool;
                   impl Pool { pub fn drain(&self, b: &mut Bucket) { b.available = 0; } }"#,
                BLOCKED,
            ),
            (
                "a field that can be written through a shared reference",
                r#"use std::cell::Cell;
                   pub struct Bucket { available: Cell<u32> }
                   impl Bucket {
                       pub fn new() -> Self { Bucket { available: Cell::new(0) } }
                       pub fn drain(&self) { self.available.set(0); }
                   }"#,
                BLOCKED,
            ),
            (
                "a free function that builds one",
                r#"pub struct Bucket { available: u32 }
                   impl Bucket { pub fn new() -> Self { Bucket { available: 0 } } }
                   pub fn make(n: u32) -> Bucket { Bucket { available: n } }"#,
                "make",
            ),
            (
                "an enum whose variant fields anyone can set",
                r#"pub enum Bucket { Full { available: u32 }, Empty }
                   impl Bucket { pub fn new() -> Self { Bucket::Empty } }"#,
                BLOCKED,
            ),
            (
                "a method handing out a mutable reference inside another type",
                r#"pub struct Bucket { available: u32 }
                   impl Bucket {
                       pub fn new() -> Self { Bucket { available: 0 } }
                       pub fn slot(&mut self) -> Option<&mut u32> { Some(&mut self.available) }
                   }"#,
                BLOCKED,
            ),
            (
                "a function taking a mutable slice of them",
                r#"pub struct Bucket { available: u32 }
                   impl Bucket { pub fn new() -> Self { Bucket { available: 0 } } }
                   pub fn drain_all(bs: &mut [Bucket]) { for b in bs { b.available = 0; } }"#,
                BLOCKED,
            ),
            (
                "a name for the type that the scan does not follow",
                r#"pub struct Bucket { available: u32 }
                   impl Bucket { pub fn new() -> Self { Bucket { available: 0 } } }
                   pub type B = Bucket;
                   pub fn drain(b: &mut B) { b.available = 0; }"#,
                BLOCKED,
            ),
            (
                "an impl written inside a function body",
                r#"pub struct Bucket { available: u32 }
                   impl Bucket { pub fn new() -> Self { Bucket { available: 0 } } }
                   pub fn helper() {
                       impl Bucket { pub fn drain(&mut self) { self.available = 0; } }
                   }"#,
                "Bucket::drain",
            ),
            (
                "another type holding one in a field anyone can reach",
                r#"pub struct Bucket { available: u32 }
                   impl Bucket { pub fn new() -> Self { Bucket { available: 0 } } }
                   pub struct Wrapper(pub Bucket);
                   impl Wrapper { pub fn wreck(&mut self) { self.0.available = 0; } }"#,
                BLOCKED,
            ),
            (
                "a function that takes one by value, changes it, and gives it back",
                r#"pub struct Bucket { available: u32 }
                   impl Bucket { pub fn new() -> Self { Bucket { available: 0 } } }
                   pub fn drained(mut b: Bucket) -> Bucket { b.available = 0; b }"#,
                BLOCKED,
            ),
            (
                "a receiver written out in full rather than as `&mut self`",
                r#"pub struct Bucket { available: u32 }
                   impl Bucket {
                       pub fn new() -> Self { Bucket { available: 0 } }
                       pub fn drain(self: &mut Self) { self.available = 0; }
                   }"#,
                "Bucket::drain",
            ),
            (
                "a raw pointer to one",
                r#"pub struct Bucket { available: u32 }
                   impl Bucket { pub fn new() -> Self { Bucket { available: 0 } } }
                   pub unsafe fn drain(b: *mut Bucket) { (*b).available = 0; }"#,
                BLOCKED,
            ),
            (
                "one held in a mutable global, reached by a function that never names it",
                r#"pub struct Bucket { available: u32 }
                   impl Bucket { pub fn new() -> Self { Bucket { available: 0 } } }
                   pub static mut GLOBAL: Bucket = Bucket { available: 0 };
                   pub unsafe fn drain_global() { GLOBAL.available = 0; }"#,
                BLOCKED,
            ),
            (
                "a function that takes one apart with a compound assignment",
                r#"pub struct Bucket { available: u32 }
                   impl Bucket { pub fn new() -> Self { Bucket { available: 0 } } }
                   pub fn spend(b: &mut Bucket, n: u32) { b.available -= n; }"#,
                BLOCKED,
            ),
            (
                "a trait implemented for a mutable reference to one",
                r#"pub struct Bucket { available: u32 }
                   impl Bucket { pub fn new() -> Self { Bucket { available: 0 } } }
                   impl Extend<u32> for &mut Bucket {
                       fn extend<T: IntoIterator<Item = u32>>(&mut self, it: T) {
                           self.available = 0;
                       }
                   }"#,
                BLOCKED,
            ),
        ];

        let mut unaccounted = Vec::new();
        for (what, src, expect) in routes {
            let i = inv(src);
            let accounted = if expect.is_empty() {
                !i.escapes.is_empty() || !i.unclassified.is_empty()
            } else {
                i.constructors
                    .iter()
                    .chain(i.mutators.iter())
                    .any(|o| o == expect)
            };
            if !accounted {
                unaccounted.push(format!(
                    "{what}\n      wanted: {}\n      constructors {:?}\n      mutators {:?}\n\
                     \x20     escapes {:?}\n      unclassified {:?}",
                    if expect.is_empty() {
                        "to be blocked".to_string()
                    } else {
                        format!("`{expect}` in the inventory")
                    },
                    i.constructors,
                    i.mutators,
                    i.escapes,
                    i.unclassified
                ));
            }
        }
        assert!(
            unaccounted.is_empty(),
            "{} of {} routes are invisible to this scan, so a property proved over the \
             operations it found would be claimed of a type these can change behind its \
             back:\n\n  {}",
            unaccounted.len(),
            routes.len(),
            unaccounted.join("\n\n  ")
        );
    }

    /// The body rule has to see a compound assignment. `-=` is not an
    /// assignment node, and reading only assignment nodes missed it.
    #[test]
    fn a_field_written_with_a_compound_assignment_is_a_write() {
        let block: syn::Block = syn::parse_str("{ b.available -= n; }").unwrap();
        let names = ["available".to_string()].into_iter().collect();
        let mut out = std::collections::BTreeSet::new();
        super::collect_field_writes(&block, &names, &mut out);
        assert!(
            out.contains("available"),
            "`-=` writes the field too: {out:?}"
        );
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
