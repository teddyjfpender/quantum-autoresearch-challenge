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


if __name__ == "__main__":
    unittest.main()
