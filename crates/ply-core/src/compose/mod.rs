//! Composing function proofs into a named component property.
//!
//! This module is **pure**: it spawns nothing, writes nothing, renders
//! nothing. It takes premises — what each function's proof licenses — and
//! produces the obligations that must close before the component property
//! may be claimed, plus the reasons a complete proof is withheld.
//!
//! The theorem it plans for, stated in full in
//! `docs/component-proof-design.md`:
//!
//! > Every state reachable from a **covered** constructor by finitely many
//! > **permitted**, **normally-returning** operations satisfies `I`.
//!
//! Three words bound that claim rather than extend it. *Permitted* means
//! operations are invoked only where their preconditions hold — this proves
//! the invariant under that discipline, not that any caller obeys it.
//! *Normally-returning* excludes a transition that panics partway.
//! *Covered* means Ply accounted for the construction path; an unaccounted
//! one voids the theorem rather than weakening it.
//!
//! Concurrency and re-entrancy are outside this theorem entirely.

pub mod inventory;
pub mod verus;

use std::collections::BTreeSet;

/// What a value's declared Rust type licenses us to assume about it.
///
/// Carried explicitly, and never inferred from silence. Modelling a `u32`
/// observer as a mathematical integer is fine; modelling it as an
/// *unbounded* one is not, so the width travels as a premise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeFacts {
    /// A machine integer, with the exact bounds of its declared type.
    Integer {
        rust_type: String,
        min: i128,
        max: i128,
    },
    Bool,
    /// A type whose value range this module will not guess. Fails closed:
    /// an observer of this shape blocks the property rather than being
    /// quietly admitted with no constraints.
    Unsupported {
        rust_type: String,
    },
}

impl RangeFacts {
    /// The bounds of a declared integer type, or `Unsupported`.
    ///
    /// `pointer_width_bits` is the **target's** width, not the host's, and
    /// it is an `Option` on purpose: `usize` and `isize` have no fixed
    /// range, and guessing 64 is wrong on a 32-bit target in the direction
    /// that matters -- it hands the solver a wider range than the program
    /// has, so an overflow the program really suffers looks impossible.
    /// Unknown width means `Unsupported`, which blocks. (Hardcoded to 64
    /// until 2026-09-09; found by review.)
    pub fn of_rust_type(name: &str, pointer_width_bits: Option<u32>) -> Self {
        let ptr = |signed: bool| -> Option<(i128, i128)> {
            match (pointer_width_bits?, signed) {
                (16, false) => Some((0, u16::MAX as i128)),
                (32, false) => Some((0, u32::MAX as i128)),
                (64, false) => Some((0, u64::MAX as i128)),
                (16, true) => Some((i16::MIN as i128, i16::MAX as i128)),
                (32, true) => Some((i32::MIN as i128, i32::MAX as i128)),
                (64, true) => Some((i64::MIN as i128, i64::MAX as i128)),
                _ => None,
            }
        };
        let bounds: Option<(i128, i128)> = match name {
            "u8" => Some((0, u8::MAX as i128)),
            "u16" => Some((0, u16::MAX as i128)),
            "u32" => Some((0, u32::MAX as i128)),
            "u64" => Some((0, u64::MAX as i128)),
            "usize" => ptr(false),
            "i8" => Some((i8::MIN as i128, i8::MAX as i128)),
            "i16" => Some((i16::MIN as i128, i16::MAX as i128)),
            "i32" => Some((i32::MIN as i128, i32::MAX as i128)),
            "i64" => Some((i64::MIN as i128, i64::MAX as i128)),
            "isize" => ptr(true),
            "bool" => return RangeFacts::Bool,
            _ => None,
        };
        match bounds {
            Some((min, max)) => RangeFacts::Integer {
                rust_type: name.to_string(),
                min,
                max,
            },
            None => RangeFacts::Unsupported {
                rust_type: name.to_string(),
            },
        }
    }
}

/// A reading the theorem is written in terms of: a `&self` method or field
/// whose value the invariant and the transition contracts mention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observer {
    pub name: String,
    pub facts: RangeFacts,
    /// True only where an effect analysis established that reading this
    /// observer cannot change the state.
    ///
    /// **`&self` is not evidence of this.** Interior mutability and shared
    /// aliases can mutate through a shared reference, so a reader that has
    /// not been analysed is `false` and blocks rather than passes.
    pub reads_are_pure: bool,
}

/// What the source says *today*, for one item. A premise proved against
/// different bytes is stale, not weaker.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceIdentity {
    pub source_fingerprint: String,
    pub contract_fingerprint: String,
    pub config: String,
}

/// Why a post-state fact about an observer is believed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameJustification {
    /// The operation's own proved contract states it.
    ProvedContract,
    /// An effect analysis established it, with its reason recorded.
    EffectAnalysis { reason: String },
}

/// "After this operation, this observer still reads what it read before."
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameFact {
    pub observer: String,
    pub justification: FrameJustification,
}

/// How far a proof reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Domain {
    /// Proved for every input of the declared types.
    Unrestricted,
    /// Proved only over a stated restriction. A premise like this cannot
    /// license an unrestricted conclusion.
    Restricted { description: String },
}

/// Where a premise's authority comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provenance {
    /// An engine discharged it.
    Proved {
        engine: String,
        version: String,
        artifact: String,
    },
    /// The author declared it true. Legitimate, and it makes every
    /// conclusion resting on it conditional — visibly, by name.
    Trusted { declared_by: String },
}

/// What kind of thing this premise describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PremiseRole {
    /// Produces a fresh value of the state type.
    Constructor,
    /// May change the state.
    Transition,
    /// Reads the state.
    Reader,
}

/// A value an operation takes in, or hands back, that its contract talks
/// about.
///
/// It needs a declared range for the same reason a reading does: in the
/// model it is a variable the solver chooses, and an unbounded choice is a
/// wider program than the one that was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parameter {
    pub name: String,
    pub facts: RangeFacts,
}

