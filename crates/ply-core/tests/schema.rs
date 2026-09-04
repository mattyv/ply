//! The-Ply-Spec.md §5: "The JSON Schema at `schema/ply.schema.json` is the
//! normative definition of the format". These are the tests that make that
//! sentence true rather than aspirational — the schema is embedded in the
//! binary, the key vocabulary every reader enforces is *read out of it*, and
//! the two places the grammar could drift apart (the serde model, the check
//! micro-syntax parser) are held against it by invariant, not by spot-check.

use ply_core::schema::{self, Level};

/// The document is a JSON Schema 2020-12 document, identified as such.
#[test]
fn the_schema_declares_the_2020_12_dialect() {
    let s = schema::schema();
    assert_eq!(
        s["$schema"], "https://json-schema.org/draft/2020-12/schema",
        "§5 names the dialect; a schema that does not declare it is not that schema"
    );
    assert!(s["$id"].is_string(), "the schema needs a stable $id");
}

/// §5.1a rule 1: "Every object in the schema sets `additionalProperties:
/// false`". Stated as an invariant over the whole document rather than a
/// list of levels, so an object added later cannot quietly skip the rule.
///
/// A *map* (`components:`, `externals:`, `profiles:`) is not an object in
/// this sense — its keys are user-chosen names, and it says so by giving
/// `additionalProperties` a schema instead of `false`. The rule therefore
/// binds exactly the nodes that declare a fixed `properties` vocabulary.
#[test]
fn every_object_with_a_fixed_key_vocabulary_forbids_unknown_keys() {
    let mut offenders = Vec::new();
    walk(schema::schema(), String::new(), &mut |node, pointer| {
        if node.get("properties").is_some()
            && node.get("additionalProperties") != Some(&false.into())
        {
            offenders.push(pointer.to_string());
        }
    });
    assert!(
        offenders.is_empty(),
        "these schema objects declare a fixed key vocabulary but accept unknown keys: {offenders:?}"
    );
}

/// The vocabulary the `E0204` diagnostic enforces is not a second list kept
/// beside the schema — it *is* the schema's `properties`. Deleting a key
/// from the schema must change what Ply accepts, or "normative" means
/// nothing.
#[test]
fn every_level_takes_its_key_vocabulary_from_the_schema() {
    for level in Level::ALL {
        let keys = schema::known_keys(level);
        assert!(
            !keys.is_empty(),
            "{level:?} resolved to no keys — its schema pointer is wrong"
        );
        for k in keys {
            assert!(
                schema::schema()
                    .pointer(&format!("{}/{k}", level.properties_pointer()))
                    .is_some(),
                "{level:?} key {k} is not in the schema"
            );
        }
    }
}

