#!/usr/bin/env bash
# Every Cargo package must inherit the workspace lints (`[lints] workspace = true`)
# and must not redeclare its own lint tables. Exits 1 naming each offending manifest.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."

status=0
checked=0
while IFS= read -r manifest; do
    checked=$((checked + 1))
    if ! awk '
        /^\[lints\][[:space:]]*$/ { in_lints = 1; next }
        /^\[/ { in_lints = 0 }
        in_lints && /^workspace[[:space:]]*=[[:space:]]*true[[:space:]]*$/ { found = 1 }
        END { exit found ? 0 : 1 }
    ' "$manifest"; then
        echo "$manifest: rule=workspace-lints missing [lints] workspace = true"
        status=1
    fi
    if grep -nE '^\[lints\.' "$manifest"; then
        echo "$manifest: rule=workspace-lints declares crate-local lint tables"
        status=1
    fi
done < <(git ls-files --cached --others --exclude-standard -- '*Cargo.toml' ':!:Cargo.toml')

if [ "$checked" -eq 0 ]; then
    echo "manifests.sh: no member manifests found"
    exit 1
fi
echo "manifests.sh: $checked member manifest(s) checked"
exit "$status"