/// One function proof, in the form composition can actually use.
///
/// "This function is proved" is not usable. Composition needs the contract
/// it was proved *against*, the domain it was proved *over*, and enough
/// identity to tell whether it still describes today's source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Premise {
    /// Canonical item path, e.g. `TokenBucket::try_take`.
    pub item: String,
    pub role: PremiseRole,
    /// Fingerprint of the source this was proved against.
    pub source_fingerprint: String,
    /// Fingerprint of the contract itself.
    pub contract_fingerprint: String,
    /// The compilation configuration the proof was taken under.
    pub config: String,
    pub domain: Domain,
    /// Every name the contract uses that is not a reading: the arguments,
    /// and the returned value where the contract mentions it. Each carries
    /// its declared range.
    pub parameters: Vec<Parameter>,
    /// Preconditions, as written.
    pub requires: Vec<String>,
    /// Relational postconditions, as written, over `old(..)` and the
    /// post-state.
    pub ensures: Vec<String>,
    /// Which observers this operation leaves alone, and why.
    pub frame: Vec<FrameFact>,
    /// Anything the proof rested on and did not discharge.
    pub assumptions: Vec<String>,
    pub provenance: Provenance,
}

/// The component property being established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    /// A name a person can read in a report.
    pub name: String,
    /// The invariant, as written in `holds:`.
    pub invariant: String,
    /// The state type it is about.
    pub state_type: String,
    /// Every observer the invariant and the contracts mention.
    pub observers: Vec<Observer>,
}

/// The complete inventory of ways this type can be built or changed, as
/// established by the boundary analysis — **not** by the sampled operation
/// pool, which excludes what it cannot construct arguments for and drops
/// some shapes with no record at all.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Inventory {
    pub constructors: Vec<String>,
    pub mutators: Vec<String>,
    /// Paths that can change the state without going through a method:
    /// a public field, an escaped `&mut`, a free function in the same
    /// module reaching into it.
    pub escapes: Vec<String>,
    /// Shapes the scan could not classify. Non-empty means the boundary is
    /// open and no complete property proof is available.
    pub unclassified: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObligationKind {
    /// Every state the constructor can produce satisfies `I`.
    Initialization { constructor: String },
    /// `I(s)` and the operation's contract imply `I(s')`.
    Preservation { operation: String },
    /// The operation's premises are satisfiable at all. A contradictory
    /// premise set discharges everything and means nothing, and this is not
    /// decidable here -- so it is asked of the backend rather than assumed.
    Reachable { operation: String },
    /// Every arithmetic operation in the translated contract stays within
    /// the declared ranges. A contract is a Rust expression, and the
    /// prover's integers are not Rust's: without this, an implementation
    /// using wrapping arithmetic satisfies its promise while breaking the
    /// invariant, and the model verifies anyway (measured 2026-09-09).
    ArithmeticSafety { item: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Obligation {
    pub kind: ObligationKind,
    /// Which premises this obligation is entitled to assume.
    pub premises: Vec<String>,
}

/// A reason the property cannot be claimed complete, whatever the solver
/// says about the obligations that *were* generated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blocker {
    /// A construction path with no premise covering it.
    UncoveredConstructor { item: String },
    /// A mutator with no premise covering it. The measured failure mode:
    /// a proof over a set that omits one mutator says nothing about the
    /// type, and reads as stronger than the sampling it replaces.
    UncoveredMutator { item: String },
    /// The state can change without passing through any operation.
    OpenBoundary { path: String },
    /// The scan could not classify something, so the inventory is not known
    /// to be complete.
    UnclassifiedPath { path: String },
    /// An operation says nothing about an observer the invariant needs, and
    /// nothing justifies treating it as unchanged.
    UnjustifiedFrame { operation: String, observer: String },
    /// An observer whose declared type has no stated value range.
    UnsupportedObserver { observer: String, rust_type: String },
    /// A premise proved only over a restriction, used where the conclusion
    /// would be unrestricted.
    RestrictedDomain { item: String, description: String },
    /// A reader that has not been shown to leave the state alone.
    UnprovenReaderPurity { observer: String },
    /// Two premises for the same item. Which one wins would otherwise
    /// depend on iteration order, so the same inputs could give two
    /// verdicts.
    DuplicatePremise { item: String },
    /// A premise proved against source, contract text or configuration
    /// that is not what is there now.
    StalePremise { item: String, field: String },
    /// A premise standing in for something it is not: a reader's contract
    /// is not a two-state contract.
    WrongRole {
        item: String,
        expected: String,
        found: String,
    },
    /// A frame fact citing the operation's contract as its justification,
    /// where the contract does not in fact state it.
    UnsupportedFrameClaim { operation: String, observer: String },
    /// A premise for an item the boundary scan never found -- direct
    /// evidence the inventory is incomplete.
    PremiseOutsideInventory { item: String },
    /// No construction path at all. A type with no constructors has no
    /// reachable states; a scan that returned nothing is likelier.
    NoConstructors,
    /// The invariant names a reading that was never declared as an
    /// observer, so the frame check has nothing to look for.
    UndeclaredObserver { observer: String },
    /// A parameter whose declared type has no stated value range. Left in,
    /// the solver ranges over values the program cannot produce.
    UnsupportedParameter {
        item: String,
        parameter: String,
        rust_type: String,
    },
    /// A name a contract uses that is neither a declared reading nor a
    /// declared parameter, so nothing in the model bounds it.
    UndeclaredName { item: String, name: String },
}

/// The plan: what must be discharged, what is assumed, and what stops the
/// property being complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub property: Property,
    pub obligations: Vec<Obligation>,
    pub blockers: Vec<Blocker>,
    /// Premises taken on trust rather than proved. Non-empty means any
    /// success is conditional, and these are the names it is conditional on.
    pub trusted: Vec<String>,
    /// Undischarged assumptions carried up from the premises.
    pub assumptions: Vec<String>,
}

