//! Comparing the design a document declares against the source that was
//! observed, and saying how much of that comparison actually happened.
//!
//! Pure. Nothing here reads a file, runs a process or parses Rust: it is
//! handed a [`SourceModel`] and a [`Document`] and returns findings. That
//! is deliberate, and it is what makes the interesting cases testable --
//! an overlapping anchor, a call through unowned code, a destination
//! nobody could resolve -- without arranging a repository that exhibits
//! them.
//!
//! **The distinction the whole module turns on** is between *no violation
//! was found* and *no violation exists*. They are the same sentence when
//! the analysis saw everything and different sentences otherwise, and a
//! tool that reports the first as the second is worse than one that
//! reports nothing: it converts a gap in its own reading into a promise
//! about someone's code. So a rule whose relevant source could not be read
//! comes back incomplete, and incompleteness never cancels a
//! counterexample that *was* found.

use std::collections::BTreeMap;

use crate::model::{Component, Document};
use crate::source_model::{Destination, ModuleId, Reference, SourceModel};

/// What a comparison established.
///
/// Ordered by how much they withhold, so aggregating a set is taking the
/// worst -- the same rule the verdict kernel applies to evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Outcome {
    /// Established for the scope that was analyzed.
    Satisfied,
    /// Relevant source or relationships were not resolved, so the rule is
    /// not known to hold. Not a failure of the code -- a limit of the
    /// reading.
    Incomplete,
    /// A definite counterexample.
    Violated,
}

/// Why a document's anchors could not be turned into ownership.
///
/// These are faults in the declaration, not in the code, and they stop the
/// comparison rather than degrading it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnershipError {
    /// Two components anchored at the same module. Which one owns it would
    /// otherwise depend on document order.
    DuplicateAnchor {
        module: ModuleId,
        components: Vec<String>,
    },
    /// An anchor naming a module the source does not have -- a typo, and
    /// one that would otherwise leave a component owning nothing while
    /// looking fine.
    AnchorNotFound { component: String, anchor: String },
}

/// Which component owns each module.
#[derive(Debug, Clone, Default)]
pub struct Ownership {
    /// Every anchored component, deepest first, so the most specific claim
    /// on a module is the first one that contains it.
    anchors: Vec<(ModuleId, String)>,
    modules: Vec<ModuleId>,
}

impl Ownership {
    /// The component owning this module, or `None` where none does.
    ///
    /// The most specific anchor containing it wins; its ancestors keep
    /// whatever it does not take.
    pub fn owner_of(&self, module: &ModuleId) -> Option<&str> {
        self.anchors
            .iter()
            .find(|(anchor, _)| anchor.contains(module))
            .map(|(_, name)| name.as_str())
    }

    /// Modules no component claims, in a stable order.
    ///
    /// Kept and reported rather than absorbed: if unowned code were
    /// folded into the nearest component, adding code outside the design
    /// would make the design look better obeyed.
    pub fn unassigned(&self) -> Vec<ModuleId> {
        let mut out: Vec<ModuleId> = self
            .modules
            .iter()
            .filter(|m| self.owner_of(m).is_none())
            .cloned()
            .collect();
        out.sort();
        out
    }
}

/// Resolve every declared anchor against the modules that exist.
pub fn resolve_ownership(doc: &Document, model: &SourceModel) -> (Ownership, Vec<OwnershipError>) {
    let mut errors = Vec::new();
    let mut claims: BTreeMap<ModuleId, Vec<String>> = BTreeMap::new();

    fn walk(qualified: &str, comp: &Component, claims: &mut BTreeMap<ModuleId, Vec<String>>) {
        if !comp.anchor.is_empty() {
            claims
                .entry(ModuleId::parse(&comp.anchor))
                .or_default()
                .push(qualified.to_string());
        }
        for (child, nested) in &comp.components {
            walk(&format!("{qualified}.{child}"), nested, claims);
        }
    }
    for (name, comp) in &doc.components {
        walk(name, comp, &mut claims);
    }

    let mut anchors: Vec<(ModuleId, String)> = Vec::new();
    for (module, owners) in claims {
        if owners.len() > 1 {
            errors.push(OwnershipError::DuplicateAnchor {
                module: module.clone(),
                components: owners,
            });
            continue;
        }
        let owner = owners.into_iter().next().unwrap_or_default();
        if !model.modules.contains(&module) {
            errors.push(OwnershipError::AnchorNotFound {
                component: owner,
                anchor: module.to_string(),
            });
            continue;
        }
        anchors.push((module, owner));
    }

    // Deepest first: `owner_of` then takes the most specific claim without
    // comparing text lengths, which would order `a::bb` above `a::b::c`.
    anchors.sort_by(|a, b| b.0.depth().cmp(&a.0.depth()).then_with(|| a.0.cmp(&b.0)));

    (
        Ownership {
            anchors,
            modules: model.modules.clone(),
        },
        errors,
    )
}

