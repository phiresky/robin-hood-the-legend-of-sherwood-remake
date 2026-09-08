# Native graphical client validation

Revision: `3e6beaa17ace3a8d90b47ed7416f00bf0e6a29ef`, 2026-09-07.
Diagnostic-only task; no task-owned production fixes.
The separately approved browser-clock fix is tested afterward, not included in
the runtime binary described below.

## Findings

The native desktop client boots and renders Leicester, its HUD, briefing,
pause menu, and options pages. Keyboard quick-save and quick-load execute,
and XTEST mouse input produces an admitted movement command and visible actor
movement. **Fresh replay playback fails hash validation after quick-load.**
The first failure is replay ordinal 625 after the load-back at ordinal 608;
earlier checkpoints through 600 match. This is not a successful replay roundtrip.

Two other failures/limitations were observed: closing the native window returns
an erroneous exit-code-publication error; the full-game debug-WIN-during-briefing
case stalls after final debriefing. Neither is proven introduced by the refactor.

Existing authentic user recordings cannot currently be played: all 15 local
recordings have schema 16, while this revision requires schema 30. The actual
client explicitly rejects the selected recording and exits 1. The recordings
were not rewritten, migrated, or relabeled.

## Environment and isolation

- Built with `CARGO_BUILD_JOBS=1 cargo build --locked -j1 -p robin_rs --bin robin --features desktop`:
  exit 0, 7m25s, unoptimized dev profile with debug information. `desktop` enables
  native filesystem, audio, dialogs, gamepad, and hardware-info; default features
  alone would omit these integrations.
- Xvfb screens `:97` and `:98`, 1280×960×24; actual window/surface 1024×768.
  Vulkan adapter: llvmpipe, LLVM 19.1.7, CPU software renderer. Actual frames
  were presented and captured. No `--headless` runs were substituted.
- Audio initialization attempted with the compiled Kira backend, then explicitly
  warned that the requested ALSA device is unavailable and disabled sound.
  `/dev/snd` and PulseAudio were absent. No audible playback or physical gamepad
  coverage is claimed.
- Runtime working directory, `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`,
  and `ROBINHOOD_SAVE_DIR` are isolated under
  `/tmp/robin-client-validation-HR7Szn`. Existing user saves/configuration were
  not used or changed. Game data is read from the main repository's absolute
  `datadirs/demo_leicester_ecoste` and, for legacy admission, `fullgame_linux`.
- Default rollback checking remains enabled. Debug logging, software rendering,
  concurrent repository validation, and an overlapping playback process affect
  timing; these are not production GPU benchmarks or before/after speedups.

## Exercised paths

| Path | Observed evidence |
| --- | --- |
| Initialization and rendering | Actual window, Vulkan surface, bank load, deterministic audio metadata, level/HUD, briefing and menu screenshots |
| Keyboard admission | Return dismisses briefing; Escape opens pause; arrows/Return open options |
| Options cancellation | Resolution label changes 1024→640; Escape and reopen restores 1024; window remains 1024×768 |
| Quick-save | F1 writes QuickSave and Continue, including PNG thumbnails; replay marker ordinal 141, timeline 140 |
| Quick-load | F5 loads the saved state; frame samples drop 231→185; recorder logs linear load-back ordinal 608→141 |
| Mouse admission | XTEST click records GroupMove at ordinal 1137; Robin moves (209,1881)→(339.99,1896.00) |
| Presentation controls | Zoom changes render a wider castle view; crouch/stand keys exercised |
| Autosave | Initial MissionTransition autosave and restart snapshot committed to isolated profile |
| Replay admission | Unmodified authentic schema-16 replay explicitly rejected; exported schema-30 recording accepted |
| Graphical replay execution | Rendered playback, matching checkpoints through 600; desync after quick-load |
| Terminal mission flow | Natural demo loss; full-game diagnostic WIN, victory popup and all debriefing pages; no successful next-mission transition |

XSendEvent keyboard input worked. XSendEvent mouse buttons did not exercise the
client's XI2 mouse path; only subsequent XTEST mouse input is counted as mouse
coverage. `python-xlib==0.33` and `six==1.17.0` were installed only into the
temporary evidence directory, not repository dependencies.

## Replay failure reproduction

The live recorder reports:

```text
save marker recorded replay_ordinal=141 timeline_frame=140 hash="53574769c85af73f"
load recorded as linear load-back replay_ordinal=608 to_ordinal=141
```

Playback of the unmodified `/get-replay` export, using the same binary/data,
reports:

```text
Replay hash OK @ frame 600: 403aa279d02eadde
Replay desync at frame 625: expected 63b47a2cc2cc28ec, got c68d5e9a031e91bc
```

