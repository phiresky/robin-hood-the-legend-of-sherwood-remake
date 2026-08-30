# Post-port Features

A list of which additional features we have added, which ones we might still want to add, and which old ones we will NOT add.

## Done

- **Untie tied NPCs.** A PC with the Tie skill can click any living tied NPC
  to release them, using the rope cursor and the authored tying animation in
  reverse. Search remains the first contextual action while the NPC carries
  loot. Untying preserves unconsciousness and concussion so normal recovery
  remains authoritative, works in Shift queues and replay/multiplayer command
  streams, and can be disabled under Gameplay to restore shipped behavior.

- **Bounded widescreen and native-resolution presentation.** The three
  original logical scale presets remain 640x480, 800x600, and 1024x768, while
  an enabled-by-default Graphics option adapts the logical canvas to window
  aspect changes. High reaches 1280x720 at 16:9; wider displays are
  letterboxed, portrait/narrow displays retain 4:3, and every preset is capped
  inside the 1280x768 gameplay envelope. The physical swapchain remains
  independent, including HiDPI browser canvases and fullscreen output. Resize
  propagation updates pointer mapping, camera bounds, minimap, HUD, and modal
  return paths without changing the three original camera zoom levels. Wider
  portrait bars expose additional slots, fixed-width border art fills the
  complete width, and Android uses a cutout-safe immersive content area.
  Disabling Adaptive Widescreen restores fixed 4:3 parity presentation.

- **Live Sherwood production forecast.** The Sherwood report now includes a
  compact, toggleable item-production panel built from current map stock and
  live production-zone membership. Once a mission is selected it shows exact
  output for that mission's authored duration; before selection it reports an
  exact one-hour rate rather than guessing a mission. Each item line exposes
  current stock, five-per-production-point capacity, overflow, worker and
  specialist inputs, authored speed, and the original game's explicit lack of
  raw-material consumption. Forecasting and campaign production call the same
  pure calculation, with boundary tests guarding truncation and saturation.

- **Data-driven mission allegiances.** Hackable JSON missions may assign a
  numeric `allegiance` to each soldier and rescue PC. IDs `0` and `1` preserve
  the legacy Royalist and Lacklandist camps; any `u16` ID is accepted, and
  distinct valid allegiances are mutually hostile. Runtime target detection,
  combat, minimap classification, cursor actions, difficulty modifiers, and
  NPC-enemy availability use relationship queries instead of assuming one
  opposing camp. Legacy RHM actors without the optional field still derive
  their allegiance from the original hostile/attitude profile flags.
  Hackable descriptors also accept `spawn_player`, `soldiers`, and `pcs`;
  soldier `profile` values may use readable CPF-filename identifiers such as
  `guard_a01` (legacy numeric indices remain accepted);
  AI-controlled heroes with `decision_policy: "enemy_ai"` run through the normal enemy
  perception, pursuit, target-reacquisition, and battle-decision lifecycle
  instead of receiving one-off mission-start pairings. Their required readable
  `ai_profile` (for example `soldier_b04`) selects the behavior personality;
  the PC profile still supplies the hero's weapons, skill, endurance, sprites,
  damage, tiredness, and reactive defence.
  Ten launchable test arenas live under `mods/multi-team-*`:
  three-way, ten-way, every soldier/PC profile in unique-allegiance circles,
  autonomous Robin versus Little John, four armies of twelve soldiers, and a
  four-army matchup where each faction uses a distinct soldier grade, and an
  all-hero ten-way circle free-for-all, four archer companies in crossfire,
  twenty Robins against twenty Black Knights, and four champions with mixed
  soldier retinues.
  Diplomacy beyond the current
  different-ID-is-hostile rule remains a future extension.

- **Orthogonal human-actor roles and control.** `Pc`, `Soldier`, and
  `Civilian` now describe body/profile archetypes only. Runtime and hackable
  level data independently describe decision ownership (`player_directed`,
  `enemy_ai`, `friendly_ai`, or `scripted`), the player command surface
  (`hero_actions`, `tactical_orders`, or `none`), mission bookkeeping role,
  combat stance, and allegiance. This permits AI heroes and tactically
  commandable soldiers or villains without pretending that body type or camp
  determines control. Legacy RHM Royalist troop commandability and the old
  custom-level `autonomous`/`aggressive_combat` fields are translated once at
  load time. Existing replay command names and structured snapshot keys remain
  readable for compatibility. Rescue heroes explicitly transition from
  `rescue_target`/no commands to `player_party`/hero actions when the original
  `CharacterAvailable(true)` sequence fires.

