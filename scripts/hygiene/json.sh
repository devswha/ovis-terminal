#!/usr/bin/env bash
# Every tracked or new JSON file (package manifests, conformance scenarios) must parse.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."

status=0
checked=0
while IFS= read -r file; do
    checked=$((checked + 1))
    if ! python3 -m json.tool "$file" > /dev/null; then
        echo "$file: rule=json invalid JSON"
        status=1
    fi
done < <(git ls-files --cached --others --exclude-standard -- '*.json')

echo "json.sh: $checked JSON file(s) checked"
exit "$status"
