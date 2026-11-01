#!/usr/bin/env bash
# 25 runs: the server and CLI under test (and their children) pinned to the loaded cores
# 124-125 with 8 busy loops there at nice 10 (the product at nice 0 keeps a share); nextest and the test binaries on cores 100-123.
set -u
LOADED=124-125; HARNESS=100-123
cargo nextest run -p ovis-terminal-cli --locked --no-run >/dev/null 2>&1 || { echo BUILD_FAILED; exit 2; }
pids=()
for _ in 1 2 3 4 5 6 7 8; do nice -n 10 taskset -c $LOADED bash -c 'while :; do :; done' & pids+=("$!"); done
cleanup() { for p in "${pids[@]}"; do if [ "$p" -gt 1 ]; then kill "$p" 2>/dev/null; fi; done; }
trap cleanup EXIT
pass=0; fail=0
for i in $(seq 1 ${RUNS:-25}); do
  load=$(cut -d' ' -f1-3 /proc/loadavg); start=$(date +%s)
  if OVIS_TEST_PIN_CORES=$LOADED taskset -c $HARNESS cargo nextest run -p ovis-terminal-cli --locked --no-fail-fast --success-output final > /dev/shm/ovt-fixtures/t31-scratch/r5-n-$i.log 2>&1; then r=ok; pass=$((pass+1)); else r=FAIL; fail=$((fail+1)); fi
  secs=$(( $(date +%s) - start )); echo "run $i $r secs=$secs load=[$load]"
  if [ "${STOP:-0}" = 1 ] && { [ $r = FAIL ] || [ $secs -gt 40 ]; }; then break; fi
done
echo "LOADED_RUNS pass=$pass fail=$fail"
