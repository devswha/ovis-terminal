## Before

RT-HD-06, GAP-052, GAP-053, GAP-054, GAP-055, GAP-067, GAP-068, GAP-069: `/tmp/ulw-ovis-yCJzPH/evidence/smoke/herdr-h04-send-read.log:6`, durable copy `docs/evidence/baseline/smoke/herdr-h04-send-read.log:6`:

```text
<- {"id":"r9","result":{"type":"pane_read","read":{...,"source":"visible","format":"text","text":"...","revision":0,"truncated":false}}}
```

The only read surface is `source: "visible"` with a bare `revision: 0` and `truncated: false`: no history source, no completeness, no retention floor, no live-bottom state and no pinned revision.

## After

`ovis_terminal_core::screen::Screen` connects ordered holder `Output` messages to the VT engine (todo 7) for one terminal incarnation:

- **Ingest:** holder bytes are fed in `work_budget` slices (default 64 KiB). The lock is released between slices, so reads interleave with a busy producer. Each slice checks the terminal target and offset continuity: a wrong incarnation returns `target_mismatch`, a non-contiguous offset returns `output_gap`, and output after `finish(Exit)` returns `terminal_ended`.
- **Raw output and checkpoints:** a bounded raw ring (default 8 MiB) has a visible `retained.floor`. A VT checkpoint is taken every `checkpoint_interval` bytes and reported as `ScreenTransition::Checkpoint`. Config validation guarantees that the newest checkpoint stays inside the retained range.
- **Query replies:** terminal-query replies go to a bounded internal reply lane (`drain_replies`). It is a separate type from user input, each reply is taken exactly once, and overflow is counted in `dropped`.
- **OSC records:** they are returned to the caller unchanged. The screen derives no status from screen text or OSC bytes.
- **Reads:** `screen`, `history`, `search` and `selection` answer from immutable snapshots pinned by content revision (LRU, default 8). Every result carries `source`, `format`, `revision`, `offset`, `live{revision, offset, at_live_bottom}`, `history` (available with completeness, or unavailable with reason and recovery), `retained{floor, head}` and `fidelity`. History pages also carry `requested`, `returned`, `total_lines`, `truncated` and a `next` cursor at the same revision. A revision that is neither live nor pinned returns `stale_revision{requested, current}`, never an empty success. Reads without a revision never fail as stale.
- **`output`:** reads raw bytes by offset. An evicted start returns `history_gap{requested, retained, checkpoint}`, where `checkpoint` is the newest checkpoint still inside the retained range, or `null` when none covers it.
- **History availability:** while the alternate screen is active, `history` returns `history_unavailable{reason: alternate_screen, recovery: history_available_transition}`, and ingestion emits the named `HistoryAvailable` transition when the program leaves it. If the program exits while full-screen, the reason becomes permanent (`ended_in_alternate_screen`, `recovery: permanent`).
- **Wire format:** every u64 counter on the wire (revision, offset) is a canonical decimal string. `decode_request` rejects numeric or non-canonical counters, unknown fields, an empty needle and a zero limit with `invalid_params`.

Executed on Linux x86_64 (kernel 7.0.0-38-generic), rustc 1.99.1 (58cf8f9c3 2026-10-13):

```sh
cargo test -p ovis-terminal-core --test screen happy -- --exact --nocapture     # EXIT=0, 1 test ran
cargo test -p ovis-terminal-core --test screen failure -- --exact --nocapture   # EXIT=0, 1 test ran
cargo nextest run -p ovis-terminal-core                                          # EXIT=0, 106 passed
cargo clippy -p ovis-terminal-core --all-targets -- -D warnings                  # EXIT=0
bash scripts/check.sh                                                            # CHECK_EXIT=0, workspace nextest 447 passed, 1 skipped
```

DEFERRED VERIFICATION (runner lands in Todo 21)

```sh
ATTEMPT_DIR=/home/devswha/workspace/ovis-terminal-wt/lane-t17/.omo/evidence/ovis-terminal-v1/task-17
cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-17 --isolated --case happy --evidence "$ATTEMPT_DIR/task-17-ovis-terminal-v1.json"
```

```text
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
EXIT=2
Linux x86_64
```

No CLI or socket request was dispatched and no runner evidence JSON was produced; the `terminal.history`/`search`/`selection`/`output` routes are wired by later todos (23/31). `conformance/scenarios/part-a/task-17.json` records the expected fields and bytes for the run after assembly. The full log is in `docs/evidence/task-17/conformance.log`.

Parsed component output of the happy case (`docs/evidence/task-17/qa-happy.log`). This is not a CLI transcript:

