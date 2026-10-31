#!/usr/bin/env bash
# Stress loop for plan todo 80: 200 sequential nextest runs of the connection suite
# under 8 busy loops. Only the busy-loop PIDs this script started are signalled.
set -u
cd "$1"
pids=()
cleanup() { for p in "${pids[@]}"; do if [ "$p" -gt 1 ]; then kill "$p"; fi; done; }
trap cleanup EXIT
for i in $(seq 8); do (while :; do :; done) & pids+=("$!"); done
for p in "${pids[@]}"; do [ "$p" -gt 1 ] || { echo "refusing pid $p"; exit 3; }; done
echo "busy loops: ${pids[*]} (nproc $(nproc)); tree $(git rev-parse HEAD) $(git status --porcelain | tr '\n' ' ')"
for i in $(seq 200); do
  echo "=== run $i"
  cargo nextest run -p ovis-terminal-server --test connection --locked 2>&1 || { echo "FAILED at run $i"; exit 1; }
done
echo "PASSED 200/200"
