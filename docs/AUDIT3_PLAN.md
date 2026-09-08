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
- `audit3-integration`: shared save-picker model after the store API stabilizes,
  integration, review and acceptance.

Each implementation uses its own same-named `.worktrees/` checkout. No stash,
target redirection, clippy, performance merges, deployment or push. Preserve
historical formats and current startup/profile identity policy. Runtime owners
are not reconstructed from persisted metadata.

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

TODO: record reviewed commits, exact executed checks, retained artifact/source
provenance, failures corrected and any remaining limits before final merge.
