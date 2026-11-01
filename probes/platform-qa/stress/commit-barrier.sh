#!/usr/bin/env bash
# Stress lane commit-barrier: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

set -euo pipefail
cargo build --locked -p ovis-terminal-cli --bin ovis-terminal
{
  uname -a
  rustc --version
  git rev-parse HEAD
  for iteration in $(seq 1 25); do
    printf '\nITERATION %s/25\n' "$iteration"
    cargo nextest run -p ovis-terminal-core --test commit --locked --no-tests=fail
    cargo nextest run -p ovis-terminal-server --test dispatch --locked --no-tests=fail -E 'test(agent_assembly::)'
    cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/e2e --isolated --case all --evidence "commit-barrier-$iteration.json"
    printf 'PASS %s/25\n' "$iteration"
  done
  printf 'RESULT 25/25\n'
} 2>&1 | tee commit-barrier-stress-macos.log
