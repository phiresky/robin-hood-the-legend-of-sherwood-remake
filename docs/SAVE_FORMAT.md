# Save format history

Current save format version.

Bumped on every incompatible change to the serialized fields.
The counter starts from 1.

## History
- **v1**: initial Rust format. `ElementData.sprite` was skipped; the
  embedded `PositionInterface` + sprite animation state did not persist.
- **v2** (2026-04-20, PI-into-Sprite refactor): `ElementData.sprite` is
  now fully serialized. The saved `Sprite` carries its `PositionInterface`
  (position / direction / layer / sector / material) plus the animation
  state (`current_row`, `current_frame`, `frame_count`, `last_action`,
  …). Arc-shared script caches re-hydrate from the sprite cache on load.
- **v3** (2026-04-29, engine-state cleanup): small sprite/titbit runtime
  values that still live inside engine-owned structs now serialize instead
  of resetting through `#[serde(skip)]`: sprite water-titbit cadence,
  sprite bbox/center, and titbit blink/dotted-line counters.
- **v4** (2026-04-29, engine-state cleanup): door after-patch lock bits
  serialize with the active lock bits so patch swap/revert behavior
  survives save/load.
- **v5** (2026-04-29, engine-state cleanup): AI door/building caches
  serialize, including live building occupant lists and soldier-register
  mappings.
- **v6** (2026-04-29, engine-state cleanup): NPC patrol route IDs
  serialize with AI controller state so alert-route switches survive
  save/load.
- **v7** (2026-04-29, engine-state cleanup): NPC actor-script
  FilterAIEvent override metadata serializes with the bound AI
  controller.
- **v8** (2026-04-29, engine-state cleanup): NPC initial guard-post
  position and facing direction serialize with AI controller state.
- **v9** (2026-04-29, engine-state cleanup): NPC focus-sync gate
  state serializes so explicit focus clears are not undone after load.
- **v10** (2026-04-29, engine-state cleanup): AI think recursion
  depth serializes with controller state instead of hiding behind
  `#[serde(skip)]`.
- **v11** (2026-04-29, engine-state cleanup): pending NPC MYTALK
  callback flags and instant music-change latches serialize with AI
  controller state.
- **v12** (2026-04-29, engine-state cleanup): NPC AI frame/building
  context caches and current max-visibility cache serialize with the
  controller instead of resetting through skipped fields.
- **v13** (2026-04-29, engine-state cleanup): first batch of AI
  pending work queues serializes with controller state: patrol
  direction broadcasts, order intents, queued stimuli, cross-NPC
  actions, and self-stimuli.
- **v14** (2026-04-29, engine-state cleanup): AI pending engine
  mutation requests for halt/deactivate/swordfight/detectable updates
  serialize with controller state.
- **v15** (2026-04-29, engine-state cleanup): AI pending target/focus
  requests serialize with controller state.
- **v16** (2026-04-29, engine-state cleanup): AI pending state-change,
  view recovery, detectable-object recovery, and guarded-PC requests
  serialize with controller state.
- **v17** (2026-04-30, engine-state cleanup): AI pending sequence,
  posture, waypoint-script, panic, and script-seek requests serialize
  with controller state.
- **v18** (2026-04-30, engine-state cleanup): VM/native pending nested
  script calls serialize instead of being silently dropped.
- **v19** (2026-04-30, engine-state cleanup): Tick side-effect queues
  serialize/hash if they ever leak into an engine snapshot.
- **v20** (2026-04-30, engine-state cleanup): AI entity-view and
  sight-obstacle dispatch caches serialize/hash with global AI state.
- **v21** (2026-04-30, engine-state cleanup): Script managers serialize
  their immutable decoded program instead of relying on skipped reattach.
- **v22** (2026-04-30, engine-state cleanup): Script native hosts
  serialize their profile-manager attachment with host state.
- **v23** (2026-04-30, engine-state cleanup): AI controllers no longer
  cache per-NPC hiking-path Arcs; path data is threaded through AI context
  and script host static data.
- **v24** (2026-04-30, engine-state cleanup): enemy AI pending archery
  release requests and sword-strike cooldowns are now serialized as
  simulation state.
- **v25** (2026-04-30, engine-state cleanup): enemy AI level-load profile
  and combat caches serialize with the owning AI state.
- **v26** (2026-04-30, engine-state cleanup): in-flight actor sweep,
  jump, rider-charge, push-followup, and roll side-effect state serializes
  with actors.
- **v27** (2026-04-30, engine-state cleanup): PC quick-action sequences
  and hero speech suppression state serialize with PC state.
