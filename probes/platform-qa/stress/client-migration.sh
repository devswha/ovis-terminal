#!/usr/bin/env bash
# Stress lane client-migration (todo 65): run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

cargo build --locked -p ovis-terminal-cli --bin ovis-terminal -p ovis-terminal-conformance --bin conformance
{
  uname -a
  git rev-parse HEAD
  for iteration in $(seq 1 10); do
    printf '\nITERATION %s/10\n' "$iteration"
    cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario client/migration --isolated --case all --guide docs/ovis-adapter.md --evidence "client-migration-$iteration.json"
    cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario client/migration --isolated --case failure --inject '{"example":"prompt.answer","version":"stale"}' --evidence "client-migration-$iteration.json" --append
    printf 'PASS %s/10\n' "$iteration"
  done
  printf 'RESULT 10/10\n'
} 2>&1 | tee client-migration-stress-macos.log
