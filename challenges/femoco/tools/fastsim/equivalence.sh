#!/usr/bin/env bash
# Lane-engine equivalence: runs every check the fast engine must pass against the reference
# (spec/FAST-EVALUATOR.md sections 5 and 10) and summarises them.
#
# Usage (from the challenge directory):
#   tools/fastsim/equivalence.sh [--candidate NAME] [--quick] [--k K]
#                                [--fuzz N] [--out DIR] [--only STAGES]
#
#   --candidate NAME  the engine compared with the reference (default: $FEMOCO_EQUIV_ENGINE, else
#                     `sliced`). `reference` checks the harness itself and the reference's
#                     thread-count determinism.
#   --quick           smaller everything: K = 1024, 1000 fuzz cases, 6 of the pinned circuits,
#                     no full-size mutants.
#   --k K             lanes per artifact run (default 4096).
#   --fuzz N          fuzz cases per fuzz seed (default 5000; two seeds).
#   --out DIR         where logs go (default: target/equivalence/<timestamp>).
#   --only STAGES     comma list of stages to run (selftest,fuzz,guard,fixtures,suites,full-mutants,
#                     pinned,binary).
#
# Stages:
#   selftest      the harness catches each deliberately wrong engine (equiv::SELFTEST_ENGINES)
#                 in the fuzzer and in the differential runner: a harness that cannot see a
#                 difference proves nothing.
#   fuzz          tests/equiv_fuzz.rs: random small valid and broken op streams, lane maps, seeds.
#   guard         tests/equiv_guard.rs: near-threshold adversarial circuits for the folding
#                 tier's guard band (lanes within 1e-15..1e-13 of AD_TOL, all-passing batches,
#                 one failing lane per batch); a guard-band counter, when the engine reports one,
#                 must be nonzero; and selftest-unguarded (a fold with no guard band) is caught.
#   fixtures      tests/equiv_mutants.rs fixture sweeps: op-stream mutants of the four small
#                 fixture circuits (flat, df, thc, nested) and the lambda sign-byte difference.
#   suites        every existing test that evaluates a circuit (all mutant suites, adversarial
#                 tests, walk tests), with `FEMOCO_EQUIV_ENGINE` set: each evaluation is also run
#                 by the candidate at 1 and 4 threads and must agree.
#   full-mutants  op-stream mutant sweeps of a full-size circuit (sa-toff imchxlgr on reiher-sa-v1).
#   pinned        the pinned spectrum-amplification circuits (tests/sa_circuits/list.rs), three
#                 seed modes each (ordinary, server seed, audit beacon).
#   binary        the trusted binaries: eval_circuit --engine/--dump-verdicts against the default
#                 path, and eval_diff on built artifacts.
#
# Every comparison is appended as one JSON line to <out>/equiv.jsonl; the summary counts them.
# Exit status: 0 when every stage passed.
set -uo pipefail
cd "$(dirname "$0")/../.."
here=$(pwd)
if [[ -z ${FEMOCO_HEAVY_HELD:-} ]]; then
  FEMOCO_HEAVY_HELD=1 exec tools/heavy.sh "$0" "$@"
fi
export RAYON_NUM_THREADS=${RAYON_NUM_THREADS:-4} CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}

cand=${FEMOCO_EQUIV_ENGINE:-sliced}
quick=0 K=4096 fuzz=5000 out="" only=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --candidate) cand="$2"; shift 2 ;;
    --quick) quick=1; shift ;;
    --k) K="$2"; shift 2 ;;
    --fuzz) fuzz="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --only) only="$2"; shift 2 ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
done
if [[ $quick == 1 ]]; then K=1024; fuzz=1000; fi
out=${out:-$here/target/equivalence/$(date +%Y%m%d-%H%M%S)-$cand}
mkdir -p "$out"
out=$(cd "$out" && pwd)
log="$out/equiv.jsonl"; touch "$log"
export FEMOCO_EQUIV_LOG="$log" FEMOCO_EQUIV_K="$K" FEMOCO_EQUIV_THREADS=1,4 FEMOCO_EQUIV_REF_THREADS=4
declare -a results=()
want() { [[ -z $only || ",$only," == *",$1,"* ]]; }
record() { results+=("$1|$2|$3"); printf '\n### %-13s %s  %s\n' "$1" "$2" "$3"; }
stage() { printf '\n=== %s ===\n' "$1"; }

