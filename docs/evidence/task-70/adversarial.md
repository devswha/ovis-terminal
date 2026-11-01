# Todo 70 adversarial pass

Each class from the todo block, what the harness does about it, and the evidence.

| Class | Behaviour | Evidence |
|---|---|---|
| Repeated interruptions | Every `remote_interruption` cuts the observer's connection and resumes it with `from_seq` = the next expected sequence of the same boot; the run clock (`Instant` taken once after the first boot, `started_unix_ms` in every checkpoint) is never reset. Sequences are accounted as received, covered by an explicit `gap` record, or holes; any hole fails `sequence_hole`. | `soak::control::tests::a_resumed_viewer_gets_an_explicit_gap_for_events_no_longer_retained`; `measured.continuity` in the 1800 s report |
| Cancel/resume | A crash or handover ends the boot; the harness reconnects a fresh observer to the next boot without touching the run clock; a ping in flight on a cut connection is forgotten, not counted late. | `measured.downtimes_ms`, `measured.control_boots` in the reports |
| Hung commands / stalled viewer | A viewer that never reads is cut at its 256 KiB queue bound with a `viewer_cut` record; publishing never blocks (bounded per-viewer queue drained by its own writer thread, no lock across I/O, V-4085); control pings keep being answered within 1000 ms. | `soak::control::tests::a_stalled_viewer_is_cut_at_its_bound_while_a_reader_misses_nothing`; `tally.viewer_cuts`, `tally.max_queued_bytes`, `tally.control_max_ms` |
| Misleading success | A run shorter than `--duration-seconds` fails `incomplete_duration`; every requested disruption must have happened (`disruption_count`); a run that errors still writes its report with `run_error` and exits 1; `--self-test` reports `counts_as_acceptance:false`; the 72 h workflow result is never pasted here (DEFERRED VERIFICATION). | `soak::tests::judging::a_complete_clean_run_passes_and_each_broken_fact_fails_its_own_rule`; the first self-test attempt in this log set (holder socket path too long) exited 1 with 10 violations instead of passing |
| Exact recovery / auto-resend (failure fixture) | The fixture passes only with `screen_exact:false`, a gap, `outcome_unknown`, and an unchanged holder input sequence across the crash boot and a later handover boot. | `soak::tests::judging::the_fixture_passes_only_without_exact_recovery_and_without_a_resend`; the fixture report |

Legacy defects enforced against (not copied):

- V-4076 (detached endless threads): every control-process thread (accept, viewer
  readers/writers, holder links, command reader) has a stop path and is joined in
  `control/shutdown.rs` / `Hub::close`; the hub test asserts `close()` returns and removes
  the socket.
- V-4077 (one failed accept ends the listener): `accept.rs` counts a failed accept,
  backs off 10 ms and keeps serving; failures are exposed as `health.accept_failures`.
- V-4085 (blocking write under a shared lock): viewer writes happen only in each
  viewer's writer thread; the reader list mutex is held for a push/take only.
- V-RT-HC-03 / EXCL-026 (success decided from status): survival comes from the holder's
  record plus the operating system's start token for the recorded PID, never from status;
  an unproven write is `outcome_unknown`, never success.
- EXCL-019 (reads that inject input): the harness reads through viewers and status
  requests only; the one input ever sent is the failure fixture's, which is the subject
  of the test and is counted (`unrequested_input` fails a happy run that sends any).
- EXCL-021 (errors as banners): every failure is a typed `kind` with a hint in the
  report and on stderr; nothing expires.
- EXCL-025 (waits that miss output during send): the harness subscribes (connects its
  observer and replays the retained ring from sequence 0) before it acts, and every
  fixture stage waits for its event, never for time.
