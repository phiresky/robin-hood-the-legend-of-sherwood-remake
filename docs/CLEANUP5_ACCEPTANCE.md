# Cleanup 5: implementation and acceptance

All five approved structural refactors and all seven actionable findings from
the subsequent bounded audit are implemented and independently reviewed.
This closes [the plan](CLEANUP5_PLAN.md) and
[the second audit](CLEANUP5_SECOND_AUDIT.md). It does not claim that no future
refactoring opportunities exist.

## Source identity and integration

- Initial base: `b70925eab8648224fd5993e7fe99a27fbcdcf655`.
- Final tested source: `3efb481524b7576720972f35988966c9f91fd0b7`.
- Final tested tree: `8899c71aa528a7cdc5f76e38c4884cd0dbe085ae`.
- The user's concurrent replay-startup commit
  `fcab4cb06ec4af8cba32762188d9ad954e12175d` was merged before final client,
  browser and native acceptance. Its performance work was preserved.
- This acceptance report and completion links are documentation-only changes
  after that frozen checkpoint; no executable source or harness changed during
  its builds or runtime gates.

Work was isolated in ten same-named `cleanup5-*` worktrees. Integration and
independent reviews checked the actual changes, not just agent summaries.
Local `main` integration is the handoff; no remote push or deployment is part
of this pass.

## Implemented changes

| Track | Result | Detailed report |
| --- | --- | --- |
| Catalog | One private slot-entry catalog replaces parallel metadata/state/generation structures; persistence and recovery have narrower boundaries. | [Catalog](CLEANUP5_CATALOG.md) |
| Executor | Explicit preparation, dispatch and typed completion policies replace the broad save/load execution path. | [Executor](CLEANUP5_EXECUTOR.md) |
| Retirement | All three session entrypoints enclose their complete bodies and restart loops in one mandatory save-retirement boundary. | [Retirement](CLEANUP5_RETIREMENT.md) |
| Picker | Shared persistent event/action controller for standalone and cooperative adapters; save-only text/IME remains adapter-specific. | [Picker](CLEANUP5_PICKER.md) |
| Admin | The approximately 12,700-line entrypoint becomes a seven-line entrypoint with explicit authority modules and colocated tests. | [Admin](CLEANUP5_ADMIN.md) |
| S1 | Synchronous manual/diagnostic publication uses the same recovery-backed transaction as background saves; committed evidence cannot be deserialized into authority. | [Publication](CLEANUP5_S1_PUBLICATION.md) |
| S2 | Prepared loads privately retain the exact local handle/payload through mission handoff; remote loads require committed transport ownership. | [Load owner](CLEANUP5_S2_LOAD_OWNER.md) |
| S3 | Typed modal publication distinguishes host decisions, client proposals and failures; one shared gate retains failed outcomes for retry. | [Modal authority](CLEANUP5_S3_MODAL.md) |
| S4 | Chronological touch routing preserves unrelated world events and correctly handles multiple HUD taps and cross-batch releases. | [Touch routing](CLEANUP5_S4_TOUCH.md) |
| S5 | Cache maintenance is single-flight application-owned work; closing/reopening a panel cannot discard its completion or admit a duplicate worker. | [Cache ownership](CLEANUP5_S5_CACHE.md) |
| S6 | A private immutable authenticated submission owner supplies one storage projection to reservation and finalization. | [Submission](CLEANUP5_S6_SUBMISSION.md) |
| S7 | Narrow shared grant-binding policies replace four duplicated request/offer validators while preserving their different authorities and error precedence. | [Grant validation](CLEANUP5_S7_GRANTS.md) |

No save/wire format or canonical signing-byte migration was introduced. Runtime
ownership is not reconstructed from serialized data. Existing SQL lease,
reserved-offer equality, bounded preflight and physical-work-draining policies
remain in place. Obsolete split-publication helpers were removed after callers
moved to the transaction boundary.

## Final test results

Native Cargo invocations used `--locked`, the worktree's ordinary `target/`,
`RUSTC_WRAPPER=`, `CARGO_BUILD_JOBS=1`, and `RUST_TEST_THREADS=2` for tests.
Builds and bounded game execution were separate. No Clippy, target redirection,
dependency installation, player-profile reuse or golden updates were performed.

| Gate | Result | Source |
| --- | --- | --- |
| `cargo test -p robin_rs --no-default-features --features release` | 1,796 library tests passed, eight explicit ignores; integration and doc tests passed. | Final `3efb48152` |
| `cargo test -p robin_rs` | 1,692 library tests passed, eight explicit ignores; integration and doc tests passed. | Final `3efb48152` |
| Separate `robin` binary builds | Default and release-feature development builds passed. | Final `3efb48152` |
| Save-worker LLVM unwind regression | Explicit ignored test selected: one passed, zero ignored. | Final `3efb48152` |
| Vulkan GPU execution | Explicit multipass/readback test selected: one passed, zero ignored. | Final `3efb48152` |
| Browser audio/shared multiplayer | Both feature checks, actual WASM link and 34 Chrome tests passed, zero ignored. | Final `3efb48152` |
| Native lifecycle | All four ordinary/save-load headless/graphical replay phases passed on unchanged rerun; post-load hashes verified. | Final `3efb48152` |
| Graphical and headless multiplayer | Each passed all ten scenario checks, four hash comparisons through frame 75, two observed rollbacks, zero desyncs and zero missed comparisons. | Final `3efb48152` |
| Actual recovery UI | All five scenarios passed on unchanged rerun, including stationary-pointer retry after repair and normal zero-exit window close. | Final `3efb48152` |
| `cargo test -p robin_highscores` | 234 passed, seven explicit ignores. | `5f8db0352` |
| Admin LLVM unwind regressions | Both explicitly selected cases passed. | `5f8db0352` |
| `cargo test -p robin_run_protocol` | 106 passed, zero ignored. | `7f11493fc` / report tip `88808e805` |
| Format / whitespace | `scripts/check-quality.sh format` and `git diff --check` passed. | Final source |

