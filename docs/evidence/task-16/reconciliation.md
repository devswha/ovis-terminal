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
