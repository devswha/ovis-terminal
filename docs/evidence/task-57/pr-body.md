

## Addendum: merge of origin/integration 804e508

Merged `origin/integration` 804e508 (T35, T4, T56, T40, T48) on request. Only
`.github/workflows/probe-stress-macos.yml` and the generated `schema/ovis-terminal.openrpc.json`
conflicted. The workflow is integration's file with `credential-rotation` appended last as an
option and a job (`yaml.safe_load` passes). The OpenRPC was regenerated: integration's
document plus this lane's two methods; `openrpc check` and `compat` exit 0. `suite.rs`
EXECUTABLE is 19 sorted entries, matching the executable files on disk.

`conformance/src/fixture/agents.rs` was 265 pure LOC on the merged tree. The fixture's
terminal registry (`Directory`, its two impls and its `directory()` constructor) moved
verbatim to `conformance/src/fixture/agents/directory.rs`, re-exported as
`fixture::agents::Directory`. No behaviour change and no `#[allow]`; `workspace-hygiene`
reports 0 violations in 1039 files.

Gates (`docs/evidence/task-57/merge-gates-804e508.log`):

```
build: exit=0
windows-check: exit=0
windows-clippy: exit=0
clippy: exit=0
hygiene: exit=0
check.sh run 1: exit=100 (Summary [  52.238s] 1041 tests run: 1040 passed, 1 failed, 1 skipped); failed ovis-terminal-core::ops bounds::capacity_accepts_1024_and_rejects_1025_without_eviction with SQLite disk I/O error (extended 778) on the shared tmpfs /tmp
isolated rerun of that test: tmpfs            47G   25G   22G  53% /tmp -> 1 passed
check.sh run 2: tmpfs            47G   25G   22G  53% /tmp (Summary [ 130.014s] 1041 tests run: 1041 passed (1 slow), 1 skipped)
{"case":"all","error":null,"evidence":"/home/devswha/workspace/ovis-terminal-wt/lane-t57/.omo/evidence/ovis-terminal-v1/task-57/merge2/rotation.json","evidence_written":null,"result":"passed","scenario":"security/rotation","totals":{"assertions":"140","cases":"2","declared_only":"0","failed":"0","passed":"2","skipped":"0"}}
rotation all: exit=0
```

The first `check.sh` run failed one test with a SQLite write I/O error while other lanes
shared the tmpfs `/tmp`. This lane does not change `core/ops` or the store. That test passed
alone, and the second full `check.sh` passed 1041/1041.
