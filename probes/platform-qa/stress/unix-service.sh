#!/usr/bin/env bash
# Stress lane unix-service: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

set -euo pipefail
{
  uname -a
  rustc --version
  git rev-parse HEAD
  cargo build --locked -p ovis-terminal-cli --bin ovis-terminal
  for iteration in $(seq 1 10); do
    printf '\nITERATION %s/10\n' "$iteration"
    cargo test --locked -p ovis-terminal-holder --lib service
    cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario platform/unix-service --isolated --case all --evidence "unix-service-$iteration.json"
  done
  session="t51ci$GITHUB_RUN_ID"
  bin=target/debug/ovis-terminal
  "$bin" --json service status --session "$session"
  status=0
  "$bin" --json service install --manager launchd --session "$session" || status=$?
  if test "$status" -eq 0; then
    "$bin" --json service start --manager launchd --session "$session"
    "$bin" --json service status --session "$session" | grep '"state":"running"'
    "$bin" --json service stop --session "$session"
    "$bin" --json service status --session "$session" | grep '"state":"stopped"'
    "$bin" --json service uninstall --manager launchd --session "$session"
    test ! -e "$HOME/Library/LaunchAgents/dev.ovis-terminal.holder.$session.plist"
  else
    printf 'launchd install refused with exit %s; the typed error above names the missing prerequisite\n' "$status"
    test "$status" -eq 4
  fi
  printf 'RESULT pass\n'
} 2>&1 | tee unix-service-macos.log
