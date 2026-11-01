#!/usr/bin/env bash
# Stress lane restart: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

set -euo pipefail
{
  uname -a
  rustc --version
  git rev-parse HEAD
  for iteration in $(seq 1 25); do
    printf '\nITERATION %s/25\n' "$iteration"
    cargo nextest run -p ovis-terminal-core --lib --locked --no-tests=fail -E 'test(restart::)'
    cargo nextest run -p ovis-terminal-server --lib --locked --no-tests=fail -E 'test(lanes::restart)'
    cargo nextest run -p ovis-terminal-protocol --lib --locked --no-tests=fail -E 'test(domain::restart)'
    printf 'PASS %s/25\n' "$iteration"
  done
  printf 'RESULT 25/25\n'
} 2>&1 | tee restart-stress-macos.log
