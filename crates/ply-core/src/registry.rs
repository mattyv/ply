//! The rule registry (`docs/rule-registry-design.md`): every diagnostic code
//! Ply can name, listed exactly once, as data rather than as a document two
//! humans have to keep in sync by hand.
//!
//! Measured on 2026-08-31 (see the invariant tests in
//! `crates/ply-core/tests/registry.rs` for how): the source and the two
//! documents that describe it (`The-Ply-Spec.md`, `docs/SCHEMA.md`) had
//! drifted apart three separate times before this table existed, each time
//! found and fixed by hand. A code appearing in a document and a code
//! appearing in the source are two different, unrelated facts unless
//! something checks that they agree — this module is that something, and
//! the two tests are the gate.
//!
//! **Where the codes here come from.** Every variant below is either found
//! by a real construction site in the source (a `Diagnostic { code: "...",
//! .. }` or `SchemaViolation`/`ArchFinding` literal, walked by
//! `tests/registry.rs`, never hand-copied from this file) or named in
//! `The-Ply-Spec.md`/`docs/SCHEMA.md`. Two codes those documents mention are
//! deliberately absent: `E0303` and `W0302` are not rules Ply promises and
//! has not built yet — §5.2a's own prose names them only to say **no such
//! code exists** ("there is no `stale` status, no `W0302`, ... and no
//! `E0303`"), so a row for either here would assert the opposite of what
//! the spec says. `F1024` (also spec text) is not a code at all: it is
//! glyph shorthand for `bounded(2) fuzz(1024)` inside a drawing's key.
//!
//! **`status` is computed, not asserted** (the whole reason this module
//! exists): a code is [`Status::Enforced`] only because a real emission
//! site for it exists in the source today, never because a document claims
//! it does. Where a code's *documented* meaning and its *emitted* behaviour
//! have quietly diverged, the row describes the emitted one — see `W0521`
//! below for the one case this build found.
//!
//! **`tier`** names the stage of Ply's own pipeline a code belongs to,
//! taken from the vocabulary the source and spec already use for
//! themselves rather than invented for this table: [`Tier::Schema`] and
//! [`Tier::Anchor`] are two of the three tiers `crate::diag::Coverage`
//! itself names for `cargo ply check` ("schema, anchors, architecture");
//! [`Tier::Crate`] and [`Tier::Item`] are the third, split the way
//! The-Ply-Spec.md §5.3 itself bolds them ("**Crate tier**", "**Item
//! tier**") because the two carry different soundness guarantees; and
//! [`Tier::Contract`] covers everything under §5.4, `cargo ply verify`'s
//! own per-function proof-engine checks.

/// One rule code, one variant, spelled exactly as the code itself so
/// [`std::fmt::Debug`] doubles as the string form (`format!("{:?}",
/// Code::E0204)` is `"E0204"`) with nothing to hand-maintain twice.
///
/// Defined by the `codes!` macro below together with [`Code::ALL`], so the
/// two can never drift apart the way a hand-written enum plus a
/// hand-written "list every variant" array could.
macro_rules! codes {
    ($($variant:ident),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[allow(non_camel_case_types)]
        pub enum Code {
            $($variant),+
        }

        impl Code {
            /// Every variant, exactly once, generated alongside the enum
            /// itself rather than maintained as a second list next to it.
            pub const ALL: &'static [Code] = &[$(Code::$variant),+];
        }
    };
}

codes!(
    // --- Tier::Schema: document-local ply.yaml validation (§5.1/§5.1a),
    // no anchored source needed. ---
    E0201, E0202, E0203, E0204, E0205, E0206, E0207, E0208, E0209, E0504, W0409, W0410,
    // --- Tier::Anchor: resolving a claim to real code (§5.2). ---
    E0301, E0304, E0306, // --- Tier::Crate: architecture, exact and sound (§5.3). ---
    A0401, A0405, A0409, A0410, A0411, A0412, A0413, A0414, A0415, W0413,
    // --- Tier::Item: architecture, approximate (§5.3) -- none of these
    // are built yet; see each row's status. ---
    A0402, A0403, A0404, A0406, A0407, A0408, W0411, W0412,
    // --- Tier::Contract: verify-time, per-function proof-engine checks
    // (§5.4). ---
    E0501, E0502, E0503, E0505, W0502, W0503, W0511, W0512, W0513, W0514, W0515, W0516, W0517,
    V0505, V0506, V0507, V0508, V0509, V0510, W0518, W0519, W0520, W0521, W0522, W0523, W0524,
    W0525, W0526, W0527, W0528, W0529, W0541, W0110, W0111, W0303, W0531, K0502, K0601, M0601,
    P0502, P0601, R0502, R0601, X0901, X0902, X0903, W0414, W0415, W0416, W0417, W0418, E0506,
    V0511,
);