The service/protocol source and Cargo manifests/lockfile were compared against
the final checkpoint and are unchanged from those package-tested commits.
Client counts are separate feature configurations, not a count of unique tests.
The eight ordinary client ignores are not represented as executed coverage.

The save-worker gate used:

```sh
cargo test --locked -p robin_rs --lib --no-default-features --features release \
  --config 'profile.test.package.robin_rs.codegen-backend="llvm"' \
  savegame::tests::llvm_owned_worker_panic_is_joined_and_reported_once \
  -- --ignored --exact
```

The service unwind invocation selected `backup_owner_unwind -- --ignored` with
the corresponding `robin_highscores` test-package LLVM override. These checks
prove unwind/join behavior separately from ordinary Cranelift execution.

## Retained acceptance evidence

Native evidence root: `/tmp/robin-cleanup5-runtime.ZQQecR/`.

- `final/robin`: retained release-feature binary, with adjacent `mods` and
  `assets/core-datadir` copied from the frozen source.
- Binary SHA256:
  `e880c212c3c0aaa9a5749c6f5d8348b08ed516a0a8eab2b0731c020aa0cbd97e`.
- `native-lifecycle-retry/summary.json`: complete four-phase acceptance, exact
  source/artifact identities, export hashes and nested driver results.
- `multiplayer-graphical/summary.json` and
  `multiplayer-headless/summary.json`: separate loopback-only namespace scenarios.
- `recovery-ui-rerun1/summary.json`: keyboard/pointer Retry, repair without moving
  the pointer, Cancel, Quit, Escape and window-close evidence. The initial error
  screenshot was visually inspected; broken index bytes were preserved on exits.
- `graphical-original-recheck/summary.json`: the original failed graphical
  export replayed unchanged, verifying hashes at frames 0 and 25 and EOF at 46.

The successful save/load run saved simulation frame 42, advanced, loaded back,
continued to simulation frame 142, and replayed to recording EOF at frame 248.
Post-load replay hashes at recording frames 150, 175, 200 and 225 matched in
graphical playback as well as the headless verification.

Browser evidence: `/tmp/robin-cleanup5-browser-final.9qZSDp/summary.json`,
`browser-tests.log` and `browser-tests.wasm`. Chrome and ChromeDriver were
152.0.7977.64; the lockfile-matched runner was 0.2.127. Module SHA256:
`3534dd884328ac79c997fa7262d3f94aa92ca662bddda83582f99f66ed0c87c2`.
All four new cache-owner cases and the unsupported synchronous browser-save
case have explicit passing lines in the real-browser log.

These are local test artifacts, not remote publication. `/tmp` is not durable
storage; retain them elsewhere before system cleanup if long-term evidence is
needed. No licensed game data was downloaded or modified.

## Failed attempts, corrections and limits

- Intermediate client/browser candidate `9e7e93481` failed compilation because
  skipped-field newtype serialization still required a serialization bound on
  pending transport authority. `08b05c743` replaced it with explicitly inert
  serialization and rejecting deserialization. Final native/browser builds and
  tests passed. Failed browser evidence remains in
  `/tmp/robin-cleanup5-browser-candidate.uF8pO0/`.
- Review required rejecting deserialization for committed save evidence;
  `a5cc123d1` implemented that correction before final acceptance.
- The first native lifecycle attempt (`native-lifecycle/`) passed ordinary
  live/headless playback but graphical playback exited zero after frame 0,
  before EOF. The entire unchanged gate subsequently passed, and an independent
  run of the exact original immutable export also passed. No deterministic
  content rejection or concrete cause was established.
- The first recovery attempt (`recovery-ui/`) passed the retry/repair and other
  exit cases but timed out on its final window-close case. All five cases passed
  on the unchanged rerun. Its cause likewise remains unexplained.

TODO: if either intermittent UI failure recurs, capture event-delivery and
frame-preparation exit reasons around the retained scenarios. These failures
are not silently discarded or claimed as fixed; passing reruns do not establish
their cause. No harness assertions were weakened to obtain acceptance.

Picker/controller tests do not constitute full OS IME automation. Cache-owner
tests establish admission/completion ownership, not a new physical cancellation
or shutdown-join guarantee. Native lifecycle disables sound; browser ownership
tests do not prove audible output or full-game browser rendering. Multiplayer
driver process cleanup is not normal game-window-close proof (the separate
recovery scenario supplies that). Crate-local transport commit methods remain
trusted call sites, not cryptographic authority merely because of their types.
Windows/Android execution, full licensed-corpus parity, performance and
deployment were not added to this pass. Existing compiler warnings remain.

## Recoverable worktree cleanup

Only these owned branches/worktrees are eligible for removal after local merge:
`cleanup5-integration`, `cleanup5-catalog`, `cleanup5-executor`,
`cleanup5-retirement`, `cleanup5-picker`, `cleanup5-admin`, `cleanup5-browser`,
`cleanup5-modal`, `cleanup5-submission`, and `cleanup5-protocol`.

Before removal, verify every worktree is clean and every branch is an ancestor
of local `main`, then create and verify the full-history recovery bundle at
`/home/phire/robinhood/.git/cleanup5-recovery.Tbmj0w/completed-cleanup.bundle`.
Its named heads retain the completed branch identities. Unrelated worktrees,
branches and the user's untracked `original-code/` must remain untouched.