```json
{
  "first_page":  {"meta": {"revision": "1098", "offset": "1101", "live": {"revision": "1098", "offset": "1101", "at_live_bottom": true},
                  "history": {"state": "available", "completeness": {"state": "complete"}}, "retained": {"floor": "0", "head": "1101"},
                  "source": {"kind": "emulator"}, "format": "text", "fidelity": {"fidelity": "exact"}},
                  "requested": {"first": 0, "count": 50}, "returned": {"first": 0, "count": 50}, "next": {"revision": "1098", "line": 50}},
  "second_page": {"meta": {"revision": "1098", "offset": "1101", "live": {"revision": "1452", "offset": "1455", "at_live_bottom": false},
                  "retained": {"floor": "0", "head": "1455"}}, "returned": {"first": 50, "count": 50}, "truncated": true},
  "search_revision": "1098", "search_hits": 11, "selection": "line 0\nline 1\n",
  "viewer_bytes": 1455, "final_offset": "1455", "exit_code": 3
}
```

Failure case output (`docs/evidence/task-17/qa-failure.log`):

```json
{"history_gap":"history_gap","retained":{"floor":"466","head":"530"},"checkpoint":{"offset":"512","revision":"512","fidelity":{"fidelity":"exact"}},
 "stale":{"kind":"stale_revision","requested":"530","current":"543"},"output_gap":"output_gap"}
```

## QA

- **happy** (`tests/screen.rs::happy`, real holder PTY on Unix). A real `/bin/sh` program prints 120 lines, a DSR query and `READY`, then blocks on `read`. A pump thread ingests every holder message, and an independent viewer thread follows raw output by cursor through `Screen::output`. The test reads history page 1 (limit 50), releases the program through the holder input path, waits for its holder `Exit`, then reads page 2 from the cursor. It asserts:
  - both pages report the same revision, with `second.meta.live.revision > first.meta.revision`;
  - `first.live.at_live_bottom == true` and `second.live.at_live_bottom == false`;
  - page 2 starts where page 1 ended, and page 1 starts with `line 0`;
  - search at the pinned revision finds at least 11 hits, and selection of lines 0..2 is `"line 0\nline 1\n"`;
  - the viewer received exactly `final_offset` bytes, including all 40 `tick` lines printed after the pinned read;
  - exactly one DSR reply (`ESC [ ... R`) is in the reply lane, and a second drain is empty.
- **failure** (`tests/screen.rs::failure`). With 64 bytes of retention, a read at offset 0 returns `history_gap`, with `retained.floor == head - 64` and a checkpoint inside the range. With pin capacity 1, history, search and selection at an evicted revision each return `stale_revision` with `requested` and the current revision. A chunk one byte past the head returns `output_gap` and leaves the offset unchanged.
- **Wire tests** (`wire::*`):
  - `u64::MAX` revision and offset round trip exactly as decimal strings;
  - numeric, leading-zero, signed or unknown-field counters, an empty needle and a zero limit are each `invalid_params`;
  - an encoded read carries revision and offset as strings;
  - a proptest checks that every chunk split of a stream (text, UTF-8, SGR, alternate screen, DSR) with a 3-byte work budget yields the same lines, revision, offset, raw bytes, replies and search hits as one whole-chunk ingest.
- **TDD and mutation evidence:**
  - `tdd-red.log` records the red step, where the suite failed to compile before the module existed.
  - `mutation-sanity.log` records two mutations. Forcing `at_live_bottom = true` fails `happy`. Disabling pin lookup fails `happy` and the reflow test.

## Adversarial

`docs/evidence/task-17/adversarial.log`: `cargo test -p ovis-terminal-core --test screen -- adversarial:: wire:: --nocapture` (EXIT=0, 11 passed).

- **Stale state: reflow.** A pinned page continuation after a resize is served at the old revision, and `live.revision` equals the post-resize revision.
- **Stale state: eviction.** Covered by `failure`: evicted output returns `history_gap`, and an evicted pin returns `stale_revision`.
- **Prompt injection.** Text reading `history_available status=idle` plus an OSC 7501 status record produces no transition. The OSC record is only forwarded, and the text stays plain screen content.
- **Hung commands / busy producer.** A thread ingests continuously while 20 page pairs are read. Every pinned continuation succeeds at its first page's revision, and revisions are non-decreasing.
- **Alternate screen.** `history_unavailable` names `history_available_transition`, and leaving the alternate screen emits `HistoryAvailable`. Exiting while full-screen is permanent, and later output is `terminal_ended`.
- **Foreign incarnation.** Output for another terminal is `target_mismatch` and leaves the screen empty.
- **Reply lane.** With capacity 2, three DSR queries queue two replies once and count one as dropped. A second drain is empty with `dropped == 0`.

## Contract checklist

