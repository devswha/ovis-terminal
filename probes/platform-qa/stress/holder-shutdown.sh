#!/usr/bin/env bash
# Stress lane holder-shutdown: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

set -euo pipefail
{
  uname -a
  rustc --version
  git rev-parse HEAD
  cargo nextest run -p ovis-terminal-holder --locked --lib --no-tests=fail
  for iteration in $(seq 1 200); do
    printf '\nITERATION %s/200\n' "$iteration"
    cargo nextest run -p ovis-terminal-holder --locked --lib \
      -E 'test(newer_fence_revokes_queued_input_behind_a_blocked_write) | test(/shutdown_tests::/)' \
      --no-tests=fail --success-output immediate
    printf 'PASS %s/200\n' "$iteration"
  done
  printf 'RESULT 200/200\n'
} 2>&1 | tee macos-stress.log
