//! The transcript: everything the diagram says, written out as text.
//!
//! ## Why this exists
//!
//! Measured on the committed trading-system diagram: 474 characters of text
//! are drawn on the canvas and 9,923 are reachable only by hovering. That is
//! 95% of what the render says — and all of the *reasoning*: why a component
//! sits where it does on the ladder, what a check actually does, which
//! ancestor a promise was inherited from, which caveats apply. A reader who
//! cannot hover gets the labels and none of the meaning, and the reader this
//! was built for — a model reading the document — cannot hover at all.
//!
//! Reading `ply.yaml` instead is not equivalent, and the gap is the point:
//! the source says what was *written*, the transcript says what is *true
//! after the rules are applied*. A missing `checks:` line inherits from the
//! nearest ancestor; a written empty one inherits nothing and means "check
//! nothing here". Those look nearly identical and mean opposite things, and
//! a reader who resolves them wrongly states a confident falsehood about
//! what is verified — the exact failure this project exists to prevent.
//!
//! ## Two rules this module lives under
//!
//! **One derivation, two serializations.** Every sentence here comes from the
//! same functions the drawing uses ([`super::svg::check_prose`],
//! [`super::svg::ceiling_tooltip_line`], and the rest). The two views cannot
//! word a fact differently because there is only one wording. Anything that
//! needs restating here rather than sharing is a design smell to raise, not
//! to route around.
//!
//! **Deterministic by construction, not by testing.** [`render_transcript`]
//! takes the parsed document and returns a string: no filesystem, no clock,
//! no environment, no locale, no randomness can enter, because none of them
//! are in scope. Every collection it walks is an `IndexMap` or a `Vec` — the
//! model chose those over hash-ordered containers precisely so iteration is
//! document order. There are no sorts (nothing here has geometry to sort by)
//! and no computed non-integers, so there is no float formatting to pin.
//!
//! It is never written to disk by the build and never committed: it is
//! generated on demand like a compiler's output, so it cannot go stale.

use super::svg::{
    ceiling_tooltip_line, check_prose, component_ceiling, deny_rule_prose, profile_rules_prose,
    weakest_declaration,
};
use crate::model::{
    Component, Document, EdgeKind, FnClaim, InheritedChecks, Mode, component_default_checks,
    effective_checks, parse_deny, parse_edge,
};
use indexmap::IndexMap;

/// Two spaces per level of nesting.
fn pad(level: usize) -> String {
    "  ".repeat(level)
}

/// `1 component` / `2 components` — the summary line reads as English or it
/// reads as a machine's output, and this one is the first thing anybody sees.
fn plural(n: usize, one: &str, many: &str) -> &'static str where {
    // Returned as a literal so no allocation and no locale can enter.
    if n == 1 {
        Box::leak(one.to_string().into_boxed_str())
    } else {
        Box::leak(many.to_string().into_boxed_str())
    }
}

