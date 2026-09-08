# Third audit implementation pass

Base: `7be45078e` on main. Scope: all eight findings in
[CODE_QUALITY_AUDIT_3.md](CODE_QUALITY_AUDIT_3.md). Performance/startup work and
untracked `original-code/` are outside scope.

## Ownership

- `audit3-worker`: physical worker completion, heartbeat/fence ownership and
  controlled failure regressions.
- `audit3-save-store`: data-only indexes, runtime directory authority, validated
  slot identity, explicit open errors, safe allocation and durable deletion.
- `audit3-user-stores`: atomic profile/key publication and obsolete global API.
- `audit3-fixtures`: validated ranked protocol fixture construction.
- `audit3-picker`: shared save-picker model after the store API stabilizes.
- `audit3-integration`: integration, independent review and acceptance.

Each implementation uses its own same-named `.worktrees/` checkout. No stash,
target redirection, clippy, deployment or push. Preserve
historical formats and current startup/profile identity policy. Runtime owners
are not reconstructed from persisted metadata.

The user's concurrent main changes through `246d6cd86` and then `2c504019a`
were merged as prerequisites, not modified or credited to this refactor pass.

## Acceptance plan

Add focused failure tests from the audit, including blocking filesystem work
after lease loss, copied/corrupt save indexes, collision-safe allocation, partial
deletion, atomic publication failures, stable picker selections and malformed
protocol fixtures. Distinguish panic expectations from actual LLVM unwind proof.

Use explicit affected service/client package suites; default and release client
features; relevant optional consumers, WASM/browser and Vulkan ownership checks;
separate native build and isolated ordinary/save-load replay acceptance. Previous
pass results are baseline evidence only. Keep checkouts frozen during builds and
provenance-sensitive gates. Heavy compilation is coordinated to avoid duplicate
cold client graphs. No unprovisioned GL success or licensed corpus claim.

## Completed implementation and acceptance

All eight audit findings are implemented. The final tested combined source is
`98081fc4090726392e6958e6d1d5a69f6dec693f`, tree
`2c17c40011e0c247b52c34314df71596e6df445e`. This includes the user's removal of
the separate-opacity sprite experiment. Subsequent acceptance-report edits are
documentation-only. See [AUDIT3_ACCEPTANCE.md](AUDIT3_ACCEPTANCE.md) for exact
commands, outcomes, retained artifact provenance, review corrections and limits.

| Finding | Implementation | Detail |
| --- | --- | --- |
| 1: actual worker completion | Task-local tracked physical work, retained operation/lease/fence on heartbeat failure, explicit unwind proof | [Worker](AUDIT3_WORKER.md) |
| 2: save storage authority | Runtime owner, data-only index, caller-bound root and validated slot identity | [Save store](AUDIT3_SAVE_STORE.md) |
| 3: broken index/allocation | Fallible opening, explicit browser backend and no-clobber unpublished saves | [Save store](AUDIT3_SAVE_STORE.md) |
| 4: durable deletion | Recovery receipt, logical index publication, retriable cleanup and stable cross-reopen selection | [Save store](AUDIT3_SAVE_STORE.md) |
| 5: small user stores | Shared staged publication with error-stage/durability semantics | [User stores](AUDIT3_USER_STORES.md) |
| 6: duplicated picker state | Shared model and navigation/deletion bridges, separate scheduling adapters | [Picker](AUDIT3_PICKER.md) |
| 7: protocol fixtures | Correlated signed fixture builders validated by production admission rules | [Fixtures](AUDIT3_FIXTURES.md) |
| 8: obsolete global store | Removed unused global key-store entry point | [User stores](AUDIT3_USER_STORES.md) |
