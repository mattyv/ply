//! The normative `ply.yaml` schema (The-Ply-Spec.md §5), embedded.
//!
//! §5 has always said "the JSON Schema at `schema/ply.schema.json` is the
//! normative definition of the format", and until Phase 1a the file did not
//! exist. It does now, and it is not decoration: the key vocabulary every
//! reader enforces (`E0204`) is read out of this document at runtime, so
//! deleting a key from the schema changes what Ply accepts.
//!
//! Two constraints are declared here and enforced by hand-written code
//! rather than by a regex engine — the check-string form
//! ([`crate::model::parse_check`]) and the code-path form
//! ([`crate::check::is_valid_path_form`]). That is a deliberate trade: no
//! regex crate in the shipping binary, at the price of one more thing that
//! could drift. The price is paid by invariant tests in
//! `tests/schema.rs`, which walk a corpus and fail on the first string the
//! schema and the parser disagree about.
//!
//! §5's other half: **Ply never reads a schema from the target workspace.**
//! This embedded copy is authoritative; the file in the repo is read-only
//! reference and IDE fodder.

use std::sync::OnceLock;

use serde_json::Value;

/// The schema, verbatim. `cargo ply skill` embeds this in the generated
/// skill file (§6) and `check` validates against it.
pub const SCHEMA_JSON: &str = include_str!("../../../schema/ply.schema.json");

/// The parsed schema. Panics on a malformed schema, which is a build-time
/// defect in this repo, never anything a user can cause.
pub fn schema() -> &'static Value {
    static PARSED: OnceLock<Value> = OnceLock::new();
    PARSED.get_or_init(|| {
        serde_json::from_str(SCHEMA_JSON).expect("schema/ply.schema.json is valid JSON")
    })
}

/// One level of the document that has a fixed key vocabulary — the levels
/// `additionalProperties: false` binds, and so the levels an unknown key
/// (`E0204`) can be found at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Document,
    Component,
    FnClaim,
    External,
    Trusted,
    Unresolved,
}

impl Level {
    pub const ALL: [Level; 6] = [
        Level::Document,
        Level::Component,
        Level::FnClaim,
        Level::External,
        Level::Trusted,
        Level::Unresolved,
    ];

    /// Where this level's schema object lives, as a JSON pointer.
    fn definition_pointer(self) -> &'static str {
        match self {
            Level::Document => "",
            Level::Component => "/$defs/component",
            Level::FnClaim => "/$defs/fn_claim",
            Level::External => "/$defs/external",
            Level::Trusted => "/$defs/trusted_claim",
            Level::Unresolved => "/$defs/unresolved_entry",
        }
    }

    /// Where this level's `properties` map lives, as a JSON pointer.
    pub fn properties_pointer(self) -> String {
        format!("{}/properties", self.definition_pointer())
    }

    /// What this level is, in words a reader can act on — the schema's own
    /// `title`, so the diagnostic and the schema cannot describe the same
    /// level differently.
    pub fn name(self) -> &'static str {
        static NAMES: OnceLock<Vec<&'static str>> = OnceLock::new();
        NAMES.get_or_init(|| {
            Level::ALL
                .iter()
                .map(|l| {
                    let node = schema()
                        .pointer(l.definition_pointer())
                        .expect("every level has a schema definition");
                    node["title"]
                        .as_str()
                        .expect("every level's schema definition has a title")
                })
                .collect()
        })[Level::ALL.iter().position(|l| *l == self).unwrap()]
    }
}

/// Every key this level accepts, in the schema's own order (sorted, since
/// that is how a JSON object round-trips). This *is* the `E0204`
/// vocabulary: there is no second list.
pub fn known_keys(level: Level) -> &'static [String] {
    static KEYS: OnceLock<Vec<Vec<String>>> = OnceLock::new();
    &KEYS.get_or_init(|| {
        Level::ALL
            .iter()
            .map(|l| {
                schema()
                    .pointer(&l.properties_pointer())
                    .and_then(Value::as_object)
                    .expect("every level has a properties map")
                    .keys()
                    .cloned()
                    .collect()
            })
            .collect()
    })[Level::ALL.iter().position(|l| *l == level).unwrap()]
}

