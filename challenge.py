#!/usr/bin/env python3
"""Participant and judge command line. See `python3 challenge.py --help`."""
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent / "tools"))

from qac.cli import main  # noqa: E402

if __name__ == "__main__":
    sys.exit(main())
