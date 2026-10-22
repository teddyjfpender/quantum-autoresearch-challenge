#!/usr/bin/env python3
"""How far can a defect on a small fraction of lanes move the qubitized walk's eigenphase?

The companion calculation of spec/SAMPLING.md. Standard library only.

Model (flat SELECT, trivial system). The step is W = (2|G><G| - I) U with |G> the uniform
state over lanes and U = diag(u_s). An honest block encoding has u_s = +1 or -1 with
<G|U|G> = a, and the walk's eigenphases on the start state are +-theta0, cos(theta0) = a.
A defect replaces u_s on a fraction f of lanes. W is a rank-one perturbation of a diagonal
unitary, so the eigenvalues that carry the start state |G> are the roots z of

    sum_c 2 w_c / (1 + z conj(u_c)) = 1,        one root per distinct value u_c of weight w_c,

and the weight of |G> on each follows from the eigenvector x_s = conj(u_s) / (1 + z conj(u_s)).

Defects compared, each on a fraction f of lanes:
  sign      u -> -u             Hermitian. The step stays a reflection and qubitization stays
                                exact for a slightly different operator: the shift is O(f).
  phase-i   u -> i u            not Hermitian, away from resonance: O(f).
  resonant  u -> -e^{i theta0}  not Hermitian, tuned so the defect's own eigenvalue lies on the
                                ground eigenphase. The two mix with amplitude sqrt(f) and the
                                level splits by about sqrt(2 f).
  pair      half the defect on each of e^{+i theta0}, e^{-i theta0}: both branches split.

    python3 walk_defect.py            the table for both tracks, and the lanes needed
    python3 walk_defect.py --lanes    the linear bound per lane count (SAMPLING.md section 1.1)
"""
from __future__ import annotations

import cmath
import math
import sys

DELTA = 1e-6
EPSILON_HA = 1.6e-3
DEFAULT_LANES = 2 ** 19
# The proposed full stage (score::CERTIFYING_SAMPLES; the judge's full stage is 2^19 today).
FULL_STAGE_LANES = 2 ** 24
# Measured sliced-engine cost per lane in microseconds, by thread count, for the two headline
# circuits (reiher/outer-hi-reconstruct-r1, li/composed-alias-m8), from the difference of a
# 2^22-lane and a 2^19-lane run so that the fixed cost of reading ops.bin drops out. Apple
# M5 Max, 2026-10-22, RAYON_NUM_THREADS as given. The judge's runner (ubuntu-24.04, 4 vCPU) is
# slower per core; the projections below are for this machine and are labelled so.
LANE_MICROSECONDS = {"reiher": {4: 5.68, 16: 1.94}, "li": {4: 18.3, 16: 6.76}}
# Lambda and lambda_eff in Ha (Low et al. 2025, Table V; challenges/femoco/targets.json).
TRACKS = {"reiher": (58.344, 21.3674), "li": (179.7296, 43.6538)}
DEFECTS = ("sign", "phase-i", "resonant", "pair")


def ground_cosine(big_lambda: float, lambda_eff: float) -> float:
    """cos(theta0) of the spectrum-amplified ground point: sin(theta0) = lambda_eff / Lambda."""
    return -math.sqrt(1 - (lambda_eff / big_lambda) ** 2)


def clusters(a: float, f: float, defect: str) -> list[tuple[float, complex]]:
    """(weight, u) of each distinct lane value: the honest +1 and -1 lanes and the defect."""
    theta0 = math.acos(a)
    plus, minus = (1 + a) / 2 - f, (1 - a) / 2
    if not 0 < f < plus:
        raise ValueError("the defect is taken from the +1 lanes and must fit there")
    resonant = -cmath.exp(1j * theta0)
    extra = {
        "sign": [(f, -1 + 0j)],
        "phase-i": [(f, 1j)],
        "resonant": [(f, resonant)],
        "pair": [(f / 2, resonant), (f / 2, resonant.conjugate())],
    }[defect]
    merged: dict[complex, float] = {}
    for weight, u in [(plus, 1 + 0j), (minus, -1 + 0j), *extra]:
        key = complex(round(u.real, 14), round(u.imag, 14))
        merged[key] = merged.get(key, 0.0) + weight
    return [(weight, u) for u, weight in merged.items()]


