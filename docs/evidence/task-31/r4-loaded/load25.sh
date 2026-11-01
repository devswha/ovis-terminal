#!/usr/bin/env bash
# 25 runs of the planned-restart tests pinned to two cores under 8 busy loops.
set -u
CORES=${CORES:-124-125}
echo "cores=$CORES"
cargo nextest run -p ovis-terminal-cli --locked --test assembly --no-run >/dev/null 2>&1 || { echo BUILD_FAILED; exit 2; }
pids=()
for _ in 1 2 3 4 5 6 7 8; do taskset -c $CORES bash -c 'while :; do :; done' & pids+=("$!"); done
cleanup() { for p in "${pids[@]}"; do if [ "$p" -gt 1 ]; then kill "$p" 2>/dev/null; fi; done; }
trap cleanup EXIT
pass=0; fail=0
for i in $(seq 1 25); do
  if taskset -c $CORES cargo nextest run -p ovis-terminal-cli --locked --test assembly -E 'test(/^restart::a_planned_restart|^update::/)' --no-fail-fast > "$AD/load-$i.log" 2>&1; then
    pass=$((pass+1))
  else
    fail=$((fail+1)); grep -E 'FAIL|panicked' "$AD/load-$i.log" | head -3
  fi
done
echo "LOADED_RUNS pass=$pass fail=$fail"
