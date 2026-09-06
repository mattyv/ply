//! Does this function's body reach a call that writes to the filesystem?
//!
//! Ply runs the real function body. That is the whole reason its evidence is
//! worth anything, and it is also why one type has stayed refused while
//! every other shape was unlocked: a `Path` parameter means generating paths
//! and *executing the code against them*. Measured on Ply's own crates when
//! that refusal was written down, 8 of the 39 public functions taking a path
//! reach a filesystem write inside their own body -- saving a record,
//! writing a generated crate's manifest, writing its source. Unlocking paths
//! without asking this question first would have Ply create and delete files
//! at paths it invented.
//!
//! So this module answers exactly one question, syntactically, and it is
//! written to be wrong in only one direction.
//!
//! **It fails closed.** Three answers, not two: [`Reach::Writes`],
//! [`Reach::None`], and [`Reach::Unknown`]. Anything this scan cannot follow
//! -- a trait method, a closure called through a variable, a function in a
//! dependency, a file it could not parse -- is `Unknown`, and `Unknown` is
//! never treated as `None` by any caller. A safety check that guesses "no"
//! when it cannot see is not a safety check.
//!
//! **That paragraph was false until 2026-09-06**, for the commonest shape
//! in real Rust. Every method call was skipped outright, on the reasoning
//! that the list of methods known to *write* had already had its say -- but
//! a method that list has never heard of is not thereby known to be safe.
//! `writer.flush()`, the ordinary way a buffered writer commits bytes to a
//! file, came back as a function that touches nothing at all.
//!
//! **The first repair was itself unsound, and the same review caught it.**
//! Passing over any method on a spelled-out list of harmless *names* reads
//! a name as a method, and it is not one: `.clone()` runs the receiver's
//! own `Clone`, which is ordinary Rust and may open a file, and so may
//! `Display` behind `.to_string()` and `Iterator::next` behind `.next()`.
//! The hole simply moved one name along.
//!
//! Two things must now hold before a method is passed over: the name is on
//! [`BENIGN_METHODS`], **and** the receiver is a parameter whose declared
//! type has no user code anywhere inside it ([`STD_TRANSPARENT_TYPES`],
//! applied through type arguments, so `Vec<u8>` qualifies and
//! `Vec<Logger>` does not). A bare parameter name is the only receiver
//! whose type this scan can look up without being the type checker it is
//! not, so a chained call, a field, or a local is `Unknown` -- a real
//! narrowing of what this can answer for, and the honest one.
//!
//! **The second repair was unsound too**, caught by the same reviewer: it
//! compared the *last path segment* of the declared type, so `my::String`
//! counted as the standard library's `String`. A name is not an identity.
//! A path qualifies only when it is genuinely std's -- bare, with nothing
//! in the fn's own file or its own type parameters having taken that name
//! ([`names_the_crate_binds`]), or written out from a real standard-library
//! root ([`STD_CRATES`]).
//!
//! **And the third was unsound as well**, the same way one level along:
//! knowing the receiver's type is standard does not say *which method
//! runs on it*. A trait is ordinary Rust and may be implemented for `u32`;
//! once one is in scope, `x.next()` is that trait's `next` and may open a
//! file.
//!
//! ## The rule, stated over the whole input space
//!
//! Four repairs in a day, each a patch on the example that had been
//! reported, each leaving the same false-safe answer reachable by a route
//! the patch had not considered: names instead of methods, names instead of
//! types, then types instead of implementations. The reviewer who found all
//! four named the invariant they were approximating, and it is this:
//!
//! > **A call is passed over only where every implementation its name could
//! > resolve to is one this scan can read. Anything else is `Unknown`.**
//!
//! [`MethodScope`] is that rule for methods. A glob import can bring in an
//! extension trait invisibly; a `use` rooted outside the standard library
//! reaches code this scan is not reading; a trait declared here may name
//! the method itself. Any of the three and the name means nothing reliable,
//! whatever the receiver is. What survives is narrow and honest: a method
//! on a genuinely standard type, in a file whose scope this can see whole.
//!
//! Both scope walks are `syn` visitors, which reach every node by
//! construction. The fifth round found the rule right and the walk wrong:
//! it enumerated where a `use` could appear -- file items, inline modules --
//! and Rust allows one in a function body, a nested block, an `impl`, or
//! another function. Completeness is the compiler's job now, not a person's
//! memory, which is the same move as the wildcard-free matches elsewhere
//! here.
//!
//! All four were latent -- nothing outside this file calls into it yet,
//! which is the only reason any of them cost nothing.
//!
//! **What it is not.** It is not the capability tier §5.3 describes and it
//! does not implement `pure`/`uses:` enforcement (`A0402`, `A0403`, `A0408`
//! are still planned and still emit nothing). It answers one question about
//! one kind of effect. Reading it as "this function is pure" would be
//! reading far more into it than it checked.

use std::collections::BTreeSet;
use std::path::Path;

use syn::visit::Visit;

/// What a scan found. Deliberately three-valued: "I could not tell" is a
/// different fact from "no", and collapsing them is how a check like this
/// stops protecting anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reach {
    /// A call that writes to the filesystem is reachable from this body.
    /// Carries the chain that leads to it, callee-last, so a refusal can
    /// name the route rather than only the verdict.
    Writes { via: Vec<String> },
    /// Nothing reachable from this body writes to the filesystem, and every
    /// step of the walk was one this scan could actually read.
    None,
    /// The walk hit something it could not follow. Never read as `None`.
    Unknown { because: String },
}

impl Reach {
    /// Whether it is safe to generate values for a parameter this function
    /// will be run against. `Unknown` is not safe -- that is the whole
    /// point of it being its own answer.
    pub fn is_safe(&self) -> bool {
        matches!(self, Reach::None)
    }
}

/// The standard-library calls that write to the filesystem, spelled the way
/// source spells them at a call site.
///
/// A deliberately short, closed list of the ones that create, modify or
/// remove something. Reading and metadata are absent on purpose: this
/// question is about what a generated path could *damage*, and opening a
/// file Ply invented the name of does nothing.
///
/// The list being closed is what makes the scan honest in the other
/// direction too -- a writing call this does not know is not silently
/// approved, because reaching *any* unresolvable call already makes the
/// answer `Unknown`.
const WRITING_CALLS: &[&str] = &[
    "std::fs::write",
    "std::fs::copy",
    "std::fs::rename",
    "std::fs::remove_file",
    "std::fs::remove_dir",
    "std::fs::remove_dir_all",
    "std::fs::create_dir",
    "std::fs::create_dir_all",
    "std::fs::hard_link",
    "std::fs::soft_link",
    "std::fs::set_permissions",
    "std::fs::File::create",
    "std::fs::File::create_new",
    "std::fs::OpenOptions::new",
    "fs::write",
    "fs::copy",
    "fs::rename",
    "fs::remove_file",
    "fs::remove_dir",
    "fs::remove_dir_all",
    "fs::create_dir",
    "fs::create_dir_all",
    "fs::hard_link",
    "fs::soft_link",
    "fs::set_permissions",
    "File::create",
    "File::create_new",
    "OpenOptions::new",
];

