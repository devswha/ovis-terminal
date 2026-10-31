## Resumed executor attempt

The same required command was executed again after lifecycle integration.
Its output and exit code are preserved verbatim below; the pre-existing
executor-attempt JSON is not runner output.

```text
status: exited_2 exit_code: 2
    Blocking waiting for file lock on build directory
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 8.60s
     Running `target/debug/conformance --binary target/debug/ovis-terminal --scenario part-a/task-16 --isolated --case happy --evidence /home/devswha/workspace/ovis-terminal-wt/lane-t16/.omo/evidence/ovis-terminal-v1/task-16/task-16-ovis-terminal-v1.json`
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
CONFORMANCE_EXIT=2
Linux x86_64
```

DEFERRED VERIFICATION (runner lands in Todo 21). No CLI/socket scenario ran.
