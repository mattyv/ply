import json
import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
FIXTURES = Path(__file__).with_name("fixtures")


def skill_text(name: str) -> str:
    return (ROOT / "skills" / name / "SKILL.md").read_text()


def table(text: str, heading: str) -> dict[str, list[str]]:
    match = re.search(
        rf"^## {re.escape(heading)}\s*$\n(?P<body>.*?)(?=^## |\Z)",
        text,
        flags=re.MULTILINE | re.DOTALL,
    )
    if not match:
        raise AssertionError(f"missing {heading!r} table")
    rows = []
    for line in match.group("body").splitlines():
        if not line.startswith("|"):
            continue
        cells = [cell.strip() for cell in line.strip("|").split("|")]
        if cells and not all(set(cell) <= {"-", ":"} for cell in cells):
            rows.append(cells)
    if len(rows) < 2:
        raise AssertionError(f"{heading!r} table has no data rows")
    return {row[0]: row[1:] for row in rows[1:]}


def fixture(name: str):
    return json.loads((FIXTURES / name).read_text())


class PlyVerifySkillTests(unittest.TestCase):
    def test_public_result_fixtures_follow_the_documented_completion_policy(self):
        policy = table(skill_text("ply-verify"), "Result policy")
        for result in fixture("verify-results.json"):
            with self.subTest(scenario=result["scenario"]):
                completion, next_action, publication = policy[result["scenario"]]
                self.assertEqual(completion, result["expectedCompletion"])
                self.assertEqual(publication, result["expectedPublication"])
                self.assertEqual(result["exitCode"] == 0, completion == "may-complete")
                self.assertIn(result["expectedActionFragment"], next_action)

    def test_result_policy_never_calls_incomplete_evidence_complete(self):
        policy = table(skill_text("ply-verify"), "Result policy")
        self.assertEqual(policy["clean"][0], "may-complete")
        for scenario in (
            "violation",
            "missing_evidence",
            "narrowed_evidence",
            "timeout",
        ):
            with self.subTest(scenario=scenario):
                self.assertEqual(policy[scenario][0], "must-not-complete")
                self.assertEqual(policy[scenario][2], "only-by-explicit-flag")
        self.assertEqual(policy["internal_tool_error"][0], "must-not-complete")
        self.assertEqual(policy["internal_tool_error"][2], "unavailable")

    def test_change_authority_stops_before_weakening_intent(self):
        authority = table(skill_text("ply-verify"), "Change authority")
        self.assertEqual(authority["implementation"][0], "may-edit")
        for protected in (
            "contract",
            "declared_check",
            "evidence_requirement",
            "architecture_contract",
        ):
            with self.subTest(protected=protected):
                self.assertEqual(authority[protected][0], "ask-first")

    def test_only_public_cli_commands_are_prescribed(self):
        """Every command the skill tells an agent to run is either a public
        Ply command or plain `cargo test`.

        `cargo test` earns its place because it is the repair loop: the
        counterexample Ply writes is ordinary Rust that fails with no engine
        installed, and iterating against it is the whole reason that file is
        written into the crate rather than kept in scratch. What stays
        banned is a command that reaches past the public surface, and
        `--fail-on error`, which turns absent evidence into success.
        """
        commands = re.findall(
            r"```(?:bash|sh)\n(.*?)```", skill_text("ply-verify"), re.DOTALL
        )
        self.assertTrue(commands, "skill must show the public workflow")
        for block in commands:
            for line in block.splitlines():
                if not line.strip():
                    continue
                self.assertRegex(
                    line, r"^(cargo ply (check|verify|explain) |cargo test\b)"
                )
                self.assertNotIn("--fail-on error", line)


