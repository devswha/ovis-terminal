#!/usr/bin/env bash
# Stress lane quality-gates: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

set -euo pipefail
cargo build --locked -p ovis-terminal-cli --bin ovis-terminal
{
  uname -a
  rustc --version
  git rev-parse HEAD
  for iteration in $(seq 1 25); do
    printf '\nITERATION %s/25\n' "$iteration"
    cargo nextest run -p ovis-terminal-probes --locked --no-tests=fail -E 'test(quality::) | binary(quality_gates) | binary(quality_repository)'
    cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario quality/gates --isolated --case all --evidence "quality-gates-$iteration.json"
    cargo run --locked --manifest-path probes/Cargo.toml --bin evidence-audit -- --input "quality-gates-$iteration.json"
    printf 'PASS %s/25\n' "$iteration"
  done
  printf 'RESULT 25/25\n'
} 2>&1 | tee quality-gates-stress-macos.log
