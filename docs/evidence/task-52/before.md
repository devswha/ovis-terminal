# Task 52 baseline (origin/integration 167c0a7)

Before: `absent`. There was no comparable capability, and this is not an executed failing test.

What the base tree says about this todo, verbatim:

- `conformance/scenarios/declared.json:162` (recovery/crash): "... todo 52 composes the fault-injection scenario over real endpoints."
- `conformance/scenarios/declared.json:172` (remote/identity): "... todo 52 composes remote dispatch across two private runtimes over real endpoints."
- `conformance/scenarios/declared.json:182` (services/plugins): "plugin.* routes exist ... but no served process registers them: todo 52 composes service lanes into the real server ..."
- `conformance/src/fixture/machines.rs:2,8`: the fixture's machines are in-process stand-ins "until todo 52 joins two private runtimes over real endpoints"; `fixture.remote` "stands in for remote dispatch (todo 52)".
- `conformance/scenarios/recovery/` and `conformance/scenarios/remote/` contain no `e2e` scenario, so `--scenario recovery/e2e` had nothing to run.

A defect found by the new real endpoints, with its base lines:

- `crates/ovis-terminal-remote/src/actors/actor/expiry.rs:42`: after a call's deadline, the T44 machine actor sends the id-less notification `{"method": "$/cancelRequest", ...}` to the remote runtime.
- `crates/ovis-terminal-server/src/conn/core.rs:130`: the real server answers any record it cannot correlate (no `id`) with `ConnectionError::Uncorrelatable` and closes the connection.

Together, a single stalled call to a real remote runtime cut that machine's line once its deadline passed. The T44 tests did not show this, because their stand-in remote ignored notifications. In this lane, before the fix, the remote/e2e happy case failed at `peer-after-stall` with `machine_unreachable`.
