#!/usr/bin/env bash
# Stress lane attach-input: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail
{
  uname -a
  rustc --version
  git rev-parse HEAD
  cargo build --locked -p ovis-terminal-cli --bin ovis-terminal
  cargo build --locked --manifest-path conformance/Cargo.toml --bin conformance
  for iteration in $(seq 1 25); do
    printf '\nITERATION %s/25\n' "$iteration"
    for case in happy failure; do
      target/debug/conformance --binary target/debug/ovis-terminal --scenario part-a/task-30 \
        --isolated --case "$case" --evidence "attach-input-$case-$iteration.json"
    done
    target/debug/conformance --binary target/debug/ovis-terminal --scenario capacity/fifty \
      --isolated --case happy --evidence "attach-input-fifty-$iteration.json"
    printf 'PASS %s/25\n' "$iteration"
  done
  printf 'RESULT 25/25\n'
} 2>&1 | tee attach-input-stress-macos.log