def _multiply(p: list[complex], q: list[complex]) -> list[complex]:
    out = [0j] * (len(p) + len(q) - 1)
    for i, x in enumerate(p):
        for j, y in enumerate(q):
            out[i + j] += x * y
    return out


def _roots(coefficients: list[complex]) -> list[complex]:
    """All roots of a polynomial (lowest degree first) by Durand-Kerner iteration."""
    lead = coefficients[-1]
    monic = [c / lead for c in coefficients]
    degree = len(monic) - 1
    roots = [(0.4 + 0.9j) ** k for k in range(degree)]
    for _ in range(2000):
        moved = 0.0
        for i, z in enumerate(roots):
            value = sum(c * z ** k for k, c in enumerate(monic))
            for j, other in enumerate(roots):
                if j != i:
                    value /= z - other
            roots[i] = z - value
            moved = max(moved, abs(value))
        if moved < 1e-15:
            break
    return roots


def spectral_measure(cl: list[tuple[float, complex]]) -> list[tuple[complex, float]]:
    """(eigenvalue, weight of the start state |G>) for every eigenvalue that carries |G>."""
    denominator = [1 + 0j]
    for _, u in cl:
        denominator = _multiply(denominator, [1, u.conjugate()])
    numerator = [0j] * len(denominator)
    for i, (weight, _) in enumerate(cl):
        term = [2 * weight + 0j]
        for j, (_, other) in enumerate(cl):
            if j != i:
                term = _multiply(term, [1, other.conjugate()])
        for k, c in enumerate(term):
            numerator[k] += c
    out = []
    for z in _roots([n - d for n, d in zip(numerator, denominator)]):
        x = [u.conjugate() / (1 + z * u.conjugate()) for _, u in cl]
        norm = sum(w * abs(v) ** 2 for (w, _), v in zip(cl, x))
        overlap = sum(w * v for (w, _), v in zip(cl, x))
        out.append((z, abs(overlap) ** 2 / norm))
    return out


def displaced_peaks(big_lambda: float, lambda_eff: float, f: float, defect: str,
                    floor: float = 0.01) -> list[tuple[float, float]]:
    """(|energy error| in Ha, weight) of each peak phase estimation would report, heaviest
    first. The energy is read as E = Lambda cos(phase)."""
    a = ground_cosine(big_lambda, lambda_eff)
    peaks = [(big_lambda * abs(math.cos(cmath.phase(z)) - a), w)
             for z, w in spectral_measure(clusters(a, f, defect)) if w > floor]
    return sorted(peaks, key=lambda p: -p[1])


def worst_shift(big_lambda: float, lambda_eff: float, f: float, defect: str) -> float:
    """The largest energy error among the peaks that carry at least a tenth of the weight."""
    return max(error for error, weight in displaced_peaks(big_lambda, lambda_eff, f, defect) if weight >= 0.1)


def scaling_exponent(big_lambda: float, lambda_eff: float, defect: str,
                     low: float = 1e-9, high: float = 1e-7) -> float:
    """d log(shift) / d log(f) between two defect fractions."""
    return (math.log(worst_shift(big_lambda, lambda_eff, high, defect) / worst_shift(big_lambda, lambda_eff, low, defect))
            / math.log(high / low))


def nested_wrong_weight(lanes: int) -> float:
    """The operator weight that may sit on wrong control-1 lanes after `lanes` sampled lanes all
    passed: 2 f_p1 + f_d1 with about a quarter of the lanes in each of the two control-1 classes
    (spec/DESIGN.md section 16.4)."""
    f = math.log(1 / DELTA) / (lanes / 4)
    return 3 * f


def lanes_needed(big_lambda: float, lambda_eff: float) -> dict[str, float]:
    """Sampled lanes for a 1.6 mHa statement at confidence 1 - 2 delta, by what is assumed."""
    per_weight = 12 * math.log(1 / DELTA)          # wrong weight g = per_weight / lanes
    return {
        # 2 Lambda g <= epsilon
        "hermitian": per_weight * 2 * big_lambda / EPSILON_HA,
        # lambda_eff sqrt(2 g) <= epsilon
        "non_hermitian": per_weight * 2 * (lambda_eff / EPSILON_HA) ** 2,
    }


def rare_branch_hit_probability(lambda_eff: float, lanes: int = DEFAULT_LANES) -> float:
    """A defect that needs a rare measurement outcome fails once per step with probability up
    to f. Phase estimation runs T = pi lambda_eff / (2 epsilon) steps."""
    steps = math.pi * lambda_eff / (2 * EPSILON_HA)
    f = math.log(1 / DELTA) / (lanes / 2)
    return 1 - math.exp(-steps * f)


