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
//! **One derivation where there is one fact.** The sentences both views share
//! come from shared functions ([`super::svg::check_prose`],
//! [`super::svg::ceiling_tooltip_line`], [`super::svg::unresolved_fn_pin_prose`]
//! and the rest), so those cannot drift. This is a discipline, not a
//! mechanism, and it has already slipped once: the seal sentence was worded
//! differently in the two views, and the open-question sentence existed here
//! as a byte-for-byte copy, one edit from disagreeing (review, 2026-08-30).
//! Anything that needs restating here rather than sharing is a design smell
//! to raise, not to route around.
//!
//! **Deterministic by construction, not by testing.** [`render_transcript`]
//! takes the parsed document and returns a string: no filesystem, no clock,
//! no environment, no locale, no randomness can enter, because none of them
//! are in scope. Every collection it walks is an `IndexMap` or a `Vec` — the
//! model chose those over hash-ordered containers precisely so iteration is
//! document order. There are no sorts (nothing here has geometry to sort by)
//! and no computed non-integers, so there is no float formatting to pin.
//!
//! Nothing here is written by hand. The build never writes it, and it is
//! never needed on disk to be read -- but one is committed beside each
//! vetting scenario, so that a change to the wording arrives in review as a
//! diff a person can read rather than as an invisible shift in what the
//! tool says. Those copies are pinned against a live render
//! (`the_committed_text_forms_still_match_what_the_documents_render_to`),
//! because a stale one would do the exact opposite of what it is for.

use super::svg::{
    ceiling_tooltip_line, check_prose, component_ceiling, declared_not_checked, deny_rule_prose,
    document_counts, profile_rules_prose, unresolved_fn_pin_prose, weakest_declaration,
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
///
/// Both arms are used for verb agreement as well as noun number, which is
/// why one call site reads `plural(n, "promises", "promise")` with the
/// singular in the `many` slot: "1 promises nothing" / "2 promise nothing".
///
/// It borrows from its arguments and allocates nothing. It previously
/// returned `&'static str` by `Box::leak`-ing a fresh `String` on every
/// call, directly under a comment claiming no allocation could enter --
/// a leak per component, per function, per render (review, 2026-08-30).
fn plural<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 { one } else { many }
}