/// The `[a-z][a-z0-9_]*` pattern from the schema, for the tests that hold
/// [`is_identifier`] to it.
pub fn identifier_pattern() -> &'static str {
    pattern("/$defs/identifier")
}

/// The check-string pattern from the schema, for the test that holds
/// [`crate::model::parse_check`] to it.
pub fn check_string_pattern() -> &'static str {
    pattern("/$defs/check_string")
}

/// The code-path pattern from the schema, for the test that holds
/// [`crate::check::is_valid_path_form`] to it.
pub fn code_path_pattern() -> &'static str {
    pattern("/$defs/code_path")
}

/// The keys `/$defs/{level}`'s `required` array names, and for each the
/// schema's own `description` of it — so a "this is missing" diagnostic
/// explains what the key is for without a second copy of that sentence.
fn required_keys(level: Level) -> Vec<(String, String)> {
    let def = schema()
        .pointer(level.definition_pointer())
        .expect("every level has a schema definition");
    let Some(required) = def.get("required").and_then(Value::as_array) else {
        return Vec::new();
    };
    required
        .iter()
        .filter_map(Value::as_str)
        .map(|k| {
            let why = def
                .pointer(&format!("/properties/{k}/description"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            (k.to_string(), why)
        })
        .collect()
}

fn pattern(def_pointer: &str) -> &'static str {
    schema()
        .pointer(&format!("{def_pointer}/pattern"))
        .and_then(Value::as_str)
        .expect("this definition declares a pattern")
}

/// The values `/$defs/{def}`'s `enum` allows.
fn enum_values(def: &str) -> &'static [String] {
    static VALUES: OnceLock<std::collections::BTreeMap<String, Vec<String>>> = OnceLock::new();
    VALUES
        .get_or_init(|| {
            ["capability", "ban"]
                .iter()
                .map(|d| {
                    let vs = schema()
                        .pointer(&format!("/$defs/{d}/enum"))
                        .and_then(Value::as_array)
                        .expect("this definition declares an enum")
                        .iter()
                        .map(|v| v.as_str().expect("enum values are strings").to_string())
                        .collect();
                    ((*d).to_string(), vs)
                })
                .collect()
        })
        .get(def)
        .expect("known enum definition")
}

/// The capabilities `uses:` accepts (§5.1), read from the schema.
pub fn capabilities() -> &'static [String] {
    enum_values("capability")
}

/// The bans a profile may impose (§5.1), read from the schema.
pub fn bans() -> &'static [String] {
    enum_values("ban")
}

/// §5.1a rule 2, by hand: `[a-z][a-z0-9_]*`, ASCII. Held to
/// [`identifier_pattern`] by an invariant test.
pub fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// One schema violation, with the JSON pointer §5 asks for.
///
/// **The source line is not here.** §5 also asks for it ("a second
/// lightweight pass over a position-marked YAML parse builds a JSON-pointer
/// → (line, col) index"); that index does not exist yet, and a guessed line
/// number is worse than none — it sends a reader to the wrong place with
/// full confidence. The pointer is exact, and it is what this carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaViolation {
    pub code: &'static str,
    pub pointer: String,
    pub message: String,
}

