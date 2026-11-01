#!/usr/bin/env bash
# Stress lane update: run by .github/workflows/probe-stress-macos.yml.
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
    cargo nextest run -p ovis-terminal-services --lib --locked --no-tests=fail -E 'test(update::)'
    cargo nextest run -p ovis-terminal-server --lib --locked --no-tests=fail -E 'test(lanes::update)'
    target/debug/conformance --binary target/debug/ovis-terminal --scenario services/update --isolated --case all --evidence "update-$iteration.json"
    printf 'PASS %s/25\n' "$iteration"
  done
  printf 'RESULT 25/25\n'
} 2>&1 | tee update-stress-macos.log