pub fn render_transcript(doc: &Document) -> String {
    let mut out = String::new();

    // The header earns its three lines: what this is, that editing it does
    // nothing, and that nothing has been run. The third is the summary
    // strip's own rule applied to the whole document -- a reader who meets a
    // page of promises without that sentence will read them as results.
    out.push_str(
        "This is a Ply transcript: everything the diagram of ply.yaml shows — every box, \
         arrow, and rule, and all the text that is otherwise visible only by hovering — \
         written out in full.\n",
    );
    out.push_str(
        "It is generated on demand and never saved, like a compiler's output. Editing this \
         text changes nothing; edit the ply.yaml document and generate it again.\n",
    );
    out.push_str(
        "Nothing here has been run. Every line is a declaration or a promise, never a result \
         — running `cargo ply verify` is what turns promises into results.\n\n",
    );

    let (components, functions, unclaimed) = counts(doc);
    out.push_str(&format!(
        "{components} {} · {functions} {} · {unclaimed} {} nothing\n",
        plural(components, "component", "components"),
        plural(functions, "function", "functions"),
        plural(unclaimed, "promises", "promise"),
    ));
    out.push_str(
        "(\"promise nothing\" counts functions with no checks against them at all — code this \
         document describes but says nothing about)\n\n",
    );

    out.push_str("components:\n");
    for (name, comp) in &doc.components {
        out.push('\n');
        write_component(&mut out, name, comp, None, 1, &doc.profiles);
    }

    out.push('\n');
    if doc.externals.is_empty() {
        out.push_str("externals — systems and people outside this codebase: none declared\n");
    } else {
        out.push_str("externals — systems and people outside this codebase:\n");
        for (name, ext) in &doc.externals {
            out.push_str(&format!(
                "{}{name} — a system or person outside this codebase: {}. Ply lists it so the \
                 boundary is visible, but checks nothing about it — every edge naming it is a \
                 declaration, not a verified fact.\n",
                pad(1),
                ext.note
            ));
        }
    }

    out.push('\n');
    if doc.edges.is_empty() {
        out.push_str("edges — who may call whom, and what data flows where: none declared\n");
    } else {
        out.push_str("edges — who may call whom, and what data flows where:\n");
        out.push_str(&format!(
            "{}(\"a -> b\" means a may call b; an undeclared cross-component call is an \
             architecture finding — a warning by default, an error if the calling component is \
             `strict` (§5.3, A0402))\n",
            pad(1)
        ));
        out.push_str(&format!(
            "{}(\"a ~> b : T\" means T data flows from a to b — declared so the flow is visible; \
             nothing checks flows in v1)\n",
            pad(1)
        ));
        for raw in &doc.edges {
            match parse_edge(raw) {
                Ok(e) => {
                    let line = match &e.kind {
                        EdgeKind::Call => format!("{} -> {}", e.from, e.to),
                        EdgeKind::Flow(ty) => format!("{} ~> {} : {ty}", e.from, e.to),
                    };
                    let outside =
                        doc.externals.contains_key(&e.from) || doc.externals.contains_key(&e.to);
                    let note = if outside {
                        let ext = if doc.externals.contains_key(&e.from) {
                            &e.from
                        } else {
                            &e.to
                        };
                        format!(
                            " — {ext} is outside this codebase, so this edge is a declaration, \
                             never a verified fact"
                        )
                    } else {
                        String::new()
                    };
                    out.push_str(&format!("{}{line}{note}\n", pad(1)));
                }
                // Never dropped: an edge the reader wrote is a fact about the
                // document even when Ply cannot make sense of it. Silence here
                // would be the transcript claiming the document says less than
                // it does.
                Err(e) => out.push_str(&format!(
                    "{}{raw} — this document declares this edge but Ply cannot read it: {e}\n",
                    pad(1)
                )),
            }
        }
    }

    out.push('\n');
    if doc.deny.is_empty() {
        out.push_str("forbidden calls: none declared\n");
    } else {
        out.push_str("forbidden calls:\n");
        for raw in &doc.deny {
            match parse_deny(raw) {
                Ok(d) => out.push_str(&format!("{}{}\n", pad(1), deny_rule_prose(&d))),
                Err(e) => out.push_str(&format!(
                    "{}{raw} — this document declares this rule but Ply cannot read it: {e}\n",
                    pad(1)
                )),
            }
        }
    }

    out.push('\n');
    if doc.profiles.is_empty() {
        out.push_str(
            "profiles — named bundles of extra rules a component can adopt: none declared\n",
        );
    } else {
        out.push_str("profiles — named bundles of extra rules a component can adopt:\n");
        for (name, rules) in &doc.profiles {
            out.push_str(&format!(
                "{}{name}: {}\n",
                pad(1),
                profile_rules_prose(rules)
            ));
        }
    }

    out.push('\n');
    if doc.unresolved.is_empty() {
        out.push_str("unresolved decisions held by the workspace as a whole: none declared\n");
    } else {
        out.push_str("unresolved decisions held by the workspace as a whole:\n");
        for u in &doc.unresolved {
            out.push_str(&format!(
                "{}#{} marks an unresolved decision — a question the design still owes an \
                 answer: {}. It belongs to the workspace as a whole, not to any function or \
                 component yet; Ply tracks it until someone resolves it (§5.6).\n",
                pad(1),
                u.id,
                u.note
            ));
        }
    }

    out
}