impl std::fmt::Display for SchemaViolation {
    /// Code, then the sentence. The sentence already says where the defect
    /// is, in the dotted form a person reads a YAML file in; the JSON
    /// pointer is structured data for `--json`, not something to repeat at
    /// a human (§5 asks for the pointer, not for it to be shouted twice).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

fn violation(code: &'static str, pointer: String, message: String) -> SchemaViolation {
    SchemaViolation {
        code,
        pointer,
        message,
    }
}

/// Validates a parsed YAML document against the constraints this schema
/// states and Ply enforces at load time: the version, the key vocabulary
/// (`E0204`), identifier names, the capability and ban vocabularies, and
/// positive unresolved ids. Returns every violation it finds, in document
/// order, so a user fixing a document sees the whole list rather than one
/// error per run.
///
/// Not here, on purpose: check strings (enforced wherever a checks list is
/// consumed, as `E0203`, which can name the fn it belongs to), edge and deny
/// strings (same), anchors and fn-key path forms (`E0304`, likewise), and
/// duplicate names or ids (`E0202`/`E0205`, which need the whole merged
/// tree). Each of those has a diagnostic that can say *where*, which a
/// pointer-only schema pass cannot.
/// [`validate`] over raw document text. `Err` carries the YAML parser's own
/// message: a document that is not YAML has no structure to report findings
/// about, and that is a different failure from a document that parses and is
/// wrong.
pub fn validate_text(text: &str) -> Result<Vec<SchemaViolation>, String> {
    let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(text).map_err(|e| e.to_string())?;
    Ok(validate(&value))
}

pub fn validate(doc: &serde_yaml_ng::Value) -> Vec<SchemaViolation> {
    let mut out = Vec::new();
    let Some(map) = doc.as_mapping() else {
        return out;
    };

    match map.get("ply") {
        None => out.push(violation(
            "E0201",
            "/ply".into(),
            "this document has no `ply:` line, so Ply cannot tell which version of the format \
             it is written in. Add `ply: 1` as the first line."
                .into(),
        )),
        Some(v) if v.as_u64() != Some(1) => out.push(violation(
            "E0201",
            "/ply".into(),
            format!(
                "`ply: {}` names a version of the ply.yaml format this build of Ply does not \
                 speak. This build reads version 1. The format changes only through Ply \
                 releases, so either upgrade Ply or set `ply: 1`.",
                yaml_scalar_text(v)
            ),
        )),
        Some(_) => {}
    }

    check_keys(doc, Level::Document, "", &mut out);
    check_named_map(doc, "components", &mut out);
    check_named_map(doc, "externals", &mut out);
    check_named_map(doc, "profiles", &mut out);

    if let Some(externals) = map.get("externals").and_then(|v| v.as_mapping()) {
        for (name, ext) in externals {
            let name = name.as_str().unwrap_or("?");
            check_keys(
                ext,
                Level::External,
                &format!("/externals/{name}"),
                &mut out,
            );
        }
    }
    if let Some(profiles) = map.get("profiles").and_then(|v| v.as_mapping()) {
        for (name, list) in profiles {
            let name = name.as_str().unwrap_or("?");
            check_enum_list(list, bans(), "ban", &format!("/profiles/{name}"), &mut out);
        }
    }
    if let Some(components) = map.get("components").and_then(|v| v.as_mapping()) {
        for (name, comp) in components {
            let name = name.as_str().unwrap_or("?");
            check_component(comp, &format!("/components/{name}"), &mut out);
        }
    }
    check_unresolved_list(doc, "", &mut out);
    out
}

fn check_component(comp: &serde_yaml_ng::Value, pointer: &str, out: &mut Vec<SchemaViolation>) {
    check_keys(comp, Level::Component, pointer, out);
    if let Some(uses) = comp.get("uses") {
        check_enum_list(
            uses,
            capabilities(),
            "capability",
            &format!("{pointer}/uses"),
            out,
        );
    }
    if let Some(profile) = comp.get("profile").and_then(|v| v.as_str())
        && !is_identifier(profile)
    {
        out.push(identifier_violation(
            profile,
            &format!("{pointer}/profile"),
            "a profile name",
        ));
    }
    if let Some(fns) = comp.get("fns").and_then(|v| v.as_mapping()) {
        for (fn_name, claim) in fns {
            let fn_name = fn_name.as_str().unwrap_or("?");
            let p = format!("{pointer}/fns/{fn_name}");
            check_keys(claim, Level::FnClaim, &p, out);
            if let Some(entries) = claim.get("entry").and_then(|v| v.as_sequence()) {
                for (i, e) in entries.iter().enumerate() {
                    if let Some(name) = e.as_str()
                        && !is_identifier(name)
                    {
                        out.push(identifier_violation(
                            name,
                            &format!("{p}/entry/{i}"),
                            "an external's name",
                        ));
                    }
                }
            }
            if let Some(trusted) = claim.get("trusted").and_then(|v| v.as_sequence()) {
                for (i, t) in trusted.iter().enumerate() {
                    check_keys(t, Level::Trusted, &format!("{p}/trusted/{i}"), out);
                }
            }
            check_unresolved_list(claim, &p, out);
        }
    }
    if let Some(nested) = comp.get("components").and_then(|v| v.as_mapping()) {
        for (name, child) in nested {
            let name = name.as_str().unwrap_or("?");
            if let Some(n) = Some(name)
                && !is_identifier(n)
            {
                out.push(identifier_violation(
                    n,
                    &format!("{pointer}/components/{n}"),
                    "a component name",
                ));
            }
            check_component(child, &format!("{pointer}/components/{name}"), out);
        }
    }
}

fn check_unresolved_list(
    parent: &serde_yaml_ng::Value,
    pointer: &str,
    out: &mut Vec<SchemaViolation>,
) {
    let Some(list) = parent.get("unresolved").and_then(|v| v.as_sequence()) else {
        return;
    };
    for (i, entry) in list.iter().enumerate() {
        let p = format!("{pointer}/unresolved/{i}");
        check_keys(entry, Level::Unresolved, &p, out);
        if let Some(id) = entry.get("id")
            && id.as_u64() == Some(0)
        {
            out.push(violation(
                "E0201",
                format!("{p}/id"),
                format!(
                    "`id: 0` is not a usable number for an open decision: ids start at 1, and \
                     0 reads as \"unset\" everywhere else the number is shown. Give this \
                     entry its own positive number. Found at `{at}` in ply.yaml.",
                    at = dotted(&format!("{p}/id")),
                ),
            ));
        }
    }
}

/// Every top-level map whose keys are names the user chose: each must be a
/// §5.1a rule 2 identifier.
fn check_named_map(doc: &serde_yaml_ng::Value, field: &str, out: &mut Vec<SchemaViolation>) {
    let Some(map) = doc.get(field).and_then(|v| v.as_mapping()) else {
        return;
    };
    let kind = match field {
        "components" => "a component name",
        "externals" => "an external's name",
        _ => "a profile name",
    };
    for name in map.keys() {
        let Some(name) = name.as_str() else { continue };
        if !is_identifier(name) {
            out.push(identifier_violation(
                name,
                &format!("/{field}/{name}"),
                kind,
            ));
        }
    }
}

fn identifier_violation(name: &str, pointer: &str, kind: &str) -> SchemaViolation {
    violation(
        "E0201",
        pointer.to_string(),
        format!(
            "{name:?} cannot be used as {kind}. Names in a ply.yaml are written the way Rust \
             module names are: a lowercase letter first, then lowercase letters, digits or \
             underscores — `pricing`, `db_raw`, `hot_path`. They are Ply's own names for the \
             pieces of your system, not Rust paths, so capitals, dashes, dots and spaces have \
             no meaning here. Found at `{at}` in ply.yaml.",
            at = dotted(pointer),
        ),
    )
}

fn check_enum_list(
    list: &serde_yaml_ng::Value,
    allowed: &[String],
    kind: &str,
    pointer: &str,
    out: &mut Vec<SchemaViolation>,
) {
    let Some(seq) = list.as_sequence() else {
        return;
    };
    for (i, item) in seq.iter().enumerate() {
        let Some(name) = item.as_str() else { continue };
        if allowed.iter().any(|a| a == name) {
            continue;
        }
        let list = allowed
            .iter()
            .map(|a| format!("`{a}`"))
            .collect::<Vec<_>>()
            .join(", ");
        out.push(violation(
            "E0201",
            format!("{pointer}/{i}"),
            format!(
                "{name:?} is not {what} Ply knows. The ones it knows are: {list}. A name \
                 outside that set is almost always a typo, and Ply refuses it rather than \
                 quietly enforcing nothing. Found at `{at}` in ply.yaml.",
                what = if kind == "ban" {
                    "a ban"
                } else {
                    "a capability"
                },
                at = dotted(&format!("{pointer}/{i}")),
            ),
        ));
    }
}

/// Both halves of a level's key contract: nothing unknown (`E0204`) and
/// nothing required missing (`E0201`).
fn check_keys(
    value: &serde_yaml_ng::Value,
    level: Level,
    pointer: &str,
    out: &mut Vec<SchemaViolation>,
) {
    let Some(map) = value.as_mapping() else {
        return;
    };
    // The document's own `ply:` requirement has a bespoke sentence (there is
    // exactly one of it, and "add `ply: 1` as the first line" is more use
    // than a generic phrasing), so it is handled in `validate` instead.
    if level != Level::Document {
        for (key, why) in required_keys(level) {
            if map.get(&key).is_some() {
                continue;
            }
            let at = format!("{pointer}/{key}");
            out.push(violation(
                "E0201",
                at.clone(),
                format!(
                    "{level_name} needs its `{key}:` line, and this one has none. {why} Found \
                     at `{loc}` in ply.yaml.",
                    level_name = level.name(),
                    loc = dotted(pointer),
                ),
            ));
        }
    }
    let known = known_keys(level);
    for key in map.keys() {
        let Some(name) = key.as_str() else { continue };
        if known.iter().any(|k| k == name) {
            continue;
        }
        let at = format!("{pointer}/{name}");
        let message = unknown_key_message(name, level, &at);
        out.push(violation("E0204", at, message));
    }
}

/// A JSON pointer rendered the way a person reads a YAML document:
/// `/components/pricing/fns/quote/ensure` becomes
/// `components.pricing.fns.quote.ensure`, and a list index becomes `[i]`.
/// The pointer is what a machine reads (§5); this is what the sentence says.
pub fn dotted(pointer: &str) -> String {
    let mut out = String::new();
    for seg in pointer.split('/').skip(1) {
        if seg.bytes().all(|b| b.is_ascii_digit()) && !seg.is_empty() {
            out.push('[');
            out.push_str(seg);
            out.push(']');
        } else {
            if !out.is_empty() {
                out.push('.');
            }
            out.push_str(seg);
        }
    }
    out
}

/// The one `E0204` sentence. `level` supplies both the accepted-key list and
/// the words for what the level is, straight out of the schema.
pub fn unknown_key_message(name: &str, level: Level, pointer: &str) -> String {
    let known = known_keys(level);
    let suggestion = match nearest_key(name, known) {
        Some(s) => format!(" Did you mean `{s}`?"),
        None => String::new(),
    };
    format!(
        "`{name}:` is not a key Ply knows. The keys {level_name} accepts are: {list}.{suggestion} \
         A key Ply does not know is almost always a typo, and a typo has to be caught rather \
         than ignored (§5.1a rule 1) -- an ignored key is a contract you think you wrote and \
         Ply never read. Found at `{where_at}` in ply.yaml.",
        level_name = level.name(),
        where_at = dotted(pointer),
        list = known
            .iter()
            .map(|k| format!("`{k}`"))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

/// Plain Levenshtein distance, used only to pick the closest known key for
/// an E0204 suggestion.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// The closest known key, when one is close enough to be worth naming.
pub fn nearest_key(unknown: &str, known: &[String]) -> Option<String> {
    known
        .iter()
        .map(|k| (edit_distance(unknown, k), k))
        .filter(|(d, k)| *d <= 3 || k.starts_with(unknown) || unknown.starts_with(k.as_str()))
        .min_by_key(|(d, _)| *d)
        .map(|(_, k)| k.clone())
}

/// A scalar as the user typed it, for quoting back in a diagnostic.
fn yaml_scalar_text(v: &serde_yaml_ng::Value) -> String {
    match v {
        serde_yaml_ng::Value::String(s) => s.clone(),
        other => serde_yaml_ng::to_string(other)
            .unwrap_or_default()
            .trim()
            .to_string(),
    }
}
