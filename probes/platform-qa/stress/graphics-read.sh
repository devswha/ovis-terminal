#!/usr/bin/env bash
# Stress lane graphics-read (plan todo 85): run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail
{
  uname -a
  rustc --version
  git rev-parse HEAD
  cargo build --locked -p ovis-terminal-cli -p ovis-terminal-conformance
  for iteration in $(seq 1 25); do
    printf '\nITERATION %s/25\n' "$iteration"
    cargo nextest run -p ovis-terminal-vt -p ovis-terminal-core -p ovis-terminal-protocol --locked --no-tests=fail placeholder image_reads include_images
    if ! target/debug/conformance --binary target/debug/ovis-terminal --scenario graphics/read --isolated --case all --evidence "graphics-read-$iteration.json"; then
      cat "graphics-read-$iteration.json"
      exit 1
    fi
    printf 'PASS %s/25\n' "$iteration"
  done
  printf 'RESULT 25/25\n'
} 2>&1 | tee graphics-read-stress-macos.log
