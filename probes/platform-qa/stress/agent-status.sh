#!/usr/bin/env bash
# Stress lane agent-status: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

set -euo pipefail
{
  uname -a
  rustc --version
  git rev-parse HEAD
  cargo build --locked -p ovis-terminal-cli --bin ovis-terminal
  cargo build --locked --manifest-path conformance/Cargo.toml --bin conformance
  for iteration in $(seq 1 50); do
    printf '\nITERATION %s/50\n' "$iteration"
    cargo test --locked -q -p ovis-terminal-agent
    target/debug/conformance --binary target/debug/ovis-terminal --scenario agent/status \
      --isolated --case all --evidence "agent-status-$iteration.json"
    printf 'PASS %s/50\n' "$iteration"
  done
  printf 'RESULT 50/50\n'
} 2>&1 | tee agent-status-stress-macos.log
