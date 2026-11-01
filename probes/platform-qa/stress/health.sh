#!/usr/bin/env bash
# Stress lane health: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

set -euo pipefail
cargo build --locked -p ovis-terminal-cli --bin ovis-terminal
{
  uname -a
  rustc --version
  git rev-parse HEAD
  for iteration in $(seq 1 25); do
    printf '\nITERATION %s/25\n' "$iteration"
    cargo nextest run -p ovis-terminal-core -p ovis-terminal-protocol -p ovis-terminal-server --lib --locked --no-tests=fail -E 'test(health::)'
    cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario observability/health --isolated --case all --evidence "health-$iteration.json"
    printf 'PASS %s/25\n' "$iteration"
  done
  printf 'RESULT 25/25\n'
} 2>&1 | tee health-stress-macos.log
