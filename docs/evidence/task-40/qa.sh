set -u
ATTEMPT_DIR=/home/devswha/workspace/ovis-terminal-wt/lane-t40/.omo/evidence/ovis-terminal-v1/task-40
B="cargo run -q --locked --manifest-path conformance/Cargo.toml --bin conformance --"
cargo build -q --locked -p ovis-terminal-cli --bin ovis-terminal
sha256sum target/debug/ovis-terminal > "$ATTEMPT_DIR/binary.sha256"
$B --binary target/debug/ovis-terminal --scenario agent/bridge --isolated --case all --evidence "$ATTEMPT_DIR/task-40-all.json" > "$ATTEMPT_DIR/all.stdout" 2> "$ATTEMPT_DIR/all.stderr"; echo "ALL_EXIT=$?"
$B --binary target/debug/ovis-terminal --scenario agent/bridge --isolated --case happy --manual-transcript --evidence "$ATTEMPT_DIR/task-40-ovis-terminal-v1.json" > "$ATTEMPT_DIR/happy.stdout" 2> "$ATTEMPT_DIR/happy.stderr"; echo "HAPPY_EXIT=$?"
$B --binary target/debug/ovis-terminal --scenario agent/bridge --isolated --case failure --manual-transcript --inject '{"method":"prompt.open","params":{"process_token":"other-process","prompt":{"kind":"form","mechanism":"extension"}}}' --evidence "$ATTEMPT_DIR/task-40-ovis-terminal-v1.json" --append > "$ATTEMPT_DIR/failure.stdout" 2> "$ATTEMPT_DIR/failure.stderr"; echo "FAILURE_EXIT=$?"
$B --binary target/debug/ovis-terminal --scenario all/all --isolated --case all --evidence "$ATTEMPT_DIR/task-40-suite.json" > "$ATTEMPT_DIR/suite.stdout" 2> "$ATTEMPT_DIR/suite.stderr"; echo "SUITE_EXIT=$?"