impl Plan {
    /// Whether a complete property proof is available at all, before the
    /// solver is asked anything.
    pub fn coverage_is_complete(&self) -> bool {
        self.blockers.is_empty()
    }

    /// Whether success would be conditional on trust.
    pub fn is_conditional(&self) -> bool {
        !self.trusted.is_empty()
    }
}

/// Plan the obligations for one component property.
///
/// Pure. Generates what must be discharged and, separately, every reason
/// the property cannot be claimed complete regardless of what a solver
/// returns about the obligations.
pub fn plan(
    property: &Property,
    inventory: &Inventory,
    premises: &[Premise],
    current: &std::collections::BTreeMap<String, SourceIdentity>,
) -> Plan {
    let mut obligations = Vec::new();
    let mut blockers = Vec::new();
    let mut trusted = Vec::new();
    let mut assumptions = Vec::new();

    // Built by hand rather than `collect()`: a duplicate must block, not
    // silently overwrite, or the same premise set yields two verdicts
    // depending on order.
    let mut by_item: std::collections::BTreeMap<&str, &Premise> = std::collections::BTreeMap::new();
    for p in premises {
        if by_item.insert(p.item.as_str(), p).is_some() {
            blockers.push(Blocker::DuplicatePremise {
                item: p.item.clone(),
            });
        }
    }

    // A premise for something the scan never found is evidence the
    // inventory is incomplete, not a spare part to discard.
    let known: BTreeSet<&str> = inventory
        .constructors
        .iter()
        .chain(inventory.mutators.iter())
        .map(|s| s.as_str())
        .collect();
    for p in premises {
        if !known.contains(p.item.as_str()) {
            blockers.push(Blocker::PremiseOutsideInventory {
                item: p.item.clone(),
            });
        }
    }

    // Staleness: a premise proved against different bytes describes a
    // different program.
    for p in premises {
        if let Some(now) = current.get(&p.item) {
            for (field, was, is) in [
                ("source", &p.source_fingerprint, &now.source_fingerprint),
                (
                    "contract",
                    &p.contract_fingerprint,
                    &now.contract_fingerprint,
                ),
                ("config", &p.config, &now.config),
            ] {
                if was != is {
                    blockers.push(Blocker::StalePremise {
                        item: p.item.clone(),
                        field: field.to_string(),
                    });
                }
            }
        }
    }

    if inventory.constructors.is_empty() {
        blockers.push(Blocker::NoConstructors);
    }

    // Every reading the invariant names has to be declared, or the frame
    // check below has nothing to look for.
    for name in identifiers(&property.invariant) {
        if !property.observers.iter().any(|o| o.name == name) {
            blockers.push(Blocker::UndeclaredObserver { observer: name });
        }
    }

    // An observer whose range we cannot state is not admitted with no
    // constraints -- that would let the solver assume anything about it.
    for obs in &property.observers {
        if let RangeFacts::Unsupported { rust_type } = &obs.facts {
            blockers.push(Blocker::UnsupportedObserver {
                observer: obs.name.clone(),
                rust_type: rust_type.clone(),
            });
        }
        if !obs.reads_are_pure {
            blockers.push(Blocker::UnprovenReaderPurity {
                observer: obs.name.clone(),
            });
        }
    }

    // The boundary. These do not depend on any premise: an open path means
    // the theorem has no subject, however well the operations are proved.
    for path in &inventory.escapes {
        blockers.push(Blocker::OpenBoundary { path: path.clone() });
    }
    for path in &inventory.unclassified {
        blockers.push(Blocker::UnclassifiedPath { path: path.clone() });
    }

    // Initialization, one per construction path.
    for ctor in &inventory.constructors {
        match by_item.get(ctor.as_str()) {
            Some(p) => {
                note_premise(p, &mut trusted, &mut assumptions, &mut blockers);
                require_role(p, PremiseRole::Constructor, &mut blockers);
                check_names(p, property, &mut blockers);
                obligations.push(Obligation {
                    kind: ObligationKind::Initialization {
                        constructor: ctor.clone(),
                    },
                    premises: vec![ctor.clone()],
                });
                obligations.push(Obligation {
                    kind: ObligationKind::ArithmeticSafety { item: ctor.clone() },
                    premises: vec![ctor.clone()],
                });
            }
            None => blockers.push(Blocker::UncoveredConstructor { item: ctor.clone() }),
        }
    }

    // Preservation, one per mutator, plus the frame check for each.
    let needed: BTreeSet<&str> = property.observers.iter().map(|o| o.name.as_str()).collect();
    for op in &inventory.mutators {
        let Some(p) = by_item.get(op.as_str()) else {
            blockers.push(Blocker::UncoveredMutator { item: op.clone() });
            continue;
        };
        note_premise(p, &mut trusted, &mut assumptions, &mut blockers);
        require_role(p, PremiseRole::Transition, &mut blockers);
        check_names(p, property, &mut blockers);

        // Every observer the invariant needs must be either constrained by
        // this operation's postcondition or justified as unchanged. Saying
        // nothing leaves it unconstrained -- never implicitly the same.
        for obs in &needed {
            let constrained = p.ensures.iter().any(|e| constrains_post_state(e, obs));
            let frame = p.frame.iter().find(|f| f.observer == *obs);
            match frame {
                // A frame fact citing the contract is a claim about the
                // contract, and gets checked against it rather than taken
                // on faith -- otherwise it re-admits the omitted-frame hole
                // through a side door.
                Some(f) if f.justification == FrameJustification::ProvedContract => {
                    if !constrained {
                        blockers.push(Blocker::UnsupportedFrameClaim {
                            operation: op.clone(),
                            observer: (*obs).to_string(),
                        });
                    }
                }
                Some(_) => {}
                None => {
                    if !constrained {
                        blockers.push(Blocker::UnjustifiedFrame {
                            operation: op.clone(),
                            observer: (*obs).to_string(),
                        });
                    }
                }
            }
        }

        for kind in [
            ObligationKind::Preservation {
                operation: op.clone(),
            },
            ObligationKind::Reachable {
                operation: op.clone(),
            },
            ObligationKind::ArithmeticSafety { item: op.clone() },
        ] {
            obligations.push(Obligation {
                kind,
                premises: vec![op.clone()],
            });
        }
    }

    trusted.sort();
    trusted.dedup();
    assumptions.sort();
    assumptions.dedup();

    Plan {
        property: property.clone(),
        obligations,
        blockers,
        trusted,
        assumptions,
    }
}