/// Method names that write through an already-open handle. Matched on the
/// method name alone, because a receiver's type is not something this
/// syntactic scan resolves -- so `w.write_all(..)` counts whatever `w` is.
///
/// That over-counts: a `Vec<u8>` also has `write_all`. Over-counting is the
/// safe direction here, and the cost is a function refused that need not
/// have been, which is a smaller cost than a file written at an invented
/// path.
const WRITING_METHODS: &[&str] = &[
    "write_all",
    "write_fmt",
    "set_len",
    "set_permissions",
    "sync_all",
    "sync_data",
    "persist",
];

/// Calls that plainly touch no file, so reaching one is not a reason to
/// give up on an answer.
///
/// This list is what makes the scan useful rather than merely safe. Without
/// it every real function reaches something -- `Ok`, `format!`, a read --
/// and comes back `Unknown`, which refuses everything and protects nothing
/// extra. Measured on Ply's own library before this list existed: 35
/// path-taking public functions, 4 correctly found writing, and **0** ever
/// cleared.
///
/// Reads are here deliberately. Opening a file whose name Ply invented
/// returns an error and changes nothing; the question this module asks is
/// what a generated path could *damage*.
///
/// Spawning a process is **not** here, and that is the point of the
/// omission: a subprocess can do anything at all, so a body that runs one
/// is `Unknown` rather than cleared.
const BENIGN_CALLS: &[&str] = &[
    // Enum and option constructors.
    "Ok",
    "Err",
    "Some",
    "None",
    // Reads and metadata: no file is created, changed or removed.
    "std::fs::read",
    "std::fs::read_to_string",
    "std::fs::read_dir",
    "std::fs::metadata",
    "std::fs::canonicalize",
    "std::fs::File::open",
    "fs::read",
    "fs::read_to_string",
    "fs::read_dir",
    "fs::metadata",
    "fs::canonicalize",
    "File::open",
];

/// Path prefixes whose calls are benign for this question: pure data
/// handling. A call whose spelling starts with one of these is not a reason
/// to give up.
const BENIGN_PREFIXES: &[&str] = &[
    "String::",
    "Vec::",
    "Path::",
    "PathBuf::",
    "OsString::",
    "format!",
    "std::string::",
    "std::vec::",
    "std::path::Path::",
    "std::path::PathBuf::",
    "std::collections::",
    "std::cmp::",
    "std::iter::",
    "std::mem::",
];

/// Associated functions on standard-library types that build or read a
/// value and touch nothing.
///
/// Closed and spelled out, for the same reason [`WRITING_CALLS`] is: a
/// blanket "any `Type::new` is fine" rule would clear a third-party
/// `Logger::new` that opens a log file, which is exactly the case this
/// module exists to catch. `Command::new` is the one that proves the point
/// -- same shape, and a subprocess can do anything at all, so it is
/// deliberately absent and a body reaching it stays `Unknown`.
const BENIGN_STD_ASSOC: &[&str] = &[
    "BTreeMap::new",
    "BTreeSet::new",
    "HashMap::new",
    "HashSet::new",
    "Vec::new",
    "Vec::with_capacity",
    "String::new",
    "String::with_capacity",
    "Instant::now",
    "SystemTime::now",
    "Duration::from_secs",
    "Duration::from_millis",
];

/// How deep the walk follows calls before giving up. A body that needs more
/// than this to reach a write is not proved safe, it is `Unknown`.
const MAX_DEPTH: usize = 6;

/// Answers the question for one function, following calls into first-party
/// source.
///
/// `fn_path` is spelled from the crate root, the same way a `ply.yaml` claim
/// spells one once its component's anchor has been applied.
pub fn reaches_filesystem_write(resolver: &mut crate::callgraph::Resolver, fn_path: &str) -> Reach {
    let mut seen = BTreeSet::new();
    walk(resolver, fn_path, 0, &mut seen)
}

fn walk(
    resolver: &mut crate::callgraph::Resolver,
    fn_path: &str,
    depth: usize,
    seen: &mut BTreeSet<String>,
) -> Reach {
    if depth > MAX_DEPTH {
        return Reach::Unknown {
            because: format!(
                "the chain of calls from here is more than {MAX_DEPTH} deep, and this scan \
                 stopped rather than keep going -- so it never reached the end and cannot say \
                 there is no write at it"
            ),
        };
    }
    // A cycle is not a write and not an unknown: every path out of it has
    // been walked already, or is being walked now by the frame that first
    // saw this function.
    if !seen.insert(fn_path.to_string()) {
        return Reach::None;
    }

    let found = match resolver.lookup_fn(fn_path) {
        crate::callgraph::Resolution::Found(f) => f,
        crate::callgraph::Resolution::NotFound => {
            return Reach::Unknown {
                because: format!(
                    "`{fn_path}` is called here and this scan could not find it in this crate's \
                     source, so what it does is unknown"
                ),
            };
        }
        crate::callgraph::Resolution::Opaque(reason)
        | crate::callgraph::Resolution::Refused(reason)
        | crate::callgraph::Resolution::Ambiguous(reason) => {
            return Reach::Unknown {
                because: format!("`{fn_path}` could not be read: {reason}"),
            };
        }
    };

    // A call this scan cannot resolve is only reported once the body has
    // been searched for a write it *can* see: "it writes" is a more useful
    // and more certain answer than "something here was unreadable", and a
    // body containing both should give the first.
    let mut unknown: Option<Reach> = None;
    let mut calls = Vec::new();
    collect(&found.item, &mut calls);
    let params = parameter_types(&found.item);
    // What the file this fn was declared in binds for itself, so a `String`
    // that is not the standard library's is not read as one. The fn's own
    // file rather than the crate root: that is the scope its signature was
    // written in, and it is what `FoundFn` carries for exactly this kind of
    // question.
    let mut shadowed = names_the_crate_binds(&found.file);
    // The function's own type parameters. `fn n<String: HasLen>(s: &String)`
    // binds `String` to whatever the caller passes, so the standard
    // library's `String` is exactly what it is not -- and shadow detection
    // walked the file's items and never looked here (external review,
    // 2026-09-06).
    for param in &found.item.sig.generics.params {
        if let syn::GenericParam::Type(t) = param {
            shadowed.insert(t.ident.to_string());
        }
    }
    // Whether a method name can be trusted to mean the standard library's
    // method at all -- see `MethodScope`.
    let scope = MethodScope::of(&found.file);

    for call in &calls {
        if let Some(name) = writing_call_name(call) {
            return Reach::Writes {
                via: vec![fn_path.to_string(), name],
            };
        }
    }

    // The module the caller lives in, so a sibling called by its bare name
    // resolves. Without this, `harness::discover_fn` calling `resolver_for`
    // reported "could not find `resolver_for`" and gave up -- the function
    // is right there, one module qualifier away, and treating an ordinary
    // same-module call as unreadable made the scan give up on most real
    // code.
    let caller_module = found
        .canonical
        .rsplit_once("::")
        .map(|(head, _)| head.to_string());

    for call in &calls {
        if is_benign(call) {
            continue;
        }
        // A method call. Two things have to hold before the walk may pass
        // over one, and for a day only the first of them did.
        //
        // The name must be on the harmless list -- that much was already
        // true, and it is what stopped `writer.flush()` reading as "touches
        // no file".
        //
        // And the receiver's type must contain no user code, because a
        // method name is not a method: `.clone()` runs the receiver's own
        // `Clone`, which is ordinary Rust and may open a file, and so may
        // `Display` behind `.to_string()` and `Iterator::next` behind
        // `.next()`. Whitelisting by name alone reintroduced the same hole
        // one name along (external review, 2026-09-06).
        //
        // The only receiver whose type this scan can look up without being
        // a type checker is a bare parameter name. Anything else -- a
        // chained call, a field, a local -- is `Unknown`, which is the
        // honest answer and the safe one.
        if let Some(rest) = call.strip_prefix('.') {
            let (method, receiver) = rest.split_once('@').unwrap_or((rest, ""));
            let receiver_is_std = (!receiver.is_empty())
                .then(|| params.get(receiver))
                .flatten()
                .is_some_and(|ty| is_transparently_std(ty, &shadowed));
            let unresolvable = scope.cannot_resolve(method);
            if receiver_is_std && BENIGN_METHODS.contains(&method) && unresolvable.is_none() {
                continue;
            }
            if unknown.is_none() {
                let because = if let Some(why) = unresolvable {
                    format!("`{fn_path}` calls `.{method}()`, and {why}")
                } else if receiver.is_empty() {
                    format!(
                        "`{fn_path}` calls `.{method}()` on a value this scan cannot name, so \
                         whose method it is -- and what that method does -- is unknown"
                    )
                } else {
                    format!(
                        "`{fn_path}` calls `.{method}()` on `{receiver}`, whose type is not one \
                         this scan can see inside, so what that method does is unknown"
                    )
                };
                unknown = Some(Reach::Unknown { because });
            }
            continue;
        }
        // Same module first, then the crate root -- the two spellings an
        // ordinary call site uses.
        let resolved = caller_module
            .as_ref()
            .map(|m| format!("{m}::{call}"))
            .filter(|q| {
                matches!(
                    resolver.lookup_fn(q),
                    crate::callgraph::Resolution::Found(_)
                )
            })
            .unwrap_or_else(|| call.clone());
        match walk(resolver, &resolved, depth + 1, seen) {
            Reach::Writes { mut via } => {
                via.insert(0, fn_path.to_string());
                return Reach::Writes { via };
            }
            Reach::None => {}
            u @ Reach::Unknown { .. } => {
                if unknown.is_none() {
                    unknown = Some(u);
                }
            }
        }
    }

    unknown.unwrap_or(Reach::None)
}

