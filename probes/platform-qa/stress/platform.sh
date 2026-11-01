#!/usr/bin/env bash
# Stress lane platform: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

set -euo pipefail
{
  uname -a
  rustc --version
  git rev-parse HEAD
  for iteration in $(seq 1 100); do
    printf '\nITERATION %s/100\n' "$iteration"
    if test "$STRESS_VARIANT" = happy; then
      cargo nextest run -p ovis-terminal-holder --test platform_probe happy --locked
    else
      cargo nextest run -p ovis-terminal-holder --test platform_probe --locked
    fi
    printf 'PASS %s/100\n' "$iteration"
  done
  printf 'RESULT 100/100\n'
} 2>&1 | tee probe-stress-macos.log
