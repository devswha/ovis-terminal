#!/usr/bin/env bash
# The four gate-review mutants: each is applied to a backed-up source file, the named
# shipped tests run, the file is restored, and the mutant must make a test fail.
set -uo pipefail
cd "$(dirname "$0")/../../.."
log=docs/evidence/task-59/mutants-r2.log
: > "$log"
mutate() {
  local name=$1 file=$2 from=$3 to=$4
  shift 4
  cp "$file" "$file.orig"
  python3 - "$file" "$from" "$to" <<'P'
import sys
path, old, new = sys.argv[1:]
text = open(path).read()
assert text.count(old) == 1, f"mutant anchor not unique in {path}: {old!r}"
open(path, "w").write(text.replace(old, new))
P
  echo "### $name: $file" >> "$log"
  echo "--- '$from'" >> "$log"
  echo "+++ '$to'" >> "$log"
  local killed=0
  while [ $# -gt 0 ]; do
    echo "\$ $1" >> "$log"
    if ! bash -c "$1" >> "$log" 2>&1; then killed=1; fi
    shift
  done
  if [ "$killed" = 1 ]; then echo "RESULT $name KILLED" >> "$log"; else echo "RESULT $name SURVIVED" >> "$log"; fi
  mv "$file.orig" "$file"
}
scenario='cargo build -q --locked -p ovis-terminal-cli --bin ovis-terminal && cargo run -q --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario observability/health --isolated --case all --evidence target/mutant-health.json'

mutate record-reads-in-the-ledger crates/ovis-terminal-server/src/dispatch/lanes/health.rs \
  'let observed = Observed::new(EventSeq::new(snapshot.revision().get()), quarantined);' \
  'let _ = self.health.recovery().record(ovis_terminal_core::recovery::Finding::new(ovis_terminal_protocol::domain::recovery::IncidentKind::StoreCorrupted, Vec::new(), None)); let observed = Observed::new(EventSeq::new(snapshot.revision().get()), quarantined);' \
  'cargo test -q --locked -p ovis-terminal-server --lib dispatch::lanes::health::'

mutate key-metrics-by-socket crates/ovis-terminal-core/src/health/consumers.rs \
  'if tracked.clone().any(|line| line.consumer == consumer) {' \
  'if tracked.clone().any(|line| line.client_id == client_id) {' \
  'cargo test -q --locked -p ovis-terminal-core --lib health::' \
  'cargo test -q --locked -p ovis-terminal-server --test streams a_real_stalled'

mutate auto-start-on-health crates/ovis-terminal-cli/src/invocation/execute.rs \
  'Ok(Err(Failure::Cli(CliError::NotRunning { state, .. }))) => return idle_status(state),' \
  'Ok(Err(Failure::Cli(CliError::NotRunning { state, path }))) => { if let Some(dir) = path.parent() { let _ = std::fs::create_dir_all(dir); } std::mem::forget(std::os::unix::net::UnixListener::bind(&path)); return idle_status(state) }' \
  'cargo test -q --locked -p ovis-terminal-cli --test cli happy' \
  "$scenario"

mutate drop-the-cardinality-cap crates/ovis-terminal-core/src/health/consumers.rs \
  'if lines.len() >= MAX_CONSUMERS {' \
  'if lines.len() >= usize::MAX {' \
  'cargo test -q --locked -p ovis-terminal-core --lib health::'

grep '^RESULT' "$log"
find crates -name '*.orig' | grep . && echo "LEFTOVER BACKUP" || echo "sources restored"
