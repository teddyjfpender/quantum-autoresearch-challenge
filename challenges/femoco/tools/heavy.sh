#!/usr/bin/env bash
# Run a heavy command under a machine-wide lock: at most one heavy job at a time.
# Usage (from the challenge directory): tools/heavy.sh <command...>
# The lock is /tmp/femoco-heavy.lock, shared with any other copy of this script on the machine.
# RAYON_NUM_THREADS and CARGO_BUILD_JOBS default to 4; set them to override.
LOCK=/tmp/femoco-heavy.lock
until mkdir $LOCK 2>/dev/null; do
  holder=$(cat $LOCK/pid 2>/dev/null)
  if [[ -n $holder ]] && ! kill -0 $holder 2>/dev/null; then rm -rf $LOCK; continue; fi
  sleep 10
done
echo $$ > $LOCK/pid
trap 'rm -rf $LOCK' EXIT INT TERM
export RAYON_NUM_THREADS=${RAYON_NUM_THREADS:-4} CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
"$@"
