#!/usr/bin/env python3
"""Pretty-print the validation server's accepted archive, per spec and per declared cell.

    python3 tools/server/frontier.py                 # both tracks
    python3 tools/server/frontier.py reiher-sa-est-v1    # named specs
    python3 tools/server/frontier.py --json li-sa-est-v1 # raw /frontier JSON

Columns: G = global Pareto in (lambda x C_step, Q_peak) within the spec, c = Pareto within its
declared cell, * = global best score. C_step and Q_peak are the 524,288-lane confirmation's
(server seed). Rows are sorted by cell, then lambda x C. Environment: FEMOCO_SERVER (default
http://127.0.0.1:8753). The server compares within a spec only; so does this table.
Cells are DECLARED, not verified (tools/server/README.md).
"""
import json
import os
import sys
import urllib.request

SERVER = os.environ.get("FEMOCO_SERVER", "http://127.0.0.1:8753")
DEFAULT = ["reiher-sa-est-v1", "li-sa-est-v1"]


def get(path):
    with urllib.request.urlopen(SERVER + path, timeout=30) as r:
        return json.load(r)


def show(spec):
    fr = get(f"/frontier?spec={spec}")
    rows = fr.get("submissions", [])
    print(f"== {spec}: {len(rows)} accepted")
    if not rows:
        return
    # toffoli is lambda x C / lambda; the server does not return C directly in /frontier, so ask
    # each record for its confirmation metrics.
    for row in rows:
        rec = get(f"/submissions/{row['id']}")
        mt = (rec.get("full") or {}).get("metrics", {})
        row["C"], row["lambda"] = mt.get("toffoli"), mt.get("lambda")
    rows.sort(key=lambda r: (r["declared_cell"], r["lambda_times_toffoli"]))
    print(f"   {'cell':<30} {'id':<12} {'C_step':>11} {'Q_peak':>7} {'lambda x C':>12}  flags")
    last = None
    for r in rows:
        cell = r["declared_cell"] if r["declared_cell"] != last else ""
        last = r["declared_cell"]
        flags = ("G" if r["global_pareto"] else "-") + ("c" if r["cell_pareto"] else "-") + \
                ("*" if r["global_best"] else "")
        print(f"   {cell:<30} {r['id'][:12]:<12} {r['C']:>11,.3f} {int(r['qubits']):>7,} "
              f"{r['lambda_times_toffoli']:>12,.1f}  {flags}")


def main(argv):
    raw = "--json" in argv
    specs = [a for a in argv if not a.startswith("--")] or DEFAULT
    for spec in specs:
        try:
            if raw:
                print(json.dumps(get(f"/frontier?spec={spec}"), indent=1))
            else:
                show(spec)
        except Exception as e:  # noqa: BLE001 - a CLI report
            print(f"== {spec}: error {e}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
