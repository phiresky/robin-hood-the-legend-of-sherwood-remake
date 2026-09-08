# Runtime correction pass

The user authorized correction of the failures found during
runtime validation. Work starts from
`ecfdfbb10`, including the concurrent cleanup/fog changes that were excluded
from the earlier pinned runtime validation. The original frozen `3e6beaa17`
overnight parity sweep continues unchanged and cannot validate these repairs.

## Parallel workstreams

| Scope | Owner branch | Required evidence |
| --- | --- | --- |
| Quick-load replay equivalence | `repair-replay-load` | Real serialized save roundtrip, identified state difference, fresh same-binary replay through the load boundary. |
| Manual-frame stepping | `repair-frame-stepping` | Full normal-frame commit before nested steps; every live forward tick recorded; graphical and replay reconstruction checks. |
| Multiplayer resynchronization | `repair-multiplayer` | Explicit host ready-barrier transition, delayed exact-frame hash comparisons/invalidation, actual two-peer post-reconnect input. |
| Window shutdown | `repair-window-close` | Actual exit-code publication before event-loop exit, no guessed success or blocking UI join; close during startup/menu/gameplay. |
| Terminal modal flow | `repair-modal-flow` | Prove the debug-WIN overlap cause; preserve deferred scripted lanes while terminal-owned work completes; ordinary terminal scenarios too. |
| Browser content | `repair-browser-content` | Correct language-neutral descriptor lookup without falling back translated strings; actual Chrome with verified original retained content. |

Every branch uses its own Cargo target directory, bounded runtime processes and
isolated writable game data. Evidence from the earlier failures remains intact.
Production fixes and regression tests were committed and combined in
`repair-integration`, with final acceptance recorded below. Android access remains blocked;
the user did not authorize device provisioning or production deployments.

## Decisions and boundaries

- Preserve strict frame contiguity, replay identity, save formats, required-data
  errors, and multiplayer admission ordering. Do not hide failures with empty
  state, guessed exit codes, or unconditional acceptance of duplicate events.
- Arbitrary backward scrubbing is not equivalent to loading a serialized save.
  Correctly recording it needs a distinct restore mode and corresponding replay
  format/admission changes. User direction on including that migration was
  requested; forward-step repair proceeds without disabling existing rewind.
- Integration normalizes the two previously reported formatting-only expressions
  in `main_entry/init.rs` and `video_player.rs`, without changing their behavior.

## Integrated verification and runtime evidence

- Baseline at `ecfdfbb10`: explicit Cargo suites for `robin_engine`,
  `robin_assets`, `robin_parity`, `robin_replay_format`, `robin_util`, and
  `robin_state_hash_derive` passed. This is baseline evidence, not verification
  of subsequent repairs.
- The named `tooling` suite passed, including its deliberate failure-injection
  tests. The named `format` suite passed after the formatting-only normalization
  and shared-descriptor repair; it must run again after final integration.
- Combined production source `83f87b1d1`: six core packages passed 4,813
  tests, with eight unit/integration tests and one doctest ignored. The client
  release-feature suite passed 1,562 tests with nine ignored; its separate
  native binary build passed. Eight additional protocol/service/scripting
  package suites passed. These results precede the final replay-driver fixes;
  the repeated combined client suite and build are recorded below.
- [Save/load](repairs/replay-load.md): ordered-map JSON decoding and serialized
  snapshot scratch-state reconstruction repaired. A fresh recording crossed
  save/load and reached EOF at frame 1,017, with all 41 checkpoints matching
  (20 after load), using committed source `185ea5783` (production-equivalent
  to `83f87b1d1`). Both live and playback processes closed successfully.
- [Frame stepping](repairs/frame-stepping.md): complete transaction ownership,
  shared graphical/headless mission-name bootstrap, and recorded modal timing.
  At `f4d922729`, one fresh 61-record export reached EOF in both graphical and
  true-headless playback with matching hashes at frames 0, 25 and 50. Graphical
  playback included recurring physical Return input, the earlier failure
  trigger. All 196 session tests passed at that source.
- [Multiplayer](repairs/multiplayer.md): actual graphical and headless two-peer
  runs exercised synchronized stepping, same-process reconnect and subsequent
  input. Both reached matching post-reconnect frame-75 hashes with no desync;
  the graphical run explicitly reported one unavailable checkpoint rather than
  counting it as agreement. Source and binary identity are recorded separately.
