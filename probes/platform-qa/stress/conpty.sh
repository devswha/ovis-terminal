#!/usr/bin/env bash
# Stress lane conpty: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

set -euo pipefail
{
  uname -a
  rustc --version
  git rev-parse HEAD
  for iteration in $(seq 1 10); do
    printf '\nITERATION %s/10\n' "$iteration"
    cargo test --locked -p ovis-terminal-holder --lib windows:: -- --nocapture
    cargo test --locked -p ovis-terminal-server --lib conpty
  done
  printf 'RESULT pass\n'
} 2>&1 | tee conpty-windows.log