stage build
cargo build --release --locked --no-default-features --bin eval_circuit --bin eval_diff 2>&1 | tail -2 || exit 2
cargo test --release --locked --no-run 2>&1 | tail -1
EVAL=$here/target/release/eval_circuit DIFF=$here/target/release/eval_diff
if ! $DIFF --root /nonexistent --candidate "$cand" >/dev/null 2>"$out/probe.err"; then
  if grep -q "no engine" "$out/probe.err"; then
    echo "!! engine '$cand' is not in this build (eval_diff: $(cat "$out/probe.err"))" >&2
    exit 2
  fi
fi
echo "candidate $cand, K $K, fuzz $fuzz x 2 seeds, logs in $out"

# cargo test with the candidate set; the test log goes to <out>/<name>.log.
ctest() { # name, env..., -- cargo test args...
  local name="$1"; shift
  local envs=()
  while [[ $1 != -- ]]; do envs+=("$1"); shift; done; shift
  env FEMOCO_EQUIV_ENGINE="$cand" ${envs[@]+"${envs[@]}"} cargo test --release --locked "$@" >"$out/$name.log" 2>&1
}

if want selftest; then
  stage "selftest (each wrong engine must be caught)"
  ok=1 caught=0 total=0
  for e in selftest-tally selftest-verdict selftest-accept selftest-hmrkey selftest-message selftest-threads; do
    total=$((total + 1))
    env FEMOCO_EQUIV_ENGINE=$e FEMOCO_EQUIV_FUZZ_CASES=300 FEMOCO_EQUIV_LOG=/dev/null \
        cargo test --release --locked --test equiv_fuzz >"$out/selftest-$e.log" 2>&1
    # Caught means the fuzzer reported differences, not merely that the test failed.
    if grep -q '[0-9] differences:' "$out/selftest-$e.log"; then
      caught=$((caught + 1)); echo "  $e: caught by the fuzzer"
    else
      echo "  $e: NOT caught by the fuzzer"; ok=0
    fi
  done
  record selftest "$([[ $ok == 1 ]] && echo PASS || echo FAIL)" "$caught of $total wrong engines caught by the fuzzer"
fi

if want fuzz; then
  stage "fuzz ($fuzz cases x 2 seeds)"
  ok=1
  for s in 1 2; do
    ctest fuzz-$s FEMOCO_EQUIV_FUZZ_CASES="$fuzz" FEMOCO_EQUIV_FUZZ_SEED=$s -- \
      --test equiv_fuzz -- --nocapture || ok=0
  done
  n=$(grep -c '"kind":"fuzz"' "$log" || true)
  record fuzz "$([[ $ok == 1 ]] && echo PASS || echo FAIL)" "$(python3 - "$log" <<'PY'
import json,sys
r=[json.loads(l) for l in open(sys.argv[1]) if '"kind":"fuzz"' in l]
r=r[-2:]
print(f"{sum(x['cases'] for x in r)} cases, {sum(x['engine_runs'] for x in r)} engine runs, "
      f"{sum(x['lanes'] for x in r)} lanes, {sum(x['valid_runs_passing'] for x in r)} valid passing, "
      f"{sum(x['runs_with_execution_error'] for x in r)} with execution errors, "
      f"{sum(len(x['differences']) for x in r)} differences")
PY
)"
fi

if want guard; then
  stage "guard band: near-threshold adversarial cases"
  ctest guard -- --test equiv_guard -- --nocapture --test-threads=1
  rc=$?
  record guard "$([[ $rc == 0 ]] && echo PASS || echo FAIL)" "$(python3 - "$log" "$cand" <<'PY'
import json,sys
r=[json.loads(l) for l in open(sys.argv[1])]
g=[x for x in r if x.get("kind")=="guard-band" and x.get("engine")==sys.argv[2]]
s=[x for x in r if x.get("kind")=="guard-band-selftest"]
gc=sum(v for x in g for k,v in x.get("engine_counters",[]) if "guard" in k)
print(f"{len(g)} cases, {sum(x['lanes'] for x in g)} lanes, {sum(x['within_1e-14'] for x in g)} within 1e-14 of AD_TOL, "
      f"{sum(not x['equal'] for x in g)} unequal; guard fallbacks reported {gc}; "
      f"unguarded fold caught in {sum(len(x['caught']) for x in s)} cases")
