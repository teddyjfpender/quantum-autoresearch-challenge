"""FeMoco: which registry architecture a manifest's build knobs produce, for the shipped builders.

The architecture a submission declares decides which board it competes on, so it cannot be left
to the manifest alone. For the builders that ship with the challenge the answer follows from the
build knobs, mirroring how `src/walk/sa_low` reads them. A builder this file does not know
(new circuit code) returns None, and the submission then needs a maintainer's approval.
"""
from __future__ import annotations

import re

RANK = re.compile(r"q\d+(?:_\d+)?(?:\.\d+)?")


def architecture_of(build: dict[str, str]) -> str | None:
    builder = build.get("FEMOCO_WALK_ARCH", "sa-low2025")
    chunks = build.get("FEMOCO_SA_CHUNKS")
    if builder in ("sa-low2025", "sa-low2025-bothspin"):
        return "qroam-word"
    if builder == "sa-lowq":
        return "streamed" if chunks is None or chunks.isdigit() and int(chunks) > 1 else "qroam-word"
    if builder == "sa-pareto":
        return "streamed" if chunks is not None and chunks.isdigit() and int(chunks) > 1 else "qroam-word"
    if builder != "sa-toff":
        return None
    tweaks = build.get("FEMOCO_SA_TWEAKS", "all")
    if tweaks == "all":
        return "qroam-word"
    if RANK.search(tweaks):
        return "rank-scheduled"
    # Lever `q<b>` is out; letters after the first '.' that is followed by a letter are
    # extension levers and carry no group count.
    core = re.split(r"\.(?=[A-Za-z])", tweaks, maxsplit=1)[0]
    digit = next((int(c) for c in core if c.isdigit()), None)
    if (digit is not None and digit >= 2) or "v" in core or "w" in core:
        return "onehot-split"
    if "u" in core or "r" in core:
        return "onehot"
    return "qroam-word"
