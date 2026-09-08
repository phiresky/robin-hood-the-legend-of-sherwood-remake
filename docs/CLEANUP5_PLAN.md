# Structural cleanup and second-pass audit

Base: `b70925eab8648224fd5993e7fe99a27fbcdcf655`.

## Scope and completion condition

Implement the five approved structural refactors, integrate and validate them,
then audit the resulting code for further concrete code-quality opportunities.
Implement all actionable findings from that bounded second audit before final
acceptance and handoff. This is not a claim that the repository can have no
future refactoring opportunities. Performance, deployment, new product policy
and unrelated in-flight user changes remain outside this pass.

## First implementation wave

1. `cleanup5-catalog`: one runtime slot entry/catalog; separate recovery and
   persistence behind the existing save-manager facade.
2. `cleanup5-executor`: explicit save/load preparation and execution stages,
   narrow inputs and typed outcomes; preserve replay and multiplayer contracts.
3. `cleanup5-retirement`: one shared session-retirement boundary across launch
   paths, including early-return/error handling and both background owners.
4. `cleanup5-picker`: common event/action controller for the two picker adapters;
   retain distinct scheduling and save-only text/IME behavior.
5. `cleanup5-admin`: module boundaries for CLI, key activation, verification,
   backup and authenticated cleanup; preserve private authority.
6. `cleanup5-integration`: reviews, combined client builds, second audit,
   integration fixes and final acceptance/report.
7. `cleanup5-browser`: isolated browser compilation and real Chrome gate on
   frozen source checkpoints; baseline warming is not final acceptance.

Use isolated same-named worktrees, no stash, no Clippy or target redirection.
No dependency installation or remote publication. Keep on-disk/wire formats and
runtime authority boundaries unchanged; do not add broad context bags or replace
one synchronization obligation with another. Explicit owner retirement, physical
work draining, digest receipts and process-local handles remain acceptance gates.

## Validation

Consolidate heavy native client compilation in integration; service tests use
the admin worktree. Run explicit affected packages and default/release client
configurations, separate binary builds, focused LLVM unwind regressions, browser
checks/execution and isolated native replay/save-load/multiplayer/recovery gates.
Preserve actual source/artifact identities and failed evidence. Final reports
must distinguish structural improvements, second-audit fixes, tests and limits.

Reconcile concurrent main updates without modifying the user's performance work.
After successful local merge, remove only this pass's clean, fully merged
worktrees/branches, with a verified recovery bundle first. No push.

## Completion

All five structural tracks and all seven findings of the second audit are
implemented and reviewed. See [final acceptance](CLEANUP5_ACCEPTANCE.md) for
the exact tested source, package/browser/native results, retained failed runs,
coverage limits and recoverable cleanup procedure.
