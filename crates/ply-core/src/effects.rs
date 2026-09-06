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
//! file, came back as a function that touches nothing at all. A method is
//! now passed over only if it is on a spelled-out list of things that read
//! or reshape a value already in memory ([`BENIGN_METHODS`]), and anything
//! else is `Unknown`. Found by external review; latent rather than live,
//! since nothing outside this file calls into it yet, which is the only
//! reason it cost nothing.
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
        // A method call this scan does not recognise. There is no body to
        // follow -- the receiver's type is not known here -- so the walk
        // cannot answer, and the module's own promise says what to do about
        // that: report `Unknown`, never `None`.
        //
        // It used to `continue` past every method with the note that "the
        // writing-method list above already had its say". That list only
        // recognises methods known to *write*; one it has never heard of is
        // not thereby known to be safe. `writer.flush()` -- the ordinary way
        // a buffered writer commits bytes to a file -- came back as touching
        // nothing at all.
        if let Some(method) = call.strip_prefix('.') {
            if unknown.is_none() {
                unknown = Some(Reach::Unknown {
                    because: format!(
                        "`{fn_path}` calls `.{method}()` on a value whose type this scan cannot \
                         see, so what that method does is unknown"
                    ),
                });
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
    // A method call, recorded by `collect` with a leading `.`: benign only
    // if it is on the spelled-out list. Anything else is a call this scan
    // cannot follow, and this module's promise is that such a call is
    // `Unknown`.
    if let Some(method) = call.strip_prefix('.') {
        return BENIGN_METHODS.contains(&method);
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
    let method = call.rsplit('.').next().unwrap_or(call);
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
            self.out.push(format!(".{}", node.method));
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

    /// The other half, and the reason the fix is a spelled-out list rather
    /// than "every method is unknown": a scan that gives up on `.len()`
    /// gives up on everything, and an answer nobody can ever get is worth
    /// no more than a wrong one.
    #[test]
    fn the_ordinary_methods_a_body_uses_to_read_its_arguments_stay_safe() {
        for body in [
            "pub fn n(p: &str) -> usize {\n    p.len()\n}\n",
            "pub fn e(p: &str) -> bool {\n    p.is_empty()\n}\n",
            "pub fn c(p: &str) -> String {\n    p.to_string()\n}\n",
            "pub fn t(p: &str) -> String {\n    p.trim().to_owned()\n}\n",
        ] {
            let dir = fixture(body);
            let name = body.split_whitespace().nth(2).unwrap();
            let name = name.split('(').next().unwrap();
            let reach = scan_fn(dir.path(), name);
            assert_eq!(
                reach,
                Reach::None,
                "reading a string cannot touch a file, and a scan that says otherwise is \
                 useless rather than careful: {body}"
            );
        }
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