- **Mission-selective shipping data.** Converted shipping datadirs now contain
  a compact boot manifest plus independently compressed mission cores and
  shared per-character RHS sprite payloads. Native, Android, and browser builds
  load only the files referenced by the selected mission; decoded files remain
  cached when missions share characters or when the player returns to a
  mission. Per-RHS grouping deliberately preserves the strong within-character
  zstd matches measured in `docs/COMPRESSION.md`.

- **Browser-native shipping audio.** Web shipping conversion transcodes voice,
  effects, and music to deterministic Opus dependencies while retaining exact
  source-duration metadata for simulation timing. The browser fetches and
  decodes only boot audio plus the selected mission's closure, reports that
  work on the loading screen, and keeps decoded PCM in Web Audio buffers rather
  than wasm linear memory. Native and Android artifacts retain source audio.

- **Shift-click quick-action queue.** Holding Shift switches the portrait
  action buttons, cursor, and projectile preview to a separate planning state:
  selecting Bow or an item does not equip it, stop the hero, or otherwise
  mutate the live PC. World clicks use the QA macro system and execute in
  order, with no three-action queue limit. The first action starts as soon as
  the actor's existing work finishes; its slot is consumed immediately, while
  the next three actions remain visible above the portrait. Shift-double-click upgrades
  the newest pending movement to a run. Planned actions may be selected even
  when their live ammo is empty, and releasing Shift or Shift-right-clicking
  clears the planned action without touching the live PC. Bow arcs are previewed from the last
  queued or live movement destination, so targets can be planned from the
  position the hero will actually occupy.

- **Self-updating native packages.** Installed Windows and Linux Velopack
  builds check the public GitHub Releases feed in the background. Stable
  installs consider stable releases only, while prerelease installs also
  follow dated `nightly-YYYY-MM-DD` prereleases. Downloads do not interrupt
  play: completed updates are applied silently after a normal game exit, and
  a previously downloaded pending update is applied before the next startup.
  Headless games, standalone archives, and developer builds do
  not attempt to update themselves.

- **Startup datadir selector.** When `ROBINHOOD_DATA_DIR` is unset, the
  game resolves the data folder itself: a previously confirmed choice
  (remembered in `datadir.txt` next to the saves) is used silently;
  otherwise it auto-detects an installation — working directory,
  executable directory, then the usual install locations of the original
  CD (`Program Files\Wanadoo Edition\<localized title>`, per the Wise
  installer script), GOG (GOG Games, GOG.com, Galaxy), and Steam,
  plus Wine/Heroic/Lutris prefixes on Linux — validated via
  `Data/robinhood.bks` (case-insensitive; `Data/datadir.bin` shipping
  bundles also count). A native dialog always confirms the result: OK
  accepts the found installation, Cancel opens the OS folder picker. The
  dialog recommends a GOG purchase. The remembered folder can be changed
  later via Options → "Game Data Folder" (applies on next launch).
  Headless runs use the auto-detected folder without a dialog and keep
  the descriptive terminal error otherwise.

- **Core overlay datadir.** `assets/core-datadir/` is registered as an
  always-on overlay ahead of the `mods/` overlays. It currently restores
  the game's native bitmap fonts (~280 KB) plus the font `manager.cfg`,
  fixing the Steam release — whose depot ships only the international
  TrueType (SimSun) font set and therefore renders every menu in a
  Windows system font in the original build too. See
  `assets/core-datadir/README.md`.

- **Hackable JSON levels.** Every subdirectory of `mods/` is registered as an
  overlay datadir at startup, and any overlay may ship an editable
  `Data/Levels/<mission>.level.json` geometry descriptor (title, spawn point,
  walkable polygon, architectural volumes) that expands into normal level
  structs at load time — no legacy RHP/RHM/terrain encoding involved.
  Backgrounds and minimaps can be plain PNGs, optionally paired with a 16-bit
  `<map>.occlusion-depth.png` for continuous sprite occlusion. Discovered
  levels get a main-menu entry (descriptor `title`) and can be launched
  directly with `--mission <name>`; they run as unscripted sandboxes. The
  repository bundles one such level in `mods/dover/`: a Dover Castle
  exploration map made from a Gaussian splat, rendered by the reproducible
  standalone WGPU renderer under `scripts/dover_splat_renderer/`, which levels
  the reconstructed ground plane, renders at the original game's 35-degree
  elevation, and keeps the playable castle orthographic while smoothly
  applying perspective only behind it.

