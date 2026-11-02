#!/usr/bin/env bash
set -uo pipefail
W=/home/devswha/workspace/ovis-terminal-wt/lane-tF2b
cd "$W"; source /home/devswha/.ovis-toolchain/env.sh
unset CARGO_PROFILE_DEV_DEBUG CARGO_PROFILE_TEST_DEBUG
G="bash $W/scripts/ci/target-footprint.sh"
run_case() { # name, cargo-env..., then the build is fixed
  local name="$1"; shift; local t=/dev/shm/ovt-target-lane-tF2b-$name; rm -rf "$t"
  echo "=== $name: $*"
  env "$@" CARGO_TARGET_DIR="$t" cargo nextest run --locked -p ovis-terminal-bench --all-targets --all-features --no-run >"$t.log" 2>&1 || { echo "build failed"; tail -5 "$t.log"; }
  env "$@" CARGO_TARGET_DIR="$t" cargo build --locked -p ovis-terminal-cli >>"$t.log" 2>&1 || { echo "cli build failed"; tail -5 "$t.log"; }
  echo "cli bin bytes: $(stat -c %s "$t/debug/ovis-terminal")"
  GITHUB_STEP_SUMMARY=/dev/null $G "$t" 2>&1 | grep -E 'rule=|largest|debug/deps:|^/dev'; echo "guard_exit=${PIPESTATUS[0]}"
  rm -rf "$t" "$t.log"
}
run_case M1_profile_default_full_debug
run_case AFTER_both_line_tables CARGO_PROFILE_DEV_DEBUG=line-tables-only CARGO_PROFILE_TEST_DEBUG=line-tables-only
run_case M2_dev_var_removed CARGO_PROFILE_TEST_DEBUG=line-tables-only
cd /dev/shm/ovt-fixtures/lane-tF2b-scratch/bt
for lvl in line-tables-only none; do
  t=/dev/shm/ovt-target-lane-tF2b-bt-$lvl; rm -rf "$t"
  echo "=== backtrace debug=$lvl"
  RUST_BACKTRACE=1 CARGO_PROFILE_DEV_DEBUG=$lvl CARGO_PROFILE_TEST_DEBUG=$lvl CARGO_TARGET_DIR="$t" cargo test --offline 2>&1 | grep -E "panicked at|bt::inner|src/lib.rs|test result" ; echo "file_line_frames=$(RUST_BACKTRACE=1 CARGO_PROFILE_DEV_DEBUG=$lvl CARGO_PROFILE_TEST_DEBUG=$lvl CARGO_TARGET_DIR="$t" cargo test --offline 2>&1 | grep -cE '^ +at .*src/lib.rs:[0-9]+')"
  rm -rf "$t"
done
echo MUTANTS_DONE
