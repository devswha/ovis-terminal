# Task 59 adversarial pass

## Classes from the todo block

- Misleading success (stale endpoint distinct from stopped): `observability/health` happy case,
  step `fixture.stale_endpoint` leaves a dead socket at the default endpoint; the CLI
  `ovis-terminal --json server status` then reports `"state": "stale_endpoint"` with
  `"server_boot_id": null`, and after `stop` the same command reports `"state": "stopped"`
  twice in a row (no new socket, no new process). Evidence: `task-59-ovis-terminal-v1.json`.
- Hung commands (metrics deadline): unit test
  `dispatch::lanes::health::qa::metrics_answer_while_a_call_hangs_and_name_the_current_boot`
  keeps a call pending inside the measured handler and requires `server.metrics` to answer;
  every scenario metrics reply is checked with `within_ms: 2000`.
- Stale state (metrics boot_id current): every scenario status/metrics reply asserts
  `server_boot_id == "boot:1"` and `runtime_lifetime == "runtime:21"` from the hello.
- Reset injection: `--inject '{"method":"server.metrics","params":{"reset":true}}'` returns
  `invalid_params`; the following metrics read still shows
  `{"phase":"degraded","corrupted":true,"incidents":"1"}` and its latency row keeps the earlier
  calls with the refusal counted as a failure.

## Mutants (`bash docs/evidence/task-59/mutants.sh`, log `mutants.log`)

| Mutant | Change | Killed by |
|---|---|---|
| resync-never-marked | `loss.resynced = true` -> `false` | `health::qa::dropped_ranges_stay_until_resynced_and_overflow_is_counted` |
| corruption-forgotten | recording an incident clears `corrupted` | `health::qa::recovery::a_recorded_corruption_survives_every_read_and_a_reopen` |
| failures-uncounted | failed calls not counted | `health::qa::latency_is_kept_per_catalog_method_with_its_slowest_operation` |
| latency-unrecorded | `Measured` never records | `dispatch::lanes::health::qa::metrics_answer_while_a_call_hangs_and_name_the_current_boot`, `...::reset_is_invalid_params_and_erases_no_counter_or_recovery_fact` |
| queue-bytes-written | queue bytes ignore written bytes | `health::qa::a_stalled_viewer_shows_its_backlog_and_lag_until_written` |

5/5 killed by a failing test (no compile failures); sources restored after each.
