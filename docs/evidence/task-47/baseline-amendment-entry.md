## 2026-11-01 update.* gains digest, target, verified build and rollback receipts

- date: 2026-11-01
- pr: (todo 47, lane/t47-stage-verified-updates-with)
- fields: update.stage params digest becomes sha256 (components.schemas.Sha256Digest, ^[0-9a-f]{64}$), target (BuildTarget, optional) is added and version becomes ReleaseVersion (^[0-9A-Za-z][0-9A-Za-z.+-]{0,63}$); UpdateRecord replaces digest and generation with sha256, target, staged_by, build (VerifiedBuild), rejection, activation and rollback; UpdateQueryResult adds active, previous, pending and live_incarnations; UpdateState docs; update.* error lists add invalid_update_state, activation_failed, no_staged_update, incompatible and too_large where raised; components ActivationPhase, ActivationReceipt, ActiveBuild, BuildManifest, BuildTarget, HolderGeneration, PendingActivation, ProbeOutcome, ReadinessEvidence, RejectReason, ReleaseVersion, RollbackReason, RollbackReceipt, Sha256Digest, UpdateRejection and VerifiedBuild are added
- reason: the todo-4 shape could not name a target, carry a checked digest or expose verification, activation and rollback outcomes (GAP-127, IDEAL-100); generation 1 has no released client of update.*
- previous: sha256:362980ac3f118f9866ae95f33defb0a5542b4d8a20825432a0a580ec809ae971
- current: sha256:53782aabf6c5ff8bbbe0a9fd28357dc5eb26f02359d7d660647fc627f3fa206d