/// Whether a contract expression constrains this observer's **post-state**
/// value.
///
/// Mentioning it is not enough. `available == old(available) + old(capacity)`
/// mentions `capacity` and says nothing about what it will be; so does a
/// comment, and so does a string. Each of those silenced the frame blocker
/// before 2026-09-09.
pub fn constrains_post_state(expr: &str, observer: &str) -> bool {
    mentions(&strip_old(&strip_noise(expr)), observer)
}

/// Remove line and block comments and string literals -- text that cannot
/// constrain anything.
fn strip_noise(expr: &str) -> String {
    let b: Vec<char> = expr.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '/' && i + 1 < b.len() && b[i + 1] == '/' {
            break;
        }
        if b[i] == '/' && i + 1 < b.len() && b[i + 1] == '*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == '*' && b[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(b.len());
            continue;
        }
        if b[i] == '"' {
            i += 1;
            while i < b.len() && b[i] != '"' {
                if b[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// Remove every `old( .. )` span. What it holds is the *before* value, and
/// a before value constrains nothing about after.
fn strip_old(expr: &str) -> String {
    let mut out = String::new();
    let b: Vec<char> = expr.chars().collect();
    let mut i = 0;
    while i < b.len() {
        let is_old = b[i] == 'o'
            && b[i..].starts_with(&['o', 'l', 'd', '('])
            && (i == 0 || !is_ident_byte(b[i - 1] as u8));
        if is_old {
            let mut depth = 0;
            let mut j = i + 3;
            while j < b.len() {
                if b[j] == '(' {
                    depth += 1;
                } else if b[j] == ')' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                j += 1;
            }
            i = j + 1;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// Every bare identifier in an expression, skipping function names (an
/// identifier immediately followed by `(`) and numeric literals.
fn identifiers(expr: &str) -> Vec<String> {
    let clean = strip_noise(expr);
    let b: Vec<char> = clean.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_alphabetic() || b[i] == '_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '_') {
                i += 1;
            }
            let name: String = b[start..i].iter().collect();
            let called = b.get(i) == Some(&'(');
            if !called && !matches!(name.as_str(), "true" | "false" | "old" | "self") {
                out.push(name);
            }
        } else {
            i += 1;
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Every name a contract uses has to be bounded by something.
///
/// Two ways it is not: a parameter declared with a type this module will
/// not put a range on, and a name declared nowhere at all. Both leave the
/// solver a variable it may choose freely, which is a larger program than
/// the one that was written -- so both block rather than being admitted
/// unconstrained.
fn check_names(p: &Premise, property: &Property, blockers: &mut Vec<Blocker>) {
    for param in &p.parameters {
        if let RangeFacts::Unsupported { rust_type } = &param.facts {
            blockers.push(Blocker::UnsupportedParameter {
                item: p.item.clone(),
                parameter: param.name.clone(),
                rust_type: rust_type.clone(),
            });
        }
    }
    let mut seen = BTreeSet::new();
    for clause in p.requires.iter().chain(p.ensures.iter()) {
        for name in identifiers(clause) {
            let declared = property.observers.iter().any(|o| o.name == name)
                || p.parameters.iter().any(|q| q.name == name);
            if !declared && seen.insert(name.clone()) {
                blockers.push(Blocker::UndeclaredName {
                    item: p.item.clone(),
                    name,
                });
            }
        }
    }
}

/// A premise standing in for something it is not.
fn require_role(p: &Premise, expected: PremiseRole, blockers: &mut Vec<Blocker>) {
    if p.role != expected {
        blockers.push(Blocker::WrongRole {
            item: p.item.clone(),
            expected: format!("{expected:?}"),
            found: format!("{:?}", p.role),
        });
    }
}

/// Whether a contract expression talks about this observer.
///
/// Word-boundary matching, so `capacity` is not found inside
/// `spare_capacity`.
fn mentions(expr: &str, observer: &str) -> bool {
    let bytes = expr.as_bytes();
    let mut from = 0;
    while let Some(at) = expr[from..].find(observer) {
        let start = from + at;
        let end = start + observer.len();
        let before_ok = start == 0 || !is_ident_byte(bytes[start - 1]);
        let after_ok = end == bytes.len() || !is_ident_byte(bytes[end]);
        if before_ok && after_ok {
            return true;
        }
        from = end;
    }
    false
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Record what a premise costs us: trust, undischarged assumptions, and a
/// domain too narrow for the conclusion.
fn note_premise(
    p: &Premise,
    trusted: &mut Vec<String>,
    assumptions: &mut Vec<String>,
    blockers: &mut Vec<Blocker>,
) {
    if let Provenance::Trusted { .. } = p.provenance {
        trusted.push(p.item.clone());
    }
    for a in &p.assumptions {
        assumptions.push(format!("{}: {a}", p.item));
    }
    if let Domain::Restricted { description } = &p.domain {
        blockers.push(Blocker::RestrictedDomain {
            item: p.item.clone(),
            description: description.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u32_observer(name: &str) -> Observer {
        Observer {
            name: name.to_string(),
            facts: RangeFacts::of_rust_type("u32", Some(64)),
            reads_are_pure: true,
        }
    }

    fn bucket_property() -> Property {
        Property {
            name: "never over capacity".into(),
            invariant: "available <= capacity".into(),
            state_type: "TokenBucket".into(),
            observers: vec![u32_observer("available"), u32_observer("capacity")],
        }
    }

    fn u32_param(name: &str) -> Parameter {
        Parameter {
            name: name.to_string(),
            facts: RangeFacts::of_rust_type("u32", Some(64)),
        }
    }

    fn bool_param(name: &str) -> Parameter {
        Parameter {
            name: name.to_string(),
            facts: RangeFacts::Bool,
        }
    }

    fn proved(
        item: &str,
        role: PremiseRole,
        params: Vec<Parameter>,
        ensures: &[&str],
        frame: &[&str],
    ) -> Premise {
        Premise {
            item: item.into(),
            role,
            parameters: params,
            source_fingerprint: "src1".into(),
            contract_fingerprint: "con1".into(),
            config: "cfg1".into(),
            domain: Domain::Unrestricted,
            requires: vec![],
            ensures: ensures.iter().map(|s| s.to_string()).collect(),
            frame: frame
                .iter()
                .map(|o| FrameFact {
                    observer: o.to_string(),
                    justification: FrameJustification::ProvedContract,
                })
                .collect(),
            assumptions: vec![],
            provenance: Provenance::Proved {
                engine: "verus".into(),
                version: "0.2026.08.23.fbbbbcf".into(),
                artifact: "a1".into(),
            },
        }
    }

    fn bucket_premises() -> Vec<Premise> {
        vec![
            proved(
                "TokenBucket::new",
                PremiseRole::Constructor,
                vec![u32_param("cap")],
                &["available == capacity", "capacity == cap"],
                &[],
            ),
            // Faithful to `tests/spike/verus-component/proof/bucket.rs`:
            // both branches, and the capacity clause stated in the contract
            // rather than asserted as a frame fact the contract does not
            // support. The first version of this fixture did the latter,
            // and the frame-claim check caught it (2026-09-09).
            proved(
                "TokenBucket::try_take",
                PremiseRole::Transition,
                vec![u32_param("tokens"), bool_param("ok")],
                &[
                    "ok == (old(available) >= tokens)",
                    "ok ==> available == old(available) - tokens",
                    "!ok ==> available == old(available)",
                    "capacity == old(capacity)",
                ],
                &[],
            ),
            proved(
                "TokenBucket::refill",
                PremiseRole::Transition,
                vec![u32_param("tokens")],
                &[
                    "available == min(old(available) + tokens, old(capacity))",
                    "capacity == old(capacity)",
                ],
                &[],
            ),
        ]
    }

    fn bucket_inventory() -> Inventory {
        Inventory {
            constructors: vec!["TokenBucket::new".into()],
            mutators: vec!["TokenBucket::try_take".into(), "TokenBucket::refill".into()],
            escapes: vec![],
            unclassified: vec![],
        }
    }

    /// A parameter with no declared range is a free variable in the model:
    /// the solver may pick any value for it, including values the program
    /// cannot produce. That is the same hole as an observer with no range,
    /// one level along, and it blocks the same way.
    #[test]
    fn a_parameter_whose_range_is_unknown_blocks() {
        let mut premises = bucket_premises();
        let take = premises
            .iter_mut()
            .find(|p| p.item.ends_with("try_take"))
            .unwrap();
        take.parameters = vec![
            Parameter {
                name: "tokens".into(),
                facts: RangeFacts::of_rust_type("Duration", None),
            },
            Parameter {
                name: "ok".into(),
                facts: RangeFacts::Bool,
            },
        ];
        let p = plan(
            &bucket_property(),
            &bucket_inventory(),
            &premises,
            &current_facts(),
        );
        assert!(
            p.blockers.contains(&Blocker::UnsupportedParameter {
                item: "TokenBucket::try_take".into(),
                parameter: "tokens".into(),
                rust_type: "Duration".into(),
            }),
            "a parameter of unknown width must block: {:?}",
            p.blockers
        );
    }

    /// A name in a contract that is neither a reading nor a declared
    /// parameter is bounded by nothing at all. Left alone it becomes a
    /// free variable the solver ranges over however it likes.
    #[test]
    fn a_name_in_a_contract_that_is_neither_a_reading_nor_a_parameter_blocks() {
        let mut premises = bucket_premises();
        let take = premises
            .iter_mut()
            .find(|p| p.item.ends_with("try_take"))
            .unwrap();
        take.ensures.push("available <= budget".into());
        let p = plan(
            &bucket_property(),
            &bucket_inventory(),
            &premises,
            &current_facts(),
        );
        assert!(
            p.blockers.contains(&Blocker::UndeclaredName {
                item: "TokenBucket::try_take".into(),
                name: "budget".into(),
            }),
            "`budget` is declared nowhere and must block: {:?}",
            p.blockers
        );
    }

    /// The baseline: a complete inventory with a premise for every path
    /// plans one initialization and one preservation each, and nothing
    /// blocks it.
    #[test]
    fn a_covered_bucket_plans_init_and_preservation_and_nothing_blocks() {
        let p = plan(
            &bucket_property(),
            &bucket_inventory(),
            &bucket_premises(),
            &current_facts(),
        );
        assert!(
            p.coverage_is_complete(),
            "nothing should block a fully covered type: {:?}",
            p.blockers
        );
        assert!(!p.is_conditional(), "no premise here is trusted");
        // One constructor: initialization + arithmetic safety.
        // Two mutators: preservation + reachability + arithmetic safety each.
        assert_eq!(
            p.obligations.len(),
            8,
            "2 for the constructor, 3 per mutator"
        );
        assert!(p.obligations.iter().any(|o| o.kind
            == ObligationKind::Initialization {
                constructor: "TokenBucket::new".into()
            }));
        for op in ["TokenBucket::try_take", "TokenBucket::refill"] {
            assert!(
                p.obligations.iter().any(|o| o.kind
                    == ObligationKind::Preservation {
                        operation: op.into()
                    }),
                "missing preservation for {op}"
            );
        }
    }

    /// **The measured failure mode.** A proof over a set that quietly omits
    /// one mutator says nothing about the type, and reads as *stronger*
    /// than the sampling it replaces. Adding a mutator with no premise must
    /// withhold the complete proof and name the path.
    #[test]
    fn an_uncontracted_mutator_withholds_the_proof_and_names_it() {
        let mut inv = bucket_inventory();
        inv.mutators.push("TokenBucket::force_set".into());
        let p = plan(
            &bucket_property(),
            &inv,
            &bucket_premises(),
            &current_facts(),
        );
        assert!(!p.coverage_is_complete());
        assert!(
            p.blockers.contains(&Blocker::UncoveredMutator {
                item: "TokenBucket::force_set".into()
            }),
            "the missing path has to be named, not just counted: {:?}",
            p.blockers
        );
    }

    /// The same for a second construction path nobody wrote a contract for.
    #[test]
    fn an_alternative_constructor_without_a_premise_withholds_the_proof() {
        let mut inv = bucket_inventory();
        inv.constructors.push("TokenBucket::empty".into());
        let p = plan(
            &bucket_property(),
            &inv,
            &bucket_premises(),
            &current_facts(),
        );
        assert!(p.blockers.contains(&Blocker::UncoveredConstructor {
            item: "TokenBucket::empty".into()
        }));
    }

    /// An omitted post-state fact is **unconstrained**, never implicitly
    /// unchanged. Dropping `try_take`'s capacity frame fact must be caught
    /// here, before a solver is ever asked.
    #[test]
    fn an_operation_silent_about_an_observer_blocks_rather_than_assuming_it_unchanged() {
        let mut premises = bucket_premises();
        premises[1].frame.clear();
        // Remove the clause that constrains capacity, leaving the contract
        // genuinely silent about it.
        premises[1].ensures.retain(|e| !e.starts_with("capacity"));
        let p = plan(
            &bucket_property(),
            &bucket_inventory(),
            &premises,
            &current_facts(),
        );
        assert!(
            p.blockers.contains(&Blocker::UnjustifiedFrame {
                operation: "TokenBucket::try_take".into(),
                observer: "capacity".into()
            }),
            "silence about `capacity` must block: {:?}",
            p.blockers
        );
    }

    /// A premise proved only over a restriction cannot license an
    /// unrestricted conclusion.
    #[test]
    fn a_narrowly_bounded_premise_does_not_promote_to_an_unbounded_property() {
        let mut premises = bucket_premises();
        premises[2].domain = Domain::Restricted {
            description: "tokens <= 16".into(),
        };
        let p = plan(
            &bucket_property(),
            &bucket_inventory(),
            &premises,
            &current_facts(),
        );
        assert!(
            p.blockers.contains(&Blocker::RestrictedDomain {
                item: "TokenBucket::refill".into(),
                description: "tokens <= 16".into()
            }),
            "a bounded premise must not silently become an unbounded claim: {:?}",
            p.blockers
        );
    }

    /// Trust is legitimate and must propagate by name.
    #[test]
    fn a_trusted_premise_makes_the_result_conditional_and_names_what_is_trusted() {
        let mut premises = bucket_premises();
        premises[1].provenance = Provenance::Trusted {
            declared_by: "maintainer".into(),
        };
        let p = plan(
            &bucket_property(),
            &bucket_inventory(),
            &premises,
            &current_facts(),
        );
        assert!(p.is_conditional());
        assert_eq!(p.trusted, vec!["TokenBucket::try_take".to_string()]);
    }

    /// A writable field or an escaped `&mut` means the state can change
    /// without passing through any operation, so the theorem has no
    /// subject however well the operations are proved.
    #[test]
    fn a_mutation_escape_opens_the_boundary_regardless_of_the_operations() {
        let mut inv = bucket_inventory();
        inv.escapes.push("TokenBucket::available is pub".into());
        let p = plan(
            &bucket_property(),
            &inv,
            &bucket_premises(),
            &current_facts(),
        );
        assert!(p.blockers.contains(&Blocker::OpenBoundary {
            path: "TokenBucket::available is pub".into()
        }));
    }

    /// Something the scan could not classify means the inventory is not
    /// known to be complete -- which is not the same as knowing it is fine.
    #[test]
    fn an_unclassified_path_blocks_because_the_inventory_is_not_known_complete() {
        let mut inv = bucket_inventory();
        inv.unclassified
            .push("impl<T> TokenBucket<T> (generic, not scanned)".into());
        let p = plan(
            &bucket_property(),
            &inv,
            &bucket_premises(),
            &current_facts(),
        );
        assert!(!p.coverage_is_complete());
        assert!(
            p.blockers
                .iter()
                .any(|b| matches!(b, Blocker::UnclassifiedPath { .. }))
        );
    }

    /// `&self` is not evidence of purity: interior mutability and shared
    /// aliases can mutate through a shared reference. An observer that has
    /// not been analysed blocks rather than passes.
    #[test]
    fn a_reader_not_shown_pure_blocks_because_shared_does_not_mean_immutable() {
        let mut prop = bucket_property();
        prop.observers[0].reads_are_pure = false;
        let p = plan(
            &prop,
            &bucket_inventory(),
            &bucket_premises(),
            &current_facts(),
        );
        assert!(p.blockers.contains(&Blocker::UnprovenReaderPurity {
            observer: "available".into()
        }));
    }

    /// An observer whose declared type has no stated range must not be
    /// admitted unconstrained -- that lets the solver assume anything.
    #[test]
    fn an_observer_with_no_stated_range_blocks_instead_of_being_admitted_freely() {
        let mut prop = bucket_property();
        prop.observers.push(Observer {
            name: "label".into(),
            facts: RangeFacts::of_rust_type("String", Some(64)),
            reads_are_pure: true,
        });
        let p = plan(
            &prop,
            &bucket_inventory(),
            &bucket_premises(),
            &current_facts(),
        );
        assert!(p.blockers.contains(&Blocker::UnsupportedObserver {
            observer: "label".into(),
            rust_type: "String".into()
        }));
    }

    /// Machine widths travel as premises. A `u32` observer is bounded by
    /// `u32`, never by the solver's idea of an integer.
    #[test]
    fn declared_widths_become_explicit_bounds_not_unbounded_integers() {
        assert_eq!(
            RangeFacts::of_rust_type("u32", Some(64)),
            RangeFacts::Integer {
                rust_type: "u32".into(),
                min: 0,
                max: 4_294_967_295
            }
        );
        assert_eq!(
            RangeFacts::of_rust_type("i8", Some(64)),
            RangeFacts::Integer {
                rust_type: "i8".into(),
                min: -128,
                max: 127
            }
        );
    }

    /// A pointer-sized type follows the **target's** width, and an unknown
    /// width blocks rather than defaulting.
    ///
    /// Guessing 64 on a 32-bit target errs in the dangerous direction: it
    /// gives the solver a wider range than the program has, so an overflow
    /// the program really suffers is proved impossible.
    #[test]
    fn a_pointer_sized_type_follows_the_target_and_refuses_to_guess() {
        assert_eq!(
            RangeFacts::of_rust_type("usize", Some(32)),
            RangeFacts::Integer {
                rust_type: "usize".into(),
                min: 0,
                max: 4_294_967_295
            }
        );
        assert_eq!(
            RangeFacts::of_rust_type("isize", Some(32)),
            RangeFacts::Integer {
                rust_type: "isize".into(),
                min: -2_147_483_648,
                max: 2_147_483_647
            }
        );
        assert_eq!(
            RangeFacts::of_rust_type("usize", None),
            RangeFacts::Unsupported {
                rust_type: "usize".into()
            },
            "an unknown target width must block, never default to 64"
        );
        // A fixed-width type is unaffected by not knowing the target.
        assert!(matches!(
            RangeFacts::of_rust_type("u16", None),
            RangeFacts::Integer { .. }
        ));
    }

    /// Undischarged assumptions travel with the property, attributed to the
    /// premise that carried them.
    #[test]
    fn assumptions_are_carried_up_attributed_to_the_premise_that_owed_them() {
        let mut premises = bucket_premises();
        premises[2]
            .assumptions
            .push("the clock is monotonic".into());
        let p = plan(
            &bucket_property(),
            &bucket_inventory(),
            &premises,
            &current_facts(),
        );
        assert_eq!(
            p.assumptions,
            vec!["TokenBucket::refill: the clock is monotonic".to_string()]
        );
    }

    /// Word boundaries: an observer named `capacity` must not be found
    /// inside `spare_capacity`, or a contract that never mentions it would
    /// read as constraining it.
    #[test]
    fn an_observer_is_matched_on_word_boundaries_not_substrings() {
        assert!(mentions("available <= capacity", "capacity"));
        assert!(!mentions("spare_capacity > 0", "capacity"));
        assert!(!mentions("capacity_hint == 3", "capacity"));
    }

    // ---- The gaps adversarial review constructed, 2026-09-09. Each of
    // these produced `coverage_is_complete() == true` when it should not.

    fn current_facts() -> std::collections::BTreeMap<String, SourceIdentity> {
        [
            "TokenBucket::new",
            "TokenBucket::try_take",
            "TokenBucket::refill",
        ]
        .iter()
        .map(|i| {
            (
                i.to_string(),
                SourceIdentity {
                    source_fingerprint: "src1".into(),
                    contract_fingerprint: "con1".into(),
                    config: "cfg1".into(),
                },
            )
        })
        .collect()
    }

    /// **The order-dependent verdict.** Two premises for one item gave two
    /// different answers depending which came last; the map silently kept
    /// one. Same inputs must not produce two verdicts.
    #[test]
    fn two_premises_for_one_item_block_rather_than_one_silently_winning() {
        let mut premises = bucket_premises();
        let mut dup = premises[2].clone();
        dup.domain = Domain::Restricted {
            description: "tokens <= 16".into(),
        };
        premises.push(dup);
        let forward = plan(
            &bucket_property(),
            &bucket_inventory(),
            &premises,
            &current_facts(),
        );
        premises.swap(2, 3);
        let reversed = plan(
            &bucket_property(),
            &bucket_inventory(),
            &premises,
            &current_facts(),
        );
        assert!(
            forward.blockers.contains(&Blocker::DuplicatePremise {
                item: "TokenBucket::refill".into()
            }),
            "a second premise for the same item must block: {:?}",
            forward.blockers
        );
        assert_eq!(
            forward.coverage_is_complete(),
            reversed.coverage_is_complete(),
            "the verdict must not depend on premise order"
        );
    }

    /// A premise whose source moved is stale, not weaker. Nothing was
    /// comparing fingerprints at all.
    #[test]
    fn a_premise_whose_source_moved_is_stale_and_blocks() {
        let mut premises = bucket_premises();
        premises[1].source_fingerprint = "moved".into();
        let p = plan(
            &bucket_property(),
            &bucket_inventory(),
            &premises,
            &current_facts(),
        );
        assert!(
            p.blockers.contains(&Blocker::StalePremise {
                item: "TokenBucket::try_take".into(),
                field: "source".into()
            }),
            "{:?}",
            p.blockers
        );
    }

    /// The compilation configuration is part of identity too.
    #[test]
    fn a_premise_taken_under_a_different_configuration_blocks() {
        let mut premises = bucket_premises();
        premises[2].config = "cfg-other".into();
        let p = plan(
            &bucket_property(),
            &bucket_inventory(),
            &premises,
            &current_facts(),
        );
        assert!(p.blockers.contains(&Blocker::StalePremise {
            item: "TokenBucket::refill".into(),
            field: "config".into()
        }));
    }

    /// A reader's contract is not a two-state contract. The role was never
    /// read.
    #[test]
    fn a_premise_with_the_wrong_role_blocks_rather_than_standing_in() {
        let mut premises = bucket_premises();
        premises[1].role = PremiseRole::Reader;
        let p = plan(
            &bucket_property(),
            &bucket_inventory(),
            &premises,
            &current_facts(),
        );
        assert!(
            p.blockers.contains(&Blocker::WrongRole {
                item: "TokenBucket::try_take".into(),
                expected: "Transition".into(),
                found: "Reader".into()
            }),
            "{:?}",
            p.blockers
        );
    }

    /// A frame fact asserting the contract states it, when the contract does
    /// not, re-admits the omitted-frame hole through a side door.
    #[test]
    fn a_frame_fact_claiming_contract_support_must_actually_have_it() {
        let mut premises = bucket_premises();
        premises[1].ensures.clear();
        premises[1].frame = vec![
            FrameFact {
                observer: "available".into(),
                justification: FrameJustification::ProvedContract,
            },
            FrameFact {
                observer: "capacity".into(),
                justification: FrameJustification::ProvedContract,
            },
        ];
        let p = plan(
            &bucket_property(),
            &bucket_inventory(),
            &premises,
            &current_facts(),
        );
        assert!(
            p.blockers.iter().any(|b| matches!(
                b,
                Blocker::UnsupportedFrameClaim { operation, observer }
                    if operation == "TokenBucket::try_take" && observer == "available"
            )),
            "a frame fact citing the contract must be checked against it: {:?}",
            p.blockers
        );
    }

    /// A premise for an operation the boundary scan never found is direct
    /// evidence the inventory is incomplete. It was dropped without trace.
    #[test]
    fn a_premise_for_an_item_outside_the_inventory_blocks() {
        let mut premises = bucket_premises();
        let mut extra = premises[2].clone();
        extra.item = "TokenBucket::force_set".into();
        premises.push(extra);
        let p = plan(
            &bucket_property(),
            &bucket_inventory(),
            &premises,
            &current_facts(),
        );
        assert!(
            p.blockers.contains(&Blocker::PremiseOutsideInventory {
                item: "TokenBucket::force_set".into()
            }),
            "{:?}",
            p.blockers
        );
    }

    /// A type with no constructors has no reachable states. A scan that
    /// returned nothing is far likelier, and must not read as covered.
    #[test]
    fn an_empty_inventory_is_not_full_coverage() {
        let p = plan(
            &bucket_property(),
            &Inventory::default(),
            &[],
            &Default::default(),
        );
        assert!(
            !p.coverage_is_complete(),
            "an empty inventory means the scan found nothing, not that nothing exists"
        );
        assert!(p.blockers.contains(&Blocker::NoConstructors));
    }

    /// Every observer the invariant names must be declared, or the frame
    /// check silently has nothing to look for.
    #[test]
    fn an_observer_named_in_the_invariant_but_undeclared_blocks() {
        let mut prop = bucket_property();
        prop.observers.retain(|o| o.name != "capacity");
        let p = plan(
            &prop,
            &bucket_inventory(),
            &bucket_premises(),
            &current_facts(),
        );
        assert!(
            p.blockers.contains(&Blocker::UndeclaredObserver {
                observer: "capacity".into()
            }),
            "{:?}",
            p.blockers
        );
    }

    /// Satisfiability is not decidable here, so every operation carries an
    /// obligation for the backend to check it. A contradictory premise set
    /// verifies everything, and that must be asked rather than assumed.
    #[test]
    fn every_operation_carries_a_reachability_obligation_for_the_backend() {
        let p = plan(
            &bucket_property(),
            &bucket_inventory(),
            &bucket_premises(),
            &current_facts(),
        );
        for op in ["TokenBucket::try_take", "TokenBucket::refill"] {
            assert!(
                p.obligations.iter().any(|o| o.kind
                    == ObligationKind::Reachable {
                        operation: op.into()
                    }),
                "no vacuity check planned for {op}"
            );
        }
    }

    /// The retracted arithmetic hole: every operation owes a non-overflow
    /// obligation, because a contract is a Rust expression and the prover's
    /// integers are not Rust's.
    #[test]
    fn every_operation_carries_an_arithmetic_safety_obligation() {
        let p = plan(
            &bucket_property(),
            &bucket_inventory(),
            &bucket_premises(),
            &current_facts(),
        );
        assert!(p.obligations.iter().any(|o| o.kind
            == ObligationKind::ArithmeticSafety {
                item: "TokenBucket::refill".into()
            }));
    }

    /// `mentions` must not count a *pre*-state reading as constraining the
    /// post-state, nor find an observer inside a comment or a string.
    #[test]
    fn a_pre_state_mention_does_not_count_as_constraining_the_post_state() {
        assert!(!constrains_post_state(
            "available == old(available) + old(capacity)",
            "capacity"
        ));
        assert!(!constrains_post_state(
            "available == old(available) - tokens /* capacity unchanged */",
            "capacity"
        ));
        assert!(!constrains_post_state(
            "available == \"capacity\".len()",
            "capacity"
        ));
        assert!(constrains_post_state(
            "capacity == old(capacity)",
            "capacity"
        ));
    }
}
