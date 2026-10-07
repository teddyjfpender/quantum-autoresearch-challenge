#!/usr/bin/env python3
"""Exact SMT obligations on the trusted evaluator's lowered classical controller.

No random assignments, no restriction to a few measurement seeds. An UNSAT miter proves
the stated obligation for every control, uniform, second-pass and HMR assignment. System
operations are opaque ordered events: equality of event parameters is sufficient to prove
measurement independence without numerical quantum simulation. This is deliberately NOT
a proof that the system trace implements the spec's selected term on every input.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import time

import z3

NONE = (1 << 32) - 1
ZERO, ONE = z3.BoolVal(False), z3.BoolVal(True)


def simp(x):
    return z3.simplify(x)


def band(*xs):
    return simp(z3.And(*xs))


def bxor(a, b):
    return simp(z3.Xor(a, b))


class Controller:
    def __init__(self, doc, c, uniform, second, measurements):
        layout = doc["layout"]
        self.system = layout["system"]
        self.first = 1 + self.system
        self.q = [ZERO] * layout["num_qubits"]
        self.bits = [ZERO] * layout["num_bits"]
        self.q[0] = c
        self.q[self.first:self.first + len(uniform)] = uniform
        self.uniform, self.second = uniform, second
        self.inner, self.regs = doc["inner"], doc["registers"]
        self.measurements = measurements
        self.phase = [ZERO] * 3
        self.mask, self.stack = ONE, []
        self.dirty, self.reflect_bad = [], []
        self.events = []
        self.hmr = 0

    def mask_for(self, cond):
        return self.mask if cond == NONE else band(self.mask, self.bits[cond])

    def phase_add(self, mask, k):
        carry = ZERO
        for i in range(3):
            old, add = self.phase[i], mask if k >> i & 1 else ZERO
            self.phase[i] = bxor(bxor(old, add), carry)
            carry = simp(z3.Or(band(old, add), band(carry, bxor(old, add))))

    def run(self, ops):
        for op in ops:
            if op == "Pop":
                self.mask = self.stack.pop()
                continue
            if op == "Reflect":
                lo, width = self.inner["lo"], self.inner["width"]
                for j in range(width):
                    q = self.first + lo + j
                    self.reflect_bad.append(bxor(self.q[q], self.uniform[lo+j]))
                    # Reflect must be unconditionally reached: export was statically checked.
                    self.q[q] = self.second[j]
                continue
            if not isinstance(op, dict) or len(op) != 1:
                raise ValueError(f"unsupported lowered op {op!r}")
            kind, a = next(iter(op.items()))
            m = self.mask_for(a.get("cond", NONE))
            if kind == "Push":
                self.stack.append(self.mask)
                self.mask = band(self.mask, self.bits[a["bit"]])
            elif kind in ("X", "Cx", "Ccx"):
                if kind == "Cx": m = band(m, self.q[a["c"]])
                if kind == "Ccx": m = band(m, self.q[a["a"]], self.q[a["b"]])
                self.q[a["t"]] = bxor(self.q[a["t"]], m)
            elif kind == "Swap":
                x, y = a["a"], a["b"]
                d = band(m, bxor(self.q[x], self.q[y]))
                self.q[x], self.q[y] = bxor(self.q[x], d), bxor(self.q[y], d)
            elif kind == "Phase":
                self.phase_add(band(m, *(self.q[q] for q in a["q"] if q != NONE)), a["k"])
            elif kind == "Neg":
                self.phase_add(m, 4)
            elif kind == "Hmr":
                r = self.measurements[self.hmr]
                self.hmr += 1
                self.phase_add(band(m, self.q[a["t"]], r), 4)
                self.q[a["t"]] = band(self.q[a["t"]], z3.Not(m))
                self.bits[a["bit"]] = simp(z3.If(m, r, self.bits[a["bit"]]))
            elif kind == "Reset":
                # The interpreter checks masked cleanliness, and leaves the wire unchanged.
                self.dirty.append(band(m, self.q[a["t"]]))
            elif kind == "Bit":
                b, k = a["bit"], a["kind"]
                self.bits[b] = (bxor(self.bits[b], m) if k == 0 else
                                band(self.bits[b], z3.Not(m)) if k == 1 else
                                simp(z3.Or(self.bits[b], m)))
            elif kind == "Sys":
                self.events.append(band(m, *(self.q[q] for q in a["ctrl"] if q != NONE)))
            elif kind == "SysS":
                self.events.append(m)
            elif kind == "SpinSwap":
                self.events.append(band(m, self.q[a["c"]]))
            elif kind == "Givens":
                # Equal per-bit, enabled angle words give the SAME exact rotation. Disabled
                # rotations and angle zero are identities. No sin/cos or floating arithmetic.
                self.events.extend(band(m, self.q[q]) for q in self.regs[a["reg"]])
            else:
                raise ValueError(f"unsupported lowered op {kind}")
        if self.stack:
            raise ValueError("unbalanced condition stack")


def prove(expressions, variables, timeout_ms, measurements=()):
    bad = simp(z3.Or(*expressions))
    solver = z3.Solver()
    solver.set(timeout=timeout_ms)
    solver.add(bad)
    start = time.monotonic()
    result = solver.check()
    report = {"status": "proved" if result == z3.unsat else
              "counterexample" if result == z3.sat else "inconclusive",
              "seconds": round(time.monotonic() - start, 3)}
    if result == z3.unknown:
        report["reason"] = solver.reason_unknown()
    elif result == z3.sat:
        model = solver.model()
        report["assignment"] = {key: str(model.eval(value, model_completion=True))
                                for key, value in variables.items()}
        if measurements:
            # Sparse but complete: every unlisted outcome is false, including unconstrained
            # measurements under disabled conditions. This makes witnesses reproducible.
            report["measurement_outcomes"] = {
                "default": False,
                "true_indices": [i for i, value in enumerate(measurements)
                                 if z3.is_true(model.eval(value, model_completion=True))],
            }
    return report


def semantic_keys(doc, uniform, second):
    """Independent exact alias decoder. Keys name M_b^dagger M_a, including its signs.

    Array entries are CONSTANTS from the trusted lane map. The keep comparison uses the
    table's full sigma width, so no representative sampling enters this SMT obligation.
    """
    outer = doc["outer"]
    inner = doc["inner_tables"]
    ko, mo, ki, mi = outer["k"], outer["mu"], inner[0]["k"], inner[0]["mu"]
    lo = doc["inner"]["lo"]
    index_width = max(ko+ki, 1)

    def lookup(values, index, bits):
        array = z3.K(z3.BitVecSort(max(bits, 1)), z3.BitVecVal(0, 32))
        for i, value in enumerate(values):
            if value: array = z3.Store(array, z3.BitVecVal(i, max(bits, 1)), z3.BitVecVal(value, 32))
        return z3.Select(array, index)

    def extract(word, low, bits):
        return z3.Extract(low+bits-1, low, word) if bits else z3.BitVecVal(0, 1)

    bucket = extract(uniform, 0, ko)
    draw = z3.ZeroExt(32-max(mo, 1), extract(uniform, ko, mo))
    keep, alt = lookup(outer["keep"], bucket, ko), lookup(outer["alt"], bucket, ko)
    item = z3.If(z3.ULT(draw, keep), z3.ZeroExt(32-max(ko, 1), bucket), alt)
    outer_index = z3.Extract(max(ko, 1)-1, 0, item)
    # Flattening (outer, inner bucket) removes any dependence on the circuit's own lookup.
    keeps = [v for table in inner for v in table["keep"]]
    alts = [v for table in inner for v in table["alt"]]
    def decode(word, offset):
        index = extract(word, offset, ki)
        flat = z3.Concat(outer_index, index) if ko and ki else outer_index if ko else index
        sigma = z3.ZeroExt(32-max(mi, 1), extract(word, offset+ki, mi))
        selected = z3.If(z3.ULT(sigma, lookup(keeps, flat, index_width)),
                         z3.ZeroExt(32-max(ki, 1), index), lookup(alts, flat, index_width))
        spin = extract(word, offset+ki+mi, 1)
        return selected, spin
    a, spin_a = decode(uniform, lo)
    b, spin_b = decode(second, 0)
    n = doc["layout"]["system"] // 2
    # One-body generators use outer spin; square generators use inner spin unless identity.
    # The identity index B is pinned by the evaluator's spec, exported with the tables.
    ob = z3.ULT(item, z3.BitVecVal(n, 32))
    identity = z3.BitVecVal(doc["b"], 32)
    spin_o = z3.If(ob, extract(uniform, ko+mo, 1), z3.BitVecVal(0, 1))
    spin_a = z3.If(z3.Or(ob, a == identity), z3.BitVecVal(0, 1), spin_a)
    spin_b = z3.If(z3.Or(ob, b == identity), z3.BitVecVal(0, 1), spin_b)
    return item, a, b, spin_o, spin_a, spin_b


def verify(doc, timeout_ms=120000):
    if doc.get("schema") != "femoco-symbolic-input-v1":
        raise ValueError("requires an evaluator-exported femoco-symbolic-input-v1")
    n, w = doc["layout"]["uniform"], doc["inner"]["width"]
    c, s, after = z3.Bool("control"), z3.BitVec("uniform", n), z3.BitVec("second", w)
    bit = lambda word, j: z3.Extract(j, j, word) == 1
    uniform, second = [bit(s, j) for j in range(n)], [bit(after, j) for j in range(w)]
    count = sum(isinstance(op, dict) and "Hmr" in op for op in doc["ops"])
    outcomes = [z3.Bool(f"hmr_{i}") for i in range(count)]
    actual = Controller(doc, c, uniform, second, outcomes)
    actual.run(doc["ops"])
    zero = Controller(doc, c, uniform, second, [ZERO] * count)
    zero.run(doc["ops"])
    expected = list(uniform)
    lo = doc["inner"]["lo"]
    expected[lo:lo+w] = second
    restore = [bxor(actual.q[0], c)] + [
        bxor(actual.q[actual.first+j], v) for j, v in enumerate(expected)]
    ancilla = actual.q[actual.first+n:]
    witness = {"control": c, "uniform": s, "second": after}
    checks = {
        "classical_restoration_and_cleanup": prove(
            restore + ancilla + actual.dirty + actual.reflect_bad, witness, timeout_ms, outcomes),
        "measurement_independent_system_trace_and_phase": prove(
            [bxor(a, b) for a, b in zip(actual.events, zero.events)] +
            [bxor(a, b) for a, b in zip(actual.phase, zero.phase)], witness, timeout_ms, outcomes),
    }
    reduction = None
    if "outer" in doc and "b" in doc:
        other_s, other_after = z3.BitVec("other_uniform", n), z3.BitVec("other_second", w)
        other = Controller(doc, c, [bit(other_s, j) for j in range(n)],
                           [bit(other_after, j) for j in range(w)], [ZERO] * count)
        other.run(doc["ops"])
        same_terms = band(*(a == b for a, b in zip(semantic_keys(doc, s, after),
                                                  semantic_keys(doc, other_s, other_after))))
        differences = [bxor(a, b) for a, b in zip(zero.events, other.events)] + [
                       bxor(a, b) for a, b in zip(zero.phase, other.phase)]
        reduction = prove([band(same_terms, z3.Or(*differences))],
                          {**witness, "other_uniform": other_s, "other_second": other_after}, timeout_ms)
    return {
        "schema": "femoco-symbolic-report-v1", "digests": doc["digests"],
        "spec": doc["spec"], "solver": z3.get_full_version(),
        "checker_sha256": hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
        "input_domain": {"controls": 2, "uniform_bits": n, "second_pass_bits": w,
                         "independent_measurement_bits": count},
        "obligations": checks,
        "semantic_trace_and_phase_invariance": reduction,
        "semantic_reduction_proved": reduction is not None and reduction["status"] == "proved",
        "controller_obligations_proved": all(x["status"] == "proved" for x in checks.values()),
        "all_obligations_proved": all(x["status"] == "proved" for x in checks.values()) and
                                  (reduction is None or reduction["status"] == "proved"),
        "full_quantum_equivalence_certified": False,
        "remaining_obligation": "Combine a proved controller/measurement/semantic reduction with "
                                "digest-matched exhaustive term-pair checks. Gaussian term checks "
                                "remain numerical, not exact algebraic quantum equivalence.",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=pathlib.Path)
    parser.add_argument("--out", type=pathlib.Path, required=True)
    parser.add_argument("--timeout-ms", type=int, default=120000)
    args = parser.parse_args()
    args.out.unlink(missing_ok=True)  # stale successful reports never survive failure
    if args.timeout_ms <= 0: parser.error("--timeout-ms must be positive")
    doc = json.loads(args.input.read_text())
    report = verify(doc, args.timeout_ms)
    report["input_sha256"] = hashlib.sha256(args.input.read_bytes()).hexdigest()
    args.out.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"spec": report["spec"], "obligations": report["obligations"]}))
    return 0 if report["all_obligations_proved"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
