#!/usr/bin/env bash
# Stress lane ops-admission: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

set -euo pipefail
{
  uname -a
  rustc --version
  git rev-parse HEAD
  for iteration in $(seq 1 20); do
    printf '\nITERATION %s/20\n' "$iteration"
    cargo nextest run -p ovis-terminal-server --test ops_admission --locked --no-tests=fail --no-capture
    cargo nextest run -p ovis-terminal-core --test events --test ops --locked --no-tests=fail -E 'test(/admission::|fairness::/)'
    printf 'PASS %s/20\n' "$iteration"
  done
  printf 'RESULT 20/20\n'
} 2>&1 | tee ops-admission-stress-macos.log
