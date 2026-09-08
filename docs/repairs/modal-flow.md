# Native terminal/modal ownership repair

## Confirmed cause and correction

Baseline: `ecfdfbb10`. The baseline diagnostic executable adds only the terminal
handoff debug trace; it does not change task admission. Evidence is retained at
`/tmp/robin-modal-repair-srUKdQ` outside this worktree.

The native early-WIN stall is a product deadlock, not merely an automation key
mistake. At terminal leaderboard handoff, the trace proves that an active
scripted modal batch remains. Terminal presentation intentionally defers that
batch, but the UI-task guard also suspends the mission-end leaderboard child
behind it. The terminal state machine cannot finish until that child completes.

`frame_simulate.rs` now makes task admission explicit. A mission-end leaderboard
task runs while its terminal owner is active, even when scripted presentation is
deferred. This exception does not run other tasks or dismiss scripted effects:

| Task with scripted presentation pending | Admission |
| --- | --- |
| Mission-end leaderboard, terminal owner active | Run |
| Mission-end leaderboard, no terminal owner | Suspend |
| Quick-load confirmation | Suspend |
| Options, save/load, quit confirmation | Cancel |

Without scripted preemption, all task kinds retain their existing runnable
behavior. The child still executes its real asynchronous leaderboard operation
and reports its actual outcome; no await is bypassed. A one-shot handoff trace
records active/queued scripted presentation and the mission-state signal. The
now-unused quick-load classifier was removed, leaving other task APIs intact.

## Graphical reproduction and verification

All cases use the actual desktop `robin` binary, Xvfb display `:96`, the Linux
full-game corpus, and `--mission H01_Lin_VL --http-server 17896`. The launcher
isolates configuration, user data, and saves in each evidence directory. It
does not overwrite existing player saves. Vulkan software rendering produced
the mission scene and debriefing screens. Audio hardware was unavailable;
audibility is not validated.

`scripts/validation/client_modal_flow.py` drives real X11 Return events and the
diagnostic WIN/LOOSE console commands. The non-early cases wait until simulation
has progressed beyond startup. These are controlled terminal-path tests, not
claims of achieving victory or loss through ordinary mission play. Direct
`--mission` mode is expected to exit after its terminal flow, not start another
campaign mission.

| Evidence subdirectory | Observed result |
| --- | --- |
| `baseline-early` | WIN at frame 2; active scripted batch at leaderboard handoff; still stalled after 14 confirmations and 170 seconds of process lifetime. Explicit diagnostic window-close ended the process; that is not terminal success. |
| `baseline-win-mounted` | WIN after startup frame 12; same retained-batch handoff; 14 confirmations leave frame 13 stalled. Explicit diagnostic close at 100 seconds, not terminal success. |
| `fixed-early` | Same early WIN and retained batch; child executes, `LevelSucceeded`, normal exit code 0. |
| `fixed-win` | WIN after startup frame 14; retained batch at handoff; `LevelSucceeded`, normal exit code 0. |
| `fixed-loss` | LOOSE after startup frame 11; Mission Lost screen observed; Continue produces `Quit`, normal exit code 0. |
| `final-early` | Clean committed-source rebuild; WIN at frame 2; retained batch at handoff; `LevelSucceeded`, client and corrected driver both exit 0. |

The tested leaderboard operation honestly reported that no published individual
level ruleset matched this mission/facet selection. Its existing unavailable
outcome completed the flow; this does not validate a live ranked submission.
The first fixed-early driver attempted another key after the client had already
exited normally and reported a missing window. The harness was corrected to
recognize the client's explicit successful-exit log; the later win/loss drivers
both completed successfully. Client process results and driver outcomes are
kept separate in the evidence.

An initial relocated-baseline launch (`baseline-win`) failed startup because
the copied executable had no adjacent required core overlay. Its failure is
retained separately. Copying the repository assets beside the preserved binary
resolved that packaging issue for `baseline-win-mounted`; no gameplay change
was made between those two launches.

Each case retains `client.log`, HTTP host-state snapshots in `driver.jsonl`,
screenshots, process samples, isolated saves/replays, and `result.json`.

## Rust verification

Focused Rust admission tests cover the terminal-child exception, preserved
script-effect queues, all unrelated task kinds with and without terminal
ownership, and the no-script truth table. The existing phase-handoff test also
remains. All tests below passed with `CARGO_BUILD_JOBS=1` and the common command
prefix `cargo test --locked -j1 -p robin_rs --features desktop --lib`:

- `game_session::frame_simulate::tests`: 4 passed (three new admission tests).
- `game_session::terminal_debriefing::tests`: 5 passed.
- `game_session::ui_task_state::tests`: 17 passed.

`cargo fmt --all --check`, `git diff --check`, and Python driver syntax checking
also passed. Existing unrelated compiler warnings remain. This is focused
repair verification, not a claim that the complete workspace suite was run.

## Final binary provenance and cleanup

The final executable was rebuilt **after** source commit
`25a368601d3d56859ef8638ced205a05618bed15` with a clean worktree, using
`CARGO_BUILD_JOBS=1 cargo build --locked -j1 -p robin_rs --features desktop --bin robin`.
That separate incremental build passed in 12.52 seconds. It includes removal of
the unused helper; the earlier fixed-case executable predates only that cleanup.
The preserved final executable itself was used for `final-early`, whose launcher
reported 50.008 seconds, normal exit code 0, and no bounded termination. At
12:17:56.662 UTC its trace retained the scripted batch; `LevelSucceeded` followed
at 12:17:56.769, and the game future returned 0 at 12:17:57.748.

Preserved files relative to the evidence root, with SHA-256:

- `robin-baseline-trace` (baseline plus trace only):
  `6b92904924aa05f4f378e0ccabcb42ef2c3a75cf2744a4ffb0167994a3beea0f`.
- `robin-fixed-before-helper-cleanup` (first three fixed cases):
  `62dde6218e008b5bcbdaf4b2066d32c0345480bc1a7ebcd783239e607b1e5ff9`.
- `robin-25a368601` (final committed source, verified identical to build output):
  `0237b8983c2dd5990c3118ee0bf9a0a685fb2f7e0f758d471335a0ddd9c563b9`.
- `final-early/data/robin_hood/replays/2026-09-07T14-17-43+02-00.rhrec.jsonl`:
  `544414eecabb14876e3e1115e270ff886c4010564f772b29c3ca44f37fb379c4`.

Required adjacent repository assets are preserved under `assets/`. The owned
Xvfb display `:96` was stopped after the final client exited. No client or Cargo
jobs remain owned by this repair. Evidence was not deleted; worktree cleanup is
left to the coordinating agent after integration.

TODO: validate a campaign next-mission transition and a live available
leaderboard response separately; neither is implied by the direct-mission exit
checks above. This repair does not address replay save/load equivalence,
manual-step ordering, or native window-close behavior owned by other tracks.
