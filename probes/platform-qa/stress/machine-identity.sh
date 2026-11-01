#!/usr/bin/env bash
# Stress lane machine-identity: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

set -euo pipefail
{
  uname -a
  rustc --version
  git rev-parse HEAD
  for iteration in $(seq 1 100); do
    printf '\nITERATION %s/100\n' "$iteration"
    cargo nextest run -p ovis-terminal-remote --lib --locked --no-tests=fail -E 'test(/identity|transport::/)'
    cargo nextest run -p ovis-terminal-server --lib --locked --no-tests=fail --no-capture -E 'test(/machine_identity::qa/)'
    printf 'PASS %s/100\n' "$iteration"
  done
  printf 'RESULT 100/100\n'
} 2>&1 | tee machine-identity-stress.log
