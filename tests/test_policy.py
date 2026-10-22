import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "tools"))

from qac import policy  # noqa: E402
from qac.policy import Change  # noqa: E402

F = "challenges/femoco"
SUB = f"{F}/submissions/reiher/my-circuit"


def added(path):
    return Change("A", "000000", "100644", path)


def modified(path, mode="100644"):
    return Change("M", "100644", mode, path)


class PolicyTest(unittest.TestCase):
    def submission(self, *extra):
        return [added(f"{SUB}/submission.json"), added(f"{SUB}/NOTES.md"), *extra]

    def test_non_submission_pull_request_is_not_judged(self):
        verdict = policy.evaluate([modified("README.md"), modified(f"{F}/src/sim/mod.rs")])
        self.assertFalse(verdict.is_submission)

    def test_minimal_submission(self):
        verdict = policy.evaluate(self.submission())
        self.assertTrue(verdict.ok, verdict.errors)
        self.assertEqual((verdict.challenge, verdict.track, verdict.submission_id), ("femoco", "reiher", "my-circuit"))
        self.assertFalse(verdict.touches_code)

    def test_code_and_registry(self):
        verdict = policy.evaluate(self.submission(modified(f"{F}/src/walk/sa_low/toff.rs"), added(f"{F}/src/walk/new.rs"),
                                                  modified(f"{F}/architectures.json")))
        self.assertTrue(verdict.ok, verdict.errors)
        self.assertTrue(verdict.touches_code and verdict.proposes_architecture)

    def test_trusted_paths_are_refused(self):
        for path in (f"{F}/src/sim/mod.rs", f"{F}/src/bin/eval_circuit.rs", f"{F}/specs/li-sa-est-v1/spec.json",
                     f"{F}/results.tsv", f"{F}/benchmark.json", f"{F}/tests/sa_digests.rs", f"{F}/Cargo.toml",
                     ".github/workflows/judge.yml", "tools/qac/ledger.py", "challenge.py", "data/site/sources.json",
                     f"{F}/src/walkabout.rs"):
            verdict = policy.evaluate(self.submission(modified(path)))
            self.assertFalse(verdict.ok, path)

    def test_submission_shape(self):
        cases = [
            [added(f"{SUB}/submission.json"), added(f"{SUB}/NOTES.md"), added(f"{SUB}/extra.bin")],
            [added(f"{SUB}/submission.json"), added(f"{SUB}/NOTES.md"), added(f"{F}/submissions/li/other/submission.json")],
            [added(f"{F}/submissions/nosuchtrack/x-y-z/submission.json")],
            [added(f"{F}/submissions/reiher/Bad_Id/submission.json")],
            [modified(f"{SUB}/submission.json")],
            [Change("D", "100644", "000000", f"{SUB}/NOTES.md")],
            [added(f"{F}/submissions/reiher/submission.json")],
        ]
        for changes in cases:
            self.assertFalse(policy.evaluate(changes).ok, changes)

    def test_modes_and_paths(self):
        self.assertFalse(policy.evaluate(self.submission(modified(f"{F}/src/walk/a.rs", "100755"))).ok)
        self.assertFalse(policy.evaluate(self.submission(Change("A", "000000", "120000", f"{F}/src/walk/link"))).ok)
        self.assertFalse(policy.evaluate(self.submission(added(f"{F}/src/walk/../sim/x.rs"))).ok)
        self.assertFalse(policy.evaluate(self.submission(Change("R", "100644", "100644", f"{F}/src/walk/a.rs"))).ok)

    def test_parse_raw(self):
        raw = b":100644 100644 aaaa bbbb M\0challenges/femoco/src/walk/a.rs\0:000000 100644 0000 cccc A\0x y\0"
        changes = policy.parse_raw(raw)
        self.assertEqual([(c.status, c.path) for c in changes], [("M", "challenges/femoco/src/walk/a.rs"), ("A", "x y")])

    def test_source_scan_closes_indirections(self):
        bad = {
            "cfg_attr path (loads an unscanned .md as a module)": '#[cfg_attr(all(), path = "notes.md")]\nmod evil;\n',
            "cfg_attr no_mangle": "#[cfg_attr(any(), no_mangle)]\npub fn getenv() {}\n",
            "inner attribute": "#![cfg_attr(all(), link_section = \"x\")]\n",
            "macro-forwarded attribute": 'macro_rules! m { ($m:meta) => { #[$m] mod evil; } }\nm!(path = "notes.md");\n',
            "macro-forwarded attribute, spaced": "macro_rules! m { ($a:ident) => { # [ $a ] fn f() {} } }\n",
            "aliased include": 'use std::include_str as rd;\nconst X: &str = rd!("/etc/passwd");\n',
            "core include in a use list": "use core::{include_bytes as b};\n",
            "unix socket": "use std::os::unix::net::UnixStream;\n",
            "unix socket by name": 'fn f() { let _ = UnixStream::connect("/run/x"); }\n',
            "embedded http server": 'fn f() { tiny_http::Server::http("0.0.0.0:80").unwrap(); }\n',
            "process": "use std::{io, process};\n",
        }
        for name, body in bad.items():
            self.assertTrue(policy.scan_source(f"{F}/src/walk/x.rs", body.encode()), name)
        fine = (
            "#[allow(unused)]\n#[allow(clippy::used_underscore_binding, clippy::path_buf_push_overwrite)]\n"
            "#[cfg_attr(test, allow(dead_code))]\n#[must_use]\n#[derive(Clone)]\nstruct S { path: u32 }\n"
            "pub fn include_terms() -> u32 { let path = std::process::id(); path }\n"
            '// include!("not code") #[path = "x"]\nconst T: &str = "include_str!(\\"x\\") #[path = \\"y\\"]";\n'
            'pub const ARCH: Option<&str> = option_env!("FEMOCO_WALK_ARCH");\n'
        )
        self.assertEqual(policy.scan_source(f"{F}/src/walk/ok.rs", fine.encode()), [])

    def test_parse_raw_tolerates_non_utf8_paths(self):
        raw = b":000000 100644 0000 cccc A\0challenges/femoco/src/walk/\xff.rs\0"
        changes = policy.parse_raw(raw)
        self.assertEqual(len(changes), 1)
        verdict = policy.evaluate(self.submission(*changes))
        self.assertFalse(verdict.ok)
        self.assertIn("characters outside", " ".join(verdict.errors))

    def test_registry_append_only(self):
        entry = {"id": "new-arch", "name": "New", "mechanism": "m" * 200, "distinguishing": "d" * 100, "references": []}
        old = {"schema": "s", "architectures": [{"id": "a"}]}
        self.assertEqual(policy.registry_is_append_only(old, {"schema": "s", "architectures": [{"id": "a"}, entry]}), [])
        self.assertTrue(policy.registry_is_append_only(old, {"schema": "s", "architectures": [entry, {"id": "a"}]}))
        self.assertTrue(policy.registry_is_append_only(old, {"schema": "s", "architectures": [{"id": "a", "name": "x"}, entry]}))
        self.assertTrue(policy.registry_is_append_only(old, {"schema": "t", "architectures": [{"id": "a"}, entry]}))
        self.assertTrue(policy.registry_is_append_only(old, {"schema": "s", "architectures": [{"id": "a"}, {**entry, "mechanism": "short"}]}))
        self.assertTrue(policy.registry_is_append_only(old, {"schema": "s", "architectures": [{"id": "a"}, {**entry, "id": "a"}]}))
        self.assertTrue(policy.registry_is_append_only(old, old))


if __name__ == "__main__":
    unittest.main()
