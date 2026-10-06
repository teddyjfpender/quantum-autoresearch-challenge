"""End-to-end intake on a scratch git repository: policy plus manifest, read from git objects."""
import contextlib
import io
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))

from qac import cli  # noqa: E402

F = "challenges/femoco"
MANIFEST = {
    "schema": "qac-submission-v1", "challenge": "femoco", "track": "reiher", "architecture": "onehot-split",
    "title": "Test circuit", "build": {"FEMOCO_WALK_ARCH": "sa-toff", "FEMOCO_SA_TWEAKS": "imchxgrdky3zab"},
    "authors": ["octocat"], "model": "none", "harness": "none",
}


class IntakeTest(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.repo = self.tmp / "repo"
        (self.repo / F / "src/walk").mkdir(parents=True)
        (self.repo / F / "src/sim").mkdir(parents=True)
        shutil.copy(ROOT / F / "architectures.json", self.repo / F / "architectures.json")
        (self.repo / F / "src/walk/mod.rs").write_text("// walk\n")
        (self.repo / F / "src/sim/mod.rs").write_text("// trusted\n")
        self.git("init", "-q", "-b", "main")
        self.base = self.commit("base")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def git(self, *args):
        return subprocess.run(["git", "-C", str(self.repo), "-c", "user.name=t", "-c", "user.email=t@example.invalid", *args],
                              check=True, capture_output=True, text=True).stdout.strip()

    def commit(self, message):
        self.git("add", "-A")
        self.git("commit", "-q", "-m", message)
        return self.git("rev-parse", "HEAD")

    def submit(self, manifest=MANIFEST, notes="n" * 2048, directory="reiher/test-circuit"):
        target = self.repo / F / "submissions" / directory
        target.mkdir(parents=True)
        (target / "submission.json").write_text(json.dumps(manifest))
        (target / "NOTES.md").write_text(notes)

    def intake(self, head):
        out = self.tmp / "out"
        shutil.rmtree(out, ignore_errors=True)
        with contextlib.redirect_stdout(io.StringIO()):
            code = cli.main(["intake", "--repo", str(self.repo), "--base", self.base, "--head", head, "--out", str(out)])
        self.assertEqual(code, 0)
        policy = json.loads((out / "policy.json").read_text())
        manifest = json.loads((out / "manifest.json").read_text()) if (out / "manifest.json").exists() else None
        return policy, manifest

    def test_valid_submission_with_code(self):
        self.submit()
        (self.repo / F / "src/walk/mod.rs").write_text("// walk, changed\n")
        policy, manifest = self.intake(self.commit("submission"))
        self.assertTrue(policy["ok"], policy["errors"])
        self.assertEqual((manifest["track"], manifest["id"], manifest["architecture"]), ("reiher", "test-circuit", "onehot-split"))

    def test_trusted_change_is_refused(self):
        self.submit()
        (self.repo / F / "src/sim/mod.rs").write_text("// tampered\n")
        policy, manifest = self.intake(self.commit("tamper"))
        self.assertFalse(policy["ok"])
        self.assertIsNone(manifest)

    def test_bad_manifest_and_short_notes(self):
        self.submit(manifest={**MANIFEST, "build": {"LD_PRELOAD": "x"}})
        policy, manifest = self.intake(self.commit("bad knob"))
        self.assertFalse(policy["ok"])
        self.assertIsNone(manifest)
        self.git("reset", "-q", "--hard", self.base)
        self.submit(notes="too short")
        policy, manifest = self.intake(self.commit("short notes"))
        self.assertFalse(policy["ok"])

    def test_new_architecture(self):
        registry = json.loads((self.repo / F / "architectures.json").read_text())
        registry["architectures"].append({"id": "brand-new", "name": "Brand new", "mechanism": "m" * 220,
                                          "distinguishing": "d" * 120, "references": []})
        (self.repo / F / "architectures.json").write_text(json.dumps(registry, indent=1) + "\n")
        # A new architecture comes with a new builder, which the rules cannot classify.
        self.submit(manifest={**MANIFEST, "architecture": "brand-new", "build": {"FEMOCO_WALK_ARCH": "sa-new"}})
        policy, manifest = self.intake(self.commit("new architecture"))
        self.assertTrue(policy["ok"], policy["errors"])
        self.assertTrue(policy["proposes_architecture"])
        self.assertIsNone(policy["classified"])
        self.assertEqual(manifest["architecture"], "brand-new")

    def test_declared_architecture_must_match_the_build_knobs(self):
        self.submit(manifest={**MANIFEST, "architecture": "qroam-word"})
        policy, manifest = self.intake(self.commit("misfiled"))
        self.assertFalse(policy["ok"])
        self.assertIsNone(manifest)
        self.assertIn("onehot-split", " ".join(policy["errors"]))

    def test_source_gate(self):
        for name, body in (
            ("a.rs", "fn f() { unsafe { core::hint::unreachable_unchecked() } }\n"),
            ("b.rs", "use std::process::Command;\n"),
            ("c.rs", 'const X: &[u8] = include_bytes!("/etc/passwd");\n'),
            ("blob.bin", "x"),
            ("Cargo.toml", "[package]\n"),
        ):
            self.git("reset", "-q", "--hard", self.base)
            self.git("clean", "-qfd")
            self.submit()
            (self.repo / F / "src/walk" / name).write_text(body)
            policy, _ = self.intake(self.commit(f"gate {name}"))
            self.assertFalse(policy["ok"], name)
        self.git("reset", "-q", "--hard", self.base)
        self.git("clean", "-qfd")
        self.submit()
        (self.repo / F / "src/walk/ok.rs").write_text('// unsafe is only a word here\npub fn f() -> &\x27static str { "extern" }\n')
        policy, _ = self.intake(self.commit("fine"))
        self.assertTrue(policy["ok"], policy["errors"])

    def test_branch_must_contain_the_base_tip(self):
        self.submit()
        head = self.commit("submission")
        self.git("checkout", "-q", "-b", "moved", self.base)
        (self.repo / F / "src/walk/mod.rs").write_text("// main moved on\n")
        tip = self.commit("another submission landed")
        out = self.tmp / "out"
        with contextlib.redirect_stdout(io.StringIO()):
            cli.main(["intake", "--repo", str(self.repo), "--base", self.base, "--head", head, "--tip", tip, "--out", str(out)])
        policy = json.loads((out / "policy.json").read_text())
        self.assertFalse(policy["ok"])
        self.assertIn("update it", " ".join(policy["errors"]))

    def test_registry_rewrite_is_refused(self):
        registry = json.loads((self.repo / F / "architectures.json").read_text())
        registry["architectures"][0]["mechanism"] = "rewritten"
        (self.repo / F / "architectures.json").write_text(json.dumps(registry, indent=1) + "\n")
        self.submit()
        policy, _ = self.intake(self.commit("rewrite"))
        self.assertFalse(policy["ok"])


if __name__ == "__main__":
    unittest.main()
