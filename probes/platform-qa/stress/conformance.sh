#!/usr/bin/env bash
# Stress lane conformance: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

set -euo pipefail
{
  uname -a
  rustc --version
  git rev-parse HEAD
  cargo build --locked -p ovis-terminal-cli --bin ovis-terminal
  for iteration in $(seq 1 25); do
    printf '\nITERATION %s/25\n' "$iteration"
    cargo nextest run -p ovis-terminal-conformance --all-targets --locked --no-tests=fail
    printf 'PASS %s/25\n' "$iteration"
  done
  printf 'RESULT 25/25\n'
} 2>&1 | tee conformance-stress-macos.log
