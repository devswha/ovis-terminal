set -u
cd /home/devswha/workspace/ovis-terminal-wt/lane-t36
export ATTEMPT_DIR=/home/devswha/workspace/ovis-terminal-wt/lane-t36/.omo/evidence/ovis-terminal-v1/task-36
rm -f "$ATTEMPT_DIR/task-36-ovis-terminal-v1.json"
uname -srm
rustc --version
sha256sum target/debug/ovis-terminal
run() {
    printf '$ %s\n' "$*"
    "$@" 2>>"$ATTEMPT_DIR/qa-stderr.log"
    printf 'EXIT=%s\n' "$?"
}
run cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/binding --isolated --case happy --evidence "$ATTEMPT_DIR/task-36-ovis-terminal-v1.json"
run cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/binding --isolated --case failure --inject '{"method":"agent.conversation","params":{"cursor":{"history_id":"h1","revision":"1","position":"0"}}}' --evidence "$ATTEMPT_DIR/task-36-ovis-terminal-v1.json" --append
run cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/binding --isolated --case failure --inject '{"method":"agent.conversation","params":{"cursor":{"history_id":"h1","revision":1,"position":"0"}}}' --evidence "$ATTEMPT_DIR/task-36-ovis-terminal-v1.json" --append
run cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/binding --isolated --case all --evidence "$ATTEMPT_DIR/task-36-ovis-terminal-v1.json" --append
run cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario all/all --isolated --case all --evidence "$ATTEMPT_DIR/all-all.json"