/// Methods that plainly touch no file, so meeting one is not a reason to
/// give up on an answer.
///
/// Closed and spelled out, for exactly the reason [`BENIGN_STD_ASSOC`] is:
/// a blanket "methods are fine" rule is what this module had until
/// 2026-09-06, and it cleared `writer.flush()` -- the ordinary way a
/// buffered writer commits bytes to a file -- as touching nothing.
///
/// Everything here reads or reshapes a value already in memory. Nothing
/// here opens, creates, truncates, renames or removes anything, and nothing
/// here can be *made* to by a caller's choice of receiver: these are
/// inherent methods and trait methods on the standard library's own
/// containers, strings, slices, options and results.
///
/// The bar for adding one: name a type whose implementation of it could
/// touch a file. If you can, it does not belong here. `flush`, `write`,
/// `send`, `spawn`, `execute` and `commit` all fail that bar, which is why
/// none of them appears -- a body reaching any of them stays `Unknown`,
/// which is the honest answer and the safe one.
const BENIGN_METHODS: &[&str] = &[
    // Length, emptiness and membership.
    "len",
    "is_empty",
    "contains",
    "contains_key",
    "starts_with",
    "ends_with",
    "count",
    // Copying and converting a value already in hand.
    "clone",
    "to_string",
    "to_owned",
    "to_vec",
    "into",
    "as_str",
    "as_ref",
    "as_bytes",
    "as_slice",
    "as_deref",
    "as_mut",
    "borrow",
    "cloned",
    "copied",
    "to_lowercase",
    "to_uppercase",
    "to_ascii_lowercase",
    "to_ascii_uppercase",
    // Reshaping text and slices.
    "trim",
    "trim_start",
    "trim_end",
    "split",
    "splitn",
    "rsplit",
    "split_once",
    "rsplit_once",
    "split_whitespace",
    "lines",
    "chars",
    "bytes",
    "join",
    "repeat",
    "replace",
    "strip_prefix",
    "strip_suffix",
    "trim_matches",
    "trim_start_matches",
    "trim_end_matches",
    "parse",
    "get",
    "first",
    "last",
    "iter",
    "iter_mut",
    "into_iter",
    "next",
    "rev",
    "collect",
    "map",
    "filter",
    "filter_map",
    "flat_map",
    "flatten",
    "any",
    "all",
    "find",
    "find_map",
    "fold",
    "sum",
    "product",
    "min",
    "max",
    "min_by_key",
    "max_by_key",
    "sort",
    "sort_by",
    "sort_by_key",
    "dedup",
    "take",
    "skip",
    "zip",
    "chain",
    "enumerate",
    "peekable",
    "position",
    // Growing an in-memory container.
    "push",
    "push_str",
    "pop",
    "insert",
    "extend",
    "retain",
    "remove",
    "entry",
    "or_default",
    "or_insert",
    "or_insert_with",
    // Options and results, with the two that panic rather than write.
    "unwrap",
    "unwrap_or",
    "unwrap_or_else",
    "unwrap_or_default",
    "expect",
    "ok",
    "ok_or",
    "ok_or_else",
    "err",
    "is_some",
    "is_none",
    "is_ok",
    "is_err",
    "and_then",
    "unwrap_err",
    // Paths: naming a file is not touching one. Every one of these answers
    // a question about a path value and opens nothing.
    "display",
    "to_path_buf",
    "to_str",
    "to_string_lossy",
    "file_name",
    "file_stem",
    "extension",
    "parent",
    "components",
    "with_extension",
    "with_file_name",
    // Formatting and comparison.
    "eq",
    "ne",
    "cmp",
    "partial_cmp",
    "hash",
    "abs",
    "saturating_sub",
    "saturating_add",
    "checked_add",
    "checked_sub",
    "checked_mul",
    "wrapping_add",
];

/// Standard-library types whose own implementations are the only ones a
/// method on them can reach.
///
/// The point is *not* "these are std types". It is that a value of one of
/// these has no user code inside it to run. `Vec<u8>::clone` clones bytes;
/// `Vec<Logger>::clone` calls `Logger::clone`, which is ordinary Rust and
/// may open a file. So this list is applied recursively through type
/// arguments, and a container of anything not on it is not on it either.
///
/// Coherence is what makes this sound for the trait methods in
/// [`BENIGN_METHODS`]: nobody outside `std` can implement `Display for str`
/// or `Clone for u32`, so `.to_string()` on a `&str` really is std's, and
/// an inherent method such as `len` wins over any trait in scope regardless.
const STD_TRANSPARENT_TYPES: &[&str] = &[
    "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32",
    "f64", "bool", "char", "str", "String", "Path", "PathBuf", "OsStr", "OsString", "Vec",
    "VecDeque", "BTreeMap", "BTreeSet", "HashMap", "HashSet", "Option", "Result", "Duration",
];

