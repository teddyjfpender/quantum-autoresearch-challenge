#!/usr/bin/env python3
"""Suggest the server's architecture cell for an sos-sa build from its knobs.

    python3 tools/server/cell.py ARCH [FEMOCO_X=v ...]      # prints delivery/erasure/outer_slot

The rule reads the build-time knobs of the arches whose levers it names (sa-toff, sa-pareto,
sa-lowq, sa-low2025*):

  delivery  : streamed if chunks >= 2; else onehot-split if lever v or w; else onehot if lever
              u, r, v or w; else word.
  erasure   : ladder+gated if a held ladder (l, L, or FEMOCO_SA_CARRIES > 0) and a gated
              erasure (sa-toff g, or g in FEMOCO_SA_NARROW); else ladder; else gated; else plain.
  outer_slot: dropped if sa-toff d or sa-pareto FEMOCO_SA_DROP_ALT=1; else held.

This is a SUGGESTION. The server's cell is declared, not verified (tools/server/README.md): a new
mechanism the three axes do not name still has to be declared honestly by its author, and the
suggestion cannot see code changes in src/walk. Exit status 2 for an arch it does not know.
"""
import sys

KNOWN = {"sa-toff", "sa-pareto", "sa-lowq", "sa-low2025", "sa-low2025-bothspin"}


def cell(arch: str, knobs: dict) -> str:
    if arch not in KNOWN:
        raise ValueError(f"no descriptor rule for arch {arch!r}")
    tw = knobs.get("FEMOCO_SA_TWEAKS", "all") if arch == "sa-toff" else ""
    if tw in ("all", "none"):
        tw = ""  # `all` turns on i m c h x only (no l, g, u, d): word/plain/held
    narrow = knobs.get("FEMOCO_SA_NARROW", "") if arch == "sa-pareto" else ""
    default_chunks = {"sa-pareto": 1, "sa-lowq": 2}.get(arch, 1)
    chunks = int(knobs.get("FEMOCO_SA_CHUNKS", default_chunks)) if arch in ("sa-pareto", "sa-lowq") else 1
    carries = int(knobs.get("FEMOCO_SA_CARRIES", 0)) if arch == "sa-pareto" else 0
    if chunks >= 2:
        delivery = "streamed"
    elif "v" in tw or "w" in tw:
        delivery = "onehot-split"
    elif any(c in tw for c in "urvw"):
        delivery = "onehot"
    else:
        delivery = "word"
    ladder = "l" in tw or "L" in tw or carries > 0
    gated = "g" in tw or "g" in narrow
    erasure = "ladder+gated" if ladder and gated else "ladder" if ladder else "gated" if gated else "plain"
    dropped = "d" in tw or (arch == "sa-pareto" and knobs.get("FEMOCO_SA_DROP_ALT", "0") not in ("", "0"))
    return f"{delivery}/{erasure}/{'dropped' if dropped else 'held'}"


def main(argv):
    if not argv:
        print(__doc__.strip().splitlines()[2], file=sys.stderr)
        return 2
    knobs = dict(a.split("=", 1) for a in argv[1:])
    try:
        print(cell(argv[0], knobs))
    except ValueError as e:
        print(f"cell.py: {e}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