pub fn render_transcript(doc: &Document) -> String {
    let mut out = String::new();

    // The header earns its three lines: what this is, that editing it does
    // nothing, and that no result reaches it. The third is the summary
    // strip's own rule applied to the whole document -- a reader who meets a
    // page of promises without that sentence will read them as results.
    //
    // It says "no result reaches this page", not "nothing has been run".
    // This function is handed a parsed document and nothing else, so whether
    // anyone has run a verification is outside what it can see; the earlier
    // wording asserted it anyway, and was a flat falsehood to anyone who had
    // just run one (review, 2026-08-30).
    out.push_str(
        // "the diagram of this document", not "of ply.yaml": these are
        // rendered from files called `003-trading-system.ply.yaml` as often
        // as from a plain `ply.yaml`, and the renderer is handed a parsed
        // document with no filename in it at all (deliberately -- where the
        // file sits must not reach the output). Committing a sample beside
        // each vetting scenario is what made the mismatch visible.
        "This is a Ply transcript: everything the diagram of this document shows — every box, \
         arrow, and rule, and all the text that is otherwise visible only by hovering — \
         written out in full.\n",
    );
    // Not "never saved": the repository commits one of these beside each
    // vetting scenario so a change to the wording shows up in review as a
    // readable diff, and a reader holding one of those files would catch the
    // contradiction immediately. What is true either way is that no one
    // edits it by hand.
    out.push_str(
        "It is generated from that document, like a compiler's output, and nothing in it is \
         written by hand. Editing this text changes nothing; edit the ply.yaml document and \
         generate it again.\n",
    );
    out.push_str(
        "No result reaches this page. Every line below is a declaration or a promise, never \
         a result — whatever `cargo ply verify` has found, it is reported there and never \
         here.\n\n",
    );

    let (components, functions, unclaimed) = document_counts(doc);
    out.push_str(&format!(
        "{components} {} · {functions} {} · {unclaimed} {} nothing\n",
        plural(components, "component", "components"),
        plural(functions, "function", "functions"),
        plural(unclaimed, "promises", "promise"),
    ));
    out.push_str(
        // Not "code this document says nothing about": a function that wrote
        // `checks: []` is counted here, and the document says something very
        // deliberate about it. In vetting 003 both counted functions are that
        // kind, so the old gloss was wrong about every function it described.
        "(\"promise nothing\" counts functions that end up with nothing checked — whether \
         nobody wrote any checks for them, or the document switched checking off for them on \
         purpose)\n\n",
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
            "{}(\"a -> b\" means a may call b; that an undeclared cross-component call is \
             forbidden is {})\n",
            pad(1),
            declared_not_checked("a call that crosses this line")
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
    // Not `else if`: a document that says `pure: true` and also lists
    // capabilities is contradicting itself, and dropping either half tells
    // the reader the document says less than it does -- the dropped half
    // being exactly the one that would explain a finding they did not
    // expect. Both are stated; which one wins is `ply check`'s call, not a
    // view's.
    if comp.pure {
        out.push_str(&format!(
            "{q}pure — a sealed promise: this component declares no capabilities and may \
             not use any. That is {}\n",
            declared_not_checked("capability use inside this sealed component")
        ));
    }
    if !comp.uses.is_empty() {
        out.push_str(&format!(
            "{q}capabilities: {} — this component may use only the capabilities it \
             declares. That limit is {}\n",
            comp.uses.join(", "),
            declared_not_checked("use of a capability this component never declared")
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
            "{q}strict — this component asks that architecture findings inside it fail the \
             build rather than warn. Nothing acts on that yet: no check this build runs \
             reads the flag\n"
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
                "{q}that level comes from its weakest part, {path} — nothing here counts as \
                 checked more strongly than the weakest thing inside it\n"
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

    // Where the list came from decides the sentence, not just what is in it.
    // Two functions can both end up with nothing verified -- one wrote
    // `checks: []`, one wrote no line at all and an ancestor's empty list
    // reached it -- and those are opposite statements about the document
    // (§5.4c). Telling the second one it "wrote an empty list" is a false
    // claim about what the author typed, made in the exact place this view
    // was argued for; it read that way until 2026-08-30.
    let from = if fc.checks.is_none() {
        inherited.map(|i| i.from_component)
    } else {
        None
    };

    match effective_checks(fc, inherited) {
        None => out.push_str(&format!(
            "{q}no checks declared — nothing about this function is verified (unclaimed)\n"
        )),
        // `Some([])` rather than a guard on `is_empty()`: the empty slice is
        // the whole condition, and saying it as a pattern keeps this arm the
        // same shape as the two around it.
        Some([]) => match from {
            Some(source) => out.push_str(&format!(
                "{q}nothing is checked here, and this function did not ask for that: it \
                 declares no checks of its own, and component {source} sets an empty default \
                 list, which switches checking off for everything inside it (unclaimed)\n"
            )),
            None => out.push_str(&format!(
                "{q}checks: [] — a written empty list: this document says to check nothing \
                 here, so nothing about this function is verified (unclaimed)\n"
            )),
        },
        Some(list) => {
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
            "{q}what this function promises — the last thing the document states before the \
             code itself takes over:\n"
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
        // Derived from the effective list, not from the contract existing.
        // A function can declare a promise and ask for nothing that would
        // test it -- a legacy boundary is exactly that shape -- and the
        // transcript used to say "the checks above test..." four lines under
        // its own "nothing about this function is verified" (external
        // review, 2026-08-30).
        let effective = effective_checks(fc, inherited);
        let nothing_runs = effective.is_none_or(|e| e.is_empty());
        out.push_str(&format!(
            "{r}{}\n",
            if nothing_runs {
                "nothing above checks this promise — it is written down, and this document \
                 asks for no check that would test it"
            } else {
                "the checks above test the function against exactly this promise"
            }
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
        // The verifier generates example tests only inside `if has_test`, so
        // examples under (say) `checks: [fuzz(64)]` are never compiled into
        // anything. Calling them tests told an author they were protected by
        // something inert (external review, 2026-08-30).
        let runs_examples =
            effective_checks(fc, inherited).is_some_and(|e| e.iter().any(|c| c.trim() == "test"));
        if !runs_examples {
            out.push_str(&format!(
                "{q}{n} worked {}, written down but not run: no check here asks for the \
                 declared examples, so nothing compiles them:\n",
                plural(n, "example", "examples"),
            ));
            let r = pad(level + 2);
            for e in &fc.examples {
                out.push_str(&format!("{r}{e}\n"));
            }
            return;
        }
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
        // Shared with the drawing rather than restated. This sentence existed
        // here as a byte-for-byte copy of the drawing's, which is one edit
        // away from the two views disagreeing about the same fact -- the
        // thing this module's own doc comment says cannot happen.
        out.push_str(&format!("{q}{}\n", unresolved_fn_pin_prose(u.id, &u.note)));
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
