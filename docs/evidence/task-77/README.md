# Task 77 evidence: op.cancel before the durable accept

| File | Command | Exit |
|---|---|---|
| `before-ops-cancel.log` | the new `ops_cancel` tests run on unmodified `origin/integration` (b280ac9) | 101 |
| `check.log` | `bash scripts/check.sh` | 0 |
| `qa-happy.log` | `cargo test -p ovis-terminal-server --test ops_cancel happy -- --exact --nocapture` | 0 |
| `qa-failure.log` | `cargo test -p ovis-terminal-server --test ops_cancel failure -- --exact --nocapture` | 0 |
| `adv-ordering.log` | `cargo test -p ovis-terminal-server --test ops_cancel cancel_race -- --nocapture` | 0 |
| `adv-stale-state.log` | `cargo test -p ovis-terminal-core --test ops pending -- --nocapture` | 0 |
| `conformance.log`, `conformance.json` | `cargo run -p ovis-terminal-conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-77 --isolated --case all --evidence docs/evidence/task-77/conformance.json` | 0 |
| `windows-check.log` | `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` | 0 |

Host: Linux x86_64. The PR body (`pr-body.md`) states what each test asserts.
