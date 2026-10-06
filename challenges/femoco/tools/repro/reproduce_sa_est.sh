#!/usr/bin/env bash
# Reproduce the baseline measurements on the two tracks: the shipped architectures built at Low
# et al.'s keep bits (9 + 9) on `reiher-sa-est-v1` / `li-sa-est-v1`, through the whole
# ./benchmark.sh pipeline at the default sample count. Every result is labelled "estimated
# rounding error (Low et al. class)" by the harness (results.tsv note, score.json
# metrics.rounding_class); none is a rigorous result.
#
# Each line: label | spec | FEMOCO_WALK_ARCH | build-time knobs (space-separated NAME=VALUE).
# Every run appends a row to the local run log run/results.tsv (note "sa-est <label> (<knobs>)@<commit>"), saves
# run/<label>.score.json and run/<label>.lanemap.bin, and deletes ops.bin. Heavy: about 10 min per
# Reiher point and 25 min per Li point at 4 threads with the reference engine; set HEAVY to run
# each point under the machine-wide lock.
#
#   tools/repro/reproduce_sa_est.sh                  # every point
#   tools/repro/reproduce_sa_est.sh E-toffL5-R E-L2   # only these labels
#   HEAVY=tools/heavy.sh tools/repro/reproduce_sa_est.sh E-R-lgr
#
# Static counts first (seconds): SA_SPECS=reiher-sa-est-v1,li-sa-est-v1 SA_TWEAKS=imchxL \
#   cargo test --release --lib sa_low::tests::pinned_quick_eval -- --ignored --nocapture
set -u
cd "$(dirname "$0")/../.."
BASE="$(git rev-parse --short HEAD 2>/dev/null || echo nogit)"
HEAVY="${HEAVY:-}"
export RAYON_NUM_THREADS="${RAYON_NUM_THREADS:-4}" CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
mkdir -p run
list='
E-low-R    | reiher-sa-est-v1 | sa-low2025 |
E-toff-R   | reiher-sa-est-v1 | sa-toff    | FEMOCO_SA_TWEAKS=all
E-toffL-R  | reiher-sa-est-v1 | sa-toff    | FEMOCO_SA_TWEAKS=imchxL
E-toffL5-R | reiher-sa-est-v1 | sa-toff    | FEMOCO_SA_TWEAKS=imchxL FEMOCO_SA_INNER_A=5
E-R1       | reiher-sa-est-v1 | sa-pareto  | FEMOCO_SA_CHUNKS=1 FEMOCO_SA_INNER_A=4 FEMOCO_SA_OUTER_A=2 FEMOCO_SA_CARRIES=3 FEMOCO_SA_DROP_ALT=0
E-R2       | reiher-sa-est-v1 | sa-pareto  | FEMOCO_SA_CHUNKS=1 FEMOCO_SA_INNER_A=4 FEMOCO_SA_OUTER_A=2 FEMOCO_SA_CARRIES=0 FEMOCO_SA_DROP_ALT=1
E-Rs3      | reiher-sa-est-v1 | sa-pareto  | FEMOCO_SA_CHUNKS=3 FEMOCO_SA_INNER_A=4 FEMOCO_SA_OUTER_A=2 FEMOCO_SA_CARRIES=0 FEMOCO_SA_DROP_ALT=1
E-low-L    | li-sa-est-v1     | sa-low2025 |
E-toff-L   | li-sa-est-v1     | sa-toff    | FEMOCO_SA_TWEAKS=all
E-toffL-L  | li-sa-est-v1     | sa-toff    | FEMOCO_SA_TWEAKS=imchxL
E-L1       | li-sa-est-v1     | sa-pareto  | FEMOCO_SA_CHUNKS=1 FEMOCO_SA_INNER_A=5 FEMOCO_SA_OUTER_A=2 FEMOCO_SA_CARRIES=3 FEMOCO_SA_DROP_ALT=0
E-L2       | li-sa-est-v1     | sa-pareto  | FEMOCO_SA_CHUNKS=1 FEMOCO_SA_INNER_A=5 FEMOCO_SA_OUTER_A=2 FEMOCO_SA_CARRIES=0 FEMOCO_SA_DROP_ALT=1
E-Ls3      | li-sa-est-v1     | sa-pareto  | FEMOCO_SA_CHUNKS=3 FEMOCO_SA_INNER_A=4 FEMOCO_SA_OUTER_A=2 FEMOCO_SA_CARRIES=0 FEMOCO_SA_DROP_ALT=1
E-R-lgr    | reiher-sa-est-v1 | sa-toff    | FEMOCO_SA_TWEAKS=imchxlgr
E-R-lgrp5  | reiher-sa-est-v1 | sa-toff    | FEMOCO_SA_TWEAKS=imchxlgrp FEMOCO_SA_INNER_A=5
E-L-vd-a4  | li-sa-est-v1     | sa-toff    | FEMOCO_SA_TWEAKS=imchxgrvd FEMOCO_SA_INNER_A=4
E-L-gr-a5  | li-sa-est-v1     | sa-toff    | FEMOCO_SA_TWEAKS=imchxgr FEMOCO_SA_INNER_A=5
'
want=" $* "
while IFS='|' read -r label spec arch knobs; do
  label="$(echo "$label" | xargs)"
  [[ -z "$label" ]] && continue
  if [[ $# -gt 0 && "$want" != *" $label "* ]]; then continue; fi
  spec="$(echo "$spec" | xargs)"; arch="$(echo "$arch" | xargs)"; knobs="$(echo "$knobs" | xargs)"
  start=$(date +%s)
  # shellcheck disable=SC2086
  if env FEMOCO_WALK_ARCH="$arch" FEMOCO_WALK_SPEC="$spec" $knobs \
       $HEAVY ./benchmark.sh --note "sa-est ${label} (${arch} ${knobs})@${BASE}" > "run/${label}.log" 2>&1; then
    cp score.json "run/${label}.score.json"
    cp lanemap.bin "run/${label}.lanemap.bin" 2>/dev/null || true
    s=$(python3 -c "import json;m=json.load(open('score.json'))['metrics'];print(f\"C {m['toffoli']:,.0f}  Q {m['qubits']:,}  ops {m['digests']['ops'][:16]}\")")
    echo "OK   ${label}  ${spec} ${arch} ${knobs}  (${s})  $(( $(date +%s) - start ))s"
  else
    echo "FAIL ${label}  $(grep -E '!!|REJECT|error' "run/${label}.log" | head -1)  $(( $(date +%s) - start ))s"
  fi
  rm -f ops.bin lanemap.bin
done <<< "$list"
echo DONE
