# Todo 16 reconciliation with todo 76

Base: `origin/integration` a9a4ed4, merged into `lane/t16-terminal-lifecycle` at dfa7de2.

## The conflict

Todo 16 kept the leader unreaped until the output pump published PTY EOF
(`waitid(NOWAIT)` -> `wait_until_drained` -> drop slave -> `wait_until_finished`
-> `begin_reap` -> reap). Todo 76 requires the reap to happen under the
holder-state mutex as soon as the leader exits, not gated on output completion,
with shutdown treating a reaped leader as `already_exited` while still killing
its live process group.

The two cannot both hold: a same-group descendant may keep the PTY slave open
indefinitely after the leader exits, so an EOF barrier before the reap blocks the
waiter (and with it shutdown's join) forever. That is exactly how the three
todo-76 shutdown tests timed out at dfa7de2.

## Resolution

The barrier was a means, not the contract. Todo 16's contract is: Hangup is
published on PTY EOF with the terminal's incarnation evidence (pid + start token),
it precedes Exit on the wire, and Exit is last. None of that needs the child to
be unreaped at EOF:

- The waiter is todo 76's again: `waitid(EXITED|NOWAIT)` (now retried on EINTR),
  then the reap under the state mutex, which sets `WaiterState::Reaped`, and only
  then the pending-byte drain and the slave drop. `DrainSignal::wait_until_finished`
  and `HolderState::begin_reap` are gone.
- `HolderState::record_hangup` already reads the PID and start token retained in
  `HeldTerminal` at spawn. They survive the reap, so a post-reap EOF still yields
  a correctly attributed Hangup. The core matches it against its stored process
  evidence, never against a live `/proc` lookup.
- Ordering is unchanged and structural. The pump publishes Hangup at EOF, then
  blocks on the waiter-report channel, then records the exit. Exit is published
  only after both the pump and the input writer finish, so Hangup precedes Exit
  and Exit stays last.
- `final_offset` is unchanged: the probe/pending-bytes drain still holds the slave
  open until the master is empty.
- `begin_reap` had also fenced todo 16's explicit `Holder::hangup` from addressing
  a reaped PID. With it gone, `hangup` now goes through todo 76's reap-serialized,
  incarnation-checked delivery path (`signalling::signal_with`). The group of a
  reaped leader is still addressed (the kill stages need that for orphaned
  descendants), a replaced PID never is, and the reserved-PID refusal of
  `process::signal::deliver` now also covers SIGHUP (`send_hangup`).

No wire record changed: `Hangup` keeps its fields and envelope placement. Only
its doc text drops the "pre-reap" wording.

## Final bytes after an early reap (CI on 10888e3)

CI's `each_launch_gets_its_own_cwd_and_environment_over_the_socket` received
Exit with no output at all. The hole was in `pty_io::read_chunk`. Its comment
said the next read drains what remains after a hang-up, but a `poll` that
reported POLLHUP returned `Hangup` at once, even with POLLIN set. Now the waiter
reaps immediately, and `FIONREAD` on the probe can read 0 while the child's
bytes are still in the tty flip buffer. The slave can therefore close before
those bytes reach the master. The pump then woke with POLLIN|POLLHUP and dropped
them, and `final_offset` (from the tail head) faithfully excluded them.

Fix: hang-up readiness from `poll` only marks the PTY as hung up. The pump
keeps reading. Output ends only when a read itself reports EOF/EIO, or when a
read after the hang-up finds nothing (`WouldBlock`). On Linux a master read
after the slave closed flushes the pending tty buffer work first. A PumpStop
wake (shutdown) still ends output immediately, as todo 76 requires.

Regression: `pty_io::tests::hangup_readiness_with_queued_final_bytes_returns_them_before_hangup`.
It uses a real pipe holding `final` with its writer closed, so the kernel's
`poll` reports POLLIN|POLLHUP, behind a reader whose first read finds nothing.
This reproduces the CI interleaving deterministically. With the old decision
line restored, the test fails ("queued final bytes were dropped as a hang-up").
With the fix it returns `Data("final")` and then `Hangup`.

## Platform fact: when the PTY master reports EOF (CI on 3a70728)

On Linux the master reports EOF/EIO only after the last slave descriptor
closes, so a same-group descendant holding the slave postpones Hangup until
it exits. On Darwin the master reported EOF as soon as the session leader
exited, while a same-group descendant still held the slave. macOS CI job
124498647420 received `[Hangup { pid: 5858, start_token: "lstart:..." }, offset 16]`
before the descendant was killed. The product behaved correctly there: the
Hangup carried the spawned incarnation, it preceded Exit, and Exit was last.
Only the test's Linux-only premise ("nothing published before the kill") was
wrong.

`reap_tests::hangup_after_reap_carries_spawned_incarnation_and_precedes_exit`
keeps that premise on Linux only. Elsewhere it accepts at most one Hangup
before the kill. Across both phases it requires exactly one Hangup with the
spawned pid + start token at `final_offset`, followed by a final Exit. The
product has no platform branch. Later todos (53/55) must not read Hangup as
"every descendant released the PTY" on Darwin: there it only means the session
leader's terminal hung up.

## Which test guards which invariant

| Test | Invariant |
|---|---|
| `shutdown_tests::shutdown_reports_already_exited_when_waiter_reaped_before_output_completion` | I-76: the waiter reaps without any pump EOF |
| `shutdown_tests::shutdown_kills_same_group_descendant_after_leader_is_reaped` | I-76: a reaped leader's live group is still killed, shutdown is bounded |
| `shutdown_tests::shutdown_stops_pump_when_reaped_leader_group_is_gone` | I-76: PumpStop wakes the pump when the group is gone |
| `shutdown_tests::shutdown_reports_zombie_without_signalling_before_waiter_reaps` | I-76: a zombie is already_exited, never signalled |
| `reap_tests::hangup_after_reap_carries_spawned_incarnation_and_precedes_exit` (replaces `child_is_unreaped_at_the_real_pty_hangup_barrier`) | I-16: with the leader reaped and no Hangup/Exit queued yet, the later EOF yields exactly one Hangup carrying the spawned pid + start token at `final_offset`, before an Exit with the leader's status, and nothing after Exit |
| `reap_tests::shutdown_with_descendant_holding_slave_is_bounded_and_keeps_hangup_before_exit` | I-16 + I-76: after the reap, the Hangup stage still reaches the HUP-ignoring descendant's group, shutdown returns within 2 s with every worker joined and `already_exited == 1`, any delivered lifecycle events are `[]`, `[Hangup]` or `[Hangup, Exit]`, and the exit is recorded at the tail head |
| `process::signal::tests::reserved_pids_are_refused_before_kill_is_reached` | SIGHUP joins the PID 0/1 refusal, exercised through the recording seam only |

All waits are event-driven: the `reaped` condvar, link receives bounded by
`WAIT`, and zero-timeout drains of messages already queued. There are no sleeps.
No test signals PID 0, 1 or -1: every real signal addresses a group led by a
test-spawned child.