/// Every name this crate binds to a type of its own -- declared here, or
/// imported from somewhere else under that name.
///
/// A bare `String` in a signature means the standard library's only if
/// nothing in the crate has taken that name, and taking it is ordinary
/// Rust: `struct String;` or `use my::String;`. Without this, a user type
/// wearing a standard-library name was read as the standard library's, and
/// its own inherent `len` -- which may write a file -- was passed over.
fn names_the_crate_binds(file: &syn::File) -> BTreeSet<String> {
    // A name bound inside a module or a body is not in scope at the crate
    // root, so this is coarser than Rust's own rules. Coarser, never wrong:
    // the trade this module makes everywhere else.
    struct V<'a>(&'a mut BTreeSet<String>);
    impl<'ast> syn::visit::Visit<'ast> for V<'_> {
        fn visit_item_struct(&mut self, i: &'ast syn::ItemStruct) {
            self.0.insert(i.ident.to_string());
            syn::visit::visit_item_struct(self, i);
        }
        fn visit_item_enum(&mut self, i: &'ast syn::ItemEnum) {
            self.0.insert(i.ident.to_string());
            syn::visit::visit_item_enum(self, i);
        }
        fn visit_item_union(&mut self, i: &'ast syn::ItemUnion) {
            self.0.insert(i.ident.to_string());
            syn::visit::visit_item_union(self, i);
        }
        fn visit_item_type(&mut self, i: &'ast syn::ItemType) {
            self.0.insert(i.ident.to_string());
            syn::visit::visit_item_type(self, i);
        }
        fn visit_item_trait(&mut self, i: &'ast syn::ItemTrait) {
            self.0.insert(i.ident.to_string());
            syn::visit::visit_item_trait(self, i);
        }
        fn visit_item_use(&mut self, i: &'ast syn::ItemUse) {
            collect_use(&i.tree, self.0);
            syn::visit::visit_item_use(self, i);
        }
    }
    fn collect_use(tree: &syn::UseTree, out: &mut BTreeSet<String>) {
        match tree {
            syn::UseTree::Path(p) => collect_use(&p.tree, out),
            syn::UseTree::Name(n) => {
                out.insert(n.ident.to_string());
            }
            syn::UseTree::Rename(r) => {
                out.insert(r.rename.to_string());
            }
            syn::UseTree::Group(g) => {
                for t in &g.items {
                    collect_use(t, out);
                }
            }
            // `use foo::*;` can bring in anything at all, including a type
            // wearing a standard-library name. Nothing here can see what,
            // so nothing after it can be trusted to be std's.
            syn::UseTree::Glob(_) => {
                out.insert("*".to_string());
            }
        }
    }
    let mut out = BTreeSet::new();
    syn::visit::Visit::visit_file(&mut V(&mut out), file);
    out
}

/// What this file lets the scan establish about *which implementation* a
/// method call runs.
///
/// Knowing the receiver's type is standard does not answer that question,
/// and the whitelist approach kept assuming it did. A trait is ordinary
/// Rust and may be implemented for `u32`; once one is in scope, `x.next()`
/// is that trait's `next` and may open a file. So the rule is no longer
/// "is this name harmless" but **"can this scan see every implementation
/// the name could resolve to"** -- and where it cannot, the answer is
/// `Unknown`. That is the invariant the four name-shaped repairs were each
/// an approximation of (external review, 2026-09-06).
struct MethodScope {
    /// Something in scope can define methods this scan cannot read, so no
    /// method name can be trusted to mean what the standard library means
    /// by it. A glob import can bring in an extension trait invisibly; a
    /// `use` rooted anywhere but the standard library reaches code this
    /// scan is not reading.
    opaque: Option<String>,
    /// Method names declared by traits this file *can* see. A trait
    /// declaring `next` puts `x.next()` beyond the standard library's
    /// `Iterator` whatever the receiver is.
    trait_methods: BTreeSet<String>,
}

impl MethodScope {
    /// Read from the file the checked function was declared in, through
    /// `syn`'s visitor: a hand-written walk covers the positions somebody
    /// thought of. See the module header.
    fn of(file: &syn::File) -> Self {
        struct V<'a>(&'a mut MethodScope);
        impl<'ast> syn::visit::Visit<'ast> for V<'_> {
            fn visit_item_trait(&mut self, t: &'ast syn::ItemTrait) {
                for i in &t.items {
                    if let syn::TraitItem::Fn(f) = i {
                        self.0.trait_methods.insert(f.sig.ident.to_string());
                    }
                }
                syn::visit::visit_item_trait(self, t);
            }
            fn visit_item_use(&mut self, u: &'ast syn::ItemUse) {
                note_use(&u.tree, true, self.0);
                syn::visit::visit_item_use(self, u);
            }
        }
        fn note_use(tree: &syn::UseTree, at_root: bool, scope: &mut MethodScope) {
            match tree {
                syn::UseTree::Path(p) => {
                    if at_root && !STD_CRATES.contains(&p.ident.to_string().as_str()) {
                        scope.opaque.get_or_insert(format!(
                            "`use {}::...` reaches code this scan is not reading, and any of it \
                             may be a trait adding methods to ordinary types",
                            p.ident
                        ));
                    }
                    note_use(&p.tree, false, scope);
                }
                syn::UseTree::Group(g) => {
                    for t in &g.items {
                        note_use(t, at_root, scope);
                    }
                }
                syn::UseTree::Glob(_) => {
                    scope.opaque.get_or_insert(
                        "a glob import can bring in a trait that adds methods to ordinary types, \
                         and nothing here can see what it brought"
                            .to_string(),
                    );
                }
                syn::UseTree::Name(n) => {
                    if at_root && !STD_CRATES.contains(&n.ident.to_string().as_str()) {
                        scope.opaque.get_or_insert(format!(
                            "`use {}` may name a trait this scan cannot read",
                            n.ident
                        ));
                    }
                }
                syn::UseTree::Rename(r) => {
                    if at_root && !STD_CRATES.contains(&r.ident.to_string().as_str()) {
                        scope.opaque.get_or_insert(format!(
                            "`use {} as ...` may name a trait this scan cannot read",
                            r.ident
                        ));
                    }
                }
            }
        }
        let mut scope = MethodScope {
            opaque: None,
            trait_methods: BTreeSet::new(),
        };
        syn::visit::Visit::visit_file(&mut V(&mut scope), file);
        scope
    }

    /// Why this method's implementation cannot be established, if it cannot.
    fn cannot_resolve(&self, method: &str) -> Option<String> {
        if let Some(reason) = &self.opaque {
            return Some(reason.clone());
        }
        self.trait_methods.contains(method).then(|| {
            format!(
                "a trait in this crate declares `{method}`, so which `{method}` runs here \
                 depends on what is implemented for the receiver"
            )
        })
    }
}

