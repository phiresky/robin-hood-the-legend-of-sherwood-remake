# Ownership and recovery follow-ups

Base: `a6884ab7035d0a2bf09b1e6144b70b64708e5663`.

Implement the four follow-ups approved after audit3:

1. Own background special-save jobs, order publication after successful payload
   work, expose completion failures and define shutdown/profile-switch behavior.
2. Close the save-manager mutation surface: stable identities, explicit drafts
   versus published slots and one runtime owner rather than cloned authority.
3. Make save-store recovery a usable application state with retry and safe exit;
   display operation errors in both picker adapters without fabricated success.
4. Retain admin backup ownership through physical partial-backup work/cleanup,
   including heartbeat failure and cancellation, preserving authenticated publication.

## Work allocation

- `followup4-save-owner`: coupled save ownership/API work and minimal consumers.
- `followup4-recovery-ui`: recovery and picker UX, integrated after API agreement.
- `followup4-backup-owner`: independent server lifecycle work and service tests.
- `followup4-integration`: source review, integration, final acceptance and report.

Keep the user's performance/deployment branches and `original-code/` untouched.
Use isolated same-named worktrees; no stash, target-directory changes, clippy,
dependency installation or remote writes. Preserve on-disk compatibility,
browser session Restart and existing user identity policy. Runtime authority
must not be reconstructed from serialized state.

## Acceptance

Use deterministic completion latches and fault injection rather than timing-only
tests. Cover write order, old completion/profile retirement, failed publication,
explicit draft transitions, stable selection, retry after external repair,
cancel/quit and visible partial-delete failure. Server tests must assert physical
completion before lock/fence release, including actual LLVM unwind where relevant.

Consolidate heavy native client compilation in the integration worktree. Run
explicit affected package suites and default/release client configurations;
browser and isolated native save/load replay acceptance on frozen combined source.
Build separately from bounded execution and retain exact artifact hashes outside
worktrees. Reuse the existing validation gates; no all-features or licensed-corpus
claims. Record actual outcomes and scope limits before merge and owned-worktree
cleanup.