PY
)"
fi

if want fixtures; then
  stage "fixture mutant sweeps"
  ctest fixtures FEMOCO_EQUIV_MUTANT_PER=2 FEMOCO_EQUIV_MUTANT_SEEDS=2 -- \
    --test equiv_mutants -- --nocapture --test-threads=1
  rc=$?
  record fixtures "$([[ $rc == 0 ]] && echo PASS || echo FAIL)" "$(grep -h '^test result' "$out/fixtures.log" | tail -1)"
fi

if want suites; then
  stage "existing suites with the candidate hooked in"
  # Every test target that evaluates through the hook (equiv::evaluate_checked, run_checked,
  # run_nested_checked), directly or through a shared fixture module, plus the library's tests.
  args=()
  for f in tests/*.rs; do
    t=$(basename "$f" .rs)
    [[ $t == equiv_* ]] && continue
    if grep -q 'evaluate_checked\|run_checked\|run_nested_checked' "$f" ||
       grep -qE '^mod (harness_common|df_common|thc_common|nested_common);' "$f"; then
      args+=(--test "$t")
    fi
  done
  before=$(wc -l <"$log" 2>/dev/null || echo 0)
  ctest suites -- --no-fail-fast --lib "${args[@]}" -- --test-threads=2
  rc=$?
  passed=$(grep -h '^test result' "$out/suites.log" | awk '{p+=$4; f+=$6} END {print p" tests passed, "f" failed"}')
  n=$(tail -n +$((before + 1)) "$log" | python3 -c "
import json,sys
r=[json.loads(l) for l in sys.stdin]
r=[x for x in r if x.get('kind') in ('evaluate','lanes')]
print(len(r),'hooked comparisons,',sum(not x['equal'] for x in r),'unequal')")
  record suites "$([[ $rc == 0 ]] && echo PASS || echo FAIL)" "$((${#args[@]} / 2)) test targets + lib, $passed; $n"
fi

if want full-mutants && [[ $quick == 0 ]]; then
  stage "full-size mutant sweeps"
  ctest full-mutants -- --test equiv_mutants -- --ignored --nocapture --test-threads=1 full_
  rc=$?
  record full-mutants "$([[ $rc == 0 ]] && echo PASS || echo FAIL)" "$(grep -h '^test result' "$out/full-mutants.log" | tail -1)"
fi

if want pinned; then
  stage "pinned SA circuits"
  if [[ $quick == 1 ]]; then
    ctest pinned -- --test equiv_pinned -- --ignored --nocapture --test-threads=1 \
      est_low_r est_toffl_r est_rs3 est_r_lgr est_l_vd_a4 est_l_gr_a5
  else
    ctest pinned -- --test equiv_pinned -- --ignored --nocapture --test-threads=1
  fi
  rc=$?
  record pinned "$([[ $rc == 0 ]] && echo PASS || echo FAIL)" "$(grep -h '^test result' "$out/pinned.log" | tail -1)"
fi

if want binary; then
  stage "trusted binaries: eval_circuit --engine and eval_diff on artifacts"
  art="$out/artifacts"; mkdir -p "$art"
  ok=1
  build_art() { # name, env...
    local name="$1"; shift
    local d="$art/$name"; mkdir -p "$d"
    ln -sfn "$here/specs" "$d/specs"; ln -sfn "$here/taxonomy" "$d/taxonomy"
    (env "$@" CARGO_TARGET_DIR="$here/target/equiv-build-$name" \
       cargo build --release --locked --bin build_circuit >/dev/null 2>&1 &&
     cd "$d" && "$here/target/equiv-build-$name/release/build_circuit" >/dev/null) || return 1
  }
  same_score() { # dir_a dir_b: score.json equal bar eval_seconds
    python3 - "$1/score.json" "$2/score.json" <<'PY'
import json,sys
def load(p):
    v=json.load(open(p))
    def strip(x):
        if isinstance(x,dict):
            x.pop("eval_seconds",None); [strip(y) for y in x.values()]
        elif isinstance(x,list): [strip(y) for y in x]
    strip(v); return v
sys.exit(0 if load(sys.argv[1])==load(sys.argv[2]) else 1)
PY
  }
  arts=(R-low R-lgr L-gr-a5)
  [[ $quick == 1 ]] && arts=(R-low R-lgr)
  for a in "${arts[@]}"; do
    case $a in
      R-low) build_art $a FEMOCO_WALK_ARCH=sa-low2025 FEMOCO_WALK_SPEC=reiher-sa-est-v1 ;;
      R-lgr) build_art $a FEMOCO_WALK_ARCH=sa-toff FEMOCO_WALK_SPEC=reiher-sa-est-v1 FEMOCO_SA_TWEAKS=imchxlgr ;;
      L-gr-a5) build_art $a FEMOCO_WALK_ARCH=sa-toff FEMOCO_WALK_SPEC=li-sa-est-v1 FEMOCO_SA_TWEAKS=imchxgr FEMOCO_SA_INNER_A=5 ;;
    esac || { echo "  $a: build failed"; ok=0; continue; }
    d="$art/$a"
    # eval_circuit: the default path and --engine/--dump-verdicts must write the same score.json.
    mkdir -p "$d/e1" "$d/e2"
    for x in e1 e2; do for f in ops.bin lanemap.bin family.out.json specs taxonomy; do ln -sfn "$d/$f" "$d/$x/$f"; done; done
    $EVAL --root "$d/e1" --samples "$K" --server-seed "$(printf 'ab%.0s' {1..32})" >"$d/e1.log" 2>&1
    $EVAL --root "$d/e2" --samples "$K" --server-seed "$(printf 'ab%.0s' {1..32})" --engine "$cand" \
      --dump-verdicts "$d/e2/verdicts.bin" >"$d/e2.log" 2>&1
    if same_score "$d/e1" "$d/e2"; then echo "  $a: eval_circuit --engine $cand: score.json equal"
    else echo "  $a: eval_circuit --engine $cand: score.json DIFFERS"; ok=0; fi
    $DIFF --root "$d" --samples "$K" --candidate "$cand" --quiet --json "$log" \
      --seed ordinary,server-label:$a,audit-label:42:$a >"$d/diff.log" 2>&1 || { ok=0; cat "$d/diff.log"; }
    tail -1 "$d/diff.log" | sed "s/^/  $a: /"
    rm -f "$d/ops.bin"
  done
  record binary "$([[ $ok == 1 ]] && echo PASS || echo FAIL)" "${#arts[@]} artifacts"
fi

stage summary
python3 - "$log" "$cand" <<'PY'
import json,os,sys,collections
rows=[json.loads(l) for l in open(sys.argv[1])] if os.path.exists(sys.argv[1]) else []
by=collections.Counter(); bad=collections.Counter(); lanes=0
for r in rows:
    k=r.get("kind","eval_diff")
    if k in ("fuzz","mutant-sweep","guard-band-selftest"): continue
    # Rows of the deliberately wrong self-test engines are expected to differ.
    if str(r.get("engine","")).startswith("selftest-") and r.get("engine")!=sys.argv[2]: continue
    by[k]+=1; bad[k]+= (not r.get("equal",True))
print(f"candidate {sys.argv[2]}: comparisons by kind (unequal in brackets)")
for k in sorted(by): print(f"  {k:10s} {by[k]:7d}  [{bad[k]}]")
fz=[r for r in rows if r.get("kind")=="fuzz"]
if fz: print(f"  fuzz: {sum(r['cases'] for r in fz)} cases, {sum(r['engine_runs'] for r in fz)} engine runs, {sum(r['lanes'] for r in fz)} lanes")
ms=[r for r in rows if r.get("kind")=="mutant-sweep"]
if ms: print(f"  mutant sweeps: {sum(r['mutants'] for r in ms)} mutants over {len(ms)} circuits")
PY
fail=0
for r in ${results[@]+"${results[@]}"}; do
  IFS='|' read -r s v d <<<"$r"
  printf '  %-13s %-5s %s\n' "$s" "$v" "$d"
  [[ $v == PASS ]] || fail=1
done
echo "logs: $out"
exit $fail
