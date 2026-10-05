#!/usr/bin/env bash
# Reproduce pinned circuits of the two tracks (li-sa-est-v1, reiher-sa-est-v1; estimated rounding
# class): the best validated circuit per qubit budget and the C x Q bests at the time of writing.
# Every line is pinned in tests/sa_circuits/list.rs (checked by tests/sa_digests.rs) under the same
# name; the build must reproduce the pinned ops SHA-256.
#
# Each line: name | spec | FEMOCO_SA_TWEAKS | keep bits (FEMOCO_SA_MU_O = _MU_I) | FEMOCO_SA_INNER_A |
# FEMOCO_SA_OUTER_A | ops SHA-256. The architecture is sa-toff throughout.
#
# Steps per line: tools/server/submit.sh --no-post --expect-ops SHA (build from this checkout in a
# sandbox, trusted eval_circuit at K = 64, refuses a different ops.bin), then, if FAST names a
# trusted evaluator binary (cargo build --release --locked --no-default-features --bin
# eval_circuit), the full 524,288-lane run `FAST --engine sliced --samples 524288` through
# tools/heavy.sh into a scratch root. Nothing is written to this directory's results.tsv or
# score.json. The trusted binaries (eval_circuit, femoco_serve) must be built first; see
# tools/server/README.md.
#
#   tools/repro/reproduce_front.sh                         # every line, K = 64 only
#   FAST=target/release/eval_circuit tools/repro/reproduce_front.sh fa_r1tr_m9   # one, with full K
#
# Heavy: Li ~1 min build + ~40 s full K, Reiher ~30 s + ~8 s, at 4 threads; one at a time.
set -u
cd "$(dirname "$0")/../.."
export RAYON_NUM_THREADS="${RAYON_NUM_THREADS:-4}" CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
SCRATCH="${SCRATCH:-$(mktemp -d)}"
list='
fa_l3q618i9_m9  | li-sa-est-v1     | imchxgrdky3zabCAXJBMnFsUtq618_1      | 9 | 9 | 3 | 9f83af58701bf10125355ea5f32bac8f6e583b0b1c20e715db822603bb18e941
fa_l3q618i9_m8  | li-sa-est-v1     | imchxgrdky3zabCAXJBMnFsUtq618_1      | 8 | 9 | 3 | 9fb8d2bb535664dc2abcc20f4f5b55042a0ad99960842b3023cfeaa8be88d90b
fa_l4tq23_m9    | li-sa-est-v1     | imchxgrdky4zabCAXZJBMnFsYUtq23_1     | 9 | 8 | 3 | 2d33e9da97622bff52c9408e902944e1e45fe89718830801d1284423bc195eb1
fa_l4tq26_m8    | li-sa-est-v1     | imchxgrdky4zabCAXZJBMnFsYUtq26_1     | 8 | 8 | 3 | ad0926ddfa88289d62b3dc09afad6d6f31762001e4640c63f74dd2cf5eb9ea19
fa_l4tpa7_m8    | li-sa-est-v1     | imchxgrdky4zabCAXZJtRNOBWMenFsY      | 8 | 7 | 3 | 2f375ef36d827ee5a3728f31a6223300cc6c77c43daa6d516510cbec773a7930
fa_l5pq27_m9    | li-sa-est-v1     | imchxgrdky5zabCHVIXJtRNOBWs+q27_1    | 9 | 3 | 2 | d631492aeb4432c1533857bdd60b817b83e5c000baf00b75e0e28602581ea4f0
fa_l6tpq28_m9   | li-sa-est-v1     | imchxgrdky6zabCHVIXZJsot+q28_1       | 9 | 3 | 2 | e7eeacc4bafe92eae33db1a33de99d45b9ca3dd29902beaf50b394522be130cc
fa_l6tpq14_m8   | li-sa-est-v1     | imchxgrdky6zabCHVIXZJsot+q14_1       | 8 | 3 | 2 | 7b5abba501c226fdab2dac6af98fa2af211f5ee4b05432812a9727f812f020a2
pf_l4tp_m9      | li-sa-est-v1     | imchxgrdky4zabCAXZJtRNOBWMenFsY      | 9 | 8 | 3 | 2b849df2c3837fd103208343901604eaba0c39d6aa0911af2f3e2902b58ac008
pf_l4tp_m8      | li-sa-est-v1     | imchxgrdky4zabCAXZJtRNOBWMenFsY      | 8 | 8 | 3 | 2a9d5c0c2f39e97248f982d47af94b4eb310c056ec8c58e017e029f1f7b7bf6b
fa_r1tr_m9      | reiher-sa-est-v1 | imchxgrdkyECHKVIXtRNOBW+             | 9 | 3 | 3 | fdbdf8dc7b62253999d95893b318e36affe781c08cbff2df2b4746e201b1dc00
fa_r1tr_m8      | reiher-sa-est-v1 | imchxgrdkyECHKVIXtRNOBW+             | 8 | 3 | 3 | 82244d6c2e20b8461b3a2e478617b21613149c364ad623c88c19eca18492f40f
pf_r1c_m9       | reiher-sa-est-v1 | imchxlgrdkyECHKVIXt+                 | 9 | 3 | 3 | 250492f0083a8460544a1982f80ac8fc2b56c23b1661dc1206d410e91bfa5f3a
pf_r1c_m8       | reiher-sa-est-v1 | imchxlgrdkyECHKVIXt+                 | 8 | 3 | 3 | c83041d7324ac0d5423d5a0e627636171659f0ee963e674596ce7ad88aa2b895
fa_r3pq150_m9   | reiher-sa-est-v1 | imchxgrdky3zabCHKVIDXtNBWs+q150_1    | 9 | 3 | 3 | bea46c39232c28f1a33e7362d01444fb6c57dcdc5b91d98b21b4392e954f745f
fa_r3pq153_m8   | reiher-sa-est-v1 | imchxgrdky3zabCHKVIDXtNBWs+q153_1    | 8 | 2 | 2 | b4b93bef810117f8e1fd3d62d0bd27597c9b6ddc6d0acadd283d8c4666abe91c
pf_r3p_m9       | reiher-sa-est-v1 | imchxgrdky3zabCHKVIDXtNBWs+          | 9 | 2 | 2 | db93b462938a630a2507121f9997b086f7494de6d8faeafbbbb278cf314fb361
pf_r3p_m8       | reiher-sa-est-v1 | imchxgrdky3zabCHKVIDXtNBWs+          | 8 | 2 | 2 | c61759501b0dbe0ec2685386238946d21b2bd1e97e2597a768860a7c30b23868
fa_r4s_m8       | reiher-sa-est-v1 | imchxgrdky4zZabCHKVITXSRNOBWs        | 8 | 2 | 2 | 019f58088d4a082f8079ec009c1178fc27868cd448aa3cae341039bed7eb6157
fa_r7s_m8       | reiher-sa-est-v1 | imchxgrdky7zabfHKVITXSRNOBWUs        | 8 | 2 | 1 | a0ace9f19aa86b2ab655ccd4d41baa32e09c1332e9165ab90802d798a642eb39
fa_r7s_m9       | reiher-sa-est-v1 | imchxgrdky7zabfHKVITXSRNOBWUs        | 9 | 2 | 1 | d0221c401b768f2b43e9fd78a2d8e77737448168a22d7e6922f2a7cc3eefc8f0
'
want=" $* "
while IFS='|' read -r name spec tw mu ia oa sha; do
  name="$(echo "$name" | xargs)"
  [[ -z "$name" ]] && continue
  if [[ $# -gt 0 && "$want" != *" $name "* ]]; then continue; fi
  spec="$(echo "$spec" | xargs)"; tw="$(echo "$tw" | xargs)"; mu="$(echo "$mu" | xargs)"
  ia="$(echo "$ia" | xargs)"; oa="$(echo "$oa" | xargs)"; sha="$(echo "$sha" | xargs)"
  out=$(tools/server/submit.sh --spec "$spec" --arch sa-toff --label "$name" --no-post --expect-ops "$sha" \
    FEMOCO_SA_TWEAKS="$tw" FEMOCO_SA_MU_O="$mu" FEMOCO_SA_MU_I="$mu" \
    FEMOCO_SA_INNER_A="$ia" FEMOCO_SA_OUTER_A="$oa" 2>&1)
  run=$(echo "$out" | sed -n 's/.*run dir: //p')
  k64=$(echo "$out" | grep -E "K=64" | head -1 | xargs)
  if [[ -z "$run" || ! -f "$run/build/ops.bin" ]]; then
    echo "FAIL $name (build or K = 64): $(echo "$out" | grep -E '!!|error|refuse' | head -1)"; continue
  fi
  if [[ -n "${FAST:-}" ]]; then
    fk="$SCRATCH/$name"; rm -rf "$fk"; mkdir -p "$fk"
    cp "$run/build/ops.bin" "$run/build/lanemap.bin" "$run/build/family.out.json" "$fk/"
    ln -s "$PWD/specs" "$fk/specs"; ln -s "$PWD/taxonomy" "$fk/taxonomy"
    tools/heavy.sh "$FAST" --engine sliced --root "$fk" --samples 524288 --note "full-K $name" > "$fk/eval.log" 2>&1
    res=$(python3 -c "import json,sys;m=json.load(open(sys.argv[1]))['metrics'];print(f\"C {m['toffoli']:,.3f}  Q {m['qubits']}\")" "$fk/score.json" 2>/dev/null)
    ok=$(grep -c "eval OK" "$fk/eval.log")
    echo "$([[ $ok -gt 0 ]] && echo OK || echo FAIL) $name  $k64  full-K $res  $(grep -oE 'fallback_batches=[0-9]+|guard_fallbacks=[0-9]+' "$fk/eval.log" | xargs)"
    rm -f "$fk/ops.bin"
  else
    echo "OK   $name  $k64  ops $sha"
  fi
  rm -f "$run/build/ops.bin" "$run/eval/ops.bin" "$run/submission.fmc"
done <<< "$list"
echo DONE
