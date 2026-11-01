#!/usr/bin/env bash
# Stress lane waits: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

set -euo pipefail
{
  uname -a
  rustc --version
  git rev-parse HEAD
  for iteration in $(seq 1 50); do
    printf '\nITERATION %s/50\n' "$iteration"
    cargo nextest run -p ovis-terminal-core --test waits --locked --no-tests=fail --no-capture
    printf 'PASS %s/50\n' "$iteration"
  done
  printf 'RESULT 50/50\n'
} 2>&1 | tee waits-stress-macos.log
