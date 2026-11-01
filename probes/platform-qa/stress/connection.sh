#!/usr/bin/env bash
# Stress lane connection: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

set -euo pipefail
{
  uname -a
  rustc --version
  git rev-parse HEAD
  for iteration in $(seq 1 200); do
    printf '\nITERATION %s/200\n' "$iteration"
    cargo nextest run -p ovis-terminal-server --test connection --locked --no-tests=fail
    printf 'PASS %s/200\n' "$iteration"
  done
  printf 'RESULT 200/200\n'
} 2>&1 | tee connection-stress-macos.log
