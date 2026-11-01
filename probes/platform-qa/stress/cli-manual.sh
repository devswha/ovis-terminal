#!/usr/bin/env bash
# Stress lane cli-manual: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

{
  uname -a
  rustc --version
  git rev-parse HEAD
  cargo build --locked -p ovis-terminal-cli --bin ovis-terminal
  cargo build --locked --manifest-path conformance/Cargo.toml --bin conformance
  for iteration in $(seq 1 25); do
    printf '\nITERATION %s/25\n' "$iteration"
    cargo test --locked -q -p ovis-terminal-cli --lib
    cargo test --locked -q -p ovis-terminal-cli --test assembly producer
    cargo test --locked -q -p ovis-terminal-cli --test cli
    target/debug/conformance --binary target/debug/ovis-terminal --scenario cli/manual \
      --isolated --case all --evidence "cli-manual-$iteration.json"
    target/debug/conformance --binary target/debug/ovis-terminal --scenario cli/manual \
      --isolated --case failure --inject 'ovis-terminal agent get --unknown-flag --json' \
      --evidence "cli-manual-$iteration.json" --append
    printf 'PASS %s/25\n' "$iteration"
  done
  printf 'RESULT 25/25\n'
} 2>&1 | tee cli-manual-stress-macos.log