class PlyAuthorSkillTests(unittest.TestCase):
    def test_authoring_never_takes_authority_over_an_existing_promise(self):
        """Adding a promise is authoring. Weakening or deleting one that
        already exists is a decision about what the codebase may do, and it
        is the developer's -- most of all when the reason is that a check is
        failing, which is exactly when the temptation is highest."""
        authority = table(skill_text("ply-author"), "Change authority")
        for target in (
            "existing_contract",
            "existing_check",
            "existing_architecture_rule",
            "deleting_any_declaration",
        ):
            self.assertIn(target, authority, f"{target} must have a stated authority")
            self.assertEqual(
                authority[target][0],
                "ask-first",
                f"{target} must never be edited without the developer",
            )

    def test_authoring_prescribes_checking_before_declaring_more(self):
        """The failure this skill exists to prevent is a document full of
        confident lines nobody resolved, so the fast check has to be in it."""
        text = skill_text("ply-author")
        self.assertIn("cargo ply check", text)

    def test_authoring_warns_against_a_promise_that_cannot_fail(self):
        """A promise true of every possible body earns a green verdict and
        says nothing, which is the most expensive mistake an author can make
        here -- so the skill has to name it and name the check that finds
        it."""
        text = skill_text("ply-author")
        self.assertIn("cannot fail", text)
        self.assertIn("mutate", text)


class PlyAuditSkillTests(unittest.TestCase):
    def test_audit_reports_and_never_repairs(self):
        """Discharging an assumption means changing code or adding a check.
        Both belong to the developer; an auditor that quietly fixes what it
        finds has stopped being an auditor."""
        authority = table(skill_text("ply-audit"), "Change authority")
        self.assertTrue(authority, "audit must state its authority")
        for target, cells in authority.items():
            self.assertEqual(
                cells[0], "ask-first", f"{target} must not be edited by this skill"
            )

    def test_audit_never_reads_the_record_directly(self):
        """The public JSON of two commands is the surface. Reconstructing a
        trust surface from `ply.lock` would be a second implementation of
        Ply's own semantics, free to disagree with it."""
        boundary = table(skill_text("ply-audit"), "Data boundary")
        self.assertEqual(boundary["ply_lock"][0], "forbidden")

    def test_audit_refuses_to_read_an_empty_list_as_a_clean_result(self):
        """Nothing found and nothing searched produce the same empty list,
        and the difference is the whole answer."""
        text = skill_text("ply-audit")
        self.assertIn("Never present an empty list as a clean bill of health", text)


class PlyCheckableCodeSkillTests(unittest.TestCase):
    """The generative counterpart to the diagnostic scan: this skill exists
    so an agent writes a shape Ply can check, rather than discovering each
    refusal after the code is written."""

    def test_every_rule_is_backed_by_something_that_happened(self):
        """A style guide nobody can argue with is a style guide nobody
        follows. Each rule here came from a real refusal in Ply's own
        source, and saying so is what makes it a finding rather than a
        preference."""
        text = skill_text("ply-checkable-code")
        self.assertIn("Every rule below has a real incident behind it", text)

    def test_it_leads_with_separating_decisions_from_side_effects(self):
        """The highest-value rule, and the one that turns a refusal into a
        design improvement rather than a workaround."""
        text = skill_text("ply-checkable-code")
        first = text.index("## 1.")
        self.assertIn("Separate deciding from writing", text[first : first + 120])

    def test_weakening_a_promise_to_pass_is_forbidden_outright(self):
        """Not ask-first. There is no version of this that is correct: it
        converts a real finding into a result nobody can trust, which is the
        one outcome the whole tool exists to prevent."""
        authority = table(skill_text("ply-checkable-code"), "Change authority")
        self.assertEqual(
            authority["weakening_a_promise_to_make_a_check_pass"][0], "never"
        )

    def test_it_says_some_functions_should_stay_unclaimed(self):
        """A skill that pushed for total coverage would push an agent to
        claim shells, which is how a document fills with promises that mean
        nothing."""
        text = skill_text("ply-checkable-code")
        self.assertIn("meant to stay unclaimed", text)


