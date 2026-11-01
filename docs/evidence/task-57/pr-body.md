

Integration then moved to b96f583, which turns the suite inventory into a slice. That head
was merged too: only `suite.rs` conflicted, and it now takes integration's slice form plus
`security/rotation.json` (20 sorted entries). Gates
(`docs/evidence/task-57/merge-gates-b96f583.log`):

```
build: exit=0
windows-check: exit=0
windows-clippy: exit=0
clippy: exit=0
hygiene: exit=0
check: exit=0
Summary [ 136.918s] 1057 tests run: 1057 passed (1 slow), 1 skipped
rotation all: exit=0
```