- **Direct custom-mission launch.** `--custom-mission <zip>` mounts a vanilla
  mod archive for the lifetime of a direct `--mission <name>` launch. Pair it
  with `--proto <map>` when the mission and proto-level basenames differ.

- **Optional tactical-unit control.** A persistent `Control Tactical Units`
  game option enables high-level control of actors whose level data exposes
  `command_interface: "tactical_orders"`, independently of allegiance. Click
  or drag a selection box to create a temporary portrait beside the heroes;
  its illustrated pin button preserves an individual or group portrait.
  Named soldier profiles can supply dedicated 112x50 visage art; Guy of
  Guisbourne, Longchamp, Prince John, Scathlock, and the Sheriff use portraits
  cropped from the Original's dialogue resources, while ordinary and mixed
  groups retain the helmet portrait. The bundled `mods/five-villains` custom
  mission remixes the original Save Stuteley mission: Guy starts in Robin's
  place and frees the other four villains from its original prisoner slots.
  Legacy roster mods can preserve compiled-script actor indices while
  overriding beam-me, rescue-PC, and tied-prisoner visuals. A per-mission
  `.text.patch.json` can override popup, short-briefing, and dialogue strings
  while retaining the base mission's descriptor pictures and timing.
  Mod-added character profiles keep their visible name and NPC exclamation
  bank separate from the internal RHS profile key, so promoted villains retain
  their own voices. NPC sprite sets used as PCs also fall back to compatible
  authored attack rows for target interactions they do not natively animate.
  Promoted NPCs can import their retail soldier combat statistics while
  retaining a playable character template's action layout. Mods can remove
  inherited contextual actions unsupported by the NPC sprite; Five Villains
  disables wall jumping while retaining authored ladder climbing.
  Allied portraits
  expose cycling hold/defensive/aggressive stances, two-point patrol targeting,
  and type-aware line, box, staggered, and flank formations. Selecting soldiers
  visualizes both player-issued patrols and authored mission patrol paths as an
  animated dotted world-space chain with waypoint and next-destination markers.
  Line formation
  places officers at the command center, knights in close escort, shield and
  melee troops on the fighting edge, and ranged troops in protected rear or
  central positions. When heroes move with the selection, soldiers deploy
  behind them instead of overlapping their formation. Long moves automatically
  narrow to a two-wide marching column before deploying at the destination.
  Double-click runs allied soldiers even when a drag selection also contains a
  hero.
  Controlled soldiers can enter swordfights, execute the normal drawn strike
  gestures, and parry once while releasing the allied selection with
  right-click. Combat autonomy follows stance: Hold
  permits only explicit gestures, smalltalk, and reactive parries; Defensive
  returns attacks without AI pursuit; Aggressive retains full combat AI.
  Explicit gestures supersede combat work the soldier AI had already queued. Hover
  tooltips name every action and its current state and appear quickly across
  each button's full cell. Soldiers receive deterministic names from the
  localized peasant-name pool. Selection uses the heroes' persistent ground
  ring and fading green outline. The portrait bar
  computes its capacity from the actual screen width (six portraits fit at
  800 px) and uses the original Sherwood left/right arrow resources with
  wraparound paging when the combined hero and allied portraits overflow.

- **Optional Hard reaction-time fix.** Options → Gameplay includes a
  persistent `Fix Hard Reaction Times` toggle. When enabled, Lacklandist NPCs
  on Hard difficulty use the intended `HARD_REACTIONTIME_MODIFICATOR` instead
  of the Easy modifier selected by the original game's copy-paste bug. It is
  on by default; the original-parity replay tool disables it explicitly.
  Changes made during a mission take effect immediately through the
  deterministic command stream.

- **Fog/night-tint all sprites option.** Options → Graphics now includes a
  `Fog/Night All Sprites` toggle. On fog and night missions it applies the
  generated ambiance sprite variant to Day-based world sprites, including
  bonuses, scrolls, animals, and mobile child sprites that the original leaves
  in the Day palette. Animation-backed FX and targets already load
  ambiance-specific pixels and are excluded to prevent double tinting. New
  profiles enable it by default; the toggle can restore original behavior.
  `render_mission_map` exposes explicit `--fog-tint-all-sprites` and
  `--no-fog-tint-all-sprites` overrides for reproducible A/B captures.

