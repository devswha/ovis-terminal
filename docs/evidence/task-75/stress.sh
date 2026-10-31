#!/usr/bin/env bash
# Runs the complete server suite 100 times while eight owned CPU workers stay busy.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../../.."

export RUSTUP_HOME=/tmp/ulw-ovis-yCJzPH/toolchain/rustup
export CARGO_HOME=/tmp/ulw-ovis-yCJzPH/toolchain/cargo
export PATH="$CARGO_HOME/bin:$PATH"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$CARGO_HOME/bin/zigcc-linker"
export CC="$CARGO_HOME/bin/zigcc-linker"
export AR='/tmp/ulw-ovis-yCJzPH/toolchain/zigvenv/bin/python -m ziglang ar'
export ATTEMPT_DIR="$PWD/.omo/evidence/ovis-terminal-v1/task-75"

load_pids=()
for worker in $(seq 1 8); do
    sha256sum /dev/zero >/dev/null 2>&1 &
    load_pids+=("$!")
done
cleanup() {
    kill "${load_pids[@]}" 2>/dev/null || true
    wait "${load_pids[@]}" 2>/dev/null || true
}
trap cleanup EXIT

printf 'CANDIDATE_SHA=%s\n' "$(GIT_MASTER=1 git rev-parse HEAD)"
printf 'LOAD_COMMAND=8 concurrent sha256sum /dev/zero workers\n'
printf 'COMMAND=cargo nextest run -p ovis-terminal-server --locked -j 8\n'
uname -srm
rustc --version
cargo nextest --version
for run in $(seq 1 100); do
    printf '\nSTRESS_RUN=%s/100\n' "$run"
    cargo nextest run -p ovis-terminal-server --locked -j 8
    printf 'STRESS_PASS=%s/100 EXIT=0\n' "$run"
done
printf '\nSTRESS_RESULT=100/100 EXIT=0\n'
