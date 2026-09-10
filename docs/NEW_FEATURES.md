# Post-port Features

A list of which additional features we have added, which ones we might still want to add, and which old ones we will NOT add.

## Done

- **Generic JSON Patch mods.** RFC 6902 operations edit decoded profiles,
  levels and resource descriptors. Profile filenames provide named keys while
  existing numeric slots are preserved. Patches compose across directory and
  ZIP overlays, with atomic installation and typed error reporting. Legacy
  mod patch formats have been removed. See [JSON Patch mods](JSON_PATCH_MODS.md)
  for filenames, examples, the profile-view exporter and current limits.

- **ZIP mod overlays:** Put a mod ZIP directly in `mods/` or the configured
  `ROBINHOOD_MODS_DIR`; `details.json` and `Data/` should be at the archive
  root. A single wrapping folder is also accepted. JSON mission discovery,
  soldier profile patches, PNG characters, standalone VQ characters, and
  shipping VQ families use the same overlay reads as directory installs.
  ZIP directories are enumerated without extraction; in-memory mission
  archives use the same sprite loader. Disposable PNG caches remain optional
  and are written only for directory installs. Install just one edition of
  a mod when editions share mission and character identifiers.

- **VQ sprite mod packages:** Custom `.rhs.d` directories can contain an
  authored `sprites.vq.zst` instead of PNGs and a manifest. The
  `encode_mod_sprites` example converts a mod using exact four-pixel RGB565
  dictionaries and the shipping adaptive VQ codec, then verifies every frame
  and animation field through the runtime reader. Transparency, shadow keys,
  odd frame widths, and profile stats are preserved. Loading reconstructs the
  existing runtime sprite representation. These packages require this engine
  update; the source PNG mod remains separately editable.
  Build with `cargo build -p robin_rs --example encode_mod_sprites`, then run
  `target/debug/examples/encode_mod_sprites SOURCE_MOD DESTINATION_MOD`.
  Whole-mod conversion now groups identical animation layouts into
  `Data/Characters/*.sprites.vq.zst` families: one frequency-ranked dictionary
  per character, entropy-selected two-hub colour prediction, temporal/direction
  references for standalone hubs, and the production 1,048,576-tile groups.
  Family files use the shipping bank, grouped encoder, and dependency-aware
  materializer directly. The converter also replaces `.map.png` terrain with
  JXL quality 80 `.map` files through `cjxl` and verifies runtime decoding.

- **Custom mission pane scrolling:** The mission list and wrapped mission details
  scroll independently under the pointer. Both show draggable scrollbars when
  their content overflows; selecting another mission resets its details to the top.
  A shared scroll view also powers Campaign Manager’s Hall of Deeds, Achievements,
  mission details, and Previous Plays, with a horizontal scrollbar for Campaign.
  Scrolling preserves selection; keyboard navigation reveals the selected mission.
  The same component handles multiplayer mission browsing, save/load lists,
  shortcut bindings, and debriefing text, including the in-mission UI paths.
  All scrollbars reuse the original menu artwork.

- **Save mission time:** In-mission save info includes elapsed simulation time
  as minutes and seconds, including autosaves. Sherwood saves omit it; older
  catalog entries without a recorded timer remain readable.

- **Language-independent audio timing.** The required core-datadir
  `Data/AudioDurations.json` supplies English speech variants and sample
  durations to the client, replay preparation, and ranked verifier. A French-only
  installation needs no English audio pack. Localized recordings play to their
  natural end while simulation completes speech on canonical frames. Startup
  rejects a missing or invalid timing file; mission loading rejects missing
  required timing entries. The Rust `generate_audio_durations` example rebuilds
  the table and core inventory from English full-game and demo audio.
  See [generation instructions](../assets/core-datadir/README.md).

