# Strict graphical replay modal admission

Source corrections: `bd96b476f` and dialogue follow-up `879a9e52a`, based on `1a698f23d` (combined runtime repairs,
headless name bootstrap, and pending headless modal control handling).

## Failure and ownership

The retained graphical reproduction at
`/tmp/robin-headless-names-0dfc83581-graphical/playback.log` fails with an unused
recorded `PopupText { text_id: 0 } / Completed` dismissal. Its immutable source
export is `/tmp/robin-headless-names-0dfc83581-live-headless/export.rhrec`.
The creating replay ordinal 1 advances timeline 1 to 2; ordinal 2 is a
stationary modal-only frame supplying the dismissal.

`ModalBatch::tick` consumed a matching recorded result correctly, but when no
result existed it called the interactive widget step even in strict playback.
That step accepts physical Return/Escape/buttons (and dialogue audio timeouts).
The graphical harness sends Return repeatedly, allowing presentation to remove
the popup before its authoritative recorded dismissal arrives. This is distinct
from the separately repaired headless pending-effect ownership issue.

## Correction

An explicit admission result now selects recorded completion, strict waiting,
or ordinary interactive handling. Strict waiting pumps window events, handles
viewport transformation, renders, and presents the existing screen; it does not
call the interactive widget step. Dialogue voice playback, completed-sentence
advancement, and mouth animation continue as non-authoritative presentation;
the final sentence stays open until recorded control. Physical input, network
proposals, and audio completion timers cannot finish the screen early. The matching
recorded result remains the only completion authority. The unused-control
assertion is unchanged and still fails for unmatched controls.

The narrow presentation-only seams cover the three `ModalBatch` lanes:
dialogue, popup scroll, and scripted debriefing. Scripted debriefing asserts its
initial body-page invariant rather than inventing a later page. Live widget
ticks and terminal debriefing retain their existing semantics. Review caught
and corrected the first patch's silent/frozen dialogue presentation: live and
replay now share audio startup, and replay dismissal explicitly stops owned
dialogue audio and restores volume attenuation before dropping the screen.
An immediately recorded dismissal does not halt unrelated audio that the screen
never started. Hardware audibility is not claimed by this repair.
Window events continue to be pumped so application close and resize handling
remain live, without turning a close event into recorded modal control.

A read-only iterator on `ReplayModalDismissals` supports the separately owned
headless preflight of unsupported aborted debriefing controls; it does not
expose mutable queue ownership.

## Verification

`CARGO_BUILD_JOBS=1 RUST_TEST_THREADS=2 cargo test --locked -j1 -p robin_rs --features desktop --lib game_session::modal_state::tests`
passed all 7 tests. New tests cover all three scripted lanes waiting across a
creation/dismissal boundary, unchanged live admission, and preserved unmatched
control failure. The failure test uses the repository's `should_panic` convention
after an initial `catch_unwind` attempt proved incompatible with this test runner.

After the dialogue follow-up, the same test command with filter `ingame_menu::`
passed all 107 widget/menu tests, and the modal suite again passed 7/7. Two new
audio-probe tests exercise real `SoundManager` callbacks: first sample starts
once, mixer completion advances the next sentence, final completion leaves the
modal open, recorded dismissal halts owned audio and restores dialogue mode,
and an immediate recorded result does not halt audio the screen never owned.
The probe is not a hardware-audibility claim. Initial test-only failures (wrong
import path and querying a serde-skipped field) were corrected before these
successful runs.

`cargo fmt --all --check`, scoped Rust formatting, and `git diff --check`
passed. `CARGO_BUILD_JOBS=1 cargo build --locked -j1 -p robin_rs --features desktop --bin robin`
passed in 59.41 seconds after the source commit. The executable is preserved as
`/tmp/robin-modal-repair-srUKdQ/robin-bd96b476f`.
The post-dialogue-follow-up build also passed, in 16.71 seconds, and final
formatting checks passed. Its preserved executable is
`/tmp/robin-modal-repair-srUKdQ/robin-879a9e52a`, SHA-256
`d78f519d658a8fc2389c4b068d7a51013668280572eaab750394f240592e2f42`,
verified identical to the worktree build output. No owned build/client jobs
remain. The report-only commit does not change the tested production source.

Same-binary graphical acceptance is coordinated with the frame-stepping repair
track. Fresh recording is required for each binary identity; the immutable
older export is evidence, not rewritten to bypass replay admission.

That track's combined popup-path check at `f4d922729` passed graphical replay
with recurring physical Return and headless replay through all 61 records,
matching hashes at ordinals 0, 25, and 50. Evidence:
`/tmp/robin-headless-names-f4d922729-graphical`. This proves the popup admission
fix with the related headless corrections, but predates the dialogue-audio
follow-up. Final combined-source runtime coverage is recorded by the
coordinating validation report, not inferred from this earlier binary.