/// The crate roots a genuinely standard-library path starts at.
const STD_CRATES: &[&str] = &["std", "core", "alloc"];

/// Whether a declared parameter type contains no user code at all -- see
/// [`STD_TRANSPARENT_TYPES`] for why that is the question rather than "is
/// this a std type".
///
/// `shadowed` is what the crate binds for itself ([`names_the_crate_binds`]).
/// A name in it is not the standard library's, whatever it spells.
fn is_transparently_std(ty: &syn::Type, shadowed: &BTreeSet<String>) -> bool {
    match ty {
        syn::Type::Reference(r) => is_transparently_std(&r.elem, shadowed),
        syn::Type::Paren(p) => is_transparently_std(&p.elem, shadowed),
        syn::Type::Group(g) => is_transparently_std(&g.elem, shadowed),
        syn::Type::Slice(s) => is_transparently_std(&s.elem, shadowed),
        syn::Type::Array(a) => is_transparently_std(&a.elem, shadowed),
        syn::Type::Tuple(t) => t.elems.iter().all(|e| is_transparently_std(e, shadowed)),
        syn::Type::Path(p) => {
            // A qualified `<T as Trait>::Assoc`: the real type comes from an
            // impl this scan is not reading.
            if p.qself.is_some() {
                return false;
            }
            let segments: Vec<String> = p
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();
            let Some(seg) = p.path.segments.last() else {
                return false;
            };
            let name = seg.ident.to_string();
            if !STD_TRANSPARENT_TYPES.contains(&name.as_str()) {
                return false;
            }
            // Reading only the last segment is what let `my::String` count
            // as `String` (external review, 2026-09-06). A name is the
            // standard library's under exactly two spellings, and `my::`
            // anything is neither.
            let is_std_path = match segments.len() {
                // Bare, and only if nothing in this crate has taken the
                // name for a type of its own. A glob import means anything
                // could have.
                1 => !shadowed.contains(&name) && !shadowed.contains("*"),
                // Qualified, and only from a real standard-library root --
                // `std::string::String`, never `my::String`. A leading `::`
                // parses with the same segments, which is still correct:
                // `::std::string::String` is std's.
                _ => STD_CRATES.contains(&segments[0].as_str()),
            };
            if !is_std_path {
                return false;
            }
            match &seg.arguments {
                syn::PathArguments::None => true,
                syn::PathArguments::AngleBracketed(args) => args.args.iter().all(|a| match a {
                    syn::GenericArgument::Type(t) => is_transparently_std(t, shadowed),
                    syn::GenericArgument::Lifetime(_) => true,
                    _ => false,
                }),
                syn::PathArguments::Parenthesized(_) => false,
            }
        }
        // A generic parameter, `impl Trait`, a trait object, a raw pointer,
        // a function pointer: every one of them is a promise that the real
        // type arrives from somewhere this scan cannot see.
        _ => false,
    }
}

/// The declared type of each named parameter, for the one lookup this scan
/// can do without being a type checker.
fn parameter_types(f: &syn::ItemFn) -> std::collections::BTreeMap<String, syn::Type> {
    let mut out = std::collections::BTreeMap::new();
    for input in &f.sig.inputs {
        if let syn::FnArg::Typed(pt) = input
            && let syn::Pat::Ident(ident) = &*pt.pat
        {
            out.insert(ident.ident.to_string(), (*pt.ty).clone());
        }
    }
    out
}

/// Whether this call is one the scan can pass over without following.
fn is_benign(call: &str) -> bool {
    let last_two = call
        .rsplit("::")
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("::");
    // A method call is not decided here: whether it is harmless depends on
    // the receiver's type, which `is_benign` cannot see. `walk` decides it.
    if call.starts_with('.') {
        return false;
    }
    BENIGN_CALLS.contains(&call)
        || BENIGN_STD_ASSOC.contains(&last_two.as_str())
        || BENIGN_PREFIXES.iter().any(|p| call.starts_with(p))
        // A bare enum variant or tuple-struct constructor: `Ok`, `Some`, or
        // a user's own `Foo(..)`. Capitalised, one segment, no lowercase
        // start -- the spelling Rust reserves for a type or variant, never
        // for a function that could write a file.
        || (!call.contains("::")
            && call.chars().next().is_some_and(char::is_uppercase))
}

/// The name of the writing call this site is, if it is one.
fn writing_call_name(call: &str) -> Option<String> {
    if WRITING_CALLS.contains(&call) {
        return Some(call.to_string());
    }
    let last = call.rsplit("::").next().unwrap_or(call);
    // A method call is recorded as `.name@receiver`; the name is what the
    // writing list is written in terms of, and it is suspicious whatever
    // the receiver turns out to be.
    let method = call
        .rsplit('.')
        .next()
        .unwrap_or(call)
        .split('@')
        .next()
        .unwrap_or(call);
    if WRITING_METHODS.contains(&last) || WRITING_METHODS.contains(&method) {
        return Some(call.to_string());
    }
    None
}

/// Every call this body makes, as source spells it. A method call is
/// recorded with a leading `.` so the walk can tell it apart from a free
/// function it could follow.
fn collect(f: &syn::ItemFn, out: &mut Vec<String>) {
    struct C<'a> {
        out: &'a mut Vec<String>,
    }
    impl<'a> Visit<'a> for C<'a> {
        fn visit_expr_call(&mut self, node: &'a syn::ExprCall) {
            if let syn::Expr::Path(p) = &*node.func {
                let path: Vec<String> = p
                    .path
                    .segments
                    .iter()
                    .map(|s| s.ident.to_string())
                    .collect();
                self.out.push(path.join("::"));
            }
            syn::visit::visit_expr_call(self, node);
        }
        fn visit_expr_method_call(&mut self, node: &'a syn::ExprMethodCall) {
            // The receiver, when it is a plain name. That is the only shape
            // whose type this scan can look up without being a type
            // checker: a bare identifier that a parameter declares. A
            // method on the result of another call, on a field, or on a
            // local whose type is inferred records no receiver, and no
            // receiver means no way to know whose method this is.
            let receiver = match &*node.receiver {
                syn::Expr::Path(p) => p.path.get_ident().map(ToString::to_string),
                syn::Expr::Unary(u) if matches!(u.op, syn::UnOp::Deref(_)) => match &*u.expr {
                    syn::Expr::Path(p) => p.path.get_ident().map(ToString::to_string),
                    _ => None,
                },
                syn::Expr::Reference(r) => match &*r.expr {
                    syn::Expr::Path(p) => p.path.get_ident().map(ToString::to_string),
                    _ => None,
                },
                _ => None,
            };
            self.out
                .push(format!(".{}@{}", node.method, receiver.unwrap_or_default()));
            syn::visit::visit_expr_method_call(self, node);
        }
    }
    let mut c = C { out };
    c.visit_block(&f.block);
}

