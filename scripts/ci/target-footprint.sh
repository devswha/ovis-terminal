#!/usr/bin/env bash
# CI-only (Linux runners): report the cargo target footprint of a finished job and fail
# when a debug executable still carries full debuginfo. Measured (docs/evidence/task-F2b):
# the ovis-terminal binary is ~146 MB with line-tables-only and ~341 MB with
# `[profile.dev] debug = true`, so the 200 MB ceiling separates the two. Local check.sh
# never runs this script.
# Usage: target-footprint.sh <target-dir>...
set -euo pipefail

ceiling="${OVIS_CI_MAX_DEBUG_BIN_BYTES:-200000000}"
summary="${GITHUB_STEP_SUMMARY:-/dev/null}"
status=0

if [ "$#" -eq 0 ]; then
    echo "target-footprint: rule=usage give at least one target directory" >&2
    exit 2
fi

report() {
    printf '%s\n' "$*"
    printf '%s\n\n' "$*" >> "$summary"
}

report "### target footprint (ceiling ${ceiling} bytes per debug executable)"
report "$(df -Pk . | awk 'NR == 2 { printf "disk: %s KiB used, %s KiB free", $3, $4 }')"

for root in "$@"; do
    if [ ! -d "$root/debug" ]; then
        echo "target-footprint: rule=missing-target $root/debug does not exist" >&2
        exit 2
    fi
    for dir in "$root" "$root/debug/deps" "$root/debug/build" "$root/debug/incremental"; do
        [ -d "$dir" ] && report "$(du -sb "$dir" | awk '{ printf "%s: %s bytes", $2, $1 }')"
    done
    largest=0
    largest_path=none
    # Executables only: binaries in debug/ and test harnesses in debug/deps (no extension).
    while IFS= read -r -d '' bin; do
        size="$(stat -c %s "$bin")"
        if [ "$size" -gt "$largest" ]; then
            largest="$size"
            largest_path="$bin"
        fi
        if [ "$size" -gt "$ceiling" ]; then
            echo "target-footprint: rule=debuginfo-ceiling $bin is $size bytes, above $ceiling" >&2
            status=1
        fi
    done < <(find "$root/debug" "$root/debug/deps" -maxdepth 1 -type f -perm -u+x ! -name '*.*' -print0)
    report "largest debug executable: $largest_path ($largest bytes)"
done

exit "$status"
