import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "tools"))

from qac import manifest  # noqa: E402
from qac.common import Challenge, ContractError  # noqa: E402


class ManifestTest(unittest.TestCase):
    def setUp(self):
        self.challenge = Challenge("femoco")
        self.architectures = self.challenge.architectures()
        self.good = {
            "schema": "qac-submission-v1", "challenge": "femoco", "track": "li", "architecture": "onehot-split",
            "title": "A title", "build": {"FEMOCO_WALK_ARCH": "sa-toff", "FEMOCO_SA_TWEAKS": "imchxgrdky4zab+-_.e"},
            "authors": ["octocat"], "model": "Some Model 1.0", "harness": "Some Harness",
        }

    def check(self, data, track="li", submission_id="my-circuit"):
        return manifest.validate(self.challenge, track, submission_id, data, self.architectures)

    def test_good(self):
        checked = self.check(self.good)
        self.assertEqual(checked["id"], "my-circuit")
        self.assertEqual(checked["parents"], [])

    def test_rejections(self):
        bad = [
            {**self.good, "track": "reiher"},
            {**self.good, "architecture": "not-registered"},
            {**self.good, "build": {"PATH": "/tmp"}},
            {**self.good, "build": {"FEMOCO_WALK_SPEC": "li-sa-est-v1"}},
            {**self.good, "build": {"FEMOCO_STAGE": "build"}},
            {**self.good, "build": {"FEMOCO_SA_TWEAKS": "a b; rm -rf /"}},
            {**self.good, "build": {"FEMOCO_SA_TWEAKS": "$(id)"}},
            {**self.good, "authors": []},
            {**self.good, "authors": ["not a login"]},
            {**self.good, "title": "tab\there"},
            {**self.good, "model": ""},
            {**self.good, "claimed": {"toffoli": -1}},
            {**self.good, "extra": 1},
            {**self.good, "parents": ["Bad Id"]},
            {**self.good, "discussion": "https://example.com/x"},
            {k: v for k, v in self.good.items() if k != "harness"},
            [],
        ]
        for data in bad:
            with self.assertRaises(ContractError, msg=data):
                self.check(data)
        with self.assertRaises(ContractError):
            self.check(self.good, submission_id="Bad_Id")
        with self.assertRaises(ContractError):
            self.check(self.good, track="nosuch")


if __name__ == "__main__":
    unittest.main()