/// [`reaches_filesystem_write`] for a caller with only a crate directory.
pub fn scan_fn(crate_dir: &Path, fn_path: &str) -> Reach {
    let lib = crate_dir.join("src/lib.rs");
    let Ok(src) = std::fs::read_to_string(&lib) else {
        return Reach::Unknown {
            because: format!(
                "this crate has no readable `src/lib.rs`, so nothing about `{fn_path}` could be \
                 read at all"
            ),
        };
    };
    let Ok(mut resolver) =
        crate::callgraph::Resolver::new(&src, crate_dir, std::collections::BTreeMap::new())
    else {
        return Reach::Unknown {
            because: "this crate's source could not be parsed, so what its functions do is \
                      unknown"
                .to_string(),
        };
    };
    reaches_filesystem_write(&mut resolver, fn_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixture with more than one source file, for the cases where "the
    /// trait is somewhere this file cannot see" is the whole point.
    fn fixture_with(src: &str, extra: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = fixture(src);
        for (name, body) in extra {
            std::fs::write(dir.path().join("src").join(name), body).unwrap();
        }
        dir
    }

    fn fixture(src: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"effects-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), src).unwrap();
        dir
    }

    /// The case the whole refusal exists for: a function that takes a path
    /// and writes to it. Ply must never generate values for this one.
    #[test]
    fn a_body_that_writes_a_file_is_reported_as_writing() {
        let dir = fixture(
            "pub fn save(p: &str, body: &str) -> bool {\n    \
             std::fs::write(p, body).is_ok()\n}\n",
        );
        let reach = scan_fn(dir.path(), "save");
        assert!(
            matches!(&reach, Reach::Writes { via } if via.last().unwrap().ends_with("fs::write")),
            "the write is right there in the body: {reach:?}"
        );
        assert!(!reach.is_safe());
    }

    /// A write one call deep. A scan that only looked at the body in front
    /// of it would clear this, which is the failure mode that matters --
    /// almost nothing writes a file directly in the function you claimed.
    #[test]
    fn a_write_reached_through_a_helper_is_still_found() {
        let dir = fixture(
            "fn inner(p: &str) -> bool {\n    std::fs::create_dir_all(p).is_ok()\n}\n\n\
             pub fn outer(p: &str) -> bool {\n    inner(p)\n}\n",
        );
        let reach = scan_fn(dir.path(), "outer");
        let Reach::Writes { via } = &reach else {
            panic!("a write one call deep is still a write: {reach:?}");
        };
        assert_eq!(
            via.first().map(String::as_str),
            Some("outer"),
            "the chain names where the walk started, so a refusal can show the route: {via:?}"
        );
        assert!(via.iter().any(|s| s.contains("create_dir_all")), "{via:?}");
    }

    /// Arithmetic on a path's length touches no file. This is the answer
    /// that unlocks anything, so it has to be reachable -- a scan that
    /// called everything unsafe would be as useless as one that called
    /// everything safe.
    #[test]
    fn a_body_that_only_reads_its_arguments_is_reported_as_safe() {
        let dir = fixture("pub fn depth(p: &str) -> usize {\n    p.len()\n}\n");
        let reach = scan_fn(dir.path(), "depth");
        assert_eq!(reach, Reach::None, "nothing here touches a file");
        assert!(reach.is_safe());
    }

    /// The direction this must be wrong in. A call it cannot follow is not
    /// evidence of safety, and treating it as such is how a check like this
    /// silently stops protecting anything.
    #[test]
    fn a_call_this_scan_cannot_follow_is_unknown_and_never_safe() {
        let dir = fixture("pub fn hand_off(p: &str) -> bool {\n    somewhere_else::store(p)\n}\n");
        let reach = scan_fn(dir.path(), "hand_off");
        assert!(
            matches!(reach, Reach::Unknown { .. }),
            "an unresolvable callee is unknown, never none: {reach:?}"
        );
        assert!(
            !reach.is_safe(),
            "and unknown must never read as safe -- this is the whole point of three answers"
        );
    }

    /// The same direction as the test above, for the shape that was
    /// getting through: a *method* this scan does not recognise.
    ///
    /// A method call was skipped outright -- "nothing to follow, and the
    /// writing-method list above already had its say" -- but that list only
    /// recognises methods known to write. A method it has never heard of is
    /// neither known to write nor known to be safe, and skipping it let the
    /// walk finish and answer `None`. So this module's own opening promise,
    /// "**It fails closed** ... anything this scan cannot follow ... is
    /// `Unknown`", was false for the commonest shape in real Rust.
    /// Reported by external review 2026-09-05.
    ///
    /// `flush` is the example that shows the stakes: it is exactly how a
    /// buffered writer commits bytes to a file, and it was answering "this
    /// function touches no file at all".
    #[test]
    fn a_method_this_scan_does_not_recognise_is_unknown_and_never_safe() {
        let dir = fixture(
            "pub fn commit<W: std::io::Write>(w: &mut W) -> bool {\n    w.flush().is_ok()\n}\n",
        );
        let reach = scan_fn(dir.path(), "commit");
        assert!(
            matches!(reach, Reach::Unknown { .. }),
            "`flush` is how a buffered writer commits bytes to a file, and this scan has never \
             heard of it -- that is the definition of a call it cannot follow: {reach:?}"
        );
        assert!(
            !reach.is_safe(),
            "and a call it cannot follow must never read as safe"
        );
    }

    /// A method name is not a method. `clone` on a user type runs that
    /// type's own `Clone`, and that body can do anything at all.
    ///
    /// The list of harmless method names added on 2026-09-06 was written as
    /// though every `.clone()` were `str`'s. It is not: a `Clone` impl is
    /// ordinary Rust and may open a file, and so may `Display` behind
    /// `.to_string()`, and `Iterator::next` behind `.next()`. So the
    /// whitelist reintroduced exactly the hole it was added to close -- one
    /// name further along -- and this scan's opening promise, that anything
    /// it cannot follow is `Unknown`, was false again for the commonest
    /// receiver in real code: a value of the user's own type.
    ///
    /// Reported by external review 2026-09-06, after the `flush` fix.
    #[test]
    fn a_harmless_looking_method_on_a_type_this_scan_cannot_see_is_unknown() {
        for body in [
            // `Clone` for a user type: the impl is right there in the file,
            // and it writes.
            "pub struct Logger;\n\nimpl Clone for Logger {\n    fn clone(&self) -> Logger {\n        \
             std::fs::write(\"/tmp/x\", b\"\").unwrap();\n        Logger\n    }\n}\n\n\
             pub fn copy_it(l: &Logger) -> Logger {\n    l.clone()\n}\n",
            // A generic receiver: the impl is not even in this crate.
            "pub fn describe<T: std::fmt::Display>(t: &T) -> String {\n    t.to_string()\n}\n",
            // A method on the result of another call: nothing names the
            // receiver's type at all.
            "pub fn first_word(s: &str) -> String {\n    s.split(' ').next().unwrap().to_string()\n}\n",
        ] {
            let dir = fixture(body);
            let name = body
                .split("pub fn ")
                .nth(1)
                .unwrap()
                .split(['(', '<'])
                .next()
                .unwrap();
            let reach = scan_fn(dir.path(), name);
            assert!(
                !reach.is_safe(),
                "the receiver's type is not knowable here, so what its method does is not \
                 knowable either, and answering `safe` is the one direction this must never be \
                 wrong in:\n{body}\ngot {reach:?}"
            );
        }
    }

    /// The other half, and the reason the fix is not "every method is
    /// unknown": a scan that gives up on `.len()` gives up on everything,
    /// and an answer nobody can ever get is worth no more than a wrong one.
    ///
    /// The line this draws is exactly what the scan can establish without
    /// being a type checker: **a named parameter whose declared type has no
    /// user code anywhere inside it.** `p: &str` qualifies, so `p.len()` is
    /// `str::len` and nothing else. `Vec<u8>` qualifies; `Vec<Logger>` does
    /// not, because cloning one clones `Logger`s. And a chained call has no
    /// name to look up at all -- `p.trim().to_owned()` is `Unknown`, tested
    /// next door, and that is a real narrowing of what this can answer for,
    /// not an oversight.
    #[test]
    fn a_std_method_on_a_declared_std_parameter_stays_safe() {
        for body in [
            "pub fn n(p: &str) -> usize {\n    p.len()\n}\n",
            "pub fn e(p: &str) -> bool {\n    p.is_empty()\n}\n",
            "pub fn c(p: &str) -> String {\n    p.to_string()\n}\n",
            "pub fn s(p: &String) -> String {\n    p.clone()\n}\n",
            "pub fn v(xs: &Vec<u8>) -> usize {\n    xs.len()\n}\n",
            "pub fn o(x: Option<u32>) -> u32 {\n    x.unwrap_or_default()\n}\n",
        ] {
            let dir = fixture(body);
            let name = body.split_whitespace().nth(2).unwrap();
            let name = name.split('(').next().unwrap();
            let reach = scan_fn(dir.path(), name);
            assert_eq!(
                reach,
                Reach::None,
                "the receiver is a parameter declared as a type with no user code inside it, so \
                 this method is the standard library's own and cannot touch a file: {body}"
            );
        }
    }

    /// A type's *name* is not a type's identity, and the receiver check was
    /// reading only the last path segment.
    ///
    /// So `my::String` counted as the standard library's `String`, and
    /// `x.len()` on one was passed over -- even where that `len` is the
    /// user's own inherent method and writes a file. The `Logger::clone`
    /// case was closed and this one, the same false-safe answer reached a
    /// different way, was not. Reported by external review 2026-09-06, on
    /// the fix for the whitelist that preceded it.
    ///
    /// Three shapes, and none of them may read as safe: a qualified path
    /// whose tail merely spells a std name, a bare name the crate shadows
    /// with a type of its own, and a bare name the crate imports from
    /// somewhere else.
    #[test]
    fn a_type_that_merely_shares_a_std_types_name_is_not_that_type() {
        const WRITES: &str = "std::fs::write(\"/tmp/x\", b\"\").unwrap();";
        for (label, body) in [
            (
                "a qualified path whose last segment spells a std name",
                format!(
                    "pub mod my {{\n    pub struct String;\n    impl String {{\n        \
                     pub fn len(&self) -> usize {{ {WRITES} 0 }}\n    }}\n}}\n\n\
                     pub fn n(s: &my::String) -> usize {{\n    s.len()\n}}\n"
                ),
            ),
            (
                "a bare name the crate declares a type for",
                format!(
                    "pub struct String;\nimpl String {{\n    pub fn len(&self) -> usize {{ \
                     {WRITES} 0 }}\n}}\n\npub fn n(s: &String) -> usize {{\n    s.len()\n}}\n"
                ),
            ),
            (
                "a bare name the crate imports from elsewhere",
                format!(
                    "pub mod my {{\n    pub struct String;\n    impl String {{\n        \
                     pub fn len(&self) -> usize {{ {WRITES} 0 }}\n    }}\n}}\n\n\
                     use my::String;\n\npub fn n(s: &String) -> usize {{\n    s.len()\n}}\n"
                ),
            ),
        ] {
            let dir = fixture(&body);
            let reach = scan_fn(dir.path(), "n");
            assert!(
                !reach.is_safe(),
                "{label}: the receiver is not the standard library's type, so its `len` is not \
                 the standard library's either -- and this one writes a file:\n{body}\ngot \
                 {reach:?}"
            );
        }
    }

    /// Knowing the receiver is a standard type does not establish which
    /// method runs on it.
    ///
    /// A trait is ordinary Rust and may be implemented for `u32`. Once one
    /// is in scope, `x.next()` is that trait's `next`, and that body can
    /// open a file. The scan accepted the receiver (`u32` is std's),
    /// accepted the name (`next` is on the harmless list) and skipped the
    /// implementation entirely -- which is the same false-safe answer as
    /// the three before it, reached a fourth way.
    ///
    /// Reported by external review 2026-09-06, with the observation that
    /// ended the whitelist approach: every skipped call must resolve to a
    /// known harmless implementation, or the answer is `Unknown`.
    #[test]
    fn a_local_trait_can_add_a_harmless_looking_method_to_a_standard_type() {
        let dir = fixture(
            "pub trait Counter {\n    fn next(&self) -> u32;\n}\n\n\
             impl Counter for u32 {\n    fn next(&self) -> u32 {\n        \
             std::fs::write(\"/tmp/x\", b\"\").unwrap();\n        *self + 1\n    }\n}\n\n\
             pub fn step(x: u32) -> u32 {\n    x.next()\n}\n",
        );
        let reach = scan_fn(dir.path(), "step");
        assert!(
            !reach.is_safe(),
            "`next` here is this crate's own trait method on `u32`, and it writes a file. The \
             receiver being a standard type says nothing about whose method runs: {reach:?}"
        );
    }

    /// A generic parameter may be *named* `String`, and then `String` in
    /// that signature is the parameter, not the standard library's type.
    ///
    /// Shadow detection walked the file's items and never looked at the
    /// function's own generics, so `fn n<String: HasLen>(s: &String)` read
    /// as the standard type and the caller's `len` -- which can be
    /// anything at all -- was skipped. Same review, same day.
    #[test]
    fn a_generic_parameter_wearing_a_std_types_name_is_not_that_type() {
        let dir = fixture(
            "pub trait HasLen {\n    fn len(&self) -> usize;\n}\n\n\
             pub fn n<String: HasLen>(s: &String) -> usize {\n    s.len()\n}\n",
        );
        let reach = scan_fn(dir.path(), "n");
        assert!(
            !reach.is_safe(),
            "`String` here is this function's own type parameter, so its `len` is whatever the \
             caller's type does: {reach:?}"
        );
    }

    /// The new rule's own boundaries, pinned rather than assumed.
    ///
    /// Four repairs to this scanner were each a patch on a reported
    /// example, and each left the same false-safe answer reachable another
    /// way. The rule now is a claim about the whole input space -- *a
    /// method is passed over only where every implementation its name could
    /// resolve to is one this scan can read* -- so the cases that make it
    /// true are worth testing directly, not just the two examples that
    /// prompted it.
    ///
    /// `p.len()` on a declared `&str` is the same call in all four, and the
    /// answer changes with what else is in scope, because that is what
    /// decides whose `len` runs.
    #[test]
    fn whether_a_method_can_be_skipped_depends_on_what_else_is_in_scope() {
        let read = "pub fn n(p: &str) -> usize {\n    p.len()\n}\n";
        for (label, prelude, safe) in [
            ("nothing else in scope", "", true),
            (
                "a standard-library import, which cannot add methods std does not have",
                "use std::fmt::Debug;\n\n",
                true,
            ),
            (
                "a glob import, which can bring in an extension trait invisibly",
                "use crate::helpers::*;\n\npub mod helpers {}\n\n",
                false,
            ),
            (
                "an import from outside the standard library, which this scan is not reading",
                "use serde::Serialize;\n\n",
                false,
            ),
            (
                "a trait in this crate declaring the same method name",
                "pub trait Sized2 {\n    fn len(&self) -> usize;\n}\n\n",
                false,
            ),
        ] {
            let body = format!("{prelude}{read}");
            let dir = fixture(&body);
            let reach = scan_fn(dir.path(), "n");
            assert_eq!(
                reach.is_safe(),
                safe,
                "{label}: `p.len()` is unchanged, and whose `len` it is is not:\n{body}\ngot \
                 {reach:?}"
            );
        }
    }

    /// The general form of the defect above, and the reason the fix is a
    /// visitor rather than one more place added to a list.
    ///
    /// Five rounds of review on this scanner, and the shape repeated: a
    /// hand-written walk over the positions somebody thought of, and a
    /// position nobody thought of. So the walk is now `syn`'s own visitor,
    /// which reaches every node by construction, and this checks that --
    /// the same import, buried somewhere new each time.
    #[test]
    fn an_import_is_found_wherever_rust_allows_one() {
        let trait_file = (
            "extensions.rs",
            "pub trait Counter {\n    fn next(&self) -> u32;\n}\n\n\
             impl Counter for u32 {\n    fn next(&self) -> u32 {\n        \
             std::fs::write(\"/tmp/x\", b\"\").unwrap();\n        *self + 1\n    }\n}\n",
        );
        for (label, body) in [
            (
                "at the top of the file",
                "pub mod extensions;\nuse crate::extensions::Counter;\n\n\
                 pub fn step(x: u32) -> u32 {\n    x.next()\n}\n",
            ),
            (
                "inside the function body",
                "pub mod extensions;\n\npub fn step(x: u32) -> u32 {\n    \
                 use crate::extensions::Counter;\n    x.next()\n}\n",
            ),
            (
                "inside a nested block within the body",
                "pub mod extensions;\n\npub fn step(x: u32) -> u32 {\n    {\n        \
                 use crate::extensions::Counter;\n        x.next()\n    }\n}\n",
            ),
            (
                "inside an inline module beside the function",
                "pub mod extensions;\nmod inner {\n    use crate::extensions::Counter;\n}\n\n\
                 pub fn step(x: u32) -> u32 {\n    x.next()\n}\n",
            ),
            (
                "inside another function entirely",
                "pub mod extensions;\n\nfn elsewhere() {\n    \
                 use crate::extensions::Counter;\n}\n\n\
                 pub fn step(x: u32) -> u32 {\n    x.next()\n}\n",
            ),
        ] {
            let dir = fixture_with(body, &[trait_file]);
            let reach = scan_fn(dir.path(), "step");
            assert!(
                !reach.is_safe(),
                "{label}: an import is an import wherever Rust allows one, and this scan must \
                 not depend on somebody having thought of the position:\n{body}\ngot {reach:?}"
            );
        }
    }

    /// The shadow check had the same hand-written shape, and the same hole.
    ///
    /// A type declared inside a function body is legal Rust and shadows the
    /// standard library's name for the rest of that body. Nothing had
    /// reported it; it was found by asking what *else* in this file walked
    /// items by hand after the method-scope walk was got round. Fixed the
    /// same way, and pinned here so the answer does not depend on the
    /// position again.
    #[test]
    fn a_type_declared_inside_the_body_shadows_the_std_name_too() {
        for (label, body) in [
            (
                "declared in the function body",
                "pub fn n(s: &String) -> usize {\n    struct String;\n    impl String {\n        \
                 fn len(&self) -> usize { std::fs::write(\"/tmp/x\", b\"\").unwrap(); 0 }\n    \
                 }\n    s.len()\n}\n",
            ),
            (
                "declared in another function entirely",
                "fn elsewhere() {\n    struct String;\n}\n\n\
                 pub fn n(s: &String) -> usize {\n    s.len()\n}\n",
            ),
        ] {
            let dir = fixture(body);
            let reach = scan_fn(dir.path(), "n");
            assert!(
                !reach.is_safe(),
                "{label}: a name this crate binds anywhere is a name this scan cannot read as \
                 the standard library's:\n{body}\ngot {reach:?}"
            );
        }
    }

    /// The same shape one step over the line: a container of a *user* type.
    /// `Vec<Logger>::clone` clones `Logger`s, and `Logger::clone` is
    /// ordinary Rust that may open a file -- so the element type has to be
    /// looked through, not just the container.
    #[test]
    fn a_std_container_of_a_user_type_is_not_transparent() {
        let dir = fixture(
            "pub struct Logger;\n\nimpl Clone for Logger {\n    fn clone(&self) -> Logger {\n        \
             std::fs::write(\"/tmp/x\", b\"\").unwrap();\n        Logger\n    }\n}\n\n\
             pub fn dup(xs: &Vec<Logger>) -> Vec<Logger> {\n    xs.clone()\n}\n",
        );
        let reach = scan_fn(dir.path(), "dup");
        assert!(
            !reach.is_safe(),
            "cloning a `Vec<Logger>` runs `Logger::clone` once per element, and that body \
             writes a file: {reach:?}"
        );
    }

    /// A write through an already-open handle, which no `fs::` path names.
    #[test]
    fn a_write_through_a_handle_is_found_by_its_method_name() {
        let dir = fixture(
            "use std::io::Write;\n\npub fn dump(f: &mut std::fs::File, b: &[u8]) -> bool {\n    \
             f.write_all(b).is_ok()\n}\n",
        );
        let reach = scan_fn(dir.path(), "dump");
        assert!(!reach.is_safe(), "{reach:?}");
    }

    /// Recursion must not hang the walk, and must not be mistaken for a
    /// write either.
    #[test]
    fn a_recursive_body_terminates_without_inventing_an_answer() {
        let dir = fixture(
            "pub fn countdown(n: u32) -> u32 {\n    if n == 0 { 0 } else { countdown(n - 1) }\n}\n",
        );
        assert_eq!(scan_fn(dir.path(), "countdown"), Reach::None);
    }

    /// A body with both an unreadable call and a visible write reports the
    /// write: it is the more certain answer and the more useful one.
    #[test]
    fn a_visible_write_wins_over_an_unreadable_call_beside_it() {
        let dir = fixture(
            "pub fn both(p: &str) -> bool {\n    let _ = elsewhere::thing(p);\n    \
             std::fs::remove_file(p).is_ok()\n}\n",
        );
        assert!(matches!(scan_fn(dir.path(), "both"), Reach::Writes { .. }));
    }
}
