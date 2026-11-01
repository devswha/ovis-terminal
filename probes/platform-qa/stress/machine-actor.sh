#!/usr/bin/env bash
# Stress lane machine-actor: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

set -euo pipefail
cargo build --locked -p ovis-terminal-cli --bin ovis-terminal
{
  uname -a
  rustc --version
  git rev-parse HEAD
  for iteration in $(seq 1 25); do
    printf '\nITERATION %s/25\n' "$iteration"
    cargo nextest run -p ovis-terminal-remote --locked --no-tests=fail -E 'test(actors::) | test(mirror::) | test(orphans::)'
    cargo nextest run -p ovis-terminal-protocol --locked --no-tests=fail -E 'test(machine_actor::)'
    cargo nextest run -p ovis-terminal-server --lib --locked --no-tests=fail -E 'test(machine_actor::)'
    cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario remote/isolation --isolated --case all --evidence "machine-actor-$iteration.json"
    printf 'PASS %s/25\n' "$iteration"
  done
  printf 'RESULT 25/25\n'
} 2>&1 | tee machine-actor-stress-macos.log
