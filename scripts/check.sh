#!/usr/bin/env bash
# Local quality gate. Every step must pass; the first failure stops the run with
# that step's exit status. CI runs the same steps (see .github/workflows/ci.yml).
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

step() {
    printf '\n==> %s\n' "$*"
    "$@"
}

step cargo +nightly fmt --all -- --check
step cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
# nextest exits non-zero when no test runs, so an empty selection cannot pass.
step cargo nextest run --workspace --all-targets --all-features --locked --no-tests=fail
step cargo test --doc --workspace --all-features --locked
step cargo run --locked --quiet -p workspace-hygiene -- check --root .
step bash scripts/hygiene/manifests.sh
step bash scripts/hygiene/json.sh

printf '\ncheck.sh: all steps passed\n'
