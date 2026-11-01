#!/usr/bin/env bash
# Stress lane boundary-policy: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

cargo build --locked -p ovis-terminal-cli --bin ovis-terminal
cargo build --locked --manifest-path conformance/Cargo.toml --bin conformance

set -euo pipefail
{
  uname -a
  rustc --version
  git rev-parse HEAD
  for iteration in $(seq 1 25); do
    printf '\nITERATION %s/25\n' "$iteration"
    cargo nextest run -p ovis-terminal-protocol --lib --locked --no-tests=fail -E 'test(boundary_policy::)'
    cargo nextest run -p ovis-terminal-authz --locked --no-tests=fail -E 'test(validation::) | test(precheck::)'
    cargo nextest run -p ovis-terminal-core --lib --locked --no-tests=fail -E 'test(redaction::)'
    cargo nextest run -p ovis-terminal-server --locked --no-tests=fail -E 'test(boundary_policy) | test(credential_rotation::)'
    target/debug/conformance --binary target/debug/ovis-terminal --scenario security/validation --isolated --case all --evidence "boundary-policy-$iteration.json"
    printf 'PASS %s/25\n' "$iteration"
  done
  printf 'RESULT 25/25\n'
} 2>&1 | tee boundary-policy-stress-macos.log
