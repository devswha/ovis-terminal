Plan todo 8: atomic SQLite storage and quarantine primitives in `crates/ovis-terminal-store`.
Closes GAP-125. Delivers the store slice of IDEAL-097, IDEAL-098 and IDEAL-101.

## What changed

- `Store::open` probes an existing `state.db` before writing anything. It checks the
  SQLite header magic, `user_version` and `PRAGMA quick_check`, then switches to WAL with
  `synchronous=FULL` and applies versioned migrations (`SCHEMA_VERSION = 1`).
- A dedicated writer thread sits behind a bounded `sync_channel` (`writer_queue`, default 64).
  It commits each `Mutation` in one `BEGIN IMMEDIATE` transaction: compare-and-swap resource
  writes, their events and the operation receipt. The `Ticket` resolves only after `COMMIT`
  returns. A full queue answers `backpressure`; an oversized payload answers `too_large`
  before anything is queued.
- `Snapshot` is a read transaction pinned to one revision (WAL snapshot). It offers
  `resource`, `receipt`, `events_after` and `recovery_evidence`.
- Retention: when events or receipts exceed the advertised limits, the oldest are evicted
  and the stored floor advances. Reads report `EventPage::Expired { floor }` /
  `ReceiptLookup::NotRetained { floor }` instead of an empty success. Deleting a resource
  leaves a versioned `Tombstone`.
- Quarantine:
  - A damaged database writes durable `evidence.json` first, then moves `state.db`,
    `-wal` and `-shm` into `quarantine/<id>/`. The open returns `store_corrupted`.
  - A newer schema is copied there and left in place. The open returns `store_unsupported`.
  - Later opens keep returning the same typed error; they never produce an empty store.
  - Reading the evidence never clears it.
  - `Store::rebuild_after_quarantine` is the explicit path to an empty store, and it carries
    the evidence into the new database.
  - Every incident records `process_survival: Unknown` and
    `relaunch: RequiresIndependentEvidence`; the store never decides that programs survived.
- `CommitBarrier::{Applied, Committed}` hooks are the named fault-injection barriers for
  crash tests.

## Before

RT-HD-05 / GAP-125: `docs/evidence/baseline/smoke/attempt2-herdr-h03-workspace.log:5` shows
`workspaces before: 1`: herdr silently restored state from a previous server
(source `<run-root>/evidence/smoke/attempt2-herdr-h03-workspace.log:5`). Under GAP-125, herdr
also replaces corrupt or newer persisted state with healthy-looking defaults: corrupt
`docs.json` becomes `docs.json.bad` and the core starts empty (CAT-3481,
`herdr-core/tests/docs.rs:48`).

## After

The plan's `After` command is not runnable yet. The `conformance` binary is still the
todo-1 scaffold (it registers no scenarios and exits 2), and `ovis-terminal` only answers
`--version`. The real-socket run of `part-a/task-8` therefore waits for todo 21 (runner)
and todo 31 (binary assembly). `conformance/scenarios/part-a/task-8.json` records the
component cases those todos will rerun. Here is what did run, on
`Linux 7.0.0-38-generic x86_64`, `rustc 1.99.1`, bundled SQLite `3.54.1`
(libsqlite3-sys 0.39.1, rusqlite 0.41.1):

| Command | Exit | Result |
|---|---|---|
| `bash scripts/check.sh` | 0 | fmt, clippy `-D warnings`, nextest 29 passed / 1 skipped (the ignored crash-child fixture), doctests, hygiene 0 violations in 69 files, manifests, JSON |
| `cargo nextest run -p ovis-terminal-store` | 0 | 18 passed, 1 skipped |
| `cargo clippy -p ovis-terminal-store --all-targets -- -D warnings` | 0 | clean |
| `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --all-features --workspace --locked` | 0 | clean |

The first CI run (41109382393) was red only on `test (windows-latest)`. Clippy rejected
the non-unix no-op `sync_dir` stub (`missing_const_for_fn`, `unnecessary_wraps`). The fix
commit defines `sync_dir` only on Unix and calls it under `#[cfg(unix)]`. Windows has no
directory fsync and NTFS journals renames, so no stub remains and no `#[allow]` was added.
After the fix, Windows clippy passes in CI; the run result is recorded in the PR conversation.