/// The other direction of the same bijection: every key the schema declares
/// is a key the serde model actually deserializes. A key in the schema that
/// the model drops is a promise the document makes and the tool never keeps
/// — vetting 004 finding 7 in miniature.
#[test]
fn every_key_the_schema_declares_is_a_key_the_model_reads() {
    let doc = ply_core::config::load_str(EVERY_KEY_DOCUMENT).expect("the exhaustive document");
    let c = &doc.components["pricing"];
    assert_eq!(c.anchor, "app::pricing");
    assert!(c.pure);
    assert!(c.strict);
    assert_eq!(c.uses, ["time"]);
    assert_eq!(c.owns, ["app::pricing::Book"]);
    assert_eq!(c.profile.as_deref(), Some("hot_path"));
    assert_eq!(
        c.checks.as_deref(),
        Some(["bounded(2)".to_string()].as_slice())
    );
    assert_eq!(c.components["curves"].anchor, "app::pricing::curves");
    let f = &c.fns["quote"];
    assert_eq!(
        f.checks.as_deref(),
        Some(
            [
                "fuzz(256)".to_string(),
                "test".to_string(),
                "mutate".to_string()
            ]
            .as_slice()
        )
    );
    assert_eq!(f.mode, ply_core::model::Mode::Synth);
    assert_eq!(f.requires, ["inst.tick > 0"]);
    assert_eq!(f.ensures, ["|result| result.bid <= result.ask"]);
    assert_eq!(f.examples, ["quote(1).bid == 4"]);
    assert_eq!(f.check_with["T"], "u64");
    assert_eq!(f.trusted[0].claim, "cross-thread safety");
    assert_eq!(f.trusted[0].evidence, "tests/loom_quote.rs");
    assert_eq!(f.unresolved[0].id, 147);
    assert_eq!(f.unresolved[0].note, "employee discount undecided");
    assert_eq!(f.entry, ["venue"]);
    let state = doc.components["pricing"]
        .state
        .as_ref()
        .expect("the exhaustive document declares a state");
    assert_eq!(state.of, "Book");
    assert_eq!(
        state
            .show
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        ["ticks"]
    );
    assert_eq!(doc.externals["venue"].note, "the exchange");
    assert_eq!(doc.edges.len(), 2);
    assert_eq!(doc.deny.len(), 1);
    assert_eq!(doc.profiles["hot_path"], ["no_panics"]);
    assert_eq!(doc.unresolved[0].id, 151);
    assert_eq!(doc.routes["Handle"], "open_handle");

    // And every key named above is exactly the schema's own vocabulary —
    // so a key added to the model without a schema entry fails here too.
    let named_in_document: std::collections::BTreeSet<&str> = EVERY_KEY_DOCUMENT
        .lines()
        .filter_map(|l| {
            l.trim()
                .strip_suffix(':')
                .or_else(|| l.trim().split(':').next())
        })
        .map(|s| s.trim_start_matches("- ").trim())
        .collect();
    for level in Level::ALL {
        for k in schema::known_keys(level) {
            assert!(
                named_in_document.contains(k.as_str()),
                "{k} is in the schema but not exercised by the exhaustive document"
            );
        }
    }
}

/// §7.1 (2026-09-04): `show:`'s mapping form declares one of seven shape
/// tokens, and the schema's own `enum` is the one place that vocabulary is
/// written down. This reads it out by JSON pointer and checks it against
/// exactly what the deserialiser accepts -- null included -- the same
/// discipline `the_schema_regex_and_the_parser_accept_exactly_the_same_check_strings`
/// already holds the check-string micro-syntax to, so the two lists cannot
/// quietly drift apart by hand.
#[test]
fn the_schema_declared_shape_enum_and_the_parser_accept_exactly_the_same_tokens() {
    let pointer =
        "/$defs/component/properties/state/properties/show/oneOf/1/additionalProperties/enum";
    let enum_value = schema::schema()
        .pointer(pointer)
        .unwrap_or_else(|| panic!("schema has no enum at {pointer}"))
        .as_array()
        .expect("the enum is a JSON array")
        .clone();

    // `null` is how a mapping entry with no value (`cursor:`) is accepted;
    // every other member is a plain string token.
    assert!(
        enum_value.iter().any(|v| v.is_null()),
        "the schema's enum must accept `null` -- an empty mapping value declares nothing: \
         {enum_value:?}"
    );
    let string_tokens: Vec<&str> = enum_value.iter().filter_map(|v| v.as_str()).collect();
    assert_eq!(
        string_tokens.len() + 1,
        enum_value.len(),
        "the enum must be the seven string tokens plus exactly one null: {enum_value:?}"
    );

    // Exact equality with the parser's own token table, not merely "each
    // schema token parses": that one-way check would stay green if a token
    // were added to the parser and forgotten in the schema, which is the
    // other half of the drift this test exists to refuse.
    let parser_tokens: Vec<&str> = ply_core::model::DeclaredShape::TOKENS
        .iter()
        .map(|(t, _)| *t)
        .collect();
    assert_eq!(
        string_tokens, parser_tokens,
        "the schema's enum and the parser's token table must be the same list -- one \
         vocabulary written down twice may not drift in either direction"
    );

    for token in &string_tokens {
        let doc = ply_core::model::parse_document(&format!(
            "ply: 1\ncomponents:\n  book:\n    anchor: app::book\n    state:\n      of: \
             Ledger\n      show:\n        f: {token}\n"
        ))
        .unwrap_or_else(|e| {
            panic!("the schema's enum accepts `{token}` but the parser refused it: {e}")
        });
        assert!(
            doc.components["book"].state.as_ref().unwrap().show[0]
                .declared
                .is_some(),
            "`{token}` is in the schema's enum, so the parser must read it as a real \
             declared shape, not silently drop it"
        );
    }

    assert!(
        ply_core::model::parse_document(
            "ply: 1\ncomponents:\n  book:\n    anchor: app::book\n    state:\n      of: \
             Ledger\n      show:\n        f: nonsense\n"
        )
        .is_err(),
        "a token the schema's enum does not list must be refused by the parser too"
    );
}