/// Components, functions, and functions promising nothing — the summary
/// strip's three numbers, counted the same way it counts them.
fn counts(doc: &Document) -> (usize, usize, usize) {
    fn walk(
        comp: &Component,
        inherited: Option<InheritedChecks>,
        c: &mut usize,
        f: &mut usize,
        u: &mut usize,
    ) {
        *c += 1;
        let default = component_default_checks("", comp, inherited);
        for fc in comp.fns.values() {
            *f += 1;
            if effective_checks(fc, default).is_none_or(|e| e.is_empty()) {
                *u += 1;
            }
        }
        for child in comp.components.values() {
            walk(child, default, c, f, u);
        }
    }
    let (mut c, mut f, mut u) = (0, 0, 0);
    for comp in doc.components.values() {
        walk(comp, None, &mut c, &mut f, &mut u);
    }
    (c, f, u)
}

fn write_component(
    out: &mut String,
    name: &str,
    comp: &Component,
    inherited: Option<InheritedChecks>,
    level: usize,
    profiles: &IndexMap<String, Vec<String>>,
) {
    let p = pad(level);
    let q = pad(level + 1);
    out.push_str(&format!(
        "{p}component {name} — maps to Rust module {}\n",
        comp.anchor
    ));

    if let Some(note) = &comp.note {
        out.push_str(&format!("{q}note: {note}\n"));
    }
    if comp.pure {
        out.push_str(&format!(
            "{q}pure — a sealed promise: this component declares no capabilities and may not \
             use any; capability use inside it is an error (A0408)\n"
        ));
    } else if !comp.uses.is_empty() {
        out.push_str(&format!(
            "{q}capabilities: {} — this component may use only the capabilities it declares; \
             using an undeclared one is an architecture finding (§5.3, A0404)\n",
            comp.uses.join(", ")
        ));
    }
    if !comp.owns.is_empty() {
        out.push_str(&format!(
            "{q}owns {} — only this component may mutate them\n",
            comp.owns.join(", ")
        ));
    }
    if let Some(profile) = &comp.profile {
        match profiles.get(profile) {
            Some(rules) => out.push_str(&format!(
                "{q}profile {profile} — a named bundle of extra rules this component must \
                 follow: {}\n",
                profile_rules_prose(rules)
            )),
            None => out.push_str(&format!(
                "{q}profile {profile} (not defined in this document)\n"
            )),
        }
    }
    if comp.strict {
        out.push_str(&format!(
            "{q}strict — architecture findings inside this component fail the build (errors, \
             not warnings)\n"
        ));
    }

    // A default written here is the §5.4c distinction the transcript exists
    // to make legible: it is invisible on any function that inherits it, so a
    // reader who only sees the functions cannot tell an inherited check from
    // one written in place, nor tell "check nothing" from "nothing written".
    match &comp.checks {
        None => {}
        Some(list) if list.is_empty() => out.push_str(&format!(
            "{q}checks: [] — an empty list written on purpose: a function in here that writes \
             no checks of its own is checked by nothing at all, and does not fall back to any \
             outer default\n"
        )),
        Some(list) => {
            out.push_str(&format!(
                "{q}default checks for anything in here that writes none of its own:\n"
            ));
            for c in list {
                out.push_str(&format!("{}{}\n", pad(level + 2), check_prose(c)));
            }
        }
    }

    let default = component_default_checks(name, comp, inherited);
    if comp.fns.is_empty() && comp.components.is_empty() {
        out.push_str(&format!(
            "{q}hollow — declares nothing inside yet: no functions, no nested components. A \
             sketch waiting for claims.\n"
        ));
    } else {
        out.push_str(&format!(
            "{q}{}\n",
            ceiling_tooltip_line(component_ceiling(name, comp, inherited))
        ));
        if let Some((path, _)) = weakest_declaration(comp, inherited, "", name) {
            out.push_str(&format!(
                "{q}the level above is set by its weakest declaration, {path}\n"
            ));
        }
    }

    // Functions before child components, always: the two are separate maps in
    // the document and their relative order is not observable after parsing,
    // so the grammar fixes it rather than leaving it to the parser.
    for (fname, fc) in &comp.fns {
        write_fn(out, fname, fc, default, level + 1);
    }
    for (cname, child) in &comp.components {
        write_component(out, cname, child, default, level + 1, profiles);
    }
}

