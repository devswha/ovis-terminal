#!/usr/bin/env bash
# Stress lane soak: run by .github/workflows/probe-stress-macos.yml.
# Native macOS: the soak's own tests, the self-test smoke of every disruption, the
# holder-tail-gap-and-lost-receipt fixture (must exit 0 only with screen_exact false, a
# gap and outcome_unknown) and a 600 s, 50-terminal continuous run with one control
# crash, one handover, interruptions, a stalled viewer and the holder-loss probe. This is
# a platform check, not the 72-hour soak (.github/workflows/soak.yml).
set -euo pipefail

cargo build --locked --release -p ovis-terminal-cli --bin ovis-terminal
cargo build --locked --release --manifest-path bench/Cargo.toml --bin soak
summary='import json,sys; r=json.load(open(sys.argv[1])); print(json.dumps({k: r.get(k) for k in ("label", "host", "measured", "probe", "fixture", "error", "verdict")}))'
{
  uname -a
  sw_vers || true
  rustc --version
  git rev-parse HEAD
  cargo nextest run --locked -p ovis-terminal-bench --no-tests=fail -E 'test(soak::)'
  printf '\nITERATION self-test\n'
  target/release/soak --binary target/release/ovis-terminal --isolated --self-test --evidence self-test.json > /dev/null
  python3 -c "$summary" self-test.json
  printf 'PASS self-test\n'
  printf '\nITERATION fixture\n'
  target/release/soak --binary target/release/ovis-terminal --isolated --inject '{"fixture":"holder-tail-gap-and-lost-receipt"}' --evidence fixture.json > /dev/null
  python3 -c "$summary" fixture.json
  printf 'PASS fixture\n'
  printf '\nITERATION continuous-600s\n'
  target/release/soak --binary target/release/ovis-terminal --isolated --terminals 50 --duration-seconds 600 --core-crashes 1 --remote-interruptions 1 --warmup-seconds 60 --window-seconds 60 --evidence continuous.json > /dev/null
  python3 -c "$summary" continuous.json
  printf 'PASS continuous-600s\n'
  printf 'RESULT 3/3\n'
} 2>&1 | tee soak-stress-macos.log
