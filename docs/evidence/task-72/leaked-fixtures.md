# Leaked fixtures during task-72 runs

Two `ovis-terminal-probe-holder serve` processes (pids 1360984, 1361577; started
by the `happy` probe, not by the stall probe) were found alive ~6 minutes after
the heavy exploratory run (400 runs, 32-way, single-read revert). That run was
aborted by this lane with `pkill -f stress.sh; pkill -f cargo-nextest`, killing
nextest mid-test; the holder fixtures `happy` had spawned were orphaned.
Cause: the lane killed test processes mid-way, not a leak on a normal pass: the
completed 50-run, 50-run and 200-run stress batches left no fixture behind
(checked with pgrep before this cleanup only for these two pids; no
`probe-stall` fixture was ever found). Both were killed manually.
The aborted exploratory log was discarded; it is not part of the evidence.