- [Window shutdown](repairs/window-close.md): actual startup/loading, briefing,
  gameplay and menu close returned zero; missing-data failure remained nonzero.
  Exit publication and never-polled-future cleanup have regression coverage.
- [Terminal modals](repairs/modal-flow.md): controlled early and post-startup
  WIN/LOOSE scenarios completed without the terminal-owned leaderboard deadlock.
  These are not naturally completed campaign missions.
- [Browser](repairs/browser-content.md): shared descriptor lookup restored
  authored tutorial text without weakening translated-string requirements;
  cooperative yielding restored browser responsiveness. Actual rendering,
  audio-source starts, manual stepping, export and BFCache restoration were
  observed. Direct Restart now consumes queued replay configuration before
  construction; final activation/EOF and dialogue-presentation verification
  are recorded below.

### Final production snapshot: `51db7aa06`

All production corrections are combined at
`51db7aa06099b54481eb1124df8fa04dd8af6a68`; subsequent report commits do not
change their source. On that snapshot:

- `CARGO_BUILD_JOBS=1 RUST_TEST_THREADS=2 cargo test --locked -j1 -p robin_rs --no-default-features --features release`
  passed: 1,554 unit and 22 integration tests, with nine ignored overall.
- The separate `cargo build --locked -j1 -p robin_rs --bin robin --no-default-features --features release`
  passed (dev profile with the release feature, not an optimized release build).
  Binary and adjacent assets/mods are retained in
  `/tmp/robin-final-release-feature-51db7aa06-QtN8TU`; binary SHA-256:
  `c1e2b9e1ace7c7c0ed38315d1e4406d8383628ba2a8a6078fa897b97e3fc45b1`.
- Named `format` and `tooling` suites passed. Tooling's deliberate corrupt-input,
  process-kill and publication-failure diagnostics are expected test cases.
- A fresh desktop recording with four normal and 30 paused manual steps yielded
  60 records. The same export reached EOF in graphical and true-headless
  playback; all three checkpoints (0, 25, 50) matched. Graphical playback also
  received recurring physical Return input. See the final snapshot section of
  [frame-stepping evidence](repairs/frame-stepping.md) for the separate desktop
  binary identity and retained artifact closure.
- [Strict modal presentation](repairs/strict-replay-modals.md) keeps recorded
  control authoritative while preserving dialogue audio and sentence progression.
  Its focused backend probes are included in the passing combined client suite.
- Matched WASM game/helper builds and all 366 named web tests passed. Two actual
  Chrome runs passed: five manual steps produced a nine-record replay, while
  ordinary held input and real pause produced a 218-record replay. Both exports
  were accepted by the matched helper, activated through the actual Restart
  action, and played to EOF without desync, content errors or page exceptions.
  Both remained responsive after persisted BFCache restoration. The natural run
  displayed the authored rescue objective and observed 636 audio decodes/five
  source starts; physical audibility is not implied. Detailed artifacts and
  immutable matched-pair identities are in the browser repair report.

## Remaining acceptance and scope limits

- The bounded final native/browser acceptance gates passed. Broader limitations
  below remain explicit follow-up scope, not passing acceptance claims.
- Strict headless aborted-debriefing batches fail explicitly before mutation;
  full terminal/debriefing parity is not implemented or claimed.
- Arbitrary rewind-branch recording still requires the separately scoped replay
  format migration described above. Existing rewind remains available.
- Android device acceptance, physical audio audibility, a naturally completed
  campaign and long-duration multiplayer acceptance remain unverified.
- The independent frozen `3e6beaa17` parity campaign remains active. Its resumed
  results preserve completed evidence and the original deadline; they do not
  validate this correction branch. See [the parity ledger](validation/parity.md).

Status: correction implementation and bounded final integration acceptance are
complete. Detailed reports retain commands, artifact paths and exact source/binary
identities so earlier successful runs are not attributed to later changes.

## Merge and cleanup

The correction pass was fast-forwarded into `main` at `a3965be9b`. Changes after
the verified production snapshot `51db7aa06` are reports only. All seven clean,
merged `repair-*` worktrees and branches were removed after their owned jobs
finished and binary/resource/evidence closures were retained outside worktrees.
Their committed changes remain recoverable from main's history; discarded local
build caches can be rebuilt. No user recordings or original game data were removed.

The active `validation-parity` worktree and controller remain untouched. At
cleanup its frozen ledger recorded 86/256 exact EOF results and no failed result;
this is a progress checkpoint, not a completed sweep. Unrelated local
reference material in the main checkout was preserved. No push or
deployment was performed.
