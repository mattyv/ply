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
        commands = re.findall(
            r"```(?:bash|sh)\n(.*?)```", skill_text("ply-verify"), re.DOTALL
        )
        self.assertTrue(commands, "skill must show the public workflow")
        for block in commands:
            for line in block.splitlines():
                if not line.strip():
                    continue
                self.assertRegex(line, r"^cargo ply (check|verify) ")
                self.assertNotIn("--fail-on error", line)


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
