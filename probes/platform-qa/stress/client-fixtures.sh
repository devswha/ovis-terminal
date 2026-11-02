#!/usr/bin/env bash
# Stress lane client-fixtures (todo 64): run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

cargo build --locked -p ovis-terminal-cli --bin ovis-terminal -p ovis-terminal-conformance --bin conformance
bun install --frozen-lockfile
{
  uname -a
  bun --version
  git rev-parse HEAD
  for iteration in $(seq 1 10); do
    printf '\nITERATION %s/10\n' "$iteration"
    cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario client/fixtures --isolated --case all --evidence "client-fixtures-$iteration.json"
    (cd packages/client-ts && bun run test:fixtures -- --fixtures ../../conformance/fixtures/ovis)
    printf 'PASS %s/10\n' "$iteration"
  done
  printf 'RESULT 10/10\n'
} 2>&1 | tee client-fixtures-stress-macos.log
