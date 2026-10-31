set -u
cd /home/devswha/workspace/ovis-terminal-wt/lane-t33
export ATTEMPT_DIR=/home/devswha/workspace/ovis-terminal-wt/lane-t33/.omo/evidence/ovis-terminal-v1/task-33
rm -f "$ATTEMPT_DIR/task-33-ovis-terminal-v1.json"
uname -srm
rustc --version
printf '%s\n' '$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/status --isolated --case happy --evidence "$ATTEMPT_DIR/task-33-ovis-terminal-v1.json"'
cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/status --isolated --case happy --evidence "$ATTEMPT_DIR/task-33-ovis-terminal-v1.json" 2>>"$ATTEMPT_DIR/qa-stderr.log"
printf 'EXIT=%s\n' "$?"
printf '%s\n' '$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/status --isolated --case failure --inject '"'"'{"method":"status.report","params":{"seq":"3","record":{"state":"done"}}}'"'"' --evidence "$ATTEMPT_DIR/task-33-ovis-terminal-v1.json" --append'
cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/status --isolated --case failure --inject '{"method":"status.report","params":{"seq":"3","record":{"state":"done"}}}' --evidence "$ATTEMPT_DIR/task-33-ovis-terminal-v1.json" --append 2>>"$ATTEMPT_DIR/qa-stderr.log"
printf 'EXIT=%s\n' "$?"
printf '%s\n' '$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/status --isolated --case failure --inject '"'"'{"method":"status.report","params":{"seq":3,"record":{"state":"done"}}}'"'"' --evidence "$ATTEMPT_DIR/task-33-ovis-terminal-v1.json" --append'
cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/status --isolated --case failure --inject '{"method":"status.report","params":{"seq":3,"record":{"state":"done"}}}' --evidence "$ATTEMPT_DIR/task-33-ovis-terminal-v1.json" --append 2>>"$ATTEMPT_DIR/qa-stderr.log"
printf 'EXIT=%s\n' "$?"
printf '%s\n' '$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/status --isolated --case all --evidence "$ATTEMPT_DIR/task-33-ovis-terminal-v1.json" --append'
cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/status --isolated --case all --evidence "$ATTEMPT_DIR/task-33-ovis-terminal-v1.json" --append 2>>"$ATTEMPT_DIR/qa-stderr.log"
printf 'EXIT=%s\n' "$?"
