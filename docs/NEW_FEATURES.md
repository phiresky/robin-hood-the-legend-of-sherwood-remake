# Post-port Features

A list of which additional features we have added, which ones we might still want to add, and which old ones we will NOT add.
## Done

- **Pathfinder `LinkSource` relaxed-grid retry**
  (`crates/robin_engine/src/pathfinder.rs`). When the strict pass links zero
  candidate nodes and `object_position_authorized(source)` is true, the
  pathfinder re-runs the link pass with `relax_grid = true`, skipping the
  `is_reachable_grid` check on the source-to-docking-point corridor. This
  avoids stalling an actor for the 100-frame `IMPOSSIBLE` timeout in narrow
  pockets where the actor's 1-px-shrunk bbox fits between motion lines but
  every full-corridor sweep clips. Per-frame motion-line collision in
  `engine/movement.rs` still clamps any detail the relaxed check glossed over.

- **Local script-RPC HTTP server** (`crates/robin_rs/src/http_server.rs`).
  Loopback-only blocking-IO server (`tiny_http`) that exposes the script VM
  and engine internals to external tooling: debug shells, test harnesses, AI
  drivers. Default port **17640**, configurable via `--http-server <port>`,
  `--http-server 0` to disable.
  - `GET /` — endpoint listing.
  - `GET /natives` — every NativeFn (index, name, return_type, params)
    with parameter names parsed from `assets/script_api/RHScriptAPI.scs`
    (embedded at compile time via `include_str!`).
  - `GET /engine-dump` — full serialized engine state for ad-hoc debugging.
  - `GET /script` — mission-script class & function listing.
  - `GET /script/decompile[?class=Foo]` — TypeScript-like pseudocode
    via `robin_assets::decompile`.
  - `GET /screenshot` — PNG capture of the next rendered frame, with debug
    overlay flags and optional resize/hide-UI query params.
  - `POST /native` — `{op, args, this?}` invokes one native; per-call
    optional `this` overrides `GameHost::script_this` for the dispatch.
  - `POST /batch` — array of native calls executed back-to-back on
    one tick; useful for `Start`/`Record*`/`Thanx` recording sessions.
  - `POST /console` — invoke a debug-console cheat (`HIGHLANDER`,
    `GIVE BLAZON`, etc.).
  - `POST /command` — apply a `PlayerCommand` (externally-tagged JSON
    enum); covers move, click, swordfight, action-bar selections.
  - `POST /step-forward`, `POST /step-back`, `POST /go-to-frame` — external
    frame stepping and replay scrubbing.
  - `GET /get-replay`, `POST /load-replay` — in-memory replay export/import
    for native and wasm drivers.
  - Threading: a dedicated listener thread funnels requests through a
    shared queue; the game-session frame loop drains it once per tick
    after `run_engine_tick`, so HTTP-driven side effects land on the
    same frame as normal script-native side effects.

- **Upscaling — shipped modes**. Options -> Graphics -> Scaling is wired
  through `crates/robin_rs/src/gpu_upscale.rs`, `shaders/*.wgsl`, and
  `build.rs`. Currently shipped in the UI: Nearest, PixelArt, Linear
  (SDL-native), plus single-pass SDL_GPU shaders: **Sharp-Bilinear**,
  **Bicubic**, **Lanczos**, and **CUT3**, plus **RetroArch Shader** preset
  selection.

- **Deterministic replay and rollback checking**. Sessions can be recorded to
  JSONL, replayed from disk or compact `rhrec-...` strings, and checked with
  per-frame state hashes. The rollback checker periodically replays recent
  frames from a snapshot and compares the reconstructed engine state against
  the live state to catch nondeterminism.

- **Basic multiplayer**. Native host/client networking, wasm WebSocket clients,
  seat IDs, input delay, rollback for late inputs, mission seed sync,
  state-hash desync detection, mid-mission state snapshots for joiners, and
  client reconnect are implemented. The current design is predictive rollback
  netcode rather than strict "wait for every peer before ticking" lockstep.