Logs: `docs/evidence/task-8/check.log`, `qa-happy.log`, `qa-failure.log`,
`mutation-probes.log`, `tdd-red.log` (the failing run before implementation),
`platform.txt`, `task-8-ovis-terminal-v1.json`.

## QA

Happy: `cargo test -p ovis-terminal-store --test transactions happy -- --exact --nocapture`,
exit 0. A child process commits one mutation and `abort()`s at `CommitBarrier::Committed`;
the parent then reopens the store:

```
reopened after crash: revision=1 resource=Live { version: Version(1), value: Payload([118, 49]), revision: Revision(1) } receipt=Found { revision: Revision(1), result: Payload([111, 112, 45, 49]) }
cas: committed=CommitReceipt { revision: Revision(2), first_event: Revision(2), versions: [(ResourceKey { kind: "doc", id: "app" }, Version(2))] } conflict.kind=conflict resource=Live { version: Version(2), value: Payload([118, 50]), revision: Revision(2) }
test happy ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out; finished in 0.16s
```

Failure: `cargo test -p ovis-terminal-store --test transactions failure -- --exact --nocapture`,
exit 0. It covers both a corrupt header and a future schema (`user_version = 999`):

```
Header: kind=store_corrupted ... evidence=RecoveryEvidence { id: "1793385736689-00", cause: CorruptHeader, location: "/tmp/ovis-store-failure-3157073-0/quarantine/1793385736689-00", preserved: ["state.db"], detected_at_unix_ms: Some(1793385736689), flags: RecoveryFlags { material: MovedToQuarantine, process_survival: Unknown, relaunch: RequiresIndependentEvidence } }
FutureSchema: kind=store_unsupported ... evidence=RecoveryEvidence { id: "1793385736694-00", cause: UnsupportedSchema { found: 999, supported: 1 }, location: "/tmp/ovis-store-failure-3157073-1/quarantine/1793385736694-00", preserved: ["state.db"], detected_at_unix_ms: Some(1793385736694), flags: RecoveryFlags { material: CopiedOriginalKept, process_survival: Unknown, relaunch: RequiresIndependentEvidence } }
test failure ... ok
```

The failure test also asserts:
- the quarantined bytes equal the damaged file;
- a second open is still the same typed error;
- reading the evidence twice returns the same records.

## Adversarial

- **Stale state**: a CAS with a stale version gets `conflict` and leaves no receipt (`happy`).
  A conflicting write inside a multi-write mutation rolls back the whole mutation
  (`a_conflicting_write_rolls_back_the_whole_mutation`). A proptest compares 24 random CAS
  sequences against a reference model (`versions_follow_the_reference_model`).
- **Cancel/resume**: the writer process is killed at two commit barriers.
  - At `Committed`, state, receipt and event all survive (`happy`).
  - At `Applied` (before `COMMIT`), none of them survive and the revision stays 0
    (`crash_before_commit_keeps_nothing`).
- **Misleading success**: the reopened durable store is the oracle in every crash test. A
  corrupt or newer database never opens as an empty store. Evicted events and receipts
  report their floor, not an empty page.
- **Tests can fail**: four temporary code mutations were each killed by the named test, and
  the code was restored and re-run green (`mutation-probes.log`). The mutants were:
  receipt not written, CAS ignoring the expected version, the barrier moved after `COMMIT`,
  and the header check returning "missing".

## Risks

- `Cargo.lock` changes (rusqlite, tokio and their dependencies). It is outside this todo's
  write scope, but adding the declared workspace dependencies requires it.
- The C compiler and linker per CI target are not pinned yet; `.github/workflows/ci.yml` is
  outside this scope. The bundled SQLite version is pinned through `Cargo.lock`
  (libsqlite3-sys 0.39.1 → SQLite 3.54.1).
- `Ticket::wait` blocks and panics inside a Tokio runtime (documented). Async callers use
  `Ticket::outcome().await`. `Snapshot` reads are blocking SQLite calls that the async
  coordinator (todo 10) must run off the executor.
- After receipt eviction, an unknown op id reports `NotRetained { floor }` rather than
  `Absent`: the store cannot tell an expired id from one that never existed.
- `lib.rs` carries one crate-level `#[expect(clippy::redundant_pub_crate)]`. That nursery
  lint contradicts the workspace's `unreachable_pub` for crate-internal items.
- On Windows, directory fsync after quarantine renames is not performed (Windows exposes no directory flush; NTFS journals the rename).
  Windows correctness is CI-smoke only.