Here the logger's “frame” is the replay ordinal, not the rewound simulation
timeline. Subsequent 25-ordinal checkpoints also fail. No causal production fix
was attempted in this diagnostic branch. Root was notified with the frozen
export and QuickSave artifacts for independent diagnosis.

Playback reached its logged end after 1,540 replay records, with 37 failing
checkpoints (625 through 1525). Independent read-only diagnosis confirms that
the save/load ordinals are consistent and the first 17 post-load RNG-owner
sequences match. There is high confidence in a semantic asymmetry between live
JSON-deserialized saves and replay-pinned cloned snapshots. One medium-confidence
candidate is serde-skipped AI target-multiplicity scratch state: JSON loading
resets it while cloning retains it. This specific field has not been proven to
cause the mismatch. Relevant paths: `save_file.rs:66`, `save_file.rs:804`,
`game_session/runtime.rs:1676`, `robin_engine/src/ai/contexts.rs:1706`, and
`robin_engine/src/engine/ai/tick_scheduling.rs:44`.
Both save paths and the candidate scratch fields already existed at review base
`a413e4c80`; the failure is not attributed to this refactor without a baseline
reproduction.

```sh
DISPLAY=:98 python3 scripts/validation/client_soak.py target/debug/robin \
  /tmp/robin-client-validation-HR7Szn/playback \
  /home/phire/robinhood/datadirs/demo_leicester_ecoste 480 \
  --replay-export /tmp/robin-client-validation-HR7Szn/live/export-response.json \
  --http-server 17798 --fast-forward
```

`--fast-forward` skips pacing, not rendering. Replay data and saves remain
outside the repository because they contain installed game-derived state.

## Terminal flow and window-close findings

Full-game `--mission H01_Lin_VL` initialized correctly. The diagnostic console
`WIN` request was sent **while startup briefing remained active**, after the
first briefing Return. Subsequent keyboard confirmations displayed the remaining
briefing, Mission Won popup, narrative/achievement pages, and statistics page.
Final confirmation at approximately 10:51 UTC left the statistics image visible
until the run was closed at 10:55:52. No next mission loaded. HTTP snapshots
remained responsive at timeline frame 2; state-hash activity continued, while
per-second presentation logs stopped. This demonstrates a debug-command/modal
interaction, not a proven ordinary-victory deadlock or completed campaign run.

One source-level hypothesis is the pause-side task guard at
`game_session/frame_simulate.rs:680`: pending scripted modal state preserves a
mission-end leaderboard task but prevents its tick; terminal debriefing then
waits for that task at `terminal_debriefing.rs:599`. Runtime pending-effect counts
are not exposed by the diagnostic endpoint, so this cause remains unconfirmed.
The guard and preservation behavior also exist at `a413e4c80`.

Normal X11 `WM_PROTOCOLS/WM_DELETE_WINDOW` was sent to each of the live,
playback, and campaign windows. All three returned exit 1 with:

```text
Window/event-loop init failed: game event loop exited before the game thread published its exit code
```

This happened on **shutdown after rendering**, not graphics initialization.
`window.rs:1336` queues Quit and immediately exits the event loop;
`window.rs:1756` immediately `try_recv`s the game thread's exit code. Source
history places the immediate close before the shallow history boundary
`6aa8dcdd0`, and the checked receive/error at `c106f35b13` (2026-07-17), before
the refactor. The log's initialization wording is misleading for this case.

## Longer-session resource observations

The live Leicester process ran **1,170.107 seconds (19m30s)** before the diagnostic
window close. It reached a natural `LevelFailed` at 10:50:38.909 UTC, roughly
14m12s after launch, timeline frame 4,318. The remainder was mission-loss and
load-picker observation, not advancing gameplay. The target 20–30 minutes of
advancing gameplay was therefore **not completed**. No live rollback-desync
message or panic was observed; that does not negate the independent playback
failure.

There are 117 process samples and 114 successful HTTP frame samples. The latter
show startup briefing fixed at frame 2, options fixed at 231, the expected load
backward movement, subsequent progression to 4,318, and the terminal plateau.
Both hero movement and later combat/mission loss are visible in state and image
evidence. Two HTTP sampling failures occurred after window termination; the
sampler was then interrupted, and those errors are retained.

