#!/usr/bin/env bash
# Usage (from the repo root, with the local toolchain env exported):
#   bash docs/evidence/task-72/negative-evidence/run.sh
set -euo pipefail
D=docs/evidence/task-72/negative-evidence
SRC=probes/platform/tests/unix_failure.rs
# Verbatim copy of the helper region (TEARDOWN_DEADLINE .. on_this_os).
awk '/^\/\/\/ How long a joined thread may linger/{p=1} /^\/\/\/ The blocked writer runs/{p=0} p' "$SRC" > "$D/helper_extract.rs"
rustc -O -A dead_code -C linker="$CC" "$D/harness.rs" -o /tmp/t72-harness 2> "$D/build.log"
N=100000
for mode in single bounded; do
  {
    echo "\$ for i in 1 2 3 4; do /tmp/t72-harness $mode $N & done; wait   # 4-way load, Linux x86_64"
    for i in 1 2 3 4; do /tmp/t72-harness "$mode" "$N" & done
    wait
  } > "$D/$mode-4way.log" 2>&1
  cat "$D/$mode-4way.log"
done