- **Partial Spellforge Lua mission support**. Custom-mission launch can extract
  and sandbox a Lua companion, register native shims, and call its
  `Initialize` / `PostInitialize` hooks. This is a post-original extension, not
  Original-game scripting parity. Timer, victory, finalize, and per-entity
  dispatch are not wired at this revision, and Lua state is not saveable or
  rollback-safe; see `PARITY_AUDIT.md` before enabling it in deterministic
  modes.

## Todo

- **Android touch polish**
  - Complete two-finger pan and pinch-zoom support. The first Android pass maps
    one-finger touch to left mouse and two-finger centroid drag to viewport pan;
    follow up with proper gesture state, inertia/clamping, pinch zoom around the
    gesture centroid, and interaction rules for UI/minimap/pause overlays.
  - Render pacing should target 60 FPS or the device screen refresh rate instead
    of the current fixed game-loop cadence. Keep simulation at the existing
    fixed timestep, but present/interpolate at display cadence where possible.

- **Widescreen and high resolutions**
  - Fix the portrait bar being cut off at the bottom.
  - At high resolution the game should force a minimum scale. Zoomed out too
    much is kind of cheating and makes the game weird: enemies feel like they
    cannot see far enough and sounds are localized too small. Basically,
    1024x768 should be the max a player can see at 1x zoom.

- **Upscaling follow-ups**
  - **Scale2x / Scale3x / xBR-lv1** are implemented in `shaders/` and
    compiled into the binary but removed from `TextureScaleMode::ALL` because
    they reproducibly GPU-reset inside `canvas.present()` on Mesa/RADV Vulkan.
    Bicubic and Lanczos with the same binding layout / `num_samplers=1` work
    fine, so it is not the descriptor layout. Still unknown whether the
    underlying bug is driver-specific, an SDL_GPU render-target-as-sampler
    layout transition issue, or a specific SPIR-V instruction pattern our
    shaders use. Re-add these modes once someone reproduces/fixes it.
  - Backend coverage: SPIR-V only (Linux / Vulkan). Metal (MSL) and D3D12
    (DXIL) still need a shader cross-compile pass; shader modes silently fall
    back to Linear on those drivers. Either hand-port `.wgsl` to HLSL and build
    via `sdl-shadercross`, or enable naga's `msl-out` / `hlsl-out` features.
  - Multi-pass shader runner candidates:
    **xBRZ**, **hqx**, **super-xbr**, **Anime4K v4**, **ScaleNX with artifact
    removal**, and CRT shaders (as a separate `TextureEffect` enum).
  - References:
    - https://github.com/libretro/slang-shaders
    - https://github.com/libretro/common-shaders
    - https://en.wikipedia.org/wiki/Pixel-art_scaling_algorithms

- **Cursor visual effects**. The SDL3 cursor path currently draws hardware
  cursors directly, so old software-cursor post-effects are not represented.
  Reintroduce only effects that have a visible gameplay hook:
  - **Quick-action recording pulse** — while recording a quick action, tint the
    cursor shadow with a pulsing green highlight so it is obvious that inputs
    are being captured.

- **Multiplayer follow-ups**
  - Add lobby leave/destroy events so clients stop showing stale sessions when
    the host closes or leaves.
  - Merge rewind, rollback checking, EngineManager history, and multiplayer
    rollback into one shared timeline/history subsystem.
  - Keep flattening blocking modal flows so network events, replay commands,
    frame stepping, and modal dismissal all pass through the same outer loop.

- **Pause side-menu task state**. The pause menu itself is already driven once
  per frame from the mission loop, but its side screens still run blocking
  async modal loops from `handle_pause_menu_events`: Options, Save/Load, save
  overwrite/delete confirmations, and the quit confirmation. If we want HTTP
  requests, replay commands, networking, frame stepping, and pause UI to keep
  sharing the same outer loop, replace those `show_*().await` calls with one
  small `ActiveUiTask` / `UiTaskOutcome` state machine. The gameplay modal stack
  already does this with `ActiveModal`; this would apply the same pattern to
  pause side screens.

