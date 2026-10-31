#!/usr/bin/env bash
# Run independent CPU load alongside every nextest invocation; preserve its status.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../../.."
export RUSTUP_HOME=/tmp/ulw-ovis-yCJzPH/toolchain/rustup
export CARGO_HOME=/tmp/ulw-ovis-yCJzPH/toolchain/cargo
export PATH="$CARGO_HOME/bin:$PATH"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$CARGO_HOME/bin/zigcc-linker"
export CC="$CARGO_HOME/bin/zigcc-linker"
export AR="/tmp/ulw-ovis-yCJzPH/toolchain/zigvenv/bin/python -m ziglang ar"
# Python owns, terminates and joins all eight load workers even on test failure.
exec python3 -c '
import multiprocessing
import os
import subprocess
import sys
import tempfile

def burn():
    value = 1
    while True:
        value = (value * 1664525 + 1013904223) % (1 << 32)

context = multiprocessing.get_context("fork")
workers = [context.Process(target=burn) for _ in range(8)]
for worker in workers:
    worker.start()
try:
    command = ["cargo", "nextest", "run", "-p", "ovis-terminal-holder",
               "--locked", "-j", "8", "--no-tests=fail"] + sys.argv[2:]
    print("CPU load workers: 8; command: " + " ".join(command), flush=True)
    for iteration in range(1, int(sys.argv[1]) + 1):
        print(f"RUN {iteration}/{sys.argv[1]}", flush=True)
        with tempfile.TemporaryDirectory(prefix="ot74-") as temporary:
            result = subprocess.run(
                command, check=False, env={**os.environ, "TMPDIR": temporary})
        print(f"RUN {iteration} exit: {result.returncode}", flush=True)
        if result.returncode:
            sys.exit(result.returncode)
    print(f"PASS {sys.argv[1]}/{sys.argv[1]}", flush=True)
finally:
    for worker in workers:
        worker.terminate()
    for worker in workers:
        worker.join()
' "${1:-100}" "${@:2}"
