#!/usr/bin/env bash
# Stress lane ops-digest: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

set -euo pipefail
{
  uname -a
  rustc --version
  git rev-parse HEAD
  for iteration in $(seq 1 25); do
    printf '\nITERATION %s/25\n' "$iteration"
    cargo nextest run -p ovis-terminal-server --test ops_digest --test documents_retention --locked --no-tests=fail
    cargo nextest run -p ovis-terminal-core --locked --no-tests=fail -E 'test(/digest|params/)'
    printf 'PASS %s/25\n' "$iteration"
  done
  printf 'RESULT 25/25\n'
} 2>&1 | tee ops-digest-stress-macos.log