class TextFormTests(unittest.TestCase):
    """A model cannot hover, and roughly 95% of a Ply drawing is hover text.

    So a skill that might otherwise reach for the picture has to name the
    text form instead. This is not a style preference: an agent that reads
    the SVG reads about a twentieth of the document and has no way to know
    it, which is the quietest wrong answer available here.
    """

    def test_skills_that_might_read_a_drawing_point_at_the_text_form(self):
        for name in ("ply-review", "ply-audit", "ply-author"):
            text = skill_text(name)
            self.assertIn(
                "--text",
                text,
                f"{name} must name the text form rather than leave an agent to "
                f"read a picture it cannot hover over",
            )

    def test_the_text_form_is_never_offered_as_evidence(self):
        """The render never sees a verdict. A skill that sends an agent to it
        has to say so, or a document full of promises gets reported as a
        codebase full of results."""
        text = skill_text("ply-review")
        self.assertIn("declared intent and not evidence", text)


class PlyReviewSkillTests(unittest.TestCase):
    def test_public_artifact_fixture_preserves_run_evidence_gaps_and_exact_source(self):
        index = fixture("view.json")
        envelope = fixture("visual.json")
        current = next(run for run in index["runs"] if run["id"] == index["currentRun"])

        self.assertEqual(index["protocolVersion"], 1)
        self.assertEqual(current["path"], f'views/{current["id"]}/visual.json')
        self.assertEqual(envelope["run"]["id"], current["id"])
        self.assertEqual(envelope["run"]["outcome"], "missing_evidence")
        self.assertEqual(envelope["run"]["root"]["path"], ".")

        element = envelope["elements"]["component:billing::charge"]
        diagnostic = next(
            item
            for item in envelope["diagnostics"]
            if item["id"] in element["diagnosticIds"]
        )
        self.assertEqual(element["evidence"]["verdict"], "missing_evidence")
        self.assertEqual(diagnostic["code"], "PLY-MISSING-EVIDENCE")
        self.assertEqual(
            element["source"],
            {
                "file": "src/billing.rs",
                "startLine": 12,
                "startColumn": 4,
                "endLine": 18,
                "endColumn": 5,
            },
        )

    def test_review_covers_every_published_outcome(self):
        policy = table(skill_text("ply-review"), "Outcome review")
        self.assertEqual(
            set(policy),
            {
                "clean",
                "violation",
                "missing_evidence",
                "narrowed_evidence",
                "timeout",
            },
        )
        for outcome, behavior in policy.items():
            with self.subTest(outcome=outcome):
                self.assertEqual(behavior[0], "report-honestly")
                self.assertTrue(behavior[1])

    def test_review_reads_public_artifacts_without_reclassifying_them(self):
        boundary = table(skill_text("ply-review"), "Data boundary")
        self.assertEqual(boundary["view_index"][0], "read")
        self.assertEqual(boundary["visual_envelope"][0], "read")
        self.assertEqual(boundary["ply_lock"][0], "forbidden")
        self.assertEqual(boundary["internal_serializer"][0], "forbidden")
        self.assertEqual(boundary["client_side_verdict_classifier"][0], "forbidden")

    def test_source_navigation_uses_the_envelope_range(self):
        source = table(skill_text("ply-review"), "Source navigation")
        self.assertEqual(source["path"][0], "source.file")
        self.assertEqual(source["start"][0], "source.startLine:source.startColumn")
        self.assertEqual(source["end"][0], "source.endLine:source.endColumn")
        self.assertEqual(source["coordinate_base"][0], "zero-based")

    def test_review_also_preserves_developer_intent(self):
        authority = table(skill_text("ply-review"), "Change authority")
        for protected in (
            "contract",
            "declared_check",
            "evidence_requirement",
            "architecture_contract",
        ):
            with self.subTest(protected=protected):
                self.assertEqual(authority[protected][0], "ask-first")


if __name__ == "__main__":
    unittest.main()
