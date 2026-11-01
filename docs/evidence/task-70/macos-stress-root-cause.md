# macOS stress run 41344754982: root cause of the continuous run's six violations

Source: the run's artifact `stress-soak-macos-latest-41344754982/soak-stress-macos.log`
(`gh run download 41344754982`), line `"label": "continuous"`. Host: macOS 26.6.2,
aarch64, `available_parallelism` 3, release build. The self-test and the fixture passed on
the same runner.

## What the artifact says

- `error`: `{"kind": "io", "message": "stat the run root failed"}`.
- `samples: 385`, `control_boots: 3`, `downtimes_ms: [core_crash 572, planned_restart 863]`,
  `planned_restart_exit_codes: [0]`. The run had been going for about 385 s, through a
  crash recovery and a planned handover, when it stopped. It did not fail at the start:
  `elapsed_ms: 0` was a reporting defect (elapsed was only recorded on success).
- Both recoveries: `survived 50, exact 50, gaps 0`; `losses: []`, `identity_changes: 0`,
  `offset_regressions: 0`, continuity `holes 0` (1 resume); `pongs 384`, `control_max_ms
  117`, `late_pongs 1`; `max_queued_bytes 54759` of 262144.

## Classification

| rule | class | cause |
|---|---|---|
| `run_error` (`io`) | (c) harness | The per-second disk walk lists the run root, then stats each entry. During a live run entries vanish in between: staged checkpoints are renamed into place, and sockets are removed at a handover. The walk turned that race into a run failure. It also reported only `io` with an operation, no path and no cause. |
| `incomplete_duration` (observed 0) | (c) harness | A consequence of `run_error`, misreported as 0 ms although 385 samples were taken. |
| `rss_windows` | (c) consequence | The final window was never reached. |
| `dedicated_holder_loss` (observed `""`) and `shared_holder_isolation` (0/0) | (c) consequence | The probe never ran; the report substituted an empty default probe instead of saying so. |
| `control_unavailable` (`117 ms max, 1 late`) | (c) harness | The latency budget held: 384 answered pings, max 117 ms against 1000 ms. On macOS the harness read RSS with one `ps` process per PID (53 per second on 3 cores), and pongs were only read between samples. A pong that arrived during that sampling burst sat unread until the next tick and was counted late. That measures the harness's own stall, not the control process. |
| product (a) | none | Every recovery: all 50 survived under their first PID and start token, exact screens, no gap, no output-offset regression, no sequence hole. |
| budget (b) | none | The observed control latency is 117 ms on a 3-core shared runner, 8.5x under the 1000 ms bound; no host-derived budget is needed. |

## Structural fixes (commit after 0dd0057)

1. `holders::stat_now`/`disk_bytes`: an entry that vanished between listing and stat
   (`NotFound`), or a directory that vanished before listing, counts as gone. Every other
   failure is a `SoakError::PathIo { operation, path, source }`. Test:
   `a_run_root_entry_that_vanished_is_gone_not_a_failure`.
2. `report::error_json` adds `causes` (the full `source()` chain) to the typed
   `kind`/`message`/`hint`, and `PathIo`'s message names the path, so the artifact explains
   itself.
3. `Run::elapsed` is updated every tick, and an interrupted run reports how long it
   actually ran.
4. `verdict::Probe::ran`: a requested probe that never ran is one rule,
   `holder_loss_probe_not_run`, not two misleading isolation failures. This is asserted in
   `the_probe_must_lose_only_the_dedicated_terminal`.
5. Observer lines carry their arrival time, stamped by the reader thread. A pong's latency
   is arrival minus send. Each tick first accounts the lines that already arrived, then
   judges and sends the ping, and only then samples.
6. macOS RSS is read with one `ps -o pid=,rss= -p <pids>` per group instead of one per
   PID. The parser is platform-neutral and tested
   (`one_ps_listing_sums_the_asked_pids_and_counts_the_missing`).