- **v28** (2026-04-30, engine-state cleanup): remaining element-owned
  spatial, combat-display, shield, alert, and patch attachment caches
  serialize with their owner structs.
- **v29** (2026-04-30, engine-state cleanup): campaign pre-mission
  snapshots serialize with campaign state so mission restart survives
  save/load.
- **v30** (2026-04-30, engine-state cleanup): patch level-static
  references serialize with patch state instead of being skipped.
- **v31** (2026-04-30, engine-state cleanup): sequence-manager pending
  immediate actions, condolations, halt latch, and actor progress index
  serialize with sequence state.
- **v32** (2026-04-30, engine-state cleanup): position-interface sprite
  center offset serializes with position state.
- **v33** (2026-04-30, engine-state cleanup): sprite script and
  conversion tables serialize with sprite state instead of relying on
  skipped runtime reattachment.
- **v34** (2026-04-30, engine-state cleanup): door geometry, links,
  jump metadata, patch binding, and action hints serialize with door
  state.
- **v35** (2026-04-30, engine-state cleanup): sector geometry, level
  references, material data, script metadata, archery points, and shadow
  metrics serialize with sector state.
- **v36** (2026-04-30, engine-state cleanup): path graph static data
  serializes through its Arc instead of being reattached after load.
- **v37** (2026-04-30, engine-state cleanup): fast-find level grid and
  shadow data serialize with grid state; per-query visited and detection
  scratch no longer lives on the grid.
- **v38** (2026-04-30, engine-state cleanup): pathfinder A* state no
  longer has hidden skipped fields.
- **v39** (2026-04-30, engine-state cleanup): script VM native host is
  passed as execution context instead of living on serialized VM state.
- **v40** (2026-04-30, engine-state cleanup): patch, sprite, and
  position-interface state no longer accepts missing fields by default.
- **v41** (2026-04-30, engine-state cleanup): mission, campaign, order,
  marker, titbit, and PC metadata no longer accepts missing snapshot
  fields by default.
- **v42** (2026-04-30, engine-state cleanup): engine-inner pending
  queues, macro state, freeze state, and script post-init flags no
  longer accept missing snapshot fields by default.
- **v43** (2026-04-30, engine-state cleanup): sequence manager lookup,
  pending immediate action, condolation, and halt state no longer
  accept missing snapshot fields by default.
- **v44** (2026-04-30, engine-state cleanup): element-owned runtime
  state no longer accepts missing snapshot fields by default.
- **v45** (2026-04-30, engine-state cleanup): AI profile caches,
  tactical state, and pending AI side-effect flags no longer accept
  missing snapshot fields by default.
- **v46** (2026-07-19, nested engine snapshot): `EngineInner` serializes
  its nine current state owners instead of the historical flat field list.
- **v47** (2026-07-19, script effects): mission scripts serialize the
  canonical `script_effects` owner with typed presentation, external, and
  simulation-barrier domains.
- **v48** (2026-07-19, ordered effects and simulation lifecycle): typed
  effects serialize in one emission-ordered stream, sequence continuation
  state is explicit, persistent game state is required, and complete
  `SimConfig` plus mission construction/restart RNG checkpoints serialize.
- **v49** (2026-07-19, snapshot-input ownership): shared-camera transition
  inputs that affect a later tick are required snapshot fields.
- **v50** (2026-07-21, Strangle owner initialization): active ability state
  records whether first-owner Strangle setup has completed.
- **v51** (2026-07-22, Execute owner identity): actor state records the
  selected Execute order identity and one-shot initialization latch matching
  Original-game last-order and new-order semantics.
- **v52** (2026-07-22, specialized AI continuations): AI state records
  result-bearing cross-NPC callback continuations, alert scan progress and
  the final report-before-formation barrier.
- **v53** (2026-08-26, automatic quick-action queue): player state records
  the active automatic queue and its serialized commands.
- **v54** (2026-08-26, resolved quick-action state): records shield danger
  geometry, independent automatic queues, resolved group-move routes, and
  resolved DropAle routes.
- **v55** (2026-08-26, exact movement-route ownership): every stored quick
  action move requires its resolved per-PC destination-sector identity, and
  sequence point Seeks require explicit live-versus-Original route
  provenance. Obsolete Rust saves are rejected instead of re-running
  spatial placement or silently re-enabling reconstructed gate search.
- **v56** (2026-08-28, human actor control): changes authoritative player
  control state and its native snapshot layout.
- **v57** (2026-08-29, full-fidelity campaign history): requires the native
  append-only attempt schema and exact practice-return snapshot. Earlier
  Rust save layouts are rejected rather than migrated.
- **v58** (2026-08-30, achievements): records deterministic achievement
  tracker state and the NPC-on-NPC Clean Hands rule.
