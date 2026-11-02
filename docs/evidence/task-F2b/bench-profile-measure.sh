#!/usr/bin/env bash
# Measure the bench crate's --all-targets build footprint under each debuginfo setting.
set -uo pipefail
cd /home/devswha/workspace/ovis-terminal-wt/lane-tF2b
source /home/devswha/.ovis-toolchain/env.sh
unset CARGO_PROFILE_DEV_DEBUG CARGO_PROFILE_TEST_DEBUG
measure() {
  local name="$1"; shift
  local t=/dev/shm/ovt-target-lane-tF2b-$name
  rm -rf "$t"
  echo "=== $name: $*"
  env "$@" CARGO_TARGET_DIR="$t" cargo nextest run --locked -p ovis-terminal-bench --all-targets --all-features --no-run >/dev/null 2>"$t.err"; rc=$?; echo "exit=$rc"; [ $rc -ne 0 ] && tail -15 "$t.err"
  echo "target_bytes=$(du -sb "$t" | cut -f1)"
  echo "debug_deps_bytes=$(du -sb "$t/debug/deps" | cut -f1)"
  for b in terminal50 soak; do [ -f "$t/debug/$b" ] && echo "bin $b $(stat -c %s "$t/debug/$b")"; done
  ls -S "$t"/debug/deps | head -3 | while read f; do echo "largest deps/$f $(stat -c %s "$t/debug/deps/$f")"; done
  rm -rf "$t" "$t.err"
}
measure before_full_debug CARGO_PROFILE_DEV_DEBUG=true
measure after_both_lto CARGO_PROFILE_DEV_DEBUG=line-tables-only CARGO_PROFILE_TEST_DEBUG=line-tables-only
measure mutant_dev_only CARGO_PROFILE_DEV_DEBUG=line-tables-only
measure mutant_test_only CARGO_PROFILE_TEST_DEBUG=line-tables-only
