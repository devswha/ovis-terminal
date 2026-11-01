#!/usr/bin/env bash
# Unix full platform QA (plan todo 60), run by .github/workflows/platform-qa.yml.
# Every command runs to completion with its exit code, stdout and stderr archived in
# <out>; the run fails when any command exits non-zero or any conformance case is
# failed or skipped. Each conformance run uses its own isolated root.
set -uo pipefail

out=${1:?usage: unix-full.sh <output-dir>}
mkdir -p "$out"
target=${CARGO_TARGET_DIR:-target}
bin="$target/debug/ovis-terminal"
runner="$target/debug/conformance"
evidence="$out/task-60-ovis-terminal-v1.json"
status=0
: > "$out/exits.tsv"

hash() {
    if command -v sha256sum > /dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi
}

record() {
    local name=$1
    shift
    printf '%q ' "$@" > "$out/$name.cmd"
    "$@" > "$out/$name.stdout" 2> "$out/$name.stderr"
    local code=$?
    printf '%s\t%s\n' "$name" "$code" >> "$out/exits.tsv"
    if [ "$code" -ne 0 ]; then
        status=1
    fi
}

{
    uname -a
    rustc --version
    git rev-parse HEAD
} > "$out/host.txt"
hash "$bin" "$runner" > "$out/binaries.sha256"

record all-families "$runner" --binary "$bin" --scenario all/all --isolated --case all \
    --evidence "$out/all-families.json"
record matrix-happy "$runner" --binary "$bin" --scenario platform/matrix --isolated --case happy \
    --manual-transcript --evidence "$evidence"
record matrix-failure-numeric "$runner" --binary "$bin" --scenario platform/matrix --isolated \
    --case failure --manual-transcript --inject '{"method":"hello","params":{"generation":999999}}' \
    --evidence "$evidence" --append
record matrix-failure-decimal "$runner" --binary "$bin" --scenario platform/matrix --isolated \
    --case failure --manual-transcript --inject '{"method":"hello","params":{"generation":"999999"}}' \
    --evidence "$evidence" --append
record service-tests cargo nextest run --locked -p ovis-terminal-holder -p ovis-terminal-cli \
    -p ovis-terminal-server --no-tests=fail \
    -E 'test(/service/) | test(/dispatch::remote::/) | test(/dispatch::services::/)'
record no-failed-or-skipped python3 probes/platform-qa/no-skips.py \
    "$out/all-families.json" "$evidence"

hash "$out"/*.json > "$out/evidence.sha256"
cat "$out/exits.tsv"
exit "$status"
