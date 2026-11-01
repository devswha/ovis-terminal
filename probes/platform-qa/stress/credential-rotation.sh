#!/usr/bin/env bash
# Stress lane credential-rotation: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

set -euo pipefail
cargo build --locked -p ovis-terminal-cli --bin ovis-terminal
{
  uname -a
  rustc --version
  git rev-parse HEAD
  for iteration in $(seq 1 25); do
    printf '\nITERATION %s/25\n' "$iteration"
    cargo nextest run -p ovis-terminal-authz --test authority --locked --no-tests=fail -E 'test(rotation::)'
    cargo nextest run -p ovis-terminal-core --test input --locked --no-tests=fail -E 'test(authority_recheck::)'
    cargo nextest run -p ovis-terminal-server --test dispatch --locked --no-tests=fail -E 'test(credential_rotation::)'
    cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario security/rotation --isolated --case all --evidence "rotation-$iteration.json"
    printf 'PASS %s/25\n' "$iteration"
  done
  printf 'RESULT 25/25\n'
} 2>&1 | tee credential-rotation-stress-macos.log
