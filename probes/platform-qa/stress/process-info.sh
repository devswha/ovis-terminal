#!/usr/bin/env bash
# Stress lane process-info: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

set -euo pipefail
{
  uname -a
  rustc --version
  git rev-parse HEAD
  cargo build --locked -p ovis-terminal-cli --bin ovis-terminal
  cargo build --locked --manifest-path conformance/Cargo.toml --bin conformance
  for iteration in $(seq 1 30); do
    printf '\nITERATION %s/30\n' "$iteration"
    cargo test --locked -q -p ovis-terminal-holder --lib process::
    cargo test --locked -q -p ovis-terminal-core --lib process_info
    cargo test --locked -q -p ovis-terminal-server --lib process_info
    target/debug/conformance --binary target/debug/ovis-terminal --scenario platform/process-info \
      --isolated --case all --evidence "process-info-$iteration.json"
    printf 'PASS %s/30\n' "$iteration"
  done
  printf 'RESULT 30/30\n'
} 2>&1 | tee process-info-stress-macos.log
