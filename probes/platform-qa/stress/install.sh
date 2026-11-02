#!/usr/bin/env bash
# Stress lane install: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

cargo build --locked -p ovis-terminal-cli --bin ovis-terminal
cargo build --locked --manifest-path probes/Cargo.toml --bin install-probe
cargo build --locked --manifest-path conformance/Cargo.toml --bin conformance
{
  uname -a
  rustc --version
  git rev-parse HEAD
  for iteration in $(seq 1 10); do
    printf '\nITERATION %s/10\n' "$iteration"
    target/debug/install-probe --script ./install.sh --isolated --evidence "install-$iteration.json"
    target/debug/conformance --binary target/debug/ovis-terminal --scenario release/install \
      --isolated --case all --evidence "install-conformance-$iteration.json"
    printf 'PASS %s/10\n' "$iteration"
  done
  printf 'RESULT 10/10\n'
} 2>&1 | tee install-stress-macos.log
