#!/usr/bin/env bash
# Stress lane terminal50: run by .github/workflows/probe-stress-macos.yml.
# Native macOS: the 16-PTY IDEAL profile (every sample <= 100 ms), the separately
# labelled 50-PTY capacity profile (p99 <= 100 ms), the delayed-writer and the
# delayed-admission failures, which must exit 1 with budget_exceeded, and a 49-terminal
# capacity run, which must be refused with exit 2. Each report is copied into the log.
set -euo pipefail

cargo build --locked --release -p ovis-terminal-cli --bin ovis-terminal
cargo build --locked --release --manifest-path bench/Cargo.toml --bin terminal50
{
  uname -a
  sw_vers || true
  rustc --version
  git rev-parse HEAD
  cargo nextest run --locked -p ovis-terminal-bench --no-tests=fail -E 'test(terminal50::)'
  for iteration in 1 2 3; do
    printf '\nITERATION %s/3\n' "$iteration"
    target/release/terminal50 --binary target/release/ovis-terminal --isolated --terminals 16 --clients 8 --inflight 16 --output-bytes-per-second 1048576 --samples 10000 --admission-max-ms 100 --event-max-ms 100 --evidence "ideal-$iteration.json" > /dev/null
    python3 -c 'import json,sys; r=json.load(open(sys.argv[1])); print(json.dumps({k: r[k] for k in ("label", "host", "requested", "realized", "admission", "event", "rejected", "clients", "footprint", "verdict")}))' "ideal-$iteration.json"
    target/release/terminal50 --binary target/release/ovis-terminal --isolated --terminals 50 --clients 8 --inflight 16 --output-bytes-per-second 1048576 --samples 10000 --admission-p99-ms 100 --event-p99-ms 100 --evidence "capacity-$iteration.json" > /dev/null
    python3 -c 'import json,sys; r=json.load(open(sys.argv[1])); print(json.dumps({k: r[k] for k in ("label", "host", "requested", "realized", "admission", "event", "rejected", "footprint", "verdict")}))' "capacity-$iteration.json"
    set +e
    target/release/terminal50 --binary target/release/ovis-terminal --isolated --terminals 16 --clients 8 --inflight 16 --output-bytes-per-second 1048576 --samples 10000 --admission-max-ms 100 --event-max-ms 100 --inject '{"fixture":"control-writer-delay","delay_ms":150}' --evidence "failure-$iteration.json" > /dev/null 2> "failure-$iteration.stderr"
    code=$?
    set -e
    cat "failure-$iteration.stderr"
    test "$code" = 1
    grep -q '^budget_exceeded: ' "failure-$iteration.stderr"
    python3 -c 'import json,sys; r=json.load(open(sys.argv[1])); print(json.dumps(r["verdict"]))' "failure-$iteration.json"
    set +e
    target/release/terminal50 --binary target/release/ovis-terminal --isolated --terminals 16 --clients 8 --inflight 16 --output-bytes-per-second 1048576 --samples 10000 --admission-max-ms 100 --event-max-ms 100 --inject '{"fixture":"admission-delay","delay_ms":150}' --evidence "admission-$iteration.json" > /dev/null 2> "admission-$iteration.stderr"
    code=$?
    target/release/terminal50 --binary target/release/ovis-terminal --isolated --terminals 49 --clients 8 --inflight 16 --output-bytes-per-second 1048576 --samples 10000 --admission-p99-ms 100 --event-p99-ms 100 > /dev/null 2> "refused-$iteration.stderr"
    refused=$?
    set -e
    cat "admission-$iteration.stderr" "refused-$iteration.stderr"
    test "$code" = 1
    grep -q '^budget_exceeded: ' "admission-$iteration.stderr"
    test "$refused" = 2
    python3 -c 'import json,sys; r=json.load(open(sys.argv[1])); print(json.dumps(r["verdict"]))' "admission-$iteration.json"
    printf 'PASS %s/3\n' "$iteration"
  done
  printf 'RESULT 3/3\n'
} 2>&1 | tee terminal50-stress-macos.log
