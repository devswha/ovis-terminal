# Released baseline amendments

`schema/released.openrpc.json` is the contract released clients are gated
against. Generation 1 is not released yet, so the baseline may still be amended,
but never silently: every change to its content gets an entry below.
`openrpc compat schema/released.openrpc.json` and the `schema` test suite refuse
a baseline whose content this ledger does not record (`baseline_amendment_missing`).

Digests are SHA-256 over the compact JSON encoding of the baseline, so they
follow content, not formatting. `- released:` is the baseline as first committed
(PR #8). Each entry needs `date`, `pr`, `fields`, `reason`, `previous` and
`current`. `previous` is the prior digest in the chain and `current` is the digest
of the baseline after the change. The last `current` must match the file.

- released: sha256:e2741c41cf3c2731dd135fa3e67027c918998236535bb2a9a5cddbce79a97546

## 2026-10-31 hello limits become decimal-string counts

- date: 2026-10-31
- pr: #36 (todo 22, commit 4c5ea95)
- fields: hello limits event_replay_events, history_lines and max_in_flight_requests (components.schemas.Limits, and their x-limits.defaults values 10000, 100000 and 64 become "10000", "100000" and "64") change from integer {format uint32, minimum 0} to $ref Count; components.schemas.Count is added (string, pattern ^(0|[1-9][0-9]{0,19})$)
- reason: every u64 counter on the wire is a canonical decimal string; these three limits were the last JSON numbers in hello, and generation 1 had no released client to break
- previous: sha256:e2741c41cf3c2731dd135fa3e67027c918998236535bb2a9a5cddbce79a97546
- current: sha256:362980ac3f118f9866ae95f33defb0a5542b4d8a20825432a0a580ec809ae971

## 2026-11-01 update.* gains digest, target, verified build and rollback receipts

- date: 2026-11-01
- pr: #60 (todo 47)
- fields: update.stage params digest becomes sha256 (components.schemas.Sha256Digest, ^[0-9a-f]{64}$), target (BuildTarget, optional) is added and version becomes ReleaseVersion (^[0-9A-Za-z][0-9A-Za-z.+-]{0,63}$); UpdateRecord replaces digest and generation with sha256, target, staged_by, build (VerifiedBuild), rejection, activation and rollback; UpdateQueryResult adds active, previous, pending and live_incarnations; UpdateState docs; update.* error lists add invalid_update_state, activation_failed, no_staged_update, incompatible and too_large where raised; components ActivationPhase, ActivationReceipt, ActiveBuild, BuildManifest, BuildTarget, HolderGeneration, PendingActivation, ProbeOutcome, ReadinessEvidence, RejectReason, ReleaseVersion, RollbackReason, RollbackReceipt, Sha256Digest, UpdateRejection and VerifiedBuild are added
- reason: the todo-4 shape could not name a target, carry a checked digest or expose verification, activation and rollback outcomes (GAP-127, IDEAL-100); generation 1 has no released client of update.*
- previous: sha256:362980ac3f118f9866ae95f33defb0a5542b4d8a20825432a0a580ec809ae971
- current: sha256:53782aabf6c5ff8bbbe0a9fd28357dc5eb26f02359d7d660647fc627f3fa206d
