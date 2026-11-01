#!/usr/bin/env bash
# Applies each mutant to a backed-up copy of one source file, runs the shipped lane tests,
# restores the file, and records whether a test failed (the mutant must be killed).
set -uo pipefail
cd "$(dirname "$0")/../../.."
log=docs/evidence/task-59/mutants.log
: > "$log"
mutate() {
  local name=$1 file=$2 from=$3 to=$4 pkg=$5
  cp "$file" "$file.orig"
  python3 - "$file" "$from" "$to" <<'P'
import sys
path, old, new = sys.argv[1:]
text = open(path).read()
assert text.count(old) == 1, f"mutant anchor not unique in {path}: {old!r}"
open(path, "w").write(text.replace(old, new))
P
  echo "### $name: $file: '$from' -> '$to'" >> "$log"
  if cargo test -q -p "$pkg" --lib health:: >> "$log" 2>&1; then
    echo "RESULT $name SURVIVED" >> "$log"
  else
    echo "RESULT $name KILLED" >> "$log"
  fi
  mv "$file.orig" "$file"
}
mutate resync-never-marked crates/ovis-terminal-core/src/health/consumers.rs \
  'Some(loss) => loss.resynced = true,' 'Some(loss) => loss.resynced = false,' ovis-terminal-core
mutate corruption-forgotten crates/ovis-terminal-core/src/health/recovery.rs \
  'journaled.corrupted |= damaged(incident.kind);' 'journaled.corrupted = false;' ovis-terminal-core
mutate failures-uncounted crates/ovis-terminal-core/src/health/latency.rs \
  'row.failures.saturating_add(u64::from(settled.failed))' 'row.failures' ovis-terminal-core
mutate latency-unrecorded crates/ovis-terminal-server/src/dispatch/lanes/health.rs \
  'self.health.latencies().record(settled);' 'drop(settled);' ovis-terminal-server
mutate queue-bytes-written crates/ovis-terminal-core/src/health/consumers.rs \
  'ByteCount::new(self.queued.saturating_sub(self.written))' 'ByteCount::new(self.queued)' ovis-terminal-core
grep '^RESULT' "$log"
git status --short -- crates | grep '\.orig' && echo "LEFTOVER BACKUP" || echo "sources restored"
