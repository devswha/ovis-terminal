

## Addendum: merge of origin/integration f835935

Integration moved to f835935 (T41 agent tree) right after the 804e508 merge was pushed, so
that head was merged as well. Conflicts were in `fixture/agents.rs`, `handler.rs`,
`serve.rs`, `server/tests/dispatch.rs` and the stress workflow, and both sides were kept
everywhere. `Probes` carries both `sequenced` and `tree`. `fixture.deliver`, `fixture.exit` and
`fixture.program` are all served. T41's change to `directory()` (chaining `TREE_TERMINALS`)
is applied in its new home, `fixture/agents/directory.rs`. `tests/dispatch.rs` lists both
`agent_tree` and the credential rotation modules. The workflow is integration's file with
`credential-rotation` last, and it parses. `suite.rs` EXECUTABLE is 20 sorted entries.
OpenRPC was regenerated and `compat` exits 0.

Gates (`docs/evidence/task-57/merge-gates-f835935.log`):

```
build: exit=0
windows-check: exit=0
windows-clippy: exit=0
clippy: exit=0
hygiene: exit=0
check: exit=0
Summary [ 132.618s] 1057 tests run: 1057 passed (1 slow), 1 skipped
{"case":"all","error":null,"evidence":"/home/devswha/workspace/ovis-terminal-wt/lane-t57/.omo/evidence/ovis-terminal-v1/task-57/merge3/rotation.json","evidence_written":null,"result":"passed","scenario":"security/rotation","totals":{"assertions":"140","cases":"2","declared_only":"0","failed":"0","passed":"2","skipped":"0"}}
rotation all: exit=0
```