/// §5, item 1: the check micro-syntax is "schema-validated by regex, parsed
/// in ply-core". Two descriptions of one language, so they must accept the
/// same strings — this walks a corpus of forms that differ only in the ways
/// that have actually bitten (whitespace, leading zeros, a `+` sign, the
/// exact range endpoints) and fails on the first disagreement.
#[test]
fn the_schema_regex_and_the_parser_accept_exactly_the_same_check_strings() {
    let re = regex::Regex::new(schema::check_string_pattern()).unwrap();
    let corpus = [
        "test",
        "prove",
        "mutate",
        "  test  ",
        "tests",
        "Test",
        "",
        "fuzz(1)",
        "fuzz(256)",
        "fuzz(1000000)",
        "fuzz(1000001)",
        "fuzz(0)",
        "fuzz(0256)",
        "fuzz(+5)",
        "fuzz( 256 )",
        "fuzz(256",
        "fuzz()",
        "fuzz(abc)",
        "fuzz(-1)",
        "fuzz(2.5)",
        "bounded(1)",
        "bounded(9)",
        "bounded(10)",
        "bounded(64)",
        "bounded(65)",
        "bounded(0)",
        "bounded(08)",
        "bounded( 3 )",
        "bounded(3",
        "bounded(abc)",
    ];
    for s in corpus {
        let by_regex = re.is_match(s.trim());
        let by_parser = ply_core::model::parse_check(s).is_ok();
        assert_eq!(
            by_regex, by_parser,
            "{s:?}: schema regex says {by_regex}, parse_check says {by_parser} — the schema is \
             the normative definition, so the parser must agree with it exactly"
        );
    }
}

/// §5.1a rule 2: component, external and profile names are `[a-z][a-z0-9_]*`.
#[test]
fn the_identifier_pattern_is_the_one_section_5_1a_states() {
    let re = regex::Regex::new(schema::identifier_pattern()).unwrap();
    for ok in ["pricing", "db_raw", "a", "x9", "hot_path"] {
        assert!(re.is_match(ok), "{ok} should be a legal identifier");
    }
    for bad in [
        "Pricing",
        "_leading",
        "9lives",
        "kebab-case",
        "with space",
        "tráfico",
    ] {
        assert!(!re.is_match(bad), "{bad} should not be a legal identifier");
    }
}

/// A document with every single key §5 defines, at every level. Used by two
/// tests above: one reads it through the model, one checks the schema names
/// nothing it does not contain.
const EVERY_KEY_DOCUMENT: &str = r#"
ply: 1
externals:
  venue:
    note: "the exchange"
components:
  pricing:
    anchor: app::pricing
    pure: true
    strict: true
    uses: [time]
    owns: [app::pricing::Book]
    state:
      of: Book
      show: [ticks]
      holds: ["state.ticks > 0"]
    profile: hot_path
    checks: [bounded(2)]
    components:
      curves:
        anchor: app::pricing::curves
    fns:
      quote:
        checks: [fuzz(256), test, mutate]
        mode: synth
        requires: ["inst.tick > 0"]
        ensures: ["|result| result.bid <= result.ask"]
        examples: ["quote(1).bid == 4"]
        check_with: { T: u64 }
        trusted:
          - claim: "cross-thread safety"
            evidence: "tests/loom_quote.rs"
        unresolved:
          - id: 147
            note: "employee discount undecided"
        entry: [venue]
edges:
  - "pricing -> parser"
  - "pricing ~> venue : app::pricing::Quote"
deny:
  - "* -> db_raw except migrations"
profiles:
  hot_path: [no_panics]
unresolved:
  - id: 151
    note: "settlement rounding rule TBD"
routes:
  Handle: open_handle
"#;

/// Every node of the schema document, with its JSON pointer.
fn walk(node: &serde_json::Value, pointer: String, f: &mut impl FnMut(&serde_json::Value, &str)) {
    if let Some(map) = node.as_object() {
        f(node, &pointer);
        for (k, v) in map {
            walk(v, format!("{pointer}/{k}"), f);
        }
    }
}

