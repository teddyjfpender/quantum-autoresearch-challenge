#!/usr/bin/env bash
# Wait for a server submission to reach a terminal state, then print its verdict.
#
#   tools/server/poll.sh ID [--interval SECONDS] [--timeout SECONDS] [--once]
#
# ID is the artifact sha256 that tools/server/submit.sh printed (a unique prefix is not
# accepted by the server; pass the full id). States: queued -> screening -> screened ->
# confirming -> accepted | rejected | failed. --once prints the current state and exits
# (exit 3 if not terminal). At a terminal state it prints: verdict, spec, declared cell,
# screen (4,096 lanes) and confirmation (524,288 lanes, server seed) C_step and Q_peak, lambda,
# lambda x C, and, for an accepted artifact, its /frontier flags (global best, global Pareto,
# cell Pareto). Exit status: 0 accepted, 1 rejected or failed, 2 timeout.
# Environment: FEMOCO_SERVER (default http://127.0.0.1:8753).
set -euo pipefail
server="${FEMOCO_SERVER:-http://127.0.0.1:8753}"
[[ $# -ge 1 ]] || { sed -n '2,12p' "$0"; exit 2; }
id="$1"; shift
interval=30 timeout=7200 once=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --interval) interval="$2"; shift 2 ;;
    --timeout) timeout="$2"; shift 2 ;;
    --once) once=1; shift ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
done
[[ "$id" =~ ^[0-9a-f]{64}$ ]] || { echo "!! id must be a 64-hex sha256" >&2; exit 2; }

start=$(date +%s) last=""
while :; do
  body="$(curl -sf "$server/submissions/$id" || true)"
  [[ -n "$body" ]] || { echo "!! no such submission (or server down): $id" >&2; exit 1; }
  state="$(printf '%s' "$body" | python3 -c 'import json,sys;print(json.load(sys.stdin)["state"])')"
  if [[ "$state" != "$last" ]]; then echo "$(date +%H:%M:%S) $state"; last="$state"; fi
  case "$state" in accepted|rejected|failed) break ;; esac
  [[ "$once" -eq 1 ]] && exit 3
  (( $(date +%s) - start > timeout )) && { echo "!! timeout after ${timeout}s in state $state" >&2; exit 2; }
  sleep "$interval"
done

frontier="$(curl -sf "$server/frontier?spec=$(printf '%s' "$body" | python3 -c 'import json,sys;print(json.load(sys.stdin)["spec"])')" || echo '{}')"
python3 - "$body" "$frontier" <<'PY'
import json, sys
r = json.loads(sys.argv[1]); fr = json.loads(sys.argv[2] or "{}")
def m(stage):
    v = r.get(stage) or {}
    return v.get("metrics", {}), v.get("score")
print(f"verdict   {r['state'].upper()}" + (f"  ({r['error']})" if r.get("error") else ""))
print(f"id        {r['id']}")
print(f"spec      {r['spec']}")
print(f"cell      {r['declared_cell']}  (declared; architecture_verified={r['architecture_verified']})")
print(f"circuit   {r.get('circuit_sha256','')}")
print(f"verifier  {r.get('verifier_sha256','')}")
for stage, name in (("quick", "screen  4,096"), ("full", "confirm 524,288")):
    mt, sc = m(stage)
    if mt:
        lam = mt.get("lambda")
        print(f"{name}: C_step {mt['toffoli']:,.3f}  Q_peak {mt['qubits']:,}  lambda {lam:.6f}  "
              f"lambda x C {lam * mt['toffoli']:,.1f}  score {sc}")
if r.get("server_seed"):
    print(f"seed      {r['server_seed']}")
for row in fr.get("submissions", []):
    if row["id"] == r["id"]:
        print(f"frontier  global_best={row['global_best']}  global_pareto={row['global_pareto']}  "
              f"cell_pareto={row['cell_pareto']}")
PY
[[ "$state" == accepted ]]
