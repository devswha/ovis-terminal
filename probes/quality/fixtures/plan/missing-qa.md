# Seeded plan

## Todos

- [x] 1. Seed explicit targets
  What to do / Must NOT do: Reject unknown target fields. Must NOT reproduce EXCL-001.
  Closes: GAP-001
  Delivers: IDEAL-001
  Parallelization: Wave 1 | Blocked by: none | Blocks: 2 | Write scope: `seed/targets/`
  QA scenarios (exact tool + invocation): happy: `conformance --scenario seed/targets --case happy` exits 0; failure: `conformance --scenario seed/targets --case failure` exits 0
- [ ] 2. Seed typed waits
  What to do / Must NOT do: Return wait outcomes as records.
  Closes: GAP-002
  Delivers: IDEAL-002
  Parallelization: Wave 2 | Blocked by: 1 | Blocks: none | Write scope: `seed/waits/`
