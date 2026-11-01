# check.sh failure on this host: inherited lock descriptor (host tooling, not lane code)

`bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` exited 100
(`docs/evidence/task-58/check.log`): fmt and workspace clippy passed; nextest stopped fail-fast
after `ovis-terminal-holder::platform_probe failure` (805/1210 run, 804 passed, 1 failed):

```
exit status: 127: /dev/shm/ovt-target-lane-t58/debug/ovis-terminal-probe-holder: error while loading shared libraries: libc.so.6: cannot open shared object file: Error 24
```

The test (`probes/platform/tests/unix_failure.rs`, `exhausted_descriptors_are_unavailable`, last
changed in 1e397d6, untouched by this lane) runs the dynamically linked probe holder under
`ulimit -n 4`. The host cargo shim introduced on 2026-11-01 runs every `cargo test`/`nextest` as
`flock -w 7200 /dev/shm/ovt-test-lane.lock systemd-run ... cargo`; without `-o`, flock(1) keeps
the lock descriptor open in the command, so every test process inherits fd 3 and the loader has
no descriptor left for libc.

Reproduction outside cargo, same probe binary (lane-t58 RAM target):

```
== without flock:            exit=0   (one SpawnFailed{kind:unavailable} frame, as the test expects)
== fds seen by a child under flock (no -o):  0 1 2 3
== under flock (no -o):       libc.so.6: cannot open shared object file: Error 24, exit=127
== under flock -o:            exit=0
```

Fix (host tooling, outside this lane): `flock -o -w 7200 ...` in
`/home/devswha/.ovis-toolchain/shim/cargo`; the flock parent keeps holding the lock.
