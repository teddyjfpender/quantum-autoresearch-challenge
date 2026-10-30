#!/usr/bin/env python3
"""Prove identity/phase/cleanup of a complete supported FEMOOPS1 controller.

Only literal bytes and numeric interface parameters enter generated Lean source.
No candidate Lean source, certificate JSON, or compiler extension is accepted.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import struct
import tempfile

PACKAGE = Path(__file__).resolve().parents[1]
ALLOW = {"propext", "Quot.sound", "Classical.choice"}
AUDIT_NAMES = (
    "FeMoco.Examples.thirty_two_outcome_defect_rejected", "FeMoco.Examples.thirty_two_outcome_witness",
    "FeMoco.Controller.Certificate.resources", "FeMoco.Boolean.checkEqual_sound", "FeMoco.Controller.symbolic_sound",
    "FeMoco.Controller.run_sound", "FeMoco.Controller.check_sound",
    "FeMoco.Controller.Certificate.correct", "FeMoco.Binary.Certificate.correct",
    "FeMoco.Examples.andErase_certificate", "FeMoco.Examples.andErase_correct",
    "FeMoco.Examples.missing_correction_rejected", "FeMoco.Examples.dirty_reset_rejected",
    "FeMoco.Examples.wrong_correction_rejected", "FeMoco.Examples.nonlinear_outcome_defect_rejected",
)


def audit(output: str, expected: tuple[str, ...]) -> None:
    found = {}
    for name, axes in re.findall(r"'([^']+)' depends on axioms: \[([^\]]*)\]", output):
        if name in found:
            raise ValueError(f"duplicate axiom output for {name}")
        found[name] = {x.strip() for x in axes.split(",") if x.strip()}
    # Lean also reports genuinely axiom-free declarations with this spelling.
    for name in re.findall(r"'([^']+)' does not depend on any axioms", output):
        if name in found:
            raise ValueError(f"duplicate axiom output for {name}")
        found[name] = set()
    if set(found) != set(expected):
        raise ValueError("missing or unexpected axiom-audit declarations")
    for name, axes in found.items():
        if axes - ALLOW:
            raise ValueError(f"unapproved axioms in {name}: {sorted(axes - ALLOW)}")


def run(args: list[str], *, extra_path: Path | None = None, timeout: int = 120) -> str:
    env = os.environ.copy()
    if extra_path is not None:
        old = env.get("LEAN_PATH", "")
        env["LEAN_PATH"] = str(extra_path) + (os.pathsep + old if old else "")
    p = subprocess.run(["lake", "env", *args], cwd=PACKAGE, env=env,
                       capture_output=True, text=True, timeout=timeout)
    if p.returncode:
        raise ValueError(f"{' '.join(args[:2])} failed:\n{p.stdout}{p.stderr}")
    return p.stdout + p.stderr


def generated_source(data: bytes, wires: int, inputs: list[int]) -> str:
    if type(wires) is not int or not 1 <= wires <= 4096:
        raise ValueError("wires must be between 1 and 4096")
    if any(type(x) is not int or not 0 <= x < wires for x in inputs) or len(set(inputs)) != len(inputs):
        raise ValueError("inputs must be distinct wire indices within the interface")
    # Work caps are refusals. They do not permit a partial input to certify.
    if len(data) > 16 + 100000 * 56:
        raise ValueError("input exceeds the Lean decoder's complete-program cap")
    byte_text = ",".join(map(str, data))
    input_text = ",".join(map(str, inputs))
    return f'''import FeMoco.Binary
set_option maxRecDepth 100000
set_option maxHeartbeats 2000000
namespace Generated
open FeMoco FeMoco.Controller

def bytes : List Nat := [{byte_text}]
def inputs : List (Fin {wires}) := [{input_text}]
def program : List (Gate {wires}) := (Binary.decode {wires} bytes).getD []
def bounds : Bounds := ⟨(program.map toffoli).sum, (program.map measurements).sum⟩
def certificate : Binary.Certificate bytes {wires} inputs bounds where
  program := program
  decoded := by decide
  controller := {{
    physical := by decide
    accepted := by decide
    toffoli_le := Nat.le_refl _
    measurements_le := Nat.le_refl _
  }}
theorem correct (v : Boolean.Assignment) :
    (runConcrete v certificate.program (evaluate v (Binary.initial {wires} inputs))).wires =
      (evaluate v (Binary.initial {wires} inputs)).wires ∧
    (runConcrete v certificate.program (evaluate v (Binary.initial {wires} inputs))).phase =
      (evaluate v (Binary.initial {wires} inputs)).phase ∧
    (runConcrete v certificate.program (evaluate v (Binary.initial {wires} inputs))).badReset = false :=
  certificate.correct v
end Generated
#print axioms Generated.certificate
#print axioms Generated.correct
'''


def prepare(timeout: int = 120) -> None:
    if run(["lean", "--short-version"], timeout=timeout).strip() != "4.28.0":
        raise ValueError("Lean v4.28.0 is required")
    p = subprocess.run(["lake", "--wfail", "build"], cwd=PACKAGE,
                       text=True, capture_output=True, timeout=timeout)
    if p.returncode:
        raise ValueError(f"Lean build failed:\n{p.stdout}{p.stderr}")


def verify(path: Path, wires: int, inputs: list[int], timeout: int = 120) -> dict:
    prepare(timeout)
    data = path.read_bytes()
    source = generated_source(data, wires, inputs)
    with tempfile.TemporaryDirectory(prefix="femoco-lean-") as temp:
        directory = Path(temp)
        src, obj = directory / "Generated.lean", directory / "Generated.olean"
        src.write_text(source)
        output = run(["lean", "-DwarningAsError=true", "-R", str(directory), "-o", str(obj), str(src)], timeout=timeout)
        audit(output, ("Generated.certificate", "Generated.correct"))
        replay = run(["leanchecker", "--verbose", "Generated"], extra_path=directory, timeout=timeout)
        if "replaying Generated" not in replay.splitlines():
            raise ValueError("kernel replay did not check the generated module")
    if path.read_bytes() != data:
        raise ValueError("input bytes changed during verification")
    return {"schema": "femoco-lean-controller-v1", "controller_identity_certified": True,
            "ops_sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data),
            "wires": wires, "inputs": inputs, "all_assignments": True,
            "kernel_replayed": True, "axiom_allowlist": sorted(ALLOW),
            "resources": "static upper bounds; no sampled mean or peak-live claim",
            "toffoli_upper_bound": sum(struct.unpack_from("<I", data, i)[0] in (13, 14)
                                        for i in range(16, len(data), 56)),
            "measurements_upper_bound": sum(struct.unpack_from("<I", data, i)[0] == 12
                                            for i in range(16, len(data), 56)),
            "full_quantum_equivalence_certified": False, "certifies_energy_error": False,
            "chemical_accuracy_certified": False}


def selfcheck(timeout: int = 120) -> None:
    prepare(timeout)
    audit(run(["lean", "-DwarningAsError=true", "scripts/Audit.lean"], timeout=timeout), AUDIT_NAMES)
    replay = run(["leanchecker", "--verbose", "FeMoco"], timeout=timeout)
    required = {"replaying FeMoco", "replaying FeMoco.Boolean", "replaying FeMoco.Controller",
                "replaying FeMoco.Binary", "replaying FeMoco.Examples"}
    if not required <= set(replay.splitlines()):
        raise ValueError("kernel replay omitted a required module")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("ops", type=Path, nargs="?")
    parser.add_argument("--wires", type=int)
    parser.add_argument("--inputs", default="")
    parser.add_argument("--selfcheck", action="store_true")
    parser.add_argument("--out", type=Path)
    parser.add_argument("--timeout", type=int, default=120)
    args = parser.parse_args()
    try:
        if shutil.which("lake") is None:
            raise ValueError("install the pinned Lean toolchain and run lake build first")
        if args.timeout <= 0:
            raise ValueError("timeout must be positive")
        if args.selfcheck:
            if args.ops or args.out:
                raise ValueError("--selfcheck does not accept an input or report destination")
            selfcheck(args.timeout)
            print("Lean kernel replay and axiom audit passed")
        else:
            if args.ops is None or args.wires is None:
                raise ValueError("ops and --wires are required")
            if args.out and args.out.resolve() == args.ops.resolve():
                raise ValueError("report destination must differ from the input")
            # An old success must never survive a failed verification attempt.
            if args.out:
                args.out.unlink(missing_ok=True)
            inputs = [int(x) for x in args.inputs.split(",") if x]
            report = verify(args.ops, args.wires, inputs, args.timeout)
            if args.out:
                args.out.write_text(json.dumps(report, indent=2) + "\n")
            print(json.dumps(report, sort_keys=True))
        return 0
    except (ValueError, OSError, subprocess.TimeoutExpired) as e:
        parser.exit(1, f"verification refused: {e}\n")


if __name__ == "__main__":
    raise SystemExit(main())