/// The stage of Ply's own pipeline a code belongs to. See the module doc
/// for where this vocabulary comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Document-local `ply.yaml` validation -- schema shape, key
    /// vocabulary, micro-syntax. No anchored Rust source involved.
    Schema,
    /// Resolving a claim (an attribute, or a `ply.yaml` entry) to the real
    /// function or component it names.
    Anchor,
    /// Architecture: the exact crate dependency graph from `cargo
    /// metadata`. Sound -- every finding here is real, always an error.
    Crate,
    /// Architecture: the approximate, syntax-based call/capability graph.
    /// Advisory by default; `strict: true` escalates most of it to error.
    Item,
    /// `cargo ply verify`'s per-function checks: what a proof/fuzz/mutation
    /// engine found (or could not find) about one function's contract.
    Contract,
}

/// Whether a code is real today, computed from the source rather than
/// asserted -- see the module doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// A real construction site for this code exists in the source.
    Enforced,
    /// No construction site exists. The code is named in a document (a
    /// promised rule not yet built) and nowhere else.
    DeclaredOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// One row: everything a reader needs to know about one code without
/// reading the source that emits it or the document that describes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleEntry {
    pub code: Code,
    pub tier: Tier,
    pub status: Status,
    pub severity: Severity,
    /// One plain sentence, meeting the newbie bar CLAUDE.md sets for every
    /// user-facing sentence: what happened, what it means, why it matters
    /// -- in that order, with no code or § reference doing work the words
    /// should do first.
    pub gloss: &'static str,
    /// Where the reasoning behind this rule lives, for a reader who wants
    /// more than the one sentence above.
    pub spec_anchor: &'static str,
}

