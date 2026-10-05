#!/usr/bin/env bash
# Build, check locally, pack and submit one sos-sa artifact to the local validation server
# (tools/server/README.md). Untrusted tooling: it never touches the repo's results.tsv or score.json.
#
#   tools/server/submit.sh --spec SPEC --arch ARCH [--cell CELL] [--label LABEL] [--no-post]
#                          [FEMOCO_X=value ...]
#
#   --spec     spec id (reiher-sa-est-v1 or li-sa-est-v1).
#   --arch     FEMOCO_WALK_ARCH (sa-toff, sa-pareto, sa-lowq, ...).
#   --cell     the declared cell: "delivery/erasure/outer_slot", a JSON object, a JSON file, or
#              "auto" (default) for tools/server/cell.py's suggestion from the knobs. Cells are
#              DECLARED, not verified: when your code change adds a mechanism the knobs do not
#              show, declare it yourself. A declared cell that differs from the suggestion is
#              kept, with a warning.
#   --label    a short name for the run directory and the submission log (default: a knob hash).
#   --no-post  stop after packing (prints the artifact's sha256, which is its server id).
#   --expect-ops SHA  refuse to submit unless ops.bin has this sha256 (e.g. a pin from
#              tests/sa_digests.rs, to re-submit a measured circuit byte for byte).
#   FEMOCO_*   build-time knobs. ONLY these are set: every other FEMOCO_* variable in the
#              caller's environment is unset for the build, so a stray export cannot leak in.
#
# Steps:
#   1. Build build_circuit from THIS checkout (the challenge directory that holds this script)
#      in its own target dir, $SUBMIT_OUT/cargo, under a lock (cargo tracks option_env!
#      knobs, so a knob change recompiles the walk crate only). The binary is copied out
#      while the lock is held.
#   2. Run it in a scratch dir (macOS: sandbox-exec, writable only there, no network), with
#      FEMOCO_ROOT pointing at this checkout's specs.
#   3. K = 64 local check with the TRUSTED eval_circuit the server runs (default: this
#      checkout's target/release/eval_circuit; override FEMOCO_EVAL), --root a scratch dir
#      (so its results.tsv / score.json land there), RAYON_NUM_THREADS=2. A failing check
#      stops the submission.
#   4. architecture.json from the cell, `femoco_serve pack`, sha256 = the server id.
#   5. POST to $FEMOCO_SERVER/submissions (retries on 503 "screening capacity full"), print the
#      id, append a row to $SUBMIT_OUT/submitted.tsv. Then: tools/server/poll.sh <id>.
#
# Environment: FEMOCO_SERVER (http://127.0.0.1:8753), FEMOCO_EVAL, FEMOCO_SERVE_BIN (pack
# binary; default next to FEMOCO_EVAL), SUBMIT_OUT (default <challenge>/target/submit).
# Every run keeps its directory $SUBMIT_OUT/runs/<label>-<time>/ with the three circuit files,
# submission.fmc, the K = 64 score (eval/score.json) and meta.json (knobs, cell, git head).
set -euo pipefail

here="$(cd "$(dirname "$0")/../.." && pwd -P)"
server="${FEMOCO_SERVER:-http://127.0.0.1:8753}"
eval_bin="${FEMOCO_EVAL:-$here/target/release/eval_circuit}"
serve_bin="${FEMOCO_SERVE_BIN:-$(dirname "$eval_bin")/femoco_serve}"
out_root="${SUBMIT_OUT:-$here/target/submit}"

spec="" arch="" cell="auto" label="" post=1 expect_ops=""
knobs=()
die() { echo "!! submit: $*" >&2; exit 1; }
while [[ $# -gt 0 ]]; do
  case "$1" in
    --spec) spec="$2"; shift 2 ;;
    --arch) arch="$2"; shift 2 ;;
    --cell) cell="$2"; shift 2 ;;
    --label) label="$2"; shift 2 ;;
    --no-post) post=0; shift ;;
    --expect-ops) expect_ops="$2"; shift 2 ;;
    -h|--help) sed -n '2,40p' "$0"; exit 0 ;;
    FEMOCO_*=*)
      k="${1%%=*}"
      [[ "$k" =~ ^FEMOCO_[A-Z0-9_]+$ ]] || die "bad knob name $k"
      [[ "$k" == FEMOCO_WALK_SPEC || "$k" == FEMOCO_WALK_ARCH ]] && die "use --spec / --arch, not $k"
      [[ "$k" == FEMOCO_ROOT ]] && die "FEMOCO_ROOT is set by this script"
      [[ "${1#*=}" =~ ^[A-Za-z0-9._,+~-]*$ ]] || die "knob value of $k has characters outside [A-Za-z0-9._,+~-] (one knob per argument)"
      knobs+=("$1"); shift ;;
    *) die "unknown argument $1 (see --help)" ;;
  esac
