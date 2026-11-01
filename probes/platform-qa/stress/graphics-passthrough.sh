#!/usr/bin/env bash
# Stress lane graphics-passthrough (plan todo 83): run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail
{
  uname -a
  rustc --version
  git rev-parse HEAD
  cargo build --locked -p ovis-terminal-cli -p ovis-terminal-conformance
  for iteration in $(seq 1 25); do
    printf '\nITERATION %s/25\n' "$iteration"
    cargo nextest run -p ovis-terminal-server --locked --no-tests=fail graphics
    target/debug/conformance --binary target/debug/ovis-terminal --scenario graphics/passthrough --isolated --case all --evidence "graphics-passthrough-$iteration.json"
    printf 'PASS %s/25\n' "$iteration"
  done
  printf 'RESULT 25/25\n'
} 2>&1 | tee graphics-passthrough-stress-macos.log