- **v59** (2026-08-30, item rebalance): combines that state with expanded
  item rules, cached ale eligibility, and ground-stone command/projectiles.
- **v60** (2026-08-30, authoritative Sherwood trading): combines those
  item-rebalance fields with the
  deterministic trading rule, exact sale commands, receipts, campaign
  ransom, and production-item inventory state.
- **v61** (2026-08-30, resolved difficulty rules): combines the preceding
  feature state with Legendary or
  validated Custom difficulty, including independent hostile-soldier
  distance, cone-width, and hearing modifiers.
- **v62** (2026-08-30, typed runtime sentinel boundaries): combines all
  preceding feature state with nullable AI entity handles and exact spatial
  provenance, preserving live arena slot zero without conflating it with
  absence. The native snapshot layout changed.
- **v63** (2026-08-30, self-describing native saves): every Rust-authored
  save requires immutable mission and player provenance in its header.
  Earlier Rust schemas are rejected; the separate original-game importer is
  the only compatibility path.
- **v64** (2026-08-30, multiplayer diagnostic saves): combines mandatory
  provenance with the required diagnostic marker that prevents a local
  multiplayer capture from becoming authoritative transition input.
- **v65** (2026-08-30, timed missions and runtime ambience): adds authored
  timer/ambience configuration and exact in-progress mission runtime state.
- **v66** (2026-08-30, mission diplomacy): adds the authoritative
  relationship matrix, player coalition, policy switches, per-faction
  mission statistics, and full-fidelity attempt-history evidence.
- **v67** (2026-08-30, advanced combat gestures): adds authoritative
  composite-gesture and quality-damage rules plus resolved gesture state in
  commands, quick actions, active sequences, and active sweeps.
- **v68** (2026-08-30, completed planned actions): adds per-seat planned
  shield protectees and deterministic tactical queue formations to the
  complete tactical-control and automatic-queue snapshot.
- **v69** (2026-08-30, shared-vision fog of war): adds the authoritative
  explored/visible grid, temporary entity intelligence, and fog gameplay
  rule to the complete native engine snapshot. The merged v69 snapshot also
  carries the exact canonical Spellforge package, bounded persistent
  journal, and rolling authenticated digest. No extra intermediate schema
  number is allocated for the previously separate Spellforge branch.
- **v70** (2026-08-30, cold custom-mission identity): every native save
  carries the mandatory bounded mission basename, proto/map identity, and
  exact built-in/archive source descriptor needed before engine creation.
- **v71** (2026-08-31, typed diplomacy relationship entries): the
  authoritative diplomacy matrix is encoded as a canonical ordered list of
  typed allegiance-pair entries instead of an impossible JSON object with
  tuple keys.
- **v72** (2026-09-02, vector fog projection state): current and explored
  gameplay visibility plus their presentation projections are stored as
  exact polygon regions rather than a low-resolution cell bitmap.
- **v73** (2026-09-08, reversible background patches): adds the opt-in
  simulation rule and remembered activation targets needed to reverse
  animated patches after saving, loading, or rewinding.
- **v76** (ordered AI detectable mutations): replaces the four pending
  detectable queues with one authoritative FIFO. Queue order is persisted
  and hashed; older native snapshots are rejected rather than reordered.
- **v77** (canonical script-global vector): replaces parallel imported-vector
  and live ID/value-map storage with one ID-indexed vector. Older native snapshots
  are rejected rather than interpreting their old layout as current state.
- **v78** (single PostInitialize latch and no stale Messenger copies):
  PostInitialize is owned by game state, not a duplicate mission-script
  flag; snapshots no longer retain the unused imported Messenger blob.
  Older native snapshot layouts are rejected. Original-game import is unchanged.
- **v79** (discard unconsumed imported sound and AI claim state): native
  snapshots no longer retain the unused imported sound blob or the
  unconsumed same-frame AI target claims. Original-game import is unchanged.
- **v80** (canonical recording session): replaces the recorder wrapper and
  its write-only sequence ID with the optional recording session itself.
  Older native layouts are rejected; Original-game import is unchanged.
- **v81** (discard unused location/object/camera state): removes the
  computed-location dummy and object repulsive-point copy from snapshots,
  and removes the unused engine-camera scratch fields. Removing hash-skipped
  fields also removes their markers from the state-hash stream.
  Older native layouts are rejected; Original-game import is unchanged.
- **v82** (canonical initial soldier camps): removes the duplicate royalist
  and lacklandist presence flags from AI snapshots and hashes; the existing
  soldier-camp set remains authoritative. Original-game import is unchanged.
