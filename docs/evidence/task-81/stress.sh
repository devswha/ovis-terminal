#!/usr/bin/env bash
# usage: stress.sh RUNS LOG [STOP_ON_FIRST_FAIL=0]
# Runs the subscriptions suite RUNS times pinned to CPUs 0-1 beside 8 busy loops on the same CPUs.
set -u
runs=$1 log=$2 stop=${3:-0}
cd "$(git rev-parse --show-toplevel)"
cargo nextest run -p ovis-terminal-core --test subscriptions --locked --no-run >/dev/null 2>&1 || { echo "build failed" >"$log"; exit 2; }
busy=()
for _ in 1 2 3 4 5 6 7 8; do taskset -c 0-1 sh -c 'while :; do :; done' & busy+=($!); done
trap 'for p in "${busy[@]}"; do [ "$p" -gt 1 ] && kill "$p"; done' EXIT
{
  echo "# $(date -u +%FT%TZ) head=$(git rev-parse --short HEAD) dirty=$(git status --porcelain crates | wc -l) runs=$runs"
  echo "# load: 8x 'taskset -c 0-1 sh -c while :; do :; done'; suite: taskset -c 0-1 cargo nextest run -p ovis-terminal-core --test subscriptions --locked"
} >"$log"
pass=0 fail=0
for i in $(seq 1 "$runs"); do
  out=$(taskset -c 0-1 cargo nextest run -p ovis-terminal-core --test subscriptions --locked --no-fail-fast 2>&1); rc=$?
  if [ $rc -eq 0 ]; then pass=$((pass+1)); echo "run $i: exit 0 $(grep -o '[0-9]* tests run: [0-9]* passed[^,]*' <<<"$out" | tail -1)" >>"$log"
  else fail=$((fail+1)); { echo "run $i: exit $rc"; grep -E 'FAIL|PASS \[|panicked|Error:|assert|left|right|SIGSEGV|TIMEOUT' <<<"$out" | grep -v '^ *PASS' | head -60; } >>"$log"
       [ "$stop" = 1 ] && break; fi
done
echo "# result: $pass passed, $fail failed of $((pass+fail)) runs" >>"$log"