- **Mission-start full-map renderer**
  (`crates/robin_rs/examples/render_mission_map.rs`). Loads a mission through
  the regular engine and GPU renderer, captures the complete map after the
  requested number of normal simulation frames (`--frame`, default zero), and
  writes a HUD-free PNG. `--reveal-all` (alias `--unblip-all`) switches every
  NPC from its blip silhouette to its normal character profile before capture;
  `--headless` keeps the screenshot renderer's GPU-backed window hidden.
  `scripts/render_all_mission_maps.sh [OUTPUT_DIR] [FRAME] [DATA_DIR]` renders
  every shipped mission using the full-game profile mapping and human-readable
  mission titles as filenames.

- **Local script-RPC HTTP server** (`crates/robin_rs/src/http_server.rs`).
  Loopback-only blocking-IO server (`tiny_http`) that exposes the script VM
  and engine internals to external tooling: debug shells, test harnesses, AI
  drivers. Default port **17640**, configurable via `--http-server <port>`,
  `--http-server 0` to disable.
  - `GET /` — endpoint listing.
  - `GET /natives` — every NativeFn (index, name, return_type, params)
    with signature provenance from `original-code/RHScriptAPI.scs`.
  - `GET /engine-dump` — full serialized engine state for ad-hoc debugging.
  - `GET /script` — mission-script class & function listing.
  - `GET /script/decompile[?class=Foo]` — TypeScript-like pseudocode
    via `robin_assets::decompile`.
  - `GET /screenshot` — PNG capture of the next rendered frame, with debug
    overlay flags and optional resize/hide-UI query params.
  - `POST /native` — `{op, args, this?}` invokes one native; per-call
    optional `this` sets the transient `ThisActor` receiver for the dispatch.
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

- **Upscaling and presentation effects**. Options -> Graphics -> Scaling now
  ships a portable multi-pass wgpu runner. It includes Nearest, Linear,
  Pixel Art/Sharp-Bilinear, Bicubic, Lanczos, CUT3, a published-corner-rule-
  derived **ScaleNX** path with artifact removal, and clearly labelled clean-room
  **HQx-style**, **xBRZ-style**, **Super-xBR-style**, and **Anime line A/B/C
  (v4 layout)** profiles. The Anime profiles follow Anime4K v4's documented
  restore/soft-restore/denoise pass ordering, but intentionally do not claim
  to reproduce Anime4K's trained kernels.
  - CRT is an independent, disableable `TextureEffect`: None,
    **CRT Guest-class**, or **CRT Royale-class**. Both implementations are
    original portable WGSL inspired by those shaders' documented controls;
    no GPL shader code is embedded.
  - Strength, edge threshold, artifact removal, scanlines, phosphor mask,
    bloom, curvature, and presentation-rate temporal flicker are persisted
    per profile. Temporal state advances only after a frame is submitted for
    presentation, independently of deterministic simulation ticks.
  - The world/video layer is scaled and effected first. Menus, HUD, cursors,
    and modal overlays are then alpha-composited with sharp-bilinear sampling
    so display effects do not blur text.
  - Standard native builds can choose bundled `.slangp` presets or import and
    compile an external preset from the Graphics screen. Preset
    parse/compile/frame errors are reported; a failed preset never silently
    falls back. Browser builds hide this unavailable choice while retaining
    all portable WGSL profiles on WebGPU and WebGL2.
  - Algorithm provenance, exactness, licensing, platform support, and shader
    restrictions are documented in `docs/UPSCALERS.md`.

- **Deterministic replay and rollback checking**. Sessions can be recorded to
  JSONL, replayed from disk or compact `rhrec-...` strings, and checked with
  per-frame state hashes. The rollback checker periodically replays recent
  frames from a snapshot and compares the reconstructed engine state against
  the live state to catch nondeterminism. Explicit playback requests are
  decoded before Engine construction and fail fatally if their required header
  cannot be read; playback never substitutes a multiplayer or default RNG
  seed. Gameplay randomness uses one serialized Engine-owned `fastrand` stream
  instead of Original's process-global C RNG; parity is reviewed at ranges and
  call-site order rather than bit-identical rolls. The reviewed inventory and
  host-only exceptions live in `RNG_AUDIT.md`; typed serialized-stream labels
  and a separately typed seed-derived authoritative peasant-name generator,
  plus a structural source test, reject unreviewed gameplay RNG additions.
  Recordings stay one linear timeline across in-mission saves and loads: a
  save at a clean frame boundary writes a save-marker record (`sv`, the
  state hash at capture), and loading a save made in the same session writes
  a load-back record (`lb`) pointing at that marker's frame. Playback pins
  the complete engine, sound, host-input, and persistent game state at each
  marker and restores it through the normal post-load path, so
  quicksave/quickload and script-triggered restarts replay bit-exactly
  without embedding save payloads. Loads of saves from other sessions cannot
  be expressed this way and log a warning that the recording is no longer
  linearly replayable.