| Measurement | Observed value | Interpretation |
| --- | --- | --- |
| RSS, post-startup sample interval 120–830s | 1,931,448–2,007,920 KiB | Process memory, including mapped game data; not GPU-only memory |
| Process high-water mark | 2,156,356 KiB | Includes startup peak |
| Final sampled RSS | 2,008,876 KiB | Terminal/load-picker state, not active-world steady state |
| Threads, 120–830s | 52–54 | Includes worker/audio/graphics infrastructure |
| Open descriptors, 120–830s | 15–16 | No unbounded descriptor growth observed in this interval |
| CPU use, 120–830s | ~1.96 CPU-seconds per wall second | Software renderer/debug build plus concurrent validation; not a speedup |
| Renderer presentation-log buckets | 1,070 buckets; reported FPS min/median/max 2/8/24 | Mixes gameplay and modal phases; bucket count is not an exact elapsed-second FPS average |
| Reported average present duration per bucket | min/median/max 10.83/17.39/106.84 ms | Per-bucket present averages, not whole-frame latency percentiles |
| Sprite atlas residency | Grew to 50,331,648 bytes (48 MiB), 2,244 cached entries | Renderer-owned atlas counter, not all textures or total GPU memory |

Atlas growth followed additional visible areas, animations, and combat. The
sample is insufficient to prove either a leak or a bounded long-run working
set. No eviction guarantee, throughput improvement, or memory reduction is
claimed. An overlapping replay run and then full-game transition probe were
active on the second software-rendered display; raw evidence preserves this
environment rather than presenting it as an isolated benchmark.

## Provenance and retained evidence

Root: `/tmp/robin-client-validation-HR7Szn` (retain until explicitly reviewed).

- `robin-3e6beaa17-desktop`: preserved 346 MiB tested native executable, copied
  out before worktree cleanup and verified against the binary SHA-256 below.
- `provenance.txt`: source, build, environment and identity.
- `legacy/client.log`, `legacy/result.json`: explicit legacy admission failure.
- `live/client.log`, `live/samples.jsonl`, `live/frames.jsonl`: renderer counters,
  process residency and read-only HTTP simulation-frame samples.
- `live/{boot,gameplay,pause,options,graphics,graphics-edited,graphics-cancelled,reloaded,moved,minimap}.png`:
  presented Xvfb captures; `minimap.png` actually demonstrates the zoomed-out
  castle view, not verified minimap behavior. RPC `current.png` has debug IDs
  enabled by the endpoint's default.
- `live/session.rhrec.jsonl`, `live/export-response.json`, `live/saves/Profile_000`:
  recording, frozen compact export, and actual saves/thumbnails.
- `playback/client.log`, `playback/frames.jsonl`,
  `playback/after-load-divergence.png`: failure and still-rendered scene.
- `campaign/{boot,outcome,debriefing,transition-next,current}.png`,
  `campaign/stalled-engine.json`, `campaign/client.log`: debug-WIN/modal stall.
- Each runtime subdirectory contains `result.json`, including observed exit code.

SHA-256:

```text
binary          e6bef656e6551b19c794356fba655bd37f71c93d014225318317142f17f0c924
authentic input e2e94f68cc285fa04e8c8da27e4aa0434ffdcddf261a2dadff6e602c04b0405d
frozen export   034589e32d39c63079999c87c6155392f089fe7a73f1e50a0fb0baddc4478b4d
QuickSave       5f971ab3e260d8c2a14966df4aee188fca0ff8f53ae6c2380f27d5454860bad8
final JSONL     d4fa64070c7243e1a388826bc3517a6afaf015d95845172fdf4eb0c01bd5b4a8
```

Authentic input:
`/home/phire/.local/share/robin_hood/replays/2026-08-29T22-44-56+02-00.rhrec.jsonl`,
3,747 records after its header, mission `H01_Lin_VL`.

## Remaining checks

TODO: Confirm the replay-load cause with a focused clone-versus-JSON restoration
test; validate an ordinary-play victory and next-mission transition; correct and
retest orderly native shutdown; run a full 20–30-minute advancing session with
hardware rendering/audio and physical gamepad input. Native clock-fix tests ran
separately after all client processes stopped, without changing the runtime
provenance above.

Diagnostic scripts passed Python syntax parsing and were exercised against the
actual clients. `cargo fmt --all --check` and `git diff --check` passed. The
browser clock fix `d2c76faeb` is locally cherry-picked as `14cf69bd3`; its focused
native check passed with
`CARGO_BUILD_JOBS=1 cargo test --locked -j1 -p robin_rs --features desktop --lib game_session::terminal_debriefing::tests`.
Result: **5 passed, 0 failed, 0 ignored, 1,443 filtered**, test-profile build
8m23s. This includes `mission_completion_clock_returns_consistent_epoch_units`
and four existing terminal-debriefing mapping/ordering tests. The test did not
exercise or correct the independently observed live terminal stall.