/// One thing the comparison established, and about what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub outcome: Outcome,
    /// Owning component of the referring code, where one owns it.
    pub from: Option<String>,
    /// Owning component of the referenced code.
    pub to: Option<String>,
    /// The reference this is about, where there is one.
    pub reference: Option<Reference>,
    /// Plain words for a reader who has never seen this tool.
    pub detail: String,
}

/// The result of comparing one document against one observed build.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub findings: Vec<Finding>,
    /// How many crossings were actually judged -- the denominator a reader
    /// needs to know how much the answer covers.
    pub crossings_checked: usize,
}

impl Report {
    pub fn violations(&self) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(|f| f.outcome == Outcome::Violated)
    }

    /// The worst thing found. A violation anywhere is the answer; short of
    /// that, any gap makes the whole comparison incomplete.
    pub fn outcome(&self) -> Outcome {
        self.findings
            .iter()
            .map(|f| f.outcome)
            .max()
            .unwrap_or(Outcome::Satisfied)
    }
}

/// Compare a document against an observed build.
pub fn compare(doc: &Document, model: &SourceModel) -> Report {
    let (ownership, _) = resolve_ownership(doc, model);
    let index = crate::arch::ComponentIndex::build(doc);
    let mut report = Report::default();

    for reference in &model.references {
        let from = ownership
            .owner_of(&reference.origin.module)
            .map(str::to_string);

        let to = match &reference.destination {
            Destination::Definite(item) => ownership.owner_of(&item.module).map(str::to_string),
            // Narrowed but not resolved: it may cross, and "may" is never
            // a violation.
            Destination::Candidates(_) | Destination::Unresolved { .. } => None,
        };

        let unresolved = matches!(
            reference.destination,
            Destination::Candidates(_) | Destination::Unresolved { .. }
        );

        if unresolved {
            report.findings.push(Finding {
                outcome: Outcome::Incomplete,
                from,
                to: None,
                reference: Some(reference.clone()),
                detail: match &reference.destination {
                    Destination::Unresolved { reason } => format!(
                        "this {} could not be followed to what it reaches ({reason}), so any \
                         rule about where it may go is not fully checked",
                        reference.kind.describe()
                    ),
                    _ => format!(
                        "this {} could reach more than one place, so it cannot be judged \
                         either way",
                        reference.kind.describe()
                    ),
                },
            });
            continue;
        }

        let (Some(from_name), Some(to_name)) = (from.clone(), to.clone()) else {
            // One end belongs to no component, so no rule speaks about it
            // -- and that is a gap in the design's coverage, not a pass.
            report.findings.push(Finding {
                outcome: Outcome::Incomplete,
                from,
                to,
                reference: Some(reference.clone()),
                detail: "one end of this reference is in code no component claims, so no rule \
                         covers it"
                    .to_string(),
            });
            continue;
        };

        if from_name == to_name {
            continue; // inside one component: not a crossing
        }

        report.crossings_checked += 1;

        // An explicit ban wins over any permission, including the one that
        // containment grants for free.
        let denied = !index.matching_deny(doc, &from_name, &to_name).is_empty();
        if denied || !index.permitted(doc, &from_name, &to_name) {
            report.findings.push(Finding {
                outcome: Outcome::Violated,
                from,
                to,
                reference: Some(reference.clone()),
                detail: format!(
                    "`{}` {} `{}`, and the design does not allow `{from_name}` to depend on \
                     `{to_name}`",
                    reference.origin,
                    reference.kind.describe(),
                    match &reference.destination {
                        Destination::Definite(item) => item.to_string(),
                        _ => String::new(),
                    }
                ),
            });
        }
    }

    report
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Document;
    use crate::source_model::{
        BuildContext, ItemId, ItemKind, ItemRecord, ReferenceKind, Span, Visibility,
    };

    /// Built from real YAML rather than from structs, so these tests go
    /// through the same parser a user's document does.
    fn doc(components: &[(&str, &str)]) -> Document {
        build(components, &[], &[])
    }

    fn build(components: &[(&str, &str)], edges: &[&str], deny: &[&str]) -> Document {
        let mut yaml = String::from("ply: 1\ncomponents:\n");
        for (name, anchor) in components {
            yaml.push_str(&format!("  {name}:\n    anchor: {anchor}\n"));
        }
        if !edges.is_empty() {
            yaml.push_str("edges:\n");
            for e in edges {
                yaml.push_str(&format!("  - {e}\n"));
            }
        }
        if !deny.is_empty() {
            yaml.push_str("deny:\n");
            for d in deny {
                yaml.push_str(&format!("  - {d}\n"));
            }
        }
        crate::model::parse_document(&yaml).expect("the fixture document must parse")
    }

    fn model(modules: &[&str]) -> SourceModel {
        let mut m = SourceModel::new(BuildContext::simple("app", "lib"));
        m.modules = modules.iter().map(|s| ModuleId::parse(s)).collect();
        m
    }

    fn call(from: &str, from_fn: &str, to: &str, to_fn: &str) -> Reference {
        Reference {
            origin: ItemId::new(ModuleId::parse(from), from_fn),
            destination: Destination::Definite(ItemId::new(ModuleId::parse(to), to_fn)),
            kind: ReferenceKind::Call,
            span: Span::at("src/lib.rs", 12),
        }
    }

    /// Two components inside one crate is the case package dependencies
    /// cannot see at all, and it has to work before anything else does.
    #[test]
    fn two_modules_of_one_crate_are_each_owned_by_their_own_component() {
        let d = doc(&[("parse", "app::parse"), ("exec", "app::exec")]);
        let m = model(&["app", "app::parse", "app::exec"]);
        let (own, errors) = resolve_ownership(&d, &m);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(own.owner_of(&ModuleId::parse("app::parse")), Some("parse"));
        assert_eq!(own.owner_of(&ModuleId::parse("app::exec")), Some("exec"));
    }

    /// The most specific anchor wins, and its ancestor keeps everything
    /// else. Picking by declaration order, or by which anchor is longer as
    /// text, both give the wrong answer somewhere.
    #[test]
    fn the_most_specific_anchor_owns_its_subtree_and_the_ancestor_keeps_the_rest() {
        let d = doc(&[("app", "app"), ("inner", "app::parse::decimal")]);
        let m = model(&[
            "app",
            "app::parse",
            "app::parse::decimal",
            "app::parse::decimal::sign",
            "app::exec",
        ]);
        let (own, errors) = resolve_ownership(&d, &m);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            own.owner_of(&ModuleId::parse("app::parse::decimal::sign")),
            Some("inner"),
            "a descendant of the specific anchor belongs to it"
        );
        assert_eq!(
            own.owner_of(&ModuleId::parse("app::parse")),
            Some("app"),
            "the module above it is residual and belongs to the crate root"
        );
    }

    /// Two components claiming one module is a contradiction in the
    /// document, not something to resolve by picking one.
    #[test]
    fn two_components_claiming_one_module_is_a_configuration_error() {
        let d = doc(&[("a", "app::parse"), ("b", "app::parse")]);
        let m = model(&["app", "app::parse"]);
        let (_, errors) = resolve_ownership(&d, &m);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, OwnershipError::DuplicateAnchor { .. })),
            "{errors:?}"
        );
    }

    /// An anchor naming a module that is not there is a typo, and a typo
    /// that silently owns nothing is how a rule stops being checked
    /// without anyone noticing.
    #[test]
    fn an_anchor_naming_no_real_module_is_an_error_rather_than_an_empty_component() {
        let d = doc(&[("ghost", "app::nosuch")]);
        let m = model(&["app", "app::parse"]);
        let (_, errors) = resolve_ownership(&d, &m);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, OwnershipError::AnchorNotFound { .. })),
            "{errors:?}"
        );
    }

    /// A permitted crossing produces no finding, and the analysis has to
    /// have actually looked -- so it is recorded as checked.
    #[test]
    fn a_permitted_crossing_is_recognised_rather_than_merely_not_reported() {
        let d = build(
            &[("parse", "app::parse"), ("exec", "app::exec")],
            &["parse -> exec"],
            &[],
        );
        let mut m = model(&["app", "app::parse", "app::exec"]);
        m.references = vec![call("app::parse", "run", "app::exec", "go")];
        let report = compare(&d, &m);
        assert!(
            report.violations().next().is_none(),
            "{:?}",
            report.findings
        );
        assert_eq!(report.crossings_checked, 1);
    }

    /// The case the whole increment exists for: a forbidden call between
    /// two modules of one crate, named at its own site.
    #[test]
    fn a_forbidden_crossing_names_the_function_and_the_line_not_just_the_components() {
        let d = doc(&[("parse", "app::parse"), ("exec", "app::exec")]);
        let mut m = model(&["app", "app::parse", "app::exec"]);
        m.references = vec![call("app::parse", "run", "app::exec", "go")];
        let report = compare(&d, &m);
        let v = report
            .violations()
            .next()
            .expect("an undeclared crossing is forbidden");
        assert_eq!(v.from.as_deref(), Some("parse"));
        assert_eq!(v.to.as_deref(), Some("exec"));
        assert_eq!(v.reference.as_ref().unwrap().origin.name, "run");
        assert_eq!(
            v.reference.as_ref().unwrap().span.line,
            12,
            "a reader needs the site, not the component pair"
        );
    }

    /// An explicit ban wins over a permission. Reading them the other way
    /// round makes every ban cancellable by adding an edge.
    #[test]
    fn an_explicit_ban_beats_a_permitted_edge() {
        let d = build(
            &[("parse", "app::parse"), ("exec", "app::exec")],
            &["parse -> exec"],
            &["parse -> exec"],
        );
        let mut m = model(&["app", "app::parse", "app::exec"]);
        m.references = vec![call("app::parse", "run", "app::exec", "go")];
        let report = compare(&d, &m);
        assert!(
            report.violations().next().is_some(),
            "the ban must win: {:?}",
            report.findings
        );
    }

    /// A design may reserve room for a dependency nobody has used yet.
    /// Reporting that as a fault would punish planning ahead.
    #[test]
    fn a_permission_nobody_used_is_not_a_violation() {
        let d = build(
            &[("parse", "app::parse"), ("exec", "app::exec")],
            &["parse -> exec"],
            &[],
        );
        let m = model(&["app", "app::parse", "app::exec"]);
        let report = compare(&d, &m);
        assert!(
            report.violations().next().is_none(),
            "{:?}",
            report.findings
        );
    }

    /// A destination the scan could not resolve leaves the rule it might
    /// have broken **incompletely checked**. Calling that clean is the
    /// failure this whole design is arranged against.
    #[test]
    fn an_unresolved_destination_leaves_the_rule_incomplete_rather_than_clean() {
        let d = doc(&[("parse", "app::parse"), ("exec", "app::exec")]);
        let mut m = model(&["app", "app::parse", "app::exec"]);
        m.references = vec![Reference {
            origin: ItemId::new(ModuleId::parse("app::parse"), "run"),
            destination: Destination::Unresolved {
                reason: "the call goes through a trait object".into(),
            },
            kind: ReferenceKind::Call,
            span: Span::at("src/parse.rs", 30),
        }];
        let report = compare(&d, &m);
        assert!(
            report.violations().next().is_none(),
            "an unresolved call is not a violation"
        );
        assert_eq!(
            report.outcome(),
            Outcome::Incomplete,
            "but the answer is not `satisfied` either: {:?}",
            report.findings
        );
    }

    /// A definite violation stands whatever else went unresolved.
    /// Incompleteness must not launder a counterexample.
    #[test]
    fn a_definite_violation_survives_alongside_incomplete_analysis() {
        let d = doc(&[("parse", "app::parse"), ("exec", "app::exec")]);
        let mut m = model(&["app", "app::parse", "app::exec"]);
        m.references = vec![
            call("app::parse", "run", "app::exec", "go"),
            Reference {
                origin: ItemId::new(ModuleId::parse("app::parse"), "other"),
                destination: Destination::Unresolved {
                    reason: "a macro hides the callee".into(),
                },
                kind: ReferenceKind::Call,
                span: Span::at("src/parse.rs", 40),
            },
        ];
        let report = compare(&d, &m);
        assert_eq!(report.outcome(), Outcome::Violated);
    }

    /// A module nobody claimed stays visible. Inventing an owner for it
    /// would make adding unowned code *strengthen* the result.
    #[test]
    fn a_module_no_component_claims_stays_visible_and_gets_no_invented_owner() {
        let d = doc(&[("parse", "app::parse")]);
        let m = model(&["app", "app::parse", "app::stray"]);
        let (own, _) = resolve_ownership(&d, &m);
        assert_eq!(own.owner_of(&ModuleId::parse("app::stray")), None);
        assert_eq!(
            own.unassigned(),
            vec![ModuleId::parse("app"), ModuleId::parse("app::stray")],
            "the crate root is unassigned here too -- no component anchors it"
        );
    }

    /// A crossing out of unowned code cannot be judged, and must not be
    /// silently dropped: the rule it might have broken is not fully
    /// checked.
    #[test]
    fn a_crossing_from_unowned_code_is_reported_rather_than_ignored() {
        let d = doc(&[("exec", "app::exec")]);
        let mut m = model(&["app", "app::stray", "app::exec"]);
        m.references = vec![call("app::stray", "sneak", "app::exec", "go")];
        let report = compare(&d, &m);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.outcome == Outcome::Incomplete && f.from.is_none()),
            "an unowned origin has to surface: {:?}",
            report.findings
        );
        assert!(report.violations().next().is_none());
    }

    /// A re-export gives an item another path, not another home. Ownership
    /// follows the declaration.
    #[test]
    fn a_re_export_does_not_move_the_item_it_re_exports() {
        let d = doc(&[("parse", "app::parse"), ("api", "app::api")]);
        let mut m = model(&["app", "app::parse", "app::api"]);
        m.items = vec![ItemRecord {
            id: ItemId::new(ModuleId::parse("app::parse"), "decode"),
            kind: ItemKind::Function,
            visibility: Visibility::Public,
            span: Span::at("src/parse.rs", 3),
        }];
        m.references = vec![Reference {
            origin: ItemId::new(ModuleId::parse("app::api"), "decode"),
            destination: Destination::Definite(ItemId::new(
                ModuleId::parse("app::parse"),
                "decode",
            )),
            kind: ReferenceKind::ReExport,
            span: Span::at("src/api.rs", 1),
        }];
        let (own, _) = resolve_ownership(&d, &m);
        assert_eq!(
            own.owner_of(&ModuleId::parse("app::parse")),
            Some("parse"),
            "the declaring module still owns it"
        );
        // And the re-export itself is a crossing like any other.
        let report = compare(&d, &m);
        assert_eq!(report.crossings_checked, 1);
    }

    /// A method's source owner is the module holding its `impl`, which is
    /// often not the module declaring the type. Attributing it to the type
    /// puts the code in the wrong component.
    #[test]
    fn a_method_belongs_to_the_module_holding_its_impl_not_the_types_module() {
        let d = doc(&[("types", "app::types"), ("exec", "app::exec")]);
        let mut m = model(&["app", "app::types", "app::exec"]);
        m.items = vec![ItemRecord {
            // `impl Order` written inside `exec`, for a type from `types`.
            id: ItemId::new(ModuleId::parse("app::exec"), "Order::submit"),
            kind: ItemKind::Method {
                implemented_for: "app::types::Order".into(),
            },
            visibility: Visibility::Public,
            span: Span::at("src/exec.rs", 8),
        }];
        let (own, _) = resolve_ownership(&d, &m);
        assert_eq!(
            own.owner_of(&m.items[0].id.module),
            Some("exec"),
            "the impl's module owns the body"
        );
    }

    /// Restrictions are about direct references. Re-reading them as
    /// reachability would report a violation nobody wrote.
    #[test]
    fn a_restriction_is_about_direct_references_not_what_is_reachable_beyond_them() {
        let d = build(
            &[("a", "app::a"), ("b", "app::b"), ("c", "app::c")],
            &["a -> b", "b -> c"],
            &[],
        );
        let mut m = model(&["app", "app::a", "app::b", "app::c"]);
        m.references = vec![
            call("app::a", "one", "app::b", "two"),
            call("app::b", "two", "app::c", "three"),
        ];
        let report = compare(&d, &m);
        assert!(
            report.violations().next().is_none(),
            "`a` never references `c` directly: {:?}",
            report.findings
        );
    }
}
