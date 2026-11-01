#!/usr/bin/env bash
# F2a loaded proof: 25 runs of the whole conformance crate (its all/all suite test is where the
# three occurrences surfaced), pinned to cores 120-123 with 8 busy loops there at nice 10
# (task-31 r5 recipe; the product at nice 0 keeps its share). One crate: run directly, outside
# the heavy slot gate (owner instruction), inside the toolchain env's cargo shim.
set -u
LOADED=120-123
SCR=/dev/shm/ovt-fixtures/f2a-scratch
cargo nextest run --locked --manifest-path conformance/Cargo.toml --no-run >/dev/null 2>&1 || { echo BUILD_FAILED; exit 2; }
pids=()
for _ in 1 2 3 4 5 6 7 8; do nice -n 10 taskset -c $LOADED bash -c 'while :; do :; done' & pids+=("$!"); done
cleanup() { for p in "${pids[@]}"; do if [ "$p" -gt 1 ]; then kill "$p" 2>/dev/null; fi; done; }
trap cleanup EXIT
pass=0; fail=0
for i in $(seq 1 ${RUNS:-25}); do
  load=$(cut -d' ' -f1-3 /proc/loadavg); start=$(date +%s)
  if taskset -c $LOADED cargo nextest run --locked --manifest-path conformance/Cargo.toml --no-fail-fast > $SCR/n-$i.log 2>&1; then r=ok; pass=$((pass+1)); else r=FAIL; fail=$((fail+1)); fi
  secs=$(( $(date +%s) - start )); echo "run $i $r secs=$secs load=[$load] $(grep -E '^ +Summary' $SCR/n-$i.log | tr -s ' ')"
done
echo "LOADED_RUNS pass=$pass fail=$fail"
