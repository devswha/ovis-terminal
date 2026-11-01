#!/usr/bin/env bash
# Local quality gate. Every step must pass; the first failure stops the run with
# that step's exit status. CI runs the same steps (see .github/workflows/ci.yml).
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

step() {
    printf '\n==> %s\n' "$*"
    "$@"
}

# /tmp is a shared RAM disk on the development host: a nearly full one turns SQLite and
# socket tests into misleading I/O failures, so refuse to start below 2 GiB free.
tmp_space() {
    local dir="${TMPDIR:-/tmp}" free_kib
    free_kib="$(df -Pk "$dir" | awk 'NR == 2 { print $4 }')"
    if [ "${free_kib:-0}" -lt 2097152 ]; then
        echo "check.sh: rule=tmp-space $dir has ${free_kib:-0} KiB free, below 2097152 KiB" >&2
        return 1
    fi
}

step tmp_space
step cargo +nightly fmt --all -- --check
step cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
# nextest exits non-zero when no test runs, so an empty selection cannot pass.
step cargo nextest run --workspace --all-targets --all-features --locked --no-tests=fail
step cargo test --doc --workspace --all-features --locked
# nextest runs each test in its own process; one threaded run catches process-global state
# that two tests share (the PUMP_GATE class of flake, todo 81).
step cargo test --locked -p ovis-terminal-core --test subscriptions
step cargo run --locked --quiet -p workspace-hygiene -- check --root .
step bash scripts/hygiene/manifests.sh
step bash scripts/hygiene/json.sh
step cargo run --locked --quiet --manifest-path probes/Cargo.toml --bin source-check -- \
    --roots crates packages/client-ts conformance bench probes --max-pure-loc 250 --max-params 3 \
    --violations docs/research/violations.md --checklist docs/quality-checklist.md

printf '\ncheck.sh: all steps passed\n'
