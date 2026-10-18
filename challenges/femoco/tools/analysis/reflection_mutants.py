#!/usr/bin/env python3
"""Tests the tests of the reflection check (spec/SAMPLING.md section 4.1).

Each entry of M is one alteration of one rule of src/sim/reflection.rs or of its decision
diagrams: a comparison skipped, a condition ignored, a term dropped. The script copies the
crate to a scratch directory, applies the alterations one at a time, and runs
tests/reflection.rs and the diagram unit tests. Every alteration must make a test fail.

Usage, from the challenge directory (about 25 minutes; run it through tools/heavy.sh):
    tools/analysis/reflection_mutants.py . SCRATCH_DIR [NAME ...]
    tools/analysis/reflection_mutants.py . --check     # only: does every anchor still exist?

Exit status 0 when every alteration is caught. An alteration whose anchor text is no longer
in the source is reported as NOT-FOUND and counts as a failure: update the entry.
"""
import os
import shutil
import subprocess
import sys

R = "src/sim/reflection.rs"
B = "src/sim/reflection/bdd.rs"

# (name, file, old, new, which occurrence)
M = [
    ("bit-invert-noop", R, "if op.kind != K::BitStore0 {\n                    v = self.xor(v, c);", "if op.kind == K::BitStore1 {\n                    v = self.xor(v, c);", 0),
    ("bit-store-keeps-old", R, "if op.kind != K::BitInvert {\n                    // Store 0", "if false {\n                    // Store 0", 0),
    ("neg-ignored", R, "let p = self.phase(c);\n                sound!();\n                self.sign(p);", "let p = self.phase(c);\n                sound!();\n                drop(p);", 0),
    ("x-ignores-condition", R, "let mut p = self.cond(op.c_condition);\n                for c in [c1, c2] {", "let mut p = TRUE;\n                for c in [c1, c2] {", 0),
    ("x-ignores-second-control", R, "for c in [c1, c2] {\n                    if c != NONE {\n                        let v = self.rd(c);", "for c in [c1] {\n                    if c != NONE {\n                        let v = self.rd(c);", 0),
    ("x-write-dropped", R, "sound!();\n                        self.val[t as usize] = v;", "sound!();\n                        drop(v);", 0),
    ("z-phase-dropped", R, "let p = self.phase(w);\n                    sound!();\n                    self.sign(p);", "let p = self.phase(w);\n                    sound!();\n                    drop(p);", 0),
    ("z-ignores-condition", R, "let mut w = self.cond(op.c_condition);\n                for &q in qs.iter()", "let mut w = TRUE;\n                for &q in qs.iter()", 0),
    ("system-z-not-logged", R, "self.log.push((i, Sys::Z { q, c: w }));", "drop(w);", 0),
    ("system-x-not-logged", R, "self.log.push((i, Sys::X { q, c: p }));", "drop(p);", 0),
    ("swap-ignores-condition", R, "let d = self.and(c, d);\n                let (x, y) = (self.xor(x, d), self.xor(y, d));", "let d = self.and(TRUE, d);\n                let (x, y) = (self.xor(x, d), self.xor(y, d));", 0),
    ("hmr-phase-dropped", R, "let p = self.phase(s);", "let p = self.phase(FALSE);", 0),
    ("hmr-phase-ignores-condition", R, "let s = self.and(cv, m);", "let s = self.and(v, m);", 0),
    ("hmr-qubit-not-cleared", R, "let w = self.xor(v, cv);", "let w = v;", 0),
    ("hmr-bit-ignores-condition", R, "let e = self.and(c, e);\n                let bit = self.xor(old, e);", "let bit = self.xor(old, e);", 0),
    ("reset-not-checked", R, "if v != FALSE {\n                    return Err(obstruct(\n                        at,\n                        \"reset-not-proven\"", "if false {\n                    return Err(obstruct(\n                        at,\n                        \"reset-not-proven\"", 0),
    ("conditioned-givens-allowed", R, "if c != TRUE {\n                    return no(\n                        \"conditioned-rotation\"", "if false {\n                    return no(\n                        \"conditioned-rotation\"", 0),
    ("givens-angle-bit-dropped", R, "angle.push(self.rd(x));", "angle.push(self.rd(x) & 0);", 0),
    ("spinswap-ignores-wire", R, "let c = self.and(c, v);\n                sound!();\n                let dagger", "let c = self.and(c, TRUE);\n                drop(v);\n                sound!();\n                let dagger", 0),
    ("spinswap-ignores-condition", R, "let c = self.cond(op.c_condition);\n                let v = self.rd(t);\n                let c = self.and(c, v);", "let c = TRUE;\n                let v = self.rd(t);\n                let c = self.and(c, v);", 0),
    ("spinswap-direction-lost", R, "let dagger = op.kind == K::SpinSwapDg;", "let dagger = false;", 0),
    ("s-gate-allowed", R, "K::S | K::Sdg => {\n                return no(\n                    \"unsupported-op\",\n                    \"S and Sdg are outside the rule set (phases are tracked as signs only)\",\n                )\n            }", "K::S | K::Sdg => {}", 0),
    ("system-control-allowed", R, "if [c1, c2].iter().any(|&c| c != NONE && sys(c).is_some()) {", "if false {", 0),
    ("two-system-qubits-allowed", R, "if on.len() > 1 {", "if on.len() > 2 {", 0),
    ("reflect-not-checked", R, "if d != FALSE && wrong.is_none() {", "if false {", 0),
    ("reflect-keeps-variables", R, "after.push(self.b.var(self.inner_level(k, true)));", "after.push(v);", 0),
    ("reflect-in-block-allowed", R, "if !self.stack.is_empty() {\n            return no(\"Reflect inside a condition block\");\n        }", "", 0),
    ("second-reflect-allowed", R, "if self.reflected {\n            return no(\"a second Reflect\");\n        }", "", 0),
    ("reflect-register-not-checked", R, "if reg != self.inner {", "if false {", 0),
    ("end-registers-not-checked", R, "if d != FALSE {\n            let what = if q < self.first {", "if false {\n            let what = if q < self.first {", 0),
    ("open-block-allowed", R, "if !self.stack.is_empty() {\n            return no(\"a condition block is left open\");\n        }", "", 0),
    ("no-reflect-allowed", R, "if !self.reflected {\n            return no(\"the stream has no Reflect\");\n        }", "", 0),
    ("cond-ignores-enclosing", R, "let base = self.stack.last().copied().unwrap_or(TRUE);", "let base = TRUE;", 0),
    ("cond-ignores-own-bit", R, "let bit = self.cl[cc as usize];\n        self.and(base, bit)", "let bit = self.cl[cc as usize];\n        self.and(base, bit | 1)", 0),
    ("push-ignores-condition", R, "let c = self.cond(op.c_condition);\n                sound!();\n                self.stack.push(c);", "let c = self.cond(NONE);\n                sound!();\n                self.stack.push(c);", 0),
    ("fold-x-crossing-dropped", R, "self.forms[k].sign = s;\n                self.forms[k].list.push((at, Sys::X { q, c }));", "drop(s);\n                self.forms[k].list.push((at, Sys::X { q, c }));", 0),
    ("fold-g-sign-misses-q", R, "let sign = self.xor(sign, self.forms[k].frame[p]);\n                let sign = self.xor(sign, self.forms[k].frame[q]);", "let sign = self.xor(sign, self.forms[k].frame[p]);", 0),
    ("fold-g-sign-misses-p", R, "let sign = self.xor(sign, self.forms[k].frame[p]);\n                let sign = self.xor(sign, self.forms[k].frame[q]);", "let sign = self.xor(sign, self.forms[k].frame[q]);", 0),
    ("fold-f-keeps-frame", R, "self.forms[k].frame[..2 * pairs].copy_from_slice(&frame);", "drop(frame);", 0),
    ("fold-f-ignores-control", R, "let d = self.and(c, d);\n                    frame.push(self.xor(x, d));", "frame.push(self.xor(x, d));", 0),
    ("fold-z-dropped", R, "self.forms[k].frame[q] = f;", "drop(f);", 0),
    ("adjoint-g-sign-kept", R, "sign: self.xor(sign, TRUE),", "sign,", 0),
    ("adjoint-f-direction-kept", R, "Sys::F { dagger, c } => Sys::F { dagger: !dagger, c },", "Sys::F { dagger, c } => Sys::F { dagger, c },", 0),
    ("adjoint-not-reversed", R, "self.forms[0].list[m - 1 - (j - n)].clone()", "self.forms[0].list[j - n].clone()", 0),
    ("adjoint-without-frame", R, "let c = self.forms[0].frame[j];\n                    (usize::MAX, Sys::Z { q: j, c })", "(usize::MAX, Sys::Z { q: j, c: FALSE })", 0),
    ("pending-outcome-phase-ignored", R, "totals.iter().find(|t| !t.0.is_empty() && t.1 != FALSE)", "totals.iter().find(|t| t.0.len() > 99 && t.1 != FALSE)", 0),
    ("plain-sign-dropped", R, "let s = self.xor(self.forms[k].sign, plain);", "let s = self.xor(self.forms[k].sign, plain & 0);", 0),
    ("outcome-in-system-ops-allowed", R, "self.independent(k, false)?;", "", 0),
    ("compare-x-control-skipped", R, "(Sys::X { q, c }, Sys::X { q: q2, c: c2 }) if q == q2 => (c != c2).then(|| {", "(Sys::X { q, c }, Sys::X { q: q2, c: c2 }) if q == q2 => (false && c != c2).then(|| {", 0),
    ("compare-x-qubit-skipped", R, "(Sys::X { q, c }, Sys::X { q: q2, c: c2 }) if q == q2 =>", "(Sys::X { q, c }, Sys::X { q: q2, c: c2 }) if q == q2 || true =>", 0),
    ("compare-f-control-skipped", R, "if dagger == d2 => (c != c2)\n                    .then(|| {", "if dagger == d2 => (false && c != c2)\n                    .then(|| {", 0),
    ("compare-f-direction-skipped", R, "if dagger == d2 => (c != c2)", "if dagger == d2 || true => (c != c2)", 0),
    ("compare-angle-bits-skipped", R, ".find(|(_, (x, y))| x != y)\n                    .map(|(j, (x, y))| {", ".find(|(_, (x, y))| false && x != y)\n                    .map(|(j, (x, y))| {", 0),
    ("compare-g-sign-skipped", R, "(sign != s2).then(|| {", "(false && sign != s2).then(|| {", 0),
    ("compare-frame-skipped", R, ".find(|(_, (x, y))| x != y)\n                .map(|(q, (x, y))| {", ".find(|(_, (x, y))| false && x != y)\n                .map(|(q, (x, y))| {", 0),
    ("compare-sign-skipped", R, "if wrong.is_none() && back.sign != twin.sign {", "if false && back.sign != twin.sign {", 0),
    ("adjoint-sign-without-lane-sign", R, "let s = e.xor(e.forms[0].sign, e.forms[2].sign);", "let s = e.xor(FALSE, e.forms[2].sign);", 0),
    ("second-pass-not-exchanged", R, "self.pass(ops, true)?;", "self.pass(ops, false)?;", 0),
    ("anf-forgets-shared-terms", R, "(Some(x), Some(y)) if x == y => {\n                        i += 1;\n                        j += 1;\n                    }", "(Some(x), Some(y)) if x == y => {\n                        form.push(with(x));\n                        i += 1;\n                        j += 1;\n                    }", 0),
    ("anf-forgets-low-branch", R, "let mut form: Vec<Mono> = a.iter().cloned().collect();", "let mut form: Vec<Mono> = Vec::new();", 0),
    ("sum-carry-drops-term", R, "f = self.xor(f, slots[k]);\n            slots[k] = FALSE;", "f = slots[k];\n            slots[k] = FALSE;", 0),
    ("sum-total-skips-a-slot", R, "s.0.iter().fold(FALSE, |t, &n| self.xor(t, n))", "s.0.iter().skip(1).fold(FALSE, |t, &n| self.xor(t, n))", 0),
    ("unsound-results-used", R, "!self.b.overflowed && !self.spent\n", "true\n", 0),
    ("work-limit-ignored", R, "*spent |= *work > limit;", "*spent |= false;", 0),
    ("collect-keeps-stale-forms", R, "self.b.collect(roots);\n        self.anf.clear();", "self.b.collect(roots);", 0),
    ("collect-forgets-sign", R, ".chain(self.phi.values_mut().flat_map(|s| s.0.iter_mut()))", "", 0),
    ("collect-forgets-forms", R, ".chain(std::iter::once(&mut f.sign))", "", 0),
    ("bdd-and-shortcut-wrong", B, "Op::And if f == TRUE || f == g => Some(g),", "Op::And if f == TRUE || f == g => Some(f),", 0),
    ("bdd-xor-shortcut-wrong", B, "Op::Xor if f == FALSE => Some(g),", "Op::Xor if f == FALSE => Some(f),", 0),
    ("bdd-reduction-off", B, "if lo == hi {\n            return lo;\n        }", "", 0),
    ("bdd-reads-from-ignores-start", B, ".filter(|&l| l >= from)", ".filter(|&l| l > from)", 0),
    ("bdd-memo-unordered", B, "let (f, g) = if f <= g { (f, g) } else { (g, f) };\n                    if let Some(r) = Self::shortcut(op, f, g) {", "if let Some(r) = Self::shortcut(Op::Xor, f, g) {", 0),
]


