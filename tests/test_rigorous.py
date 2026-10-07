"""Track isolation, promotion provenance and staging-only pending baselines."""
import copy
import hashlib
import json
import pathlib
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "tools"))
from qac import check, manifest  # noqa: E402
from qac.common import Challenge  # noqa: E402


class RigorousTest(unittest.TestCase):
    def setUp(self):
        self.challenge = Challenge("femoco")
        self.track = copy.deepcopy(self.challenge.tracks["reiher-rigorous"])

    def test_new_tracks_use_rigorous_specs_and_require_deterministic_coverage(self):
        for name, spec in (("reiher", "reiher-sa"), ("li", "li-sa")):
            self.assertEqual(self.challenge.tracks[name]["spec"], spec + "-est-v1")
            rigorous = self.challenge.tracks[name + "-rigorous"]
            self.assertEqual(rigorous["spec"], spec + "-v1")
            self.assertEqual(rigorous["validation"]["coverage"], "terms")
            self.assertEqual(rigorous["standard"]["budgetHa"], "1/10000")
            self.assertEqual(check.check_baseline(self.challenge, rigorous, []), [])

    def test_pending_baseline_cannot_be_used_on_live_challenge_or_track(self):
        for field in ("challenge", "track"):
            if field == "challenge": self.challenge.contract["status"] = "live"
            else: self.track["status"] = "live"
            self.assertTrue(check.check_baseline(self.challenge, self.track, []))
            self.challenge.contract["status"] = self.track["status"] = "staging"

    def test_pending_baseline_cannot_claim_existing_estimated_result(self):
        self.track["baseline"]["submission"] = "reiher/est-low-r"
        self.assertTrue(check.check_baseline(self.challenge, self.track, [
            {"submission": "reiher/est-low-r", "track": "reiher"}]))

    def test_catalogue_must_match_spec_track_and_baseline_role(self):
        self.track["baseline"]["candidate"] = "not-present"
        self.assertTrue(check.check_baseline(self.challenge, self.track, []))
        self.track["baseline"]["candidate"] = "reiher-fewest-toffoli-rigorous"
        self.assertTrue(check.check_baseline(self.challenge, self.track, []))
        self.track["baseline"]["candidate"] = "reiher-low2025-rigorous"
        self.track["spec"] = "reiher-sa-est-v1"
        self.assertTrue(check.check_baseline(self.challenge, self.track, []))
        with patch.object(check, "load_json", return_value={"schema": "wrong"}):
            self.assertTrue(check.check_baseline(self.challenge, self.track, []))

    def test_recorded_baseline_must_belong_to_the_track(self):
        self.track["baseline"] = {"submission": "reiher-rigorous/baseline"}
        self.assertTrue(check.check_baseline(self.challenge, self.track, []))
        row = {"submission": "reiher-rigorous/baseline", "track": "reiher"}
        self.assertTrue(check.check_baseline(self.challenge, self.track, [row]))
        row["track"] = "reiher-rigorous"
        self.assertEqual(check.check_baseline(self.challenge, self.track, [row]), [])

    def test_catalogue_pins_sources_and_all_candidates_are_valid_build_manifests(self):
        catalogue = json.loads((self.challenge.dir / "rigorous/promotions.json").read_text())
        candidates = catalogue["candidates"]
        self.assertEqual(len(candidates), 8)
        self.assertEqual(len({c["id"] for c in candidates}), 8)
        for c in candidates:
            source = self.challenge.dir / "submissions" / c["source"] / "submission.json"
            self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(), c["source_manifest_sha256"])
            self.assertEqual(self.challenge.tracks[c["track"]]["spec"], c["spec"])
            build_manifest = {
                "schema": "qac-submission-v1", "challenge": "femoco", "track": c["track"],
                "architecture": c["architecture"], "title": c["id"], "build": c["build"],
                "authors": ["octocat"], "model": catalogue["model"], "harness": catalogue["harness"],
            }
            manifest.validate(self.challenge, c["track"], c["id"], build_manifest, self.challenge.architectures())


class EvidenceTest(unittest.TestCase):
    """rigorous/ holds generated evidence. It must be the output of the current promotion
    catalogue and the current symbolic checker, and the README must quote it faithfully."""

    def setUp(self):
        self.dir = Challenge("femoco").dir
        self.results = json.loads((self.dir / "rigorous/validation-results.json").read_text())
        self.by_id = {r["id"]: r for r in self.results["results"]}

    def sha(self, relative):
        return hashlib.sha256((self.dir / relative).read_bytes()).hexdigest()

    def test_results_are_for_the_current_catalogue_and_every_candidate_passed(self):
        catalogue = json.loads((self.dir / "rigorous/promotions.json").read_text())
        self.assertEqual(self.results["catalogue_sha256"], self.sha("rigorous/promotions.json"))
        self.assertEqual(set(self.by_id), {c["id"] for c in catalogue["candidates"]})
        self.assertIs(self.results["authenticated_ledger_result"], False)
        self.assertIs(self.results["environment"]["source_clean"], True)
        for r in self.by_id.values():
            with self.subTest(candidate=r["id"]):
                self.assertEqual(r["status"], "passed")
                self.assertEqual(r["reference_screen"]["status"], "passed")
                v = r["validation"]
                self.assertEqual(v["protocol"], "femoco-deterministic-v2")
                self.assertEqual((v["mode"], v["status"], v["certified"]), ("Terms", "passed", False))
                self.assertEqual(v["lanes"], 2 * (v["term_pair_cases"] + v["boundary_cases"]))
                self.assertLessEqual(r["rounding_error"], 1e-4)

    def test_certificates_are_from_the_current_checker_and_bound_to_their_candidates(self):
        exact = self.sha("tools/verification/exact.py")
        reports = sorted((self.dir / "rigorous").glob("exact-*.json"))
        claimed = {r["exact"]["report"]: r for r in self.by_id.values() if "exact" in r}
        self.assertEqual({p.name for p in reports}, set(claimed))
        self.assertTrue(reports, "at least one symbolic certificate is kept")
        for path in reports:
            with self.subTest(report=path.name):
                report, result = json.loads(path.read_text()), claimed[path.name]
                self.assertEqual(report["schema"], "femoco-exact-report-v2")
                self.assertEqual(report["checker_sha256"], exact, "exact.py changed: run the checker again")
                self.assertEqual(result["exact"]["checker_sha256"], exact)
                self.assertIs(report["certified"], True)
                self.assertIs(report["circuit_files_checked"], True)
                parts = report["partitions"]
                self.assertEqual([p["partition"] for p in parts], list(range(1 << report["partition_bits"])))
                self.assertTrue(all(p["status"] == "proved" for p in parts))
                self.assertEqual(report["digests"], result["digests"])
                self.assertEqual(report["digests"]["spec"], self.sha(f"specs/{result['spec']}/sa.bin"))

    def test_readme_table_quotes_the_results(self):
        text = (self.dir / "rigorous/README.md").read_text()
        rows = [line for line in text.splitlines() if line.startswith(("| Reiher |", "| Li |"))]
        self.assertEqual(len(rows), len(self.by_id))
        quoted = set()
        for line in rows:
            cells = [c.strip().replace(",", "") for c in line.strip("|").split("|")]
            quoted.add((float(cells[2]), int(cells[3]), int(cells[4]), int(cells[5])))
        actual = {(round(r["toffoli"], 3), r["qubits"], round(r["product"]), r["validation"]["lanes"])
                  for r in self.by_id.values()}
        self.assertEqual(quoted, actual)


if __name__ == "__main__":
    unittest.main()
