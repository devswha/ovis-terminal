#!/usr/bin/env bash
# Stress lane conversation-records: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

set -euo pipefail
cargo build --locked -p ovis-terminal-cli --bin ovis-terminal
{
  uname -a
  rustc --version
  git rev-parse HEAD
  for iteration in $(seq 1 25); do
    printf '\nITERATION %s/25\n' "$iteration"
    cargo nextest run -p ovis-terminal-agent -p ovis-terminal-protocol --locked --no-tests=fail -E 'test(conversation)'
    cargo nextest run -p ovis-terminal-server --test dispatch --locked --no-tests=fail -E 'test(conversation_records) | test(session_binding)'
    cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/records --isolated --case all --evidence "conversation-records-$iteration.json"
    printf 'PASS %s/25\n' "$iteration"
  done
  printf 'RESULT 25/25\n'
} 2>&1 | tee conversation-records-stress-macos.log