- **Mission details and previous plays:** Campaign Manager's Mission Details tab
  combines the original localized briefing, entry requirements, and a complete
  history across saved and archived campaigns. The briefing and play-history
  columns scroll independently under the pointer, with scroll-position indicators.
  Double-clicking a mission in the tree or gallery opens its details, including
  locked missions. Active unfinished missions are labeled In progress. Outcomes, dates,
  durations, and practice runs remain visible even without a recording.
  Watch Replay opens an available recording in a separate desktop viewer while
  preserving the current session. New terminal recordings are linked by exact
  attempt identity; older compatible local recordings are indexed in the
  background. See [campaign history](CAMPAIGN_HISTORY.md#campaign-manager-ui-and-offscreen-captures)
  for controls and browser limitations.

- **Campaign story navigation:** the main prerequisite route stays visible above
  stage-grouped story branches, optional missions, and ambushes. Training and
  campaign events are labeled separately; unused map placeholders are omitted
  unless they hold archived results. Mission Details explains actual money,
  gang, mission, expiry, and story restrictions, with keyboard and mouse scrolling.
  Achievement cards describe exact conditions and campaign-versus-mission scope.
  These presentation changes preserve campaign selection and award eligibility.

- **Campaign manager layout:** fixed 1024×768 presentation with twelve-card
  gallery pages, a navigable progress-tree viewport, wrapped mission titles,
  separate mission details, and three distinct views: Campaign for the current
  save, Hall of Deeds for permanent mission badges and best results across
  attempts, and Achievements for player-wide awards with current campaign
  progress beneath each. Whole-campaign awards require one qualifying campaign;
  loading an older save retains earned awards and archived best results.
  The scripted Sherwood ending is labeled Epilogue, placed after the finale in
  the tree and last in the gallery, without changing mission unlock rules.
  Both menu entry points and Sherwood share the layout. An opt-in offscreen GPU capture
  test produces PNGs for visual iteration without a game window; see
  [campaign history](CAMPAIGN_HISTORY.md#campaign-manager-ui-and-offscreen-captures).


- **Campaign manager from menus:** Campaign Manager on the main menu or
  Escape → Campaign Manager during any mission opens the existing progress
  tree / Hall of Deeds, including before reaching Sherwood. The main menu
  reads the selected player's latest resumable save, or a fresh campaign if
  there is no save. Navigation, history, and badge details use the same UI;
  mission launching is unavailable in this pause-side view. Escape returns to
  the originating menu without changing the active mission or campaign.


- **Combined leaderboards and custom gameplay settings.** Mission and full-campaign
  boards default to all admitted rulesets, with stable ranking and pagination across
  difficulties. Exact Standard/Original boards remain selectable. Published metadata
  retains every mission/ruleset pair, and Full-game field missions support individual
  runs. Open rulesets accept the complete recorded simulation configuration, including
  Legendary and Custom difficulty. The existing signed session digest fixes the settings
  before play; verification checks the replay against those settings and independently
  reconstructs the canonical fresh campaign. Each run exposes its exact gameplay settings.

- **Scrollable mission debriefings.** Narrative, achievement conditions, and
  statistics scroll within the parchment using the scrollbar, mouse wheel,
  arrows, Page Up/Down, or Home/End. The footer controls stay visible.

- **Earlier browser replay downloads.** Admitted replay URLs prepare their exact
  mission selection once and start a bounded batch of required files before the
  normal mission loader, which reuses those requests. Five 16 Mbit/s pairs saved
  a median 172 ms with unchanged payload bytes; loopback showed no gain.
  See [measurements and validation](perf/replay-plan-earlier.md).

- **Recoverable save-store admission.** Damaged or inaccessible save indexes
  offer Retry and safe cancellation/exit without creating an empty replacement
  store. Save-picker errors are shown in yielding, scrollable acknowledgements;
  failed save operations also produce an in-game failure notice. Unpublished
  save drafts remain retryable but are not offered as loadable saves.

- **Shared scripted modal replay ownership.** Headless replay now shares scripted
  modal batch ownership with graphical sessions, supporting recorded aborted
  debriefings without discarding later-emitted batches. This does not add headless
  campaign/profile promotion or final load/restart presentation; those
  replay-required boundaries fail explicitly rather than report EOF.

- **Offline editor reconstruction.** `PIPELINE_OFFLINE=1` (or the AI CLI's
  `--offline`) permits reconstruction from validated provider caches without
  credentials or network calls. Missing or corrupt cache entries fail with
  actionable errors; completed cache publications are atomic.

- **Inspectable datadir conversion plans.** The converter writes
  `conversion-plan.json` with source dependencies and grouped output decisions.
  An incomplete checkpoint remains available if conversion fails; successful
  publication marks the plan complete. This diagnostic file is not part of the
  runtime payload or web content manifest.


- **Advanced combat gestures.** Swordfighting accepts nine optional
  single-stroke composite techniques in addition to the Original A-I gesture
  vocabulary. Composite recognition runs only after the original-game classifier
  returns `Attempt`, so enabling it cannot steal an already recognized legacy
  strike. Each technique expands into two ordinary authored sword commands,
  retaining their normal animation, interruption, targeting, energy, and
  protection behavior. An independent quality option quantizes template
  accuracy to deterministic 25/50/75/100 percent tiers and scales only cutting
  and concussion; protection RNG and geometry are unchanged. Optional guide
  and post-stroke coach overlays teach both the legacy and composite paths.
  Gameplay rules are authoritative mission state; presentation remains local.
  All four controls are independent under Gameplay, whose standalone screen is
  paginated so every row stays reachable at 640x480. Mouse and one-finger touch
  share the same recognizer, and replays/rollback/multiplayer carry the resolved
  typed technique and quality rather than platform-dependent pointer samples.
  Malformed, disabled, or mismatched payloads fail before simulation or
  quick-action recording. The incompatible native layouts advance to save
  **67**, replay **25**, and multiplayer protocol **34**.

- **Timed missions and runtime ambience.** Hackable JSON missions can author
  an active-play time limit and ordered Day/Night/Fog ambience cues. Timers
  pause with single-player/noninteractive simulation gates, stop once victory
  is achieved, and use the ordinary mission-loss flow at expiry. Ambience cues
  deterministically change AI perception, light-sector gameplay, filtered
  sound sources, lighting crossfades, sprite dictionaries, background art and
  minimaps; missing alternate art warns and falls back to Day. Gameplay and
  presentation/countdown controls are independently configurable. The hashed,
  serialized tick/cue/crossfade state is shared by saves, replay, rollback and
  multiplayer. `mods/timed-ambience-demo/` is a launchable example; authoring
  rules are documented in
  [Timed missions and runtime ambience](TIMED_MISSIONS_AND_AMBIENCE.md). This state
  advances the combined native save, replay, and network schemas to
  **65 / 23 / 32** respectively.

- **Cooperative pause side screens.** Options (including Graphics, Sounds,
  Shortcuts, and Gameplay), Save/Load,
  overwrite/delete prompts, and Quit confirmation now run as one-frame
  `ActiveUiTask` states owned by the mission loop instead of nested blocking
  async loops. Networking, HTTP control, replay bookkeeping, and frame
  stepping therefore keep reaching their normal outer-loop boundaries.
  Single-player retains the original paused-timeline behavior; a local menu
  in multiplayer captures only local input/presentation and does not stop the
  authoritative simulation. Window close propagates to mission exit, nested
  confirmations retain the picker underneath, stable save filenames protect
  selection across list mutations, and ordinary HTTP screenshots capture the
  presented topmost pause UI. Settings rows carry typed actions, enabled state,
  labels, and help text; large pages use bounded 12-row pagination, so the
  integrated Gameplay page exposes all 42 current settings without index or
  hit-box remapping. The native desktop data-folder chooser remains
  a synchronous OS dialog launched from the cooperative Options state.
- **Non-blocking mission-end leaderboards and verified-run consent.** The
  mission-end overlay opens verified score/time boards after wins, losses, and
  interrupted attempts, with tabs for every board applicable to the run.
  Board fetches, server-authored submission offers, compact-replay export,
  participant authorization, and upload are frame-polled tasks on native and
  browser builds; none waits inside rendering or pauses multiplayer.
  Submission is available only for locally eligible won missions, defaults to
  per-run consent, and has an explicit default-off **Always Submit Won Runs**
  preference. The independent **Mission-end Leaderboards** presentation
  setting defaults on. Both controls are available under Options →
  Leaderboards; neither can disable canonical replay/history capture or alter
  the ranked wire/storage format.

  Exactly one canonical compact replay and its exact starting campaign enter
  an upload. The controller rejects substituted offers, non-canonical replay
  bytes, wrong mission/campaign evidence, altered signed envelopes, invalid
  Ed25519 signatures, and incomplete or foreign multiplayer signer sets.
  Native and browser single-player signing delegate to the shared durable
  leaderboard identity rather than creating another key store. Multiplayer
  co-signing is an injected authenticated, session-bound typed task so the
  networking layer can gather every participant signature without exposing a
  generic signing primitive.

- **Self-describing save metadata.** Every newly written manual, quick,
  rotating-autosave, continue, restart, and Sherwood save freezes its
  wall-clock timestamp, mission title, stable player-profile identity, and
  player name into both the payload header and lightweight slot index. The save
  picker shows mission/player provenance, honest relative age (including future
  times after clock correction), exact local time, and expanded campaign
  details. The per-profile **Detailed Save Metadata** option switches to a
  compact presentation without discarding stored metadata. Native saves
  require provenance, the multiplayer diagnostic-authority marker, and exact
  deterministic Spellforge package/journal state; incompatible older Rust
  schemas are rejected, and only the separate original-game importer may
  produce incomplete historical detail.

- **Mission badges and campaign achievements.** The accepted catalogue now has
  24 achievements: 10 mission badge types and 16 campaign achievement types,
  with **Clean Hands** and **Ghost** available at both scopes. The complete
  conditions and scope decisions are in [Achievement proposals](ACHIEVEMENT_PROPOSALS.md).
  [Gameplay validation](ACHIEVEMENT_VALIDATION.md) records live observations,
  original-data checks, and remaining proof routes. Mission metadata now filters
  the campaign badge catalogue as well as simulation results; the ransom feat
  requires the authored dispatch flag, and pursuit escape requires the party
  to remain on-map.
  The former all-enemies-stashed achievement is removed, and its stable ID 3
  is retired. New badges include **Ruthless**, **I'm off home**, beggar-info
  completion, banner purchase challenges, **Leave Everyone Standing**,
  **Not a Scratch**, and the generic-only optional-mission challenge.

  Campaign-only feats cover story completion/ransom, companions, permanent
  losses, camp workers who contribute in the field, civilian deaths, and
  one-time coordination, pursuit escape, capture, beer, wasp, and six-sling-
  knockout feats. Rich-civilian knockouts are counted by identity and remain
  credited after waking. The final hint payment is distinct from a later
  unrewarded donation. Deaths after initialization count for Kill a Civilian,
  including NPC and environmental/script damage; pre-existing corpses do not.
  Damage followed by healing still fails damage-free challenges.

  Tracking establishes a post-initialization NPC baseline, records authoritative
  effects, and includes its evidence in save/replay/rollback state. Clean Hands
  retains its configurable NPC-death rule; Ghost remains independent of kills.
  Campaign history practice restores progression deeds, so it cannot erase
  permanent losses or manufacture previous camp work.

  Results and metrics are frozen into the canonical attempt, and an exactly-once
  host attestation decides whether the badges may enter campaign and lifetime
  history. Custom, cheated, headless, and replay-playback runs remain auditable
  but do not award icons. A normal history replay is an eligible campaign
  practice attempt, so a player can return to one mission and earn a missed
  badge after the fact. Campaign badges, detailed debrief conditions, exact
  selected-character sword/bow XP, the speedrun clock, and the optional
  top-left achievement trackers have separate settings. Fresh profiles show
  campaign badges and debrief details but leave the HUD trackers and detailed
  XP off; settings documents that predate these presentation fields leave all
  of those presentations off without discarding calculated history. The authoritative rules live in
  `crates/robin_engine/src/achievement.rs`; presentation lives in
  `crates/robin_rs/src/achievement_hud.rs`.

  Mission-only badges do not create duplicate campaign awards. **Clean Hands**
  and **Ghost** require the badge on every successful canonical mission in one
  completed campaign path; one-time campaign feats need one eligible success.
  The campaign catalogue is paged, while the classic map shows compact totals.
  The expanded deterministic state uses native save 74, replay 32, network 41,
  campaign history 3, and profile history 4. Older native schemas are rejected;
  original-game imports retain incomplete evidence. The required path is
  frozen by campaign run ID and terminal sequence when the Original campaign
  completion boundary is crossed. Lost, failed, and interrupted attempts stay
  in full history but never expand that won-mission set. A later practice
  replay may fill evidence for a mission already in the envelope, but cannot
  add a mission or combine evidence from another campaign. Completed envelopes
  live in the profile's lifetime archive and survive campaign reset.
  Incomplete original-game imports are shown as unverifiable until real eligible
  evidence exists; import never fabricates an award.

  Deterministic tracker fields and the NPC-death gameplay rule are part of the
  native state contract. This feature therefore advances native saves to v58,
  replays to v18, and multiplayer to protocol v25; obsolete Rust layouts fail
  closed instead of being decoded as plausible achievement evidence.

- **Rotating autosaves.** Single-player missions create interruption-safe
  recovery points at mission boundaries, when the native window or browser page
  is backgrounded, and every five minutes of active gameplay. One ordered
  writer commits an immutable full save payload before publishing a separate
  manifest, retains exactly the newest three generations, drains accepted work
  during clean shutdown, and removes unpublished crash leftovers on recovery.
  Browser records use the same portable JSON value model inside compressed,
  SHA-256-checked storage envelopes; lifecycle events wake the game before
  hidden-page timer throttling and urgent snapshots publish synchronously.
  Autosaves appear only in Load lists and cannot be manually overwritten or
  deleted. The independent Gameplay option is enabled by default. Multiplayer,
  replay playback, and headless automation never autosave.

- **Legendary and custom difficulty.** Player profiles may select the three
  retail presets, a fixed Legendary preset, or a validated Custom ruleset.
  The resolved integer/boolean rules are authoritative simulation state and
  travel through profiles, saves, replay snapshots, and multiplayer welcome
  messages. Legendary continues the retail combat/health/reaction/capacity
  progression and additionally gives hostile soldiers 135% view distance,
  125% view-cone width, and 150% noise sensitivity. Those perception rules
  are independently editable in Custom; Easy, Medium, and Hard remain at
  their original 100% perception. Original-RNG parity maps Legendary/Custom
  back to their explicit retail compatibility preset. Exact Standard/Original boards
  retain immutable Easy/Medium/Hard identities; open boards also accept Legendary
  and Custom settings. Story mode is
  intentionally absent.
  - TODO: what can we do to make legendary harder without being boring?
    - just increasing health is boring
    - increase vision distance or vision cone angle
    - increase hearing sensitivity (or PC make more noise)
    - decrease unconcious time
    - longer distance for archers / crossbowmen (NPCs) and opposite (PCs) - although making items less useful is boring as well
    - smarter enemy AI - e.g. archers should try to avoid and escape hand to hand combat more - normally once you have an archer in normal combat you've basically won instantly. what else?
    - MORE enemies (can we programmatically just increase NPCs in missions?, just add more randomly somehow)
      - make patrols larger (commander + 4 soldiers -> 6 soldiers)
      - add more enemies in same spread (e.g. 5 archers on one walkable surface (wall) -> 7 archers spread out on same surface?)
      - needs tweaking / candidate review
    - no saving / permadeath / no clovers for reviving (?)
    - enemies look around more

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

- **Authoritative Sherwood item trading.** Hosts can sell any stored production
  item for a fixed, documented ransom value through an independently toggleable
  Sherwood panel. The built-in **T** action and the touch/mouse-accessible pause
  menu row share one fail-closed modal request; both require Sherwood, the host
  seat, and the enabled gameplay rule. Explicit **Sell 1** and **Sell 5**
  actions require a second matching activation, then exact stock removal and
  ransom mutation occur only in the deterministic command frame. Replays,
  rollback, multiplayer, saves, and Original-parity policy carry or reject the
  same typed command and configuration.
  Controls, prices, production rationale, and integrity rules are documented
  in [Sherwood item trading](SHERWOOD_TRADING.md).

- **Data-driven mission allegiances.** Hackable JSON missions may assign a
  numeric `allegiance` to each soldier and rescue PC. IDs `0` and `1` preserve
  the legacy Royalist and Lacklandist camps; any `u16` ID is accepted, and
  distinct valid allegiances are mutually hostile unless an optional
  `diplomacy` block declares a symmetric `allied`, `neutral`, or `hostile`
  relationship. The block can also declare a multi-allegiance
  `player_coalition`; coalition members are always allied. Runtime target detection,
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
  Ten launchable test arenas share the `mods/multi-team-demos` mod:
  three-way, ten-way, every soldier/PC profile in unique-allegiance circles,
  autonomous Robin versus Little John, four armies of twelve soldiers, and a
  four-army matchup where each faction uses a distinct soldier grade, and an
  all-hero ten-way circle free-for-all, four archer companies in crossfire,
  twenty Robins against twenty Black Knights, and four champions with mixed
  soldier retinues.
  Relationships can change deterministically during play through
  `GetDiplomacyRelationship` / `SetDiplomacyRelationship`, the
  `DIPLOMACY <first> <second> <allied|neutral|hostile>` console command, or
  serialized player commands. A change immediately reconciles perception,
  AI targets, swordfights, projectile protection, cursors, and minimap colours;
  neutral factions use amber. Options → Gameplay can disable authored
  diplomacy or all NPC-vs-NPC faction wars, and Options → Graphics can disable
  relationship-aware colours. Per-mission statistics retain deterministic
  per-allegiance soldier encounters, survivors, deaths, and player-caused
  deaths alongside the legacy aggregate rows. The three-way test arena is an
  editable allied/neutral/hostile example.

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

- **Planned quick-action queue.** Holding the rebindable `Plan Quick Actions`
  control (Shift by default) switches the portrait
  action buttons, cursor, and projectile preview to a separate planning state:
  selecting Bow or an item does not equip it, stop the hero, or otherwise
  mutate the live PC. World clicks use the QA macro system and execute in
  order, with no three-action queue limit and without consuming the Original
  three explicit macro slots. The first action starts as soon as the actor's
  existing work finishes; its slot is consumed immediately, while pending work
  remains visible in an independently animated strip. Plan-double-click upgrades
  the newest pending movement to a run. Planned actions may be selected even
  when their live ammo is empty, and releasing the control or plan-right-clicking
  clears the planned action without touching the live PC. Bow arcs are previewed from the last
  queued or live movement destination, so targets can be planned from the
  position the hero will actually occupy. Shield and Big Shield retain their
  exact protectee-then-danger-point interaction. When the separate tactical-unit
  control extension is enabled, directly controlled units can queue formation
  movement and combat, with group-portrait queue feedback. Touch-capable builds
  expose a sticky plan/cancel HUD button. The per-profile
  Gameplay setting disables all live planning UI/input and defaults on;
  Original-parity replay forces it off.

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
  Windows system font in the original build too. It also carries 19 engine UI
  PNGs, including the allied controls and villain portraits. Packaged native
  targets validate the strictly sorted 32-file size/SHA-256 inventory in
  `core-overlay-manifest.json`; desktop startup validates the exact physical
  tree before mounting it, while Android packages it as a retail-content-free
  asset root and validates every entry before UI startup. Browser builds
  explicitly preload only their build-reachable font/UI subset. See
  `assets/core-datadir/README.md`.

- **Runtime language switching.** Main-menu Options discovers and validates
  every installed retail locale and offers an application-global `Automatic`
  or explicit language choice without restarting the game. Active-mission and
  pause-menu Options deliberately do not expose the selector; the selected
  language applies when the next session is built. Applying a language
  atomically replaces loose-datadir or shipping-bundle lookup, rebuilds eager
  menu presentation caches, and returns to Options. Text never silently falls
  through to a different language; only missing recorded speech and
  cinematics may use an installed English pack. Shipping datadir format v15
  preserves complete per-locale overlays for native, Android, and browser
  packages; older manifests fail loudly and must be regenerated. Generated
  character names and persisted save labels remain frozen, multiplayer mission
  text and playback are client-local, and logical speech timing comes from
  the required English core audio timing table, independent of installed voice
  packs and the client's active presentation language.

- **Hackable JSON levels.** Every subdirectory of `mods/` is registered as an
  overlay datadir at startup, and any overlay may ship an editable
  `Data/Levels/<mission>.level.json` geometry descriptor (title, spawn point,
  walkable polygon, architectural volumes) that expands into normal level
  structs at load time — no legacy RHP/RHM/terrain encoding involved.
  Backgrounds and minimaps can be plain PNGs, optionally paired with a 16-bit
  `<map>.occlusion-depth.png` for continuous sprite occlusion. Discovered
  levels get a main-menu entry (descriptor `title`) and can be launched
  directly with `--mission <name>`; they run as unscripted sandboxes.

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
  groups retain the helmet portrait.
  Level JSON patches can preserve compiled-script actor indices while
  overriding beam-me, rescue-PC, and tied-prisoner visuals. A per-mission
  `.descriptors.patch.json` can override popup, short-briefing, and dialogue strings
  while retaining the base mission's descriptor pictures and timing.
  Mod-added character profiles keep their visible name and NPC exclamation
  bank separate from the internal RHS profile key, so promoted villains retain
  their own voices. NPC sprite sets used as PCs also fall back to compatible
  authored attack rows for target interactions they do not natively animate.
  Promoted NPCs can import their retail soldier combat statistics while
  retaining a playable character template's action layout. Mods can remove
  inherited contextual actions unsupported by the NPC sprite.
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

- **PC-centred fog of war and re-hide intelligence.** Options → Gameplay
  includes an off-by-default `Fog of War` toggle. The world and minimap now use
  unexplored, explored/dimmed, and currently visible polygon regions derived
  from active playable-PC vision plus opaque line of sight. Hostile actors disappear outside
  current sight after a short deterministic hysteresis and leave a stationary
  minimap marker that fades over ten seconds. Listen grants a temporary
  reveal, allied NPCs do not create reveal circles, and script-authored minimap highlights
  pierce fog. The authoritative current region is the exact union of the PCs'
  Original-compatible 3D shadow polygons, and explored terrain is their
  cumulative polygon union. World and minimap presentation rasterize those
  regions at up to one texture pixel per map pixel; GPU linear sampling
  antialiases that pixel-scale result. The live boundary is evaluated from each PC's exact position using
  1.2× the mission's standard vision range, producing smooth, progressive
  motion without cell transitions. Both polygon sets are hashed and serialized
  in saves, rollback, replays, and multiplayer snapshots; the alpha texture is
  only a cached presentation derivative. Visibility is refreshed each simulation
  frame through the same 3D obstacle projection used by the view overlay.
  Actors query the current ground polygon directly, keeping disclosure exactly
  aligned with the clear ground beneath them. World presentation constructs the
  authored closed 3D mesh for every sight obstacle (top triangles and side
  triangles) over a `z=0` map plane. Player shadows are projected onto each
  triangle's own plane, while pairwise affine isometric-depth clipping retains
  only the camera-nearest surface at every projected position. The resulting
  visible and explored unions therefore expose real walls, roofs, ramps, and
  elevated ground without facade expansion, bounded-top guesses, painter-order
  dependence, or whole-building projection-hull leaks. Unchanged observers
  reuse a geometry-signature cache; eye movement or any static/dynamic obstacle
  change invalidates it exactly. The cache is presentation-only and is omitted
  from saves, rollback snapshots, replay payloads, and deterministic hashes.
  Large patch animations
  defer visibility to the final per-pixel fog composite rather than being
  rejected by a single hidden anchor point, so replacement artwork is clipped
  by the exact fog boundary without clearing terrain behind it.
  Camera
  movement alone reuses the cached GPU texture. This state is deliberately
  separate from the Original's
  permanent `blipped` identity-discovery flag, so `UNBLIP` behavior is
  unchanged. Disabling the setting takes the original rendering/input path;
  original-parity tooling forces it off. Full-map exports also default it off
  and expose `--fog-of-war` when a fogged capture is desired.

  This adds serialized deterministic state and a replayed toggle command.
  The complete current graph, including cold custom-mission identity, uses
  native save schema 72, replay schema 30, and multiplayer protocol 39;
  obsolete incompatible Rust layouts are rejected.

- **Original-compatible 3D view overlays.** View cones now project complete
  opaque obstacle volumes independently onto the ground and every visible
  platform plane. As in the Original, feet and head shadows are evaluated
  separately and an area remains visible when either endpoint can be seen.
  This replaces the earlier two-vertex ground-wedge approximation, including
  its incorrect elevated and through-wall results. The projection core is
  renderer-independent and accepts arbitrary polygonal domains and target
  surfaces, so precise fog masks can reuse the same geometry instead of
  maintaining a second obstacle model.

- **Fog/night-tint all sprites option.** Options → Graphics now includes a
  `Fog/Night All Sprites` toggle. On fog and night missions it applies the
  generated ambiance sprite variant to Day-based world sprites, including
  bonuses, scrolls, animals, and mobile child sprites that the original leaves
  in the Day palette. Animation-backed FX and targets already load
  ambiance-specific pixels and are excluded to prevent double tinting. New
  profiles enable it by default; the toggle can restore original behavior.
  `render_mission_map` exposes explicit `--fog-tint-all-sprites` and
  `--no-fog-tint-all-sprites` overrides for reproducible A/B captures.

- **Quick-action recording cursor pulse preference.** The classic three-slot
  recorder uses the original opaque RGB565 shadow pulse, from black to pale
  yellow-green. Its 20-step triangle runs on a cursor-local clock (40 ms per
  step), independently of simulation, pause, rewind, and replay time. Options →
  Graphics → `Quick-Action Cursor Pulse` can disable it. Bow-target shadow
  colors apply normally outside recording. The effect's animation state is
  never saved or recorded.

- **Mission-start full-map renderer**
  (`crates/robin_rs/examples/render_mission_map.rs`). Loads a mission through
  the regular engine and GPU renderer, captures the complete map after the
  requested number of normal simulation frames (`--frame`, default zero), and
  writes a HUD-free PNG. `--reveal-all` (alias `--unblip-all`) switches every
  NPC from its blip silhouette to its normal character profile before capture;
  `--headless` keeps the screenshot renderer's GPU-backed window hidden.
  Complete exports bypass gameplay fog unless `--fog-of-war` is requested.
  `scripts/render_all_mission_maps.sh [OUTPUT_DIR] [FRAME] [DATA_DIR]` renders
  every shipped mission using the full-game profile mapping and human-readable
  mission titles as filenames.

- **Local script-RPC HTTP server** (`crates/robin_rs/src/http_server.rs`).
  Loopback HTTP access to script natives, engine inspection, screenshots, player
  commands, and replay control for external debug tools, test harnesses, and AI
  drivers. See [HTTP automation server](JSON_SERVER.md) for setup, endpoints,
  and examples. The HTTP transport is desktop-only; Android disables it, while
  browser builds expose the same queue through the `rh_rpc` JavaScript bridge.
  Replay recording uses a mission-generation-scoped, 64 MiB segmented spool
  that publishes only complete flush boundaries and fails closed on overflow
  or durable-writer errors. Export has single-flight backpressure and produces
  the canonical compact bitcode replay through `robin_replay_format`, with no
  alternate JSONL export or ranked format.

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
    restrictions are documented in [Upscalers](UPSCALERS.md).

- **Deterministic replay and rollback checking**. Sessions can be recorded to
  JSONL, replayed from disk or compact `rhrec-...` strings, and checked with
  per-frame state hashes. Ranked ingestion applies the same post-decode frame,
  metadata, campaign-byte, and per-frame work ceilings to raw JSONL and compact
  containers; compact input additionally has independent base64, compressed,
  decompressed, and zstd-window limits. The rollback checker periodically
  replays recent frames from a snapshot and compares the reconstructed engine
  state against the live state to catch nondeterminism. Explicit playback
  requests are decoded before Engine construction and fail fatally if their
  required header cannot be read; playback never substitutes a multiplayer or
  default RNG seed. Gameplay randomness uses one serialized Engine-owned
  `fastrand` stream instead of Original's process-global C RNG; parity is
  reviewed at ranges and call-site order rather than bit-identical rolls. The
  reviewed inventory and host-only exceptions are enforced in code; typed
  serialized-stream labels and a separately typed seed-derived authoritative
  peasant-name generator, plus a structural source test, reject unreviewed
  gameplay RNG additions.
  Each mission recording is a directory of append-only JSONL chunks, indexed
  by `mission.json`. Every save capture (including background autosaves and
  Restart) writes and flushes a marker before the save can be published. Saves
  reference the directory, chunk, and marker, binding the reference to the
  complete captured payload. Markers occupy host-only records: queued gameplay
  commands execute afterward, without a fabricated simulation tick.
  Every load starts a new chunk with both a chronological predecessor and the
  restored save reference. Mission-wide ordinals preserve all abandoned gameplay,
  saves, and reloads, including when the application is restarted. Export and
  leaderboard submission use one self-contained artifact assembled from the
  complete chronology; playback needs no original save files. An individual
  chunk path replays history through that chunk, so earlier attempts remain
  watchable after subsequent loads. `--record <directory>` creates a new mission
  directory; `--replay <directory>` plays its complete history. Existing
  standalone JSONL and compact replay files remain supported. Browser and native
  playback consume recorded terminal updates without opening live debriefing or
  leaderboard flows, so abandoned wins/losses can be followed by another restore.
  Timeline scrubbing addresses recording ordinals, so repeated simulation
  frames across saves and reloads remain individually reachable. Backward seeks
  reconstruct from the mission start; forward seeks execute every intervening
  record. Recorded simulation gates remain authoritative during modal playback.
  Native and wasm use fixed-width random index draws for campaign names and
  simulation shuffles, preserving the native stream across both platforms.
  Current native save schema is 75 and replay schema is 33. Missing or invalid
  referenced history is reported explicitly; it cannot become leaderboard
  evidence. Fully verified marker restores qualify for the normal leaderboard:
  verification executes abandoned gameplay too, and restores only states derived
  from verified save markers. The mission directory retains the original signed
  admission for resumes across application restarts. Release rules permit these
  complete histories; embedded foreign snapshots remain ineligible. Browser
  chunks persist in localStorage, with explicit storage/quota failures; moving
  this storage to IndexedDB remains a performance and capacity improvement.

- **Original-game parity traces**
  (`crates/robin_parity/src/original_parity_replay.rs`). A diagnostic runner
  streams the neutral JSONL trace emitted by an instrumented original game,
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
  with no broker, master server, or configuration. Matchmaking `/1` host
  announcements are signed by the persistent game identity, expire after a
  short validity window, bind the advertised endpoint to the signer, and use
  per-host issuance watermarks so captured older lobby state cannot roll a
  listing backward. The current design is predictive rollback netcode rather
  than strict "wait for every peer before ticking" lockstep. Inputs older than
  the retained correction horizon now force a complete transport reconnect
  and fresh authoritative snapshot instead of being applied at the wrong
  frame.

- **Unified mission timeline and non-blocking multiplayer UI**. Rewind,
  multiplayer correction, and rollback verification share one mission-owned
  command journal with dense recent and exponentially retained checkpoint
  tiers. Snapshot/load adoption seeds an explicit checkpoint at its exact
  frame, including between normal sparse boundaries. Blocking gameplay modal
  traffic uses client proposals and host-only decisions; remote peers cannot
  choose the host's restart or load outcome. Campaign map/description and
  launch confirmations, cross-mission QuickLoad confirmation, pseudo-mission
  debrief, lost-Sherwood, and terminal mission-state/debrief/load flows retain
  state across outer frames so networking and replay services keep draining
  while simulation is paused. A player's pause menu is a local overlay in
  multiplayer and does not stop the shared simulation. HTTP timeline stepping
  accepts typed modal outcomes, defaults to automation-friendly auto-dismiss,
  validates each result against its modal kind, and reports or blocks unresolved
  UI explicitly. Ordinary keyboard/HTTP timeline movement is disabled during
  multiplayer; explicit host automation must opt into synchronized stepping,
  after which every peer reconnects from the resulting authoritative snapshot.
  Local keyboard and HTTP pause changes are rejected in multiplayer so one peer
  cannot stop only its own timeline.

- **Host-authoritative multiplayer session transitions.** Load, Restart,
  QuickLoad, and Sherwood campaign launch use a prepare/ready/commit barrier.
  The host encodes authoritative save or campaign state once, every connected
  peer validates and retains those identical bytes, and only then do all
  participants tear down the old mission transport and enter the next ready
  barrier. Load/Restart controls remain disabled for clients and throughout a
  transition. Multiplayer Save and QuickSave create tagged local diagnostic
  captures: connected load pickers hide them and the central transition path
  rejects them even if UI filtering is bypassed. Sherwood campaign UI remains
  non-pausing, but only host-authored campaign commands can mutate simulation
  state. Client modal choices remain visible host proposals; only the host can
  publish the decision that closes a shared modal. Replacement missions
  consume a one-shot continuation containing the same session id,
  authenticated owner/seat roster, expected player count, and pinned relay
  route. Durable browser identities and process-held native client identities
  therefore reclaim the same seats without nickname authority or a stale-relay
  reconnect race. The native server rejects peer-authored deterministic
  settings, campaign mutations, modal decisions, and seat-lifecycle commands
  before broadcast; the engine repeats that check at deterministic command
  admission so replay/rollback cannot bypass transport authority.

- **Authenticated browser multiplayer**. A native host can publish a
  30-minute, fragment-only `rhmp3` invitation for
  `https://robinhood.phiresky.xyz/`. Browser peers use iroh's
  relay-over-WebSocket transport with the protocol-34 game wire,
  prove a durable non-extractable identity through an isolated typed signer,
  and reclaim only their parked seat generation. Demo and Full joins fail
  before boot unless the ticket-selected engine artifact, exact native
  Data/locale closure, and every browser package byte agree. Reconnect adopts
  an authoritative replacement snapshot even when it predates the abandoned
  prediction future, then clears future inputs/hashes/history. Only the host
  records the canonical server-ordered replay. Relay observability is stated
  in the invitation UI, and browser-link publication is a default-on persisted
  privacy setting that can be disabled without affecting native iroh play.

- **Deterministic Spellforge Lua missions**. A versioned package contract
  supports upstream same-name Lua replacement plus explicit SCB augmentation.
  The engine callback driver dispatches Initialize/PostInitialize, one-second
  timers, three-second victory checks, Finalize, messages, keys, and all actor,
  target, scroll, zone, and waypoint events. Native calls suspend through the
  same synchronous engine operations as SCB. Exact package bytes/hash and a
  nested event/native tape are authoritative state. The tape is an append-only
  persistent journal: per-frame rollback clones share their immutable history,
  state hashes consume a rolling SHA-256 chain instead of walking every prior
  event, and serialization flattens the journal to a non-recursive wire list.
  Event, native-call, argument, nested-transcript, retained-byte, and combined
  package/tape snapshot ceilings are part of the executable ABI; reaching one
  aborts the mission with a typed resource-limit diagnostic. Arbitrary Lua heap
  and callback closures reconstruct after save/load, rewind, rollback, replay,
  headless execution, multiplayer snapshots, and reconnect without reapplying
  engine side effects. Snapshot admission recomputes the package digest and all
  journal accounting before state hashing or level attachment. Missing
  packages, hash/version/ABI mismatches, script failures, and tape divergence
  are fatal rather than SCB fallbacks. Profiles
  can disable Spellforge missions in Gameplay settings; required replacement
  missions then stop with an explicit launch error. Native and browser builds
  execute gameplay through the same safe-Rust Lua 5.1.1 VM and conformance
  suite; browser launchers can hand archive bytes directly to the in-memory
  package loader, with no JavaScript evaluation or Emscripten side runtime.
  The author contract, limits, checker workflow, corpus gate, and unresolved
  product-policy seams are documented in `docs/SPELLFORGE.md`.

- **Cold custom-mission save/replay identity.** Every current native save and
  replay carries a mandatory bounded descriptor for its mission basename,
  profile proto/map identity, and immutable asset source. Archive missions
  record exact mission/shared ZIP hashes and sizes, the selected nested RHM
  entry, and safe logical installed and/or distributed-cache locators; absolute,
  traversing, platform-ambiguous, control/invisible, and sentinel identities
  are rejected. Cold load verifies and mounts those exact bytes before engine
  construction and fails on missing or mismatched content. The package already
  embedded in a save/replay remains the sole Lua authority. Local and browser
  playback use a separate 64-MiB bounded isolated-worker lane that fits a
  maximum valid Spellforge package; ranked/server admission retains its
  stricter 16-MiB default and does not inherit local content trust.

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

- **Touch camera gestures and native-refresh presentation**. Touch input now
  classifies taps, drags, double-taps, and two-finger transforms without
  leaking cancelled pointer actions into gameplay. World gestures support
  anchored pinch zoom, pan, bounded inertia, and UI/minimap exclusion, with an
  independent Gameplay toggle. A separate Graphics toggle presents and
  interpolates at the display's actual cadence while deterministic simulation
  remains fixed at 25 Hz; 60/90/120/144/240 Hz are covered without a
  hard-coded refresh-rate policy.

- **Level selection tree and campaign history.** Campaign progress is
  available through selectable Classic Map, prerequisite/progress tree, and a
  modal Sherwood Hall of Deeds exhibit grid. Every Rust campaign always
  retains immutable full-fidelity records for every attempt (including
  losses and practice replays) and derives totals/bests from those records.
  Only original-game saves are imported; their limited status/recent-mission
  data remains explicitly incomplete. See `docs/CAMPAIGN_HISTORY.md`.

- Item reliability rebalances are implemented as independent Gameplay
  settings. Direct apples can interrupt active swordfights; wasps acquire
  valid initial targets within 75 instead of 50 units; Will Scarlet's stone
  can use the sibling throwable base range 300 instead of its shipped 200;
  resistant VIPs/riders/Stuteley are skipped while a net catches other people
  in its original strict 40-unit circle; and outdoor non-VIP soldiers with
  authored beer value zero accept ale at minimum potency 20. Positive authored
  beer, net terrain crumpling, ally capture, and purse behavior are unchanged.
  Each rule defaults on independently; Original-parity sessions force them off.
- Ground-targeted stone noise distractions are implemented. With the
  independent gameplay toggle enabled, a real Stone projectile may target
  valid ground and emits one deterministic 240-unit noise stimulus on its
  terminal impact. Guards use the existing heard-noise search behavior. The
  command, target layer, replay, rollback, multiplayer, and quick-action paths
  remain authoritative; its additional impact cue is independently toggleable.
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

### Additive hackable sprite mods

- Overlay mods can append soldier profiles through
  `Data/Configuration/profiles.patch.json` without replacing the
  retail CPF profile table.
- The sprite authoring tool can extrapolate one additional combat-stat tier
  from two adjacent retail tiers and emit concrete JSON Patch operations.
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

### Legendary enemy-placement candidate generator

- `docs/legendary-enemy-proposals/generate.py` builds a deterministic,
  browser-based mission review from a hackable datadir. It renders authored
  start facings, walkable surfaces, active and inactive walking paths, and
  before/after crops for each candidate.
- The generator accepts mission, surface, background, location, and output
  arguments. It resolves every used soldier profile through the hackable CPF
  profile table, classifies its gameplay role, and renders the profile's actual
  directional sprites. This supports full-game missions rather than only the
  Leicester demo roster.
- Candidate rules expand safe officer/subordinate groups with paired ranks,
  distribute cloned groups evenly along suitable patrol paths, reinforce
  established archer lines only on locally thin wall-like surfaces, create
  narrow guard posts, cover open route ends with sentry pairs, and find ground
  gaps outside both authored starts and complete active patrol corridors. The
  rejected guard-to-archer conversion rule is deliberately absent. Unsafe
  candidates retain their attempted coordinates and render as red-highlighted
  before/after diagnostics rather than being silently omitted. Ambiguous inputs
  that cannot produce one meaningful placement are listed separately as not
  placeable.
- Second-patrol candidates prefer a 50% cycle offset, then alternate outward
  one percentage point at a time through 40–60% until a valid formation is
  found. Eligibility requires the officer itself to own the group's one
  unambiguous path and start within 80 pixels of it; subordinate-only path
  references do not turn a stationary formation into a patrol. Each attempt
  aligns the officer-to-escort axis and authored facings to the target
  walking-path direction. The additional route frame fits the complete shared
  path while dimming the map and hiding unrelated paths, surfaces, and soldiers.
- Presets and scalar command-line options control patrol count and spacing,
  officer roster targets, stationary groups, guard posts, route-end sentries,
  wall-archer density, per-sector limits, and whole-mission reinforcement
  budgets. The full-game batch report keeps accepted, alternative,
  budget-limited, conflicting, and low-confidence candidates visible so a
  designer can audit what the rules attempted instead of only seeing winners.
- The `legendary` preset's reviewed selections are exported as an embedded
  runtime manifest. On Legendary difficulty, the engine deterministically
  clones the selected authored soldier profiles after loading live navigation
  geometry and before spawning mission entities. Every placement is resolved
  against the runtime walkable grid, authored soldier indices remain stable,
  officer subordinate lists are extended explicitly, and cloned patrols receive
  their own commander/follower ownership. Normal, Hard, and custom difficulty
  missions are unchanged; stale manifests fail mission startup loudly if their
  expected authored roster no longer matches the loaded mission.

## Todo

- **Multiplayer follow-ups**
  - Sign matchmaking announcements with the game identity key so a peer
    cannot advertise a game under another host's endpoint id.

- Add a method to unhorse horsed soldiers without killing them; no-kill runs
  are annoying with horses.
  - Add an option for Merry Men to knock people out instead of killing them.
- A freely walkable world-space Sherwood Hall of Deeds (the modal exhibit
  grid is implemented).

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

## Reversible background patch animations

Gameplay options includes **Reversible Background Patches (Next Launch)**
(`gameplay_config.reversible_background_patches`, default `false`). Start a new
mission after enabling it. Animated patches that were authored as one-shot
remain active: triggering them again plays their transition backwards and
restores the original background, collision/pathfinding obstacles, sight
obstacles, occlusion masks, sectors, lines and connected door rights. Subsequent
triggers alternate both states. Existing reversible patches retain their normal
behavior; patches without a transition animation retain their original policy.

The policy is recorded in simulation configuration and resolved patch state,
so current-format saves/replays retain it. Original-parity mode disables it and
standard ranked policy does not admit it. Existing sessions keep their resolved
patch policy. Target callbacks remember the animated patch group they apply;
subsequent activations replay that group directly, bypassing script one-shot
guards without repeating mission messages or rewards. Locks and transitions
prevent repeated activation while a mechanism is unavailable or still moving.
Captured controls retain their usable sprite instead of accepting a queued
one-shot freeze pose (Lincoln's cut drawbridge rope otherwise becomes entirely
transparent and cannot be clicked again). Other targets retain their authored
animation behavior.
TODO: patches applied later by delayed script commands need authored trigger
metadata; only patch changes made during the target callback are captured.

This state changes the native formats to save 73, replay 31 and multiplayer
protocol 40. Earlier Rust saves/replays retain their original files but are
rejected by the existing strict schema checks; they are not silently migrated.


### Replay recording across arbitrary save loads

Replay schema 33 embeds the exact serde JSON save before post-load fixups when
loading a save without a reproducible timeline marker. This includes saves from
earlier sessions, asynchronous autosaves, and saves captured after commands in
the current frame. Playback restores engine, sound, and persistent game state
through the normal load path, without depending on the original save file.
Clean same-session saves still use compact load-back markers. Loading after a
terminal recording starts a new recording with the embedded restore boundary.
Save loads remain ineligible for ranked submissions.