def linear_bound_mha(lanes: int, big_lambda: float) -> float:
    """`2 Lambda g` in mHa after `lanes` sampled lanes all passed: the bound on the encoded
    operator's distance from the declared one, which the reflection check makes unconditional
    (spec/SAMPLING.md section 2.1). Single draw, confidence 1 - 2 delta."""
    return 2 * big_lambda * nested_wrong_weight(lanes) * 1e3


def lanes_table(threads: int = 4) -> str:
    """The linear bound per lane count for both tracks, with the sliced engine's projected
    wall time on this machine at `threads` threads (LANE_MICROSECONDS), as a Markdown table."""
    head = "| Lanes | `g` |" + "".join(f" {t.capitalize()} bound, mHa | {t.capitalize()} lanes, min ({threads} thr) |" for t in TRACKS)
    rows = [head, "| ---: | ---: |" + " ---: | ---: |" * len(TRACKS)]
    for e in range(19, 29):
        lanes = 2 ** e
        cells = [f"`2^{e}`", f"{nested_wrong_weight(lanes):.2e}"]
        for track, (big_lambda, _) in TRACKS.items():
            bound = linear_bound_mha(lanes, big_lambda)
            mark = " *" if bound < EPSILON_HA * 1e3 else ""
            minutes = lanes * LANE_MICROSECONDS[track][threads] / 60e6
            cells += [f"{bound:.2f}{mark}", f"{minutes:.1f}"]
        rows.append("| " + " | ".join(cells) + " |")
    return "\n".join(rows)


def main() -> None:
    if "--lanes" in sys.argv[1:]:
        print(lanes_table())
        print()
        print("* below chemical accuracy (1.6 mHa). Lane time is the sliced engine on the machine of "
              "LANE_MICROSECONDS, without the reflection check.")
        for track, (big_lambda, _) in TRACKS.items():
            need = lanes_needed(big_lambda, 0.0)["hermitian"]
            print(f"{track}: 1.6 mHa needs 2^{math.log2(need):.1f} lanes; the smallest power of two is "
                  f"2^{math.ceil(math.log2(need))}, where the bound is "
                  f"{linear_bound_mha(2 ** math.ceil(math.log2(need)), big_lambda):.2f} mHa; "
                  f"at the proposed full stage 2^{int(math.log2(FULL_STAGE_LANES))} it is "
                  f"{linear_bound_mha(FULL_STAGE_LANES, big_lambda):.2f} mHa")
        return
    for track, (big_lambda, lambda_eff) in TRACKS.items():
        g = nested_wrong_weight(DEFAULT_LANES)
        print(f"{track}: Lambda {big_lambda} Ha, lambda_eff {lambda_eff} Ha")
        print(f"  after 2^19 lanes the wrong weight is at most g = {g:.3e}")
        print(f"    if every lane is Hermitian: operator error <= 2 Lambda g = {2 * big_lambda * g * 1e3:.1f} mHa")
        print(f"    if not: phase scale sqrt(2 g) = {math.sqrt(2 * g):.4f} rad, about {lambda_eff * math.sqrt(2 * g) * 1e3:.0f} mHa")
        print(f"  {'f':>8} " + " ".join(f"{d:>10}" for d in DEFECTS) + "   worst displaced peak, mHa")
        for f in (1e-5, 1.9e-6, 1e-7, 1e-8, 1e-9):
            shifts = [worst_shift(big_lambda, lambda_eff, f, d) * 1e3 for d in DEFECTS]
            print(f"  {f:8.1e} " + " ".join(f"{s:10.3f}" for s in shifts))
        print("  scaling exponents: " + ", ".join(f"{d} {scaling_exponent(big_lambda, lambda_eff, d):.2f}" for d in DEFECTS))
        need = lanes_needed(big_lambda, lambda_eff)
        print(f"  lanes for a 1.6 mHa statement: Hermitian lanes only 2^{math.log2(need['hermitian']):.1f}, "
              f"any defect 2^{math.log2(need['non_hermitian']):.1f}")
        print(f"  a rare-measurement-branch defect at today's bound is hit in a run with probability "
              f"{rare_branch_hit_probability(lambda_eff):.2f}\n")


if __name__ == "__main__":
    main()
