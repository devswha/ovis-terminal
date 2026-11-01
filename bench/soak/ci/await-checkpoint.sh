#!/usr/bin/env bash
# Blocks until the running soak's checkpoint reports at least <hours> of measured time,
# or the soak process has exited. Used by .github/workflows/soak.yml before each external
# artifact checkpoint upload. Exits 1 when the soak ended before reaching <hours> without
# a final report, so a stopped run can never look complete.
set -euo pipefail

dir="$1"
hours="$2"
wanted=$((hours * 3600 * 1000))
checkpoint="$dir/soak.checkpoint.json"
pid="$(cat "$dir/soak.pid")"

elapsed() {
    python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["elapsed_ms"])' "$checkpoint" 2>/dev/null || echo 0
}

while kill -0 "$pid" 2>/dev/null; do
    if [ "$(elapsed)" -ge "$wanted" ]; then
        echo "checkpoint at $(elapsed) ms >= ${hours} h"
        exit 0
    fi
    sleep 60
done
if [ -f "$dir/soak.json" ]; then
    echo "soak exited; final report present"
    exit 0
fi
echo "soak exited before ${hours} h without a final report (checkpoint at $(elapsed) ms)" >&2
exit 1