/// §5.1a rule 3, the same bijection as the check-string test above: the
/// schema's `code_path` pattern and `is_valid_path_form` (which raises
/// `E0304`) must accept the same paths.
#[test]
fn the_schema_pattern_and_the_path_checker_accept_exactly_the_same_paths() {
    let re = regex::Regex::new(schema::code_path_pattern()).unwrap();
    let corpus = [
        "pricing",
        "app::pricing",
        "app::pricing::curves",
        "Quote::new",
        "_private::f",
        "",
        "app::",
        "::app",
        "app::Foo<T>",
        "<T as Trait>::f",
        "app::foo'a",
        "app pricing",
        "app.pricing",
        "9lives",
    ];
    for s in corpus {
        let by_regex = re.is_match(s);
        let by_checker = ply_core::check::is_valid_path_form(s);
        assert_eq!(
            by_regex, by_checker,
            "{s:?}: schema pattern says {by_regex}, is_valid_path_form says {by_checker} — \
             E0304 must refuse exactly what the schema refuses"
        );
    }
}

/// §5.4b's cross-crate extension (defect 2, 2026-09-02): a `routes:` value
/// may now carry an explicit input-type list in parens
/// (`std::ffi::OsString::from(String)`), alongside the original bare-path
/// form (`code_path`'s own pattern). `schema/ply.schema.json`'s own
/// `route_value` definition must accept both shapes, and reject a value
/// that is neither -- documenting the grammar this task adds, the same way
/// `code_path_pattern`'s own test documents the original one.
#[test]
fn the_schema_route_value_pattern_accepts_both_route_forms() {
    let re = regex::Regex::new(schema::route_value_pattern()).unwrap();
    for good in [
        "open_handle",
        "Token::via_route",
        "std::ffi::OsString::from(String)",
        "std::ffi::OsString::from()",
        "some::path::f(String, u32)",
    ] {
        assert!(
            re.is_match(good),
            "{good:?} must match the route value pattern"
        );
    }
    for bad in ["not a type( at all", "open handle", "f(", "f)", ""] {
        assert!(
            !re.is_match(bad),
            "{bad:?} must not match the route value pattern"
        );
    }
}

/// §9's schema goldens: "a fixture set of valid and invalid `ply.yaml`
/// documents pins validation behavior and E0201 pointer paths".
///
/// Every valid fixture must produce no violation at all *and* load; every
/// invalid one must produce exactly the diagnostics recorded beside it,
/// pointer path included. The goldens are reviewed like API diffs — a
/// changed `.expected` file is a changed promise to a user.
#[test]
fn the_valid_fixtures_produce_no_schema_violation_and_load() {
    for path in fixtures("valid") {
        let text = std::fs::read_to_string(&path).unwrap();
        let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&text).unwrap();
        let violations = schema::validate(&value);
        assert!(
            violations.is_empty(),
            "{} should be clean, got: {violations:#?}",
            path.display()
        );
        ply_core::config::load_str(&text)
            .unwrap_or_else(|e| panic!("{} should load: {e}", path.display()));
    }
}

#[test]
fn every_invalid_fixture_reports_exactly_its_recorded_diagnostics() {
    for path in fixtures("invalid") {
        let text = std::fs::read_to_string(&path).unwrap();
        let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&text).unwrap();
        let actual: String = schema::validate(&value)
            .iter()
            .map(|v| format!("{v}\n"))
            .collect();
        let golden = path.with_extension("expected");
        let expected = std::fs::read_to_string(&golden).unwrap_or_else(|_| {
            panic!(
                "no golden at {} — write it from a reviewed run, never from a blind accept",
                golden.display()
            )
        });
        assert_eq!(
            actual,
            expected,
            "{} no longer reports what its golden records",
            path.display()
        );
        assert!(
            !actual.is_empty(),
            "{} is filed as invalid but produced nothing",
            path.display()
        );
    }
}

fn fixtures(kind: &str) -> Vec<std::path::PathBuf> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/schema")
        .join(kind);
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".ply.yaml"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no fixtures in {}", dir.display());
    paths
}
