Loaded runs of the assembly restart and update tests (scripts in this directory: load25.sh, upd25b.sh, gdbprobe.py, gdbfresh.py, gdbquery.py).

1. Restart fix, recipe `taskset -c 0-1` + 8 busy loops, 25 runs of restart::a_planned_restart_* and update::an_activated_update_* (heavy gate):
   restart test 25/25 passed, max 334.2 s;
   update test 23/25 passed, max 228.2 s (load-2.log: update.query deadline; load-22.log: a 10 s socket read, WouldBlock).
   Cores 0-1 were shared at the time with another lane running the same recipe (8 more busy loops plus its tests), so this load was uncontrolled and higher than the recipe.
2. Update test alone, idle cores 124-125 + exactly 8 busy loops, 25 runs: 23/25 passed, passing runs 7.3-14.1 s; both failures (before-7.log, before-12.log) are the first update.query (update.rs:62) exceeding the CLI's 10 s deadline.
3. The same step in isolation, same load: 25 fresh servers, terminal.run then the first update.query under gdb: 0.16-0.37 s every time, none stalled (20 fresh servers' first terminal.run: 0.16-0.42 s; 40 consecutive terminal.run on one server: 0.21-1.47 s).

4. After all six fixes, `final25bt.sh` (idle cores 124-125, exactly 8 busy loops, `RUST_BACKTRACE=1`), 25 runs of the planned-restart test and both update tests (logs in `final25/`):
   restart::a_planned_restart_keeps_the_program_and_its_lifetime_and_serves_the_successor_boot: 25/25 passed, max 1.56 s
   update::a_query_racing_a_planned_restart_is_answered_by_one_boot_or_the_next: 25/25 passed, max 13.97 s
   update::an_activated_update_restarts_control_onto_the_staged_build_and_keeps_the_program: 25/25 passed, max 1.69 s
   Earlier counts in items 1-3 predate the fixes; item 3's 0.16-0.37 s figures came from a probe that did not check exit codes and are withdrawn (the real call took 5-6 s).