done
[[ -n "$spec" && -n "$arch" ]] || die "--spec and --arch are required"
[[ "$spec" =~ ^[a-z0-9-]+$ && -f "$here/specs/$spec/spec.json" ]] || die "unknown spec $spec"
python3 -c "import json,sys; sys.exit(json.load(open(sys.argv[1])).get('encoding')!='sos-sa')" \
  "$here/specs/$spec/spec.json" || die "the server accepts sos-sa specs only"
[[ -x "$eval_bin" ]] || die "trusted eval_circuit not found at $eval_bin (set FEMOCO_EVAL)"
[[ -x "$serve_bin" ]] || die "femoco_serve not found at $serve_bin (set FEMOCO_SERVE_BIN)"

sorted_knobs="$(printf '%s\n' "${knobs[@]+"${knobs[@]}"}" | LC_ALL=C sort | sed '/^$/d')"
key="$(printf '%s|%s|%s' "$arch" "$spec" "$sorted_knobs" | shasum -a 256 | cut -c1-12)"
label="${label:-$key}"
[[ "$label" =~ ^[A-Za-z0-9._+-]+$ ]] || die "label must match [A-Za-z0-9._+-]+"

# The cell.
suggested="$(python3 "$here/tools/server/cell.py" "$arch" "${knobs[@]+"${knobs[@]}"}" 2>/dev/null || true)"
case "$cell" in
  auto) [[ -n "$suggested" ]] || die "no cell rule for arch $arch; pass --cell"; cell="$suggested" ;;
  \{*) cell="$(python3 -c 'import json,sys;a=json.loads(sys.argv[1]);print(f"{a[\"delivery\"]}/{a[\"erasure\"]}/{a[\"outer_slot\"]}")' "$cell")" ;;
  */*/*) ;;
  *) [[ -f "$cell" ]] || die "--cell: not a cell, JSON or file: $cell"
     cell="$(python3 -c 'import json,sys;a=json.load(open(sys.argv[1]));print(f"{a[\"delivery\"]}/{a[\"erasure\"]}/{a[\"outer_slot\"]}")' "$cell")" ;;
esac
IFS=/ read -r delivery erasure outer_slot <<< "$cell"
if [[ -n "$suggested" && "$suggested" != "$cell" ]]; then
  echo "warning: declared cell $cell differs from the knob rule's $suggested (kept as declared)" >&2
fi

run="$out_root/runs/$label-$(date +%Y%m%dT%H%M%S)"
mkdir -p "$run/build" "$run/eval"
run="$(cd "$run" && pwd -P)"
echo "== $label: $arch on $spec, cell $cell"
echo "   knobs: ${sorted_knobs:-(none)}"
echo "   run dir: $run"

# 1. Build under a lock (one cargo target dir per checkout).
lock="$out_root/build.lock"
until mkdir "$lock" 2>/dev/null; do
  holder="$(cat "$lock/pid" 2>/dev/null || true)"
  if [[ -n "$holder" ]] && ! kill -0 "$holder" 2>/dev/null; then rm -rf "$lock"; continue; fi
  sleep 3
done
echo $$ > "$lock/pid"
trap 'rm -rf "$lock"' EXIT
unset_args=()
while IFS= read -r v; do [[ -n "$v" ]] && unset_args+=(-u "$v"); done \
  < <(env | sed -n 's/^\(FEMOCO_[A-Za-z0-9_]*\)=.*/\1/p')
(
  cd "$here"
  env "${unset_args[@]+"${unset_args[@]}"}" "${knobs[@]+"${knobs[@]}"}" \
    FEMOCO_WALK_ARCH="$arch" FEMOCO_WALK_SPEC="$spec" \
    CARGO_TARGET_DIR="$out_root/cargo" CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" CARGO_NET_OFFLINE=true \
    cargo build --release --locked --offline --quiet --bin build_circuit
) || die "cargo build failed"
cp "$out_root/cargo/release/build_circuit" "$run/build_circuit"
rm -rf "$lock"; trap - EXIT

# 2. Run the untrusted builder in its scratch dir.
if [[ "$(uname -s)" == Darwin ]] && command -v sandbox-exec >/dev/null 2>&1; then
  profile="(version 1)(allow default)(deny file-write*)(allow file-write* (subpath \"$run/build\"))(allow file-write* (subpath \"/dev\"))(deny network*)"
  sandbox-exec -p "$profile" /bin/bash -c 'cd "$1" && export TMPDIR="$1" FEMOCO_ROOT="$3" && exec "$2"' \
    _ "$run/build" "$run/build_circuit" "$here" > "$run/build.log" 2>&1 || { cat "$run/build.log" >&2; die "build_circuit failed"; }
else
  (cd "$run/build" && FEMOCO_ROOT="$here" "$run/build_circuit") > "$run/build.log" 2>&1 \
    || { cat "$run/build.log" >&2; die "build_circuit failed"; }
fi
for f in ops.bin lanemap.bin family.out.json; do
  [[ -f "$run/build/$f" && ! -L "$run/build/$f" && -s "$run/build/$f" ]] || die "build_circuit did not write $f"
done
got="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["spec"])' "$run/build/family.out.json")"
[[ "$got" == "$spec" ]] || die "family.out.json says spec $got, not $spec (was FEMOCO_WALK_SPEC tracked?)"

ops_sha="$(shasum -a 256 "$run/build/ops.bin" | cut -d' ' -f1)"
echo "   ops.bin sha256 $ops_sha"
if [[ -n "$expect_ops" && "$ops_sha" != "$expect_ops" ]]; then
  die "ops.bin sha256 $ops_sha is not the expected $expect_ops"
fi

# 3. K = 64 local check with the trusted evaluator (never the repo's results.tsv).
cp "$run/build/ops.bin" "$run/build/lanemap.bin" "$run/build/family.out.json" "$run/eval/"
ln -s "$here/specs" "$run/eval/specs"
ln -s "$here/taxonomy" "$run/eval/taxonomy"
start=$(date +%s)
if ! RAYON_NUM_THREADS=2 "$eval_bin" --root "$run/eval" --samples 64 --note "submit.sh local K=64 $label" \
     > "$run/eval.log" 2>&1; then
  tail -5 "$run/eval.log" >&2
  die "K = 64 local check REJECTED the circuit; not submitted (see $run/eval.log)"
fi
read -r c64 q64 < <(python3 -c 'import json,sys;m=json.load(open(sys.argv[1]))["metrics"];print(m["toffoli"],m["qubits"])' "$run/eval/score.json")
echo "   K=64 local: C_step $c64  Q_peak $q64  ($(( $(date +%s) - start ))s)"

# 4. Pack.
printf '{"delivery":"%s","erasure":"%s","outer_slot":"%s"}\n' "$delivery" "$erasure" "$outer_slot" > "$run/architecture.json"
"$serve_bin" pack "$run/build/ops.bin" "$run/build/lanemap.bin" "$run/build/family.out.json" \
  "$run/architecture.json" "$run/submission.fmc" || die "pack failed"
sha="$(shasum -a 256 "$run/submission.fmc" | cut -d' ' -f1)"
head="$(git -C "$here" rev-parse --short HEAD 2>/dev/null || echo nogit)$([[ -n "$(git -C "$here" status --porcelain -- src/walk 2>/dev/null)" ]] && echo -dirty || true)"
[[ "$head" == *-dirty ]] && echo "warning: src/walk has uncommitted changes; commit before citing this artifact" >&2
python3 - "$run/meta.json" <<PY
import json, sys
json.dump({"label": "$label", "spec": "$spec", "arch": "$arch", "knobs": """$sorted_knobs""".split(),
           "cell": "$cell", "cell_suggested": "$suggested", "git": "$head", "ops_sha256": "$ops_sha",
           "k64": {"toffoli": $c64, "qubits": $q64}, "artifact_sha256": "$sha"},
          open(sys.argv[1], "w"), indent=1)
PY
echo "   artifact sha256 $sha"
if [[ "$post" -eq 0 ]]; then echo "   --no-post: not submitted ($run/submission.fmc)"; exit 0; fi

# 5. Submit.
for attempt in $(seq 1 40); do
  code="$(curl -s -o "$run/post.json" -w '%{http_code}' --data-binary @"$run/submission.fmc" "$server/submissions" || echo 000)"
  [[ "$code" != 503 ]] && break
  echo "   server busy (503), retry $attempt in 15s" >&2; sleep 15
done
if [[ "$code" != 202 ]]; then
  echo "!! submit: server answered $code: $(cat "$run/post.json" 2>/dev/null)" >&2
  exit 1
fi
state="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["state"])' "$run/post.json")"
mkdir -p "$out_root"
[[ -f "$out_root/submitted.tsv" ]] || printf 'time\tlabel\tspec\tarch\tknobs\tcell\tgit\tk64_C\tk64_Q\tid\n' > "$out_root/submitted.tsv"
printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$(date -u +%FT%TZ)" "$label" "$spec" "$arch" \
  "$(echo $sorted_knobs)" "$cell" "$head" "$c64" "$q64" "$sha" >> "$out_root/submitted.tsv"
echo "   submitted: state $state"
echo "id $sha"