- **Original-game parity traces**
  (`crates/robin_rs/examples/original_parity_replay.rs`). A diagnostic runner
  streams the neutral JSONL trace emitted by the instrumented C++ game,
  applies its resolved player commands on the recorded frames, and compares
  typed entity state using exact floating-point bits. Unsupported legacy
  command values and malformed/non-contiguous traces fail loudly; the first
  divergent frame is reported field-by-field.

- **Basic multiplayer**. Native host/client networking over iroh
  (peer-to-peer QUIC with relay fallback; peers addressed by endpoint id, no
  port forwarding), seat IDs, input delay, rollback for late inputs, mission
  seed sync, state-hash desync detection, mid-mission state snapshots for
  joiners, and client reconnect are implemented. Matchmaking is fully
  serverless: the multiplayer menu joins a well-known iroh-gossip topic
  bootstrapped through the BitTorrent Mainline DHT, so games are discovered
  with no broker, master server, or configuration. The current design is
  predictive rollback netcode rather than strict "wait for every peer before
  ticking" lockstep.

- **Authenticated browser multiplayer**. A native host can publish a
  30-minute, fragment-only `rhmp3` invitation for
  `https://robinhood.phiresky.xyz/`. Browser peers use iroh's
  relay-over-WebSocket transport with the unchanged protocol-25 game wire,
  prove a durable non-extractable identity through an isolated typed signer,
  and reclaim only their parked seat generation. Demo and Full joins fail
  before boot unless the ticket-selected engine artifact, exact native
  Data/locale closure, and every browser package byte agree. Reconnect adopts
  an authoritative replacement snapshot even when it predates the abandoned
  prediction future, then clears future inputs/hashes/history. Only the host
  records the canonical server-ordered replay. Relay observability is stated
  in the invitation UI, and browser-link publication is a default-on persisted
  privacy setting that can be disabled without affecting native iroh play.

- **Partial Spellforge Lua mission support**. Custom-mission launch can extract
  and sandbox a Lua companion, register native shims, and call its
  `Initialize` / `PostInitialize` hooks. This is a post-original extension, not
  Original-game scripting parity. Timer, victory, finalize, and per-entity
  dispatch are not wired at this revision. Required Lua construction and
  startup-event failures abort mission startup with context; they are never
  replaced by an SCB-only continuation. Lua state is not saveable or
  rollback-safe, so replay playback, rollback/determinism verification, and
  multiplayer host/client launches reject Spellforge before authoritative
  simulation starts. The custom-mission picker explicitly omits the default
  rollback diagnostic for ordinary single-player Spellforge play. This
  containment is intentional until a versioned Spellforge contract and Lua
  snapshot policy exist.

- **Shipping dictionary rank permutation** (`convert_datadir
  --rank-dictionaries`, default on). Sprite dictionaries are reordered by
  tile-use frequency and all VQ indices rewritten to match at conversion time;
  invisible to the decoder, ~-2.9% on the RHS chunk bucket. Verified
  pixel-identical via `sprite_compression_probe --verify-shipping`.

- **VQ sprite context-model codec** (`robin_assets::sprite_codec`, library
  only — not yet wired into the shipping schema). Adaptive PPM + range coder
  over tile-index grids with optional cross-variant base coding; measures the
  full character corpus at 2.27x smaller than zstd-19. Integration design in
  `docs/COMPRESSION.md` (schema v7 section).

## Todo

- **Android touch polish**
  - Complete two-finger pan and pinch-zoom support. The first Android pass maps
    one-finger touch to left mouse and two-finger centroid drag to viewport pan;
    follow up with proper gesture state, inertia/clamping, pinch zoom around the
    gesture centroid, and interaction rules for UI/minimap/pause overlays.
  - Render pacing should target 60 FPS or the device screen refresh rate instead
    of the current fixed game-loop cadence. Keep simulation at the existing
    fixed timestep, but present/interpolate at display cadence where possible.

- **Cursor visual effects**. The wgpu cursor path draws the cursor as a regular
  sprite, but old software-cursor post-effects are not represented.
  Reintroduce only effects that have a visible gameplay hook:
  - **Quick-action recording pulse** — while recording a quick action, tint the
    cursor shadow with a pulsing green highlight so it is obvious that inputs
    are being captured.

