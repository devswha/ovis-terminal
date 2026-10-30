## Real-socket conformance is not available on this base

`conformance/src/main.rs` states that the real-socket runner is implemented by todo 21; the current scaffold registers no scenarios and exits with status 2. `crates/ovis-terminal-cli/src/main.rs` currently handles only `--version` and parse errors. Todo 31 assembles the runtime binary later.

Therefore the required `part-a/task-3` socket transcript cannot be produced from `origin/integration` yet. No conformance pass is claimed and no synthetic task-3 conformance result is supplied.
