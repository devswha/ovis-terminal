#!/usr/bin/env bash
# Stress lane client-ts: run by .github/workflows/probe-stress-macos.yml.
set -euo pipefail

set -euo pipefail
cargo build --locked -p ovis-terminal-cli --bin ovis-terminal -p ovis-terminal-conformance --bin conformance
bun install --frozen-lockfile
cd packages/client-ts
{
  uname -a
  bun --version
  git rev-parse HEAD
  bun run generate:check
  bun run typecheck
  for iteration in $(seq 1 25); do
    printf '\nITERATION %s/25\n' "$iteration"
    bun test
    printf 'PASS %s/25\n' "$iteration"
  done
  printf 'RESULT 25/25\n'
} 2>&1 | tee ../../client-ts-stress-macos.log