- **Multiplayer follow-ups**
  - Sign matchmaking announcements with the game identity key so a peer
    cannot advertise a game under another host's endpoint id.
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
  - Implemented as selectable Classic Map, prerequisite/progress tree, and a
    modal Sherwood Hall of Deeds exhibit grid. Every Rust campaign always
    retains immutable full-fidelity records for every attempt (including
    losses and practice replays) and derives totals/bests from those records.
    Only Original C++ saves are imported; their limited status/recent-mission
    data remains explicitly incomplete. A freely walkable world-space Hall is
    deferred. See `docs/CAMPAIGN_HISTORY.md`.

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
- other achievements:
    - pile 10 bodies in the same building
    - stash ALL enemies in the same building
- Add a method to unhorse horsed soldiers without killing them; no-kill runs
  are annoying with horses.
  - Add an option for Merry Men to knock people out instead of killing them.
- More combat gestures; only 9 different ones feels too low.
- Gesture quality: the more accurately a fighting gesture is drawn, the more
  damage points it applies. Needs to show the correct template somehow so the
  user can learn.
- Allow switching language in settings mid-game.
- More difficulty settings than in the original.
- Every save should have a timestamp automatically, plus mission name and
  player name. Timestamp should be shown as relative time too (`x hours ago`).
- Add autosave support.
- trading: if you over produce an item, maybe you can sell it for money?
- throw something skill that makes a noise somewhere else so guards run there
- Cloaking (implemented, optional): selected heroes whose sprite profile has
  the shipped cape rows can put the cloak back on with a rebindable key. The
  reversed original transition leads to a dedicated stationary Cloaked state;
  unaware distant hostiles are deceived, while remembered targets, ordinary
  line-of-sight after a reveal, and close scrutiny see through it. Acting or
  taking damage reveals the hero. Fresh profiles enable this; migrated
  profiles preserve original one-way cape behavior until enabled in Gameplay.
  Original replay construction always forces the feature off. The shipped
  human visibility routine has no character-specific detector and the unused
  animal runtime has no shipped mission instances, so the explicit authored
  detector seam remains disabled with a TODO for a future mod schema instead
  of assigning invented special senses. `cloak_art_audit` validates both cape
  rows for every declared PC profile: the full Linux data has 10/10 available
  and eligible tracks; the Leicester demo has 5/5 available tracks eligible
  (its CPF also declares five full-game profiles whose RHS files are absent).
- timed mission - you only have a certain time limit to finish the mission. ambience transition - mission moves from day to night to fog to day after time
- improvements to quick actions: shift-click should queue an action
- Most items seem useless, like the apple throw. Maybe rebalance items to be
  more useful.
### Additive hackable sprite mods

- Overlay mods can append soldier profiles through
  `Data/Configuration/soldier-profiles.patch.json` without replacing the
  retail CPF profile table.
- Added profiles may specify `progression_from` alongside their `template` to
  extrapolate one additional combat-stat tier from two adjacent retail tiers.
  This supports elite variants beyond the original black-guard ceiling while
  retaining each unit role's established progression.
- Readable soldier identifiers use normalized CPF filenames. When the retail
  CPF repeats a filename, hackable levels retain the original numeric identity
  with `<name>__<cpf-index>` (for example `archer05__47`) instead of silently
  choosing one record.
- Hackable RHS manifests explicitly select `rgba` or `legacy_color_keys` PNG
  semantics. Legacy green transparency and blue cast-shadow masks remain
  available to ambience-aware rendering instead of being baked into alpha.
- One overlay mod may expose multiple hackable missions through the
  `hackable_missions` array, and large character packs may opt into
  mission-scoped sprite loading.
- Native builds compile each hackable `.rhs.d` PNG tree into an atomic,
  zstd-compressed runtime cache beside its manifest. Cache hits reuse packed
  engine sprites and animation tables; manifest hashes and source file
  metadata invalidate stale caches automatically.

### Code Quality

- Finish moving legacy sentinels to typed runtime boundaries. Entity IDs,
  titbit IDs, layers, sectors, obstacles and AI patrol paths now use nominal
  handles and `Option` where absence is meaningful. Raw level-data structs
  still retain authored `0xFFFF` values, and a few animation/ammunition fields
  use the maximum value as real Original-game protocol. Convert remaining
  runtime fields only when their semantics are proven; keep asset-reader
  translation at the binary boundary instead of spreading sentinel checks.


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

- **Software-renderer parity**. wgpu rendering is the supported path.
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
