//! §5.4b's generator hook (this task, 2026-09-02): "a type is buildable if
//! there is a public way to get one from parts Ply can already build."
//! `routehook` is the routeprobe's own shape, permanent in-repo: a struct
//! made only by a free function (`Handle`/`open_handle`), the associated-
//! constructor case that already worked (`Token`/`parse_unchecked`), and the
//! one failure a stale-route compile error cannot catch on its own -- a
//! route that ignores its own input (`Stuck`/`make_stuck`).

use ply_e2e::{build_cargo_ply, copy_fixture, run_verify};

fn find_fn_node<'a>(node: &'a serde_json::Value, id: &str) -> Option<&'a serde_json::Value> {
    if node["id"] == id {
        return Some(node);
    }
    node["children"]
        .as_array()?
        .iter()
        .find_map(|c| find_fn_node(c, id))
}

fn statuses_of(node: &serde_json::Value) -> Vec<&str> {
    node["statuses"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default()
}

/// The probe's own three cases, permanent regression: a struct with a
/// private field made only by a free function now builds (and its false
/// promise gives a real violation, proving the check actually runs against
/// route-built values rather than merely accepting them); a list of that
/// same struct builds too, through the same composition grammar that
/// already closes over a constructor-built type; and the associated-
/// constructor case is unchanged.
#[test]
fn a_type_made_only_by_a_free_function_is_built_via_its_declared_route() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("routehook");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    // `use_handle`'s promise (`*result < 100`) is false on almost every
    // `u32` id Ply's own generator draws -- proving the check actually
    // bites on a route-built value, not merely that it runs.
    let use_handle = find_fn_node(&run.json["root"], "use_handle")
        .unwrap_or_else(|| panic!("no node for use_handle in {}", run.json));
    assert_eq!(
        use_handle["verdict"], "violation",
        "a promise this false on a route-built value must fail for real: {}",
        run.json
    );
    assert!(
        statuses_of(use_handle).contains(&"route-built"),
        "a violation built through a declared route must still carry the mark naming where \
         its evidence came from: {}",
        run.json
    );

    // The route composes inside a `Vec`, the same as a constructor-built
    // type already does.
    let many = find_fn_node(&run.json["root"], "use_many_handles")
        .unwrap_or_else(|| panic!("no node for use_many_handles in {}", run.json));
    assert_eq!(
        many["verdict"], "fuzzed(64)",
        "a list of a route-built type must compile and run for real: {}",
        run.json
    );
    assert!(
        statuses_of(many).contains(&"route-built"),
        "nested inside a Vec or not, this value still came through a declared route: {}",
        run.json
    );

    // The associated-constructor case: already worked before this task,
    // unchanged by it, and must carry no route mark at all.
    let token = find_fn_node(&run.json["root"], "use_token")
        .unwrap_or_else(|| panic!("no node for use_token in {}", run.json));
    assert_eq!(token["verdict"], "fuzzed(64)");
    assert!(
        statuses_of(token).is_empty(),
        "an ordinary constructor Ply found on its own is not a declared route, and must not \
         be marked as one: {}",
        run.json
    );
}

