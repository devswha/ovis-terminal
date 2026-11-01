#!/usr/bin/env bash
# The two gate-review round-2 mutants: each is applied to a backed-up source file, the
# strengthened real-producer test runs, the file is restored, and the test must fail.
set -uo pipefail
cd "$(dirname "$0")/../../.."
log=docs/evidence/task-59/mutants-r3.log
: > "$log"
mutate() {
  local name=$1 file=$2 from=$3 to=$4 test=$5
  cp "$file" "$file.orig"
  python3 - "$file" "$from" "$to" <<'P'
import sys
path, old, new = sys.argv[1:]
text = open(path).read()
assert text.count(old) == 1, f"mutant anchor not unique in {path}: {old!r}"
open(path, "w").write(text.replace(old, new))
P
  {
    echo "### $name: $file"
    echo "--- '$from'"
    echo "+++ '$to'"
    echo "\$ $test"
  } >> "$log"
  if bash -c "$test" >> "$log" 2>&1; then
    echo "RESULT $name SURVIVED" >> "$log"
  else
    echo "RESULT $name KILLED" >> "$log"
  fi
  mv "$file.orig" "$file"
}

mutate half-range crates/ovis-terminal-server/src/streams/resync.rs \
  'skipped(from.get(), painted.paint.offset.get())' \
  'skipped(from.get(), from.get().saturating_add(painted.paint.offset.get().saturating_sub(from.get()) / 2))' \
  'cargo test -q --locked -p ovis-terminal-server --test streams a_real_stalled_viewer_reports_its_backlog_and_resynced_ranges'

mutate forged-label crates/ovis-terminal-core/src/subscriptions/gauge.rs \
  'health.consumers().track(client.clone(), consumer)' \
  'health.consumers().track(ClientId::new("subscription".to_owned()), consumer)' \
  'cargo test -q --locked -p ovis-terminal-core --test subscriptions health::'

grep '^RESULT' "$log"
find crates -name '*.orig' | grep . && echo "LEFTOVER BACKUP" || echo "sources restored"