- **Level selection tree**
  - Show campaign progress: completed missions, stats, and other information
    currently lost after the level-end screen.
  - Could be implemented as a custom map where you can walk around and inspect
    missions.

- Track how many are dead at the start of a mission so we can tell if the
  player is actually responsible for killing anyone (Clean Hands achievement).
- Ghost achievement: never seen by anyone. Independent of Clean Hands: if you
  kill someone, you count as unseen, but for Clean Hands + Ghost, living people
  remember you, so you must also never be seen.
- Fog of war system
- Unblitting (enemies that are revealed are permanently visible in the original, maybe we want to re-hide them when they are too far away?)
- Show detailed XP info somewhere: sword XP, arrow XP, etc.
- Settings to enable trackers in the top-left corner: speedrun timer, each
  achievement fulfillment.
- Add a method to unhorse horsed soldiers without killing them; no-kill runs
  are annoying with horses.
  - Add an option for Merry Men to knock people out instead of killing them.
- Production in Sherwood: show how many items will be produced.
- More combat gestures; only 9 different ones feels too low.
- Gesture quality: the more accurately a fighting gesture is drawn, the more
  damage points it applies. Needs to show the correct template somehow so the
  user can learn.
- Allow switching language in settings mid-game.
- More difficulty settings than in the original.
- Every save should have a timestamp automatically, plus mission name and
  player name. Timestamp should be shown as relative time too (`x hours ago`).
- Add autosave support.

### Code Quality

- Replace `0xFFFF` / `u16::MAX` sentinels with `Option<u16>` (and
  `0xFFFFFFFF` / `u32::MAX` with `Option<u32>`). Fields like `path_id`,
  `alert_path_id`, `obstacle_index`, `Sector.layer`, `max_occupants`,
  `INVALID_TITBIT_ID`, `INVALID_PROFILE_ID` are currently plain integers that
  use a magic "no such thing" value inherited from the old binary format.
  Porting them to `Option` makes intent explicit at the type level, surfaces
  every sentinel check at compile time, and makes the hackable JSON render
  `null` instead of `65535`. Keep the binary reader/writer mapping
  `0xFFFF` <-> `None` so on-disk format stays stable.

### Rebalancing

- Most items seem useless, like the apple throw. Maybe rebalance items to be
  more useful.

## Not-Todos

These are intentionally out of scope. Do not move them back into `Todo`
unless the project goals change.

- **JPEG / TGA / BMP write support for the asset picture layer**. The game
  data path does not need general-purpose image import/export. Keep the
  runtime focused on the formats actually used by shipped assets and current
  tooling.

- **General legacy parser utilities**. Do not rebuild small ad-hoc text
  parsers unless a current asset or tool path needs them. Prefer structured
  formats and existing Rust crates for new tooling.

- **Archive mounting as a user-facing feature**. Loading from the configured
  data directory is enough for normal play and development. Extra mount-stack
  behavior only belongs in a tool if a concrete workflow needs it.

- **Editor-only picture operations**. Pixel blits, format conversion helpers,
  and save/info paths that only supported an external editor are not gameplay
  features. Add focused command-line tools instead if we need asset inspection
  or conversion.

- **Software-renderer parity**. SDL/GPU rendering is the supported path.
  Rebuilding a complete CPU renderer is not a feature goal.

- **Unused platform abstraction layers**. Mobile, timing, and placeholder
  subsystem stubs should not be reintroduced as standalone compatibility work.
  Add platform code only when it directly supports a target we actually ship.

- **Motion blur / blind tunnel-mask cursor effects**. The apparent blur path
  was not a real gameplay-visible motion-blur feature. Keep the cursor work to
  explicit effects with current gameplay hooks.

- **Sniper zoom or gun-specific UI**. This game has no guns or sniper
  mechanics, so any zoom work should stay framed as camera readability,
  widescreen limits, or accessibility.

- **Bug-for-bug fidelity when it makes the game worse**. Keep deterministic
  behavior and mission compatibility, but do not preserve dead code, obscure UI
  quirks, or obviously unused systems solely because an older implementation
  had them.
