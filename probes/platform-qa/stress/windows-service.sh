#!/usr/bin/env bash
# Stress lane windows-service: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

set -euo pipefail
{
  uname -a
  rustc --version
  git rev-parse HEAD
  cargo build --locked -p ovis-terminal-cli --bin ovis-terminal
  cp target/debug/ovis-terminal.exe /c/ovis54/ovis-terminal.exe
  for iteration in $(seq 1 3); do
    printf '\nITERATION %s/3\n' "$iteration"
    cargo test --locked -p ovis-terminal-holder --lib service::windows:: -- --nocapture
    cargo test --locked -p ovis-terminal-server --lib -- listen::pipe dispatch::lanes::windows_service
    cargo test --locked -p ovis-terminal-cli --lib service::windows::fixture_tests -- --ignored --nocapture
  done
  printf 'RESULT pass\n'
} 2>&1 | tee windows-service.log