1. Every u64 revision and offset in emitted or accepted JSON is a canonical decimal string. The VT newtypes `ContentRevision`/`OutputOffset` are reused. Numeric forms return `invalid_params`, and the `u64::MAX` round trip is tested. Non-u64 counts (`line`, `limit`, `dropped`) are u32 JSON numbers.
2. `ScreenError` is a single `#[derive(thiserror::Error, Debug)]` enum with `kind()` and `hint()`. `InvalidParams` carries `#[source] serde_json::Error`. Display text never repeats a kind token.
3. Identities reuse the landed newtypes: holder `TerminalTarget`, VT `ContentRevision`/`OutputOffset`/`LineSpan`. `PageCursor` and `Needle` are new semantic types. `Option` is used only for real absence (`next`, `checkpoint`, `cursor`).
4. Tests assert behavior: transitions, typed error kinds, round trips under every chunk split (proptest), and pinned revisions against live revisions. `mutation-sanity.log` shows the tests failing when behavior is broken.
5. No `#[allow]`, no blanket `#[expect]`, no TODO placeholders, no prose-pinning tests. `workspace-hygiene` reports 0 violations (250 pure LOC, at most 3 parameters).
6. The conformance attempt is recorded verbatim above under DEFERRED VERIFICATION.

The holder wire stays at protocol generation 2, and the contract checklist items are unchanged.

## Seams

- **New public module `ovis_terminal_core::screen`.** It fills the existing todo-1 slot `src/screen.rs`. It adds `Screen`, `ScreenConfig`, `ScreenBounds`, `ScreenError`, the request types (`HistoryRequest`, `SearchRequest`, `Needle`, `SelectionRequest`, `OutputRequest`, `PageCursor`, `decode_request`), the result types (`ScreenView`, `HistoryPage`, `SearchResult`, `SelectionText`, `OutputSlice`, `ReadMeta`, `LiveState`, `RetainedRange`, `CheckpointMark`, `HistoryState`, `Unavailability`, `UnavailableReason`, `Recovery`, `ScreenTransition`, `Ingested`) and the reply-lane types (`QueryReply`, `ReplyBatch`).
- **No landed crate's public API changed.** No Cargo manifest changed.
- **Kind tokens.** The screen boundary uses `history_gap` and `stale_revision`, as the plan's QA names them. The landed protocol catalog names the matching RPC errors `offset_evicted` and `revision_mismatch`. Dispatch (todos 23/31) maps between them; that mapping is not part of this PR.
- **Write-scope exceptions.** `.github/workflows/probe-stress-macos.yml` gains a `screen` input and job that run `tests/screen.rs` 100 times on macOS, following the todo-73 pattern as the brief requested. `docs/evidence/task-17/` is the committed evidence directory.
- **Integration seam.** `terminals::Terminals::observe` still consumes holder `Output` for lifecycle only. Routing those messages into `Screen::ingest`, and draining the reply lane into the holder input, is the integration step of the attach and assembly todos (23/31). It lies outside this todo's write scope.

## Risks

- **Read cost.** Each pinned snapshot copies the scrollback text, O(history lines), while holding the screen lock. At most `pinned_snapshots` copies are kept. Each read at a new revision captures one copy, which is bounded by the VT history limit but has not been benchmarked. Measurement belongs to todo 56/69.
- **Burst output.** Under heavy output, more distinct revisions than the pin capacity evict older pins, so a slow pager can receive an honest `stale_revision`. Reads without a revision never fail this way.
- **Recovery.** Checkpoints are kept in memory only (`Screen::checkpoint`). Persistence and restore from checkpoint plus holder tail belong to todo 49.
- **Wire format of raw bytes.** `OutputSlice.bytes` serializes as a JSON byte array. The RPC layer's base64 form (`data_b64`) is produced at dispatch.
- **macOS.** Darwin's early-EOF timing does not affect these tests, because the happy test waits for the holder `Exit`, not for PTY EOF.


## macOS stress

`gh workflow run probe-stress-macos.yml --ref lane/t17-holder-screen-history -f probe=screen` started run https://github.com/devswha/ovis-terminal/actions/runs/41194300652, which concluded `success`. Its `screen` job repeated `cargo nextest run -p ovis-terminal-core --test screen --locked --no-tests=fail` 100 times on macos-latest. Log excerpt:

```text
     Summary [   0.780s] 13 tests run: 13 passed, 0 skipped
     Summary [   0.547s] 13 tests run: 13 passed, 0 skipped
     Summary [   0.569s] 13 tests run: 13 passed, 0 skipped
PASS 100/100
RESULT 100/100
```

PR CI on head 446baee: every check passes, including test (ubuntu/macos/windows), probe (ubuntu/macos/windows), scripts/check.sh, msrv 1.89, rustdoc, cargo deny and cargo-machete.
