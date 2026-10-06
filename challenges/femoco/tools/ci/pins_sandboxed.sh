#!/usr/bin/env bash
# Run the pin test (tests/sa_digests.rs) with the working tree's walk code, confined like the
# circuit build: read-only filesystem, no network, unprivileged uid, one writable scratch dir.
#
# The judge uses this on a pull request's code: every recorded circuit must still build byte for
# byte. Compilation happens outside the sandbox (it executes none of the walk code); only the
# test binary, which does, runs inside it. Without bubblewrap (local development) it runs
# unconfined with a warning.
#
#   tools/ci/pins_sandboxed.sh [TEST ARGS...]                 every pin, or the tests the arguments select
#   PIN_EXPORT_DIR=DIR tools/ci/pins_sandboxed.sh --exact P   also write pin P's circuit to DIR/P/
#
# With PIN_EXPORT_DIR the directory is the sandbox's only other writable path. Whatever lands
# there was written by walk code: read it as data and check its digests before use.
set -euo pipefail
cd "$(dirname "$0")/../.."

exe="$(cargo test --release --locked --test sa_digests --no-run --message-format=json \
  | python3 -c '
import json, sys
for line in sys.stdin:
    try:
        m = json.loads(line)
    except ValueError:
        continue
    if m.get("reason") == "compiler-artifact" and m.get("target", {}).get("name") == "sa_digests" and m.get("executable"):
        print(m["executable"])
')"
[[ -x "${exe}" ]] || { echo "!! pin test binary not found" >&2; exit 1; }

scratch="$(cd "$(mktemp -d)" && pwd -P)"
chmod 0777 "${scratch}"
trap 'rm -rf "${scratch}" 2>/dev/null || sudo -n rm -rf "${scratch}" 2>/dev/null || true' EXIT
threads="${PIN_TEST_THREADS:-4}"
export_args=( --unsetenv FEMOCO_PIN_EXPORT_DIR )
unset FEMOCO_PIN_EXPORT_DIR
if [[ -n "${PIN_EXPORT_DIR:-}" ]]; then
  mkdir -p "${PIN_EXPORT_DIR}"
  export_dir="$(cd "${PIN_EXPORT_DIR}" && pwd -P)"
  chmod 0777 "${export_dir}"
  export_args=( --bind "${export_dir}" "${export_dir}" --setenv FEMOCO_PIN_EXPORT_DIR "${export_dir}" )
  export FEMOCO_PIN_EXPORT_DIR="${export_dir}"
fi

if command -v bwrap >/dev/null 2>&1; then
  bw=( bwrap )
  if [[ "$(id -u)" -ne 0 ]] && command -v sudo >/dev/null 2>&1 && sudo -n true >/dev/null 2>&1; then
    bw=( sudo -n bwrap )
  elif command -v setpriv >/dev/null 2>&1; then
    bw=( setpriv --no-new-privs bwrap )
  fi
  "${bw[@]}" \
    --ro-bind / / --dev /dev --proc /proc \
    --bind "${scratch}" "${scratch}" --chdir "$(pwd -P)" \
    --setenv TMPDIR "${scratch}" "${export_args[@]}" \
    --unshare-user --unshare-pid --unshare-net --unshare-ipc --unshare-uts --unshare-cgroup \
    --cap-drop ALL --new-session --die-with-parent \
    --uid 65534 --gid 65534 \
    -- "${exe}" --test-threads="${threads}" "$@"
else
  echo "!! bubblewrap not found; running the pin test UNCONFINED (local development only)" >&2
  TMPDIR="${scratch}" "${exe}" --test-threads="${threads}" "$@"
fi