impl Code {
    /// This code's row. One `match` arm per variant, and the compiler
    /// refuses to build without every arm present -- the exhaustiveness
    /// `docs/rule-registry-design.md` recommends the `const` table for:
    /// add a variant to the `codes!` list above with no arm here, and nothing
    /// compiles until this function says what the new rule means.
    pub const fn entry(self) -> RuleEntry {
        use Code::*;
        use Severity::*;
        use Status::*;
        use Tier::*;
        match self {
            // ---------------- Schema ----------------
            E0201 => RuleEntry {
                code: self,
                tier: Schema,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.1a",
                gloss: "A value in ply.yaml doesn't match the shape Ply expects there -- the wrong type, the wrong structure, or a schema version Ply doesn't understand -- and is reported before any other check runs, because a document this malformed can't be trusted for what comes after.",
            },
            E0202 => RuleEntry {
                code: self,
                tier: Schema,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.1",
                gloss: "Two components declared in this project share the same name, so a rule meant for one could end up applying to the other; component names must be unique.",
            },
            E0203 => RuleEntry {
                code: self,
                tier: Schema,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.1a",
                gloss: "A short-form line in ply.yaml (an edge, a deny rule, or similar) doesn't match the exact form Ply parses for it, so Ply names the form it expected rather than silently treating a typo as doing nothing.",
            },
            E0204 => RuleEntry {
                code: self,
                tier: Schema,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.1a",
                gloss: "ply.yaml uses a key Ply doesn't recognize at that spot in the document -- almost always a typo, since a silently-ignored key is a rule you think you wrote that Ply never read -- and Ply names the closest real key it thinks you meant.",
            },
            E0205 => RuleEntry {
                code: self,
                tier: Schema,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.1a",
                gloss: "Two entries under `unresolved:` share the same id, so a later message pointing at 'marker 3' couldn't say which of the two it means; each marker needs an id of its own.",
            },
            E0206 => RuleEntry {
                code: self,
                tier: Schema,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.1a",
                gloss: "A name in an edge, a deny rule, or a reference could mean more than one thing -- for example it matches both a declared external and a same-named component -- and Ply lists every candidate rather than guess which one you meant.",
            },
            E0207 => RuleEntry {
                code: self,
                tier: Schema,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.3",
                gloss: "An edge or a deny rule treats something Ply has declared as external (code outside this project) as a normal target it can verify calls into -- but Ply cannot check code it cannot see, so this needs the separate, declared-not-checked line form instead.",
            },
            E0208 => RuleEntry {
                code: self,
                tier: Schema,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.3",
                gloss: "A declared-not-checked line connects two things this document has declared external to each other, describing two outside systems talking to one another -- which isn't this project's own document's business to declare.",
            },
            E0209 => RuleEntry {
                code: self,
                tier: Schema,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.1a",
                gloss: "An edge or a flow line names an external that was never declared under `externals:` in this document, so there's nothing for the reference to resolve to.",
            },
            E0504 => RuleEntry {
                code: self,
                tier: Schema,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.1",
                gloss: "A function's `checks:` list asks for mutation testing (`mutate`) without also asking for `test` or `fuzz` in the same list, but `mutate` needs one of those two to measure itself against and cannot run alone.",
            },
            W0409 => RuleEntry {
                code: self,
                tier: Schema,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.3",
                gloss: "An edge is declared between a component and its own nested child, but nesting already grants full permission to call between the two, so the edge does nothing and the diagram draws nothing for it either.",
            },
            W0410 => RuleEntry {
                code: self,
                tier: Schema,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.1",
                gloss: "A declared external doesn't appear in this document's `entry:` list, so nothing here says how the rest of the project actually reaches it -- the external is declared, just disconnected.",
            },

            // ---------------- Anchor ----------------
            E0301 => RuleEntry {
                code: self,
                tier: Anchor,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.2",
                gloss: "Ply could not find the real function a claim is supposed to attach to -- most often because it was renamed, moved, or deleted after the claim was written -- and suggests the nearest real name it can find.",
            },
            E0304 => RuleEntry {
                code: self,
                tier: Anchor,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.1a",
                gloss: "A path naming which function or component a claim is about uses a form Ply's path reader doesn't accept -- generics, lifetimes, or a trait-qualified path -- where only a plain `module::item` path is allowed.",
            },
            E0306 => RuleEntry {
                code: self,
                tier: Anchor,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.2",
                gloss: "A `Type::method` reference matches more than one real function, and Ply refuses to guess which one a claim means, since attaching the verdict to the wrong one would be worse than attaching it to none.",
            },

            // ---------------- Crate ----------------
            A0401 => RuleEntry {
                code: self,
                tier: Crate,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.3",
                gloss: "One crate in this workspace depends on another whose component this document has not given it permission to reach, checked against the exact, real dependency graph rather than a guess -- this finding is never a false alarm.",
            },
            A0405 => RuleEntry {
                code: self,
                tier: Crate,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.3",
                gloss: "A real crate dependency matches a `deny:` rule that forbids it, checked against the same exact dependency graph as A0401.",
            },
            A0409 => RuleEntry {
                code: self,
                tier: Crate,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.3",
                gloss: "Ply could not check any crate-level boundary at all, because it could not get this workspace's real dependency graph -- a broken Cargo.toml, `cargo` missing, or a dependency cycle -- and reports this honestly as a run that did not look, never as a clean pass.",
            },
            W0413 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.1",
                gloss: "A component declares `state:` and Ply could not resolve it -- a workspace-root document, or a component anchored at a crate whose source is not here. The claim is not wrong, it is unverified, and a claim nobody looked at must never read like one that passed.",
            },
            A0414 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.1",
                gloss: "A component says its state lives in a type this crate does not declare. `state:` earns its keep by being unable to lie about the code, so a type nobody wrote is a finding rather than a box with a name in it.",
            },
            A0415 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.1",
                gloss: "A component asks to show a field its state type does not declare. The finding names the fields that do exist, so a typo takes one guess to fix rather than three.",
            },
            A0410 => RuleEntry {
                code: self,
                tier: Crate,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.3",
                gloss: "A component's anchor names a crate that Ply cannot find anywhere in this workspace's real dependency graph, so the component owns no crate and every rule written for it is silently doing nothing -- usually a typo or a rename.",
            },
            A0411 => RuleEntry {
                code: self,
                tier: Crate,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.3",
                gloss: "Two different components are anchored at the same crate; Ply keeps only the one declared first, so every edge or deny rule written for the second never fires until this is fixed.",
            },
            A0412 => RuleEntry {
                code: self,
                tier: Crate,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.3",
                gloss: "Two different crates in this workspace both build a library with the same name, so Ply cannot tell which one a component's anchor -- or a real dependency -- actually means, and checks nothing involving that name until one library is renamed.",
            },
            A0413 => RuleEntry {
                code: self,
                tier: Crate,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.3",
                gloss: "An edge or deny rule names a component that isn't declared anywhere in this document, so the line can never match anything and is silently doing nothing -- check the spelling against the document's real component names.",
            },

            // ---------------- Item (none built yet) ----------------
            A0402 => RuleEntry {
                code: self,
                tier: Item,
                status: DeclaredOnly,
                severity: Warning,
                spec_anchor: "§5.3",
                gloss: "Planned: a function call would cross between two declared components with no edge permitting it, found from an approximate, syntax-based call graph rather than the exact crate graph -- not built yet.",
            },
            A0403 => RuleEntry {
                code: self,
                tier: Item,
                status: DeclaredOnly,
                severity: Warning,
                spec_anchor: "§5.3",
                gloss: "Planned: a component marked `pure` (meant to have no side effects) touches a capability such as the network, the filesystem, the clock, or randomness -- not built yet.",
            },
            A0404 => RuleEntry {
                code: self,
                tier: Item,
                status: DeclaredOnly,
                severity: Warning,
                spec_anchor: "§5.3",
                gloss: "Planned: a component reaches a capability outside the set it declared, through its own code rather than through a declared edge into a component that has it -- not built yet.",
            },
            A0406 => RuleEntry {
                code: self,
                tier: Item,
                status: DeclaredOnly,
                severity: Warning,
                spec_anchor: "§5.3",
                gloss: "Planned: code outside the component that declared `owns` on a type mutates that type anyway -- not built yet.",
            },
            A0407 => RuleEntry {
                code: self,
                tier: Item,
                status: DeclaredOnly,
                severity: Error,
                spec_anchor: "§5.3",
                gloss: "Planned: code violates one of this project's profile bans, a syntactic rule about what a component's code may contain -- always meant to be an error, with no advisory form, but not built yet.",
            },
            A0408 => RuleEntry {
                code: self,
                tier: Item,
                status: DeclaredOnly,
                severity: Error,
                spec_anchor: "§5.4a",
                gloss: "Planned: a `#[ply::pure]` helper used inside a contract touches a capability, breaking the promise that a pure helper has no side effects -- always meant to be an error regardless of strict mode, but not built yet.",
            },
            W0411 => RuleEntry {
                code: self,
                tier: Item,
                status: DeclaredOnly,
                severity: Warning,
                spec_anchor: "§5.3",
                gloss: "Planned: a call goes through a trait object (dynamic dispatch) whose real implementations live in a component the caller may not be allowed to reach -- not built yet.",
            },
            W0412 => RuleEntry {
                code: self,
                tier: Item,
                status: DeclaredOnly,
                severity: Warning,
                spec_anchor: "§5.3 (D11)",
                gloss: "Planned: a call site Ply's extractor could not resolve has a plausible textual match that would need an undeclared edge -- today an unresolved call site is only counted, not named this specifically, so this is not built yet.",
            },

            // ---------------- Contract ----------------
            E0501 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.4a",
                gloss: "A `requires`/`ensures` clause uses a construct outside the small, checkable expression subset Ply supports -- indexing, most method calls, a closure, and more -- and Ply names the exact construct rather than silently skip it.",
            },
            E0502 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.5",
                gloss: "A promise declared for a function this proof calls into can never be true of anything -- Ply searched every value and found none that satisfies it -- so Ply refuses to run the proof rather than let an impossible assumption make it pass for the wrong reason.",
            },
            E0503 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.5",
                gloss: "A `requires` clause on a function this proof calls into ranges over a type with no values in it at all, so a proof resting on that empty precondition would prove nothing.",
            },
            W0502 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.4c",
                gloss: "Mutation testing found small changes to this function's code that none of its checks noticed, meaning the spec doesn't pin the behaviour down tightly enough to catch that class of bug -- the number of such changes that survived is included.",
            },
            W0503 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.4c",
                gloss: "The random-sample check gave up generating inputs for this function before it reached the case count asked for, because too many generated values were rejected by the function's own precondition -- so no evidence was earned at all, not a smaller passing count.",
            },
            E0505 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.4",
                gloss: "A requires: or ensures: line written in ply.yaml could not be read as Rust, so it cannot be checked -- the line is quoted back with what was expected, rather than dropped, because a promise nobody checks is worse than one nobody wrote.",
            },
            W0511 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.5",
                gloss: "This proof rests on an assumed promise for at least one function it calls, rather than that function's own checked behaviour -- every assumed function is named, and the verdict is conditional on those assumptions actually holding.",
            },
            W0512 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.5",
                gloss: "A function this proof calls into carries no checked promise of its own -- nobody has vouched for what it does -- so Ply refuses to assume anything about it and the proof does not run.",
            },
            W0513 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.5",
                gloss: "A call inside this proof goes through a glob or wildcard Ply cannot resolve to one specific function, so it can be neither assumed nor verified -- reported rather than silently skipped over.",
            },
            W0514 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.5",
                gloss: "A `requires` clause meant to narrow which values are considered turned out to rule out every value once combined with the rest of the proof, so this is reported as unchecked rather than as a pass that would mean nothing.",
            },
            W0515 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.5",
                gloss: "This proof earned no evidence for a reason specific to how it composes with the functions it calls, spelled out in the message rather than left for the reader to guess from the verdict alone.",
            },
            W0516 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.2a",
                gloss: "A previously recorded result for this exact check no longer matches what Ply can verify about it today, so the old record is refused rather than trusted, and the check runs again from scratch.",
            },
            W0517 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Info,
                spec_anchor: "§5.5",
                gloss: "Names every other function's proof, and the bound it was checked at, that this proof's own result depends on -- informational, not a problem, just a disclosure of what a passing result here is actually resting on.",
            },
            V0505 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.4b",
                gloss: "None of Ply's check engines can build test inputs for one of this function's parameter or return types, so it is reported as unsupported, by name, rather than the check silently hanging or being skipped without saying why.",
            },
            V0506 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.4a",
                gloss: "A postcondition reads a by-value parameter's original value, but that parameter has already been consumed by the call itself by the time the postcondition runs, so there is no value left to read -- Ply refuses to generate a test that could not compile.",
            },
            V0507 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.2",
                gloss: "A claim's anchor names a real function -- a method with a receiver, a trait method, or an item inside a generic implementation -- that this version of Ply declines to check; the function exists, Ply simply doesn't attempt this shape yet.",
            },
            V0508 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.4b",
                gloss: "This function's types work fine for a random-sample check but are too costly, or deliberately excluded (as with floating-point), for the exhaustive check specifically -- named so the function still reads as checked by whichever check does cover it.",
            },
            V0509 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.4b",
                gloss: "A parameter's type is a real struct or enum Ply found in this crate, but Ply could not build a value of it -- no usable constructor, a private field in the way, or a nested type it can't build either -- and the specific reason is named.",
            },
            V0510 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.4b",
                gloss: "A promise reads a private field of the value this function returns, but the generated code for this kind of check lives in a separate crate that cannot see private fields of your type -- refused up front rather than left to fail as an unexplained compiler error.",
            },
            W0518 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Info,
                spec_anchor: "§5.4c",
                gloss: "This function was checked with randomly generated floating-point values, and Ply leaves out NaN and infinity by default because comparisons involving NaN almost always look broken even when nothing is actually wrong -- informational, but this run says nothing about NaN or infinity specifically.",
            },
            W0519 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Info,
                spec_anchor: "§5.4c",
                gloss: "This function takes no input at all, so there was exactly one possible call to make -- Ply made it and it held, reported as tested rather than as a random-sample count, since a bigger number would not have looked at anything new.",
            },
            W0520 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.4c",
                gloss: "A random-sample check on a method needed a value to call it on, so Ply built one itself and made a bounded number of random calls to it before the checked call -- names exactly what was built and called, and what, if anything, was left out.",
            },
            W0521 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Info,
                spec_anchor: "§5.4c",
                // NOTE (found while building this table, 2026-08-31): The-Ply-Spec.md
                // §5.6 also names `W0521`, for a completely different rule -- the cap
                // that limits a function containing `unresolved!()` to check `test`.
                // That rule is real and still unbuilt (`crates/ply-cli/src/worklist.rs`
                // says so in its own words: "Ply does not apply that cap yet"), but it
                // is NOT what fires under this code today. The only real construction
                // site for the literal code "W0521" is `string_sampling_diag` in
                // `crates/ply-cli/src/verify.rs`, which this row describes. One code
                // number is carrying two unrelated rules -- the spec's §5.6 sentence
                // about `W0521` is describing a rule that has no emitting site under
                // that code, and should be corrected or given a different code. Left
                // exactly as found: fixing it is a spec/behaviour decision, not part of
                // building this table.
                gloss: "A random-sample check on a function taking a string built that string by sampling characters, and by default excludes certain control characters the same way NaN is excluded for floats -- informational disclosure of what was and wasn't generated.",
            },
            W0522 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Info,
                spec_anchor: "§5.4b",
                gloss: "A random-sample check built a struct or enum parameter by filling in its already-public fields directly, assuming that can never violate a relationship the type's own methods maintain between them -- that assumption is disclosed, not proved.",
            },
            W0523 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Info,
                spec_anchor: "§5.4c",
                gloss: "A random-sample check on a value built by parsing text (a version, an identifier, a URL) grew its inputs from a pool of already-valid values -- some written by hand as examples, some accepted by the parser itself during this run -- instead of guessing text uniformly, because almost none of that would ever parse. The case count is real, but the inputs are drawn from near what is already known to work, not from the whole space of text.",
            },
            W0524 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Info,
                spec_anchor: "§5.4c",
                gloss: "A random-sample check on a plain parameter Ply's ordinary generator cannot build at all (an optional or list-shaped value wrapping text, say) grew its inputs from example value(s) written by hand, varied by trying different text, instead of being refused outright. The case count is real, but the inputs are drawn from near what is already known, not from the whole range of possible values.",
            },
            W0525 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.4a",
                gloss: "A function declares worked `examples:`, but none of its declared checks actually runs them -- only the `test` check compiles an example into a real assertion. The examples are still read and recorded as part of what this claim's result depends on, so editing one still re-checks the function, but nothing ever ran it against the promise.",
            },
            W0526 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.4c",
                gloss: "A random-sample check's postcondition is written as an \"either this, or that\" promise (`||`), and Ply reports which side of it decided each case that held -- always, whether the split is balanced or not. Severity rises from an info-level disclosure to this warning only when one side decided more than half of every case, the same threshold the high-rejection warning already uses: real cases ran and the promise really held, but one side of it was barely exercised.",
            },
            W0527 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.4b",
                gloss: "A value passed to this function was built by calling a function this project's own configuration names (a declared \"route\" to a type Ply has no other way to build) -- Ply counted how many genuinely different values that call produced across every case it ran, and reports the count always. Severity rises from an info-level disclosure to this warning only when every case built the exact same value: real cases ran, but the named function never produced more than one result, which is as good as testing nothing new. Where the value cannot even be printed for comparison, Ply says plainly that it could not count at all rather than guessing a number.",
            },
            W0528 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.4b",
                gloss: "A `routes:` entry names a function to build a value of some type, but no function or field anywhere in this crate ever needs one -- so nothing checked whether the declaration actually works. Ply validates it on its own once the run is otherwise done, and this names what went wrong: every declared route is used or refused by name, never merely unmentioned.",
            },
            W0529 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.4c",
                gloss: "A constructor taking no arguments has nothing to vary, so every case ran against the same value however many cases ran -- `fuzzed(256)` on one value is one test run 256 times. Ply already refuses to build through `T::default()` for exactly this reason; this is the same rule for an inherent `new()` that takes nothing, said rather than assumed.",
            },
            W0541 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Error,
                spec_anchor: "§8 (D7)",
                gloss: "A failing input was found, but Ply could not also render it as an ordinary Rust test that fails the same way -- often because the failure only exists thanks to a stubbed-out assumption about another function -- so the raw evidence is kept and Ply says why the friendly replay test is missing.",
            },
            W0110 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§3",
                gloss: "One of the external tools a check needs (the exhaustive prover, the random-sample fuzzer, the mutation tester, ...) isn't installed, so that check could not run at all -- never counted as a failed or a passed check, just an absence of evidence.",
            },
            W0111 => RuleEntry {
                code: self,
                tier: Contract,
                status: DeclaredOnly,
                severity: Warning,
                spec_anchor: "§3",
                gloss: "Planned: a function's contract is written both inline in the code and in ply.yaml and the two disagree; ply.yaml is meant to win and Ply is meant to say so, but nothing detects or reports the conflict yet.",
            },
            W0303 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.2",
                gloss: "Ply read this claim's declared promise but did not run its checks, because doing so needs a crate outside the one this run was asked to check -- checks run one crate at a time -- and names the crate the claim actually lives in.",
            },
            W0531 => RuleEntry {
                code: self,
                tier: Contract,
                status: DeclaredOnly,
                severity: Warning,
                spec_anchor: "§5.7",
                gloss: "Planned: a function marked as derived (its body auto-generated from its own contract) was hand-edited afterward, so its recorded body hash no longer matches -- meant to fall back to an ordinary checked function with this warning, but the experimental derive mode this belongs to isn't built yet.",
            },
            K0502 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Error,
                spec_anchor: "§8",
                gloss: "The exhaustive check searched every possible value and found one that breaks this function's contract, with a concrete counterexample.",
            },
            K0601 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§6",
                gloss: "The exhaustive check ran out of time before it could finish searching every possible value -- reported as a timeout, never as a pass or a violation, with raising the time budget or lowering the bound as the first things to try.",
            },
            M0601 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.4c",
                gloss: "Mutation testing ran out of time before it could finish checking this function, reported as a timeout rather than folded into the weak-spec count.",
            },
            P0502 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.4c",
                gloss: "The random-sample check found an input that breaks this function's own postcondition -- or made it panic before the postcondition could even run -- and shrank it down to the smallest example that still fails: a real, reproduced violation with a witness.",
            },
            P0601 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.4c",
                gloss: "The random-sample check ran out of time before finishing the number of cases asked for -- reported as a timeout, never as a violation, even if some cases had already passed.",
            },
            R0502 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.4c",
                gloss: "One or more of this function's own concrete example tests, or Ply's own generated boundary-case tests, failed -- each is a specific input checked directly against the contract, so this is a real, reproduced violation rather than a probabilistic one.",
            },
            R0601 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.4c",
                gloss: "This function's example and generated boundary-case tests did not finish within their time budget -- reported as a timeout, never as a violation.",
            },
            X0901 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.4c",
                gloss: "The generated check code for this function failed to compile, or otherwise could not even start running -- most often because a hand-written example entry doesn't type-check -- reported with the compiler's own first error attached, and never counted as a pass or a violation, because no evidence exists either way.",
            },
            W0414 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.1",
                gloss: "A component says what must always be true of the structure it holds, but that structure lives in another crate and checks run one crate at a time -- so nothing here built one or tried the promise against it, and this run says nothing about whether it holds.",
            },
            W0415 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.1",
                gloss: "A component says what must always be true of the structure it holds, and no struct or enum by that name is declared anywhere in this crate -- so there was nothing to build and nothing to check the promise against.",
            },
            W0416 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.1",
                gloss: "A component says what must always be true of the structure it holds, and Ply has no way to make one of those values -- a promise about a structure is checked by building one through the type's own constructor and calling the type's own operations on it, and that could not start.",
            },
            E0506 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.1",
                gloss: "One of the things a component says must always be true of the structure it holds could not be read as an expression about that value, so none of them were checked -- checking only the readable ones and reporting that as checked is the failure this refuses.",
            },
            V0511 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Error,
                spec_anchor: "§5.1",
                gloss: "A structure does not always hold what the document says it holds: Ply built a value through the type's own constructor, called only the type's own public operations on it, and reached a state the promise says is impossible -- so anything resting on that promise was resting on something untrue.",
            },
            X0903 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.1",
                gloss: "The check for what a structure promises about itself ran out of time, so this run does not know whether the promise holds -- never reported as holding, because a check that did not finish is not a check that passed.",
            },
            W0417 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.1",
                gloss: "The check for what a structure promises about itself ran and never managed to build a single value -- the type's only way in turned every generated value away -- so no promise was ever tried, and this is reported as no evidence rather than as a pass.",
            },
            W0418 => RuleEntry {
                code: self,
                tier: Contract,
                status: Enforced,
                severity: Warning,
                spec_anchor: "§5.1",
                gloss: "What a structure promises about itself was checked against fewer values than this tier asks for, because the type's own way in turned the rest away -- the verdict counts what really ran, and this names the gap between that and what was asked for, along with any operation or second constructor the run never reached.",
            },
            X0902 => RuleEntry {
                code: self,
                tier: Contract,
                status: DeclaredOnly,
                severity: Error,
                spec_anchor: "§9",
                gloss: "Planned: one of Ply's own internal correctness checks failed -- a rendered replay test that should fail didn't, or a stored counterexample didn't replay under the real proof engine -- pointing at a bug in Ply itself rather than in the code Ply is checking; not yet wired into the full test suite.",
            },
        }
    }
}