def main():
    crate, scratch = sys.argv[1], sys.argv[2]
    only = set(sys.argv[3:])
    missing = [name for name, rel, old, _, _ in M if old not in open(os.path.join(crate, rel)).read()]
    names = [m[0] for m in M]
    assert len(set(names)) == len(names)
    if missing or scratch == "--check":
        print(f"{len(M)} alterations; anchors not found: {missing}")
        sys.exit(1 if missing else 0)
    work = os.path.join(scratch, "crate")
    if os.path.exists(work):
        shutil.rmtree(work)
    shutil.copytree(crate, work, ignore=shutil.ignore_patterns("target", "ops.bin", "target-*"))
    env = dict(os.environ, CARGO_TARGET_DIR=os.path.join(scratch, "target"), CARGO_BUILD_JOBS="4",
               RAYON_NUM_THREADS="4", CARGO_PROFILE_RELEASE_LTO="off", CARGO_PROFILE_RELEASE_CODEGEN_UNITS="16")

    def run():
        a = subprocess.run(["cargo", "test", "--release", "--test", "reflection"], cwd=work, env=env,
                           capture_output=True, text=True)
        if "could not compile" in a.stderr:
            return "COMPILE", a.stderr[-1500:]
        b = subprocess.run(["cargo", "test", "--release", "--lib", "sim::reflection"], cwd=work, env=env,
                           capture_output=True, text=True)
        failed = [l.split()[1] for l in (a.stdout + b.stdout).splitlines() if l.startswith("test ") and l.endswith("FAILED")]
        if a.returncode == 0 and b.returncode == 0:
            return "SURVIVED", ""
        return "killed", " ".join(failed[:4]) + (f" (+{len(failed) - 4})" if len(failed) > 4 else "")

    state, why = run()
    print(f"unaltered: {state} {why}", flush=True)
    if state != "SURVIVED":
        sys.exit("the unaltered copy does not pass")
    tally = {}
    for name, rel, old, new, which in M:
        if only and name not in only:
            continue
        path = os.path.join(work, rel)
        src = open(path).read()
        at, start = -1, 0
        for _ in range(which + 1):
            at = src.find(old, start)
            start = at + 1
            if at < 0:
                break
        if at < 0:
            print(f"{name}: NOT-FOUND", flush=True)
            tally["NOT-FOUND"] = tally.get("NOT-FOUND", 0) + 1
            continue
        open(path, "w").write(src[:at] + new + src[at + len(old):])
        state, why = run()
        open(path, "w").write(src)
        tally[state] = tally.get(state, 0) + 1
        print(f"{name}: {state} {why}", flush=True)
    print("tally:", tally, flush=True)
    if set(tally) - {"killed"}:
        sys.exit(1)


main()
