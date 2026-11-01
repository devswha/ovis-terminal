#!/usr/bin/env bash
# Stress lane platform-qa: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

{
  uname -a
  rustc --version
  git rev-parse HEAD
  cargo build --locked -p ovis-terminal-cli --bin ovis-terminal
  cargo build --locked --manifest-path conformance/Cargo.toml --bin conformance
  for iteration in $(seq 1 25); do
    printf '\nITERATION %s/25\n' "$iteration"
    cargo nextest run -p ovis-terminal-probes --locked --no-tests=fail -E 'test(platform_qa) | binary(platform_qa)'
    target/debug/conformance --binary target/debug/ovis-terminal --scenario platform/matrix \
      --isolated --case all --evidence "platform-matrix-$iteration.json"
    printf 'PASS %s/25\n' "$iteration"
  done
  printf 'RESULT 25/25\n'
} 2>&1 | tee platform-qa-stress-macos.log