/// Every row, in [`Code::ALL`] order. Cheap to call (no allocation beyond
/// the returned `Vec` itself, and there are fewer than a hundred rows), so
/// callers needing the whole table just call this rather than caching it.
/// The letter every code starts with, and what it says about who is
/// speaking. Written down here because it was not written down anywhere: a
/// reader with `K0502` in their terminal had no way to learn that the `K`
/// means the exhaustive prover found it, short of reading this file.
pub fn family(code: Code) -> &'static str {
    match code.entry().letter() {
        'E' => "Ply reading your document, or looking for the code a claim names",
        'A' => "Ply checking the architecture rules your document declares",
        'W' => "any part of Ply, as a warning rather than a stop",
        'V' => "Ply itself, refusing or qualifying a check before an engine ran",
        'K' => "the exhaustive prover (Kani)",
        'P' => "the random sampler (proptest)",
        'R' => "a plain test run",
        'M' => "the mutation tester",
        'X' => "Ply's own tooling, when it broke rather than found something",
        _ => "Ply",
    }
}

impl RuleEntry {
    /// The code's leading letter.
    pub fn letter(&self) -> char {
        format!("{:?}", self.code).chars().next().unwrap_or('?')
    }
}

/// The row for a code written the way a user types it (`K0502`), or `None`
/// when no such code exists.
///
/// Case-insensitive, because a code copied out of a terminal and retyped by
/// hand is as likely to arrive lowercase as not, and refusing that would be
/// a refusal about typing rather than about the code.
pub fn lookup(code: &str) -> Option<RuleEntry> {
    let wanted = code.trim().to_ascii_uppercase();
    Code::ALL
        .iter()
        .find(|c| format!("{c:?}") == wanted)
        .map(|c| c.entry())
}