fn write_fn(
    out: &mut String,
    name: &str,
    fc: &FnClaim,
    inherited: Option<InheritedChecks>,
    level: usize,
) {
    let p = pad(level);
    let q = pad(level + 1);
    out.push_str(&format!("{p}fn {name}\n"));

    match effective_checks(fc, inherited) {
        None => out.push_str(&format!(
            "{q}no checks declared — nothing about this function is verified (unclaimed)\n"
        )),
        Some(list) if list.is_empty() => out.push_str(&format!(
            "{q}checks: [] — a written empty list: this document says to check nothing here, \
             so nothing about this function is verified (unclaimed)\n"
        )),
        Some(list) => {
            // A written empty list and no list at all are different statements
            // (§5.4c) and the transcript keeps them apart -- the drawing
            // currently does not, which is a difference worth stating rather
            // than hiding.
            let from = if fc.checks.is_none() {
                inherited.map(|i| i.from_component)
            } else {
                None
            };
            for c in list {
                match from {
                    Some(source) => out.push_str(&format!(
                        "{q}inherited from component {source}: {}\n",
                        check_prose(c)
                    )),
                    None => out.push_str(&format!("{q}{}\n", check_prose(c))),
                }
            }
        }
    }

    if !fc.check_with.is_empty() {
        let pairs = fc
            .check_with
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "{q}generic — every check runs with {pairs}; whatever they earn covers only that \
             type\n"
        ));
    }

    if !fc.requires.is_empty() || !fc.ensures.is_empty() {
        out.push_str(&format!(
            "{q}contract at the watermark — the line where declaration stops and the body \
             begins:\n"
        ));
        let r = pad(level + 2);
        for c in &fc.requires {
            out.push_str(&format!(
                "{r}requires: {c} (what the caller must guarantee going in)\n"
            ));
        }
        for c in &fc.ensures {
            out.push_str(&format!(
                "{r}ensures: {c} (what the function guarantees coming out)\n"
            ));
        }
        out.push_str(&format!(
            "{r}the checks above test the function against exactly this promise\n"
        ));
    }

    if fc.mode == Mode::Synth {
        out.push_str(&format!(
            "{q}machine-written — the body below this function's contract is synthesized by a \
             model, with the checks holding the line\n"
        ));
    }

    for t in &fc.trusted {
        out.push_str(&format!(
            "{q}trusted (a human vouches for this; no machine checks it): {} — evidence: {}\n",
            t.claim, t.evidence
        ));
    }

    if !fc.examples.is_empty() {
        let n = fc.examples.len();
        out.push_str(&format!(
            "{q}{n} worked {}{} compiled into a test:\n",
            plural(n, "example", "examples"),
            if n == 1 { "," } else { ", each" },
        ));
        let r = pad(level + 2);
        for e in &fc.examples {
            out.push_str(&format!("{r}{e}\n"));
        }
    }

    for u in &fc.unresolved {
        out.push_str(&format!(
            "{q}#{} marks an unresolved decision — a question this function still owes an \
             answer: {}. Until it is resolved, this function's checks cap at `test` (§5.6)\n",
            u.id, u.note
        ));
    }

    for ext in &fc.entry {
        out.push_str(&format!(
            "{q}entry — {ext} can reach this function from outside this codebase; Ply never \
             checks this — it is declared, not verified\n"
        ));
        let r = pad(level + 2);
        if fc.requires.is_empty() {
            out.push_str(&format!(
                "{r}no requires are declared on this function, so it makes no environmental \
                 assumption yet\n"
            ));
        } else {
            out.push_str(&format!(
                "{r}its requires clauses above now stand as environmental assumptions — \
                 promises the outside caller must keep, which nothing here can check\n"
            ));
        }
    }
}
