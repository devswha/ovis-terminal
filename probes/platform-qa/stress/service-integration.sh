#!/usr/bin/env bash
# Stress lane service-integration: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

set -euo pipefail
cargo build --locked -p ovis-terminal-cli --bin ovis-terminal
{
  uname -a
  rustc --version
  git rev-parse HEAD
  for iteration in $(seq 1 25); do
    printf '\nITERATION %s/25\n' "$iteration"
    cargo nextest run -p ovis-terminal-server --lib --locked --no-tests=fail -E 'test(dispatch::remote::) | test(dispatch::services::) | test(conn::core::)'
    cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario remote/e2e --isolated --case all --evidence "remote-e2e-$iteration.json"
    cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario recovery/e2e --isolated --case all --evidence "recovery-e2e-$iteration.json"
    printf 'PASS %s/25\n' "$iteration"
  done
  printf 'RESULT 25/25\n'
} 2>&1 | tee service-integration-stress-macos.log