pub fn all() -> Vec<RuleEntry> {
    Code::ALL.iter().map(|c| c.entry()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one property a hand-copied match block could get wrong without
    /// the compiler ever noticing: that a row's own `code` field actually
    /// names the arm it sits in, rather than a copy-pasted neighbour.
    #[test]
    fn every_row_s_code_field_matches_its_own_match_arm() {
        for &code in Code::ALL {
            let entry = code.entry();
            assert_eq!(
                entry.code, code,
                "Code::{code:?}'s row carries code {:?} -- looks like a copy-pasted neighbour",
                entry.code
            );
        }
    }

    #[test]
    fn debug_format_is_the_bare_code_string() {
        assert_eq!(format!("{:?}", Code::E0204), "E0204");
        assert_eq!(format!("{:?}", Code::A0401), "A0401");
    }

    #[test]
    fn all_lists_every_variant_exactly_once() {
        let mut seen = std::collections::HashSet::new();
        for &code in Code::ALL {
            assert!(
                seen.insert(code),
                "Code::{code:?} appears twice in Code::ALL"
            );
        }
        // A ballpark, not a pin: catches Code::ALL silently losing entries
        // without hard-coding the exact count here too.
        assert!(
            Code::ALL.len() > 60,
            "expected on the order of 70 codes, got {}",
            Code::ALL.len()
        );
    }
}