/// The guard this feature cannot ship without (TODO.md): a route that
/// ignores its own input and returns the same value every time must not
/// pass silently as though 64 genuinely different cases had run.
#[test]
fn a_route_that_ignores_its_input_is_caught_and_named() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("routehook");
    let run = run_verify(&cargo_ply, fixture.path(), 90);

    let stuck = find_fn_node(&run.json["root"], "use_stuck")
        .unwrap_or_else(|| panic!("no node for use_stuck in {}", run.json));
    // The guard is disclosure, never a verdict change (CLAUDE.md: the same
    // rule the branch-decided measurement and the high-rejection warning
    // both already follow).
    assert_eq!(
        stuck["verdict"], "fuzzed(64)",
        "the guard must disclose the collapse, never invent a failure that never happened: {}",
        run.json
    );
    assert!(
        statuses_of(stuck).contains(&"route-collapsed"),
        "a route that built one distinct value across 64 cases must carry the collapse mark: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let disclosure = diagnostics
        .iter()
        .find(|d| d["node_id"] == "routehook::use_stuck" && d["code"] == "W0527")
        .unwrap_or_else(|| panic!("no W0527 distinct-value disclosure: {}", run.json));
    assert_eq!(
        disclosure["severity"], "warning",
        "every one of 64 cases building the same value is exactly the collapse this guard \
         exists to catch: {}",
        disclosure
    );
    // Exact-string (CLAUDE.md: "assert the sentence a user reads, exact-
    // string"), pinned against a real run of this fixture.
    assert_eq!(
        disclosure["title"].as_str().unwrap(),
        "`use_stuck`'s `s` parameter is built by calling `make_stuck` -- the function ply.yaml \
         names as the way to make one, rather than a value Ply's own generator drew directly. \
         Of the 64 cases that ran, 1 distinct value reached `use_stuck`. (W0527)"
    );
    // This fixture's own `use_handle` is a real, separate violation (proven
    // by the other test above), so the *whole run's* exit code is already
    // nonzero for a reason that has nothing to do with this guard -- the
    // fact worth pinning here is narrower: nothing this guard itself
    // reported for `use_stuck` is error-severity, which is what "a warning
    // must not fail the run on its own" actually means.
    assert!(
        diagnostics
            .iter()
            .all(|d| d["node_id"] != "routehook::use_stuck" || d["severity"] != "error"),
        "the degenerate-route guard is a warning, never an error, the same rule the \
         branch-decided measurement's own warning already follows: {}",
        run.json
    );
}

/// The stale-route requirement: renaming the function a route names must
/// fail loudly and name the function it looked for -- never a silent fall
/// through to rule 2's direct field construction, and never a quiet drop of
/// the claim.
#[test]
fn a_stale_route_is_refused_loudly_and_names_the_function() {
    let cargo_ply = build_cargo_ply();
    let fixture = copy_fixture("routehook");
    let src = std::fs::read_to_string(fixture.lib_rs_path()).unwrap();
    assert!(
        src.contains("pub fn open_handle("),
        "fixture no longer declares `open_handle` as expected -- update this test's rename \
         alongside it"
    );
    // ply.yaml still names `open_handle`; the crate no longer declares it.
    let renamed = src.replace("pub fn open_handle(", "pub fn open_handle_renamed(");
    std::fs::write(fixture.lib_rs_path(), renamed).unwrap();

    let run = run_verify(&cargo_ply, fixture.path(), 90);

    assert_ne!(
        run.exit_code,
        Some(0),
        "a stale route must fail the run -- it earns no evidence at all, and §1 says an \
         absence of evidence fails by default: {}",
        run.json
    );

    let use_handle = find_fn_node(&run.json["root"], "use_handle")
        .unwrap_or_else(|| panic!("no node for use_handle in {}", run.json));
    assert_ne!(
        use_handle["verdict"], "fuzzed(64)",
        "a renamed route must not silently keep reporting the old verdict: {}",
        run.json
    );

    let diagnostics = run.json["diagnostics"].as_array().unwrap();
    let stale = diagnostics
        .iter()
        .find(|d| {
            d["node_id"] == "routehook::use_handle"
                && d["title"]
                    .as_str()
                    .is_some_and(|t| t.contains("open_handle"))
        })
        .unwrap_or_else(|| {
            panic!(
                "no diagnostic names the stale route `open_handle` at all -- a renamed route \
                 must be a loud, named failure, not a silent skip: {}",
                run.json
            )
        });
    let title = stale["title"].as_str().unwrap();
    assert!(
        title.contains("could not find fn `open_handle`"),
        "the diagnostic must say Ply looked for the declared function and failed to find it, \
         not merely that the type is unsupported: {title}"
    );
}
