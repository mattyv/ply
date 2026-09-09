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
    pub fn of_rust_type(name: &str) -> Self {
        let (min, max): (i128, i128) = match name {
            "u8" => (0, u8::MAX as i128),
            "u16" => (0, u16::MAX as i128),
            "u32" => (0, u32::MAX as i128),
            "u64" => (0, u64::MAX as i128),
            "usize" => (0, u64::MAX as i128),
            "i8" => (i8::MIN as i128, i8::MAX as i128),
            "i16" => (i16::MIN as i128, i16::MAX as i128),
            "i32" => (i32::MIN as i128, i32::MAX as i128),
            "i64" => (i64::MIN as i128, i64::MAX as i128),
            "isize" => (i64::MIN as i128, i64::MAX as i128),
            "bool" => return RangeFacts::Bool,
            _ => {
                return RangeFacts::Unsupported {
                    rust_type: name.to_string(),
                };
            }
        };
        RangeFacts::Integer {
            rust_type: name.to_string(),
            min,
            max,
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
pub fn plan(property: &Property, inventory: &Inventory, premises: &[Premise]) -> Plan {
    let mut obligations = Vec::new();
    let mut blockers = Vec::new();
    let mut trusted = Vec::new();
    let mut assumptions = Vec::new();

    let by_item: std::collections::BTreeMap<&str, &Premise> =
        premises.iter().map(|p| (p.item.as_str(), p)).collect();

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
                obligations.push(Obligation {
                    kind: ObligationKind::Initialization {
                        constructor: ctor.clone(),
                    },
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

        // Every observer the invariant needs must be either constrained by
        // this operation's postcondition or justified as unchanged. Saying
        // nothing leaves it unconstrained -- never implicitly the same.
        for obs in &needed {
            let constrained = p.ensures.iter().any(|e| mentions(e, obs));
            let framed = p.frame.iter().any(|f| f.observer == *obs);
            if !constrained && !framed {
                blockers.push(Blocker::UnjustifiedFrame {
                    operation: op.clone(),
                    observer: (*obs).to_string(),
                });
            }
        }

        obligations.push(Obligation {
            kind: ObligationKind::Preservation {
                operation: op.clone(),
            },
            premises: vec![op.clone()],
        });
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
            facts: RangeFacts::of_rust_type("u32"),
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

    fn proved(item: &str, role: PremiseRole, ensures: &[&str], frame: &[&str]) -> Premise {
        Premise {
            item: item.into(),
            role,
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
                &["available == capacity", "capacity == cap"],
                &[],
            ),
            proved(
                "TokenBucket::try_take",
                PremiseRole::Transition,
                &["ok ==> available == old(available) - tokens"],
                &["capacity"],
            ),
            proved(
                "TokenBucket::refill",
                PremiseRole::Transition,
                &["available == min(old(available) + tokens, old(capacity))"],
                &["capacity"],
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

    /// The baseline: a complete inventory with a premise for every path
    /// plans one initialization and one preservation each, and nothing
    /// blocks it.
    #[test]
    fn a_covered_bucket_plans_init_and_preservation_and_nothing_blocks() {
        let p = plan(&bucket_property(), &bucket_inventory(), &bucket_premises());
        assert!(
            p.coverage_is_complete(),
            "nothing should block a fully covered type: {:?}",
            p.blockers
        );
        assert!(!p.is_conditional(), "no premise here is trusted");
        assert_eq!(p.obligations.len(), 3, "one init, two preservation");
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
        let p = plan(&bucket_property(), &inv, &bucket_premises());
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
        let p = plan(&bucket_property(), &inv, &bucket_premises());
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
        let p = plan(&bucket_property(), &bucket_inventory(), &premises);
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
        let p = plan(&bucket_property(), &bucket_inventory(), &premises);
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
        let p = plan(&bucket_property(), &bucket_inventory(), &premises);
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
        let p = plan(&bucket_property(), &inv, &bucket_premises());
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
        let p = plan(&bucket_property(), &inv, &bucket_premises());
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
        let p = plan(&prop, &bucket_inventory(), &bucket_premises());
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
            facts: RangeFacts::of_rust_type("String"),
            reads_are_pure: true,
        });
        let p = plan(&prop, &bucket_inventory(), &bucket_premises());
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
            RangeFacts::of_rust_type("u32"),
            RangeFacts::Integer {
                rust_type: "u32".into(),
                min: 0,
                max: 4_294_967_295
            }
        );
        assert_eq!(
            RangeFacts::of_rust_type("i8"),
            RangeFacts::Integer {
                rust_type: "i8".into(),
                min: -128,
                max: 127
            }
        );
    }

    /// Undischarged assumptions travel with the property, attributed to the
    /// premise that carried them.
    #[test]
    fn assumptions_are_carried_up_attributed_to_the_premise_that_owed_them() {
        let mut premises = bucket_premises();
        premises[2]
            .assumptions
            .push("the clock is monotonic".into());
        let p = plan(&bucket_property(), &bucket_inventory(), &premises);
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
}
