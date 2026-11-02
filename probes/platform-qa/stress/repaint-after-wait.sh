#!/usr/bin/env bash
# Stress lane repaint-after-wait (plan lane P6): run by .github/workflows/probe-stress-macos.yml.
# Evidence is written as *.evidence.log because the workflow uploads only ./*.log.
set -euo pipefail
{
  uname -a
  rustc --version
  git rev-parse HEAD
  cargo build --locked -p ovis-terminal-cli -p ovis-terminal-conformance
  cargo nextest run -p ovis-terminal-cli --lib --locked --no-tests=fail -E 'test(commit::)'
  for iteration in $(seq 1 25); do
    printf '\nITERATION %s/25\n' "$iteration"
    for scenario in repaint-after-wait adversarial; do
      evidence="$scenario-$iteration.evidence.log"
      if ! target/debug/conformance --binary target/debug/ovis-terminal --scenario "attach-input/$scenario" --isolated --case all --evidence "$evidence"; then
        cat "$evidence"
        exit 1
      fi
      rm -f "$evidence"
    done
    printf 'PASS %s/25\n' "$iteration"
  done
  printf 'RESULT 25/25\n'
} 2>&1 | tee repaint-after-wait-stress-macos.log
