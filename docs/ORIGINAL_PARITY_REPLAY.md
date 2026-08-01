# Original-game parity replay ledger

This is the evergreen record of behavioral changes found through replaying an
instrumented Original game session in the Rust engine. Update it whenever the
trace exposes a new divergence, an implementation changes because of one, or a
regression test closes one. It records logical parity, not incidental identity:
entity IDs may differ when the two worlds can be mapped isomorphically.

## Baseline and contract

- Required Original trace schema: 12. Older schemas are rejected because they
  do not record concrete logical sound-manager speech resolutions.
- Preferred replay start state: `mission_start` with a complete versioned
  campaign snapshot captured before engine initialization, including
  progression, mission state, gang/reservists/mission team, character status
  and inventory, persistent production state, relics, names, values, and
  campaign pointers encoded as stable indices. Schema-12 `loaded_save`
  sessions additionally embed the exact native v48 checkpoint and are admitted
  only through the strict legacy-save importer. Automatic mission-start saves
  can replay when their recorded campaign/config/RNG reconstructs the same
  world. Schema 10 cannot represent a genuinely mid-mission engine state.
- Compatibility: schemas 1–9 are deliberately rejected. Schemas 1–3 cannot
  reconstruct an arbitrary campaign and can silently create a plausible but
  different world. Schema 4 has complete campaign state, but was recorded with
  native i686 x87 extended-precision intermediates and is not a valid numeric
  oracle for the scalar-SSE Rust engine. Schema 5 predates complete recording
  of direct resolved object interactions and their speed changes. Schema 6
  does not record display-driven director completion boundaries, so a
  host-local camera position can change when gameplay sequences resume.
  Schema 7 lacks the explicit simulation-body gate. Schema 8 reaches the
  current Lincoln divergence but cannot distinguish path-input or live
  motion-grid differences, so it is retained only as historical evidence.
  Schema 9 adds that path oracle but can omit action selection/cancellation
  resolved inside a raw mouse message. Schema 10 records those authoritative
  nested resolved inputs, but not the loaded engine checkpoint. Schema 11 adds
  the exact source save plus its producer ABI and integrity metadata, but does
  not identify which pending speech samples the host sound manager resolved.
  Schema 12 adds those ordered concrete resolutions with stable actor identity,
  exclamation ID, selected entry/variant, and decoded duration in simulation
  frames.
- Inputs: resolved game commands, applied on their recorded simulation frames,
  plus resolved camera-director sequence completions produced between
  simulation frames and applied before the following frame's commands
- Start point: each Original engine session has its own numbered file. Menu and
  raw input-device behavior are out of scope.
- Completion: a cleanly closed Original session always ends with an
  `rng_suffix` record, even when its draw batch is empty. The terminator also
  records `final_frame` and `frame_count`; raw EOF without this record means
  the capture was interrupted or truncated and must not be accepted as a
  complete replay oracle.
- Pathfinding: deterministic synchronous A*, while retaining the Original's
  request/processing phase boundary
- Randomness: the Original's filtered gameplay `rand()` values form one global
  draw stream. Rust must consume the same values in the same order and fails at
  the first cursor mismatch. Call-site offsets are diagnostic provenance, not a
  requirement that Rust use identical addresses or function boundaries.
- Floating point: native Original parity captures use scalar SSE2 evaluation,
  with FP contraction disabled. This removes x87's register-lifetime-dependent
  excess precision and makes stored `f32` state the shared numeric contract.
- Comparison: entities are mapped by stable logical traits and creation order;
  compared floats use exact bits. Numeric IDs alone are never treated as a
  behavioral divergence.

The superseded schema-4 expansion recording is
`original-code/parity-traces/original-fullgame-schema4-session-1.jsonl`: mission
`H01_Lin_VL` (Lincoln), 2,347 contiguous gameplay frames (0 through 2,346), and
3,117 filtered simulation RNG draws. The Original process exited after the
last flushed frame, but the trace is complete and structurally valid through
that frame. Active historical parity work reached frame 47. It must not be used
as the current correctness oracle because it predates the scalar-SSE recorder
build contract. A fresh schema-5 capture is required.

The first multi-session schema-5 capture is
`original-code/parity-traces/original-fullgame-schema5-multisession-session-0001.jsonl`:
mission `H01_Lin_VL` (Lincoln), explicitly marked `loaded_save`, with 797
contiguous frames (0 through 796) and 1,240 contiguous global RNG draws. It
validates the new recorder lifecycle and is retained as save-load evidence.
It predates the current schema and is not a parity baseline.

Sessions `0002` and `0003` are also retained loaded-state evidence: `0002`
contains 12 frames after the profile Continue load, while `0003` contains 386
frames after the Original's in-engine restart deserialization. A restart looks
like the mission opening but is not a fresh engine construction, so it remains
classified as `loaded_save`.

The last schema-5 parity baseline is
`original-code/parity-traces/original-fullgame-schema5-multisession-session-0004.jsonl`:
mission `H01_Lin_VL` (Lincoln), 862 contiguous gameplay frames (0 through 861),
30 resolved commands, and 1,238 contiguous global RNG draws (1,107 simulation,
131 audio). It is a direct `mission_start` capture, has a clean RNG suffix, and
has SHA-256
`da434b99786c012baaa0e7d6c5751c73dfbe6bf49312dbef18d5e44c1f49b659`.
Parity reaches frame 507, where the capture exposes an input-schema omission
rather than a reachability difference. An earlier click on scroll 111 queued a
`SEEK` followed by `TAKE` while PC 126 was in a non-interruptible door route.
The recorder captured a later ground move at frame 464 but not the direct
object click. Original resumes the queued seek at frame 508; Rust has no
recorded command from which to reconstruct its target or post-seek action.
Schema 6 records direct object and other bypass click handlers explicitly and
is now required. This schema-5 session remains useful evidence through frame
507 but cannot be the correctness oracle afterward. The next capture must use
schema 6. A diagnostic-only repaired copy places the omitted click in the
frame-507 input batch, while `PASS_DOOR` is still selected; that reproduces
Original's postponed seek and advances the comparison through frame 524. This
timing was inferred from state and is not a substitute for a schema-6 capture.

The last schema-6 baseline is
`original-code/parity-traces/original-fullgame-schema6-session-0001.jsonl`:
mission `H01_Lin_VL` (Lincoln), 2,315 complete gameplay frames (1 through
2,315), a complete versioned campaign snapshot, 2,979 filtered simulation RNG
draws, and the direct `TAKE` interaction that schema 5 omitted. The Original
then crashed after completing a money pickup because the Linux data contains
`jingle_04.ogg` while `PlayJingle(CASH_WON)` tried only
`jingle_04.wav`; the trace is valid JSON through its final flushed frame.
Original commit `f5604189` adds WAV-to-Ogg jingle fallback, makes an absent
jingle nonfatal, and prevents fatal error handling from calling a null
termination callback. Initial replay reaches frame 24, where both engines
previously appeared to process the same direct path request. The recorded goal
sector `434` is actually an Original heterogeneous fast-grid array index, not
Rust's canonical motion-sector number `59`. Schema-6 replay now resolves this
identity isomorphically from the recorded destination and layer, rejects
missing or ambiguous matches, and constructs the Original's two-gate
`24 -> 43 -> 59` route. Rust also no longer invents a direct cross-sector move
when gate routing fails; like `RHSequence::AppendMoveToSequence`, it reports
the unreachable destination and emits no move. Replay now reaches frame 67,
where both engines select `WaitFreeLift`, but Rust previously failed to
execute and terminate it because the wait carries movement-shaped metadata.
Wait commands now remain generic actor execution owners, and selecting the
following `PassDoor` walk no longer applies Execute-owned action state early.
The terminal ladder-up transition now applies the Original's `OnLadder/Moving`
state before selecting its `PassingDoor` successor. Fast ladder/wall orders
retain their non-animation dispatch semantics after lift animation
translation, so their ordinary climb sprite motion runs twice per actor slot
as in the Original. The ladder-exit transition now performs its raw-`START`
midpoint alignment and raw-`DONE` posture/state update on their respective
owner slots. Replay now reaches frame 129 during the following crouch-to-
upright transition. Door-pass successor installation is now state-neutral;
the successor movement Execute owns its state change. Replay now reaches frame
209. Cross-sector group formation now authorizes each per-PC destination
before gate routing, including the upright move box used for narrow lifts, and
ordinary wall/ladder moves use the same thick-reachability decision as the
Original. Schema-6 door and jump overlay hits remain semantic spatial
selections instead of being forced into a motion-sector override. Replay now
reaches frame 333. Authored wall/ladder directions now survive every waypoint
instead of being retranslated from each local path segment. Door-pass exit
transitions retain the preceding state until their own execution, preserve
their in-place destination/facing, and restore movement only when the following
walk actually executes. Replay now reaches frame 378.
Door-sector group clicks now retain the clicked door as the terminal gate
instead of converting it into a walk-to-door point. Normal AI timers read the
live current order after the actor slot, so a same-frame bored-random reroll
suppresses side-looking exactly as in the Original. Schema-6 overlay clicks
outside every sector polygon are accepted only after the canonical door click
polygon independently validates them. Replay now reaches frame 550.
Empty cross-NPC action batches no longer build speculative simulation scratch
state, so they cannot consume unrelated building-exit RNG. Owner-local
condolation-card drains likewise avoid globally forecasting other actors.
Building-door `Select` is now a real one-slot non-animation order, including
its hulk callback, rather than an eagerly executed route step. Building
sectors are mapped dynamically by inactive actor identity while their layer is
the authored final conventional layer; sector identity remains isomorphic but
layer identity is still compared exactly. Cross-sector route construction now
prefers the actor's live door-pass state when the position interface has not
yet committed the door, matching `GetDoor()` during building traversal.
`ChangePosition` treats its encoded sector as a source assertion, preserves
the current topology and projection plane, and recomputes 3D coordinates at
the new map point without a destination projection query. Finally, resolved
`StopPc` follows the full normal-priority sequence stop path: it preserves the
identity-owned default Wait (and therefore a bored idle), while still
rewriting/stopping real movement and cancelling pending work. Subsequent
corrections to synchronous `UnlockAI`, 3D sight coordinates, scroll attachment,
seek countdown, cutscene locking, and movement-stop recursion advance the
current baseline through frame 1,255.

Frame 1,256 is not an engine result that schema 6 can reconstruct. The
Original finishes EdwardZone's script-driven camera slide before that
hourglass, advances through its 15- and 40-frame timers, unlocks the user and
all AI, and consumes the corresponding return-to-duty RNG cluster. Rust is
still 28 timer ticks from the unlock because its camera began from a different
host viewport. Schema 6 records neither resolved viewport scrolling, camera
state, nor the display-driven camera-sequence completion event. The trace
therefore proves the downstream unlock boundary but contains no input from
which the preceding camera duration can be reproduced. The next schema must
record this resolved nondeterministic boundary (or make complete camera state
and input part of the parity contract); no frame-specific timer adjustment is
acceptable.

The schema-2/3/4 sessions below remain historical evidence for completed parity
corrections, but are not accepted by the current replay runner.

The current expansion trace is
`original-code/parity-traces/original-demo-little-john-domino.jsonl`: schema 2,
7,248 gameplay frames, and 20,765 filtered simulation RNG draws. It is
logically isomorphic through frame 531. Frame 532 exposes an irrecoverable
schema-2 capture gap rather than an engine result: continuous bow orientation
changed PC 198's facing and launched `RaiseBow`, but the resolved 3D cursor
target was not recorded. Expected after-state must not be fed back into replay,
so this session cannot provide authoritative parity coverage after that point;
the next schema-3 capture supersedes it.

The superseded schema-3 session-2 capture is
`original-code/parity-traces/original-demo-schema3-session-2.jsonl`: 1,814
contiguous gameplay frames (0 through 1,813), 5,353 simulation RNG draws, 550
audio RNG draws, and 130 resolved commands. It exercises the stable semantic
command/orientation and RNG-domain contract. Parity reaches frame 451. Its
frame-452 bow command is not suitable as a continuing correctness oracle:
Original visibility had reused a result from its lossy 2,000-entry
reachability cache, whereas Rust performed the exact obstacle test. The cache
has been removed from the Original; the next schema-3 capture supersedes this
session for parity work after frame 451.

The final schema-3 capture was
`original-code/parity-traces/original-demo-schema3-session-3.jsonl`: 2,279
contiguous gameplay frames (0 through 2,278), 6,197 simulation RNG draws
including the startup prefix, 706 audio RNG draws, 69 resolved commands, and
6,426 exact opaque visibility queries. All visibility queries correctly report
`cache_hit: false`. Both the normal first-divergence replay and the independent
`--scan-all` pass match the whole recording.

The last schema-8 baseline is
`original-code/parity-traces/original-fullgame-schema8-session-0001.jsonl`:
mission `H01_Lin_VL` (Lincoln), 3,087 contiguous simulated frames (0 through
3,086), four recorded director completions, and 4,224 filtered simulation RNG
draws. It is a direct `mission_start` capture with the complete campaign,
resolved-command, director-completion, simulation-body, and global-RNG
contracts. Replay currently matches through frame 2,317. The frame-2,284
crossbowman divergence combined three general elevation/combat-context errors:
hearing mixed map and world coordinates, loaded bow users were not marked as
archers in persistent AI state, and tactical fighter snapshots imposed a
same-layer restriction absent from the Original's global fighter registry.
With those corrected, soldier 79 hears and sees PC 126, selects the Original
archer retreat decision, and enters `AttackingArcherRetireFromCombat`. The
next two differences were a lift-exit animation translation and a seek timer
that was rearmed when a compound route resumed after that door. The first
remaining divergence is soldier 79's archer-retreat waypoint at frame 2,318.
Rust accepts the first proposed straight retreat from `(709,1292)` to
`(652.55054,1324.378)` on layer 4 with half-diagonal `(6,4)` and therefore
never invokes A*. Its thick corridor contains eight active motion-line
candidates, none of which intersects either side or has its tested endpoint
inside. The Original instead emits graph waypoints. A source audit proved that
the Original and Rust static line geometry, corridor construction,
intersection arithmetic, and owner-relative fighter position agree for the
observed Rust request; the retreat endpoint derives bit-for-bit from the
correct pre-owner-slot PC position. Therefore no speculative reachability or
fighter-timing change is warranted. Schema 9 now records the complete oriented
motion-line grid, sparse active-line changes, and ordered queued/completed path
requests (including the exact source, goal, half diagonal, flags, validity,
and waypoints). A new schema-9 capture is required to identify the remaining
general difference.

The current schema-9 baseline is
`original-code/parity-traces/original-fullgame-schema9-session-0002.jsonl`:
mission `H01_Lin_VL` (Lincoln), 7,128 contiguous gameplay frames (0 through
7,127), with 9,610 filtered simulation RNG draws in its validated cache. It is
a direct `mission_start` capture with the complete schema-9 campaign,
resolved-command, director-completion, simulation-body, motion-grid,
path-request, and global-RNG contracts. Replay currently matches through frame
4,099. The corrections since frame 1,929 cover positive integer hearing
thresholds, stale ability retirement, boundary-inclusive sector membership,
synchronous released-Seek translation, nested owner-card FIFO closure,
creation-slot entity-seek refresh, stable base seek distance, and
topology-aware AI `GoTo` routing. Frame 2,153 then exposed two coupled
entity-seek completion errors: reaching an old concrete waypoint launched the
post-seek hit without validating the live target, and `SEEK_STOP_NPC` deferred
the target's `EVENT_STOP` until after sequence-manager dispatch. Both now
follow the direct Original `PerformSeek`/`Think(EVENT_STOP)` call boundary.
The following hit exposed a missing common civilian
`EVENT_LOSE_CONSCIOUSNESS` transition at frame 2,172; that base-AI handler is
now shared semantically with the existing enemy implementation. A full
mission-start replay then exposed PC pickup progression, explicit transitions
on lifts, the paired fast-stair motion contract, full-position shadow facing,
and world-ground Follow/Stare coordinates; all now follow their Original
dispatch boundaries. The next section of the recording exposed authored
PassDoor action preservation, sector-qualified door-combat positions,
synchronous door-combat AI dispatch, inline AI route construction/RNG
ordering, retained post-Execute timer ownership, and terminal
cross-postponement cleanup. The next correction preserves the two distinct
Original statement orders around `Face` and `SetState`: a Turn instructed
before an attentive-mode transition retains its direction side effect, while
a Turn registered after that transition remains inert while postponed and
applies its stored direction only when later instructed. The following combat
section corrected two inverted random look choices, removed a non-Original
timer exit from a side-looking state, completed synchronous postponed Turn and
parry dispatch, and restored per-frame principal-opponent facing during normal
parry holds. Subsequent corrections preserve alert-report dialogue statement
ordering, load the real mission speech durations in the replay tool, and use
the Original's full 3-D actor visibility calculation when patrols admit or
reacquire members. Later corrections restore common route `SetState` and
same-frame Turn ordering, officer formation ordering, attentive-command
resume, direct target-click routing, and atomic NPC PointTo sequencing. The
following target interaction exposed the Original's independently preserved
target sprite/action-point anchors, its three-order sword-hit translation, and
the non-isometric hotspot-facing convention used while executing target hits.
The replay now matches through frame 4,108. The first remaining divergence is
the much larger seek-area RNG fan-out beginning at frame 4,109.

The replacement schema-9 baseline after resetting Original's cached seek-point
state is
`original-code/parity-traces/original-fullgame-seek-reset-session-0001.jsonl`:
mission `H01_Lin_VL` (Lincoln), 2,480 contiguous gameplay frames and 3,474
filtered simulation RNG draws. Replay currently matches through frame 788.
The first remaining divergence is at frame 789: PC 126 starts `UnequipBow` in
Original while Rust remains in `Wait`.

The first schema-10 baseline is
`original-code/parity-traces/original-fullgame-schema10-session-0001.jsonl`:
mission `H01_Lin_VL` (Lincoln), 2,262 contiguous gameplay frames and 3,199
filtered simulation RNG draws. It is a direct `mission_start` capture with
complete nested resolved action input. Replay currently matches through frame
123. The first schema-10 divergence at frame 100 was caused by Robin crossing
obstacle 62's local dry-grass material polygon: Original's exact crossed
`LINE_SOUND` sector changed running-noise volume from 70 to 150, while Rust had
registered only global material polygons before the fast grid's block arrays
existed and retained Ground. Source-associated global and obstacle-local sound
lines are now registered after grid allocation on their authored layers, keep
their raw material-sector owner, and apply crossed sectors in Original
intersection order. Soldier 64 consequently receives the same frame-100
footstep hearing edge.

Frame 124 exposed undefined behavior in the Original rather than a Rust logic
rule worth reproducing. While Robin was traversing door 73,
`RHArtificialIntelligence::Position` read the pass-door movement element's
`mswDirection` to choose `point_in` or `point_out`, but the pass-door
constructor had never initialized that member. This capture happened to read
false and made soldier 64 face `point_out`; the authored direct traversal and
Rust both select `point_in`. Original commit `d1a6e049` initializes the field
and sets it from the gate direction. The trace remains valid evidence through
frame 123, but is deliberately invalid from frame 124 onward and must be
replaced.

The current post-fix schema-10 baseline is
`original-code/parity-traces/original-fullgame-schema10-passdoor-fixed-session-0001.jsonl`:
mission `H01_Lin_VL` (Lincoln), 1,724 contiguous gameplay frames and 2,332
filtered simulation RNG draws. It is a direct `mission_start` capture made
after Original commit `d1a6e049`. Replay passes the former undefined
frame-124 boundary and all 1,724 recorded frames now match, including all 2,332
filtered simulation RNG draws.

The active loaded-save coverage trace is
`original-code/parity-traces/original-fullgame-schema10-round2-session-0001.jsonl`:
mission `H01_Lin_VL`, 5,553 contiguous frames and 7,394 simulation RNG draws.
Replay is exact through frame 4,357 after the general movement, combat,
sequence-ordering, pathfinding, and AI fixes recorded below. The capture is not
authoritative from frame 4,358: it predates Original commits `c33a88e6` and
`f8a22811`, and a civilian there faces a panic center produced by the same
`INFO_HUMAN`-tagged/pointer-as-`INFO_POS` bug described for Nottingham below.
The recorded direction 4 came from process-pointer union bytes; fixed Original
and Rust use Soldier 87's actual source position and produce direction 7.
Emulating the old pointer value is explicitly out of scope, so the remaining
1,195 frames require a fresh post-fix capture.

The companion Nottingham mission-start oracle is
`original-code/parity-traces/original-fullgame-schema10-round2-session-0002-normalized-v2.jsonl`
(SHA-256 `78d3d64a50be5cc834b08cc6fdf45ea58f9f72ebf8ddd6cd2429e0b03cd30db1`):
2,398 contiguous frames and 8,875 simulation RNG draws. Replay is exact through
frame 389 after owner-local AI/RNG, seated-repulsion, and attentive-mode
ordering corrections. The capture is deliberately invalid from frame 390:
Original `NearbyCiviliansPanic` tagged its `EVENT_PANIC` payload as
`INFO_HUMAN`, while `RHArtificialBonhomie::Think` read those same union bytes as
an `INFO_POS` `RHposition`. Its escape-door choice therefore depended on
pointer/stale bytes and process address layout. Original now records the
caller's actual `Position(mpMe)` in the stimulus; the same correction applies
to the brawl-completion and international-war panic broadcasts, which had the
identical tag/payload mismatch. Rust already uses the semantic source position.
No pointer-derived replay compatibility is permitted, and a fresh Nottingham
recording is required for authoritative coverage after frame 389. The `-v2`
normalization also relabels two class-16391 `RHCLASSID_OBJECT_BONUS_NET`
creations from the old recorder's runtime `net` kind to `bonus`, matching
Original commit `ffcac2bf`; the raw and first normalized artifacts fail strict
initial entity mapping and are not parity oracles.

## Change ledger

| Status | Area | Trace evidence and Original behavior | Rust change / regression coverage |
| --- | --- | --- | --- |
| Done | Zero-index changing-obstacle patch flag | nicouzouf Profile 001 Savegames 053, 059, and 065 apply a patch whose encoded changing-obstacle value is `1`. `RHPatch::InitializeFromProtoStream` sets `mbUseChangingObstacles` before adapting that value to obstacle index zero; Rust instead tested the adapted index and left motion-grid lines 302 through 309 inactive. | Patch loading now derives the enable flag from the preserved sector/layer presence rather than the decoded obstacle index. Savegames 053 and 059 match completely; Savegame 065 passes the former frame-176 motion-grid divergence and exposes a separate frame-200 RNG-consumption mismatch. |
| Open | Strict clean-close footer validation | Schema-10 traces whose final frame consumed the last RNG draw previously ended at that frame because `RHParity::Close` emitted `rng_suffix` only when trailing values existed. Clean completion was therefore byte-for-byte indistinguishable from abrupt EOF after the same flushed frame. Original commit `5b7495da` now always emits the suffix and adds `final_frame` plus `frame_count`, without changing the schema or RNG batch shape. | The Rust decoder already accepts an empty suffix and ignores the additive extent fields, so existing parsing remains compatible. Its cache builder still synthesizes an in-memory end record when raw JSONL reaches EOF; it must instead require the explicit suffix and validate the declared extent before treating a new capture as complete. |
| Done | Authored attentive-before-swordfight ordering | At Nottingham frame 390 a near-enemy `EventView` calls `SetState(ATTACKING, REACTIONTIME)` before `BattleDecisions -> BeginSwordfight`. Original synchronously registers `ENTER_ATTENTIVE_MODE` first, then registers `ENTER_SWORDFIGHT`; Rust collected both effects but drained swordfight first, making the PC and soldier enter combat before the authored attentive transition. | The owner-local effect drain now applies the pending attentive request before its later swordfight request. This preserves call order for every near-enemy reaction and removes the PC 252 / soldier 131 command divergences without actor-specific logic. |
| Recording required | Typed civilian panic stimuli | `NearbyCiviliansPanic`, brawl completion, and the international-war violation broadcast all constructed `EVENT_PANIC` through a human-pointer overload, but the civilian handler reads `stimulusInfo.posPosition`. Nottingham frame 390 exercised the first path and selected an escape door from pointer/stale-memory-derived coordinates; all three were address-dependent. | Original now constructs all three broadcasts with a concrete `RHposition`: `Position(mpMe)` for AI-owned broadcasts and the PC's current position for the static violation helper. Rust closes the recipient's complete owner-local `Think(EVENT_PANIC)` boundary so state, speech, door choice, and queued movement settle synchronously. The old trace is invalid from frame 390 and must not be patched with its accidental door identity. |
| Done | `INFO_STOLEN` stimulus serialization boundary | `RHStimulus::Serialize` serialized the two `RHstolenObject` pointers, then fell through into `INFO_COMBAT`. Saving wrote unrelated stale union bytes and loading overwrote the reconstructed stolen-object payload as combat fields/pointers, so a queued `EVENT_OBJECT_AWAY` could be corrupted across save/load. | Original now terminates the `INFO_STOLEN` switch arm after its two authoritative pointers. This changes only malformed legacy serialization; new recordings and saves retain a typed, stable stolen-object stimulus instead of depending on union residue. |
| Done | Source-associated `LINE_SOUND` material crossings | PC 126 enters obstacle 62's local Grass polygon between frames 94 and 95. Original stores the owning `RHSectorMaterial*` on every boundary line and changes the PC to dry grass, producing run volume 150; soldier 64's frame-100 phased hearing check is then barely in range. Rust synthesized only globally listed lines at layer 0 before fast-grid allocation, lost their block registration, omitted obstacle-local polygons, and globally re-queried material instead of using the crossed owner. | The complete authored material table now remains index-addressable. Global and per-obstacle sound polygons register after motion-grid allocation on the same layers as Original; each edge carries its raw material index. Crossing removes old-position boundary repeats, sorts by intersection distance, and applies each exact sector with obstacle/default fallback. The schema-10 baseline advances from frame 99 through frame 123. |
| Done | Initialized pass-door AI destination direction | At frame 124 Original `AI::Position` consults `RHSequenceElementMovement::GetDirection` while PC 126 passes door 73. The pass-door constructor never initialized `mswDirection`, so stale heap contents selected `point_out` despite an authored direct traversal. That moved the requested facing across the direction-14/15 boundary and changed soldier 64's reaction substate. | Original commit `d1a6e049` initializes the member and assigns the attached gate's direct/indirect direction. Rust already uses that authored direction. Rust facing also preserves Original's explicit target-elevation-to-`SWORD` conversion and exact `SBGeoVector2D` classifier literals. The replacement trace passes this boundary; no replay-specific emulation of the old undefined value was added. |
| Done | Heard-steps investigation launch | At frame 167 soldier 64's reaction timer enters `SUBSTATE_SEEKING_HEARDSTEPS`. Original calls ordinary `GoTo(mposSeekPosition)` with no flags and launches a 200-frame timer. Rust used `GoNear` with the search-noise tolerance and a 10-frame timer; because the soldier was already within that tolerance, it immediately synthesized `EVENT_REACHPOINT` and advanced to `SeekingJustWatching`. | The heard-steps reaction now launches the exact ordinary, zero-flag `GoTo` and 200-frame timer. This preserves `special_action`'s normal close-point gate and applies to every soldier investigating a heard footstep. Replay advances from frame 166 through frame 190. |
| Done | Delayed special-strike state handoff | At frame 191 soldier 64's prepared strike advances from its `WaitTimer` to `SwordstrikeThrustA`. Original exposes substate 161 (`AttackingSwordfightSpecialStrike`) only during that delayed preparation and changes to ordinary substate 160 when the actual strike is instructed. Rust retained 161 for the entire in-flight strike. Direct reactive counterstrikes are different: they have no preparation wait and retain 161 until `EVENT_DONE`. | Strike dispatch recognizes the general authored `WaitTimer -> swordstrike` sequence shape and closes only that preparation state, while retaining an independent lifecycle latch until strike completion. Direct counterstrikes remain special. Completion releases the latch before its re-entrant `EVENT_DONE`, so ordinary swordfight reconsideration can authorize the next strike. |
| Done | Re-entrant postponed damage instruction | Closing the prepared strike's corrected completion boundary released actor 126's postponed `ReceiveSwordDamage` while still inside the attacker's `SendCondolationCard -> Ready -> StartPostponedSequenceElement` stack. Original immediately calls the victim's normal `Instruct/Translate` damage path there; Rust's synchronous owner dispatcher handled movement, waits, parries, and several other commands but rejected every damage command. | All seven receive-damage command families now use the ordinary damage translator when reached through the synchronous owner stack. Missing commands still fail loudly. With this source-general re-entrant path, the complete 1,724-frame baseline and global RNG stream match. |
| Done | Post-motion script-zone registration | Rust parsed mission script sectors during its early environment metadata pass, before `FastFindGrid::allocate_layers`. The global sector and `LINE_SCRIPT` objects survived, but allocation replaced every per-layer and per-block spatial index, so crossing and occupant queries could not find authored zones. Original constructs mission script geometry only after its spatial grid exists. | Location-handle metadata remains in the early pass, while script polygons and crossing lines register immediately after proto motion allocation on their authored layers. The one-to-one zone index mapping is preserved, and an index outside Original's `u16` domain fails loudly. Restoring these general triggers exposes the next previously masked Lincoln ladder divergence at frame 265. |
| Done | Post-motion archery tactic layers | Archery topology was also parsed before Rust's motion `sector_number_map` existed. Every sector/waypoint lookup therefore fell through to layer 0, including elevated authored ways. | After motion registration, every archery sector and waypoint resolves its referenced motion sector to the authored layer. Missing sectors and topology-count inconsistencies are level-load errors rather than silent layer-zero substitutes. |
| Done | Fast-climb first-call termination barrier | Lincoln PC 126's fast wall/ladder route reaches a waypoint on the first of the two literal `Turn`/`PerformMotion` calls. Original immediately returns `RHMOTION_TERMINATED` and skips the second call. Rust deferred position commitment until after both sprite calls, so it advanced the shared animation counter once too far; the next climb order first moved at frame 265 instead of 266. | Fast-climb dispatch projects the first call through the same anti-collision inputs and tests the live goal before issuing the second sprite call. A first-call arrival now retains exactly one call's frame distance and animation progression, including deviated/blocked geometry rather than relying on a replay coordinate. |
| Done | Retained stop-transition goal ownership | Soldier 66 hears Robin at frame 311 while walking. Original's first `StopAll` retains the selected movement as a walking-to-waiting transition, so its sprite goal remains `(1780,899)` for that frame. At frame 312 attentive-mode arbitration interrupts that retained transition; actor-base `SendCondolationCard` then clears the goal before the attentive/Turn replacements are translated. Rust cleared on the first halt and, after that was corrected, lost the second cleanup because manager selection had already moved to the incoming element. | Halt distinguishes a retained live movement from a detached one, and priority arbitration snapshots the actor's runtime `mpSequenceElement` identity before manager selection changes. Attentive-mode interruption of an already-retained exit transition applies actor-base goal cleanup at the synchronous launch boundary. Ordinary standalone `FaceTo` still retains a live movement goal, while a second halt against its exit transition does not resurrect it. Focused halt-condolence tests pass; replay advances through frame 317. |
| Done | Direct target-click route identity | At frame 4,035 schema 9 recorded `HIT_TARGET` for target 99. Rust replayed it through the generic `AddInteractionWithSeek` model and armed the shared seek-refresh timer to 25, while Original retained 8. `RHElementTarget::MouseClicked` instead calls `AppendMoveToSequence` directly with the target as `pVictim`, tolerance zero, and no `RHMOVE_SEEK`, then appends Turn and HitTarget. | Gate-route goals now distinguish a target-bearing ordinary Move from an entity Seek: the target pointer survives gate approaches and the final move without seek flags or refresh state. Schema-9 `HIT_TARGET` commands use this source-backed route, including the Original double-click acceleration and acceptance bark. The trace's generic `launch_interaction` shape cannot distinguish every other default-target click from an active ability against that same target; a future schema must record resolved route provenance instead of inferring it from command and entity kind. Replay advances through frame 4,035. |
| Done | Atomic NPC `PointTo` sequence | At frame 4,046 soldier 94's report timer called `Say` then `PointTo`. Original resolves one full-position direction and launches a single two-level sequence containing Turn then Point. Rust emitted two independent single-order Generic sequences; Point interrupted Turn immediately and its same-frame completion advanced the report from point substate 97 to end substate 98. | `AiController::point_to` now resolves the 3-D/isometric sector once at the call boundary, stores it on both authored elements, and sends one `Turn -> Point` sequence through the existing full-sequence outbox. Point cannot start or emit `EVENT_DONE` before Turn completes. All enemy and civilian PointTo callers share the correction; replay advances through frame 4,061. |
| Done | Preserved target sprite/action anchors | Target 99's authored interaction point is `(1259,1768)`, but its visible sprite is placed from the independently loaded 3-D position. Original computes and retains that sprite position before overwriting only `PositionMap` with the action point; Rust reconstructed sprite top-left from the action point and produced a hotspot on the wrong side of the PC at frame 4,062. | C++ sprite-position reconstruction now uses the already preserved visual/3-D target anchor only for FX targets, while ordinary entities retain their existing map-position rule. Current-row hotspots, target hit facing, bow anchors, hit tests, and every other target sprite-point consumer now share the same general split. |
| Done | Complete PC target-hit order chain | `RHElementActorPC::Translate(HIT_TARGET)` appends `TRANSITION_RAISING_SWORD`, `HITTING_TARGET`, and `TRANSITION_LOWERING_SWORD`. Rust appended only the hit, so at frame 4,069 it played the same visible hit row while reporting `Waiting` instead of the Original `WaitingSword`; it would also have omitted the lowering transition. | HitTarget dispatch now appends the complete three-order FIFO, with direction computation disabled on each order and the target antagonist attached to the hit. Existing transition lifecycle handlers provide the Original sword-state entry and exit, and target activation still fires on the hit order's `DONE` edge. Replay advances through frame 4,088. |
| Done | Screen-space target-action facing | On initialization of `HITTING_TARGET` and `HANDLING_TARGET`, Original resolves the target's live current-row hotspot with `GetCurrentPointMap`, calls plain `GetSector0to15()` (aspect 1), and freezes the action's first frame while `Turn()` remains incomplete. Rust retained the route's earlier isometric direction and used default animation progression. | Both PC target-action orders now refresh their goal from the live hotspot with the exact non-isometric sector classifier, call progressive Turn every tick, and use `FrozenFirstFrame` until aligned. This applies to every target profile and row without a recorded coordinate or identity special case. Replay advances through frame 4,099. |
| Done | Scripted one-shot noise call boundary | At frame 4,101 the mission script emits a Drawbridge noise at `(1135,1843)` in layer 0, sector 24. Original resolves that point through `PositionToPoint3D`, giving elevation 220, compares source and listener in full world-space Y, and calls every listener's `Think(EVENT_HEAR)` synchronously in NPC creation order. Rust discarded the sector, broadcast from elevation zero, omitted the listener elevation from Y, and deferred the resulting events, changing both the listeners and the same-frame RNG stream. | `MakeNoise` now requires and retains the resolved layer/sector, derives source elevation through the shared projection geometry, and uses the corrected world-space hearing vector. A synchronous broadcast drains each listener at the Original call boundary in creation order; queued broadcasts remain available to callers with their own explicit barrier. Replay advances through frame 4,101. |
| Done | `StopAll` before `SetState` ordering | A frame-4,101 hear reaction performs `StopAll` before entering `Wondering`. Original settles the selected movement's actor-base condolence before `SetState` installs attentive replacement work, clearing the old map goal. Rust left the halt in the same outbox as the replacement; by callback time the attentive element obscured which movement had been selected, leaving goal `(1280,1660)`. | State-change notifications now detach and execute actor effects authored before `SetState` as a true prefix. The halt barrier delivers its condolence before replacement effects, while `halt_actor` snapshots selected-movement ownership so its actor-base goal cleanup cannot be lost when sequence identity changes. This applies to all stop-then-state reactions and carries no replay identities. Replay matches through frame 4,108. |
| Done | Live `SeekArea` friend multiplier | At frame 4,109 nine soldiers synchronously build seek lists around the same report point. Original `SeekArea` scans the live global NPC register at each call and counts every other Soldier above green alert within raw map-space radius 500. Rust populated this input only during detection scans and additionally filtered by camp, layer, combat readiness, AI state, and isometric distance, so report/timer entry received zero and consumed only 64 of the Original frame's 343 draws. | The ordinary per-Think tick-data builder now performs the exact source-level live scan for every entry path. Detection snapshots no longer overwrite it with a stale, differently filtered aggregate. Rust consumes 280 draws and constructs all nine full seek lists. |
| Recording required | Frame-4,109 inherited seek-point status | The remaining 63 draws are explained by shared seek-point interest: Original selection-test attempts grow across the nine calls (15, 15, 16, 18, 18, 19, 20, 20, and 22), while Rust sees every global point at pristine 100% interest and needs 15 each time. Schema 9 does not record global seek-point interest/lock state. This is session 2 for the same cached Lincoln proto, so a reused Original grid can carry invisible static status into a nominal mission-start session; the first observable symptom is 4,109 frames later. | Original mission initialization now resets transient seek-point interest/locks beside the NPC register reset while preserving cached geometry; loaded-save/restart deserialization may then restore its authoritative saved status. The existing schema-9 trace cannot supply the missing initial values without a replay-specific fabrication and must be replaced. Original commit: `89a91dff`. |
| Done | Execute-arm-specific entity-seek countdown | The fresh trace first diverged at frame 387 because Rust aged the entity-seek refresh countdown whenever the movement element carried `RHMOVE_SEEK`. Original ages it only in `PerformSeek`; wall and ladder Execute arms retain the flag but call `PerformMotion` directly, while fast stairs literally invoke `PerformSeek` twice. | Movement countdown aging now follows the selected Original Execute arm: ordinary walk/run/transition/stair forms age once, fast stairs twice, and direct wall/ladder motion not at all. The rule is action-family based and applies to every entity seek. Replay advances through frame 456. |
| Done | Flattened gate-route post-seek handoff | At frame 457 Robin was already within the scroll's live interaction tolerance while executing the final walking-to-waiting transition. Original retained Take in `mpPostSeekSequence`, launched it immediately, and returned before decrementing `mulWaitTime`, leaving the value at 10. Rust's gate representation stored Take as the next sequence element, so it selected the same command but incorrectly aged the counter to 9. | A following element on Rust's entity-seek gate route is now recognized as the equivalent live post-seek handoff for `PerformSeek` arrival semantics. It suppresses the countdown decrement and pre-motion sprite work while ordinary sequence advancement still promotes the general continuation. Replay advances through frame 569. |
| Done | World-space obstacle view-radius slicing | Soldier 58 first sees PC 126 emerging on the wall at frame 565. Original passes the full world-space `ComputeEyesPoint` into `ComputeViewRadius`, slices the view sphere against PC 126's sloped projection-obstacle plane, and caches visibility `1.0056405`; suspicion reaches the shadow threshold and raises `EVENT_SEES_SHADOW` at frame 570. Rust mixed projected map Y with the world-space plane origin, computed a zero effective radius, and never admitted the otherwise clear 3-D ray. | `compute_view_radius` now requires a `WorldPoint3D` and uses it for obstacle-plane distance, night/fog reference construction, and light-sector LOS. Every periodic, synchronous-AI, and script-native call site supplies the Original eye coordinate family. The no-obstacle sphere slice also preserves the Original `sqrt(fabs(R²-Z²))` behavior. Generic optical-refresh and blocker traces remain available without actor or replay identities. |
| Done | Projected-map `PositionToPoint3D` plane solve | The frame-570 shadow reaction calls `Face(RHposition)`. Original resolves the recorded map point with `z = (bz*y + az*x + dz) / (1-bz)`, reconstructs world Y as `mapY + z`, and turns soldier 58 toward direction 14. Rust fed projected map Y to a world-coordinate plane query, derived an impossible elevation near 2,748, and selected direction 8. The same ambiguity affected elevated hit/push flight goals. | Sight obstacles now expose distinct world-coordinate and projected-map top-plane queries. AI position reconstruction, the engine's shared sector projection, and projected melee flight goals use the isometric plane solve; LOS, physical landing, and other genuine world-coordinate callers retain the direct plane query. Replay advances through frame 609. |
| Done | Non-interruptible admission precedes transitions | A second Take command arrives at frame 589 while PC 126 is executing a non-interruptible wall `PassDoor`. Original stamps the incoming Seek's live state, then handles the non-interruptible current element before `GenerateTransition`: the Seek is postponed, equal-normal priority replaces the older postponed group Move, and release at frame 610 lowers the Seek and arms `TIME_SEEK_REFRESH=25`. Rust's deferred manager path tried to generate a transition from temporary `Flying` posture first, marked the Seek impossible, resumed the stale group Move, and retained the prior scroll seek countdown at 10. | Deferred `InstructOwner` now applies the shared non-interruptible guard before transition generation and settles any card caused by replacement of an older postponed command at that exact boundary. Ordinary priority arbitration retains its later transition-first ordering. This covers all commands arriving through the sequence-manager queue during door passes, not just Take/Seek; replay advances through frame 747. |
| Done | Bow line-of-fire origin uses actor elevation once | At frame 748 PC 126 moves the cursor from a long-shot target to a normal-shot target. Original `ComputeBowPoint` starts independently from the actor's base elevation, finds the normal trajectory clear, and launches `LowerBow`. Rust passed the already elevated hand point to a helper that expects the actor base position, added the 25-unit hand elevation a second time, intersected obstacle 77, and incorrectly upgraded the shot to Long. | Cursor bow LOS now passes the actor's base 3-D position to the same `compute_bow_point` convention already used by the combat path. Hand position remains the source for range and shoot-mode distance only. Generic trace events expose the bow-aim gates and resolved classification without any replay-specific branch. Replay advances through frame 767. |
| Done | Aspect-corrected 3-D patrol ordering | Officer 71 reinitializes its patrol at frame 763. Original `InitializePatrol` orders admitted members using `SquareDistance`: full 3-D ground-position deltas with world Y stretched by `INVERSE_ASPECT_RATIO`, and inserts equal-distance members before existing entries. Rust used a stable sort of raw projected-map `dx² + dy²`, making soldiers 74/70 the leading pair instead of 70/69. The next eighth-frame formation refresh consequently sent the only available row to the wrong members and left soldier 69 waiting. | Patrol reinitialization reconstructs world Y from map Y and elevation, includes the Z delta, applies the Original aspect stretch, and reproduces its strict-greater insertion/tie behavior before the unchanged left/right determinant pass. This is shared by every patrol rebuild and contains no actor or trace identity. Replay advances through frame 788. |
| Recording required | Nested resolved action cancellation | At frame 789 the user right-clicks while Bow is selected. Original `PerformMouseRightClick` finds an empty shoot list and forwards `MSG_SELECT_ACTION(NoAction)` inside the root raw-mouse dispatch; `SelectAction → UnSelectAction` then launches `UnequipBow`. The recorder admitted nested character selection only, so the frame retained its earlier automatic bow orientation but omitted the authoritative cancellation. Inferring it from the later actor command would be replay-specific and invalid. | Original now sends every depth-two PC message under a raw-mouse input root through the existing semantic `RecordInputMessage` filter, which records this as `cancel_action` without admitting deeper simulation callbacks. Rust accepts the Original's global no-PC cancel shape and maps it to `UnselectAllActions`. Schema 10 invalidates incomplete older traces, so a fresh capture is required. Original commits: `2bc07664`, `a087cc3a`. |
| Done | Authoritative per-frame PC ammunition comparison | The broad Rust engine dump exposed an entity-local legacy ammo mirror containing zero while the authoritative campaign character still contained the correct one remaining arrow. The logical comparator previously ignored the trace's complete `pc.ammo` payload, making a real campaign-counter divergence capable of hiding until a later ability decision. | The parity view now compares all nine recorded ammunition counters against the Rust campaign character status every frame. Missing PC/campaign mappings fail loudly. The current replay's authoritative counters match; the stale diagnostic mirror is not used as gameplay state. |
| Done | Complete mission-start campaign state | Mission/proto names and an RNG seed cannot reconstruct progressed mission status, the selected team, character inventory/skills, persistent Sherwood production, relics, or script-visible campaign values. Capturing after `RHEngine::Initialize` is also too late because the Original consumes the mission team and marks its descriptions instanced during construction. | Original schema 4 introduced a neutral JSON campaign snapshot captured immediately before engine initialization and written into the trace header after initialization RNG has accumulated into the normal prefix. Schema 8 retains that state contract. Rust requires schema 8, validates profile IDs/names and every stored index, reconstructs the complete campaign before level loading, and rejects all older traces. Production script attachments/points remain level-derived exactly as in the Original save format. |
| Done | Deterministic native floating point | Native i686 GCC defaulted to x87 evaluation even though SSE2 instructions were available. Extended intermediates then depended on compiler register lifetime, so exact frame-state bits could diverge from Rust and change after unrelated Original rebuilds. | Schema 5 captures are built with scalar SSE2 evaluation and FP contraction disabled across every in-tree Original target. Schema 4 recordings are deliberately invalidated; a new schema-5 recording is required rather than teaching Rust to emulate unstable x87 spill behavior. |
| Done | Compiler-independent signed numeric conversions | Several Original paths converted negative floating-point values directly to unsigned integers, which is undefined and changed behavior between the shipped GCC 2.95–3.3 builds and the current compiler. The camera magnitude check consequently cancelled left/up scrolling; diagonal NPC hearing, unusually short jump timing, and negative anti-aliased-line increments had the same latent dependency. | Original now compares camera magnitudes as floats, rejects non-positive perceived noise before conversion, clamps jump waits before conversion, and converts signed fixed-point renderer increments through `SLONG` before their intentional modulo-`ULONG` representation. Alpha interpolation also avoids shifting a negative signed value. Ordinary positive truncation and modular renderer stepping remain unchanged. Original commits: `501e2b4b` and `0233a0f7`. |
| Done | Mission-start idle ownership | Rust eagerly installed a fallback `Wait` for every actor during level loading. Original actors are constructed without an order and acquire a fallback only from `Actor::Hourglass` when they still have no live order. Startup initialization replaced Rust's eager waits before the first actor tick, but their sprites nevertheless executed once, causing a second `Start` pulse and delaying every idle action point by one frame. | Removed the level-loader fallback pass. The existing per-actor Hourglass path remains the sole lazy fallback, while startup-created actor orders enter the first tick directly. This is a general actor lifecycle correction, not trace-specific identity handling; the schema-5 replay advances from frame 46 through frame 78. |
| Done | Authored initial `Wait` dispatch | Original `InitState` sets the authored sleeping, sitting, dead, unconscious, or special pose and calls the actor's `Wait`. Priority-`Wait` launch then synchronously executes `NextSequenceElementsGo -> Go -> Instruct` before `InitOneAI` returns. Rust queued that final instruction until the first sequence phase. `Actor::Hourglass` therefore saw the actor as empty, executed a lazy fallback first, and replaced it with the authored order one frame later. Lincoln soldier 64's halberd animation consequently reached its frame-40 gameplay remark one tick late. | AI initialization now closes the synchronous sequence-registration stream after every `InitOneAI`, without touching ordinary deferred sequence elements or pathfinding. The authored order is installed before the first actor tick and no transient fallback is created. This fixes every initial-pose variant governed by the same Original call chain. |
| Done | Elevated blip reveal coordinates | Original soldier 58 is revealed at frame 23: `PC::SeesBlip` subtracts world-space eye points, then applies the isometric Y stretch. The resulting distance is about 599, just inside the 600-unit reveal radius. Rust used projected map Y while also retaining elevation in Z, effectively counted the height difference twice, obtained about 692, and left the soldier in its alternate silhouette profile until frame 71. The alternate idle has two shorter frame delays, which surfaced only later as a frame-79 Bored-state mismatch. | The blip range test now reconstructs and uses world eye points for both range and 3D opaque reachability. Exact frame-23 geometry coverage proves the world calculation passes while the old projected calculation fails. The replay comparator now checks the logical `blipped` flag directly, preventing profile-dependent gameplay state from hiding behind a later symptom. Replay advances through frame 179. |
| Done | Soldier special-remark timing | For ordinary soldiers, `RHElementActorSoldier::Execute` tests `RHSprite::IsAtStartOfAnim()` before `PerformAction`; this means exactly `current_frame == 0 && frame_count == 0`, not the broader sequence motion state named `Start`. The halberdman is a deliberate exception: its frame-40 test occurs after `PerformAction`. Applying one post-advance test to both branches made ordinary remarks one frame early once authored startup ordering was corrected. | Soldier Special remarks now sample the exact before/after sprite phase used by each Original branch. Non-soldiers do not inherit the soldier-only side effect. Focused coverage proves ordinary remarks use the pre-perform phase while the halberd frame-40 exception uses the post-perform phase. |
| Done | Synchronous scripted path assignment | At Lincoln frame 186 civilian 51 reaches a script-bearing waypoint. Its `ReachPoint` callback assigns path 64. Original `AssignNewPatrolPath` synchronously runs `Think(EVENT_RETURN_TO_DUTY)` before the callback returns; only afterward does `ExecuteWaypointScript` send `EVENT_AFTER_SCRIPT_GO_ON`, whose `DefaultEnroute` transition is therefore final. Rust deferred the assignment's self-stimulus until after the waypoint continuation and ended in `DefaultGotoRoute`. The same callback assigns soldier 87 a path, but its resulting normal-priority `Move` is only registered; Original's sequence-manager phase instructs it after all entity ticks, so movement cannot begin until frame 187. | `AssignPath` now yields the script VM to an engine-owned synchronization barrier. Path replacement and owner-local `ReturnToDuty`/`GoTo` registration finish before the script resumes, while the generated normal-priority movement remains deferred to the ordinary sequence phase. Focused native coverage verifies the yield contract; both actors match at frame 186 and the replay advances through frame 202. |
| Done | Deferred hit-flight initialization | Lincoln PC 126 finishes a punch after soldier 64's actor slot at frame 203. Original `TranslateHitDamage` synchronously applies concussion/knockout effects but only appends a `FALLING_HIT_UPRIGHT` order with `bComputeDirection = false` and the hitter as antagonist. Its first `ExecuteFallingHit` at frame 204 samples the live positions, changes facing, prepares flight, and raises priority. Rust performed those initialization side effects while translating the damage and exposed direction 13 and an active flight one frame early. | Hit translation now stores the exact order metadata without sampling geometry. Non-hard hit flight initialization runs once from the order's first-Execute path, so actor creation-slot ordering is observable exactly as in the Original. Focused coverage separates translation from first execution; replay advances through frame 218. |
| Done | Explicit hit-flight geometry and completion | Soldier 64's elevated hit flight reaches its last nonterminal increment at frame 219. Original `ReadyForTakeOff` constrains the goal to the starting sector, resolves its projection obstacle and full 3D elevation, and installs that obstacle immediately. `PerformFlight` adds the stored increment on its `DONE` frame and only snaps to the exact goal on `TERMINATED`. Rust inferred 2D versus 3D flight from a nonzero Z increment/obstacle, omitted the hit goal's sector/obstacle/elevation, and snapped to an implicit zero-elevation goal one phase early. | Active flights now declare `GroundPlane` or `World3d` geometry explicitly. Hit setup records the resolved layer, sector, obstacle, elevation, and 3D increment even when `dz` is zero; the shared integrator applies the final increment before retaining the flight for terminal snapping. Focused coverage protects the `DONE`/`TERMINATED` boundary, and replay advances through frame 231. |
| Done | Cached movement direction goal | Soldier 86 began walking on sloped ground with direction 13, but Rust recomputed its goal as 12 on the following frame from the remaining map-space delta. Original `RHPositionInterface::ComputeIncrementAll` derives direction once from the normalized 3D ground-plane increment and keeps it while the cached increment remains valid. | Ordinary movement no longer overwrites the direction goal every tick. Motion initialization and explicit trajectory invalidation/rebuild boundaries remain responsible for it, matching the Original cache lifetime. Replay advances from frame 3 to frame 18. |
| Done | Patrol endpoint direction side effect | At frame 18 Soldier 60 reaches path 1 waypoint 0, whose macro is backward-only and waits for 250 ticks. To obtain the preceding waypoint for its arrival Turn, Original executes `--path`, reads it, then `++path` on the live `RHPath`; at endpoint zero this round trip leaves traversal reversed. Rust performed the lookup on a clone, stayed forward, rejected the macro, and omitted its RNG draw. | Route-turn lookup now performs the same live iterator round trip, preserving the endpoint reversal that controls `DIR_FORWARD`/`DIR_BACKWARD` macro applicability. Focused coverage verifies waypoint zero remains selected while traversal flips backward. Replay advances to frame 32. |
| Superseded | Schema-4 x87 ground-plane arithmetic | Soldier 86 followed the correct bit-exact map positions on Lincoln's shallow slope, but its reconstructed elevation drifted because the old Original build retained plane intermediates in x87 registers. | The temporary Rust `f64` emulation was removed with schema 5. Plane construction now preserves the Original source operation order using ordinary `f32`, and the native Original uses the same scalar-SSE storage semantics. Exact coefficient/projection coverage uses Lincoln obstacle 52's point bits under the new contract. |
| Done | Original recorder | A useful comparison needs deterministic state and resolved commands on every tick. | The C++ game writes schema-2 JSONL with frame state, resolved commands, creation order, and RNG batches. Per-NPC records also expose all detection accumulators, maximum visibility, view/alert status, and every detectable's target, visibility, and edge latches, avoiding one-off instrumentation when a hidden perception total diverges. Deterministic/synchronous pathfinding is enabled for captures. Original commits: `502a7b3`, `a97c9dd`, and `8310b3e`. |
| Done | Multi-session Original capture | Returning to the menu, changing mission, loading a save, or restarting can construct or restore more than one engine state in one process. The old recorder retained its first campaign snapshot forever and crashed on the next engine. The Original's process-global NPC register counter also survived mission teardown, changing every later session's staggered periodic-AI/RNG phases while Rust reconstructed each mission from zero. | `-PARITYTRACE` is now a base path. Every engine session is closed independently and written to the next unused `-session-NNNN.jsonl` file with its own campaign snapshot and zero-based slice of the process-global RNG draw stream. NPC register numbering resets immediately before each mission stream constructs actors. Headers identify `mission_start` versus `loaded_save` and the actual initial frame. Rust now attempts both through the strict setup-RNG and frame-state oracle: the automatic Lincoln mission-start save in `original-fullgame-schema10-round2-session-0001.jsonl` matches all 5,553 frames and 7,394 simulation draws, while a non-reconstructible mid-mission save remains a loud first-boundary failure. |
| Done | Exact loaded-save fixture capture | Schema 10 retained campaign state but not the engine body restored from a mid-mission save. Reconstructing the mission and rewinding RNG could therefore agree briefly while serialized AI timers, sequences, paths, and entities were already different. Loading fixtures through menus also depended on the active profile and could overwrite its special Restart/Continue slots. | Schema 11 embeds the exact source v48 save bytes in `initial_save` with base64, length, SHA-256, slot, mission/version fields, and an explicit `linux_i386_rhsg_v48` or `windows_i386_gshr_v48` source profile. `-PARITYSAVE <path>` resolves the fixture before the data-directory `chdir`, validates its header and campaign mission, and routes a manager-owned save through the ordinary campaign/game load path without menus. Direct mode rejects active-profile special-slot aliases and suppresses automatic Restart/Continue writes. The Linux Original loader accepts RHSG fixtures only; preserved GSHR fixtures fail early with an explicit Rust-import requirement because representative Windows bodies fail this Linux engine ABI's stream signatures. Original commits: `03014252`, `b19cccbf`. Rust envelope commits: `2f00f14e6`, `fdf1c99d7`. |
| In progress | Native v48 loaded-save import | A schema-11 checkpoint is the only authoritative source for AI timers, active sequences, path state, VM members, and dynamic elements after a mid-mission load. Direct serde/bincode decoding is impossible because the Original stream is serialization-call-order dependent, contains context-sized bodies, and includes audited 32-bit ABI padding and pointer echoes. | Rust now has a bounded, contextual `LegacyReader`, explicit `GSHR` Windows-x86 and `RHSG` Linux-i386 v48 ABI profiles, and a strict top-level reader for the complete `RHEngine::Serialize` call order. Typed decoding covers campaigns and the engine prefix; both element passes and every concrete class observed in the corpus; compiled VM members; inline and manager-owned sequences with all pointer fixups; FastFindGrid; user lock; HikingGuide; the standalone trajectory projectile; failed paths; minimap and selections; SequenceManager; ground marks and titbits; global VM/globals/timers/camera/AI; Pathfinder; mission statistics; pending shield state; and exact EOF. Mission initialization now retains the omitted sparse grid/element topology and persists exact Original creation-order identities instead of deriving them from Rust IDs. Atomic adoption plans now cover engine preamble scalars and identity counters; exact position topology; common element/sprite, NPC view, and local-AI core state; FastFindGrid runtime state; SequenceManager identity/order/state; user lock and selections; follow/view references; ground marks and titbits; global AI seek/archery/alert state; and mission statistics. Every plan validates into owned candidate data before mutation, and incomplete authoritative sections remain deliberately disconnected from replay rather than being replaced with mission-start defaults. All five current compressed Linux fixtures decode completely, contain 127 static elements and zero dynamic elements, and exercise canonical EOF plus the explicitly recognized historical copied-save trailing-byte artifact. The runner accepts plain or zstd-compressed traces and invokes this mission-aware decoder before replay. Its first frame still compares mission-start Rust state with loaded Original state, so remaining work is the actor/NPC leaf remainder, campaign/VM/timer/path/projectile/host state, final atomic orchestration, and then frame-by-frame parity repair. Dynamic factories remain required for general save loading even though current Linux fixtures do not exercise them. Windows-save loading remains a separate compatibility track until native Linux adoption is complete; no guessed byte skips or replay-specific state substitutions are allowed. |
| In progress | Linux-v48 adoption completeness audit | Passing the original set of child preflights was not sufficient: common adoption still omitted PC/Human state, concrete object/FX leaves, actor sequence ownership and VM heaps, camera/locker state, several NPC continuation fields, identity-resolved minimap state, and required post-load side effects. Original zero-based order IDs were also mixed with Rust's nonzero ID domain. Initialized Linux AI enums are ordinary full 32-bit words; the exceptional arbitrary values come from explicitly serialized but uninitialized dormant fields such as `mOldState`, not from overlapping short enums. VM `Location` handles additionally require one allocation order spanning interleaved element VMs, grid zones, HikingGuide waypoints, and the global VM. | The atomic coordinator and corpus preflight now expose the first named unsupported section without mutating the live mission. Gate pointers are mapped isomorphically by per-kind construction ordinal across Original's building/lift → jump → reinforcement order and Rust's building/lift → reinforcement → jump order (`309b9a444`). Common actor continuations, remaining Soldier/Civilian state, sound/Messenger/RHGame state, camera normalization, complete PC/Human leaves, and Object/Bonus/Scroll/Target/FX leaves have dedicated strict plans (`dcc1364e5`, `d70e3b7ae`, `8721d1517`, `92b27aa57`, `5239a034b`, `c54d541dc`). Gameplay-relevant Linux enums retain strict full-word validation, while dormant uninitialized storage is preserved raw; every restored order-related field shares the manager's `legacy + 1` identity mapping (`fc152d8e7`, `b8f97fedf`). Exact attached-scroll identity, body-visitor/freeze latches, and the complete serialized NPC view continuation are now authoritative Rust state (`f458340ad`). Base-element delayed map/world positions are restored with finite-value validation and consumed at the original Actor Hourglass boundary, including map-before-world priority and line-crossing continuation. Identity-resolved minimap restoration, common actor ownership, and the shared VM arena are being integrated before the public install boundary is enabled. Current Linux fixtures still contain zero dynamic elements; factories remain mandatory for arbitrary saves after this corpus is running. |
| Done | General v48 dynamic-element construction plan | The current Linux checkpoint corpus contains no elements created after mission initialization, but arbitrary saves can contain dropped projectiles, pickups, scrolls, wasps, nets, capes, and newly instanced PCs. Reusing mission-start entities or fabricating empty sprites would corrupt creation-order references and immutable animation/profile data before phase-two payload adoption. | A strict, mutation-free phase-one plan now mirrors the Original load factory: it validates static identity/class matches, resolves every object or character sprite master, constructs missing elements in saved order, consumes the restored global creation counter once per constructor, and then installs the exact saved creation-order identities. Every Original v48 factory kind has a concrete Rust entity mapping; missing profiles/masters, mobile masters, class/factory disagreement, stale runtime dynamics, and counter overflow are explicit errors. Level loading preloads all Original object masters and every campaign character master so construction never depends on which types happen to be authored in the current mission. The final atomic coordinator still needs to invoke this plan before the existing payload plans. |
| Done | Deterministic standalone trajectory save identity | After the element array, Original serializes a separate `RHElementProjectile` used for trajectory previews. No concrete item constructor assigned this helper's `muwClassID`, but its inherited Element payload still wrote the field, making two bytes of otherwise identical checkpoints depend on uninitialized process memory. | Original assigns the valid generic `RHCLASSID_OBJECT` immediately after constructing the helper. It never enters the engine element registry, so gameplay classification and element dispatch are unchanged. Rust treats the corresponding field in existing Windows/Linux saves as opaque at this one source-derived site. Original commit: `db79ecbf`. |
| Done | Linux copied-save EOF corruption | `RHSaveGame::CopyFiles` implemented Continue/Restart slot copies as `while (!feof(in)) fputc(fgetc(in), out)`. The final failed read was converted to `0xff`, so every copy appended one non-serialized byte; repeated copies could accumulate more. One current compressed Linux Continue trace contains exactly this artifact. | Original now reads first and writes only successful bytes, so new copied saves end at the final shield reference (`79f31f43`). The Rust v48 decoder retains strict boundaries while explicitly consuming and reporting only a bounded run of trailing `0xff` bytes for the Linux ABI; Windows and every other trailing value remain errors (`b5c617862`). |
| Done | Hidden validity and LOS diagnostics | Session 2 could not directly explain why Original rejected PC 198's frame-452 bow command: the target's hidden state, authoritative ammunition, identity gates, and the visibility query that maintained the hidden state were absent from the snapshot. | Schema 3 now records `blipped`, human camp/unconscious/VIP/civilian state, all nine PC inventory counters, and every opaque 3D reachability query with bit-exact endpoints, result, cache metadata, exact blocking reason, and blocker geometry where applicable. Original commits: `a9404d5` and `c6f83ca`. |
| Done | Exact Original visibility | Original's performance cache reduced a 3D ray to a lossy integer key in one of 2,000 buckets and reused the cached Boolean without incorporating obstacle state. Distinct rays could therefore share stale visibility, making target discovery depend on unrelated prior queries. Rust already tested the exact active obstacle set on every query. | The Original now always executes its existing exact obstacle test. Legacy key/offset values remain diagnostic fields in the trace, but new captures report no cache hits. This is a general engine-correctness change rather than a replay-specific exception; a fresh recording is required because session 2 contains the old cached result. |
| Done | Structured Rust frame dump | Broad parity snapshots did not expose transient AI, sequence, movement, and vision state, forcing repeated one-off logging changes. | `original_parity_replay --dump-jsonl` writes stable JSONL records containing the complete serializable engine snapshot, resolved commands, RNG cursor/batch, entity mapping, and parity differences. `--dump-from`, `--dump-through`, and repeatable `--dump-entity kind:index` filters keep targeted captures manageable; omitting the entity filter retains the whole engine. An ordinary first-divergence run now retains a rolling full-engine window and automatically writes the divergent frame plus its 32 predecessors to a unique temporary JSONL file. Explicit dump and `--scan-all` behavior remains unchanged. |
| Done | Visual parity replay | A headless mismatch report does not show how the divergent frame looks in motion. | `original_parity_replay --visual` runs the same authoritative resolved-command/RNG replay in the normal window/GPU runner, ignores live gameplay input, renders the decoded map and current sprites at a visible rate, and freezes the first divergent state until the window is closed. Headless remains the default for fast iteration. |
| Done | Corpus frame-zero screenshots | Save adoption can be logically wrong in a visually obvious way even before the first simulation frame, but opening every replay interactively is impractical. | `original_parity_replay --frame-zero-screenshot-dir DIR` constructs the exact recorded campaign, adopts the embedded Original save, and captures the pre-tick viewport through the normal game-session renderer. PNGs therefore include the real HUD, selection markers, sprite shadows, cursor, and saved camera rather than the parity visualizer's simplified world-only view. Corpus-relative path components are flattened into the filename so profiles with identical save-slot names cannot overwrite one another. The required GPU window is hidden unless `--visual` is supplied, and the one-shot process exits after capture. |
| Done | Global RNG replay | Bored/waiting choices affect head direction and later AI behavior; reproducing only a seed is insufficient across different implementations. | Rust can consume the trace's filtered global libc `rand()` draw stream and records typed Rust consumption sites. Startup and every frame assert the exact draw cursor. |
| Done | Stable RNG domains | Rebuilding the Original shifted every raw return-address offset, so the first quiet-music draw was misclassified as simulation RNG despite identical values and state. | Original schema 3 introduced a parallel `simulation`/`audio` domain for every global libc draw while retaining callsite offsets as diagnostics (`a72d8c0`). The schema-8-only Rust runner consumes stable domains directly and has no symbol lookup or hard-coded-offset fallback. A fingerprinted RNG sidecar avoids rescanning multi-gigabyte traces during every first-divergence iteration. |
| Done | Isomorphic identity | Original and Rust IDs and hidden startup objects differ, while the logical world is equivalent. | The runner constructs a mission-start entity bijection from kind, stable data, and creation order. Original's hidden 31-object prefix is retained where creation order is itself gameplay state, such as staggered detection. |
| Done | Trace command decoding | Raw mouse/keyboard behavior is not under test. The Lincoln capture first emits the recorder's semantic `stop_pc` command at frame 232; the engine command already existed, but the trace-side enum omitted it and aborted before replaying the frame. | Recorded resolved commands, including `StopPc`, are translated through the entity bijection; unsupported command values and malformed or non-contiguous traces fail loudly. |
| Done | Direct-click resolved commands | Several Original click handlers construct their seek/action sequences directly instead of using the shared `AddInteractionWithSeek` helper. Schema 5 therefore omitted object/net pickup, target actions, corpse/shoulder interactions, one VIP speech route, and associated double-click speed changes. In session 0004 an omitted scroll click remains queued through later door routes and first becomes visible when its seek resumes at frame 508. | Schema 6 records each accepted direct interaction at its source handler, after validation but before launch, and records non-macro double-click `MakeFast` changes. Recording at the generic sequence manager would incorrectly include AI/script work, so shared helper hooks remain in place and only genuine bypasses receive explicit hooks. Original commits `11b05f88`, `f733661d`, and `03340b1b`; Rust rejects schema 5 rather than inventing a target from later state. |
| Done | Isomorphic resolved-command sector identity | Schema 6 records `RHSector::GetSectorNumber()` for a group move, which is an index in the Original's heterogeneous fast-grid sector table. Rust treated Lincoln index 434 as canonical motion sector 434, failed gate routing, and then attempted an invalid direct move across the sector boundary. | The replay boundary resolves the recorded destination and layer to exactly one Rust motion area and panics on missing or ambiguous identity; this maps the command to canonical sector 59 and produces the Original's gate route through sectors 24 and 43. Cross-sector gate failure now follows `RHSequence::AppendMoveToSequence`: speak the unable response and append no direct move. The baseline advances through frame 66. |
| Done | Movement-shaped wait execution ownership | `WAIT_FREE_LIFT` carries gate and sector data in `RHSequenceElementMovement`, but Original executes its idle order through ordinary actor `Execute` before checking and reserving the lift. Rust excluded it from the movement driver, then also rejected it from generic execution while the PC retained `MovingFast`, leaving the free lift unreserved at frame 67. | Generic actor eligibility now follows command ownership rather than payload shape for `WaitTimer` and `WaitFreeLift`. The first ladder wait executes, reserves, terminates, and selects `PassDoor` on the Original frame. Initial door-walk installation records the selected order and active pass without applying movement action state before its first Execute. Replay advances through frame 74. |
| Done | Ladder-up transition terminal state | The low/direct ladder route's `TransitionWaitingUprightClimbingLadderUp` reaches its terminal edge at frame 75. Original sets `OnLadder/Moving` on both `DONE` and `TERMINATED` before selecting `PassingDoor`; Rust selected the successor while remaining Upright. | Movement transition state effects now include both ordinary and alerted ladder-up start transitions on their `DONE`/`TERMINATED` edges. The successor remains selected but unexecuted until its own owner slot. Replay advances through frame 86. |
| Done | Fast climb dispatch survives lift translation | The inter-gate move is instructed as `ClimbingLadderUpFast`. On its first Execute, Original dispatches that non-animation token by running the ordinary climb sprite motion twice. Rust translated the already directional fast token again, obtained ordinary `ClimbingLadderUp`, and then tested only the translated sprite action for fast semantics, so its first slot performed one zero-distance animation tick. | Fast-climb execution now retains the sequence order's dispatch token while continuing to use the ordinary translated sprite animation for both motion calls. This is generic to authored fast ladder and wall movement in either direction. Replay advances through frame 108. |
| Done | Ladder-exit transition Execute effects | `TransitionClimbingLadderUpWaitingCrouched` has two separately timed effects: its raw `START` aligns the actor to the canonical gate midpoint and recomputes the projected position without changing topology, while raw `DONE` changes a PC to Crouched/Waiting (non-PC actors to Upright/Waiting). Rust played the transition without either effect. | Active door-pass ladder-exit transitions now queue the exact `START` midpoint alignment and `DONE` state update for both ordinary and alerted variants. The later `PassingDoor` order remains the only sector/layer topology mutation. Replay advances through frame 128. |
| Done | Door-pass successor selection is state-neutral | After frame 129's `PassingDoor`, Rust queued the following `WalkingUpright` order and immediately set the actor to Moving through `apply_door_pass_continue_state`, even though the last executed order was still `PassingDoor`. Original leaves the actor Waiting until the walk's next owner slot returns `RHMOTION_START`. | Door-pass continuation orders are now only installed/selected. Posture and action-state changes remain owned by their authored transition or movement Execute arms; the generic movement `START` handling changes the following walk to Moving on the next frame. Replay advances through frame 208. |
| Done | Authorized cross-sector formation destinations | The recorded frame-171 group click is `(1789,990)`, but Original's `PerformGroupMove` first places Robin's upright move box at every per-PC formation slot and calls `FindAuthorizedPosition`. On narrow wall sector 77 this shifts the actual authored destination to `(1787.2975,992.0809)`, which is directly thick-reachable. Rust fed the raw click into gate routing, then used a blanket wall/ladder shortcut to hide its missing authorization step. | Every non-door cross-sector formation slot is now resolved through the same move-box authorization used by same-sector movement before gate A*. The resolved point drives the gate goal, final movement element, and marker. Wall/ladder sources no longer bypass the ordinary posture-sized thick-reachability test. Replay advances through frame 266. |
| Done | Schema-6 semantic overlay identity | The frame-267 command's raw Original sector index `245` has no Rust motion-area counterpart at `(1738,988)`/layer 4 because the selected object is canonical door overlay 22. Treating every recorded group goal as a motion sector either panicked or would erase the door click's routing behavior. | When the command point/layer uniquely identifies a Rust motion area, schema-6 replay still uses its canonical motion-sector override. A unique door or jump overlay instead remains a semantic spatial selection and re-enters the engine's ordinary canonical door/jump hit path. Missing and ambiguous identities still fail loudly. Replay advances through frame 332. The next recorder schema must store this semantic goal explicitly rather than requiring spatial reconstruction. |
| Done | Authored climb direction survives path bends | PC 126's wall route contains two orders authored as `ClimbingWallUp`; its second waypoint bends briefly in the opposite direction. Original's `DetermineMovementAnimation` translates the movement element once, so both orders remain wall-up. Rust re-ran the lift dot-product for every waypoint, changed the second order to wall-down, reset the sprite row, and moved one frame early at frame 333. | Directional wall/ladder actions—including alerted and fast variants—are now recognized as already translated and remain unchanged at execution. Only generic upright movement is translated from the live lift geometry. Replay advances through frame 354. |
| Done | Door exit successor state is Execute-owned | At frame 355 the wall-down walk terminates and merely selects `TransitionClimbingWallDownWaitingUpright`. Original remains `OnWall/Moving` until that transition's `DONE`/`TERMINATED` effect; Rust changed to Waiting while constructing the successor. The transition also inherited no destination, which erased its prior goal/facing, and the later `PassingDoor` slot restored Moving while merely selecting the following walk. | Door transitions are materialized at the actor's current destination with direction computation disabled, selection preserves the preceding state, and wall-down exit is explicitly an in-place transition. The saved movement state is restored only when the concrete following distance-motion order reaches its owner slot. Replay advances through frame 377. |
| Done | Door clicks retain the terminal gate | The frame-267 semantic click targets canonical door 22. Rust used `find_path_to_door`, whose contract deliberately removes that door and returns its near-side anchor, then emitted a plain terminal move. Original group movement uses `FindPathIntoDoor`, retains the target in the gate list, and switches to its `PassDoor` element at frame 378. | Group movement to a selected door now uses the full path-into-door result and disables the redundant trailing point move. The ordinary gate builder emits the target door's approach, assert, pass, and post-pass assert, including its existing lock/lift behavior. Replay advances through frame 391. |
| Done | Normal AI timer sees same-slot order mutation | Soldier 68's bored idle terminates on frame 392 and the first RNG draw changes its current order to `WaitingUprightBoredRandom`. Original then reaches the NPC timer in the same owner slot; `GetAnimation()` reads that mutated order and suppresses `LookSidewards`. Rust built the timer context from stale `Sprite::last_action`, changed to the looking substate, and reused the next bored-time draw as a look direction. | The normal-timer Think boundary now overrides `AiContext.self_animation` from the live sequence-manager order (or `NonanimationEnd`), matching the existing re-entrant and periodic AI boundaries. Draw ownership returns to the bored timer and replay advances through frame 533. |
| Done | Door overlays beyond sector polygons | The frame-534 schema-6 command targets a door overlay whose authored click polygon contains `(1928,945)` but whose fast-grid sector polygon does not. The motion/overlay reconstruction rejected it as missing. | When neither a motion nor overlay sector polygon contains a schema-6 destination, replay accepts the semantic spatial route only if the engine's independent canonical door click-polygon lookup resolves the point. Missing hits still panic instead of silently snapping to ordinary terrain. Replay advances through frame 549. |
| Done | Frame-525 post-seek transition result | PC 126 reaches scroll 111 while its selected seek order is the walking-to-waiting end transition. Original's in-tolerance `PerformSeek` branch skips the transition's sprite motion and launches `TAKE`, but returns `TERMINATED` through the surrounding transition Execute arm; that arm still changes the actor from Moving to Waiting before the interaction is instructed. Rust terminated the seek externally while exposing `IN_PROGRESS`, bypassed the end-transition state effect, and made `TAKE` generate a redundant second transition. | A successful pre-motion entity-seek handoff now exposes `TERMINATED` to its current movement order without advancing the sprite. Pending movement-transition terminal posture/action-state effects run before the deferred post-seek interaction is instructed. This is generic to every post-seek command and end-transition type. The repaired Lincoln diagnostic passes frame 525 and reaches the next RNG mismatch after frame 545. |
| Done | Live sequence-manager FIFO | A frame-246 cross-sector group move begins with `AssertPosition`, which terminates immediately and opens a normal-priority `Move`. Original `RHSequenceManager::Hourglass` drains its registration list with a live `while` loop, so the successor is instructed to `MoveOk` in the same manager phase even though its first actor execution waits until frame 247. Rust detached the initial queue into a vector and only spliced immediate/Wait callback work, leaving the normal successor `Todo` for one frame. | The Rust sequence phase now continues to a fixed point after every action: re-entrant immediate/Wait actions retain stack-like front precedence, while newly registered normal actions append behind already waiting manager work and still drain in the current frame. Focused ordering tests cover both front and tail rules; replay advances through frame 277. |
| Done | One door sub-order per actor slot | At frame 278 Rust completed `TransitionWaitingUprightClimbingWallUp`, immediately fired the following `PassingDoor`, and selected `ClimbingWallUp`. Original `Hourglass` executes exactly one current order, then `DoNextOrder` only selects its successor: frame 278 ends on the source side with `PassingDoor` current, frame 279 executes the topology swap without moving, and the climb first executes at frame 280. The premature climb had also masked a missing terminal `OnWall` posture update. | `PassingDoor` is now materialized as a real actor order. Door transitions and walks may install one successor but never execute it in their completion slot; the action point consumes the next owner slot and only installs the climb. Terminal wall-transition side effects are sampled after `TILL_LAST_FRAME` can rewrite the motion result to `Terminated`. A focused low-wall regression covers the three distinct slots; replay advances through frame 279. |
| Done | Elevation-line crossing while climbing | At frame 280 PC 126's first wall-climb step had matching map coordinates but retained the outside flat plane at elevation `220.001`; Original crossed the lift's elevation line, selected the lift-side obstacle, and reprojected to `219.96617`. Rust's eligibility predicate incorrectly excluded `OnWall`, `OnLadder`, and several carrying postures. Original excludes only flying actors, humans that are themselves carried, stationary actors, and out-of-grid positions. | Line-crossing eligibility now follows the Original predicate: wall/ladder climbers and actors carrying someone remain eligible, while flying actors and humans with a live carrier do not. A focused predicate regression covers wall, ladder, flying, carried, and out-of-grid cases; replay advances through frame 369. |
| Done | Selected non-movement condolence cleanup | At frame 370 a new route's matching `AssertPosition` terminates synchronously between the old climb and its successor `Move`. Original keeps the Assert selected during translation, so its actor-base `SendCondolationCard` clears the old sprite map goal; Rust tied goal clearing to `active_movement`, which Assert never enters. A first broad capture also showed why an immediate sibling at frame 77 must not clear the movement selected again after its callback returns. | Pending condolence cards now record whether their element was selected at the terminal boundary. Dispatch clears the goal only while that selection remains authoritative; movement tracking is detached independently. `AssertPosition` retains its incoming selection throughout translation. Focused regressions cover selected non-movement cleanup, immediate-sibling preservation, and an outgoing movement interrupted after the incoming action becomes selected; replay advances through frame 381. |
| Done | Door transition projection without topology mutation | At frame 382 the high-crenel climb-up transition reaches its `DONE` action point. Original teleports to the door midpoint and selects the outside sector's projection obstacle/plane, but remains a member of the wall lift until the later explicit `PassingDoor` order. Rust's shared special-motion helper used the projection lookup layer/sector as the actor's new topology and moved PC 126 from layer 3/sector 62 to layer 2/sector 50 early. | Special-motion finalization can now resolve obstacle/material/elevation from an explicit projection sector without applying that sector to actor membership. The crenel transition uses this path; `PassingDoor` remains the sole topology swap. A focused regression asserts far-side projection selection preserves current layer/sector; replay advances through frame 382. |
| Done | Lift-facing assignment only on order initialization | The crenel action point at frame 382 recomputes its point-out increment and changes direction-goal from 0 to 15 after that frame's `Turn`. Original keeps that goal; frame 383's next `Turn` rotates 0→15. Rust reapplied the authored lift direction every tick, erasing the action-point goal. At frame 403 Rust also snapped direction back to the lift direction during terminal posture cleanup, which Original does not do. | Door/lift authored facing is now gated by the actor's exact `execute_order_initialising` flag in both normal and FrozenAll paths; ordinary `Turn` remains per-frame. Transition completion changes posture/action/position only and does not reapply direction. The fresh-order lift callback regression explicitly models actor initialization; replay advances through frame 403. |
| Done | Unreserved wall-lift release | Wall routes do not contain `WAIT_FREE_LIFT`, so they never increment the lift occupancy counter. Original nevertheless calls the shared wall/ladder release on exit: its unsigned zero decrement wraps to `65535`, its immediate signed comparison reads `-1`, and it normalizes the state back to zero. Rust's stricter `checked_sub` invented a reservation invariant and panicked when PC 126 left wall sector 62. | Lift release now models the Original's effective clamp-to-zero operation for both directions, including clearing direction flags and cooldown at zero. Door passage routes the release through the shared direction-specific lift methods; ladder reservations still decrement normally, while unreserved wall exits remain state-neutral. |
| Done | Generic crouch-transition completion | PC 126's door route materialized `TransitionCrouchingUp` as a generic actor order. At frame 411 its sprite returned `DONE`; Original immediately set Upright/Waiting and sent `MSG_STATURE_CHANGE_END`, while keeping the order alive until its later `TERMINATED` edge. Rust had the correct state mapping only in the separate movement driver, so the generic owner left the PC crouched. | Generic actor execution now applies crouch-up/down state changes on both `DONE` and `TERMINATED`, turns on every execution slot like the Original arms, and emits the PC stature-change notification on both edges. A focused table regression covers both directions and both terminal motion states; replay advances through frame 412. |
| Done | Bow facing sampled on shooting initialization | At frame 413 mission script 386 translates Soldier 70's `ShootBowOnce`, queueing bored→waiting, equip, load, shoot, and unequip orders. Original preserves direction goal 1 throughout those preparation animations and samples the live target only when `ShootingWithBow` initializes at frame 444. Rust eagerly set goal 2 while registering `ActiveShot`, 31 frames before the shooting order executed, then resampled every shoot tick. | Shot translation now only registers the shot and appends orders. The shooting order samples its live target exactly on actor-order initialization, retains that goal for the rest of the order, and still calls `Turn` every execution slot. Regressions verify registration preserves an existing goal, the initialization sample uses C++ ground coordinates, and later target motion does not retarget the same shot. |
| Done | Door continuation selection does not execute movement | At frame 414 `TransitionCrouchingUp` terminates Upright/Waiting and `DoNextOrder` merely selects a zero-distance `WalkingUpright` continuation. Original remains Waiting; at frame 415 the walk's exact-destination initialization returns `TERMINATED` before `START`, so it never enters Moving. Rust's transition-resume installer eagerly applied the selected walk's posture/action state twice and reported Moving a frame early. | Special-motion continuation installation now attaches the order and `active_movement` bookkeeping without applying Execute-owned posture/action effects. The next actor slot remains authoritative; immediate-arrival walks that never return `START` retain Waiting. Replay advances through frame 415. |
| Done | Exact-goal movement completes on its first Execute | The zero-distance door continuation selected at frame 414 initialized at frame 415 with zero sprite displacement. Rust's movement loop treated every zero-speed sample as still in progress, even though Original's exact-destination `PerformMotion` returns `TERMINATED` immediately. That delayed the final `PassingDoor`, goal cleanup, condolence cascade, and successor `MoveOk` by one frame. | Zero speed defers only a motion that remains a positive distance from its goal. Exact-position ordinary movement and pre-motion seek-tolerance arrivals enter the shared completion tail without displacement. A focused regression preserves the non-arrived stationary case; the replay now matches through frame 464. |
| Done | Crenel transition projection does not enter topology | At frame 461 the high/direct climb-down transition's `DONE` action point moves PC 126 to the inside projection and changes posture to `OnWall`, but Original retains outside sector 50/layer 2. Rust used the projection sector as logical topology immediately, then reset the completed transition's posture to `Flying` again on its `TERMINATED` tick. The separate `PassingDoor` order is the only operation allowed to enter sector 62/layer 3, at frame 463. | Crenel climb-down now resolves the inside obstacle/material/elevation through the projection-only finalizer while preserving current membership. Crenel transitions assign their initial `Flying` posture only on exact order initialization, so `DONE` state persists through `TERMINATED`. Replay advances through frame 506. |
| Done | Speed changes rewrite lazy door-route orders | PC 126 receives `MakeFast` while a high-wall `PassDoor` route is active. Original has already materialized the entire route inside its movement element, so `RHSequenceElementMovement::MakeFast` rewrites the future exit walk to running and `InsertTransitionStart` turns its zero-distance tail into `TransitionWalkingUprightRunningUpright`. Rust stored the future door steps separately and rewrote only the currently materialized sequence orders, selecting the final `PassingDoor` one frame early. | `actor_make_fast` now rewrites the lazy door tail and applies the Original's distance-based start-transition insertion algorithm across those steps. This is based on route geometry and animation distance, not a replay frame or counter threshold. Replay advances through frame 507; equivalent lazy-tail handling for the other posture/speed rewrite operations remains a general follow-up. |
| Done | Dynamic isomorphic entity mapping | The mission-start bijection covered only static entities, so the first runtime-created arrow previously caused an unmapped-ID panic. Original's recorded `entity_id.index` is merely its current element-array position and can shift after physical deletion; immutable `creation_order` is its actual within-session identity. | The runner extends the bijection after each frame using stable creation order, kind, exact logical creation state, and per-engine creation order as the final tie-break. It refreshes the current raw-index view from the stable registry, rejects unequal numbers of runtime creations, and never requires Original and Rust numeric IDs to match. A focused regression covers an Original index shift for an existing creation serial. |
| Done | Projectile initialization and inactive retention | Frame 446's Arrow from Soldier 70 to Target 96 had bit-exact flight position and projected velocity, but Rust used that velocity as element facing, left sector empty until landing, deactivated on impact, and physically removed the entity. Original keeps element direction at its constructor default, stores gameplay flight direction separately, exposes trajectory-resolved membership throughout flight, records the impact frame before its stationary `Refresh` deactivates the arrow, and retains its inactive slot. | Projectile flight direction is now separate gameplay state used by damage. Trajectory construction installs landing sector/layer without prematurely projecting flight elevation, every Hourglass snapshots old position, and a retirement latch preserves the terminal pre-refresh snapshot before deactivating the retained arrow in its following owner slot. All projectile removals retain inactive tombstones; the replay matches through frame 460. |
| Open | Projectile visual refresh phase | Original arrow row/frame mutation belongs to `RHElementArrow::Refresh`, not projectile `Hourglass`; a newly appended arrow's logical trace can therefore precede its first directional visual refresh. Rust currently computes the directional sprite in its simulation tick. | Move projectile row/frame refresh into an explicit renderer/refresh phase (or explicit next-frame pending visual state). Do not infer phase eligibility from trajectory `frame_count`; that counter is projectile motion state, not a scheduling token. |
| Open | Materialize door `Select` action points | Building-door `Select` is also a real non-animation order in the Original and therefore consumes one actor `Hourglass` slot. Rust still invokes its existing hulk callback while advancing to the next translated door step in the same slot. | Route `Select` through the generic actor coordinator as a queued order, preserving the same one-order-per-slot rule now used by `PassingDoor`, and add a building-door timing regression. |
| Done | Stable action and command names | Schema 2 stored bare action/command ordinals. The replay confused semantic `RHACTION_BOW = 1` with portrait slot 1, and rebuilt Rust/Original command enums have intentional ordinal differences. | Original schema 3 introduced stable semantic action names (`da51753`) and command names for actor state and resolved commands (`f7acb56`). The schema-8 runner requires the semantic fields, ignores diagnostic ordinals, and has no ordinal decoding fallback. Rust has a semantic `SelectResolvedAction` command. |
| Done | Continuous resolved orientation | Original `PerformOrientation` continuously derives bow aim, throwable facing, help-climb facing, and beggar facing from the cursor without forwarding a messenger command. Schema 2 therefore cannot reproduce PC 198's frame-532 `RaiseBow` transition. | Original schema 3 now emits a bit-exact `orient_action_at` record for each actor immediately before every live gated mutation, including semantic action, resolved 3D target, and map cursor (`867eb4e`). Rust applies that cursor-independent operation to only the mapped PC through the ordinary bow/throw/help/beggar orientation behavior. The old raw orientation event was removed to prevent duplicate application or focus re-resolution. Focused tests cover schema decoding and targeted throw facing. |
| Done | Empty schema-3 RNG batches | A frame with no libc draws legitimately records empty `values`, `callsite_offsets`, and `domains` arrays. The Rust reader treated the absent draw-domain payload as schema-2 data and demanded a binary classifier even though there was nothing to classify. | Both the streaming replay and its RNG-sidecar scan now accept an entirely empty batch without weakening length/domain validation for nonempty batches. |
| Done | Parade timer stop gate | At frame 324 Soldier 82's Parade timer expired after the actor had already returned to `WaitingSword`. Original queues `STOP_PARRY_SWORD` only for exactly `RHACTIONSTATE_PARRYING_SWORD`; Rust queued it unconditionally, and terminating the stale element synchronously emitted `EventDone`, reconsidered combat, and consumed three extra RNG draws. | The timer still returns to ordinary swordfight and restarts its 20-tick heartbeat, but emits StopParrySword only from the exact normal-parry state. Regression coverage verifies `WaitingSword` and `ParryingSwordLow` do not stop while `ParryingSword` does. The schema-3 replay advances to frame 337. |
| Done | Parried-hit strike learning | Soldier 82 can remember only one enemy strike. It knew D until translating the intervening E damage at frame 335; Original calls `MakeBadSwordstrikeExperience(origin->GetCommand())` before its sound/parry return, so even the fully parried E evicts D and the guard correctly ignores the next D at frame 338. Rust returned from its parry-sound branch before the learning block and incorrectly retained D, producing an extra proposal draw and Parade state. | Live-command learning now occurs before the parry return for every translated sword-damage element. The existing live-command rule remains intact—payload strike is not substituted—and low-skill eviction is regression-tested with a parried hit. Replay advances to frame 367. |
| Done | Repeated parry command | At frame 367 Soldier 82 was already in either normal/low `ParryingSword` state when reacting to another known strike. Original terminates both `PARRY_SWORD` variants immediately in either active parade state, leaving the existing animation/action state alone and exposing the fallback Wait command. Rust skipped only the transition but appended a fresh endless hold order, exposing `ParrySword`. | Parry dispatch now terminates without orders when either parade variant is already active; normal and low requests from both active states have direct regression coverage. Replay advances to frame 452. |
| Done | Enum/state representation | The trace exposed a missing Original discriminant rather than an ID mismatch. | Added the missing discriminant and made the compared logical state representable without substituting fake values. |
| Done | Sequence callbacks and owner boundaries | Original sequence completion, condolence cards, and re-entrant AI callbacks can execute synchronously before an outer dispatch returns. | Sequence effects now preserve callback FIFO order and owner-local synchronous boundaries, including nested condolations. Regression tests cover callback/condolation ordering. |
| Done | `GOTO_DONTSTOP` and waypoint patrol | Patrol setup and continuation differed before visible movement. | Matched Original flag handling, AI callback/lifecycle behavior, and waypoint-route advancement. |
| Done | AI `Move` promotion | A newly launched patrol move must not interrupt the actor's current order before that actor's update slot. | AI moves enter the sequence pipeline and defer owner instruction. Callers that are true synchronous native/condolation boundaries explicitly drain the deferred action; ordinary patrol does not. |
| Done | Path scheduling barrier | Even synchronous Original A* observes `MoveWaiting` for the frame where a request is issued, then exposes `MoveOk` in the next path phase. | All A*-requiring moves enter the queue. Synchronous mode computes and installs the result at the next path barrier instead of bypassing the queue or adding an asynchronous extra frame. |
| Done | Movement animation loops | A walk/run animation row looping does not terminate travel; only `TILL_LAST_FRAME` observes natural animation termination. | `Sprite::perform_motion` keeps ordinary movement alive across animation loops. Unit coverage: `perform_motion_walk_does_not_terminate_when_animation_loops`. |
| Done | Motion start tick | Original initializes a new motion order and advances its animation in the same invocation, making frame-zero distance immediately available. | `perform_motion` performs that first increment while the general action entry point retains its distinct start semantics. Unit coverage: `perform_motion_start_tick_advances_and_emits_frame_zero_distance`. |
| Done | Direction sector classifier | `atan2` plus rounding differed for boundary vectors created by anti-collision. | Ported `SBGeoVector2D::GetSector0to15` as the same f32 half-plane classifier and literal constants. |
| Done | Anti-collision direction | During deviation, Original faces the committed deviation step and rebuilds its normal increment when deviation ends. | Direction/increment are derived after the committed step; deviation recovery invalidates and recomputes the normal goal increment. |
| Done | Mobile anti-collision scope | Original calls `IsBlockedByMobile` only when `GetMobileRepulsiveObjects` reported a mobile intersecting the actor's future box. Layer-wide mobile geometry must not block an unrelated actor's recovery or break-through corridor. | Mobile blocking checks in deviation recovery, ordinary commit, and break-through are gated by the future-box intersection result. |
| Done | Transition anti-collision | Soldier 89 retained `deviated=true` through a running-to-walking transition at frame 25, so frame 26 incorrectly used anti-vibration turning and remained at direction 3. Original sends every nonzero `PerformMotion` distance, including `TILL_LAST_FRAME`, through `UpdatePositionAntiCollision`; it recovered on frame 25 and ordinarily turned to direction 4. | Nonzero movement transitions now use the same anti-collision step and recovery commit as walking motion while retaining their distinct animation-completion semantics. |
| Done | Vision geometry | Rust applied the map's isometric Y correction to the target delta at a point where Original compares raw view-space X/Y, shifting cone membership. At frame 38 Rust then saw soldier 82 from soldier 83 while Original kept looking left. Original applies the correction by scaling the rotated cone-side vectors instead; Rust had omitted that second part and consequently made diagonal cones too wide. | Cone determinants now compare the raw target delta against side vectors whose Y components carry `ASPECT_RATIO`, exactly as `RefreshView` and `IsDetecting` do. The frame-38 soldier geometry has direct regression coverage. Normal eye movement also uses the active PI/8 step and PI/80 iterator constants instead of values from Original's commented old-values block. |
| Done | `Turn`/`FaceTo` during movement | Original turns once toward the retained movement goal, halts/promotes the turn immediately, retains the map goal, and drives the movement-exit animation through ordinary `Execute` even though action state remains `Moving` until that transition completes. | The old-goal turn step precedes halt, the turn sequence is launched eagerly, its requested direction goal is installed, and the retained map goal is restored. Generic animation dispatch admits non-movement walking/running exit transitions while the old moving state is still live. Unit coverage classifies the admitted transition family. |
| Done | Actor idle exit inside movement | Original's bored-to-waiting animation always uses base `PerformAction`, including when the order is stored in a movement element; only particular movement animation arms branch on `IsMovement()` and use `PerformMotion`. | Generic animation execution no longer rejects an order solely because its command/element is movement-shaped. The live animation-arm catalog remains the owner selector, and the universal base-actor idle state effects now apply to PCs as well as NPCs. Regression coverage verifies a PC changes from `Bored` to `Waiting` on bored-exit completion. |
| Done | Patrol `GoToSpeed` close-point gate | At frame 24 Rust changed soldier 92 from patrol-running to waiting and issued `Turn`; Original downgraded it to patrol-walking and retained `MoveOk`. `RHArtificialIntelligence::GoTo` only synthesizes `EventReachPoint` within five units when the current animation is an idle wait. | `go_to_speed`, which represents the same Original overload, now applies the idle-animation, likes-to-sit, and special-action gates before setting `already_on_point`. Regression coverage exercises both ordinary and speed variants, including a nearby running actor that must still queue movement. |
| Done | Re-entrant move after condolation | Soldier 94's Turn completion advances `DefaultGotoRouteTurn` to `DefaultEnroute` and launches the next patrol move during the sequence-manager pass. Original's active `Hourglass` loop instructs that newly registered Move before returning; Rust left it queued and exposed `Wait` for frame 26. | Generic condolation dispatch now promotes and synchronously instructs only the card owner's re-entrant Move before `Ready()` resumes. Other owners retain their FIFO positions, preserving the earlier deferred-patrol ordering fix. |
| Done | Scripted waypoint owner boundary | At frame 27 soldier 104's Turn condolence advances path 13 to scripted waypoint 0. Original synchronously calls `Officier_jaloux__0___8000024c::ReachPoint`, consumes `Rand(2)` (the odd result skips the optional animation), fires `EventAfterScriptGoOn`, and launches the next patrol Move before returning. Rust deferred the waypoint VM until the next frame. | Waypoint callbacks now have an owner-specific drain used inside both condolation paths, followed by same-stack `EventAfterScriptGoOn` and owner Move promotion. The compatibility global drain delegates to the same owner-local implementation without interleaving other NPCs. |
| Done | Deviated arrival ordering | At frame 31 soldier 89 was deviated, blocked, and within ten units of its goal. Rust's pre-movement `IsGoalReached` emulation took the blocked proximity shortcut and snapped to the waypoint. Original first runs `UpdatePositionAntiCollision`, which commits a recovery step and rebuilds the increment; only then does `IsGoalReached` remain false. Soldier90's mismatch was downstream because its later owner slot observed soldier89's incorrect snapped position. | Pre-movement crossed-waypoint retirement is suppressed while deviated, preserving the Original anti-collision-before-arrival order. Replay verifies both soldiers' exact positions and soldier89's rebuilt direction goal. |
| Done | Post-step intermediate arrival | At frame 32 soldier 89 has crossed its non-final waypoint only after committing the anti-collision step. Original runs `IsGoalReached` after `PerformMotion` and retires the waypoint in that owner slot; Rust only had the pre-step arrival check and retained the old patrol goal. | Non-final zero-tolerance movement now performs the post-step arrival check and pops the intermediate order at the same owner boundary. A worktree refactor is generalizing the shared final/tolerance/door arrival tail and its regression coverage. |
| Done | Running patrol member `FaceTo` boundary | At frame 32 soldiers 95 and 97 receive a near-point-backwards `FaceTo` from their chief before their own creation slots. Original `Stop` rewrites the live run into a running-to-waiting transition, lets that slot commit its six-unit frame, then instructs the already-computed Turn in the sequence-manager phase. Eager Rust instruction skipped the transition movement; partially deferring it pre-stamped the Turn and therefore suppressed the live running-to-waiting transition, then recomputed the resolved direction from the order's dummy target. | Standalone Turns produced by cross-owner patrol coordination remain unstamped and untranslated until their ordered `InstructOwner` boundary. That boundary samples the live post-member-slot state, generates the running exit transition, restores the retained movement goal, and appends a Turning order with `compute_direction=false` because `FaceTo` already resolved its direction. Walking/mixed-order patrol cases retain their synchronous behavior. Replay matches the former direction-goal divergence and remains clean through frame 46. |
| Done | Cached movement increments and transition arrival | At frame 39 Rust advanced soldier 96 but left soldier 98 at the end of their run-to-walk transitions; Original did the reverse. The branch was determined by tiny signed dot products. Rust renormalized the remaining goal vector every ordinary frame instead of using `GetIncrementMap`, drifting the chief's historical patrol positions and the later formation targets. Rust also omitted `RHMOTIONMETHOD_TILL_LAST_FRAME`'s per-step arrival snap/zeroing and treated a merely-near destination as exactly equal when initializing the copied continuation. | Ordinary motion now uses the increment cached by order initialization and rebuilt by anti-collision, the 16-sector helper uses Original's literal float table, and non-deflecting anti-collision avoids a subtract/add round trip. Transition motion checks arrival after every committed step, zeros both increment representations, and snaps exactly under Original's deviation/tolerance gates. Fresh non-transition motion only short-circuits on exact point equality. This selects the correct continuation for soldiers 96/98 and retires it at the correct owner boundary, matching through frame 44. |
| Done | Inactive actor Hourglass | At frame 45 inactive soldier 122 changes from `Waiting` to `Bored` in Original while Rust leaves its idle sequence frozen. `RHEngine::Hourglass` calls every element's virtual Hourglass without testing `IsActive`; `RHElementActor::Hourglass` still installs a missing Wait and executes the current order. Plain `WAITING_UPRIGHT` forwards `PerformAction`'s terminal result so Hourglass advances into the authored waiting-to-bored transition; it is not one of the derived endless idle arms. | Inactive actors now receive lazy Wait initialization and generic sprite Execute at their normal creation-order slot, and plain `WaitingUpright` forwards termination into the next order. Regression coverage verifies both activity gates and the terminal idle result. The baseline passes the full transition and its action-state change at frame 45. |
| Done | Route-arrival same-direction `Turn` | At frame 47 soldier 113 reaches path 5 waypoint 1 already facing sector 10. Original's `DefaultGotoRoute` handler directly constructs `RHCOMMAND_TURN`, so it remains in `DefaultGotoRouteTurn`; Rust called `FaceTo`, whose valid waiting/bored same-direction shortcut recursively fired `EventDone` and entered the waypoint macro in the same owner boundary. | The route-arrival handler now launches an unconditional direction-resolved Turn while ordinary `FaceTo` retains its same-direction shortcut. Regression coverage verifies that an already-aligned waiting actor still queues the Turn and does not synthesize a self stimulus. Replay matches through frame 49. |
| Done | Periodic bored-roll animation source | At frame 50 Original soldier 127 runs `RHArtificialMalignity::The16thFrame` while `WAITING_UPRIGHT_BORED` and consumes draw 124 (`294702567 % 12 == 3`, so no remark follows). Rust skipped the draw because the periodic gate read `AiEntityView::current_animation`, which is the previous `ActionChange` value and was still `Invalid`. | The periodic gate now reads `AiContext::self_animation`, populated from live `Sprite::last_action` to match `GetAnimation()`. Regression coverage supplies deliberately stale action-change history and verifies that the live bored animation still consumes the draw. Replay now matches through frame 74. |
| Done | NPC periodic register number | Original staggers NPC periodic work with the persistent NPC-only construction counter `muwRegisterNumber`. Demo civilians 63–80 receive registers 0–17 and soldiers 82–155 receive 18–91. At frame 100 Original therefore runs beggar 64's exact-phase-zero `RandomSpeech` draw before soldier 113's bored `The16thFrame` draw; the old entity-slot surrogate preserved only the soldier's low-four-bit cadence by coincidence. | `NpcData` now serializes the construction register, mission loading assigns it across CIVI then EVIL exactly as the authored demo stream does, and both civilian speech and shared 16th-frame work use it. A TODO retains Original's never-reset global-counter requirement for future in-process mission reloads or dynamic NPC construction. Replay matches through frame 101. |
| Done | Civilian upright-idle override | At frame 75 thirteen civilians reach `RHMOTION_DONE` while their requested waiting-to-bored transition is visually coerced to the bored idle. `RHElementActorCivilian::Execute` returns directly from this family, so `RHElementActor::Execute` never changes their action state. | Rust still coerces the sprite animation, but now skips the base-actor posture/action-state side effects for the five civilian upright-idle arms. PC and soldier behavior remains unchanged. |
| Done | Bored-loop `NewID` timing | Original PC 200 consumes the bored-loop choice at frames 39 and 78. The first nonzero roll keeps both `WAITING_UPRIGHT_BORED` and its order ID; Rust allocated a new ID even when the variant did not change, causing a fresh `RHMOTION_START` tick and delaying the next loop/RNG draw by one frame. | `BoredAnimationChoice` now calls the equivalent of `NewID()` only inside the 1-in-10 bored-to-random mutation branch; random-to-bored still always changes ID. Regression coverage verifies a rejected random variant preserves the order ID. |
| Done | Predetection suspect ordering | At frame 95 soldier 82 receives sharpness 102 from PC 198. Original `HandlePredetection` tests the prior Enemy suspect accumulator (zero), then adds this frame's sharpness; Rust added first and crossed the shadow threshold one frame early. Original also returns for non-PC and guarded-PC targets before changing their shadow latch. | Both detection paths now evaluate the shadow edge against `suspects_before_scan`, preserve the latch on Original's early-return cases, and only then accumulate current sharpness. Focused tests cover the prior-accumulator threshold and latch preservation. Replay matches through frame 95. |
| Done | Synchronous positional `Face` instruction | At frame 96 both engines put soldier 82 in the shadow-reaction substate. Original positional `Face` resolves sector 8 and synchronous Turn instruction writes that direction goal immediately; Rust launched the positional Turn after the global turn phase and left goal 9 until the next frame. | Contextual positional `Face` now resolves to an authored sector before launch and retains Original's waiting/bored same-direction shortcut. The non-movement Turn drain also synchronously computes and installs goals for no-context positional intents while retaining the separate deferred formation-turn rule. Focused coverage verifies the resolved directional intent; replay matches through frame 100. |
| Done | Synchronous script `LockAI` barrier | At frame 102 soldier 104's waypoint `ReachPoint` callback calls `LockAI`, then immediately records a positional Turn toward the officer. Original `ScriptLockAI` stops the outgoing actor work before returning to the script, so the replacement Turn survives with goal 11. Rust queued a deferred AI halt, resumed the script, launched the Turn, and then halted that new work, leaving `Wait` and goal 1. | The native now yields to an engine barrier that applies the lock, performs Original's normal-priority stop and synchronous condolations, then resumes the VM. The `RHCOMMAND_LOCK_AI` self-stop exception is retained. Native regression coverage verifies the yield precedes replacement script work. |
| Done | Live actor animation identity | At frame 102 Original soldiers 83 and 147 both consume the `The16thFrame` bored roll. Soldier 83 completed its waiting-to-bored transition during actor execution, so `GetAnimation()` already reads the promoted `WaitingUprightBored` order even though the sprite still displays transition row 62. Rust built `AiContext::self_animation` from `Sprite::last_action` and skipped soldier 83's draw. | The periodic owner boundary now samples the live current sequence order, falling back to `NonanimationEnd` like a null C++ `mpOrder`, without overwriting legitimate displayed-sprite state. The generic context documents its sprite-action fallback and retains a TODO to thread live-order identity through the other `GetAnimation()` consumers. Replay matches through frame 110. |
| Done | One-point macro recursive reach | At frame 111 soldier 83's `DefaultDetectedCharly` timer resumes an interrupted one-point macro with no bytes remaining. Original clears the macro, changes to `DefaultEnroute`, and calls `Think(EventReachPoint)` synchronously; selecting the same waypoint's next macro section immediately draws `(rand() % 100) + 1`. Rust queued the prior `SetState(DefaultInMacro)` notification, completed the macro tail into `DefaultEnroute`, then drained the unavailable script callback by rewinding and recommitting the stale incoming substate. The recursive reach consequently observed `DefaultInMacro` and was ignored. | State-change notifications with no callable `FilterAIEvent` are now consumed without changing canonical AI state, preserving state mutations performed later in the same pure-Rust handler. Focused coverage verifies an unavailable callback cannot rewind a later direct handler-tail state. Replay consumes the missing draw and matches through frame 113. |
| Done | Non-movement exit-transition state | At frame 114 PC 198's non-movement `Wait` sequence has advanced from `TransitionWalkingUprightWaitingUpright` to `WaitingUpright`; its sprite legitimately still displays the transition. Original's base `RHElementActor::Execute` applies `(Upright, Waiting)` when that transition finishes, while Rust only implemented the completion state for soldiers and movement elements, leaving the PC `Moving`. | The universal actor animation side effects now apply Original's complete non-movement exit family: walking/running upright to upright waiting and walking crouched to crouched waiting on `Done` or `Terminated`. Focused PC coverage exercises all three arms. Replay matches through frame 118. |
| Done | Civilian coerced-idle RNG | During original frame 119 Rust consumed thirteen extra `BoredAnimationChoice` draws before the two correct `VipIdleRemark` draws. They belonged to civilians 63, 64, 66, 68, and 72–80, whose coerced bored sprite loops terminated together. Original `RHElementActorCivilian::Execute` returns directly after `PerformAction(..., WAITING_UPRIGHT_BORED)` for the whole upright-idle family, bypassing base actor's `rand() % 10` bored-variant selection and base actor's forced `InProgress` return. | Civilian bored-loop completion now forwards the raw sprite motion without entering the base actor random-choice arm, so `Terminated` can advance through `Hourglass` exactly as in the override. A deterministic zero-roll regression verifies the forwarded result and that the civilian neither mutates its order type nor allocates a new order ID. The two legitimate periodic draws for soldiers 132 and 148 remain in creation order; replay matches through frame 122. |
| Done | Late-frame AI move promotion and timer facing | At frame 123 soldier 82 finishes an old-path step, then its normal timer reconsiders the enemy approach. Original `Focus(primary_target)` changes only the view cone; Rust had a synthetic timer pre-dispatch snap that reset body direction 5 to sector 8. Original also registers the replacement `GoNear` before the after-entity sequence-manager pass, which instructs it as `MoveWaiting` after that frame's path phase; Rust left the intent pending until the next tick. | Removed the non-Original timer-facing snap. AI Move intents emitted by entity/NPC work are now promoted immediately before the end-of-frame sequence-manager pass, so they become `MoveWaiting` in the issuing frame and resolve at the following path barrier. Focused timer coverage verifies an alerted primary target does not turn the actor. Replay matches through frame 155. |
| Done | Idle Wait alongside future script work | After soldier 104's scripted Turn ends, an ownerless Timer keeps later owned script elements at future levels. Original `RHElementActor::Hourglass` installs its low-priority Wait whenever the actor has no current order, so its waiting-to-bored transition completes at frame 156. Rust treated any future `Todo`/`Postponed` owner element as active work and left the old Turning sprite frozen. | `ensure_wait_element` now suppresses the idle only for a current `InProgress` owner element. Regression coverage verifies a current Wait can coexist with future owned work behind an ownerless timer. Replay matches through frame 168. |
| Done | Speed-factor/turn-minimum order | Soldier 95 receives the same formation goal in both engines, but at frame 161 Original advances exactly `0.7` units while Rust advances about `0.6986`. The tiny drift eventually changes which anti-collision side is selected at frame 169. `RHSprite::PerformMotion` multiplies the raw frame distance by the sequence speed factor before applying the `0.6` turn slowdown and `0.7` minimum; Rust scaled the already-clamped result. | Movement now applies the formation factor before the turning slowdown/minimum, while transition motion retains its implicit factor of one. Focused coverage reproduces soldier 95's raw distance and factor and verifies the exact `0.7` result. Replay matches through frame 198. |
| Done | Custom animation wrapper semantics | Soldier 104's script requests `PlayAnim(Pointing)`. Original stores logical order `RHNONANIMATION_PLAY_CUSTOM` and uses Pointing only for `PerformAction`, so the normal Pointing completion side effect never changes `Bored` to `Waiting`. Rust stored Pointing as the logical order and leaked its NPC side effect at frame 199. | Actor `PlayAnim*` translation now retains the four `PlayCustom*` wrapper order types and reads `AnimationId` only for sprite playback. Requested custom visuals no longer run their ordinary actor/NPC/soldier gameplay side effects. Focused coverage verifies custom Pointing retains the actor's Bored state. Replay matches through frame 282. |
| Done | Swordfight approach distance | At frame 283 Original begins swordfight because `ReconsiderEnemyApproach` computes the raw map-coordinate norm between soldier 82 and PC 198 and truncates it through `UWORD`: about 72.4 becomes 72, within the 75-unit threshold. Rust incorrectly applied the general isometric Y stretch and obtained about 77.0. | This specific approach routine now uses raw X/Y Euclidean distance with Original's `UWORD` truncation, including friend-target reassignment. Focused coverage reproduces the boundary where the general aspect-corrected helper would reject combat. Replay installs the matching combat commands and substate at frame 283. |
| Done | Event-driven enemy strike proposal | Once Rust entered the matching swordfight substate at frame 283, its global melee scan immediately called `ProposeGoodSwordStrike` and consumed RNG. Original calls that routine only from the tail of event-driven `ReconsiderSwordfight`; its first legitimate call in this fight is the frame-293 reach-point cascade. | `ReconsiderSwordfight` now hands the engine a serialized one-shot strike-consideration latch. The melee pass consumes it even when a later range, honour, readiness, or selection check rejects the proposal; merely occupying `AttackingSwordfight` no longer retries every frame. Focused tests cover entry without authorization and one-shot rejection. |
| Done | Sober combat RNG gates | Original evaluates `rand() % 100 <= bloodAlcohol || rand() % 100 <= bloodAlcohol` even when blood alcohol is zero. Rust skipped both draws for sober actors, which would desynchronize the global stream at the first legitimate combat reconsideration. | The two gates now always execute with literal short-circuit order. Focused RNG-trace coverage verifies two nonzero draws for a sober soldier and the rare first-zero one-draw freeze. |
| Done | `EnterSwordfight` instruction/execute boundary | At frame 283 Original installs command 50 while PC 198 remains `Bored`, soldier 82 remains `MovingFast`, and both retain their old directions. Rust eagerly changed both to `WaitingSword` and updated facing. Original establishes opponent relationships during instruction, queues the raising-sword order behind authored exit work, and changes action state/facing only when that order executes in a later owner slot; the Soldier override delays `WaitingSword` until the raising animation is done. | AI engagement now queues the normal `EnterSwordfight` element, opponent-list setup is free of animation/state mutations, and raising-sword state/facing effects occur during actor execution. Focused instruction and animation tests preserve the Human/PC versus Soldier timing asymmetry. |
| Done | Sword movement logical dispatch tokens | Soldier 82's short approach begins at frame 292. Original rewrites `RUNNING_UPRIGHT` to logical `RHNONANIMATION_RUNNING_WITH_SWORD`; the Human Execute override then calls `FaceOpponent` to choose a concrete sword animation and uses `RHMOTIONMETHOD_FAST`. Rust incorrectly gated that rewrite on `Sprite::has_animation(RunningWithSword)`, although the value is deliberately not a sprite row, and retained ordinary running. | Upright walking/running actions now become the logical sword movement tokens unconditionally in sword context, exactly as `RHElementActorHuman::DetermineMovementAnimation` does. Concrete sprite availability remains the later `FaceOpponent` concern. Focused coverage verifies both dispatch rewrites and distance-motion classification; replay matches the start cadence and RNG boundary through frame 292. |
| Done | Event-owned strike RNG ordering | At frame 293 Original's soldier 82 executes `ProposeGoodSwordStrike` directly inside `ReconsiderSwordfight`, consuming its rejecting roll before the later soldier 104 waypoint script calls `Rand(2)`. Rust deferred the one-shot strike authorization to the global melee pass, swapping those draws: soldier 82 incorrectly launched a preparation `WaitTimer`, and soldier 104 skipped its authored `Turn`. | The canonical filtered-Think boundary now consumes that owner's pending strike consideration immediately after the AI handler releases its borrow. Owner-local consumption skips unrelated once-per-frame special-strike reconciliation, which remains in global melee maintenance. Focused coverage verifies a rejecting owner-scoped strike draw remains ahead of a later script draw; replay matches both actors and passes through frame 310. |
| Done | Successful strike inline state visibility | Original's successful `ProposeGoodSwordStrike` changes to the special-strike substate inside `ReconsiderSwordfight`; the immediately following substate test therefore suppresses `CombatInsult`. A rejected proposal remains in ordinary swordfight and insults. Rust consumed the proposal just after the pure AI handler and could not make that state visible to its remaining tail. | The AI records a serialized deferred-insult latch alongside the owner-local strike authorization. The engine settles both at the same owner boundary, emitting the insult only for rejection and suppressing it after a successful special strike. Focused accepted/rejected coverage and the owner RNG-order regression pass. |
| Done | Postponed seek release in manager drain | PC 198's frame-296 sword strike creates a dynamic-target Seek with a post-seek thrust. Its normal priority postpones it behind the high-priority `EnterSwordfight`. When sword raising ends at frame 311, Original's synchronous `SendCondolationCard` / `Ready` stack re-registers the Seek before the later `SequenceManager::Hourglass`, which instructs it that frame and arms wait 25; path installation still waits for frame 312's earlier path phase. Rust drained the terminal card just after the manager pass and stranded the restored Seek in `elements_to_go` for a frame. | The sequence phase now closes terminal cards produced by the preceding entity phase before collecting manager work. A focused cross-sequence postponement regression verifies the successor reaches `InProgress` in that drain. The queued path request retains the next-frame barrier. Replay matches `MoveOk`/25 at frame 311 and the installed path at frame 312. |
| Done | Isomorphic overloaded actor wait field | Original reuses `RHElementActor::mulWaitTime` for ordinary waits and seek refresh, while Rust deliberately stores seek refresh separately. Literal storage comparison reported zero even though the authoritative Rust seek countdown was 25/24. | `actor_legacy_wait_time` exposes the currently owned legacy scalar to parity/debug consumers. Seek and ordinary-wait initialization clear the competing split domain, and the post-seek interaction retains the last seek countdown exactly as Original does. |
| Done | Entity-target `PerformSeek` start wrapper | At frame 312 Original begins the sword-running sprite while retaining `WaitingSword`. Entity-target `PerformSeek` wraps the sprite's `START` return as `IN_PROGRESS`, so the Human Execute switch does not enter its moving-sword state arm. Rust applied the raw sprite result and changed to `MovingFastSword`. | Movement state effects now observe the wrapper-visible result for entity-target walking/sword movement; point seeks and ordinary movement retain raw `START`, and upright running retains its Original unconditional `MovingFast` behavior. Focused coverage verifies the sword case. |
| Done | Entity-seek tolerance sample phase | PC 198 begins frame 314 about 60.397 units from soldier 82, outside thrust D's effective 56.7-unit arrival radius. Its six-unit step crosses into range. Original retains the pre-motion false tolerance result for that whole `PerformSeek`, remains `MoveOk`/22, and launches the thrust before movement on frame 315. Rust recomputed tolerance after committing the step and launched one frame early. | Entity-target tolerance is sampled once before movement. Starting in range takes the frozen/post-seek arm without a step; crossing into range is observed on the next actor tick. The creation-order tolerance regression now covers both an already-in-range target and a step that crosses into range. |
| Done | Frame-315 post-seek cleanup | Original's synchronous seek `SendCondolationCard` clears the movement goal to zero before launching thrust D and leaves the overloaded seek countdown at 22. Rust launched the correct thrust but retained the old target goal, while the parity mapper stopped exposing the split countdown after `active_movement` cleared. | The post-seek handoff now clears the selected movement goal before launch, with focused coverage, and the isomorphic legacy-wait view retains the seek-domain value across the interaction. Entity-target `PerformSeek` owns the countdown decrement before transition and zero-motion exits, while its successful pre-motion post-seek return preserves the counter. The baseline now passes through frame 325. |
| Done | Frame-326 second strike handoff | Original begins `SwordstrikeThrustE` and has already cleared its selected movement goal, while Rust remains in `MoveOk` toward the target. The renewed seek begins inside the effective tolerance on a zero-distance animation tick. Original tests tolerance before dispatching sprite motion; Rust returned on zero speed before its already-computed arrival could launch the post-seek sequence. | Pre-motion entity-seek arrival now bypasses both transition execution and the zero-speed return so it reaches the shared frozen/post-seek tail. Non-arriving zero-distance and transition ticks retain their existing behavior. The baseline advances to frame 334. |
| Done | Frame-334 sword damage RNG | Rust consumes two `SwordDamageProtection` draws during Original frame 334, while the recorded global stream contains one additional draw before the next frame boundary. | The third draw is the post-damage Provoke chance. Original evaluates it before selected-PC suppression, so a controlled attacker owns the draw even though it cannot taunt. Rust now rolls after nonzero, non-parried sword damage and only then applies selected-PC suppression; the provoke roll also precedes hero-speech evaluation as in Original. |
| Done | Frame-336 sword hit reaction state | Soldier 82 executes `ReceiveSwordDamage` in both engines, but Original has entered `WaitingSword` with hit-reaction animation 102 while Rust retains `MovingFastSword`/`WalkingSword`. Rust selected the correct `BeingHitSword` injury order but its generic driver refused to execute while the actor retained a stale movement state. | Selected non-movement injury/lethal commands now supersede ordinary, sword, and shield movement states, allowing the existing `BeingHitSword` START handler to enter `WaitingSword`. Normal waits remain movement-owned. |
| Done | Frame-340 sword initiative and speech RNG | Rust initially consumed only Soldier 97's periodic idle draw; Original then consumed PC 198's `EvaluateSwordfight` left/right choice and launched a smalltalk strike. Rust transferred initiative from the raw sprite `START` hidden by entity-target `PerformSeek`, and later fired the smalltalk hero-speech chance as soon as the new order was selected. | Sword movement initiative now observes the wrapper-visible Execute result, so a hidden raw `START` does not transfer it. PC combat speech now waits for the selected order's actual sprite `START`, matching Original's Execute macro rather than treating manager-tail selection as execution. The global stream consumes the idle and smalltalk-side draws on frame 340; speech remains deferred. |
| Done | RNG-mismatch engine dumps | `--dump-jsonl` previously panicked on an RNG cursor mismatch before writing the divergent frame. | The runner now compares and writes the complete frame snapshot first, then reports the RNG mismatch, making generic engine state available without adding one-off logging. |
| Done | Frame-344 soldier special-strike substate | Original Soldier 82 enters observable legacy substate 161 (`AttackingSwordfightSpecialStrike`) after a successful event-owned proposal. That state has distinct done/timer exits, accepts incoming sword-strike warnings, and gates good/lethal-strike speech. | Rust now retains substate 161 for the sequence lifetime instead of folding it into ordinary swordfight plus a boolean. The latch remains only for cancellation recovery, explicit completion returns to substate 160 with the 20-frame heartbeat, incoming strikes admit 161, and a newer reaction state is never overwritten by stale reconciliation. Focused coverage exercises completion and reaction-state preservation. |
| Done | Frame-354 reactive-parry stop boundary | Original handles PC 198's strike-start warning synchronously while Soldier 82 is preparing a special strike: it calls `StopAll`, chooses `ParrySword`, enters parade substate 162, and manager-tail dispatch installs the parry that frame. Rust initially deferred the halt until after launching the replacement parry, so the stale halt interrupted it; once the parry survived, special-strike cancellation reconciliation incorrectly replaced parade with ordinary swordfight. | Added a narrow synchronous pending-AI-halt barrier for stop-then-replace routines and use it before launching the parry. Special-strike reconciliation now clears stale lifecycle bookkeeping without overwriting a newer combat reaction. The baseline matches both command and substate through frame 354. |
| Done | Frame-355 retained lateral sweep | Original stores a lateral/circle strike's victim list and angles on the human rather than the interrupted sequence. PC 198's completed D action point is interrupted, but the shared sweep survives; replacement strike E's `START` leaves it dormant and its first `IN_PROGRESS` advances the retained geometry using E's current direction, rotation, profile, and damage, hitting Soldier 82. | Generic actor-stop cleanup now preserves human-owned sweep state. Sweep driving distinguishes dormant/start/in-progress/initialized phases and rebinds retained geometry to the currently executing sweep strike before advancing it. Focused coverage interrupts D, verifies E's start is inert, and proves E's first in-progress arc hits only the victim selected by E's semantics. |
| Done | Frame-362 target death and swordfight exit | Original accepts PC 198's new seek/strike command while Soldier 82 is alive and synchronously resolves its path to `MoveOk`. The old strike kills Soldier 82 later in the same frame; relationship cleanup does not replace that accepted movement. On the next actor tick the non-forced sword movement observes its empty opponent list, aborts before motion, launches explicit `QuitSwordfight`, and then begins lowering on the following actor slot. | Relationship cleanup no longer mutates action state or synthesizes a lowering order. Orphaned non-forced sword movement performs the explicit quit lifecycle, while forced movement remains valid. Lowering `START` restores upright/waiting state, and stale `Done` results cannot complete a newly installed replacement order because completion propagation verifies order identity. Focused coverage exercises death cleanup, explicit quit, forced/non-forced orphan movement, lowering start, and replacement-order completion identity. |
| Done | Resolved `box_select` trace command | The Original recorder emits a root `box_select` gesture and then its nested resolved `unselect_all_pcs` and ordered `select_pc` messages. Replaying the geometry and nested messages would duplicate selection state, echo, and speech effects. | The runner accepts the rectangle/modifier record as gesture metadata and applies only the following resolved selection messages through normal engine commands. This preserves PC-array ordering, selectability checks, and isomorphic entity translation without replaying raw mouse input. |
| Done | Frame-384 lethal-reaction completion | Original's lethal `ReceiveSwordDamage` element owns only its dying order. When that order terminates, the exhausted damage element ends and exposes the normal no-order `Wait` fallback for one frame; the following actor Hourglass launches a real `Wait`, whose dead-posture translation installs `BeingDeadSword`. Rust had permanently attached the corpse hold to the damage element. | Death handling now queues only an actual dying animation, or no order for an already-ground victim. The ordinary idle lifecycle owns the subsequent corpse hold, so logical command identity and the deliberate one-frame no-order boundary match for all lethal damage types. |
| Done | Frame-401 immediate scripted `LockAI` | Mission script message 1004 launches `LockAI → PlayAnim(RaisingShield) → UnlockAI` for Soldier 130 before its actor slot. Original `ScriptLockAI` synchronously stops the actor's previously selected animation sequence before terminating the zero-frame lock; Rust suppressed that stop, allowing the old trailing `UnlockAI` to emit `ReturnToDuty` and replace the new animation with `MoveOk`. | Owner-immediate `LockAI` now applies lock/macro teardown, synchronously stops the previous normal-priority owner work and closes its condolations, and only then terminates the lock to register its successor. This mirrors the existing native-script lock path and keeps replacement animation ordering general. |
| Done | Frame-404 per-order anti-collision target and force radius | PC 198's group-move order passes close to dead Soldier 82. Original copies the current motion order's null antagonist into `PositionInterface`, so the corpse repels the PC; Rust instead used the stale prior seek target and excluded it. Rust also passed only the input falloff radius to deviation math, while Original passes `mfActionRadius` after `SetForce` has added the inner radius. | Anti-collision target filtering now reads the active movement order's antagonist without conflating it with persistent seek bookkeeping. Point and line deviation receive the stored total action radius. Focused coverage proves an intermediate targetless order sees a corpse, an antagonist-bearing final order excludes it, and the expanded force threshold drives the exact deviation. |
| Done | Frame-410 return-to-post live animation and initial facing | After Soldier 130's scripted raising-shield sequence unlocks AI, Original sees no live order (`RHNONANIMATION_END`), takes `GoTo`'s already-at-post path, and turns from body sector 2 toward stored view sector 1. Rust built the recursive self-Think context from stale `sprite.last_action=RaisingShield`, queued a zero-distance `MoveOk`, and cached the mission-start body sector directly. Once the live-order fix exposed the turn path, that raw cache falsely produced same-direction `Done` and an extra bored-time RNG draw. | Re-entrant self-stimuli now rebuild `GetAnimation` from the current sequence order on every recursive dispatch, defaulting to `NonanimationEnd`. Mission-start view facing now mirrors `StoreInitialPositionParameters`: create the initial vector at aspect 1, then cache the sector produced by the later `FaceTo(vector)` aspect-ratio binning (diagonal body sector 2 becomes view sector 1; cardinal directions remain stable). Dynamic post/path assignments retain their separately authored aspect-aware behavior. Focused regressions cover both the no-order/stale-sprite boundary and the initial direction conversion. |
| Done | Frame-459 interrupted movement goal cleanup | Five soldiers enter shadow-response Turns after `StopAll` interrupts their selected `MoveOk` elements. Original actor-base `SendCondolationCard` clears the selected element's movement goal and detaches its order before the NPC override returns early for `mbInsideHaltMethod`; Rust's halt-tagged condolence skipped that actor-base cleanup and retained the destinations. The shadow handler then performs a second, command-internal halt for `FaceTo`; unlike an ordinary standalone Face, this double-halt path must not restore the stopped destination. | Selected movement condolations now clear `active_movement` and `position_goal_map` before all NPC early-return gates. Identity is matched exactly, so unrelated/postponed cards cannot erase a replacement goal; ordinary exhausted-order completion retains its existing synchronous cleanup. The order drain carries prior explicit-`StopAll` provenance into Face, suppressing goal retention only for that same double-halt boundary. Replacement movement such as frame 123 still installs and retains its newly authored destination. Focused coverage exercises selected/unrelated cards and explicit StopAll-before-Face. |
| Done | Frame-462 shadow-response Turn state | Soldiers 92 and 93 run the same walking-to-waiting transition for their shadow-response Turn, but Rust initially changed to waiting one frame early. Soldier 91's detection-tail shadow event synchronously fans out to the patrol before 92/93's creation slots. Original registers their Turns immediately but does not instruct them until the later sequence-manager pass, so their upcoming actor slots execute the already selected halt transition. Rust lost the deferred-instruction mode at the nested cross-NPC `SendStimulus` boundary and selected the new Turn transition in those slots. | Deferred standalone Turn scheduling now applies to ordinary walking as well as running and is threaded through depth-first synchronous patrol-stimulus recursion, including fallback delivery. Direct/global callers retain their existing immediate mode. The member slots consume the old halt transition; manager-tail instruction installs the Turn afterward, so its transition reaches `Done` and changes action state on the same frame as Original. |
| Done | Frame-469 shadow-response direction goals | When the shadow timer expires, patrol minions return to their nearby chief. Their synchronous `DefaultGotoChief` reach-point handler faces the chief's live position, producing member-specific goals 13 and 14. Rust's general per-tick AI context left `patrol_chief_position` at the stub origin outside the dedicated coordinate call, so both members faced sector 15. The missing data was especially easy to hit when `primary_target == 0`, because the builder returned early. | The central enemy tick-data builder now snapshots a referenced patrol chief's live position and AI state before the no-primary-target return. Missing/stale chief references fail loudly rather than supplying fake data. A focused regression verifies position, sector, level, and state without a combat target. |
| Done | Frame-472 patrol replacement manager boundary | Chief 91's eighth-frame patrol refresh synchronously coordinates later-slot member 93. Original `CoordinatePatrol` interrupts the selected return-to-chief transition, whose actor-base halt condolence clears the old goal, then only registers the replacement Move for the after-entity `SequenceManager::Hourglass`; the member cannot execute it in its frame-472 slot. Rust's halt-tagged condolence correctly suppressed `Think` but still ran the generic post-card continuation drain, which stole and synchronously instructed the replacement Move. Its generated waiting-to-walking transition therefore executed one slot early. | Halt-tagged cards still perform unconditional actor/human cleanup, but no longer drain self-stimuli, waypoint scripts, or replacement Moves: the NPC condolence override returned before `Think`, so none can be causally produced by that card. The caller's replacement remains queued for normal owner/manager promotion. Focused coverage verifies a prequeued Move survives a halt card uninstructed. Replay now matches through frame 475. |
| Partially superseded | Frame-476 patrol movement completion boundary | Soldiers 89 and 93 advanced from their first formation waypoint in Rust while the schema-3 x87 Original consumed a copied endpoint continuation. Part of the drift came from x87 retaining the normalization norm; anti-collision also incorrectly recomputed the norm of the rounded `increment * distance` vector instead of forwarding the authoritative animation distance. | The x87 normalization emulation has been removed: schema-5 Original and Rust both perform the source operations in scalar binary32. The independent anti-collision distance fix remains valid and retained. Exact-bit normalization coverage now asserts the scalar-SSE result. |
| Done | Frame-502 same-direction chief facing | Soldier 89's shadow timer returns it to duty while it is already within the chief's talk radius. The synchronous `EventReachPoint` handler faces the chief; the resulting sector is the soldier's current sector 12. Original `FaceTo` detects the waiting, already-facing actor, sets `mbAlreadyTurned`, and returns without launching a Turn. Rust used a context-free facing helper that could not perform this check and queued a redundant Turn. | `DefaultGotoChief` now uses the context-aware facing path, preserving Original's waiting/bored same-direction shortcut and same-stack `EventDone`. Focused coverage exercises the close-chief, already-facing reach event. Replay matches through frame 625. |
| Done | Shadow music-only alert flag | Original's shadow response calls `SetAlertStatus(ALERT_YELLOW, ALERT_ONLY_MUSIC)`: suspicious activity raises the soundtrack alert without changing the NPC's visual alert level. Rust raised both channels, then incorrectly used the music channel for `ComputeVisibility`'s refresh-always gate, recomputing vision every frame instead of retaining the visual-Green cadence. | `EventSeesShadowStandardProcedure` routes Yellow through the existing `ONLY_MUSIC` flag-aware setter, and detection reads `view_alert_status`. Focused coverage verifies the channel split and cadence gate. Correcting this exposed an independent cached-maximum bug that the extra refreshes had masked. |
| Done | Frame-469 cached maximal sharpness | On a closed two-frame PC cadence, Original `ComputeVisibility(RHDetectable, ...)` returns the detectable's cached visibility, converts it to integer sharpness, and folds that into `muwMaximalVisibility`. Rust instead folded the pre-cache raw visibility, which is deliberately zero on a closed frame. Soldier 91's shadow timer therefore saw zero at frame 469 and returned to its route, while Original saw sharpness 5 and kept looking. The wrong music-driven refresh cadence had previously hidden this bug by opening that frame's gate. | The maximum now has Original's integer-sharpness representation, uses post-cache sharpness, and spans Enemy, Body, Object, Friend, MissedFriend, and Beggar scans. Regression coverage proves cached visibility contributes on a closed cadence frame. |
| Done | Frame-626 patrol detection threshold | With the visual alert channel and cached maximum corrected, Soldier 91's Enemy suspect accumulator follows Original's normal two-frame cadence: 981 at frame 625, 998 at frame 626, and 1015 at frame 627. Rust had crossed the threshold one frame early and synchronously fanned `EVENT_VIEW` across soldiers 89–93. | The two general detection fixes above align the threshold and patrol-wide transition without any actor, frame, or replay-specific condition. Replay matches through frame 636. |
| Done | Frame-637 observe threshold | With one visible enemy and courage 45, Original compares three nearer friends against the floating threshold `1 + 1 * (0.045f * 45) = 3.025`, so Soldier 92 fights. Rust truncated the courage bonus to an integer before comparing and incorrectly allowed the soldier to observe. | The Lacklandist observe decision preserves Original's `f32` operation grouping through the comparison. Focused coverage proves three friends are insufficient while four are sufficient at this fractional boundary. |
| Done | Frame-638 synchronous focus ordering | `BattleDecisions` first calls `Focus(NULL)`, then the Observe branch calls `Focus(primary_target)`. Original applies both immediately and ends in Follow mode with its extended view cone. Rust's deferred outbox retained both independent channels and applied unfocus last, collapsing Soldier 90's cone and spuriously launching the lost-enemy search with 18 RNG draws. | AI focus, focus-point, and unfocus writes now share a last-write-wins API mirroring synchronous call order. All call sites use it, and focused coverage exercises all three overwrite directions. Replay matches through frame 641. |
| Done | Frame-642 fast reaction turn | Original's standard-distance `EventViewStandardProcedure` calls `Face(enemy, true)`. Soldier 93's turn waits behind an attentive-mode transition and becomes selected at frame 642 as `TurnFast`; Rust discarded the fast argument and eventually selected ordinary `Turn`. | Fast facing is represented on the generic AI turn intent and preserved through both immediate and deferred sequence launches. Focused tests cover intent geometry and a deferred fast turn surviving instruction with its direction and retained movement goal intact. Replay matches through frame 647. |
| Done | Frame-648 movement-replacement goal lifetime | Soldier 92 replaces a live `MoveOk` while its startup transition owns goal `(1022.93677, 1793.7484)`. Original's `Halt` preserves movement, selects the new A*-pending movement, then interrupts the old element; its condolence is no longer for the selected element and cannot clear the cached goal. Rust halted and cleared the old movement before registering its replacement. | AI movement intents carry the selected goal into their replacement element. A `MoveWaiting` replacement retains it until a concrete path order initializes the sprite, and old-movement condolence cleanup honors that replacement ownership. Explicit `StopAll` and genuine exhaustion keep their zero-goal cleanup. Focused lifecycle coverage exercises the handoff. Replay matches through frame 648. |
| Done | Frame-649 attentive movement animation family | Soldier 90's authored orders remain ordinary Waiting→Walking and Walking actions, but Original `RHElementActorSoldier::Execute` plays their alerted variants while `mbAttentive` is set. Rust used the ordinary transition, which terminated one tick earlier, then ordinary walking with a shorter per-frame distance. | Movement now resolves the concrete soldier sprite animation from the authored order, attentive flag, and branch-specific sword-state guards without rewriting the order itself. The six upright movement transitions, ordinary walking, stairs, and turning follow the Original substitution table; lift translation sees the resolved upright animation. Focused coverage exercises every mapping and the distinct sword guards. Replay matches through frame 649. |
| Done | Frame-650 sprite-neutral pathfinding freeze | During Soldier 92's replacement `MoveWaiting`, Original `RHNONANIMATION_FREEZING` returns `InProgress` without touching the sprite. Rust's generic animation path replaced the previous alerted transition with frozen `WaitingUpright`. When the replacement resumed the same transition, Original preserved its frame phase and moved four units while Rust restarted at frame zero and moved two. | `Freezing` now short-circuits before every sprite selection, stamp, or increment for every posture; `PlayCustomFrozen` remains a distinct animated command. The regression seeds a nontrivial transition row/frame/counter, runs both movement and generic actor slots for `MoveWaiting/Freezing`, and proves the complete animation phase remains unchanged. Replay matches through frame 687. |
| Done | Frame-688 raise-sword-only swordfight entry | `AttackingApproachToObserve` directly sets the direction goal, stops current work, then launches `EnterSwordfight` with an explicit null opponent to raise the sword without engaging. Rust queued a standalone Turn, emitted a bare element, and rejected it because eager validity required a concrete opponent. After admission was fixed, Rust also turned during `TransitionRaisingSword`, although Original only calls `Turn()` there when the order has an antagonist. | The AI outbox has an ordered direct-direction-goal write applied before StopAll, so this path launches no Turn element. Raise-sword encodes `Opponent=Integer(0)` and a null jump line; validity accepts exactly that explicit legacy null while still rejecting missing/mistyped fields. Null-antagonist raising-sword animations no longer rotate the body. Focused validity coverage preserves the malformed-versus-null distinction. Replay matches through frame 693. |
| Done | Frame-694 deferred Point facing | When `GatherSoldiers` hands off to `Point`, Original translation only books the `Pointing` order. The direction property is read on the first actual Execute tick, installed as the progressive goal, and `Turn()` advances one sector before sprite playback. Rust applied the property instantly during translation, changing both direction and goal on the handoff frame. | Point translation is now sprite/direction neutral. Pointing initialization requires and installs its integer direction as the goal, then the existing per-tick turn path advances toward it. The trace progression matches `8/8 → 9/12 → 10/12 → 11/12 → 12/12`; point-related tests pass. Replay matches through frame 726. |
| Done | Frame-727 stationary sword wait after movement | Soldier 92's selected `WaitTimer` owns `WaitingSword`, but its action-state enum still names the just-finished fast sword movement. Original dispatches the selected non-movement order immediately, decrements the timer, and runs `EvaluateSwordfight`; Rust's generic animation gate treated the stale action state as authoritative and skipped the whole owner slot, losing the non-mutual-opponent RNG draw. | Selected `WaitTimer` and `WaitFreeLift` orders now supersede stale ordinary/sword/shield movement action states. The first stationary animation, swordfight evaluation, and countdown tick all run in the selection frame. The base bored-animation completion guard also follows Original's exact `command != WaitTimer` rule instead of limiting rerolls to plain `Wait`. Replay matches through frame 734. |
| Done | Frame-735 swordfight initiative and idle fallback | Original `IsSwordfighting()` means only that the opponent list is non-empty. Each fresh reciprocal `AddOpponent` insertion recomputes strength and conditionally takes initiative in call order; re-adding an established pair does nothing. Rust gated initiative on sword action states and then reset it unconditionally after every enter call, so Soldier 89 lacked initiative. Independently, its completed raise-sword element left Rust owner-empty until the next frame, while Original installed fallback `Wait` in the same `Actor::Hourglass` slot. Together these skipped Soldier 89's smalltalk-side draw and strike. | Initiative now uses relationship state, runs after each fresh reciprocal insertion, and is untouched on re-entry. After any terminal actor Execute settles its callbacks, Rust installs fallback `Wait` immediately if no real successor exists; timer/lift waits are no longer a special case. Soldier 89 launches the same left smalltalk strike, and replay matches through frame 736. |
| Done | Frame-737 observing-combat side-step coin flip | Original's observer reposition gate is the literal predicate `rand() % 2 == 0`. Rust used the generic boolean RNG helper, whose true half is the odd residue, so Soldier 90 made the opposite side-step decision despite consuming the correct draw. | The reviewed call site now preserves the explicit even-residue predicate while retaining the global draw stream and general combat-position logic. Replay matches through frame 746. |
| Done | Frame-747 deferred sword-damage instruction | Soldier 92's strike reaches its victim before PC 198's later creation slot. Original `LaunchSequenceElement` only registers `ReceiveSwordDamage`; PC 198 therefore performs one final six-unit seek tick and decrements its wait timer before `RHSequenceManager::Hourglass` instructs the damage at the end of the frame. Rust synchronously arbitrated and dispatched the damage inside Soldier 92's actor slot, suppressing that victim tick and starting the hit reaction one frame early. | Sword strikes and collision pushes now queue real damage elements on the ordinary sequence-manager FIFO. The manager-tail `InstructOwner` boundary stamps the victim's then-current state, arbitrates the injury, and dispatches damage after all actor slots. This is the shared damage lifecycle, with no actor or replay special case. Replay matches through frame 750. |
| Done | Frame-751 live recovery-animation honor gate | Soldier 89 reconsiders its fight after the expected two drunk-combat draws and one reposition draw. Original reads PC 198's live `BeingHitSword` animation through `GetAnimation()` and returns before strike selection. Rust read `ActorData::old_action`, which is only ActionChange history and remained `Invalid`, so it consumed an extra `SwordStrikeSelection` draw. | A shared live-animation accessor reads the selected sequence order with a sprite-animation fallback. Sword-recovery honor snapshots, direct attack admission, and opponent strike timing use it while retaining the separate logical sword-action-state gate. Focused coverage proves visible recovery rejects the reconsideration even with stale `old_action`. Replay matches through frame 755. |
| Done | Frame-756 replacement input inside a postponed chain | PC 198's earlier seek-enabled strike had become a Preference strike postponed behind an Injury. A newer resolved strike-with-seek registers a Normal seek. Original's nested `RHSequenceElement::Postpone` compares the existing Preference successor with the newer Normal seek, interrupts the stale strike, and installs the new seek behind the injury; Rust deferred all admission until the injury released its old successor, which then displaced the input. Pre-admitting the replacement exposed a second general bug where duplicate manager registrations made an in-progress element arbitrate against and interrupt itself. | Seek-enabled player strikes keep translation deferred but reconcile with an existing postponed chain at registration, preserving Original's nested priority replacement. `arbitrate_instruct` now treats a duplicate instruction for the exact current element as idempotent. Focused tests cover both the replacement chain and duplicate instruction. Replay matches through frame 763. |
| Done | Frame-764 postponed movement retranslation | Soldier 91's movement was already `MoveWaiting` when a higher-priority leave-attentive transition postponed it. Original's movement-specific `SetState(Postponed)` cancels the path request and restores both `MoveWaiting` and `MoveOk` to untranslated `Move`; Rust cleared the orders but retained `MoveWaiting`. When the transition completed, Rust's owner dispatcher therefore treated the internal command as unhandled, terminated it, and installed fallback `Wait`. | Movement postponement now mirrors the Original virtual state-change hook: pending and failed path work is cancelled, translated movement commands return to `Move`, and normal instruction translates the resumed movement again. Focused coverage exercises the trace-derived `MoveWaiting` case and its failure bookkeeping. |
| Done | Frame-766 combat movement facing | Soldier 90 begins forced sword movement toward its observation position. Original `FaceOpponent` computes facing with `GetSector0to15(ASPECT_RATIO)` and classifies the signed `Angle(displacement, facing)`; Rust used raw map-space sector binning and the opposite angle sign, choosing the opposite strafe and moving along its different sprite increment. | Combat facing now uses the shared isometric sector conversion, and directional sword/shield animation selection measures the angle from displacement to facing. Focused tests cover the aspect-ratio bin and right/left strafe sign; the replay now passes Soldier 90 and leaves only Soldier 89 at this frame. |
| Done | Post-hit swordfight-entry lifecycle | Original full/half/lateral sweeps register `ReceiveSwordDamage` and then conditional `EnterSwordfight` on the sequence-manager FIFO. Push records victims when damage is registered at motion-done, then re-evaluates eligibility and queues entry at motion-terminated. Straight strikes queue damage only. Rust instead entered synchronously from every path, and its fallback-timed push completion bypassed both the predicate and manager queue. | One deferred helper emits Original's exact entry payload (`Opponent`, zero `JumplineDestination`, `SwordfightPrepared=false`). Sweeps queue damage then entry, straight strikes queue only damage, and both push-completion paths re-check current attacker/victim state before queueing entry. Table entry now persists and honors `SwordfightPrepared`, preventing preparation movement from repeating after resumption. Focused tests cover push eligibility/timing, the queued payload, and the preparation marker. |
| Done | Frame-766 synchronous interruption callback | Manager-tail damage interrupts Soldier 89's smalltalk strike. Original selects the incoming injury, calls the outgoing element's `SetState(Interrupted)`, and synchronously completes `SendCondolationCard → EventDone → ReconsiderSwordfight` before translating damage. Its strike-selection gate therefore consumes raw draw `40776180` and rejects the strike; damage-side cutting, stunning, and provoke draws follow. Rust queued the old card until after applying damage, so reconsideration consumed the later provoke draw and selected special-strike substate 161. | Manager `InstructOwner` now follows Original's order: generate the incoming transition, arbitrate, expose the still-`Todo` incoming element as the actor's transient selected element, close outgoing condolence callbacks synchronously, then translate the incoming command. Recursive work during the callback arbitrates against that selected injury without falsely promoting it to `InProgress`. Focused coverage locks the nested-arbitration identity; replay matches through frame 766. |
| Done | Frame-767 Halt movement transition | Soldier 91's lost-target handler performs `StopAll → SeekArea`. Original `StopMovement` rewrites the live walk to a shortened walking-to-waiting transition, and generic `Stop` deliberately leaves it alive. Although `GoTo` appears to Halt again, its condition is written `flags & GOTO_NOHALT == 0`; C/C++ precedence makes it always false. Rust both interrupted the rewritten movement in generic owner-stop and faithfully implemented the apparent second Halt, so the callback saw no replacement and zeroed the near goal. | Generic owner-stop now preserves a successfully rewritten movement transition. Ordinary AI GoTo preserves the shipped precedence bug and does not Halt before launch; its separate effective “old movement is computing a path” tail Halt remains modeled. Callback cleanup also separates stale movement tracking from sprite-goal ownership. Focused coverage locks the explicit-Halt-plus-GoTo compound boundary; full replay passed frame 767 and reached frame 780. |
| Done | Frame-780 live strike learning and parade timing | Delayed damage from PC 198's old thrust D is translated after the PC has already selected thrust E. Original calls `MakeBadSwordstrikeExperience(pDamage->GetOrigin()->GetCommand())`, so Soldiers 89 and 92 memorize the attacker's live E command. Rust memorized the D snapshotted in the damage payload. When E starts on the following frame, Original recognizes it and both soldiers enter `ConsiderToBeginParade`, consuming one `ProposeGoodSwordStrike(true)` draw each. After restoring those calls, Soldier 89 counter-struck instead of parrying because Rust gave proposal an unlimited response window from stale `old_action`; Original reads the attacker's live E animation and its remaining frames. | Damage-side learning converts the origin actor's current sequence command at translation time, including Original's parried/no-damage cases, while the damage payload remains authoritative for damage geometry. Strike-start warnings classify from the live animation as `GetSwordStrikeFromAnimation(GetAnimation())` does, and both reactive-parade and unselected-PC proposal timing read that same live animation instead of ActionChange history. Replay passes both soldiers' frame-781 Parade transition and reaches frame 787. |
| Done | Frame-787 event-authorized sword proposals | After the parade correction, Rust consumed each soldier's two drunken-combat guard draws and combat-reposition draw, but omitted both `ProposeGoodSwordStrike` draws. `EventAfterCombatInjury` had reached `ReconsiderSwordfight` from the valid Parade substate and emitted a one-shot proposal authorization, but the later engine consumer re-applied polling-era exact-substate, cooldown, tiredness, pending-special, active-melee, and range gates that do not exist at that point in Original. Successful `StopAll` also failed to interrupt Preference parries postponed beneath the still-running Injury, so Rust resumed them after the injury while Original remained waiting. | The one-shot handoff is now authoritative at the exact Original call site; only the live target recovery and logical sword-action checks that belong immediately before proposal remain deferred. Principal opponents resolve through their actual typed legacy slot, required profiles panic rather than fabricate defaults, rejected proposals persist boredom mutations, and successful proposals execute the full `SetState → StopAll/Halt → synchronous condolations → Say → launch` boundary. Generic owner Stop now descends into Postponed actor work while preserving a stronger Injury, interrupting and detaching the hidden Preference chain. Replay consumes all eight frame-787 combat draws and advances to frame 789. |
| Done | Frame-789 deferred corpse placement | Lethal damage translation at the manager tail selected Soldier 89's `DyingSword` order and reduced life to zero. Rust immediately ran `FindPlaceToDie`, moving the actor by `(-0.4490356, +1.1580811)`, while Original only queues the order on that frame and performs the identical relocation on the actor's next `Hourglass`. | Death and knockout translation now only queue the selected dying/falling order. The shared actor animation path runs `FindPlaceToDie` under the Original `IsInitialisation()` equivalent immediately before `PerformAction`, covering the exact eight `Dying*`/`FallingBack*` human animation families. The geometry was already bit-identical; its lifecycle now is too. Replay passes the death transition and reaches frame 811. |
| Done | Frame-811 PC 198 QuitSwordfight lifecycle | After both opponents died, PC 198 completed its queued parry and ran one sword-state fallback `Wait`. `EvaluateSwordfight` then launched the correct `QuitSwordfight`, interrupted that Wait, and translated the lowering-sword order, but Rust left the accepted element `Todo`. With no `InProgress` current element, command reporting fell back to `Wait`; Original marks the translated Quit element current at the end of the same manager pass and only starts lowering on the next actor tick. | `dispatch_quit_swordfight` now promotes a successfully translated lowering-sword element to `InProgress`, retaining `QuitSwordfight` as the selected command without executing its animation early. Focused coverage locks both the deferred action-state change and current-element visibility. Replay passes frame 811 and reaches frame 839. |
| Done | Frame-839 Soldier 93 corpse avoidance | Soldier 93 began avoiding Soldier 92's corpse two frames before Original because Rust's end-of-pass posture scan exposed both same-frame falls simultaneously. Processing the second transition then cleared its small-corpse radius, while Original's synchronous, creation-order `SetPosture` callbacks leave both overlapping corpses small. | Corpse-intersection batching now reconstructs the pre-pass lying population and exposes transitions incrementally in creation order. A regression covers two overlapping actors falling in the same pass. The full replay passes frame 839 and reaches frame 913. |
| Done | Frames 913–914 Soldier 90 swordfight exit | On reaching its combat-observation destination, Soldier 90 chose the same ordinary GoNear in both engines. Original's GoTo builds one ordered `QuitSwordfight → Move` sequence when leaving any sword action state; Rust cleared relationships through a standalone effect and launched an independent move, which exposed `MoveWaiting`. Once the compound sequence was restored, Rust still skipped the lowering order because generic Execute rejected it while the stale action state remained `MovingSword`. | AI movement intents now carry the required sword-exit prefix through deferred dispatch, and the engine builds one level-ordered sequence. Synchronous owner drains dispatch that Quit through the normal command path, and `TransitionLoweringSword` is recognized as a legal non-movement exit from a moving action state. Focused tests cover both intent construction and generic Execute eligibility. Replay passes frames 913–914 and reaches frame 936. |
| Done | Frame-936 Soldier 91 pride decision | Soldier 91's overview timer reran `BattleDecisions`. Original remained `TooProudToAttack` because lower-pride Soldier 90 was already in `RunningToEnemy` against the same target. Rust required the ally to have an active sword relationship, fell through to `Fight`, and only then launched the observed `EnterAttentiveMode`. | The pride test now uses Original's broad `_ANY_SWORDFIGHT_SUBSTATE_` family, which includes running/walking/charging approaches as well as active swordfight states. The shared raw-substate helper also gained the omitted special-strike member. Replay passes frame 936 and reaches frame 1013. |
| Done | Frame-1013 Soldier 90 strike selection | Original's lateral-strike estimator scans every active human other than the attacker before applying its arc and friendly-fire veto, including friendly actors that its regular strike-victim predicate rejects (such as nearby corpses). Rust globally prefiltered candidates through that regular predicate, erased the friendly blockers, selected lateral strike D, and entered special-strike substate 161 with a timed wait. | Strike candidates now retain whether they pass the regular-victim predicate, and each strike kind applies Original's own collector rules: lateral and straight estimation can inspect the broader active-human set, while push/half/circle retain regular eligibility; the strict 150-unit max-norm cutoff applies only to the four Original collectors that have it. A focused regression proves a regular-ineligible friendly active human can veto a lateral proposal. Replay passes frame 1013. |
| Done | Frame-1014 Soldier 90 post-movement wait | Soldier 90 finishes sword-running and selects an ordinary `Wait` translated as `WaitingSword`. Original executes it immediately despite the stale `MovingFastSword` action-state enum, faces the principal opponent, and changes `direction_goal` from 12 to 13. Rust's stationary post-movement exception admitted only `WaitTimer` and `WaitFreeLift`, so it skipped the selected plain wait. | The full stationary wait command family now shares this selected-order exception. Focused command-family coverage includes `Wait`, `WaitTimer`, and `WaitFreeLift`. Replay passes frame 1014 and reaches frame 1026. |
| Done | Frame-1026 Soldier 90 parry transition | Soldier 90's `WaitingSword` evaluation selects `ParrySmalltalkRight`. Original dispatches the selected order unconditionally even though the completed movement left the raw action-state enum at `MovingFastSword`; the parry's animation-start side effect restores `WaitingSword`. Rust stamped the selected order but its generic animation gate suppressed it based on that stale enum. | Generic Execute eligibility now follows Original's structural ownership rule: any currently selected non-movement sequence element executes normally, while orders stored in a movement element remain movement-driven. This replaces the growing transition/injury/wait allowlist and covers arbitrary interactions such as smalltalk parries. The full animation test module passes; replay passes frame 1026 and reaches frame 1047. |
| Done | Frame-1047 PC 199 movement-start state | PC 199's `TransitionWaitingUprightRunningUpright` reached its target while its raw sprite result was still `InProgress`; because a same-animation successor existed, the TILL_LAST_FRAME arrival tail converted that result to `Terminated` and advanced the order. Original applies the transition's `MovingFast` side effect from the final Execute result before `Proceed` rewrites its reported motion for the successor. Rust evaluated the state effect before its arrival tail and missed it. | Transition Execute state effects now run after the transition-only arrival/termination adjustment, while non-transition effects retain their earlier path. Replay passes frame 1047 and reaches frame 1083. |
| Done | Frame-1083 Soldier 91 attentive exit | A leave-attentive request was postponed behind Soldier 91's enter-attentive animation. By the time Original promoted and translated the leave, the live pose was attentive even though the desired `will-be-attentive` target had already changed to false, so it started the leave animation directly. Rust tested the desired target, prepended a redundant enter animation, and delayed completion by 14 frames. | Soldier final-transition generation now follows Original's `mbAttentive` live-pose test. Regressions cover both the direct current-pose rule and a postponed Enter→Leave sequence that must resume with only the leave animation. Replay passes frame 1083 and reaches frame 1113. |
| Done | Frame-1113 PC 199 movement-goal lifetime | A `HitCmd` input launches an outer Seek after PC 199's actor slot. Original translates that selected wrapper by building a separate concrete movement, self-interrupting the wrapper, and synchronously clearing its old sprite goal before launching the replacement; its first order does not install the new goal until the next actor tick. Rust flattened wrapper and concrete movement, so the outgoing movement's old transition goal survived. Periodic RefreshSeek replacement/failure paths had the same deferred-card identity loss. | Initial Seek translation now preserves Original's two-element lifecycle: the transient selected `SEEK` is interrupted and a separately registered concrete `MOVE | SEEK` owns path dispatch. Selected-Seek cleanup clears the old goal before detaching active mechanics, shared with periodic/cross-sector RefreshSeek boundaries. Ordinary movement replacements retain their distinct goal-preservation rule. Focused regressions cover both initial and periodic replacement lifecycles. The repaired Lincoln diagnostic now passes frame 509 and reaches frame 525. |
| Done | Frame-1121 PC 198 strike-launch facing | Original queues `SwordstrikeThrustA` after PC 198's actor slot and leaves direction/goal at sector 5 until the strike executes next frame. Rust snapped both immediately during sequence translation, using a bare map-space classifier that produced sector 4. Original straight-strike Execute instead classifies the target's ground-space vector with `ASPECT_RATIO`, installs a direction goal, and turns one sector; its hit branch never snaps direction. | Strike dispatch now only creates the order and active-melee state. Execute-time melee direction uses required entities' ground positions and the literal Original aspect classifier, and the non-Original hit-time snap is gone. Regressions cover dispatch timing and the shallow sector-4/5 vector. Replay passes frame 1121 and reaches frame 1122. |
| Done | Frame-1122 PC 199 seek-transition action state | The Running→Walking sprite reaches its raw `Done` marker while PC 199's entity-target Seek remains active. Original `PerformSeek` masks every non-terminal sprite result as `InProgress`, so its Execute switch retains `MovingFast` until the wrapper truly terminates next frame. Rust fed the raw `Done` into the transition state table and changed to `Moving` one tick early. | Entity-target movement now masks non-terminal Execute-visible motion consistently, including the late transition side-effect path; ordinary/point movement still exposes raw `Done`, and RunningUpright retains its Original unconditional `MovingFast` exception. A focused regression covers raw `Done`, wrapper termination, and the non-seek case. Replay passes frame 1122 and reaches frame 1139. |
| Done | Frame-1139 PC 199 seek-refresh counter | Original's entity-target `PerformSeek` unconditionally decrements its shared unsigned `mulWaitTime`; after reaching zero it wraps to `UINT_MAX`. Its refresh gate reinterprets the scalar as signed, so zero and wrapped high-bit values remain expired and can immediately refresh a moved target. Rust stopped decrementing at zero and used an unsigned-positive gate. | The split Rust seek counter now uses wrapping decrement at every corresponding owner tick and signed legacy expiry semantics. Focused regressions cover countdown, zero wrap, continued wrap, and signed expiry. Replay passes frame 1139 and reaches frame 1159. |
| Done | Frames 1159-1160 PC 199 seek-to-Hit boundary | Original path callbacks stamp tolerance/antagonist only when the raw path has more than one point; this direct singleton therefore kept order defaults while its movement element retained the live seek tolerance. Rust stamped the fractional tolerance on the singleton order and completed one frame early. The Original Hit input path also truncates sprite action distance through `UWORD`. On the correct next-frame arrival, Original `StartPostSeekSequence` and Hit translation preserve Moving state/facing until the generated Walking→Waiting transition completes; Rust eagerly forced Waiting and faced the target. | Final-order metadata now follows raw path cardinality before source removal, the exact interaction families reproduce the Original `UWORD` cast, post-seek launch no longer rewrites action state, and Hit translation no longer clears path, snaps state, or faces. Focused regressions cover singleton versus two-point metadata, fractional interaction distance, and Hit translation state/facing. Replay passes frames 1159-1160 and reaches frame 1162. |
| Done | Frame-1162 Soldier 91 persistent enemy list | PC 199's frame-1153 `EVENT_VIEW` explicitly rebuilt Soldier 91's Them list as `[198, 199]`, which Original retained until the overview timer. Rust added an unconditional rebuild at `BattleDecisions` entry; the timer's narrower snapshot contained only primary target 198, so it erased unoccupied PC 199. That made the pride check consider only PC 198, who was already swordfighting, and choose `TooProudToAttack` instead of `Fight` and its attentive transition. | `BattleDecisions` now consumes the persistent Them list exactly as Original does; list rebuilding remains at the explicit perception and state-machine call sites. A focused regression supplies a one-enemy timer snapshot over a two-enemy persistent list and proves the second target survives. Replay passes frame 1162 and reaches frame 1166. |
| Done | Frame-1166 PC 199 Hit initialization | PC 199 and target Soldier 90 remained isomorphic through frame 1165. On the first actual `Hitting` Execute, Original samples the antagonist's live ground position, sets only the progressive direction goal with the non-aspect sector classifier, validates the interaction, then calls `Turn()` and freezes the animation's first frame while still rotating. Rust's generic ability driver omitted that entire Hit-specific initialization/turn branch. | Hit now initializes live ground-space facing at the existing actor order-initialization boundary, preserves the Original direction-before-validity order, turns every execution tick, and uses `FrozenFirstFrame` until aligned. Translation remains facing-neutral, so this does not regress the earlier seek-to-Hit boundary. A focused regression proves each turn leaves Hitting on frame zero. Replay passes frame 1166 and reaches frame 1177. |
| Done | Frame-1177 Soldier 91 target-selection movement goal | The differing startup-transition vector was not a transition or pathfinding arithmetic error: Original aimed exactly at unoccupied PC 199, while Rust aimed at occupied PC 198. Timer tick data only cached distance for the old primary target; Rust's target selector assigned every absent persistent Them-list candidate a fake distance of 10000. Original instead reads each list pointer's live position, applies a stretched-Y max-norm pre-gate, truncates Euclidean distance to `UWORD`, and then applies the occupancy penalty. | Primary-target selection now scores required live entity views, reproduces the max-norm pre-gate, `UWORD` distance/penalty arithmetic, and the Original weak-versus-strong `else if`. Missing required list entities panic instead of receiving fabricated data. A focused timer-style regression proves an uncached unoccupied target beats the cached occupied target. Replay passes frame 1177 and reaches frame 1183. |
| Done | Frames 1183–1184 Soldier 90 hard-hit knockout | The Original interruption crosses the concussion threshold inside `RECEIVE_HIT_DAMAGE`: base `SetConcussionOfTheBrain` synchronously quits the swordfight, adds the unconscious titbit, initializes healing, then the NPC override clears suspects and synchronously handles `EVENT_LOSE_CONSCIOUSNESS`. It suppresses `EVENT_GOTHIT`, performs the PC-stun/money-fight branch, and explicitly quits once more. Rust ignored `ConcussionOutcome`, queued GotHit/Quit/Lose for later, reset `WaitingSword` to `Waiting`, and appended a redundant standalone `FallingBackSword` after the translated hard-hit wrapper. It also applied the non-hard flight pose to the hard `PerformAction` branch. | Hit damage now completes the knockout callbacks synchronously while preserving older detection FIFO work, emits GotHit only for a conscious survivor, retains the sword action family, reproduces PC/same-camp aftermath, and lets the single `FALLING_HIT_HARDER_*` wrapper own its fall. `QuitSwordFight` callbacks are synchronous generally, and `EVENT_GOTHIT` uses the live opponent-list predicate rather than a stale swordfight substate. Hard hit wrappers preserve posture/action until landing. Focused regressions cover the live GotHit predicate and hard-hit pose; replay passes through frame 1198. |
| Done | Frame-1199 Soldier 90 hard-hit landing | The hard-hit sword wrapper reached its sprite `Done` marker while remaining the current order. Original's `ExecuteFallingHit(..., true)` lands posture on both `Done` and `Terminated`; Rust grouped every falling-hit wrapper under the non-hard flight rule and waited for `Terminated`. | Hard-hit wrappers now set lying/dead-back posture on `Done` without prematurely restoring the wrapper-specific action family; that restoration remains at `Terminated`. Non-hard flight wrappers remain termination-only. A focused regression separates the `Done` posture edge from the later sword-action restoration. Replay passes frame 1199 and reaches frame 1225. |
| Done | Frame-1225 Soldier 91 pushed-flight step | Rust queried timing from the logical `FallingPushedWithSword` wrapper and used a generic animation-duration sum, producing an 8-tick flight where Original's `ReadyForTakeOff` uses the resolved `FallingBackSword` animation and `1 + sum(delay + 1)` over every frame except the last, for 16 ticks here. | Falling-hit/pushed flight setup now resolves the wrapper to its concrete sprite animation and uses a dedicated legacy `ReadyForTakeOff` duration helper. Unrelated roll, ladder, and jump timing retains the existing generic duration path. |
| Done | Frame-1240 pushed-flight and true-circle completion | Soldier 91 reached the correct geometric goal but Rust immediately landed it and applied sector metadata on the sprite `Done` frame; Original remains `Flying` until `Terminated`. Independently, PC 199's true-circle tail first reached its final angle on frame 1239, but Rust cleared the sweep before Original's following Execute call exposed terminal direction 5. | Combat flights now retain a zero-increment flight at the goal and defer posture-independent landing metadata, obstacle application, zone refresh, and clearing until the animation completion pass has observed `Terminated`. True-circle sweeps retain geometry for one terminal Execute tick after first reaching the final angle. Replay passes frame 1240 and remains isomorphic through frame 1269. |
| Done | Frame-1270 stunned-sword initiative | Rust merged `BeingWeakSword` and `BeingStunnedSword` initialization, so stunned Soldier 91 incorrectly handed initiative to PC 199. PC 199 consumed the received flag later in frame 1269 and made an extra `MeleeInitiative` draw on frame 1270. Original's stunned wrapper creates the weak/stunned titbit and notifies adversary AI but only `ExecuteWeakness` transfers initiative. | The shared visual/AI side effect now carries its exact wrapper type and gates initiative transfer to `BeingWeakSword`. Symmetric regressions prove weakness still transfers initiative while stun preserves both fighters' initiative flags and still creates its titbit. Replay passes frame 1270. |
| Done | Frame-1381 locked-beggar random speech | At its staggered phase zero, locked beggar NPC 64 selected `CivBeggarBegging`. Rust's direct `RandomSpeech` AI call queued `Say`, then the following `BEGGAR` lock gate returned before any later drain; Original `Say` settles synchronously before checking the lock. | `tick_civilian_random_speech_for_npc` now closes the full no-forecast owner-local AI boundary immediately after `RandomSpeech`, including rejection callbacks and recursive work. A focused regression uses the exact two Original RNG values and proves the queue settles before the lock gate. The owner-work invariant now reports frame and queued work details. |
| Done | Frame-1390 PC 199 WakeUp facing | Original `RHCOMMAND_WAKE_UP` first sets the rescuer's progressive direction goal toward the target, then queues `Turning → WakingUp`; Rust had queued only the interaction animation. After restoring that order, Rust turned one frame too early because the outgoing entity-target Seek unconditionally advanced anti-vibration turning on its successful terminal-tolerance sample. Original returns from that tolerance branch before its later `Turn`/`PerformMotion` block. | WakeUp translation now books the source-faithful Turning order before WakingUp. Entity-target Seek only turns in its non-arrival branch, so a post-seek interaction takes over without an extra terminal turn. Focused regressions cover both the translated order queue and a tolerance arrival with a primed anti-vibration counter. Replay passes frame 1390. |
| Done | Frame-1426 PC 198 wake recovery | On WakingUp `Done`, Original sets the living target to lying, clears concussion, and unconditionally calls `target->Wait()`. That fresh priority-Wait replaces the existing unconscious Wait and synchronously translates to StandingUp at frame 1425; StandingUp starts and sets upright posture at frame 1426. Rust used `ensure_wait_element`, which retained the stale BeingUnconscious order until frame 1426 and booked StandingUp one frame late. | Wake completion now launches the ordinary fresh `actor_wait` element, relying on standard equal-priority arbitration and posture-aware Wait translation rather than forcing posture or animation. The regression seeds a live unconscious Wait and proves it is interrupted and synchronously replaced with StandingUp. The normal replay matches all 1,469 frames. |
| Done | Full-trace scan | The normal first-divergence replay establishes an exact logical match for every recorded frame, including later player interaction, combat, AI, effects, and mission scripting in this capture. | The independent `--scan-all` pass also matches all 1,469 frames without a logical or RNG divergence. Further captures should broaden behavioral coverage beyond this mission-start session. |
| Done | Schema-6 frame-1128 synchronous `UnlockAI` | The Original native does not merely clear `script_locked`: it synchronously re-enters `Think(EVENT_RETURN_TO_DUTY)`, settles that owner's direct AI work, and registers any resulting `GoTo` before the script VM resumes. Rust resumed the VM first and exposed civilian 53's stale `MoveOk`. | `UnlockAI` now uses an explicit VM synchronization request. The engine validates the required actor/AI, unlocks it, closes the owner-local no-forecast AI boundary, and dispatches script-native movement before resuming the callback. Missing required state is an error rather than a fabricated false result. |
| Done | Schema-6 frame-1148 world-space sight coordinates | Several elevated visibility and detection paths mixed projected map Y with a retained world Z, counting elevation twice and changing exact cone/range/opaque tests. | AI sight contexts now carry the same full 3D ground/eye coordinates used by the Original and defer projection only to the routines that explicitly require it. The correction is shared across friendly, enemy, post-detection, and soldier helper paths rather than patched for one actor. |
| Done | Schema-6 frames-1166–1167 scroll and seek state | The attached-scroll interaction relies on the Original's live attachment Boolean, literal 30-unit interaction radius, and wrapping/signed legacy seek countdown. Approximating any of those changed whether the post-seek action launched and on which owner tick. | Rust now synchronizes the attachment flag with script attachment state, uses the source literal for this interaction, and preserves the Original countdown/expiry semantics through the refresh and post-seek boundary. |
| Done | Schema-6 frame-1188 cutscene lock boundary | EdwardZone launches a recorded sequence directly from `Thanx`. Original closes the initial immediate stack before returning from the VM, so `LockUser` and message 13 synchronously stop every AI before their next actor slots. Rust left those actions queued. | Script callback return/resume now drains synchronous sequence actions. Director-completed sequence successors are likewise closed at the Original between-frame callback boundary, so cutscene locks cannot leak an extra actor movement tick. |
| Done | Schema-6 frame-1188 movement-stop destination ownership | Original `RHSequenceElementMovement::StopMovement` changes the movement element's `mptDestination` but leaves the surviving transition order's `pointDestination2D` at its old path goal. Rust shortened both and exposed a different goal/increment. | Stopping movement truncates trailing orders and shortens only the element-owned logical destination. The retained transition order remains aimed at its original point, matching the source's distinct setters. |
| Done | Schema-6 frame-1192 recursive stop through protected movement | A normal/script stop cannot interrupt PC 126's stronger in-progress `PassDoor`, but Original base `Stop` still recurses into and interrupts its weaker queued successor. Rust skipped the movement element entirely, allowing the successor move to survive. | Owner stop always calls the selected element's command-specific stop path. Movement preserves its stronger current transition where required while base recursion still stops weaker descendants; only the identity-owned priority-`Wait` fallback is exempt. |
| Implemented; capture required | Schema-6 frame-1256 camera completion capture | `CameraGoto` completion is driven by the current host viewport, resolution/clamps, and display passes. Schema 6 contains none of those and records no completion event, so the Original's unlock at frame 1256 cannot be derived from the trace. | Schema 7 introduced natural `camera_goto` and `zoom_level` director completions during the Original draw and emits them on the following frame; schema 8 retains them. Rust disables autonomous sequence release while Original-director replay is active, validates and applies each recorded completion before that frame's resolved commands, and synchronously closes immediate successors at the same prior-Draw boundary. Schemas 1–7 are rejected rather than inferring completion or body-gate state from selection/RNG consequences. A fresh schema-8 capture is required. |
| Done | Schema-6 frame-1267 transition endpoint | Rust treated a remaining sub-centipixel transition distance as zero. Original keeps progressing every nonzero vector and therefore retains the transition for one more actor tick. | Removed the `distance > 0.01` transition cutoff. Only an exactly zero vector skips normalization, preserving the general endpoint lifecycle without a trace-specific tolerance. |
| Done | Schema-6 frame-1302 current-position map projection | An elevated actor's turn/interaction path must use the Original's current map position, which is reconstructed from the live 3D position and elevation rather than conditionally selecting an already-projected field. | Current-position mapping now follows the Original projection formula uniformly. Entity identities remain mapped isomorphically; no recorded ID or coordinate is injected into simulation. |
| Done | Schema-6 frame-1305 `TurnElement` and `LockAI` boundary | Original `TurnElement` sets a progressive direction goal and lets normal turning advance it; `LockAI` also preserves the selected command while stopping the actor. Rust snapped/translated the turn differently and exposed an idle fallback command. | Scripted turns use the ordinary progressive goal/turn path, and `LockAI` preserves the selected-command lifecycle while still applying the real general AI lock. |
| Done | Schema-6 frame-1309 terminal animation condolence | Original `RHSequenceElement::SetState(TERMINATED)` synchronously calls the owner's `SendCondolationCard`, `Ready`, and immediate successors before the derived NPC Hourglass tail. Rust left that termination in a global queue, so `UnlockAI` occurred after detection and periodic AI. | The actor slot now closes only that owner's newly terminated sequence stack after animation completion and before `ActionChange`/the NPC tail. This restores the source call boundary for all actor-owned terminal animations. |
| Implemented; capture required | Schema-8 simulation-body gate | A quiet frame cannot reveal whether `PerformHourglass` returned after incrementing the mission clock because zoom-up, zoom-down, or `mbLockEngine` was active. Guessing from absent RNG hid movement and periodic-AI work but could not reconstruct concurrent director/selection changes. | Every Original frame now records mandatory `simulation_body_ran` at the actual post-clock gate. Rust applies that one-frame gate without mutating persistent lock state. The runner accepts exactly schema 8, so an old recording cannot be mistaken for an authoritative oracle at this boundary. |
| Done | Remove obsolete trace compatibility | Supporting incomplete recordings kept symbol-table probing, build-specific RNG offsets, optional semantic command fields, ordinal decoding, and schema-dependent execution paths in the parity runner. Those paths could silently turn a malformed recording into a plausible but invalid replay. | The runner now accepts exactly schema 8. RNG domains, semantic command/action names, director completions, and the simulation-body marker are mandatory; raw numeric ordinals are ignored as diagnostic producer metadata. Symbol lookup, fixed audio offsets, ordinal fallbacks, and all schema-conditioned replay behavior have been removed. |
| Done | Synchronous reciprocal swordfight entry | A terminal combat callback can append the opponent's reciprocal `EnterSwordfight` while the Original sequence manager is already walking its live registration list. `Ready -> Go -> Instruct` therefore dispatches that owner before the manager pass returns; Rust rejected the command in its synchronous owner drain. | The synchronous owner dispatcher now routes `EnterSwordfight` and `PrepareSwordfight` through the same ordinary source-backed translator, including the required opponent payload. It remains a general re-entrant sequence boundary and does not identify the replay or actor. |
| Done | Map-space sword-strike proposal geometry | At frame 1,012 Rust added attacker elevation to only the attacker's projected Y before subtracting the victim's `GetPositionMap()`. That mixed world and map coordinates, made adjacent fighters appear roughly 502 units apart instead of 49, and rejected the Original's first legal strike proposal. | Strike proposal snapshots now use `GetPositionMap()`-equivalent X/Y for both combatants and carry elevation separately for the estimator, matching `GetPossibleVictimsOfSwordStrike`. Replay advances through frame 1,012. |
| Done | Isometric human direction points | At frame 1,060 Soldier 93 is leaning out behind an opaque wall. Original `GetDirectionVector()` compresses direction Y by `ASPECT_RATIO`, placing the eye ray on the blocked side; Rust's Cartesian unit direction shifted the endpoint by about 17 map units and falsely revealed the blip. | Eye, detection, rider/star, dead/dead-back star, leaning-star, and menacer-facing offsets now use the shared 16-sector isometric vector. The audit covers every matching point-helper `GetDirectionVector()` arm; literal crawling-offset tables remain unchanged because they are already pre-scaled. Replay advances through frame 1,212. |
| Done | Exact stale shot retirement | A completed or interrupted shot left Rust's independent `active_shot` tracker live after the selected sequence element had gone away. Original has no independent tracker: deleting the element and its orders makes the shot unavailable immediately. | Owner-card cleanup now clears a shot only when its exact sequence, element, and owner match, so stale shots cannot block a later valid one and unrelated condolence cards cannot erase replacements. |
| Done | Long-jump command and lifecycle | The Lincoln line jump exposed several coupled differences: Rust decoded the authored jump as `Roll`, snapped facing, imposed a fixed four-frame segment cap, delayed airborne motion and topology changes, and settled the retained map goal on landing. Original uses `JumpCmd`, preserves the outgoing moving pose, faces from the source-line normal with gradual turns, derives flight wait from distance/speed, advances consecutive trajectory segments continuously, changes destination topology on the final airborne step, and terminates the jump at landing completion. | Jump translation and execution now follow those source-backed boundaries, including exact takeoff direction, immediate first airborne motion, distance-derived wait, same-animation segment continuity, destination layer/sector timing, landing posture/action timing, retained-goal behavior, and anti-collision restoration. The change is driven by authored jump kinds and works independently of this replay's IDs. |
| Done | Interruptible post-jump tail | Rust marked the final click-to-destination move appended after a line jump with `MAP`, making it non-interruptible. Original constructs that tail with movement flags `0`, so a later click can interrupt its ending transition while the actor is still logically moving. | The line-jump builder now emits an ordinary normal-priority tail. Its existing construction test verifies empty flags, and replay advances through the return jump and replacement click. |
| Done | Live cross-postponed owner continuation | Original terminal `SetState` performs `SendCondolationCard`, `Ready`, and `StartPostponedSequenceElement` synchronously. That boundary may instruct the released successor, but actor `Execute` remains entry-latched: it does not execute a second order in the same actor Hourglass merely because the old order terminated. Rust initially waited too long to instruct the successor, then overcorrected by executing its first movement order in the same slot. | Owner condolence closure returns exact released handles and closes same-owner instructions in manager FIFO order. A released movement successor is installed synchronously but receives its first `Execute` on the next actor Hourglass. Postponed elements retain their original transition-state snapshot, path post-processing uses the Original's live-versus-stored state rule, and `AssertPosition` routes through the normal position-assertion translator. |
| Done | Cancelled pathfinder head occupancy | Original `CancelPathRequest` does not remove a matching list head: it ignores that result when ready, deletes only later matching requests, and lets the stale head occupy the call's single completion slot. Rust deleted every matching queued/in-flight request, allowing a replacement request to start and complete one frame early. | The deterministic synchronous queue now preserves a cancelled head as stale while deleting later same-owner requests. Synchronous delivery remains allowed only when no older in-flight result consumed the barrier's one-result slot. Replay advances through frame 2,131. |
| Done | Ready-before-postponed owner FIFO | When a non-interruptible `PassDoor` terminates, Original `SetState(TERMINATED)` calls sequence `Ready` before `StartPostponedSequenceElement`. The stale route's newly-ready successor is therefore registered before the newer cross-postponed route; normal equal-priority arbitration then lets the newer route interrupt the stale successor. Rust extracted the cross-postponed handle from the middle of its deferred queue, reversed those two instructions, and let the stale frame-1,909 route interrupt the frame-2,074 replacement. | Synchronous owner-boundary dispatch now removes and executes same-owner deferred actions through the released successor in their existing manager-FIFO order, without moving foreign-owner work. Already-terminal postponed links remain a no-op. A focused sequence-manager regression covers `Ready` successor → cross-postponed successor ordering. Replay advances through frame 2,135. |
| Done | Cross-layer human visibility | At frame 2,136 Original soldier 78's Enemy suspect total crossed the shadow threshold after clear visibility samples of PC 126 on a neighboring elevated movement layer. Rust unconditionally zeroed and discarded cached visibility whenever target and observer layer IDs differed, despite matching positions and view direction. | The enemy optical pass no longer invents a same-layer requirement. The shared visibility query already uses full 3D eye/detection points and exact 3D opaque reachability, matching Original `ComputeVisibility(RHElementActorHuman*)` and covering stairs, roofs, and adjoining elevations generally. Replay advances through frame 2,283. |
| Done | Elevated acoustic distance | At frame 2,284 soldier 79 and PC 126 were geometrically close on adjoining elevations. Original `GetHearVolume` subtracts full world-space listener and source positions. Rust started from projected map Y, subtracted source elevation again, and made the pair hundreds of units farther apart, suppressing `EVENT_HEAR`. | Acoustic distance and sound-cover sampling now use the listener's full world X/Y/Z, while only the broad `IsInsideMyHearNoiseBox` prefilter remains in projected map coordinates. This matches the Original coordinate split for all elevated sound sources rather than special-casing the recorded actors. |
| Done | Profile-driven archer identity | Soldier 79 uses the Crossbowman profile with shooting weapon 4. Original `IsArcher()` is exactly `GetBow() != NULL`, established when profile weapons are initialized. Rust derived that fact in transient snapshots but left the persistent `EnemyAi.is_archer_unit` false after mission loading, so the same attentive stimulus entered swordfight behavior. | Mission loading validates every nonzero shooting-weapon reference and initializes persistent archer identity from the resolved bow profile. Missing required bow data now fails loudly. The ordinary AI decision tree therefore sees the same role as transient combat snapshots. |
| Done | Global cross-layer fighter overview | Original `FillListWithAllNearFighters` scans the engine-wide camp fighter arrays, applies stretched-Y max-norm distance and `IsAbleToFight`, and has no logical-layer test. Friendly additions require active swordfight because self is inserted first; enemy additions do not. Rust rebuilt queued-Think tactical data from a stale detection list in one path and discarded every cross-layer fighter in the live path, hiding PC 126 from `ArcherIsToNearToEnemy`. | Both tactical snapshot paths now follow the global registry semantics without a layer gate, retain the Original friendly-swordfight condition, and rebuild hostile PCs independently of the prior Them list. Required primary targets missing from the resulting fighter view panic instead of silently selecting another tactic. Replay advances through frame 2,306. |
| Done | Pre-door lift animation translation | PC 126 exited a stair lift at frame 2,307. Original calls `DetermineMovementAnimation` while the actor is still in the lift sector, rewrites the PassDoor element from `WalkingUpright` to `WalkingStairs`, and therefore keeps the stair row and its authored distances across both door segments. Rust selected the action from actor state alone, switched to upright walking outside, and moved exactly 80% as far. | Lift animation translation is now shared by ordinary movement and PassDoor construction, samples the actor's pre-callback current sector, preserves fast movement, and writes the translated action back through the linked movement element before installing orders. The later forced-crouch rewrite remains authoritative. Replay advances through frame 2,311. |
| Done | Compound seek timer ownership | The entity-target seek armed its 25-tick refresh countdown before constructing a multi-element route. It aged to 16, remained frozen during PassDoor, and Original resumed the concrete `MOVE | SEEK` leg by decrementing 16 to 15 at frame 2,312. Rust generically rearmed every concrete child at path completion and exposed 24. | The outer `SEEK` translation now owns the initial target snapshot/countdown, explicit `RefreshSeek` owns every resample/rearm, and generated `MOVE | SEEK` children preserve both across door and path boundaries. This also prevents a later compound leg from silently shifting the target-movement reference point. Replay advances through frame 2,317. |
| Implemented; capture required | Generic motion-grid and path-request oracle | Schema 8 showed different retreat routing but did not expose the exact request accepted by the Original or the live obstacle-line set. Static source/data analysis showed no difference for the request reconstructed from Rust, making a gameplay change unjustified. | Original schema 9 records every oriented motion line and initial active state, sparse activation changes, and ordered queued/completed path requests with all behavior-relevant inputs and output waypoints (`a33e7de2`). The Rust runner maps line identity isomorphically by geometry plus repulsive behavior, compares live activation every frame, requires schema 9, and includes Original path events in automatic surrounding-frame dumps (`88872ec42`). |
| Done | Source-adapted zero-gate routes | A cross-sector route may become topologically local after PassDoor source adaptation: the actor's live door state can already represent the requested source sector, leaving no gate shots but still requiring the final movement order. Rust treated an empty gate list as route failure and discarded that trailing move. | Door-route construction distinguishes a valid zero-gate route from an unreachable route and always emits its final direct movement. The rule follows the effective adapted source/goal topology and contains no replay or entity identity exception. |
| Done | Started PassDoor replacement and cascade | Original's live pre-priority guard rejects a new `Move` while the current `PassDoor` has started, while an executing-but-not-started PassDoor postpones it. Ordinary `InterruptCurrent` state changes use the no-argument `SetState` default, `CASCADE_NEXT_LEVEL`; omitting that cascade changed which linked elements survived. | Priority resolution now implements the live started/executing distinction and ordinary interruption cascades to the next level. Focused tests cover both started rejection and executing postponement. |
| Done | Transition boundary crossing | Transition-animation movement crosses elevation, patch, and sound lines through the same actor Hourglass crossing check as ordinary movement. The crossing uses the final post-arrival position; a transition step which overshoots and snaps must not report a boundary crossed only by the discarded overshoot. | Transition displacement now feeds the general elevation/patch/sound crossing dispatcher after arrival correction, using the same eligibility and final-position rules as ordinary actor motion. |
| Done | Lazy PassDoor transition continuation | Original keeps the complete PassDoor route in one order list. If a `TILL_LAST_FRAME` transition exhausts its animation before reaching its target, it copies the current order at the first following distinct movement animation and retains the old target for the continuation. Rust's lazy door tail initially hid that following animation. | Transition termination also inspects the untranslated door-pass tail, materializes the source-equivalent continuation, and prevents a completed speed transition from restoring an older saved walking state. |
| Done | Jump formation authorization | `PerformGroupMove` authorizes every PC's formation destination before selecting and dispatching a jump, including a spatial jump click whose raw point is not itself an ordinary motion-sector destination. Rust skipped that per-PC authorization for the jump branch. | Jump dispatch now resolves each PC's effective destination with the normal move-box authorization and uses it consistently for jump-line choice, the post-jump tail, and the marker. This is formation geometry, not a recorded-coordinate override. |
| Done | In-place Make rewrite animation restart | Original mutates the selected live `RHOrder::action` during `MakeFast`, `MakeSlow`, posture rewrites, and related path post-processing. `RHSprite` observes the changed action as a fresh animation even though the order pointer is unchanged. Rust keyed motion initialization only by its stable order ID and reused the previous action/frame. | When a make rewrite changes the selected action, Rust reseeds that order's runtime identity and synchronizes the active lazy PassDoor animation mirror. Unchanged actions retain their existing sprite progress. |
| Done | Unified transition distance across lazy PassDoor tail | At schema-9 frame 1,762, `MakeFast` inserts a four-unit walking-to-running transition. Original `InsertTransitionStart` consumes 3.6204 units in the already-materialized segment, walks across zero-position `Select`/`PassingDoor` action points without moving its cursor, and places only the remaining 0.3796 units in the following route segment. Rust restarted the full four units at its lazy-tail boundary. | The transition scan now carries its last nonzero destination and remaining distance from materialized orders into the separately stored door-pass tail, and skips lazy insertion if the materialized prefix already contains the complete transition. This mirrors the unified C++ list for every route shape and advances parity from frame 1,762 through frame 1,929. |
| Done | Integer hearing-volume threshold | At frame 1,930 soldier 79 samples a 40-volume footstep at distance 39.8138. Original `GetHearVolume` truncates the positive 0.1862 remainder to `UWORD` zero before applying deafness and rejects it. Rust tested the float first, then dispatched `EVENT_HEAR` carrying zero volume. The same ordering governs explicit/scripted noise broadcasts. | A shared hearing tail now subtracts distance, rejects non-positive values, truncates to `u16`, then compares and subtracts integer deafness. Both periodic PC acoustic detection and broadcast noise use it, so fractional volume can no longer trigger a zero-volume stimulus. Replay advances through frame 1,932. |
| Done | Exact stale ability retirement | PC 126's earlier punch applied its `DONE` effect and was interrupted before the sprite's `TERMINATED` edge. The sequence and orders disappeared, but Rust's independent `active_ability` mirror survived and rejected a later valid punch at frame 1,933. Original has no independent ability tracker after the selected element/order is detached. | Condolence cleanup clears an active ability only when its sequence and element exactly match the terminal card, including Listen/ReceivePurse phase cleanup. Incoming replacement abilities are untouched, and no action-state fallback is fabricated. |
| Done | Boundary-inclusive sector polygons | A waypoint exactly on an authored sector polygon edge was rejected by Rust's strict interior test even though the Original's sector containment accepts boundary points. That changed the topology selected for the following route. | `GridSector::contains_point` now treats an exact edge or vertex as inside before applying the ordinary crossing test. Focused coverage includes the exact recorded vertex without identifying its sector or frame in gameplay code. |
| Done | Released Seek command translation | A postponed `Seek` can be released synchronously when an older non-interruptible owner terminates. Rust's synchronous owner dispatcher did not support that command even though ordinary sequence dispatch did. | Both paths now use the same ordered movement/seek translator. Released commands retain their authored flags, target, tolerance, and post-seek tail instead of requiring a replay-side decoder workaround. |
| Done | Per-action nested condolence FIFO | Original closes the nested `SetState -> SendCondolationCard -> Ready` work produced by each released action before advancing to the next older sibling. Rust drained several extracted actions first, allowing a later concrete Seek to overtake an earlier released Move. | Owner-boundary condolence dispatch now reaches a fixed point after every extracted action. This preserves manager FIFO and same-stack arbitration for all nested owner work, not only movement. |
| Done | Creation-slot seek refresh and stable radius | Rust refreshed all moving-target seeks in a global pre-pass and repeatedly reused the adapted concrete movement tolerance as the next seek radius. Original evaluates `PerformSeek` at each actor's creation-order slot and keeps `mfSeekDistance` as an actor-owned unadapted interaction radius. | Seek refresh now runs immediately before that owner's movement, so earlier-created target mutations are visible and later-created mutations are not. `ActorData::seek_distance` persists the base radius; every refresh derives its concrete chase tolerance from that value rather than recursively halving it. |
| Done | Topology-aware AI resolved movement | An AI `GoTo` intent retained only an X/Y point. A destination in another sector/layer was therefore emitted as an impossible straight local walk, while Original expands it through synchronous gate pathfinding. | Resolved AI intents retain optional target sector/layer. Cross-topology moves use the shared deterministic gate-route builder; intentional direct map-exit movement remains direct. Same-sector authored `STRAIGHT` is preserved, while cross-sector routing strips it before expansion. |
| Done | Live validation of terminal entity seeks | At frame 2,153 PC 126 reached the concrete waypoint stored when civilian 50 was at `(551.39685, 1375.96289)`, but the live target had moved to `(525, 1367)` and remained outside the base 33-unit seek radius. Generic Rust order exhaustion launched `HitCmd`; Original's terminal `PerformSeek` checks same sector and requires either an unchanged target position or the pre-motion live-distance predicate, otherwise it immediately calls `RefreshSeek`. | Final entity-target waypoint exhaustion now performs that exact live validation before it may consume the movement or launch a post-seek tail. A stale waypoint refreshes and suppresses the old order pop. The live predicate uses the actor's base seek distance; the adapted path tolerance remains path-only. Successful seeks without a post-seek tail remain in progress with an immediate refresh check, matching the Original. |
| Done | Synchronous `SEEK_STOP_NPC` | The refreshed seek correctly generated `EVENT_STOP`, but Rust queued it until the end-of-frame self-stimulus drain. Sequence-manager Hourglass ran first and promoted civilian 50's queued gate continuation into non-interruptible `PassDoor`; the later halt could no longer cancel it. Original calls `target->Think(EVENT_STOP)` directly inside `RefreshSeek`, while the continuation is still only registered. | All initial and refreshed `SEEK_STOP_NPC` paths use the canonical synchronous AI Think boundary, preserving older deferred detection FIFO while settling `StopAll`, halt, condolence, and re-entrant effects before RefreshSeek returns. The queued interruptible gate successor is therefore cancelled before manager dispatch, with no door, actor, or replay special case. |
| Done | Civilian base-AI incapacitation events | At frame 2,172 civilian 50 received `EVENT_LOSE_CONSCIOUSNESS`. Original common `RHArtificialIntelligence::StartThink` consumed the event before the friendly derived dispatcher and entered `Sleeping/SleepingUnconscious`; Rust's friendly path admitted it to an alerting-event arm that intentionally did nothing, leaving the earlier Seeking state live. The same omitted common switch owns Wasp and Net incapacitation. | Friendly post-filter StartThink now consumes all three common special events: it breaks macros, applies the authored state/substate, eye status, emoticon/alert/sorrow effects, logs the Original refusal reason, and returns before derived dispatch. Soldier-only coin and attentive-mode cleanup remains enemy-only. The pre-existing STOP timer is deliberately not cancelled early because Original performs its stale-timer check before changing state. |
| Done | Turn-frozen PC pickup | At frame 2,249 PC 126 reached `TAKING` action-done and deactivated bonus 105 four ticks early. Original `TAKING`/`TAKING_CROUCHED` initializes facing from the live owner/object map vector, calls `Turn()` every tick, and freezes sprite progression on frame zero while rotation continues. | PC pickup execution now recomputes the live map-space direction goal and uses `FrozenFirstFrame` until aligned. Both upright and crouched variants share completion and terminal posture/state behavior; soldier Taking and Human TakingNet retain their distinct default progression. |
| Done | Explicit lift-transition animation | `MakeFast` at frame 2,480 inserted the same four-unit walk-to-run transition in both engines. Rust then reinterpreted that explicit order through the actor's live stair sector as `WalkingStairs`, consuming the endpoint in one tick; Original lift translation mutates the movement element when instructed, before `PostProcessPath` inserts transition orders, and Execute dispatches those orders literally. | Runtime lift translation is restricted to concrete distance-motion actions. Explicit start/end and posture transitions retain their authored animation on stairs, ladders, and walls. |
| Done | Paired fast-stair motion | The following stair `PassDoor` selected `RUNNING_STAIRS`, which is an Original non-animation dispatch token: it plays ordinary `WALKING_STAIRS` twice, with a separate `Turn()` and per-call slowdown before each motion. Rust first tried to play the token as a sprite animation, then initially combined both raw distances under the final aligned direction. | `RUNNING_STAIRS` now shares the normal-animation/two-call dispatch used by fast ladder and wall tokens. Each literal motion call applies speed and turning slowdown against its own immediately preceding turn state before the distances are combined. Replay advances through frame 2,607. |
| Done | Full-position shadow facing | At frame 2,608 soldier 78 reacted to an elevated PC shadow. Original passes the complete `RHposition` through `PositionToPoint3D`, so sector/layer elevation changes the resulting facing sector and the already-facing `FaceTo` shortcut. Rust retained only flat map X/Y and selected the neighboring direction. | Optical and human target snapshots retain sector/layer, and the shadow handler resolves its target through the contextual 3D facing path. This fixes every elevated positional shadow response without depending on actor identity. |
| Done | World-ground Follow/Stare coordinates | At frame 2,610 soldier 78's narrowed Follow cone spuriously lost PC 126. Original `RefreshView` subtracts `GetPositionGround()` world-horizontal X/Y for both actors; Rust subtracted projected map X/Y, omitting the 45-unit elevation delta from the stare vector. The false `EVENT_OUTOFVIEW` entered randomized `SeekArea` and consumed eleven RNG draws that the Original had not reached. Positional `Focus(RHposition)` had the same latent coordinate-family error. | View refresh now represents own, followed-target, and stored stare points explicitly as ground coordinates. Follow preserves creation-slot target position timing and authoritative airborne height, while positional Focus first uses the shared `PositionToPoint3D` projection. The legitimate periodic bored-speech draw remains in the global stream; only the downstream seek cascade disappears. Replay advances through frame 2,661. |
| Done | Authored PassDoor movement action | At frame 2,662 soldier 78's AI-authored route selected `RunningUpright`, but Rust inferred walking solely from the movement element's FAST flag. Original `DetermineMovementAnimation` starts with the element's authored action; FAST is an independent path property. | PassDoor construction now carries the movement element's authored action through the route builder and applies only the Original posture/lift translations. AI-authored running therefore remains running without any actor or replay special case. |
| Done | Sector-qualified synchronous door combat | Door-battle formation points retained the correct layer but discarded `door.sector_out`. That made a later building-exit route topologically different and shifted its two RNG draws. Rust also queued soldiers' `EVENT_DOOR_COMBAT`, allowing a same-frame `EVENT_REACH_POINT` to observe the old state. | Door-battle positions preserve both authored layer and sector. Soldier `SendBeforeDoorToFight` uses the canonical synchronous Think boundary, matching the direct Original call while preserving older detection FIFO. Replay advances through frame 2,699. |
| Done | Inline AI route construction and RNG | Soldier 78's timer-fired `GoTo` was queued until the global sequence phase. Original expands `AppendMoveToSequence` inline during Think, so its building-exit RNG draws occur between the periodic owner slots for soldiers 74 and 90. Rust consumed soldier 90's idle draw first and produced a wait of 4 instead of 8. | Every synchronous AI order barrier now promotes that owner's queued movement into a deferred sequence immediately. Topology expansion and construction-time RNG occur at the Think boundary, while owner instruction remains in the ordinary sequence-manager phase. Replay advances through frame 2,765. |
| Done | Execute-entry timer ownership | At frame 2,766 soldier 78 entered `Execute` on a `WaitTimer` with one tick left. A synchronous WaitingSword callback interrupted that element before Rust's post-Execute modifier scanned for the live command, so the final decrement was skipped. C++ retains the selected `mpSequenceElement` pointer while the current Actor Hourglass stack unwinds. | `ActorExecuteResult`'s entry element is now the fallback command identity when no genuinely instructed live replacement exists. The selected timer consumes its final tick, while any resulting completion still targets the then-live element under the existing base-Actor rule. |
| Done | Terminal cross-postponement cleanup | A lethal injury postponed soldier 78's active strike, then the same stop cascade interrupted that strike. Rust cleared incoming cross-links only for directly stopped targets, not cascade-stopped descendants. Friday cleanup removed the dead strike sequence, leaving the injury with a dangling target that panicked when the dying animation completed at frame 2,831. | The central sequence state-transition processor now removes every incoming cross-postponement link as soon as its target becomes Terminated, Interrupted, or Impossible. Direct and cascaded stops therefore share the same pointer-lifetime rule, and periodic cleanup cannot invalidate a live link. Replay advances through frame 3,364. |
| Done | Authored SetState/Face instruction order | `SeekingHeardstepsPreReactiontime` has opposite call order in its two branches. The investigate branch calls `Face` then `SetState`: the Turn is instructed, writes its direction, and is subsequently postponed by `EnterAttentiveMode`. The ignore branch calls `SetState` then `Face`: attentive mode registers first, so the later Turn loses priority arbitration before translation and must not change direction until it resumes. Rust batched all Face orders before the attentive request and also wrote direction before arbitration. | Face intents record whether they were authored after the pending attentive boundary. The drain registers pre-boundary work first, attentive mode second, and following Turns in manager FIFO without eager instruction. Immediate Turns now install their direction property before arbitration and change the actor only when instruction succeeds. The synchronous postponed-successor path supports Turn translation on resume. No actor, frame, or replay identity is involved; replay advances through frame 3,424. |
| Done | Seeking side-look choices and completion | Two `rand() & 1` translations inverted `LOOK_LEFT_RIGHT` and `LOOK_RIGHT_LEFT`. At frame 3,425 an even recorded draw therefore selected `LookRight` first instead of `LookLeft`. Rust also treated the ten-frame timer launched beside `SeekingJustWatchingSidewards` as a completion event, while Original leaves that substate only on the look sequence's `EVENT_DONE`. | Both affected seeking call sites now preserve their distinct Original odd/even mappings; an audit confirmed every other random LookSidewards mapping and the downstream command expansion. `SeekingJustWatchingSidewards` ignores timer expiry and waits for actual sequence completion. Replay advances beyond frame 3,435 without RNG or lifecycle drift. |
| Done | Synchronous parry resume and live facing | An attentive-mode condolation synchronously resumed a postponed parry through `Ready -> Go -> Instruct`, but Rust's owner-boundary dispatcher supported the ordinary hourglass parry commands only and panicked. Once resumed, Rust called `Turn()` during `ParryingSword` but did not refresh its goal as PC 126 moved, leaving soldier 67 one direction sector behind at frame 3,516. | Synchronous owner dispatch routes ParrySword, low parry, and stop-parry through their normal translators. Normal parry holds now recompute their direction goal from the live principal opponent before every Turn, sharing the existing WaitingSword geometry; low parry remains unchanged as in Original. Replay advances through frame 3,541. |
| Done | Parry-counter expiry instruction boundary | At frame 3,542 Rust selected `StopParrySword` on the tick that the normal parry counter reached zero, while Original retained `ParrySword` until frame 3,543 and then executed the stop transition. Original decrements the counter and launches the replacement from inside the actor's `RHANIMATION_PARRYING_SWORD` execution; that newly launched command cannot replace the actor until the following engine pass. Rust performed the same bookkeeping in a later global combat batch, where immediate arbitration exposed the replacement too early. | Normal parry expiry is now split across the same observable boundary: the late combat batch only decrements the counter, and the next frame's leading deferred phase launches an expired normal-parry stop before actor execution. Low parry keeps its distinct direct-termination behavior. Replay advances through frame 3,557. |
| Done | Authored seek-point and ambush event flow | Audit of `RHArtificialMalignity::Think` found several timer substitutions around seek points: ordinary `SeekingSeekpoint` accepted a timer without an actual point, passed-ambush reach events synthesized a one-frame timer instead of the Original's re-entrant `Think(EVENT_REACHPOINT)`, and ambush glances armed a second timer and accepted it as completion even though Original waits only for `EVENT_DONE`. The seek-direction list also retained the arrival direction and skipped the first `rand() % 1` draw. | Seek-point arrival now requires `EVENT_REACHPOINT` and a real authored point, shares one inline handler with passed-ambush recursion, filters directions within one sector of the arrival direction using the Original expression, and consumes one insertion draw for every retained direction. Ambush look completion is animation-driven and resumes toward the resolved actual seek point rather than a cached fallback position. |
| Done | Live smalltalk antagonist and facing | Original stores the interaction antagonist on every smalltalk strike/parry order. During execution, smalltalk parries refresh facing from the principal opponent while smalltalk strikes face the order antagonist; Rust retained neither live relationship and eventually turned the fighter toward a stale goal. | Smalltalk translation validates and stores the antagonist on the ordinary order. The shared combat-facing refresh follows the two Original branches for every left/right/low strike and parry animation, without depending on a replay actor or direction. Replay advances through frame 3,761. |
| Done | Door-committed group-move route source | A group move issued while PC 126 was inside non-interruptible gate 59 targeted the side from which the PC had entered. Original `PerformGroupMove`/`AppendMoveToSequence` authored the replacement from the gate's committed far-side sector and therefore included the reverse gate traversal before postponing it. Rust classified from the still-visible near-side sector, authored a local `Move`, and later allowed the stale route to interrupt it. | Group formation and same/cross-sector classification now use the existing live-door source adaptation for every actor actively crossing a gate. Commands that must wait for the crossing are authored from the side on which they will begin, preserving reverse gates and other topology generally. Replay advances through frame 3,869. |
| Done | Cross-sector point-goal tolerance | Soldier 94's `GoNear(..., AI_TALK_DISTANCE)` crossed gate 18. Original preserves the 70-unit arrival tolerance on the final `MoveOk`, so `InsertTransitionEnd` places the running endpoint 105 units before the destination: the Archer's 35-unit stopping animation plus the requested interaction radius. Rust's shared gate builder retained tolerance for entity `Seek` goals but silently reset ordinary positional goals to zero. | Point goals now carry their caller's tolerance through deterministic gate-route expansion. AI `GoNear`, cross-sector combat approaches, and all future positional callers therefore stop at the same authored radius without actor or replay special cases. Replay advances through frame 3,878. |
| Done | Contextual NPC facing and direct report calls | At frame 3,879 an officer already faced the reporting soldier. Original's `Face(RHElement*)` includes target elevation and `FaceTo` returns immediately for an already-facing Waiting/Bored actor; Rust's flattened camp-snapshot helper always queued a Turn. The report dialogue also invented a fallback-to-sender rule for two direct `Think(CALL_YOURTALK_1)` calls, which can bounce an unhandled call forever even though the Original ignores their return values. | NPC-facing call sites now use the live entity-aware facing path and its already-facing shortcut. Direct report-dialogue calls have no fallback; only Original call sites that inspect or deliberately redirect a rejected result retain one. |
| Done | Replay-tool mission speech metadata | The parity example constructed the engine directly but skipped the normal mission-audio loading phase. Every NPC voice therefore received a zero-frame simulated duration, and the reporting soldier's `EVENT_MYTALK_1` arrived at frame 3,900 instead of the Original sample completion at frame 3,947. The installed voice samples also live under the registered locale's `Data/Sounds/Exclamations` tree while `actors.res` stores paths relative to `Exclamations`. | Direct-engine tools can invoke the normal headless mission-audio setup, and the parity runner registers the installed language directory first. Sample resolution supports the Original `actors.res` relative-path convention for both deterministic duration decoding and live playback. The Lincoln replay now loads 2,324 speech-duration entries instead of treating all dialogue as absent. |
| Done | Re-entrant alert-report statement order | When the first report line finishes, Original calls the officer with `CALL_REPORT`, then `CALL_YOURTALK_1`, and only after both nested calls return changes the soldier from report-start to report-point. The blipped officer rejects its reply immediately and calls the soldier back while that soldier is deliberately still in report-start, where the callback is ignored. Rust committed report-point before draining either cross-NPC call, accepted the nested callback, and completed the point action at frame 3,947. | The second direct officer call now carries a typed caller continuation. The ordered synchronous drain closes both recipient stacks before committing report-point and its timer; the non-point branch likewise preserves its distinct Original order of local state change before `CALL_REPORT`. Replay advances through frame 3,975. |
| Done | Patrol admission uses full 3-D actor visibility | Officer 71's authored patrol is rebuilt at frame 3,862. Original `IsDetecting360Degrees(RHElementActorHuman*)` tests the upright chief eye point against each member's posture-dependent detection point; its recorded `(2142,1221,265) -> (2025,1169,265)` ray to soldier 69 is clear. Rust instead tested the projected ground segment against 2-D obstacle polygons, classified that duty soldier as missed, and omitted him from the later alert group at frame 3,976. | Initial patrol admission and missed-member reacquisition now share the existing actor-accurate 3-D distance and opaque-ray implementation, including posture, rider height, direction-dependent detection point, and building gates. No patrol, actor, or trace identity is special-cased. Replay passes the alert-group transition and advances through frame 3,990. |
| Done | Virtual common-AI `SetState` continuation | Soldier 62 reached a route waypoint at frame 3,991. The common Original handler calls the virtual enemy/friendly `SetState`, which runs the actor script's `FilterAIEvent` synchronously before committing the incoming state, and only then resumes `InitializePatrol` and the route-turn tail. Rust committed the state and continued the common handler before exposing the script callback, so re-entrant mission work could observe and modify the wrong side of the state boundary. | Common route arrival now splits at the virtual call: actor effects authored before `SetState` are isolated, the owner-local callback runs against the outgoing state, the incoming pair is committed, and a typed continuation resumes the remaining common-handler statements. This models the source call order generally for that handler without identifying the actor or trace. |
| Done | Direct route-turn manager ordering | The same route handler constructs `RHCOMMAND_TURN` and calls `LaunchSequenceElement` directly; it neither calls `FaceTo` nor instructs the actor inline. A later same-frame `AssignPath` registered another direct turn. Rust reused a normal Face intent, whose implicit `Halt` removed the first pending element before the sequence-manager pass and prevented its ordered interruption callback from advancing the waypoint macro. | Direct route turns carry explicit no-halt/deferred-instruction semantics. Both turns remain in manager FIFO order; the first is instructed, the second wins normal-priority arbitration, and the first turn's synchronous `EVENT_DONE` enters the waypoint macro at the same boundary as Original. Replay passes frame 3,991 and advances through frame 3,996. |
| Done | Farthest-first officer formation assignment | Original's alert code comments that `mlistAlertedUs` is sorted by increasing distance, but its actual insertion condition advances while the new distance is smaller and therefore produces a farthest-first list with later equal-distance soldiers first. Rust followed the misleading comment. Because formation slots are assigned greedily by removing each soldier's nearest remaining slot, the reversed order changed multiple soldiers' movement rays at frame 3,997. Rust also accumulated the officer's average direction after sorting, while Original accumulates it in engine acceptance order. | Alert formation now reproduces the executable ordering bug, including aspect-stretched squared distance and reverse acceptance order for ties, while preserving acceptance order for floating-point direction accumulation. The ordinary greedy slot assignment then yields the Original formation without actor-specific coordinates. |
| Done | Synchronous attentive-mode successor translation | After the corrected formation advanced, soldier 72 completed an owner action whose condolation resumed a postponed `LeaveAttentiveModeOfficer` through `Ready -> Go -> Instruct`. The owner-local synchronous dispatcher rejected that command even though the ordinary manager hourglass already supported all attentive transitions. | Re-entrant owner dispatch now routes enter/leave/officer-leave attentive commands through the shared `NpcAttentionCommandContext`, preserving the same translation and state lifecycle on normal and condolation-driven instruction paths. Replay advances through frame 4,034. |
| Done | Lazy door-route continuation identity | Original stores a translated PassDoor route in one order list. When a walk-to-run transition materializes its running continuation, that continuation becomes the one authoritative current action. Rust updated the concrete sequence order but left its parallel `ActiveDoorPass.current_action` on the completed transition, so the next Lincoln step used transition distance 2 instead of running distance 7. | Materializing a door continuation now updates the parallel action/reverse mirror at the same boundary. Lift handling and the next actor slot therefore observe the same current action as the translated sequence. Replay advances through frame 3,334. Commit: `e0361bd55`. |
| Done | Live actor snapshot on postponed resume | `RHElementActor::Instruct` re-reads the actor's live posture/action whenever a cross-postponed command resumes. Rust reused the snapshot captured when the command was first postponed, so an alerted-running transition initialized against stale state and moved on the wrong first frame. | Resumed actor commands refresh their instruction snapshot from live actor state before translation. The rule applies to every postponed actor element and carries no replay identity. Replay advances through frame 3,524. Commit: `9421ef71f`. |
| Done | Sword movement speed from the selected order | The active actor state can still describe the outgoing transition when a sword movement order begins. Rust selected walking/fast distance from that stale state and dispatched one extra sword-motion step. Original selects motion from the current order. | Sword movement dispatch now derives its speed family from the current order being executed. Replay advances through frame 3,540. Commit: `0a887c9dc`. |
| Done | Deferred strike launch and tolerance-facing order | Original registers direct player strikes at the manager tail; it does not eagerly arbitrate them ahead of an older postponed strike. Its combat seek also applies `FaceOpponent`/`FaceDangerPoint` before returning from a pre-motion tolerance hit. Rust reversed both boundaries, choosing the wrong thrust/facing. | Direct strike input remains in manager FIFO order, and combat seeks preserve the authored turn before the tolerance return. These are general sequence/seek rules rather than recorded direction substitutions. Replay advances through frame 3,641. Commits: `27d3b5317`, `dd31c091b`. |
| Done | One-sided repulsive-line constructor | Every C++ `RHRepulsiveLine` constructor initializes `mType = 0` and only marks its configured side repulsive. Rust defaulted constructed lines to total/two-sided, so actors deviated around authored lines while standing on their non-repulsive side. | `RepulsiveLine::new` now preserves the Original one-sided default. Explicit callers may still request total lines. The Lincoln PC consequently follows the naive motion vector at the affected boundary, and replay advances through frame 4,025. Commit: `1426a0e04`. |
| Done | `RefreshSeek` moving-state restoration | Original retains the actor's Moving state while a completed `MoveOk` remains selected through refresh. Rust finalized the action state to Waiting one owner boundary early even though command, animation, position, and motion all still matched. | Refresh restores the source-equivalent Moving state before processing the retained seek/movement lifecycle. Replay advances through frame 4,039. Commit: `5f1383a30`. |
| Done | Synchronous panic door fallback routing | `NearbyCiviliansPanic` directly calls the recipient's `Think(EVENT_PANIC)`. Original launches the directed `GoTo`, resolves its path failure synchronously, and only then retries the undirected door search using authorization for that exact civilian and live building capacity. Rust drained re-entrant callbacks but not ordinary actor orders, reused soldier/villain authorization for all actors, and checked fallback before path resolution. | Panic closes the recipient's owner-local order/move boundary before both fallback checks. Gate search snapshots canonical per-actor door authorization and live capacity, preserves exit-side link expansion, and promotes the real AI order rather than fabricating a result. Gate and panic suites pass 55/55 and 21/21. Replay advances through frame 4,046. Commit: `776a2cfa3`. |
| Done | Reciprocal swordfight manager phase | A reciprocal `EnterSwordfight` registered during combat work is instructed by the post-entity sequence-manager hourglass. Rust forced synchronous owner promotion, allowing Robin to execute the raising-sword order in the same frame; Original could first execute it on the next actor slot. | Reciprocal entry now uses the canonical deferred manager queue, while genuinely re-entrant actor callbacks retain their synchronous path. Replay advances through frame 4,171. Commit: `dfa82e605`. |
| Done | Live sequence-manager FIFO visibility | During Savegame 024 frame 59, two `ReceiveSwordDamage` instructions interrupted active smalltalk parries while later `EnterSwordfight` elements for the same soldiers were still registered. Original `Hourglass` removes and executes one queue entry at a time, so the re-entrant `EVENT_DONE -> ReconsiderSwordfight` observes the later entry through `SequenceElementIsAboutToBeLaunched` and returns before drawing RNG. Rust detached the whole manager queue into `SequencePhase`, hiding those entries and consuming an extra reconsideration group for each soldier. | The engine phase now pops one manager action at a time and refreshes the source predicate at the actual Think boundary. Later deferred work stays visible during earlier callbacks, while synchronous/WAIT continuations retain their front-of-queue ordering. Savegame 024's extra twelve-draw RNG suffix and resulting damage/command mismatches are gone; replay advances to the independent Soldier 105 quitting-state difference after frame 60. |
| Done | Final-opponent deletion callback | While Savegame 024 frame 59 rearranged a multi-opponent fight, Soldier 105 lost its final opponent. Original `DeleteOpponent` immediately calls `QuitSwordFight` when the live list becomes empty, synchronously delivering `EVENT_QUIT_SWORDFIGHT` and changing a real swordfight substate to `AttackingQuittingSwordfight`. Rust's entry cleanup directly removed vector entries, so the soldier remained in `AttackingSwordfightSpecialStrike`. Rust also drained cloned snapshots, unlike the Original loops which index their live shrinking lists. | A side-effectful `delete_opponent` path now mirrors strength/initiative refresh, PC action restoration, and the soldier quit callback. Both multi-opponent cleanup loops use it and preserve the Original's live-list index advancement. Focused coverage verifies that deleting a soldier's final opponent synchronously enters the quitting substate. |
| Done | Elevated shield-danger facing | Savegame 024 Soldier 92 raised a shield toward an enemy at map `(1083,1563)` and elevation `160`. Original writes `RHElement::GetPosition()` into `SHIELD_DANGER_POINT`, producing world point `(1083,1723,160)`; Rust wrote the AI snapshot's map coordinates with zero elevation. Raising-shield initialization consequently chose direction goal 1 instead of 3 and visibly turned one sector. | AI shield orders now carry the target's world ground point (`world_y = map_y + elevation`, plus world Z), and raising-shield facing subtracts the owner's world ground position just like `GetPositionGround()`. Focused coverage locks the conversion. Savegame 024 passes frame 61 and reaches the independent three-soldier AI-substate frontier after frame 68. |
| Done | Shot obstruction uses the battle us-list | At Savegame 024 frame 68, tower guard 47 chose `Shoot`, but Rust rejected its only in-range target because an undetected same-camp soldier in the broad 500-unit fighter snapshot happened to lie near the firing line. Original `BattleDecisions` rebuilds `mlistUs` from friends passing `IsDetecting360Degrees` and `ProposeShotTarget` scans that exact list for obstruction and bow-target multiplicity. | Shot selection now resolves friendly blockers and cooperating archers in ordered `list_us`, preserving the Original perception and AI-state filters instead of treating every nearby same-camp fighter as a participant. The guard consequently enters `AttackingBowLoading`; the same frame then exposes an independent hidden swordfight-relationship divergence. |
| Done | Deferred direct player-strike admission | The no-seek mouse strike path in Original wraps its element in a sequence and calls `LaunchSequenceElement`. Rust eagerly launched it for the owner, interrupting a done-but-not-terminated strike before the actor slot, creating a transient Wait, and consuming a pending smalltalk parry too early. | No-seek player strikes now launch as deferred one-element sequences. The focused strike lifecycle test passes, and replay advances through frame 4,357. Commit: `9b40ac345`. |
| Done | Exact replay entity isomorphism | The replay runner paired startup entities by kind and mutable map position. On loaded saves it compares an untouched Rust pre-frame state with the Original post-frame record; several inactive beam PCs can also share a position. Windows Save030 therefore produced a three-PC permutation and Save038 a two-PC swap even though each logical character matched exactly under another ID. | Mission loading and legacy-save adoption already preserve each entity's exact Original `RHElement::mulCreationOrder`, including gaps consumed by mobile masters. The engine now exposes that read-only identity and the runner uses it exclusively at startup and for runtime-created entities, validating kind and failing loudly on missing or duplicate identities. No gameplay state or raw table index participates in the mapping. |
| Recording required | Lincoln frame-4,358 legacy panic union bytes | This capture was recorded before the three Original `EVENT_PANIC` producers were corrected. Civilian 51 retained a panic center made by reading an `INFO_HUMAN` pointer payload as `INFO_POS`; much later its `Face` produced recorded direction 4 from those address-dependent bytes. Fixed Original and Rust use the actual broadcaster position, which deterministically yields direction 7. | No compatibility shim or replay-specific direction is permitted. The trace is authoritative only through frame 4,357; a post-`f8a22811` Lincoln capture is required for the remaining 1,195 frames. |

## Workflow

Capture paths are bases rather than individual output files. For example,
`-PARITYTRACE /tmp/lincoln-schema11.jsonl` writes
`/tmp/lincoln-schema11-session-0001.jsonl`, then `...-0002.jsonl`, and skips
existing session numbers on later process runs:

```sh
ROBINHOOD_DATA_DIR=datadirs/fullgame_linux \
  original-code/build/native-full/robin \
  -PARITYTRACE original-code/parity-traces/original-fullgame-schema11.jsonl \
  -PARITYSEED 1
```

Each file is a complete recorder session. Schema-11 `loaded_save` headers embed
the exact v48 checkpoint that produced the initial engine state. Schema 10
remains temporarily accepted by the Rust tool as an old oracle during importer
development, but it cannot represent arbitrary mid-mission state and should
not be used for new captures.

To start directly from a Linux i386 RHSG fixture without menu or profile-save
selection, use `-PARITYSAVE`. Relative fixture and trace paths are resolved
against the launch directory before `ROBINHOOD_DATA_DIR` changes the working
directory:

```sh
ROBINHOOD_DATA_DIR=datadirs/fullgame_linux \
  original-code/build/native-full/robin \
  -PARITYSAVE /tmp/lincoln-restart.rhsg \
  -PARITYTRACE original-code/parity-traces/lincoln-loaded-schema11.jsonl \
  -PARITYSEED 1
```

Use an external fixture or a copy under `/tmp`. The launcher rejects canonical
aliases of the active profile's Continue, Restart, QuickSave, ExQuickSave, and
Sherwood slots. It also suppresses the normal automatic Restart and Continue
writes for direct fixture sessions. Windows i386 `GSHR` v48 files are preserved
and identified by the schema, but the Linux C++ launcher rejects them before
mission construction; feed those checkpoints to the Rust importer instead.

Final full-game builds compile the ordinary developer `-MISSION`/`-PROTO`
switches out. Use the recorder-only direct launch option to bypass Continue
without modifying the active profile or its save:

```sh
ROBINHOOD_DATA_DIR=datadirs/fullgame_linux \
  original-code/build/native-full/robin \
  -PARITYMISSION H01_Lin_VL Lincoln \
  -PARITYTRACE original-code/parity-traces/original-fullgame-schema11.jsonl \
  -PARITYSEED 1
```

Build once, then use the first-divergence run for iteration:

```sh
cargo build --example original_parity_replay
TRACE_JSONL=/path/to/schema11-trace.jsonl
ROBINHOOD_DATA_DIR=datadirs/demo_leicester_linux \
  target/debug/examples/original_parity_replay \
  "$TRACE_JSONL"
```

The first run converts the JSONL source into an adjacent
`*.parity-cache-v4.native-bincode.zst` file. The cache contains the typed trace
header, RNG prefix/suffix, and every frame as length-delimited native bincode
records inside a streaming zstd level-0 payload. Subsequent runs read only that
cache. Its source-length and modification-time fingerprint automatically
invalidates it when the JSONL changes, and conversion is written to a temporary
file before being atomically persisted so an interrupted conversion is never
accepted as complete.

On the first logical or RNG divergence, this default run writes a complete
JSONL snapshot for the divergent frame and its 32 predecessors to a unique
temporary path and prints that path. Use the explicit `--dump-jsonl` options
only when a different frame range or entity filter is needed.

For long traces where only the first divergent frame is needed, skip the
33-frame rolling engine snapshot:

```sh
ROBINHOOD_DATA_DIR=datadirs/fullgame_gog \
  target/debug/examples/original_parity_replay --no-auto-dump \
  "$TRACE_JSONL"
```

This retains first-divergence comparison and reporting, but does not write the
automatic engine dump. Re-run a narrowed range with `--dump-jsonl` when the
full diagnostic state is needed.

To inspect the authoritative global RNG stream during an Original replay, set
`ROBIN_TRACE_RNG=1`. Each consumed draw reports its zero-based stream index,
reviewed call site, and raw Original `rand()` value. Use
`ROBIN_TRACE_RNG=backtrace` when call stacks are also needed. Late divergences
can be isolated with inclusive `ROBIN_TRACE_RNG_FROM` and
`ROBIN_TRACE_RNG_THROUGH` draw indices.

To watch that same authoritative replay, add `--visual`. The window freezes on
the first divergence while the normal logical mismatch report is printed:

```sh
TRACE_JSONL=/path/to/schema10-trace.jsonl
ROBINHOOD_DATA_DIR=datadirs/demo_leicester_linux \
  target/debug/examples/original_parity_replay --visual \
  "$TRACE_JSONL"
```

To capture the adopted state before frame 1 into a chosen directory, use the
frame-zero screenshot option. It restores the recorded campaign and embedded
Original save through the ordinary mission frontend, then uses the complete
viewport renderer with HUD and correct sprite-shadow handling. The required
GPU window stays hidden unless `--visual` is also present; flattened
corpus-relative names avoid collisions between profiles:

```sh
TRACE_JSONL=/path/to/parity-save-replays/traces/Profile/Savegame_000-session-0001.jsonl.zst
ROBINHOOD_DATA_DIR=datadirs/fullgame_linux \
  target/release/examples/original_parity_replay \
  --frame-zero-screenshot-dir output/parity-frame-zero \
  "$TRACE_JSONL"
```

After the first-divergence run is clean, collect the first occurrence of every
remaining compared-field mismatch with:

```sh
TRACE_JSONL=/path/to/schema10-trace.jsonl
ROBINHOOD_DATA_DIR=datadirs/demo_leicester_linux \
  target/debug/examples/original_parity_replay --scan-all \
  "$TRACE_JSONL"
```

For interactive inspection, start the headless runner paused with its local
HTTP endpoint:

```sh
TRACE_JSONL=/path/to/schema10-trace.jsonl
ROBINHOOD_DATA_DIR=datadirs/demo_leicester_linux \
  target/debug/examples/original_parity_replay \
  --http-server 17640 --start-paused \
  "$TRACE_JSONL"
```

While it is running, `GET /engine-dump` returns the complete current engine,
`GET /state` reports the frame, and the existing controls advance the Original
trace while retaining its recorded commands and RNG stream:

```sh
curl -s http://127.0.0.1:17640/engine-dump
curl -s -X POST -H 'content-type: application/json' \
  -d '{"n":1}' http://127.0.0.1:17640/step-forward
curl -s -X POST -H 'content-type: application/json' \
  -d '{"frame":45}' http://127.0.0.1:17640/go-to-frame
```

Forward steps reply only after all requested frames match. A divergence halts
advancement and makes the in-flight step fail, but leaves `/engine-dump` and the
other inspection endpoints alive at the divergent frame. Backward stepping is
deliberately unsupported: restart the runner and use forward `go-to-frame` so
every inspected state is rebuilt from the mission-start trace contract.

For every newly fixed divergence:

1. Identify the first causal mismatch, not merely the largest downstream diff.
2. Confirm the behavior in `original-code` and record the relevant Original
   function or rule in this ledger or the code comment.
3. Implement the smallest general behavioral correction; do not special-case a
   recorded frame, actor, or ID.
4. Add focused regression coverage where practical.
5. Re-run from mission start because an earlier fix can change all later state.
6. Update the clean prefix, ledger status, and remaining first divergence here.

### Native-save post-load side effects

Linux-v48 adoption now has a dedicated post-load plan for state which Original
does not merely deserialize. In Original stream order it completes any NPC
remark that was active when saved, restores the signed 32-bit `time_t` seed
through `srand(unsigned int)` for a normal load, clears only the global
forbidden-remark list, and rebuilds PC produced noise after sequence-pointer
fixup. Parity replay selects a separate explicit policy which preserves and
continues the recorded global draw stream.

The same plan returns an exact host-reset contract for the old-click and drag
targets, click-suppression flags, focus and multi-selection latches, selected
layer/sector/patch, door display, trajectory preview, background validity,
transient thunder effects, and last-viewed sight caches. These are reset-only
load consequences, not fields to reconstruct from the file. Rust has no
persistent equivalent of Original's sight/projection last-viewed optimization;
current sight is recomputed, while host-owned derived displays are invalidated.
The plan remains outside the public install boundary until the complete
Linux-v48 coordinator can apply it atomically with every serialized slice.

### Windows-v48 save ABI normalization

Retail Windows `GSHR` saves now use the same atomic adoption path as Linux
`RHSG` saves after the contextual decoder has normalized their audited
call-site layouts. `RHArtificialIntelligence::SerializeThisAI` stores each
Windows AI log entry as the complete naturally aligned 12-byte `RHlogLine`
aggregate, including its inert two-byte padding, while the Linux port's
`CHECKENUM` path stores only the first four-byte type word. The producer ABI is
therefore threaded into every mission-local AI decoder instead of assuming the
Linux record width.

Repulsive-point width is selected by serializer call site, not merely by file
magic. Windows Human payloads use the wide geometry compatibility layout, but
`RHElementObject` payloads (including the standalone trajectory helper) use
the ordinary narrow layout. The importer preserves that asymmetry without
scanning for fingerprints or skipping guessed bytes.

Windows music stream positions are legitimate saved values and are retained in
the deterministic sound/host state. Retail Human building indices use both
zero and `0xffff` for null and may contain a stale sector from another level
build; exactly as in the Original compatibility loader, a valid building is
accepted directly and any other non-null value is resolved from the actor's
authoritative deserialized position sector. The first Cyrdach Windows fixture
now decodes to exact EOF, adopts atomically, and matches all 250 recorded
frames.

Base-element delayed coordinates are authoritative only while their matching
`position_map_delayed` or `position_delayed` bit is set. Original's setters
overwrite the complete point before enabling the bit, so inactive coordinate
bytes are dormant constructor storage and can legitimately be non-finite.
Adoption validates active delayed points strictly but does not reject dormant
bytes; Cyrdach's Windows `Continue-session-0005` contains such an inactive
`NaN` and now adopts without weakening validation of values Original will
consume.

`RHElementActor::mmotionState` is likewise a serialized diagnostic rather than
a load continuation input. On every first post-load `Hourglass`, Original
assigns the result of `Execute()` before the switch which reads this member.
Initialized enum values remain preserved, while arbitrary out-of-domain
Windows storage is canonicalized to the constructor's `RHMOTION_DONE`.
Cyrdach's `Restart-session-0003` contains a pointer-like word in this dormant
slot.

The actor's material-boundary distance is a cache serialized twice. Original
does not validate either copy: a null material sector ignores the cache, while
a changed map position recomputes it before returning it. Rust retains the
exact cached float bits, including legacy `NaN`, but still requires both
redundant copies to agree so a structural decode error cannot pass as cache
storage.

### Linux-v48 AI `mOldState` storage

Profile 011 creation order 85 stores `local_ai.old_state = 119`, outside the
seven-value `RHaIState` domain. Auditing every neighboring field and NPC shows
that initialized Linux enum records are full clean 32-bit values; the earlier
signed-low-word interpretation was incorrect. `RHArtificialIntelligence`
initializes `mCurrentState` but never initializes `mOldState`; `StartThink` is
its only assignment, and the Original has no reads of the member. Other NPCs in
the same fixture contain arbitrary full storage words such as `0x00960003` and
`0x5a3b010e`, confirming constructor indeterminacy rather than a packed-enum
ABI.

Rust retains `old_state` as the exact signed 32-bit storage word, overwrites it
with the current valid state at the same pre-filter `StartThink` boundary, and
never fabricates a valid enum for untouched brains. All behaviorally consumed
neighboring enums validate their complete serialized word instead of masking
corruption through low-16-bit truncation.

### Linux-v48 inactive patrol-macro cursors

Linux saves contain both never-started macro cursors and interrupted macros
whose pointer difference lies outside the current waypoint. The latter can
retain a nonzero `remaining_macro_bytes`: `BreakMacro` clears only
`mbMacroInProgress`, leaving both the cursor and counter untouched, while
`SerializeThisAI` writes them whenever a patrol path exists.

Rust therefore uses `macro_in_progress` as the authoritative liveness bit and
preserves the raw inactive offset and byte count for exact state inspection.
Command-kind and `offset + remaining` bounds remain strict whenever that bit is
set, so malformed live bytecode still fails instead of being replaced with a
plausible cursor. Interrupted dormant storage is never dereferenced, matching
Original's resume gates.

### Linux-v48 enemy previous-state storage

Profile 011 creation order 89 stores `local_ai.enemy.previous_state = 72`,
outside the seven-value `RHaIState` domain. `RHArtificialMalignity` initializes
neither `mPreviousState` nor `mPreviousSubstate`; it nevertheless serializes
both complete four-byte enum words. The pair is assigned together only when a
non-default soldier sees Charly and enters
`SUBSTATE_SEEKING_DETECTED_CHARLY`, and is consumed only by that substate's
officer/timer branch.

Rust therefore retains both serialized words exactly and overwrites the pair
at the matching Charly transition. It converts them to validated enums only at
the one semantically live restore-state branch. An invalid live value remains
a hard failure instead of being replaced with a plausible state; arbitrary
constructor bytes in an inert save no longer prevent adoption.

### Linux-v48 default stimulus storage

Profile 011 creation order 103 contains an active delayed-stimulus entry whose
type word is `0xef280000`, but whose other fields exactly match a
default-constructed `RHStimulus`: `INFO_NONE`, null owner, and no whole-patrol
flag. This is not decoded as low-byte `EVENT_VIEW`; a real view stimulus carries
human information, and the Linux build does not use short enums.
`RHStimulus::RHStimulus` initialized every one of those surrounding members but
left `mType` indeterminate.

For old saves, Rust accepts an invalid type only with that exact constructor
shape, retains its raw word in `StimulusInfo::LegacyInvalidType`, and routes it
through `NO_EVENT`, which has the same default script/event dispatch behavior
as Original's unknown switch value. Invalid types with any live payload, owner,
or patrol flag remain hard errors. Original now initializes `mType` to
`NO_EVENT`. Its loader canonicalizes only this exact dormant legacy shape to
`NO_EVENT`; any invalid type with live surrounding state is rejected. Its write
path also rejects noncanonical enum words and always emits the canonical
four-byte value, so future saves and parity recordings cannot propagate
process-dependent bytes at this site.

### Original command discriminant ABI

Profile 011 first exposed the issue as actor 81's fallback
`mpWaitSequenceElement` resolving to `WAIT_TIMER`. The pointer topology was
correct: it referred to the selected in-progress element, had wait priority,
and contained the expected bored-idle orders. The command discriminant was
wrong because Rust's enum differed structurally from `RHcommand`.

The complete 179-value Original enum is now preserved ordinal-for-ordinal.
Rust restores the dead `RHCOMMAND_ROLL` slot at 76 and the misspelled dead
`RHCOMMAND_ANNIMAL_AFFRAID` slot at 145. Rust-only `Jump`, `LeaveSpy`, and
`LeaveTree` commands live after Original's final value rather than shifting
serialized ranges. This corrects not only `WAIT`/`WAIT_TIMER`, but every v48
command between the earlier inserted/omitted slots; pinned boundary tests guard
the legacy domain and the separate Rust-only tail.

### Linux-v48 restored turn goals

Profile 011 resumes two guards with authoritative serialized direction goals
14 and 12 and in-progress `TURN` elements whose orders have dormant `(0, 0)`
destinations. Original `Translate(TURN)` writes the goal once from
`RHFIELD_DIRECTION`; its subsequently serialized `RHOrder::bComputeDirection`
remains at the constructor default but never asks actor Hourglass to derive a
new turn goal from the order destination.

Rust's positional AI-turn sweep gives its `compute_direction` flag that
additional meaning. Import therefore clears the runtime flag on restored
`TURN`, `TURN_FAST`, and `TURN_ELEMENT` `TURNING` orders, matching live Rust
translation and preserving the saved goal. Without that semantic conversion,
the first post-load frame incorrectly faced each actor toward map origin.

### Linux-v48 dormant object repulsive-point storage

Linux2/Linux3 saves contain non-finite values in different scalar members of
`RHElementObject::mrepulsivePoint`: action radius, both force coefficients, and
radius. This is expected legacy constructor storage, not live collision
geometry. `RHRepulsivePoint::RHRepulsivePoint()` initializes flags, concavity,
and identity but leaves those four scalars indeterminate, while
`RHElementObject::Serialize` writes the complete point unconditionally.

Original's default `GetRepulsiveObjects` either omits the point for an object
with zero radius or overwrites its position and all four force scalars before
inserting it. `RHElementNet` does the same before inserting either embedded
point, and `RHElementCoin` never inserts one. Rust therefore retains every
serialized float as its exact `u32` bit pattern in a JSON-safe compatibility
sidecar, along with the point's flags and identity, but never treats that
sidecar as runtime geometry or semantic state-hash input. Live
static-grid/actor repulsive geometry keeps strict finite-value validation; this
exception is limited to the proven dormant object-owned leaf.

### Linux-v48 dynamic bonus constructor draws

Loaded-save traces initially stopped at the setup boundary with Rust at global
RNG cursor 228 and Original between 229 and 266. Across Linux2 and Linux3, the
cursor excess exactly equals the number of bonus objects reconstructed by the
phase-one load factory: representative saves contain 3, 6, 13, 14, and 19
dynamic bonuses and differ by precisely those counts. Dynamic PCs and other
object classes account for none of the excess.

`RHElementBonus::Initialize` calls `ForceRandomSpriteFrame` once from every
constructor, including constructors used only to rebuild a saved element.
Phase two overwrites that temporary frame from the serialized sprite payload,
but the global `rand()` draw remains authoritative for every later consumer.
Rust now consumes the same `LevelBonusInitialFrame` draw in saved element
order immediately before installing each constructed bonus. It does not skip
the replay cursor or infer a correction from the trace header; the ordinary
class-specific load factory itself reproduces the source side effect.

### Linux-v48 undefined live-PC eye points

After the dynamic constructor RNG cursor matched, multiple Linux2/Linux3 saves
reached active, playable dynamic PC 101 with serialized posture
`RHPOSTURE_UNDEFINED` before that actor's later owner slot established a
concrete posture. Rust's eager AI snapshot computed eye/detection height for
the PC and panicked.

Original does not substitute an upright posture here. `ComputeEyesPoint`
initializes its result to `GetPosition()`, reports the unknown posture through
its nonfatal default arm, and returns the unchanged point.
`ComputeDetectionPoint` also initializes from `GetPosition()` and, in the
shipping/replay build, returns unchanged after its default-arm assertion.
Rust now represents those exact zero-offset points for `Undefined`/unused
postures (and for the eye-only `Carried` default arm), while retaining the
serialized posture itself. Living optical targets consequently carry a valid
ground-height detection point instead of being rejected before Original's
ordinary active/building/detection gates can decide whether it is consumed.

### Linux-v48 dynamic-PC constructor geometry

Several Linux2/Linux3 saves next reached a moving dynamic PC whose serialized
map-space move box was valid, but whose local centered move box remained unset
and whose pathfinder index remained `u16::MAX`. Anti-collision therefore
failed when it requested the local half diagonal. This was not malformed save
geometry: `RHPositionInterface::Serialize` deliberately omits both
constructor-owned fields.

`RHElementActorPC` reconstructs them after loading the character sprite,
looking up `ubPathFinderIndex` in the active fast-find grid and installing the
resulting centered box plus the index. Rust's dynamic load factory now performs
that same initialization before phase two restores the serialized position
interface. Preflight fails explicitly if the saved PC's profile references a
move-box table entry absent from the loaded mission, rather than fabricating a
unit box. Static PCs continue to retain their mission-start constructor state.

### Linux-v48 exact PC campaign-description identity

After constructor geometry was restored, every representative save reached its
first snapshot but reported groups of PC ammunition counters from a different
character. Rust had validated the serialized `mpDescription` index and then
discarded it, resolving later status access through the first campaign
description with the same character profile. Profiles are not unique: rescued
and replacement characters routinely share one profile while owning separate
status records.

`RHElementActorPC` retains `mpDescription` and its `mpStatus` alias exactly;
`mubListIndex` is unrelated actor/UI state serialized separately. Rust now
stores the exact campaign-description index on each PC, sets it on every
mission/rescue/reinforcement constructor, and restores it from the saved
pointer during Linux-v48 adoption. Status access validates both the index and
profile identity and never falls back to a profile search. A focused duplicate-
profile test keeps the first matching description full while the selected
description is empty, proving pickup capacity follows the exact pointer.

The serialized description may intentionally name a different profile than
the mission-start PC constructor. Original calls this out for PCs waiting to
be rescued: after reading `mpDescription`, `RHElementActorPC::Serialize`
replaces `mpProfile` from that description. Linux-v48 adoption now does the
same and refreshes Rust's profile-derived character kind and contextual
movement permissions. It retains constructor-only state that Original does
not replace at this point, including the `mbRobin` flag and constructed sprite
geometry.

Loaded campaigns can also cause `PopulateBeamMes` to construct the selected
team in a different order and with different transient beam-me indices than
the running mission that wrote the save. Creation order is therefore not a
logical PC identity. Before adopting element references, Rust now pairs static
serialized team PCs with initialized team entities by character profile,
preferring an exact `mpDescription` campaign index when it is still present,
and rebuilds both Original reference maps from that isomorphism. Duplicate
campaign descriptions sharing a profile are interchangeable until their exact
saved descriptions are applied. Dynamically reconstructed and rescue PCs
retain their already-exact mappings. This prevents one hero's saved sprite row
and frame from being applied to another hero's sprite profile.

### Linux-v48 dormant carried-posture storage

Profile 011 creation order 157 stores `mCarriedPosture = 161437968` while
`mpCarried` is null. `RHElementActorPC` initialized the pointer but not its
associated posture, then serialized both fields unconditionally. The posture
becomes live only when a PC takes a body, at which point Original overwrites it
from that body's current posture before later restoring it while dropping the
body.

Rust retains the exact raw word while `carried` is null, overwrites it on every
carry/climb transition, and converts it to a validated `Posture` only at live
drop/synchronization sites. Import rejects an invalid posture whenever the save
also contains a carried actor.

Original now initializes the dormant posture to `RHPOSTURE_UNDEFINED`. Its
loader retains the four-byte v48 layout and canonicalizes an invalid legacy
word only when there is no carried actor; an invalid posture paired with a live
carried pointer is rejected. The writer rejects noncanonical posture words, so
future saves and parity recordings cannot reproduce uninitialized constructor
storage at this site.

### Linux-v48 dormant sequence transition posture

Profile 011's `Restart` save contains SequenceManager element ID 85 with
`mpostureAfterTransition = 252736` and
`mactionStateAfterTransition = 48`; TODO element ID 86 also stores invalid
action state `913`. Element 85 is a level-one `RHCOMMAND_MOVE` in `RHSEQ_TODO`
with `RHPRIORITY_NOT_YET_SET`. Both
`RHSequenceElement` constructors left the transition posture and action state
uninitialized even though `Serialize` always wrote them.

The posture is not live in this record: `RHSequenceElement::Go` sends a TODO or
postponed actor command through `RHElementActor::Instruct`, which stamps the
actor's current posture and action state before transition generation or
translation can inspect them. An in-progress element is different—its stored
transition result is authoritative and is read throughout movement and command
execution.

Original now initializes new sequence elements to
`RHPOSTURE_UNDEFINED`/`RHACTIONSTATE_WAITING`. Its loader retains both four-byte
v48 enum fields. Rust treats both words as dormant scratch storage for every
non-progress element, retaining the exact raw words for save fidelity while
resetting the runtime fields to the constructor sentinels. This applies even
when a dormant word is a valid enum: a later expanded-corpus save contained a
TODO move with the valid but obsolete `RHACTIONSTATE_WAITING_SWORD`; preserving
it made Rust translate ordinary walking into sword movement after the actor had
already left combat. Original overwrites both words from the actor's live state
at every `RHElementActor::Instruct` boundary. In-progress transition state
remains authoritative and invalid values there are rejected. The writer rejects
noncanonical posture or action-state words, and sequence/manager/engine
serialization propagates that failure instead of continuing a malformed
stream.

### Transition-distance continuation start state

Three expanded Linux saves reached an ordinary walking path whose
`TRANSITION_WAITING_UPRIGHT_WALKING_UPRIGHT` animation exhausted before
covering its four-unit target. Original retains the unreached target by copying
the transition order at the following animation change. The resulting walking
continuation moves on its first actor slot but exposes `RHMOTION_IN_PROGRESS`,
so the `RHMOTION_START` arm does not change the actor from `WAITING` to
`MOVING` until a later ordinary walking booking. Rust generated the same
continuation and exact displacement, but treated it as a fresh walking start
and exposed `MOVING` one frame early.

Every generated transition-distance continuation now carries a one-shot
runtime tag for its first `Execute` slot. If that slot exposes `START`, its
posture/action-state effect is deferred until the movement result establishes
whether that copied order survived the call. If the first slot already reports
`IN_PROGRESS`, the tag is still consumed: a later real `START` is authoritative
and must not be suppressed. A continuation which remains current enters
`MOVING`; a short continuation which satisfies its arrival predicate and hands
off in that call retains the transition's `WAITING` state for that frame.

PC continuations retain their distinct two-stage handoff: the copied
continuation defers its START until the same survival check, and a second
one-shot tag establishes the deferred movement state when the authored walking
successor remains short of its goal after executing. This matters when the
sprite continues the same animation or the proximity wrapper reports
`TERMINATED`: neither diagnostic result alone reproduces the Original actor
state. A copied or authored successor which actually reaches its goal and hands
off in the same call retains `WAITING`.

The ordinary START side-effect path must also skip tagged PC successors. The
tagged handoff owns that effect exclusively after it verifies that the order is
still current; otherwise the generic path can set `MOVING` before a short
successor is replaced by its stop transition. Cyrdach's Windows
`Continue-session-0005` exercises this exact same-call replacement and now
matches its complete recording.

Cyrdach Windows save 007 exercises the other branch: its copied PC
continuation survives its START call. The same post-execution identity check
therefore establishes `MOVING`, matching Original at frame 1,264.

This two-stage handoff fixes the repeated Linux3 Profile 001 `MoveOk` cluster;
save 021 now matches through its complete recording instead of diverging on
PC 114 at frame 202. It also preserves the complete matches for Linux2 Profile
002 saves 013 and 021 and Linux3 Profile 003 save 064, including the latter
save's earlier continuation at frame 5,929. Linux3 Profile 002 `ExQuickSave`
still exposes the surviving Soldier 62 continuation as an ordinary START at
frame 17,422, while Linux3 Profile 003 save 011 keeps Soldier 97 `WAITING` for
its same-call-consumed continuation at frame 22,732. Save 011 now matches every
recorded frame.

### Isometric smalltalk step-back direction

Two loaded-save traces reached the same `IsStepBackNeeded` displacement with
the correct animation distance and X coordinate but an overlarge Y component:
Linux3 Profile 003 save 040 at frame 260 and save 064 at frame 5,983. Rust used
the unprojected 16-sector unit vector for both the opponent-facing test and the
step-back destination. Original `RHElement::GetDirectionVector` calls
`SBGeoVector2D::SetSector0to15(direction, ASPECT_RATIO)`, which compresses the Y
component into isometric map space.

The smalltalk step-back path now uses the shared aspect-corrected sector vector
for both operations. This is the general `GetDirectionVector` translation and
does not depend on recorded positions or entity identities. Save 040 advances
from frame 260 to 267 and save 064 from frame 5,983 to 5,992; both then expose
the same independent `Provoke` versus `Wait` command mismatch.

### Sword-movement termination and synchronous Provoke

Original walking `Execute` launches a range-based `Provoke` when a sword
movement returns `RHMOTION_TERMINATED`. Rust staged position commitment after
the sprite call, so a final step could retain the sprite's pre-geometry
`RHMOTION_DONE` result and skip the callback. Once collected, Rust also
instructed the Wait-priority taunt before advancing the still-InProgress
movement element, causing priority arbitration to abandon it. Original
registers the element inside `Execute`, but its `Ready -> Go -> Instruct`
dispatch occurs after `DoNextOrder` closes the movement.

Sword-movement termination is now collected at the committed waypoint-arrival
boundary. Its taunt is instructed after movement order advancement, and the
owner-boundary synchronous dispatcher shares the ordinary Provoke translator.
Save 040 advances from frame 267 to 381, where it joins the independent
smalltalk strike/AI mismatch; save 064 advances past frame 5,992 and exposes an
independent missing synchronous `LookLeft` route.

### Loaded PassDoor queues without runtime sidecars

Linux3 Profile 003 saves 000, 001, 015, 017, 032, 050, and 052 all loaded an
InProgress `PassDoor` element whose current order was `PassingDoor`, then
panicked because `ActorData::active_door_pass` was absent. That sidecar is a
Rust translation aid and has no Original serialized counterpart. Original
serializes the already-translated order queue, `RHPositionInterface::mpDoor`
plus its direction, and `RHElementActor::mbPassingDoorDirectly`. On the first
`RHNONANIMATION_PASSING_DOOR`, a non-null saved door is consumed and performs
the sector/layer change; the later action point sees a null door and only
re-enables anti-collision.

Legacy-adopted movement elements now execute their authoritative serialized
queue directly when no runtime pass sidecar exists. A non-null restored door
queues the normal shared door callback and is then cleared; a null door
restores anti-collision. Runtime-created passes retain strict sidecar
invariants, and both paths forward Original's stature message. Saves 000 and
001 subsequently match every recorded frame. The other five all pass their
startup door action and reach independent later divergences between frames
2,192 and 9,271 (save 052 instead reaches its later RNG exhaustion at frame
4,060).

### Synchronous attentive look continuations

Several loaded saves resumed `LookLeft` (and potentially the following
`LookRight`/`LeanOut`) through a terminal owner condolence card. Original
`SetState -> SendCondolationCard -> Ready -> Go -> Instruct` remains on the
same stack, but Rust's synchronous owner dispatcher supported attentive-mode
entry/exit commands without the look commands translated by the same ordinary
sequence path.

The synchronous route now delegates `LookLeft`, `LookRight`, and `LeanOut` to
`NpcAttentionCommandContext`, including the live attentive-state choice
between ordinary and alerted animation rows. Linux3 Profile 003 saves 022,
030, 031, 045, 056, and 061 all pass their former unsupported-command
boundary and reach independent later divergences. Save 064 advances from
frame 5,992 to 6,048.

### Overview timer starts after the right look

Saves 022, 030, 056, and 061 next converged on soldier 21 changing from
`AttackingOverviewLookRight` (Original substate 159) to
`AttackingApproachToObserve` (Rust substate 166) while the alerted right-look
animation was still running. Rust armed `AI_END_OVERVIEW_TIME` when the
left-look `EVENT_DONE` selected and launched the right look. Original
`RHArtificialMalignity` only changes substate and calls
`LookSidewards(LOOK_RIGHT)` there; it starts a 10-frame timer when the
right-look itself reports `EVENT_DONE`.

The premature left-look timer has been removed. Save 030, save 061, and the
previously longer save 064 now match every recorded frame. Saves 022 and 056
pass the AI transition and reach unrelated RNG-boundary divergences at frames
2,268 and 555 respectively.

### Large-window compressed trace input

The expanded Linux2/Linux3 corpus includes single-frame zstd streams whose
declared window follows the total uncompressed JSONL size and exceeds the
decoder library's 128 MiB default. These are valid recorder outputs, not
oversized in-memory replay records. The streaming trace reader now opts into
zstd's bounded platform maximum window log (31 on 64-bit, 30 on 32-bit), while
retaining line-by-line decoding and all JSON record limits. A synthetic
256 MiB-window regression demonstrates both the default rejection and the
parity reader's exact output.

### Linux-v48 timer sequence states

`RHSequenceElement::Go` dispatches an ownerless `RHCOMMAND_TIMER` directly to
`RHEngine::PerformExecuteCommand`. That command appends the element to
`mlistTimerElements` but does not transition it from `RHSEQ_TODO` to
`RHSEQ_INPROGRESS`; a live timer is therefore commonly serialized as TODO.
Rust accepts TODO and INPROGRESS only for Timer commands referenced by the
active timer list. Null references, wrong commands, terminal states, and
missing integer Timer properties remain hard failures.

### Linux-v48 ambush array load side effects

The enemy-AI reader contains an authoritative historical asymmetry. Before
reading per-NPC ambush statuses it deletes the static shared
`marrayAmbushPoints`, but it does not delete the constructor-initialized
per-NPC `marrayAmbushPointStatus`; saved statuses are appended. Rust performs
the same global deletion once enemy AI is restored and appends each saved enum
to the initialized local vector. In particular, a zero-length saved list
preserves initialized local statuses while leaving the shared topology empty.

### Linux-v48 PC campaign-status identity

Original serializes `RHElementActorPC::mpDescription` as its own campaign-table
pointer and then restores `mpStatus` as an alias into that description.
`mubListIndex` is separate actor/UI storage and is never used for that lookup.
Rust already validates the saved description's character-profile identity
during PC adoption; all later status/ammunition access now resolves the
campaign description by that profile identity instead of interpreting
`PcData::list_index` as a campaign index.

### Linux-v48 historical patrol endpoint storage

`RHPath::SerializeStatus` restores `mubLastWaypointIndex` as a plain byte and
does not validate it against the currently authored path. Saves consequently
retain historical last-waypoint indices after a path topology changes or a
shorter path is selected. Rust preserves that byte exactly. The current
waypoint remains strictly range-checked because Original immediately uses it
to reconstruct the live patrol/macro cursor.

### Linux-v48 nullable shield selection state

The engine serializes `mbShieldProtected`, the danger point, and nullable
`mpShieldProtected` independently. The constructor initializes the pointer and
point but not the mode flag, and selecting Shield/BigShield explicitly enters
the pre-click state with mode enabled and a null protected PC. Rust now
preserves all flag/null combinations exactly without synthesizing a PC or
normalizing the flag. Non-finite danger points and non-null references to the
wrong entity class remain rejected.

### Loaded-campaign mission initialization

Before applying an embedded engine body, replay constructs the mission against
the recorded campaign. Initialization scripts can request `AddPCToGang` for
the exact VIP description already present in that campaign. Rust's gang list
was already deduplicated by character index, but a debug assertion ran before
that no-op check and aborted valid loaded-save setup. Re-adding the same
description is now explicitly idempotent; the duplicate-VIP-profile guard
still applies when a genuinely different description would be inserted.

### Restored `MOVE_OK` stop priority

`RHCOMMAND_MOVE_OK` is the internal translated form of `MOVE` and normally
retains the priority assigned before translation. Some v48 saves retain an
in-progress `MOVE_OK` with `RHPRIORITY_NOT_YET_SET`. Original's base
`DeterminePriority` has no arm for this internal command: its release-build
default yields `RHPRIORITY_NONE`, and `RHSequenceElement::Stop` immediately
promotes that result to Normal. Rust models this explicit fallback for
`MOVE_OK` while retaining diagnostics for every other unhandled command.

### Automatic divergence dump location

Default replay failures retain the configured rolling frame window as JSONL
under `.codex-tmp/parity-dumps/`, with a unique filename containing the first
divergent frame. The diagnostic no longer uses the host temporary directory,
so full-corpus work stays inside the workspace and concurrent replays do not
clobber a shared path.

### Linux-v48 gameplay posture restoration

Original stores an actor's posture in `RHPositionInterface`; Rust additionally
keeps a gameplay-facing copy in `ElementData`. Loaded-save adoption previously
restored only the position-interface field, leaving the gameplay copy at its
mission-initialized value. That split changed visibility events and AI state,
which in turn changed the global RNG draw order several frames later.

Adoption now synchronizes both Rust posture representations directly from the
serialized posture for every entity kind. This is restoration of one Original
field into Rust's two representations, not a runtime transition: it therefore
bypasses the normal posture transition guards while the detached save candidate
is installed.

### Linux-v48 script-global restoration

Original owns one indexed engine-wide script-global array, used by
`InitGlobal`, `SetGlobal`, and `GetGlobal`. Rust currently also has a
mission-native map for those calls. Loaded-save adoption restored the indexed
array but left the native map at mission-initialization values, so a completed
one-shot condition could run again after loading.

Adoption now rebuilds the native map from every serialized array slot. This
also preserves Original's validity rule: zero-filled slack slots inside the
serialized array remain valid global IDs rather than becoming absent map
entries.

### Synchronous mission-start patrol topology

`RHArtificialMalignity::InitOneAI` transforms an authored patrol chief's
soldier indices into live members and runs `InitializePatrol` before
evaluating the chief's initial state. That initialization immediately writes
the chief pointer into every admitted minion. The write is observable while
the remaining NPCs are still being initialized: a later minion with its own
hiking path returns to its chief instead of starting that independent route.

Rust previously retained the authored patrol list but deferred its first
admission/sorting pass and the minion-chief links until the first simulation
tick. Besides starting the wrong movement, that made affected minions execute
path macro selection during setup and consume extra global RNG draws.
Mission bootstrap now performs the Original admission gates, 3-D distance
ordering, left/right pair arrangement, and chief-link writes synchronously.
Ordinary runtime patrol refresh remains owner-ticked. Linux3 Profile 003
saves 028, 029, 054, and 063 all consume the exact recorded mission-start RNG
prefix; saves 028, 029, and 063 then match every recorded frame.

### Deferred movement-replacement goal ownership

An AI movement replacement may be registered before the outgoing actor's
creation slot and instructed after it. If the outgoing movement reaches an
intermediate waypoint in that slot, `PerformMotion`/`DoNextOrder` leaves the
sprite goal at the newly selected waypoint. Original then selects the pending
replacement before interrupting the old movement, so the old condolence card
does not own—and therefore does not clear or restore—the sprite goal.

Rust carried a queue-time snapshot of the outgoing goal into the replacement
to compensate for an earlier eager-cleanup difference. It unconditionally
restored that stale snapshot when the A* request became `MoveWaiting`, erasing
any waypoint advancement between registration and instruction. Replacement
handoff now uses the snapshot only when eager cleanup actually left a zero
goal; a nonzero live goal remains authoritative until the new path installs
its first concrete order. This is the general selected-element ownership rule,
independent of patrols or replay identities. Linux3 Profile 003 save 054 now
passes its former frame-312 goal divergence.

### Restoring in-progress ability ownership from loaded sequences

Original does not serialize a separate hero-ability controller. Its selected
in-progress sequence element and current `RHOrder` remain the authority after a
load, and the actor resumes executing that order directly. Rust normally
creates an `ActiveAbility` latch while translating a newly launched command,
so Linux saves taken after translation previously restored the animation and
sequence but lost the Rust-only latch. The animation then continued without
its completion effect; for example, a saved `TYING` order finished without
changing its antagonist from lying to tied.

Linux-v48 post-load adoption now derives the latch from each current recognized
ability order, retaining its sequence/element/order identity and antagonist.
This covers the ordinary one-shot abilities as well as the phase-specific
Listen and ReceivePurse order chains. It is a reconstruction of Rust-only
bookkeeping from Original's authoritative state, not new serialized state.

Ability termination also now closes `SendCondolationCard` synchronously for
every ability. Original clears the actor's selected sequence/order at that
boundary, making `GetCommand()` report `WAIT`, and may instruct a successor
before returning. Rust previously performed that owner-boundary dispatch only
for Strangle. Tie also repeats Original's antagonist-validity check on every
Execute: its own DONE effect changes the target from lying to tied, so the next
Execute aborts and releases the now-invalid Tie order rather than playing its
unused animation tail. Linux3 Profile 003 save 011 exercises these fixes with
a tie that was already in progress when the save was written.

### Profile-derived beam-me capacity in loaded-save replay

The standalone parity runner loaded `profile.cpf` directly but omitted the
normal game bootstrap's `ProfileManager::import_beam_mes` enrichment pass.
Consequently every mission reported a deployment capacity of zero. In
Sherwood's deployment-zone script, even an empty mission team then satisfied
`GetSizeOfMissionTeam() >= GetNumberOfBeamMes()`, incorrectly sending an
entering PC away and consuming a `Rand` draw.

The runner now imports beam-me metadata from `Data/Levels` before restoring the
campaign, exactly like the ordinary game startup path. Linux3 Profile 003 save
056 consequently matches every recorded frame.

### Shipping sequence transitions remain replayable in debug builds

`RHSequenceElement::SetState` writes the new state before validating the old
state in its switch. For a `TERMINATED` transition from an already terminal
state, the Original debug build asserts, while the shipped build used to
record the Linux traces simply keeps the state assignment and performs no
duplicate owner, sequence-ready, postponed, or cascade effects.

Rust previously used a live `debug_assert!` for that validation. Loaded
shipping saves can resume at exactly this edge, so debug parity replay aborted
even though the authoritative executable continued. Rust now retains the
Original release behavior and emits a structural warning containing sequence,
element, and old-state identity. Linux2 Profile 002 `Restart-session-0002`
advances past its two former immediate panics and reaches ordinary logical
comparison.

### Script zones follow crossed lines, not a frame-wide polygon scan

Ordinary script-zone changes in Original are owned by
`RHElementActor::CheckForLineCrossing` (`original-code/RHelementactor.cpp`).
After removing boundaries on which the actor's old position lies, it orders
the remaining non-elevation lines by intersection distance. Each crossed
`LINE_SCRIPT` then tests its associated sector polygon at the new position and
calls `Enter` or `Leave`. Those calls update the occupant list, invoke the zone
script, recursively process a PC's carried actor, and finally update the PC's
production work icon.

Rust previously reconciled every actor against every zone polygon once per
frame. Besides changing callback ownership and ordering, that could manufacture
an enter event when an actor merely moved within a concave polygon's bounding
region without crossing one of its boundary lines. The resulting script call
also consumed global RNG that Original did not draw.

Runtime movement now registers active script-sector edges in the fast grid and
dispatches the exact crossed edges from both ordinary and delayed-position
movement. It preserves Original's old-position rejection, distance order,
per-line callbacks, carried recursion order, and production-icon timing. The
global reconciliation pass has been removed.

Polygon-wide membership checks remain only where Original explicitly uses
`IsReallyInside`: initial/silent occupant reconstruction and
`UpdateScriptSectorsAfterFlight`. The flight path reconciles only the landed
actor and includes Original's layer and owning motion-sector checks. Linux2
Profile 002 saves 017, 022, and 032 match every recorded frame after this
change; Linux3 Profile 003 save 056 is an additional full-trace control.

### Script action queries preserve the deferred wait-order handoff

Original `GetCurrentAction` returns `RHElementActor::GetAnimation`, which reads
the actor's installed `mpOrder`. The sequence manager can select a succeeding
`WAIT`, `WAIT_TIMER`, or `WAIT_FREE_LIFT` element at the end of a frame, but
that wait order does not replace `mpOrder` until the actor's next
`Hourglass`. Other newly selected actions are already visible through
`mpOrder`; this asymmetry matters to scripts that test an exact animation.

Rust previously returned the selected sequence element's front order
unconditionally. During the one-frame wait handoff it therefore exposed
`WAITING_SWORD` too early. Sherwood's sword-training script interpreted the
trainer as ready, selected a random zone occupant, and consumed a global RNG
draw absent from the Original.

`GetCurrentAction` now keeps returning the sprite's actually processed
animation while one of those three wait commands is selected but not yet
executed. Linux3 Profile 001 save 004 consequently matches every recorded
frame, including its combat-animation RNG at frame 1950.

### Wasp movement is a serialized 3-D vector

`RHElementWasp::Serialize` writes `mvtMovement` before deliberately invoking
the `RHElementObject` parent serializer. Although most of the wasp's steering
logic operates in map space, the member is an `SBGeoVector3D`, whose serializer
writes all three floating-point components.

The Linux-v48 decoder previously treated that member as a 2-D point. Active
wasps therefore shifted the following object payload by four bytes, making the
movement Z component appear to be the first bytes of the
`RHElementObject` fingerprint. The decoder now consumes the full 3-D vector,
matching both the declared Original member type and its serializer. This is a
save-format correction for every active wasp, independent of replay content.

### Patrol reacquisition uses the configured real view radius

`RHElementActorNPC::IsDetecting360Degrees` compares against
`mViewParameters.uwRealRadius`. This is the configured/base range, not the
current cone radius, which can temporarily grow while the NPC is alert.

Rust's patrol initialization and missed-member reacquisition passed the
animated cone radius into the otherwise actor-accurate 3-D visibility query.
In Linux3 Profile 003 save 011, an officer whose current cone had grown from
300 to 420 therefore readmitted a separated member before the Original did.
The member received a formation command one refresh cycle early, changing its
movement lifecycle at frame 22,640.

Both patrol admission paths now use the NPC's base/real radius while retaining
the shared posture, rider-height, building, and opaque-ray checks. This is the
same field selected by the Original overload and applies to every patrol.

### Deferred turns do not overwrite a newer live movement goal

`FaceTo` halts a selected movement before launching its turn sequence.
Original `RHSequenceElementMovement::StopMovement` converts the current order
to a stop transition while leaving the actor's live sprite goal available to
the remaining owner execution. That owner slot can advance the goal after the
turn has been registered but before the sequence manager instructs it.

Rust retains the outgoing goal because its eager halt cleanup can clear the
sprite before a deferred turn is instructed. The deferred instruction
previously restored that retained value unconditionally. In Linux3 Profile 003
save 011 this replaced Soldier 97's newly advanced formation goal at frame
22,720 with the preceding waypoint.

Deferred turn instruction now restores the retained goal only when cleanup
actually left the live goal at zero. A goal advanced by the outgoing actor slot
remains authoritative, matching the Original `StopMovement` ordering for every
deferred `FaceTo`.

### Pre-bound actor VMs keep their initialized beam-slot class

Beam-me assignment can place the same saved character in a different authored
slot while loading a campaign. PC identity adoption therefore maps saved and
initialized actors by campaign-description/profile identity rather than
incidental construction order. The authored per-slot script classes can differ
after that remap.

This is legal in the Original. `RHElementActor::Serialize` reads the saved
`mstrScriptClass`, but calls `Bind` only when `mbScriptInitialized` is false.
Mission-created beam-me PCs were already bound by
`InitializeScriptFromStream`, so their live VM remains bound to the newly
assigned slot class. The save's member payload is then deserialized through
that live binding. Linux3 Profile 003 save 046 demonstrates this with saved
`hidden_pc02_800000b5` and initialized `hidden_pc03_800000b6`; both expose the
same `deja_fait` member schema.

Actor VM adoption now reproduces that distinction: class names may differ for
an already initialized actor, but member count, names, types, heap ranges, and
referenced values remain strictly validated against the live class. Target and
scroll VMs retain exact class-name validation because they do not pass through
the actor serializer's `mbScriptInitialized` guard.

### Seek-point sideward looks complete from their sequence event

Original's sideward-look sequence owns its own completion. The sprite reaches
the action point, emits `EVENT_DONE`, and the selected sequence advances from
that event. There is no second actor timer which independently terminates the
look.

Rust also armed a fixed-duration `stare_remaining` timer for these seek-point
looks. It could expire before the sequence's animation event, interrupt the
live look, and let AI start unrelated work such as leaving a building. That
changed both sequence ownership and later RNG consumption. Seek-point
sideward looks now rely exclusively on their sequence event, matching the
Original completion path. Linux2 Profile 002 save 000 consequently matches
every recorded frame.

### Actor seek sectors retain door identities

`RHElementActor::mpSeekSector` is an `RHSector*`, not specifically a motion
sector. A point seek whose goal is a door stores the corresponding
`RHSectorDoor`; the pointer remains serialized even when another seek mode
currently makes it dormant.

Rust's Linux-v48 adoption previously resolved this field only through the
compact motion/building-sector table and rejected a valid sparse door-sector
slot. The retained Original topology now maps every sparse door-sector
constructor identity through the isomorphic gate order, and actor continuation
state represents a seek sector as either a position sector or a door. Linux3
Profile 003 save 055 now adopts successfully and reaches ordinary frame
comparison instead of failing on sparse sector 416.

### Frame comparison maps every sparse position-sector identity

`RHElement::GetSectorNumber()` exposes the sector object's slot in Original's
heterogeneous `RHFastFindGrid::marraySectors`. Rust stores the corresponding
motion/building sector in a compact canonical registry. Those numbers can
coincide, but equality is not their identity contract.

The parity runner previously inferred only differing building-sector pairs
from inactive hidden occupants. That made the mapping depend on mutable frame
state: Linux3 Profile 003 QuickSave begins with Civilian 74 active inside door
5, where Original sparse slot 303 is Rust canonical building sector 146. The
first recorded movement frame therefore reported a false sector divergence
before an inactive occupant could teach the runner the pair.

Frame comparison now builds the complete sparse-to-canonical sector
isomorphism from the retained Original construction topology. It validates
that the mapping is one-to-one and treats hidden occupants as consistency
checks instead of identity evidence. QuickSave passes frame 74697 and reaches
the next unrelated command divergence at frame 74701; Linux2 Profile 002 saves
011 and 013 and Linux3 Profile 003 save 064 remain fully matched.

### Scripted NPC bow shots do not use the PC empty-quiver gate

Linux3 Profile 003 QuickSave's mission script locks Soldier 245 and launches a
two-element `ShootBowOnce`/`UnlockAI` sequence at Target 257. The saved
soldier's live arrow counter is zero. Original still accepts the shot:
`RHElementActorHuman::CanShootWithBowAt` rejects zero ammunition only when
`IsPC()`, and the release build's later NPC decrement saturates at zero.

Rust had copied the empty-quiver rejection into both generic bow-target
validation and sequence dispatch without the `IsPC()` condition. The scripted
sequence was therefore allocated correctly and immediately marked impossible,
leaving the actor on `Wait` at frame 74701. Both gates now apply only to PCs;
ordinary NPC AI still uses its own remaining-arrow decisions, while authored
NPC shots retain Original semantics. A trace-level sequence-launch record now
prints every launched element's owner, command level, state, priority, and
typed data, making this class of transient rejection observable without adding
replay-specific instrumentation.

QuickSave now matches all 250 recorded frames. Linux2 Profile 002 saves 011 and
013 and Linux3 Profile 003 save 064 remain fully matched.

### Loaded target hotspots retain the serialized sprite position

`RHSprite::GetCurrentPointMap` adds the current row's hotspot to the cached
integer `mposPositionSprite`. That cache is serialized independently from the
target's 3-D position and action `PositionMap`. Reconstructing it later as
`floor(position.to_map() - center)` is not equivalent: binary32 projection can
land infinitesimally below an integer even when the serialized cache contains
that integer.

Linux3 Profile 003 save 055 exposed this at its final target interaction. The
saved target sprite top-left was `(2791,171)`, while reconstruction produced
`(2791,170)` from the same 3-D values. `HANDLING_TARGET` therefore faced a
hotspot one pixel too high and selected direction 14 instead of 13. Rust now
marks an exact sprite-position cache when restoring a v48 payload, seeds the
same cache during ordinary mission target placement, and uses it for FX-target
hotspot queries. Position changes invalidate the cache; the target-specific
action-point overwrite deliberately preserves it, matching the Original's
split placement. Save 055 now matches every recorded frame.

### Destination forecasts require the live C++ door pointer

`RHElementActorHuman::ForecastDestinationForIA` first reads `GetDoor()`. It
uses the current position, sector, and layer without any building-exit RNG
when that pointer is null, even if other door-passage bookkeeping has not yet
been retired.

Rust built forecast input from its higher-level `active_door_pass` mirror
alone. Linux3 Profile 003 Restart contains a civilian whose saved passage
mirror still names building door 13 while the authoritative serialized
position-interface door is null. Merely constructing a global AI snapshot
therefore selected a random alternate building gate one frame after the
Original had stopped doing so. Forecast extraction now exposes a door pass
only while the position interface has a live door, including the duplicated
camp-soldier snapshot path. A focused regression preserves both the null-door
fallback and the live-door forecast case.

### Battle friend lists require owner-relative 360-degree detection

`RHArtificialMalignity::BattleDecisions` scans the fighter registry, but it
adds a friendly human to `mlistUs` only when the evaluating soldier's
`IsDetecting360Degrees` query succeeds. This is the same posture-aware,
three-dimensional view-radius and opaque-line-of-sight test used elsewhere;
the broad 500-unit combat-neighborhood query is not a substitute.

Rust previously rebuilt this list from every able friendly fighter in that
broad neighborhood. Linux3 Profile 001 save 014 consequently counted two
hidden soldiers, including an officer. Their points and officer bonus changed
the battle predecision, skipped the Original courage RNG draw, and selected
`TowerGuardAlert` instead of `LookForHelp`.

Per-owner fighter snapshots now carry the exact 360-degree detection result.
Both snapshot construction paths use it when building friend lists and battle
aggregates, and `BattleDecisions` enforces it when rebuilding the persistent
list. Save 014 advances from frame 17481 to frame 17636, where it reaches a
later independent wait/timer divergence. The other five traces in the same
initial RNG-failure cluster also advance to distinct later behavior or RNG
sites, confirming that the shared battle-list cause is removed.

### Patrol regrouping uses the virtual combat predicate and full 360-degree sight

`ReturnToDutyCommonStuff` only sends a patrol member to its chief when the
chief's virtual `IsAbleToFight()` succeeds and `IsDetecting360Degrees(chief)`
can see the chief. The latter is not a radius-only convenience check: both
humans must be active outside buildings, their posture-aware eye/detection
points must fit inside the NPC's live 3-D view radius, and opaque line of sight
must be clear.

Rust previously approximated this branch with a flat distance to the chief.
Its shared entity snapshot also treated every living civilian as able to
fight, although civilians inherit the Original base-human implementation that
always returns false. Linux3 Profile 003 Restart consequently diverted an
entire civilian-led patrol toward its chief instead of resuming route 17.
Entity views now preserve the virtual hierarchy semantics (including the PC
Tree/Spy exclusions), and patrol regrouping uses the same posture-aware,
opaque-LOS 360-degree query as the other synchronous AI callers. Restart now
passes the patrol transition and advances from frame 121 to the next unrelated
movement divergence at frame 193.

### Static repulsion keeps the motion sector's oriented boundary normal

`RHRepulsiveLine::InitializeNormal` deliberately orients an AREA motion
sector's boundary normal opposite to a solid obstacle's boundary normal.
Rust's canonical `GridLine` retained that distinction, but the anti-collision
gather rebuilt a new repulsive line from endpoints and thereby assigned every
boundary the solid-obstacle orientation.

Linux3 Profile 003 Restart exposed the loss when Soldier 75 recovered from a
deviation beside an AREA corner. The reversed normal made the boundary
one-sided in the wrong direction, so Rust resumed the straight route while
the Original continued around the corner. Level repulsive-line conversion now
copies the already-oriented grid normal and records whether it is the AREA
orientation. A focused geometry test covers both orientations. Restart now
passes frame 193 and advances to a separate patrol-route transition at frame
240.

### Release posture transitions reject unsupported tied commands

The Original `MakePostureTransition` switch has debug assertions in its
unhandled-posture arms, but its release behavior returns `false`. A loaded
sequence may legitimately retain an upright-only command such as `RaiseBow`
for an actor whose saved posture is already `Tied`; that command is rejected
without generating transition orders and the simulation continues.

Rust previously converted those debug assertions into unconditional panics.
Both the upright and crouched transition checks now preserve the Original
release result for unsupported postures, with a debug diagnostic instead of
inventing an order or aborting the replay. Linux3 Profile 001 save 018,
including its loaded tied soldier, now matches every recorded frame.

### Loaded Hourglass traversal follows Original creation identity

`RHEngine::SerializeElements` sorts its compact element array by
`mulCreationOrder` before writing a save. The loaded array retains that order,
and `PerformHourglass` walks it directly. Rust deliberately retains the
initialized mission's sparse entity identities during save adoption, so
numeric Rust slots need not have the saved array order.

The actor/non-actor owner walk now sorts live entities by the restored Original
creation identity. It also mirrors the mutable compact-array loop: removals
compact immediately, while elements constructed during callbacks join the
monotonic creation-order tail. A focused regression installs a saved order
which differs from Rust slot order and verifies the production walk. Linux2
Profile 002 save 038 consequently assigns simultaneous smalltalk initiatives
to the same PCs as the Original and advances beyond frame 5615.

PC strike remarks now close at the same owner boundary. The Original PC
`Execute` override evaluates its eventual-remark RNG immediately after the
wrapped human strike `Execute` returns `START`; Rust previously scanned all PCs
in the later global melee tail. Both generic and active-melee START edges now
settle the owning PC before the next Original creation slot, while the global
tail remains responsible for DONE-edge arrow-extraction remarks.

### Fighter snapshots preserve authoritative position sectors

Original combat code passes complete `RHposition` values through fighter
pointers. That includes the sector pointer, not only projected XY and layer.
Rust's per-owner fighter snapshot deliberately replaced the sector of every
live soldier and PC with `None`. A derived destination could therefore have
the exact Original coordinates while failing `GoTo` immediately as a null
sector.

The snapshot builder now carries each entity's live sector. Linux2 Profile 002
save 003 exposed this when an archer derived its cover point from a stationary
shield bearer in sector 18: the Original launched the run-behind-shield
movement, while Rust self-dispatched `EVENT_COULDNT_REACHPOINT`. The replay now
passes that formation decision at frame 13786.

### Raising a shield faces the danger point at animation initialization

`RHElementActorHuman::Execute(RHANIMATION_RAISING_SHIELD)` installs
`RHFIELD_SHIELD_DANGER_POINT` as the direction goal on the first execution of
the raising order and then calls `Turn()`. This is intentionally later than
command dispatch because posture/action exit transitions may precede the
raising order.

Rust now applies the stored shield face point at that same animation-owner
boundary for every human, including NPC soldiers. Applying it when the command
was dispatched changed direction too early; omitting it for NPCs left a shield
bearer one sector away once raising began. Linux2 Profile 002 save 003 now
passes the direction transition at frame 13795 and advances to a later
command-timing difference at frame 13814.

Shield-maintenance timers use a different Original operation:
`RHElement::SetDirection` updates the progressive goal and calls
`UpdateShield` without launching a `Turn` sequence. Rust now represents this
direct write explicitly and uses it for both a lone shield bearer protecting
an archer and an established phalanx. Save 003 consequently keeps the
`WaitingShield` command at frame 13814 and advances to an independent door
movement difference at frame 13821.

### Manager-instructed loaded elements resolve owner priority

`RHElementActor::Instruct` calls the virtual `DeterminePriority` chain when an
element still carries `RHPRIORITY_NOT_YET_SET`. Eager Rust launch wrappers
already did this, but a successor in a prebuilt or loaded sequence can arrive
directly from the sequence-manager Hourglass.

The ordered `InstructOwner` boundary now resolves those elements too, before
the non-interruptible guard and ordinary arbitration. This restores the
Original `PASS_DOOR` priority: an AI reaction Move issued while a door pass is
active is postponed rather than interrupting the traversal. Linux2 Profile
002 save 003 now passes frame 13821 and advances to an independent
movement-goal difference at frame 13828.

### A prepared sequence shape does not universally close the special-strike state

`WaitTimer -> swordstrike` identifies an AI-prepared strike, but it does not by
itself imply a state change when the successor is instructed. The previously
observed Thrust A handoff also runs that command's unique reciprocal
principal-opponent normalization. Other strike commands do not inherit that
side effect.

Rust previously applied the Thrust A state handoff to every prepared strike.
Linux3 Profile 001 Continue consequently changed Soldier 110 from
`AttackingSwordfightSpecialStrike` to ordinary `AttackingSwordfight` when its
prepared Thrust D began at frame 518. The handoff is now restricted to the
source-backed Thrust A path; Thrust D remains special through its own
completion, and the replay advances to a later independent combat boundary.

### `GoNear` uses Euclidean squared distance for its immediate reach test

The Original has two distinct early-arrival predicates inside `GoTo`: ordinary
movement uses `MaxNorm() < 5`, while `GOTO_NEAR` uses
`SquareNorm() <= tolerance * tolerance` on the same layer. Rust reused the
ordinary MaxNorm predicate with the near tolerance, accepting diagonal points
outside the requested circle.

Linux3 Profile 001 Continue exposed this at frame 554. Soldier 114 was within
50 units on each axis but about 66 units away in Euclidean distance; Rust
stopped its run and entered swordfight while the Original continued `MoveOk`
in `AttackingRunningToEnemy`. The near fast path now uses the exact circular
predicate without changing ordinary `GoTo`.

### Invulnerability restores literal 100 life points

`RHElementActorHuman::SetLifePoints` stores the literal value 100 whenever the
human is invulnerable. It does not use the actor profile's maximum life or its
difficulty-scaled cached equivalent. Rust's shared combat setter instead
restored that maximum, which only happened to agree for ordinary 100-HP actors.

Linux3 Profile 001 Savegame 011 exposed the distinction at frame 289: an
invulnerable civilian whose profile maximum is 120 received a zero-damage
sword message. The Original restored life from 100 to 100, while Rust raised it
to 120. All shared damage paths now retain the Original literal, including
actors whose authored maximum differs.

### Return-to-post completion writes posture instead of relaunching the action

`GoTo(..., GOTO_SPECIAL_ACTION)` already appends the return-to-post Turn and
SitDown/EnterLeisure commands. When their final `EVENT_DONE` reaches
`SUBSTATE_DEFAULT_GOTOPOST_TURN`, the Original directly calls `SetPosture` with
Sitting or Leisure and enters `DEFAULT_ONPOST`.

Rust instead launched another SitDown/EnterLeisure command from that completion
callback. Linux3 Profile 001 Savegame 009 exposed the duplicate at frame 12986:
the Original civilian selected its ordinary Wait after sitting, while Rust
began a redundant upright-to-sitting sequence. The callback now performs the
literal posture write. That replay passes the return-to-post boundary and
advances another 117 frames to an independent RNG-consumption difference.

### Loaded actors preserve the overloaded seek/wait countdown

The Original stores ordinary command waits and seek-refresh aging in the same
unsigned `RHElementActor::mulWaitTime` field. Rust deliberately separates those
responsibilities, but Linux-v48 adoption previously restored the serialized
scalar only into `wait_time`. A seek already active in a save therefore aged a
fresh `seek_refresh_wait`, while the stale serialized value reappeared after
the post-seek interaction took over.

Adoption now seeds both split candidates from the authoritative saved scalar,
and every seek-refresh decrement mirrors its wrapped value into the legacy
wait copy. The live command still selects which Rust counter drives behavior;
the mirror only preserves the Original scalar across every possible seek exit.
A live `WAIT_TIMER` likewise owns the isomorphic scalar even if the actor
retains a seek target and post-seek continuation; the dormant seek-refresh
copy must not hide the timer decremented by `Actor::Hourglass`. Linux3 Profile
001 Savegame 002 exposed that projection-only mismatch on its first frame.
A successful `StartPostSeekSequence` also folds the seek copy back before
discarding its ownership markers. Linux2 QuickSave consequently advances from
the first exposed mismatch at frame 1927 to an independent path/command
boundary at frame 1968, while Save 024 advances from frame 32482 to an
independent interaction boundary at frame 32717.

### Patrol dissolution returns members to duty synchronously

The Original script native calls `RHArtificialIntelligence::ClearPatrol`
directly (`RHScript.cpp:8557`, `RHartificialintelligence.cpp:7502`).
`ClearPatrol` clears each theoretical member's chief pointer and immediately
calls `ForceReturnToDuty` for members in the default AI state. Those nested
Think calls can register replacement movement before the next sequence-manager
pass.

Rust previously approximated each nested call with a deferred self-stimulus.
Because those stimuli drained after the sequence manager, every replacement
route was instructed one simulation frame late. Linux3 Profile 003 Restart
exposed the ordering difference at frame 240 when a six-member patrol was
dissolved. `RemoveAllSubordinates` now crosses an engine-owned script barrier:
it clears the patrol in Original order and synchronously dispatches each
required return-to-duty stimulus with live engine context. Ordinary sequence
instruction remains with the following manager phase. The complete Restart
recording now matches every recorded frame.

### Retained human stimuli rebuild their live antagonist context

The Original retains an entire `RHStimulus`, including its human pointer, while
an NPC is busy or script-locked. When the lock clears, handlers such as the
civilian `EventViewStandardProcedure` therefore inspect that human's current
camp, position, PC identity, and combat state.

Rust preserved the typed human handle and rebuilt target-specific tick data,
but its retained-stimulus path left `AiContext::antagonist` empty. The accepted
handler then returned without reacting. Linux3 Profile 003 Continue exposed
this at frame 13539: a civilian's queued view of a friendly hero should have
interrupted its route with the admiring-hero turn. Retained human stimuli now
resolve the handle against the live entity view and populate the same
antagonist context used by immediate detection dispatch. The complete Continue
recording now matches every frame.

### Heal facing begins at the first Execute boundary

Selecting an `RHCOMMAND_HEAL` element and executing its Healing order are
separate Original phases. Selection installs the order without changing the
healer's facing. On the actor's next live slot,
`RHElementActorPC::Execute(RHANIMATION_HEALING)` validates the interaction,
sets the direction goal from the target's live map position, calls `Turn`, and
then advances the animation.

Rust previously faced the target immediately in `begin_heal`, one frame before
the selected order could execute. Linux3 Profile 003 ExQuickSave exposed the
early write at frame 74491. Heal selection now preserves the existing facing;
first-Execute initialization installs the goal and ordinary Heal ticks call
`Turn` without freezing animation progress, matching the Original. The
complete ExQuickSave recording now matches every frame.

### Sight planes preserve the Original binary32 equation

`SBGeoPlane3D` normalizes the cross-product of its three plane points, derives
cached `AZ/BZ/DZ` coefficients, and evaluates `x*AZ + y*BZ + DZ`
(`sb3dstuff.cpp:194-247`). Rust's sight path previously used an algebraically
equivalent unnormalized relative-point equation. The two expressions are not
binary32-equivalent at plane boundaries.

Linux3 Profile 001 Savegame 034 exposed this at frame 28862. A bonus-to-PC ray
started exactly on the large sloped top plane of obstacle 90; the simplified
equation rounded the plane one ULP above the origin, falsely blocked the ray,
and delayed bonus discovery. World-space sight queries now reuse the cached
coefficient construction that preserves the Original SSE2 `FLOAT` operation
sequence and evaluate it in the Original multiplication/addition order.

### Deferred SetState callbacks preserve synchronous caller-tail state

Original Enemy/Friendly `SetState` calls `FilterAIEvent` synchronously, commits
the incoming state, and then returns to its caller. Rust defers that script
callback through an owner-local barrier while the pure-Rust caller continues.
The barrier already isolated ordered effects, but it could later recommit the
earlier incoming state over a newer direct state assignment made after
`SetState` returned.

Linux3 Profile 001 Savegame 034 exposed this when a restored patrol macro had
its serialized in-progress bit set and zero bytes remaining. The timer handler
entered `DefaultInMacro`, then `ExecuteNextMacroCommand` synchronously completed
the empty macro and entered `DefaultEnroute`. The deferred inherited
`FilterAIEvent` transaction subsequently rewound the NPC to `DefaultInMacro`,
eventually consuming an extra bored-animation RNG draw. The barrier now
captures the caller-tail canonical state before rewinding for the callback,
performs the outgoing-to-incoming transaction, then reapplies any newer
caller-tail pair. Savegame 034 now matches every recorded frame; the already
green Profile 003 Restart replay remains exact.

### Full swordfight exit preserves survivor principal ordering

`RHElementActorHuman::QuitSwordFight` removes the quitter from each opponent by
calling `DeleteOpponent`, then clears the quitter's own list. `DeleteOpponent`
recomputes relative fighting ability and smalltalk initiative, but a full exit
does not call `EvaluateOpponents` for survivors and therefore does not randomly
choose a new principal opponent.

Rust had added a principal-opponent reshuffle whenever a survivor retained two
or more opponents. Besides changing list order, even a single face-cone
candidate consumed an authoritative RNG draw. Linux3 Profile 003 Savegame 006
exposed that extra draw at frame 6264 when one PC exited a three-on-one fight.
Full swordfight exit now retains each survivor list's first remaining entry and
performs only the `DeleteOpponent` strength/initiative consequences. The replay
advances to an independent command-order divergence at frame 6275.

### Zero-opponent evaluation preserves pending movement

`RHElementActorHuman::EvaluateOpponents` treats the actor's selected movement
element as mutable authoritative work. When the last opponent disappears, an
active `WALKING_WITH_SWORD` or `RUNNING_WITH_SWORD` action is rewritten to its
upright equivalent before `QUIT_SWORDFIGHT` is launched. The sequence manager
postpones that same movement while the sword is lowered, then translates it
again without losing the destination or movement flags.

Rust previously left the movement's authored sword action unchanged. After
lowering, retranslating it immediately discovered an empty opponent list and
launched another quit instead of resuming the move. Linux3 Profile 003 Savegame
006 exposed this at frame 6275. Zero-opponent evaluation now performs the same
in-place action rewrite on the actor's current movement and rejects any
unexpected movement action as an invariant violation.

### Patrol direction turns wait for SequenceManager instruction

Every eighth frame a patrol chief synchronously calls
`CALL_PATROL_COORDINATE` and `GetInstructedPatrolDirection` on its members.
Those calls can construct a `FaceTo` sequence before a later member's actor
slot, but the Original only registers the new sequence element there. Its
separate SequenceManager hourglass instructs the owner after the element walk,
so the member's already-selected transition still receives that frame's
`Execute`.

Linux3 Profile 001 Savegame 035 exposed Rust promoting the direction turn
immediately at frame 50176. That restarted the member's walk-to-wait
transition one actor slot early and completed it at frame 50180 instead of
50181. Patrol direction delivery now uses the existing generic deferred
`FaceTo` instruction path: synchronous AI state and sequence registration stay
visible immediately, while translation and owner instruction occur at the
Original SequenceManager boundary. Savegame 035 now matches every recorded
frame.

### Attentive transitions postpone re-entrant facing turns

An AI state change can synchronously register an attentive-mode transition and
then deliver a re-entrant `EVENT_REACHPOINT` which registers `FaceTo`. Original
SequenceManager arbitration postpones that turn without translating it while
`ENTER_ATTENTIVE_MODE`, `LEAVE_ATTENTIVE_MODE`, or the officer variant is
selected or waiting to be instructed. The actor therefore retains its previous
direction goal until the attentive transition finishes.

Linux3 Profile 003 Savegame 009 exposed Rust eagerly translating the re-entrant
turn at frame 18075. Pending turn launch now checks both the selected element
and manager-registered elements waiting for instruction, and uses the generic
deferred untranslated turn path when an attentive transition owns the actor.

### Body queues inspect the complete entity snapshot

`ExamineOtherBodies` stores humans precisely because they are out of order. The
Original checks each queued body directly with `IsOutOfOrder`; it does not use
the nearby tactical-fighter list, which deliberately excludes actors unable to
fight. Linux3 Profile 003 Savegame 009 exposed Rust interpreting absence from
that filtered list as recovery and discarding a tied, unconscious guard at
frame 18078. Body pruning now resolves the handle through the complete entity
view and retains every actor that is still unable to fight.

### Anti-collision recovery rebuilds zero-distance trajectories

When a deviated actor can return to its authored trajectory, Original
`UpdatePositionAntiCollision` always clears deviation, commits the future
position, invalidates the cached increment, and calls
`ComputeIncrementAll(true)`. This also happens when the current animation frame
contributes a zero-length step. Linux3 Profile 003 Savegame 009 exposed Rust
leaving the temporary avoidance direction as its goal at frame 18085 because
the rebuild was incorrectly gated on nonzero displacement. Both movement paths
now rebuild unconditionally on deviation recovery.

### Seeking-body timers do not stand in for arrival

In `SUBSTATE_SEEKING_BODY`, `EVENT_TIMER` only checks whether the watched human
has recovered and become detectable; otherwise it rearms the ten-frame timer.
Body examination, dead-body observation, and waking a tied or unconscious
human belong exclusively to `EVENT_REACHPOINT`. Rust had merged the timer and
arrival arms, allowing a timer at frame 18088 of Linux3 Profile 003 Savegame
009 to wake the target before the movement transition actually completed. The
two event paths now follow the Original switch independently.

### Produced noise observes the Actor's live order pointer

`RHElementActor::Hourglass` latches `mpOrder` before `Execute`. If
`DoNextOrder` advances within the same sequence element, that pointer changes
to the next order immediately. If the element terminates,
`SendCondolationCard` clears both `mpSequenceElement` and `mpOrder`; an element
that is genuinely instructed synchronously then writes its first order back.
An element merely registered for the later sequence-manager pass remains
invisible to the subsequent `RHElementActorHuman::RefreshProducedNoise` call.

Linux2 Profile 002 Savegame 003 exercised all three boundaries. At frame
13843, `WaitingSword` evaluated a smalltalk parry while still executing. The
launch belongs to the later manager pass, so the Human tail remains silent;
eagerly resolving its wait priority routed it into Rust's synchronous queue
and emitted fight noise three frames early. At frame 13858 the parry
terminates and clears the pointer, so Rust's synthetic next-frame idle Wait
must likewise remain invisible and the noise must fall to zero. The nearby
guard then observes the next real noise edge at frame 13861. The fused owner
walk now derives Human-tail noise from the surviving same-element order or a
truly instructed successor, never from deferred registration or synthetic
idle bookkeeping.

### Script animation queries observe the Actor's post-slot order

The same `mpOrder` boundary is visible to script natives such as
`GetAnimation`. A sequence element can advance from a transition to its bored
wait order during `RHElementActor::Hourglass`; the new order is then
authoritative immediately, even though the sprite has not rendered it yet.
Conversely, when the actor tail derives no surviving order, `mpOrder` is null
and the deferred-wait compatibility path must retain the sprite action instead
of inventing an `Invalid` animation.

Rust now latches the derived actor-tail order as an explicit shadow of
`mpOrder->action`. Deferred wait animation queries use that shadow only while
it is valid, otherwise preserving the prior rendered action. Linux3 Profile
003 Savegame 013 consequently performs Sherwood's second
`MakeRandomBBQActions` draw at frame 7700 and matches through the end, while
Linux3 Profile 001 Savegame 004 and ExQuickSave retain their prior exact
results.

### PassDoor preserves Soldier runtime animation overrides

`RHElementActorSoldier::Execute` replaces a logical `WalkingUpright` order
with `WalkingAlerted` whenever the soldier is attentive. That virtual dispatch
still applies to movement steps created by PassDoor; the logical order itself
is not rewritten. Rust's door-pass driver used the translated step action
directly and bypassed the Soldier override, so Linux2 Profile 002 Savegame 003
moved Soldier 130 by the upright row's 1.2-unit sample instead of the alerted
row's 1.8-unit sample at frame 13890. Door steps now pass through the same
runtime Soldier animation selection as ordinary movement.

### Leaving Sleeping opens eyes at the SetState boundary

Enemy `SetState` calls the scripted state-change filter while the outgoing
state is still observable, then sets `EYES_LOOK_FORWARD` whenever the
transition leaves `STATE_SLEEPING`. Rust had placed that eye write in an
outbox drained only by concussion/recovery paths, so an ordinary noise event
could move a sleeping guard into Seeking while leaving its view permanently
closed. Eye restoration now uses the ordered owner-work queue immediately
after the state-change callback. In Linux2 Profile 002 Savegame 003 this lets
Soldier 130 acquire visibility after leaving its building and enter the
expected `AttackingReactiontimeTurning` state at frame 13904. The replay now
matches every recorded frame.

### Waiting-sword launches wait for the actor completion boundary

`RHElementActorHuman::Execute` evaluates smalltalk hints and ordinary
swordfight behavior while its current `WAIT_TIMER` order is still selected.
`LaunchSequenceElement` only registers the resulting parry, smalltalk strike,
or distance-adjustment move. The base actor hourglass then applies the
zero-timer completion, and SequenceManager instructs the newly registered
element in FIFO order. A prepared preference-priority strike can consequently
interrupt that wait-priority smalltalk element and synchronously deliver its
`EVENT_DONE` before the prepared strike is translated.

Rust previously ran the complete owned-element `Instruct` path inline from the
WaitingSword Execute callback. Against the still-selected normal-priority
timer, wait-priority smalltalk was abandoned as `Impossible`; Original first
accepted it after the timer completed and then interrupted it with the
prepared strike. Linux3 Profile 003 Savegame 010 exposed both forms: a
smalltalk strike at frame 18970 and a hinted parry at frame 19009 each return
the enemy AI from special-strike substate 161 to ordinary swordfight substate
160 through the interrupted element's `EVENT_DONE`. WaitingSword-created
elements now use a deferred owned registration path, and their manager work is
drained only after the actor completion stack settles. The full Savegame 010
trace matches every recorded frame while Profile 001's prepared lateral strike
still retains substate 161 when no such interrupted smalltalk element exists.

### Human instruction validates commands at their command-specific boundary

Original `RHElementActorHuman::Instruct` does not run the general sequence
validity predicate before delegating to `RHElementActor::Instruct`. Commands
which need a fresh position or target check perform it in their own first
`Execute` arm. WakeUp intentionally has no such check. Rust's generic
`InstructOwner` preflight rejected a valid WakeUp in Linux3 Profile 003
Savegame 009 before its authored turning and interaction orders could run.
The broad preflight is gone; the existing command-specific Execute checks
remain authoritative.

### Turning orders retain the direction resolved during translation

Turn, TurnFast, TurnElement, and WakeUp all establish the actor's direction
goal while translating the command. Their `Turning` order drives the actor
toward that goal; it does not recompute a direction from the order's generic
target coordinates. The removed Rust post-pass treated default `(0, 0)` order
coordinates as a map target and redirected a waking actor toward the origin.
All turn producers now rely on their translation-time goal, matching the
Original order contract.

### WakeUp delivers recovery AI synchronously

At `WakingUp DONE`, Original sets the target lying, clears concussion, calls
the target's `Wait`, and leaves the target's current action-state field for the
new order lifecycle to replace. Clearing concussion invokes the NPC's
`Think(EVENT_FITAGAIN)` inline, including resurrection bookkeeping, eye
changes, and wake redetection broadcasts, even if that NPC's creation-ordered
actor slot has already run. Rust now drains this exact FIFO prefix and applies
those inline side effects before launching Wait, while animation Execute still
obeys normal actor ordering. Linux3 Profile 003 Savegame 009 consequently
matches all recorded frames.

### Combat-position damage and score intermediates retain C++ lifetime

`RHcombatPosition::swEstimatedDamage` is a lazy cache, including for the
shared friend and enemy positions reused while scoring every proposed
attacker position. Original may update those records' target directions for a
later proposal, but an already estimated damage value deliberately remains
unchanged. Rust recomputed damage after every direction update, causing the
from-behind bonus to migrate between candidates and selecting a different
side of the same opponent.

The surrounding arithmetic also has two explicit narrowing boundaries:
`MaxNorm` is truncated into `UWORD` before the fractional distance malus, and
the resulting own score is truncated into `SLONG` before applying the egoism
factor. Rust now preserves both boundaries and the lazy cache. Linux3 Profile
001 Continue consequently chooses the same positions at frames 594 and 651
and matches every recorded frame.

### Side-looking seek completion preserves the Original rank switch

`SUBSTATE_SEEKING_JUST_WATCHING_SIDEWARDS` handles `EVENT_DONE` with explicit
`RANK_SOLDIER` and `RANK_OFFICER` arms and no default arm. Rust previously
treated every non-officer as a soldier, making a knight loaded in that
substate return to duty and launch `LeaveAttentiveMode`. The handler now
matches the rank switch exactly: ordinary soldiers return to duty, officers
look for a soldier to report to, and knights or rankless actors remain
unchanged. Cyrdach Profile 156 Savegame 010 now matches every recorded frame.

### Repulsive deviation preserves float-expression promotion boundaries

The repulsive point and line helpers store geometry, radii, movement distance,
and force coefficients as `FLOAT`, but use local `DOUBLE` variables for later
geometry. C++ therefore evaluates coefficient expressions and
`fMovement * fMovement`/`fRadius * fRadius` in single precision before
promoting their results. Rust had promoted the operands first and evaluated
the whole expressions in `f64`. The final displacement is stored back in
single precision, but repeated crowded anti-collision turns can still expose
the different intermediate result. The port now stages those expressions at
the same widths as the Original. Cyrdach Profile 156 Savegame 018's crowded
PC movement at frame 1164 now matches bit-for-bit; Savegame 007 remains a full
match.

### Lost-enemy handling uses the live ground-space stare point

Original's `_ANY_SWORDFIGHT_SUBSTATE_` switch arm intentionally has no
`break`: when the primary target is not still detectable in 360 degrees, it
falls through the same look-vector/stare-vector guard as
`REACTIONTIME_RUNNING`, `APPROACH_TO_OBSERVE`, and `ADVANCING_WITH_SHIELD`.
Rust previously skipped that guard for swordfight substates. Its existing
guard also substituted the AI's map-space `seek_position` for
`mViewParameters.starePoint`, although the latter is a ground-space point and
can lie on the other side of the actor after elevation projection. The AI
context now carries the live ground-space stare point and all of these
fallthrough cases share the exact guard. This prevents a simultaneous
OUTOFVIEW pair from spuriously launching a seek-area RNG burst in Linux2
Profile 002 `Continue-session-0002`.

### Entering a swordfight retains the outgoing movement goal

`RHArtificialMalignity::BeginSwordfight` calls `StopAll()` and then registers
`ENTER_SWORDFIGHT`, but `Stop(PREFERENCE)` does not directly run the selected
movement element's condolence callback. The sprite therefore retains its last
`PositionGoalMap` while the sword transition takes ownership. Rust had an
eager clear at the AI request-drain boundary, exposing `(0, 0)` as soon as the
snapshot switched from movement to `ENTER_SWORDFIGHT`. The eager clear is
removed; ordinary selected-element completion remains responsible for clearing
the goal at its actual callback boundary.

### Walking START state waits for the committed step result

`RHSprite::PerformMotion` commits its movement before it returns to the
actor's `Execute` switch. A newly selected walking order can report raw
`START`, deviate through anti-collision into its goal predicate during that
same call, and ultimately return `TERMINATED`. The START-only posture/action
rewrite must therefore not be applied merely from the pre-step sprite state.
Rust now defers ordinary walking START state effects until the committed step
has established that the same order remains current. This preserves Waiting
when a loaded, nearly complete waypoint terminates immediately, while
surviving walking orders still enter Moving on their first frame. Cyrdach
Profile 156 Savegame 023 now matches every recorded frame.

### Anti-collision break-through retains the cached increment

Original distinguishes two anti-collision position commits. A successfully
accepted repulsive deviation uses `SetPositionMap`, resets the increment
cache, and recomputes it toward the authored goal. The blocked-count
break-through path instead calls `MoveMap` and deliberately leaves the old
cached increment untouched. Rust rebuilt the cache after both paths because
both leave the persistent `deviated` flag set. That usually produced only
sub-threshold ULP drift, but a later near-tangent repulsive-point comparison
could amplify it into a different collision branch. The port now rebuilds
only after an accepted deviation resets the blocked counter; break-through
movement retains its previous cache. Cyrdach Profile 156 Savegame 025 now
matches PC 108's Original trajectory through the former frame-752 divergence.

### Every queued visibility callback reads the live detectable list

Original executes each `VIEW`/`OUTOFVIEW` callback synchronously and
`ReinitializeThemList` reads the NPC's authoritative detectable list at that
exact boundary. Rust correctly overlaid that live list for the one queued
stimulus carrying the completed detection-scan aggregate, but later visibility
stimuli in the same FIFO fell back to a geometric tick-data reconstruction.
That could resurrect an enemy after its `seen_now` latch had already been
cleared. The shared immediate-dispatch path now overlays the live detectable
list for every visibility callback, just as the retained-stimulus path already
did. Thus any `ReinitializeThemList` call made by a later visibility handler
sees the same latch state as Original.

### Opponent-list rebuilds use detectable latches on every Think

`ReinitializeThemList` does not perform a fresh geometric visibility query.
It walks the NPC's Enemy detectable list in pointer order, keeps entries whose
live `seen_now` latch is set, and drops dead targets. Rust previously rebuilt
from `AiPerTickData::enemy_sq_distances`; that happens to agree during many
detection callbacks but can disagree during timers, animation completion, and
pathfinding callbacks between detection refreshes. `AiContext` now snapshots
the authoritative seen handles at every synchronous Think boundary and
`reinitialize_them_list` consumes that list directly. In Linux2 Profile 002
Continue this matters when `EVENT_COULDNT_REACHPOINT` enters a battle overview:
the target remains geometrically visible but its detectable latch is already
clear, so Original seeks the lost PC instead of re-entering sword combat.

### Authored beam-me topology and the Original gang-index bug are preserved

Original creates PCs only at beam-me slots authored in the mission. Rust's
overlay convenience previously synthesized extra nearby slots when a campaign
team was larger than that authored set. Besides changing gameplay, the extra
PC consumed a static creation identity and caused the first dynamic object in
a v48 save to collide with an initialized ActorPC. Synthetic slots are no
longer created by the engine; overlays that need more placements must author
real beam-mes.

`RHCampaign::IsCharacterValidForThisSlot` also contains an observable indexing
bug: callers pass an index into `mMissionTeam`, but the function looks that
index up in `maGang`. Rust now preserves that lookup when solving beam-me
requirements, because it determines which character receives each scripted
slot and therefore the static actor/script identity that later saves restore.

### Actor VM decoding follows the live binding, not the serialized name

`RHElementActor::Serialize` always overwrites `mstrScriptClass` while loading,
but calls `Bind` only for an actor that was not already script-initialized.
Member bytes for a static actor therefore use its existing live VM class even
when the serialized class name differs. The v48 decoder now obtains that live
class from the initialized mission and uses the serialized name only when no
binding exists. A complementary zero-member case is inherently byte-ambiguous:
an empty serialized name writes no member bytes whether the actor is unbound
or retains an initialized zero-member VM. Adoption now preserves that live
binding, matching Original instead of rejecting the save on a false presence
mismatch.

### Forest behavior comes from the proto level, not the campaign-map location

Original assigns `RHEngine::mbForestLevel` directly from the proto stream's
`MISC` chunk. Rust loaded that value correctly and then overwrote it from the
mission profile's campaign-map location. Most profiles made the two values
look interchangeable, but `SherwoodOutro` deliberately combines the Sherwood
proto with the `Cross2` map location. The overwrite disabled forest behavior
there, including the Royalist 180-degree view rule. Rust now retains the proto
value as the sole authority, matching `InitializeMiscFromProtoStream`.
Linux3 Profile 001 QuickSave consequently matches every recorded frame instead
of missing a civilian's clear view of a hero at frame 5287.

### Restored pending paths retain their serialized waiting-order tail

`RHEngine::ProcessPathRequests` installs the first waypoint by renewing and
mutating the movement element's existing last order; it does not rebuild the
whole queue. This is observable when a v48 save contains an unresolved
`MOVE_WAITING`: the serialized start-transition prefix remains ahead of the
waiting order that becomes movement. Rust previously retained only its
runtime-generated transition-count prefix and discarded the rest, moving the
actor one frame early when the restored request completed.

Pending requests now record whether they came from the v48 pathfinder FIFO.
Only those restored requests reuse the exact saved tail in place. Requests
created later during live play keep Rust's established generated-prefix
representation, so save provenance cannot leak into subsequent pathfinding.
Cyrdach Profile 156 Savegame 028 now matches every recorded frame.

### Paying turns before advancing its animation

`RHElementActorPC::Perform(RHANIMATION_PAYING)` installs the direction opposite
the beggar as a goal, then calls `Turn` every frame. While the PC is still
turning, the Paying sprite remains frozen on its first frame. Rust had snapped
both current and goal directions immediately and advanced the animation during
alignment. The Pay ability now preserves the current direction at
initialization and uses the same progressive-turn/frozen-frame branch.
Linux3 Profile 003 Savegame 018 consequently matches every recorded frame.

### Loaded movement restores its executing-owner latch

Original retains an executing movement element in the actor's
`mpSequenceElement`. Rust also keeps a derived `ActiveMovement` identity that
owner-local movement and anti-collision code use to find the exact current
order. A loaded in-progress movement previously left that Rust-only latch
empty, so its final seek order did not hide the antagonist from
anti-collision and the approaching actor was deflected by the very target it
was trying to reach.

Post-load adoption now rebuilds `ActiveMovement` from each in-progress
movement element. Linux3 Profile 003 Savegame 019 consequently reaches its
downed Tie target on the Original path and matches every recorded frame.

### A zero-distance animation frame still exposes movement START

`PerformMotion` can return `START` on an animation frame whose authored
distance is zero. The actor does not change position on that call, but its
Execute switch still observes `START` and enters the corresponding Moving
state. Rust's stationary-motion guard retained the order correctly but skipped
that state side effect. It now applies the visible Execute result before
waiting for a later nonzero movement sample. Linux3 Profile 003 Savegame 020
therefore enters `MovingSword` on the first `WalkingWithSword` frame and
matches every recorded frame.

### Route arrival rebuilds patrols at the synchronous owner boundary

The `SUBSTATE_DEFAULT_GOTOROUTE` `EVENT_REACHPOINT` handler calls the virtual
`SetState`, waits for its `FilterAIEvent` callback, and then invokes
`InitializePatrol` inline. Rust previously represented the callback barrier
correctly but reduced the following patrol initialization to a one-shot flag
consumed by the next NPC hourglass. That one-frame delay exposed movement from
later legacy slots and could reorder equally close formation members, changing
their assigned side, facing, and subsequent commands.

The route-arrival continuation now carries the positions visible at its exact
owner boundary. After the script callback returns, patrol admission, distance
ordering, pair orientation, and chief assignment run synchronously from those
positions while all non-position state is read live so callback mutations
remain authoritative. Other initialization sites retain their existing
owner-ticked behavior. Linux3 Profile 001 Restart now matches every recorded
frame instead of diverging at frame 32.

### Rejected seek points are unlocked by recursive candidate selection

`RHArtificialMalignity::SeekNextPoint` stores the front candidate in
`mpActualSeekPoint` before checking its global lock and current interest.
When that check rejects the point, the recursive call first unlocks the
candidate it just rejected. This is an observable Original side effect:
another investigator later in the same frame may consume an RNG draw for the
newly unlocked point instead of skipping it.

Rust previously assigned `actual_seek_point` only after acceptance, leaving
locked and uninteresting candidates locked across the recursive call. Candidate
assignment now occurs immediately after the list pop, matching Original's
recursive unlock order. Linux2 Profile 002 QuickSave therefore selects the same
search point and keeps the global seek-point RNG stream attributed to the same
candidates.

### Fresh motion orders advance through an inherited action-done frame

`RHSprite::PerformMotion` has its own new-order path. It initializes the
action-done marker, retains `RHMOTION_START`, and then advances the animation
unconditionally. Unlike `PerformAction`, it does not rewrite that start result
to `RHMOTION_DONE` merely because a preceding order left the same animation on
its action-done frame.

Rust implemented motion by calling `perform_action` and only performed the
motion-specific first increment when that helper still returned `Start`.
Consecutive patrol orders could therefore inherit the same running animation
exactly at action-done, return `Done`, and skip one animation increment. The
positions initially remained equal because adjacent authored frames happened
to carry the same distance, but the chief reached its later running-to-walking
handoff one frame late and generated formation transition goals from newer
positions.

Fresh motion-order detection now owns the first-increment decision and restores
the Original `Start` result independently of `PerformAction`'s action-done
classification. This applies to every movement animation and order identity;
Linux2 Profile 002 QuickSave no longer shifts the patrol handoff at frame 2061.

### Initializing climb orders keep the lift-facing direction

Every ladder/wall climbing Execute arm calls `SetDirection` with the lift
sector's authored direction and clears the selected order's
`bComputeDirection` during `mbNewOrder` initialization. Rust installed the
lift-facing goal but left `compute_direction` enabled, so `PerformMotion`
immediately replaced it with the waypoint vector. This is especially visible
when a loaded save resumes an already-running fast climb with `mbNewOrder`
set: two turn/motion iterations then diverge in direction, elevation, and
position on the first frame.

Initializing authored climb orders now persistently clear direction
recomputation before sprite motion. Linux3 Profile 001 Savegame 008 therefore
matches the restored fast-ladder step and advances from frame 3915 to a later,
independent AI transition.

### Persistent PC properties use saved campaign-description identity

`RHElementActorPC::Serialize` restores `mpDescription` from a serialized
campaign-description pointer and aliases `mpStatus` to that exact
description. Character profiles are not unique identities: Sherwood can
instantiate several workers with the same profile but independent inventory.

Rust's persistent-property natives previously searched the campaign for the
first matching profile. A Windows Sherwood worker with zero arrows therefore
read ten arrows from another worker, passed the arrow-training script gate,
and launched a movement sequence that the Original never created. Persistent
ammo and name reads/writes now use the PC's adopted
`campaign_description_index`; live actor ammo remains the fallback only when
there is no campaign-backed description. Nescafe Profile 003 Savegame 016 now
matches every recorded frame.

### Smalltalk back-strikes use the normal delayed damage pipeline

The four playful smalltalk sword animations do more than play a swipe sound.
At their action-done tag, Original tests the animation's actual antagonist
(rather than looking up the actor's current principal opponent). If that
antagonist is holding a sword and the attacker is behind them, it launches a
normal delayed `RECEIVE_SWORD_DAMAGE` element with special left/right
smalltalk strike values.

Rust now retains the antagonist and handedness through the deferred animation
side-effect drain, performs the same facing/action-state test, and launches the
ordinary manager-tail damage element. The special strikes have Original's
fixed cutting effect of one, zero stunning and push effects, and no directional
thrust. They still consume both protection draws when the defender has a
weapon, but are excluded from the provoke draw because they are not real
profile thrusts. Their unusual parry behavior is also preserved: a high sword
parry silently absorbs smalltalk damage, while a low sword parry reports an
ordinary parry. Linux3 Profile 003 Savegame 022 now matches every frame.

### Resumed door passes retain two direction semantics

A movement element saved after its actor crosses a gate still contains the
direction with which the complete traversal began. At load time, however, the
remaining physical step chain must be rebuilt from the actor's current,
destination-side sector. These directions can therefore be opposites.

Rust now retains the saved C++ direction separately from the physical resumed
step direction. Physical motion follows the current sector, while
`RHArtificialIntelligence::Position(actor)`, route-source selection, forecast
input, and the legacy passing-door flag use the saved direction and hence the
committed gate-side point. Owner-slot AI views preserve that committed point;
periodic visibility continues to inspect the live interpolated position.
The override is the complete Original `RHposition`, including gate-side sector
and layer rather than only map coordinates. Battle decisions can therefore
recognize that a pursued target is committed to a ladder before its animated
body crosses the sector boundary. Linux3 Profile 001 Savegame 008 consequently
matches the Original detection, facing, and ladder-approach decisions after
its restored PC door pass.

That committed position is not a replacement for the element's physical
position. Source sites that call `GetSector` directly still observe the live
interpolated body sector. In particular, the ladder-wait handler tests
`mpPrimaryTarget->GetSector()->IsLift()` and repeatedly reconsiders its
approach until the target physically enters the lift. Per-tick AI metadata now
carries both views explicitly, so each translated source expression selects
the same one as C++.

### Lost-enemy swordfight exit is an explicit command

`EndSwordfight` launches `QUIT_SWORDFIGHT`; it does not directly clear the
opponent relationship. The command's translation owns both relationship
teardown and the lowering-sword transition, including its synchronous AI
callback and sequence arbitration.

The `EVENT_OUTOFVIEW` no-follow branch also snaps toward the missed human's
current position and enters the ordinary battle overview. Its forecast is
only for a possible chase, and the branch does not request `FAST_OVERVIEW`.
Keeping those three source-level boundaries distinct prevents a speculative
turn or newly selected nearby target from delaying the swordfight exit.

### Seek transitions inherit the movement speed factor

Original transition Execute arms have two motion paths. A direct transition
calls `RHSprite::PerformMotion` without a factor and uses its default `1.0`;
a transition carrying `RHMOVE_SEEK` calls `RHElementActor::PerformSeek`,
which explicitly forwards the movement element's speed factor. Rust now makes
the same distinction instead of treating every transition as unscaled.

### Seek refresh timing remains observable after route replacement

Original overloads `mulWaitTime` as the seek refresh counter. Both initial
entity-seek launch and refresh assign `TIME_SEEK_REFRESH` before a cross-sector
path builder can synchronously replace the selected movement with its door
route. Rust keeps separate semantic counters, but now mirrors the assignment
to both at that same source boundary and before either the direct-path or
cross-sector branch. A following `WAIT_FREE_LIFT` therefore observes the same
retained actor field as C++.

### A re-entrant turn clears the superseded movement goal at execution

`FaceTo` can replace a movement sequence re-entrantly after the outgoing
`GoNear` has already installed its map goal. Original leaves that destination
observable during the launch frame, then the outgoing movement element's
condolence callback clears it before the turn element executes its first
order. Rust stages those callbacks separately, so the selected turn now clears
the superseded goal at that equivalent first-execution boundary. This preserves
both the launch-frame observation and the following turn state without a
replay-specific exception.

### Swordfight event retargeting does not retarget the eyes

`EVENT_ENTER_SWORDFIGHT` assigns the incoming opponent to
`mpPrimaryTarget`, but does not call `Focus`. This differs from the ordinary
`BattleDecisions` approach path, which explicitly focuses its chosen target.
The AI's combat target and the NPC view cone's followed element are therefore
independent state.

Rust's edge-triggered bridge between deferred AI target writes and NPC focus
mistook every target assignment for an implicit `Focus` call. A soldier
restored while following one PC could begin a fight with another PC, turn its
eyes toward the new opponent one frame later, and consequently accept
`EVENT_OUTOFVIEW` events that Original rejects using the old stare vector.
There is no such automatic synchronization in Original, so the bridge has
been removed: translated `Focus(element)`, `Focus(position)`, and
`Focus(NULL)` calls are now the only operations that change NPC focus.
Nescafe Profile 003 Savegames 010, 011, and 014 match every recorded frame,
and the related traces proceed to later independent combat decisions.

### Path walking-style changes relaunch movement inside the native

Original `RHArtificialIntelligence::SetPathWalkingFlags` writes the new
default flags and immediately calls `GoTo` for an NPC already travelling its
patrol route. This entire replacement launch is nested inside the
`SetPathWalkingStyle` script native. Rust already reproduced the route and
flag selection, but left the resulting order in the AI outbox for the next
frame's global order pass.

The script-effect bridge first promoted that owner's move before the next
frame, but still let the VM execute later statements before the relaunch. This
is observable when `SetPathWalkingStyle` is followed by `Thanx()`: the recorded
script sequence was registered ahead of the replacement Move even though the
C++ native had already returned from `GoTo`. The relaunch is now a synchronous
VM request. Only after its Move is registered does the script resume and launch
subsequent recorded work. Consequently both the sampled position and sequence
arbitration order match the native call stack. Linux2 Profile 002 QuickSave
passes the former frame-2061 transition-goal mismatch, while nicouzouf Profile
001 Savegame 024 passes its former frame-95 command/direction divergence.

### Patrol direction turns wait for the sequence-manager pass

A patrol chief can finish a movement from its creation-ordered actor slot and
synchronously broadcast a direction to the other patrol members. Each waiting
member calls `FaceTo`, but Original `FaceTo` only registers its new Turn with
`RHSequenceManager`; it cannot instruct that element after the manager's
Hourglass pass has already completed for the frame.

Rust still closes the member's synchronous AI side effects at the broadcast
boundary, but now leaves the registered Turn uninstructed until the next
sequence-manager pass. This prevents later-created patrol members from
executing the turn one frame early merely because their actor slots have not
yet run. Linux3 Profile 003 Savegame 024 exercises the boundary with four
synchronized patrol members.

### Ladder and wall idle waits preserve the moving action state

When Actor::Hourglass installs its implicit Wait while an actor is on a ladder
or wall, Original translates the command to the non-animation Freezing order.
`MakeActionTransition` has no ladder/wall arm, so it intentionally leaves the
actor's prior action state unchanged while holding the last climb frame. Rust
no longer normalizes that action state to Waiting. This keeps the serialized
Moving state visible for the frozen ladder/wall idle just as in Original.

### Post-seek interactions launch only from PerformSeek

Original keeps a pending interaction such as Tie in the actor's
`mpPostSeekSequence` while a cross-sector seek traverses its intermediate
movement, door, and assertion elements. Only `PerformSeek` may consume that
continuation: either the live target has entered the actor's sector and
tolerance, or the final seek order has terminated and passed its live-target
validation.

`RHElementActor::RefreshSeek` also leaves that continuation attached to the
actor whenever a moving target causes it to replace the current cross-sector
route. Rust previously moved the continuation into the first transient gate
route. A later refresh interrupted that route and destroyed the continuation,
so reaching the target merely entered the frozen seek wait instead of launching
the intended interaction. Route replacement now retains the actor-owned
continuation across any number of refreshes. Linux3 Profile 001 Savegame 008
advances past its cross-sector swordfight arrival at frame 4,029.

Rust also had a fallback in generic `DoNextOrder` cleanup which launched the
post-seek sequence whenever any SEEK-flagged `MoveOk` exhausted its local
orders. That incorrectly fired Tie at the first gate approach, where a later
route assertion interrupted it, and left an implicit Wait at the real
destination. The fallback is gone; the existing `PerformSeek` arrival paths
are now the sole owners of the handoff.

Tie translation itself now only attaches the Tying order. Its first Execute
validates the target, installs the progressive direction goal, calls `Turn`,
and advances the action animation, matching `RHElementActorPC::Execute`.
Translation no longer stops the actor or snaps its facing before that order
ever owns an actor slot. Linux3 Profile 003 Savegame 035 exercises both
boundaries and matches every recorded frame.

### Synchronous GoTo failures re-enter Think before it returns

Original `GoTo` constructs its path through `AppendMoveToSequence` inline.
When a seek point is unreachable, that construction sets
`mbCouldntReachPoint` before the enclosing `EndThink`, which immediately
re-enters the AI with `EVENT_COULDNT_REACHPOINT`. A single `SeekNextPoint`
call can consequently reject several unreachable candidates—and consume each
candidate's acceptance draw—before finding a usable route.

Rust releases the controller borrow before constructing its queued movement.
The path result was therefore arriving after controller-side `end_think`, and
the synchronous owner fixed point did not turn it back into a self stimulus.
Synchronous Think drains now surface an engine-side path failure immediately
after movement construction, then continue the same owner-local fixed point.
This is the general `GoTo`/`EndThink` boundary; it contains no actor, point, or
replay-specific condition. Linux Profile 002 QuickSave advances from frame
2,074 to frame 2,113.

### Swordfight exit interruption re-enters Think before the caller resumes

Original `EndSwordfight` launches an explicit `QUIT_SWORDFIGHT` element. Its
`Instruct` arbitration can interrupt the selected actor command and deliver
that command's condolence card synchronously. The resulting `EVENT_DONE`
therefore re-enters the enemy AI while the caller's old swordfight substate is
still current; only after that nested callback returns does the outer
lost-enemy path face the missed human and install its battle overview.

Rust already arbitrated the quit inline, but left the generated condolence card
queued until after the outer handler had installed the overview. The nested
`EVENT_DONE` consequently advanced the newly installed look state instead of
being handled by the outgoing swordfight state. Quit arbitration now closes
its condolence boundary immediately, while the lost-enemy continuation is held
by the outer drain and resumed afterward with fresh live context. Nescafe
Profile 003 Savegames 012 and 013 match every recorded frame, including the
former downstream script-RNG split.

### Flying projectiles remain outside a layer without a landing sector

Original `RHElementProjectile::ComputeTrajectory` first assigns layer
`0xFFFF`, clears the sector and obstacle, and constructs the complete flight.
It restores the prospective landing layer only if the landing point resolves
to a motion sector. An arrow landing outside all motion sectors therefore
retains the no-layer sentinel throughout its flight.

Rust initialized arrows on the shooter's layer and later assigned the landing
resolver's default layer even when that resolver returned no sector. Projectile
creation now starts with the explicit no-layer state, and both initial and
later landing-resolution paths require a resolved motion sector before
assigning a layer. Nescafe Profile 003 Savegame 005 matches every recorded
frame after this correction.

### Existing enemy detectables survive later camp changes

Original applies its camp/role policy when `AddDetectable(..., ENEMY)` creates
an entry. `CleanUpDetectables` later removes dead targets only; it does not
revalidate the original insertion policy. A serialized detectable can
therefore remain authoritative after its observer or target changes camp.

Rust still applies the policy to newly appended entries, and still rejects
missing or non-human saved targets as corrupt, but no longer rejects a
well-formed existing entry solely because its current camps would prevent a
new insertion. This lets Nescafe Profile 003 Restart adopt and simulate its
saved historical detection lists instead of panicking on the first frame.

### Exact-position transitions do not reuse stale movement increments

A generated waiting-to-walking transition may have a destination exactly equal
to the actor's current map position. Original still advances the transition
animation, but contributes no map displacement. Rust passed the transition's
nonzero sprite-frame distance to anti-collision together with the
position-interface increment retained from the preceding order. That moved the
actor away from the already-reached destination and changed its direction goal.

Transition animation advancement is now independent from physical displacement:
an exact-position transition continues to tick but skips map movement,
anti-collision, and line-crossing work. Positive-distance transitions retain
their existing movement and collision behavior. Linux3 Profile 003 Savegame 042
matches every recorded frame after this correction.

### Deferred turns keep an explicitly retained movement goal

Rust has a synthetic first-Turn goal clear which compensates for its staged
condolence callbacks: an ordinary `GoNear` followed by `FaceTo` must not leave
the superseded movement destination on the sprite. A deferred FaceTo carrying
`RetainedMovementGoal` represents the opposite Original boundary. The outgoing
movement's condolence card has already observed that Turn as the actor's
selected element, so it deliberately preserves the goal; Turn execution itself
does not clear it.

The synthetic clear now applies only to unmarked Turns. This preserves the
general staged-callback correction without erasing a goal which the sequence
explicitly retained. Linux2 Profile 002 QuickSave matches every recorded frame,
including the former civilian patrol divergence after frame 2,113.

### Final stop transitions complete entity seeks before retiring

`RHElementActor::PerformSeek` wraps the complete movement order stream,
including the final walking/running-to-waiting transition. When that transition
terminates, Original revalidates the live entity target just as it does after a
plain final waypoint: a stale target refreshes the seek, while an unchanged or
in-range target launches the actor-owned post-seek interaction.

Rust's transition-terminated fast path bypassed that validation and exhausted
the movement directly into Wait, leaving actions such as `HealCmd` dormant.
The transition path now performs the same final-target handoff, refresh, and
no-action frozen-wait behavior as the ordinary arrival path. Linux2 Profile 002
Savegame 016 matches every recorded frame.

### Immediate LockAi stops the still-selected outgoing command

`RHSequenceElement::ExecutedImmediately` invokes the NPC's
`ExecuteImmediately` directly; it does not call `Go`/`Instruct` and therefore
does not install the `LOCK_AI` element as `mpSequenceElement`.
`ScriptLockAI` consequently sees the actor's outgoing command and calls
`Stop(Normal)` synchronously before the LockAi element terminates.

Rust previously assumed the immediate element had already been selected and
suppressed that stop. A moving NPC could advance one extra frame before the
following scripted animation reached the later sequence-manager pass. The
immediate handler now closes the outgoing stop and its condolence stack at the
LockAi boundary. Linux2 Profile 002 Savegame 018 matches every recorded frame.

### Deviated goal checks include the current antagonist radius

`RHPositionInterface::IsGoalReached` has a special anti-collision arrival
branch. When a mover is deviated and still has a nonzero blocked counter, it
accepts a waypoint whose center separation is below the mover radius plus the
current target radius plus ten units. `RHSprite::PerformMotion` supplies the
target cached from the current order.

Rust implemented the radius-aware predicate but every movement caller passed
`None`, disabling it. This was particularly visible immediately after loading
a mid-pursuit save: both engines committed the same final step, but Original
retired the waypoint while Rust left its `MoveOk` element and movement goal
live. Movement now snapshots the current order antagonist's radius and supplies
it to every projected, transition, and committed goal check. Linux2 Profile 002
Savegame 023 matches every recorded frame.

### Fast-climb double motion preserves both position roundings

The non-animation fast stairs, ladder, and wall tokens execute the ordinary
sprite motion call twice in `RHElementActor::Execute`. Each call immediately
stores its own map-position update. Even when both calls retain the same cached
increment, adding their scaled distances first is not binary32-equivalent:
large map coordinates round once after each Original call.

Rust already advanced the animation and applied the two turn slowdowns
separately, but combined the resulting distances for one physical commit. It
now retains both distances and, on the anti-collision-disabled lift/door path,
commits the two map updates in Original order. This removes the one-ULP X/Y
drift which a steep stair plane amplified into the first Linux2 Profile 002
Savegame 029 elevation mismatch at frame 4,041; that replay now advances to an
independent facing divergence at frame 4,123.

### Sword movement faces opponents in ground space

`RHElementActorHuman::FaceOpponent` subtracts the actors'
`GetPositionGround()` values and passes that ground-space vector to
`GetSector0to15(ASPECT_RATIO)`. The elevation contribution to world Y is
therefore authoritative when combatants stand on different surfaces.

Rust snapshotted projected map positions for the sword-movement facing pass.
That is equivalent only when the two elevations cancel; otherwise it can place
the opponent vector in an adjacent direction sector and rotate the mover one
frame early. Sword opponent snapshots now retain ground X/Y, while shield
targets keep their existing coordinate contract. Linux2 Profile 002 Savegame
029 now matches every recorded frame, including the former backward-sword
facing divergence at frame 4,123.

## Current Linux-v48 loaded-save result

The schema-11 runner decodes and atomically installs the embedded Linux-v48
save before replay. Dynamic elements are constructed on the detached candidate
before payload preflight, the shared VM arena and post-load consequences are
applied in Original order, the recorded global RNG stream remains
authoritative, and serialized camera/minimap/view state is carried into
headless and visual replay hosts.

Every frame in the original five-trace corpus matches:

- Profile 005 `Continue-session-0002`
- Profile 005 `Restart-session-0002`
- Profile 011 `Continue-session-0002`
- Profile 011 `Restart-session-0002`
- Profile 011 `Savegame_000-session-0001`

The expanded authoritative Linux audit adds 48 `Savegame_linux2` traces and
140 `Savegame_linux3` traces. Unlike the first group, these exercise up to
hundreds of dynamic bonuses/projectiles and a much wider set of interrupted
runtime states. All five Linux3 Profile 002 traces match every recorded frame.
Linux3 Profile 003 is also completely green: Savegame 000--075 plus Continue,
ExQuickSave, QuickSave, Restart, and Sherwood all match every recorded frame.
The remaining corpus is the active completion set; failures are grouped by
their first general cause and the whole affected shard is rerun after each fix.

The formerly excluded static Linux3 Profile 003 Savegame 013 trace received a
separate frozen release-mode recheck after the loaded-save and battle-ordering
repairs. Its 250 frames (7597 through 7847), 67-element save including 16
dynamic elements, and 360 authoritative RNG draws match exactly through EOF.
The same frozen runner also verifies formerly excluded Savegame 021: all 250
frames (13 through 263), its 72-element save including 21 dynamic elements,
and 346 authoritative RNG draws match exactly through EOF. Savegame 030 is
exact as well across all 250 frames (1729 through 1979), with 85 saved elements
including 34 dynamic elements and 424 authoritative RNG draws. Savegame 033
also matches all 250 frames (3713 through 3963), including its 92 saved elements,
41 dynamic elements, and 424 authoritative RNG draws. Savegame 041 matches all
250 frames (2174 through 2424), including its 98 saved elements, 47 dynamic
elements, and 481 authoritative RNG draws. Savegame 045 also matches all 250
frames (3711 through 3961), including its 105 saved elements, 54 dynamic
elements, and 499 authoritative RNG draws. Savegame 047 matches all 250 frames
(2068 through 2318), including its 105 saved elements, 54 dynamic elements, and
551 authoritative RNG draws. Savegame 053 matches all 250 frames (1675 through
1925), including its 104 saved elements, 53 dynamic elements, and 553
authoritative RNG draws. Savegame 056 matches all 250 frames (490 through 740),
including its 108 saved elements, 57 dynamic elements, and 519 authoritative
RNG draws. Savegame 057 matches all 250 frames (1537 through 1787), including
its 108 saved elements, 57 dynamic elements, and 748 authoritative RNG draws.
Savegame 061 matches all 250 frames (344 through 594), including its 115 saved
elements, 64 dynamic elements, and 587 authoritative RNG draws. Savegame 062
matches all 250 frames (7682 through 7932), including its 115 saved elements, 64
dynamic elements, and 537 authoritative RNG draws. Savegame 064 matches all 250
frames (5865 through 6115), including its 128 saved elements, 77 dynamic
elements, and 530 authoritative RNG draws. Savegame 070 matches all 250 frames
(3117 through 3367), including its 137 saved elements, 86 dynamic elements, and
549 authoritative RNG draws. The static Sherwood trace matches all 250 frames
(4385 through 4635), including its 137 saved elements, 86 dynamic elements, and
544 authoritative RNG draws. No replay-specific compatibility or code change
was required for these traces.

Windows `GSHR` saves now share the atomic adoption path. The complete Cyrdach
Savegame 000–037 corpus matches every recorded frame; Nescafe Profile 003
Savegames 010–014 and 016 have also been verified. The remaining Windows
sessions are part of the active completion set.

### Rejected bow shots retain their generated action transition

`RHElementActor::Instruct` generates the posture/action transition before
`RHElementActorHuman::Translate` checks whether a bow target is reachable and
within range. A rejected `SHOOT_BOW` body therefore does not make the whole
sequence element impossible: any generated equip/load transition continues to
run, and only the shot body is absent.

Rust now preserves and executes that transition prefix, or terminates normally
when no prefix was needed. This is general command-order behavior rather than a
trace exception. It advances Nescafe Profile 003 Restart from frame 147 to its
next independent movement-geometry divergence at frame 207.

### AI `GetAnimation` reads the actor order, not a background idle

The original `RHElementActor::GetAnimation()` returns the current actor order.
That can remain `WAITING_UPRIGHT` while the sprite independently displays a
bored background animation. Rust's AI context instead exposed
`Sprite::last_action`, causing `GoTo` to miss its five-unit already-on-point
shortcut and launch a redundant movement to an adjacent patrol waypoint.

AI contexts now prefer the actor's latched current order and use the sprite only
before the first order has been latched. The close-point callback also preserves
the original recursion boundary: calls made inside `Think` defer through
`already_on_point`, while calls made by the macro timer outside `Think` queue
the synchronous owner-boundary `EVENT_REACHPOINT` re-entry. Nescafe Profile 003
Restart now consumes its complete recorded RNG stream and reaches the end of
the 240-frame session.

### Movement translation does not enter the moving action state

Original `RHElementActor::Translate` and `PostProcessPath` populate the order
queue but do not update the actor's posture/action pair. The corresponding
`Execute` animation arm performs that update only when `PerformMotion` returns
`RHMOTION_START`. This is a full frame later when `RHSequenceManager::Hourglass`
instructs the movement after the entity loop.

Rust previously entered `Moving` or `MovingFast` as soon as a path was
installed. The eager update usually became invisible because the actor still
executed later in the same frame, but it leaked at the post-entity manager
boundary. Movement setup now installs only the active-movement identity; the
ordinary Execute START effects cover upright, alerted, stairs, running, sword,
and wall movement. This resolves Nescafe Profile 003 Restart's loaded-state
Civilian 94 mismatch on frame 1 without special-casing the save or actor, and
the entire recording now matches.

### Postponed actor commands re-enter the manager instruction boundary

`StartPostponedSequenceElement` appends a released element to the sequence
manager FIFO. It does not call the actor's `Instruct` method synchronously from
the terminating actor slot. When the manager later drains that FIFO, `Instruct`
again snapshots the actor's current posture and action state before priority
arbitration, transition generation, and translation.

Rust previously promoted a cross-postponed command inside the actor callback
and retained the posture/action snapshot from its first, postponed instruction.
The early promotion let later work in the same NPC hourglass cancel the command;
the stale snapshot could also resume an upright run after the actor had raised
his sword. Released ordinary commands now remain in manager order and mark
their transition snapshot stale for the later instruction call.

### Special strikes remain special through their strike action

The delayed AI strike sequence is `WAIT_TIMER` followed by the actual
sword-strike command. `IsLastRealAction` suppresses an event for the preparation
wait because the strike is a following real action. The
`ATTACKING_SWORDFIGHT_SPECIAL_STRIKE` substate therefore remains active until
the strike sends `EVENT_DONE` or its timer expires.

Rust had a Thrust-A-only shortcut which returned to ordinary swordfight when
the strike element was instructed. That shortcut has been removed; cancellation
continues to use the independent active-strike reconciliation path.

### End-of-hourglass death unselection is synchronous

After the entity and sequence-manager passes, Original scans selected PCs and
forwards `MSG_UNSELECT_CHARACTER` immediately for dead or unconscious members.
The messenger routes that selection mutation synchronously. Rust now applies
the same selection and macro-recording state at that boundary instead of
leaving the authoritative unselection queued until the next frame.

### Strike warnings use the original victim and duel predicates

Straight-strike warning candidates now pass through the common
`IsPossibleSwordStrikeVictim` equivalent. Separately, a PC's `WarnForStrike`
guard uses the original definition of `IsSwordfighting`: a non-empty opponent
list, not merely a sword-flavoured action state. A dead PC whose visual action
has not yet changed but whose duel links are already cleared therefore cannot
consume counter-strike RNG or launch another action.

Together these corrections make Linux3 Profile 003 Savegames 043–051 match
every recorded frame.

### A cleared actor order reports the non-animation sentinel

`RHElementActor::GetAnimation()` returns `RHNONANIMATION_END` when its
authoritative `mpOrder` has been cleared. This is not the same thing as either
the sprite's last visible action or an invalid animation value. In particular,
`GoTo` includes `RHNONANIMATION_END` in its close-point fast path: a patrol
member already within five map units of its new formation coordinate
synchronously reports `EVENT_REACHPOINT`, remains in the waiting patrol
substate, and can then accept the chief's instructed turn.

Rust's actor-hourglass latch uses `OrderType::Invalid` to represent that
cleared-order boundary. AI contexts now translate that internal latch to
`NonanimationEnd`, preserving the Original API's observable sentinel and the
synchronous `GoTo` shortcut. This makes Linux3 Profile 003 Savegames 052–058
match every recorded frame.

### Remembered stimuli retain their complete payload

`EVENT_AFTER_SCRIPT_GO_ON` drains the Original AI stimulus queue by recursively
calling `Think` with each saved `RHStimulus`. The payload is authoritative:
for example, a remembered `EVENT_VIEW` carries the human that was seen, and a
fleeing civilian can react to that human with another directed panic in the
same call chain.

The friendly Rust AI previously reduced queued stimuli to `StimulusType` before
re-dispatch. This discarded the viewed actor, so the second `EVENT_VIEW` was
logged but did nothing and two panic RNG draws were skipped. Friendly AI now
mirrors the Original and the enemy implementation by recursively dispatching
the complete saved stimulus in FIFO order. This advances Linux3 Profile 001
Savegame 008 from frame 4056 to its next independent divergence at frame 4075.

### Panic seek retries observe synchronous route failure

Original `GoTo` constructs its route before returning to the panic
`EVENT_COULDNT_REACHPOINT` handler. If the randomly selected emergency
seek-point route also fails, the handler immediately clears the failure,
decrements the remaining panic-run count, and recursively processes
`EVENT_REACHPOINT`. This can consume all five random retry pairs and enter the
hiding state within one `Think` call.

Rust now closes that same owner-local path-construction boundary after issuing
the fallback `GoTo`, then observes and processes route failure before the
enclosing AI fixed point returns. This advances Linux3 Profile 001 Savegame 008
from frame 4075 to its next independent divergence at frame 4079.

### Panic fallback `GoTo` reads the live actor order

The movement that raises `EVENT_COULDNT_REACHPOINT` has already sent its
condolence callback when Original selects the nearest panic seek point. The
nested fallback `GoTo` therefore reads the sequence manager's current order
through `RHElementActor::GetAnimation()`, commonly receiving
`RHNONANIMATION_END`; it does not see the just-finished running animation.

Rust now refreshes that animation value from the live sequence-manager order
at the fallback boundary. A seek point coincident with the actor consequently
takes `GoTo`'s already-on-point path and recursively selects the next panic
segment in the same frame. With this correction, Linux3 Profile 001 Savegame
008 matches every recorded frame and all 875 simulation RNG draws.

### Mission-team queries return the exact live PC

`RHScript::GetPCFromMissionTeam` retrieves an `RHPCDescription` from the
campaign and passes it to `RHEngine::GetPC`. Its result is the live actor
instantiated from that exact campaign description, not the description's
shared character-profile number.

Rust previously returned the character-profile index as a raw script value.
This happened to resemble a small integer handle but could never compare equal
to a real actor. Sherwood's deployment-zone cleanup consequently classified
every member of a five-PC mission team as an outsider and launched moves for
all of them. The native now resolves the mission-team character index through
each live PC's stable campaign-description index and returns the corresponding
actor handle.

### Failed strike proposals still decay strike boredom

`RHElementActorHuman::ProposeGoodSwordStrike` decrements all normal-strike
boredom counters while evaluating a proposal. Those mutations remain even
when no strike is viable and the function returns `RHCOMMAND_NULL`.

The Rust PC wrapper evaluated a cloned boredom array but only wrote it back
after selecting a strike. Repeated failed proposals therefore retained stale
boredom and could suppress a later strike that the Original accepted. Rust now
persists the evaluated array before the no-proposal return. Together with the
mission-team query correction, this makes Linux2 Profile 002 Savegame 038 match
every recorded frame.

### Additional Linux loaded-session coverage

The current parity implementation also matches every recorded frame in the
complete Linux3 Profile 002 set: Continue, ExQuickSave, QuickSave, Restart, and
Savegame 000. The earlier Linux corpus is green in full as well: Profile 005's
Continue and Restart sessions, and Profile 011's Continue, Restart, and
Savegame 000 sessions. These are independent loaded-session checks; they do not
require replay-specific state substitutions or compatibility paths.

### Mobile-element topology retains its authored count

Original inserts each `RHElementMobile` master and its masked child sprites in
the mission element stream. Rust constructs the master outside the ordinary
entity arena, but loaded-save topology still needs the authoritative number of
authored masters. The mobile load stage previously populated the live world
without copying that count into `LevelEntityAssets`, so every mission with a
mobile master rejected its own initialized topology as one live mobile against
zero authored mobiles. The stage now retains the count directly from the
decoded mission stream. This exposes the separate, explicit mobile-master
state-adoption work instead of failing earlier on inconsistent metadata.

### `CALL_LOOKTHERE` completes an already-satisfied turn synchronously

Original `CallLookThereStandardProcedure` calls `Face(RHposition)`, which
reaches `FaceTo` and immediately raises `EVENT_DONE` when an idle actor already
faces the requested sector. Rust's context-free facing helper could not make
that test and always registered a `TURN` sequence, leaving the redundant
command observable for one frame even though the actor's direction and goal
were already equal. The handler now uses its available actor context and takes
the same synchronous completion path. This makes Linux3 Profile 001 Savegame
010 match every recorded frame.

### Mobile masters keep a separate save identity

Original places each `RHElementMobile` master in `marrayElements`, while Rust
stores the master in its dedicated mobile arena and only puts the masked child
FX in the entity arena. Save adoption now preserves both spaces explicitly:
ordinary creation orders and AI slots continue to resolve only to real Rust
entities, and mobile creation orders resolve to mobile indices. Sequence
commands owned by a mobile master map isomorphically to its first masked child,
the same proxy used by Rust's existing Start/Stop/Activate/Deactivate runtime.

The mobile payload now restores the master position, old position, goal,
increment, path cursor and direction, active/stopped state, speed target and
acceleration, plus masked-child position, active state, and animation speed.
The motion polygon is translated by the saved master displacement just as
`RHElementMobile::Serialize` translates its motion sector and collision
geometry on load. This supports loaded sessions without inventing an entity ID
for the non-entity master.

### A selected Turn preserves the interrupted movement goal

Original `RHElementActor::SendCondolationCard` clears the sprite goal only when
the completed or interrupted element is still the actor's selected
`mpSequenceElement`. During `Instruct`, an incoming Turn becomes selected
before the outgoing movement is interrupted, so that movement's synchronous
card deliberately leaves its destination cached.

Rust now records that retained goal on a Turn which wins normal priority
arbitration against a movement. Its transition-to-waiting and subsequent turn
therefore observe the same selected-owner boundary instead of clearing the
goal when the first Turning order initializes. This advances nicouzouf
Savegame 008 from frame 110 to the mobile-motion boundary at frame 178.

### Loaded building occupants update the house AI view

`RHSectorBuilding::Serialize` restores each building's ordered occupant list
and arrow reserve from the save. Rust already adopted those values into the
script-facing building domain, but `EnemyInHouseAlert` reads the separate
typed occupant list held by `AiGlobalState::houses`. That view remained at its
mission-start contents, so a loaded house containing both camps and civilians
could incorrectly skip panic propagation and battle-before-door setup.

Grid adoption now updates both representations from the same preflighted,
typed occupant list while preserving the save's ordering. This follows the
Original's single authoritative building list and makes Linux3 Profile 001
Savegame 014 match every recorded frame.

### Actor movement snapshots are frame-local

`RHElementActor::Hourglass` calls `NewMove` after installing a lazy Wait and
immediately before it samples the selected order and enters `Execute`.
Delayed-position branches take an earlier snapshot for their crossing segment
and still reach this second call. Rust now performs the same per-owner
snapshot, so `IsMoving` and `IsMovingMap` compare against the start of the
actor's current hourglass slot rather than a stale loaded-save or previous
movement snapshot.

### Anti-collision queries use the actor's current radius

`RHPositionInterface::UpdatePositionAntiCollision` computes its neighbour
query half-diagonal as `MAX_REPULSIVE_DISTANCE + mfRadius`. The radius is live
state: repeated blocked moves can shrink it below the normal human radius.
Rust previously used the fixed normal radius for this query even though it
correctly used the shrunken radius in the later deviation math.

The extra query width could include a neighbour which the Original excluded;
the mere presence of that neighbour also enables the level-corner repulsion
pass, making a one-unit query discrepancy produce a visible turn and position
change. Rust now uses the same current radius for both the query and deviation
stages. This makes Nescafe Profile 001 Continue, Restart, and Savegames 000–004
match every recorded frame. The complete Nescafe Profile 003 corpus—Continue,
Restart, Savegames 000–016, and Sherwood—also matches every recorded frame.

### Non-enemy visibility retains its periodic cadence

Original `ComputeVisibility(RHDetectable&)` computes a `bRefreshAlways` flag
when an observer is staring, following, or visually above Green alert, but only
consults that flag in the Lacklandist Enemy branches. Body, Friend,
MissedFriend, Beggar, and Object entries continue to reuse their cached
visibility until the detectable type's modulo frequency opens.

Rust had applied the enemy-only shortcut to every detectable type. An alerted
soldier therefore refreshed a nearby friend early, emitted
`EVENT_SEES_SOLDIER`, and launched a new movement one frame before Original.
The non-enemy human and object passes now remain strictly cadence-bound while
the existing enemy behavior retains its refresh shortcut. This is a direct,
general translation of the Original branch structure and makes Linux3 Profile
001 Savegame 016 match every recorded frame.

### Standard strikes are owned by their selected order and sprite cursor

Original persists an in-flight standard sword strike through the actor's
selected sequence element, its current order and antagonist, and the sprite's
live action cursor. Rust formerly duplicated those values in an `ActiveMelee`
cache, including invented fixed-frame fallback timers and a separate
`hit_applied` latch. Besides having no Original counterpart, that cache could
desynchronize from the selected order and required special reconstruction
when adopting a save.

`ActiveMelee` has been removed. Strike type now comes from the selected order's
animation, the target comes from its typed antagonist, and damage occurs only
on the sprite's one-frame `MotionState::Done` result. Termination closes the
owning sequence element. Save adoption therefore needs no melee-specific
reconstruction: the ordinary restored order and sprite cursor resume the exact
Original execution path directly. Linux3 Profile 003 Savegame 066, whose
loaded in-flight strike originally exposed the missing execution state, still
matches every recorded frame with the cache removed.

### Parry countdowns run in actor creation order

`RHElementActorHuman::Execute` decrements `muwParryCounter` inside the active
parry animation. A normal parry queues `STOP_PARRY_SWORD` at that exact actor
slot; a low parry terminates its own order. This ordering is observable when a
later-created attacker queues damage during the same engine traversal.

Rust now advances the counter in each actor's Execute slot rather than in a
post-traversal combat batch. It also preserves the Original unsigned decrement
followed by signed 16-bit expiry check, including zero wrapping to an expired
negative value.

### `EVENT_OUTOFVIEW` handles the event's explicit enemy

The Original handler receives the actor which left view and applies the active
approach or stationary-attacking branch to that explicit actor. It does not
require the actor to remain the AI's current primary target. Rust removed that
extra primary-target guard, so target replacement during queued visibility
events no longer causes a valid out-of-view event to be discarded.

### Too-proud combat facing includes elevation

The `ATTACKING_TOO_PROUD_TO_ATTACK` decision calls `Face(mpPrimaryTarget)` in
Original. That overload uses the target element's full position, including
elevation, before choosing the 16-way direction. Rust now uses the same
element-aware facing path instead of projecting only the map-plane delta.

### An amulet coma preserves the interrupted action state

`RHElementActorPC::GetWounded` activates a lethal-hit amulet by setting the
campaign coma flag, five life points, maximum concussion, lying posture, UI
state, and campaign inventory. `SetConcussionOfTheBrain` quits swordfight, but
neither function rewrites the actor action state. That retained state selects
the appropriate ordinary or sword-specific unconscious animation after the
damage interruption.

Rust's coma helper no longer forces `Waiting`; it leaves the interrupted state
intact while clearing only Rust's derived live execution caches.

### Battle cleanup retains newly unconscious targets

`BattleDecisions` begins with the persistent `mlistThem`, removes entries which
can no longer fight, and simultaneously collects living, unconscious,
non-carried enemies into a local list. It later uses that exact ordered list to
approach and finish a sleeping enemy. Re-running visibility is not equivalent:
an enemy can cease being detectable immediately on knockout while its
authoritative Them-list entry is still pending consumption.

Rust now constructs the sleeping candidates during the persistent-list cleanup
itself rather than substituting the current frame's detection snapshot. This
keeps an opponent knocked into an amulet coma on the approach path instead of
making the observing soldier return to duty.

### PC coma lookup uses campaign-description identity

Original's `RHElementActorPC::IsInComa` reads the status behind the PC's exact
`mpDescription` campaign pointer. Rust exposes that identity as
`campaign_description_index`. The separately serialized `list_index` is only
actor/UI list state and can refer to a completely different campaign entry.

AI entity views now resolve `in_coma` through `campaign_description_index` and
fail loudly if a production PC lacks a valid campaign identity. The
approaching-sleeping-enemy handler consequently recognizes the coma PC and
starts the authored menace interaction rather than issuing a downward killing
strike.

### MAP Move/Seek disables anti-collision at instruction time

After accepting `RHCOMMAND_MOVE` or `RHCOMMAND_SEEK`, Original inspects the
movement flags inside `RHElementActor::Instruct`. `RHMOVE_MAP` immediately
calls `SetAntiCollisionOn(false)` before Seek substitution, path translation,
or execution. This means a loaded PositionInterface can deserialize with
anti-collision enabled and then have the selected saved MAP movement disable
it again during ownership adoption.

Rust now applies that side effect at the same Instruct boundary in both the
ordinary sequence-manager path and the synchronous script-native path. It is
not inferred from a later concrete movement order, so pending paths and failed
paths retain the same state as Original.

### Menace and sleep transitions retain sequence ownership

Soldier `Translate` appends two orders for `START_MENACE` and `STOP_MENACE`,
and one for `STOP_SLEEP`. The owning sequence element remains selected and in
progress until the actor executes the final order. Rust previously terminated
these elements immediately after translation, orphaning their order queues and
making an idle `Wait` observable instead of the authored command.

These elements now retain ownership through animation completion. Together
with the resumed-strike, parry, facing, coma, sleeping-target, and campaign
identity corrections above, Linux3 Profile 003 Savegame 066 matches every
recorded frame.

### Nested route continuations finish the completed child Think

Original `DefaultGotoRoute(EVENT_REACHPOINT)` calls `SetState` synchronously
and resumes `InitializePatrol`, `Turn`, or `Think(EVENT_DONE)` before the
current `EndThink` returns. Rust must release the AI borrow before running the
equivalent state callback, so it represents the remainder as a typed route
continuation and closes the completed handler when that continuation resumes.

A recursively entered Think can still leave a suspended parent at that point.
The completed child therefore must process its completion flags without
requiring the controller's total recursion depth to be zero. This is the same
logical child `EndThink` boundary as Original; it neither drains nor completes
the suspended parent early.

With that invalid zero-depth assumption removed, every Linux3 Profile 003
recording in Savegames 067–075, Continue, ExQuickSave, QuickSave, Restart, and
Sherwood matches every recorded frame. The large states in this group include
Savegame 075 with 942 loaded elements and 595 dynamic elements.

### Nicouzouf Profile 001 loaded-session coverage

Every Linux replay in `Savegame_nicouzouf/Profile_001` has been exercised
through its complete recorded window. Savegames 000, 002, 006, 008–010,
013–023, 025–038, 040, 042–044, 046–048, 050, 052, 055–056,
058, 060–062, 064, 066–075 match every recorded frame. This includes dense
city states with 321 and 347 loaded elements.

The formerly excluded static Savegame 006 trace also received a frozen
release-mode recheck. Its 250 frames (61 through 311), 56-element save including
5 dynamic elements, and 299 authoritative RNG draws match exactly through EOF.
The static Savegame 009 trace also matches all 250 frames (95 through 345),
including its 59 saved elements, 8 dynamic elements, and 311 authoritative RNG
draws. Savegame 013 matches all 250 frames (121 through 371), including its 59
saved elements, 8 dynamic elements, and 327 authoritative RNG draws. Savegame
015 matches all 250 frames (68 through 318), including its 59 saved elements, 8
dynamic elements, and 326 authoritative RNG draws. Savegame 017 matches all 250
frames (56 through 306), including its 59 saved elements, 8 dynamic elements,
and 308 authoritative RNG draws. No replay-specific compatibility or code
change was required.

This completes the frozen recheck of the entire excluded static batch: 15
Linux3 Profile 003 traces and 5 nicouzouf Profile 001 traces, totaling 5,000
exactly matching frames and 9,223 authoritative RNG draws. Their loaded start
states collectively exercise 1,868 saved element entries, including 848 dynamic
element entries.

The remaining failures group into reusable behavior boundaries rather than
save-specific mismatches:

- Savegames 051 and 057 take the same extra rider anti-collision
  deviation at frame 106.
- Savegames 024, 041, and 049 retain `MoveOk` where Original selects `Turn`
  at frame 95; Savegames 039 and 076 expose related loaded Turn completion
  boundaries.
- Savegames 053, 059, and 065 disable the same eight motion-grid lines at
  frame 176.
- Savegames 045, 054, and 063 reach late sound/combat/AI activity before an
  RNG draw-count mismatch.

Savegame 076 also successfully decodes and atomically adopts the corpus's
largest save payload: 636 elements, including 289 dynamic elements.

### Retained detection stimuli preserve the Turn instruction barrier

Original `RefreshDetection` builds a FIFO of stimuli and invokes each `Think`
before the later `RHSequenceManager::Hourglass`. A script lock can retain those
stimuli until the NPC's queued-stimulus tail, but replaying them there does not
make `FaceTo` synchronous: every Turn remains only registered until the same
later manager hourglass. Nested `SetState`/`FilterAIEvent` callbacks inherit
that boundary as well.

Rust now carries the deferred-Turn mode through the filtered-Think and nested
owner-work drains, and uses it when replaying retained stimuli in the NPC tail.
This prevents an intermediate, subsequently halted `FaceTo` from writing a
direction goal before the final stimulus is processed. The Windows SuN1Sh1nE
Profile 004 Continue recording now matches every recorded frame.

### Condolation detaches the completed actor order before NPC Think

Original `RHElementActor::SendCondolationCard` clears the selected
`mpSequenceElement` and `mpOrder` before dispatching the NPC callback. Rust's
sequence manager had already deselected the terminal element, but its separate
actor animation latch still named the just-completed walking transition. A
patrol reaching its exact destination could therefore enter a recursive
`EVENT_REACHPOINT`, see a non-idle animation, and launch a zero-distance Move
instead of taking Original's synchronous GoTo shortcut.

Rust now clears the animation latch at the same actor-base condolence boundary,
while preserving an incoming replacement order when one is already selected.
All nested Think callbacks consequently observe `NONANIMATION_END` until they
install a real replacement. Nicouzouf Profile 001 Savegame 014 now matches
every recorded frame.

### Empty patrol formation updates still retire old history

Original `RHArtificialIntelligence::RefreshPatrol` calls
`RHPath::ComputePatrolPositions` on every eighth eligible frame even when the
active patrol list is empty but missed members remain. With a requested patrol
size of zero, the formation loop does no work, but the function's unconditional
post-loop cleanup discards every history entry except the newest. Only after
that cleanup does `RefreshPatrol` consider missed members for re-acquisition.

Rust previously skipped the formation call for an empty active list. It could
therefore retain a long stale trail, re-acquire a nearby missed member on that
same eighth frame, and have enough old history to coordinate the new member on
the very next formation update. Rust now invokes the formation computation for
zero members as well, preserving the Original's history-retirement side effect.
SuN1Sh1nE Profile 004 Savegame 008 and nicouzouf Profile 001 Savegame 022 now
match every recorded frame.

### Restored in-progress Turns retain their serialized sprite goal

A v48 save can contain an in-progress `TURN` sequence while the sprite still
holds the destination of the interrupted movement. On the first frame after
load, Original resumes that Turn directly. There is no outgoing movement
condolence in the actor slot, and Turn execution does not clear the sprite's
map goal, so the serialized destination remains observable throughout the
rotation.

Rust's runtime-only compensation for staged `FaceTo` callbacks cleared the map
goal whenever a Turn order initialized without an explicit retained-goal
property. That condition also caught restored in-progress elements, erasing
authoritative loaded state. The cleanup is now limited to ordinary runtime
Turns; legacy-restored elements preserve their adopted sprite goal. Windows
SuN1Sh1nE Profile 004 Savegame 038 now matches every recorded frame.

### Combat fighter scans include non-party PCs

Original rebuilds swordfight `mlistThem` from the complete camp fighter
registry, not from the current controllable party. Scripted and training PCs
therefore remain valid combat candidates even when they are absent from the
party's `pc_ids` selection list. Rust's combat snapshot now iterates every PC
entity and applies the ordinary active, radius, and fighting gates afterward.

The same path exposed a geometry error in `IsDetecting180Degrees`. Original
builds `GetDirectionVector()` with Y compressed by `ASPECT_RATIO`, then
stretches that Y by `INVERSE_ASPECT_RATIO` for the dot product. The result is
the plain unit direction table. Rust started from that already-uncompressed
table and stretched it again, incorrectly narrowing the forward half-plane.
It now uses the unit table directly. The nearby training PC at this boundary
is consequently admitted and selected exactly as in Original, and Windows
SuN1Sh1nE Profile 004 Savegame 014 matches every recorded frame.

### Swordfight preparation completes the interrupted command first

Original `RHElementActorHuman::PrepareToEnterSwordFight` calls
`Stop(RHPRIORITY_PREFERENCE)` before it dispatches
`EVENT_ENTER_SWORDFIGHT`. The stop reaches `SetState(INTERRUPTED)`, whose
`SendCondolationCard` callback is synchronous, so the old command's
`EVENT_DONE` or `EVENT_IMPOSSIBLE` reaction finishes in the old AI substate
before swordfight entry begins.

Rust queued that stop condolence while immediately dispatching the enter event.
An interrupted officer `POINT` could consequently deliver its old
`EVENT_DONE` after changing to `ATTACKING_SWORDFIGHT`, reinterpret the
completion as a swordfight heartbeat, and quit the fight before the engine
attached opponents. The preparation path now closes the stopped owner's
condolence boundary first.

### Swordfight reconsideration uses the 3D camp-fighter registry

Original `ReconsiderSwordfight` does not reuse the general nearby-fighter
query when rebuilding `mlistUs`. It walks every entry in
`marrayFighters[myCamp]`, keeps the actively swordfighting entries without an
`IsAbleToFight` gate, and compares `(UWORD)MaxNormDistance` with 500.
`MaxNormDistance` operates on the full 3D world positions and stretches world
Y by `INVERSE_ASPECT_RATIO`; projected map Y is therefore not equivalent for
actors at different elevations.

Rust now prepares a dedicated, registration-ordered friendly snapshot for
this one call site. The shared map-space `nearby_fighters` query remains
unchanged because its other consumers depend on its able-to-fight and
projected-radius contract. In Windows SuN1Sh1nE Profile 004 Savegame 024 this
admits the elevated friendly soldier that Original counts, restores the
missing `CombatReposition` RNG draw at frame 31, and advances parity to the
next independent mismatch at frame 35. Linux Profile 003 Savegame 053 remains
exact across the complete recording, guarding the shared fighter-cache
contract that an earlier broad geometry change disturbed.

### Raising a sword initializes the sprite goal in the launch frame

Original `RHSprite::PerformAction` copies every fresh order's
`pointDestination2D` into the sprite map goal. The order produced by
`ENTER_SWORDFIGHT` has the default zero destination, so it clears the stale
goal of a movement element that the stronger swordfight element postponed.

Rust's non-movement animation path is separate from `PositionInterface`, and
a recursively launched swordfight order can be installed after that actor's
animation owner slot has already run. Waiting for the next animation tick left
the old movement goal visible for one extra frame. The enter-swordfight
instruction now applies its raising-sword order's zero goal when it installs
the order, matching the Original launch-frame state. Windows SuN1Sh1nE
Profile 004 Savegame 024 advances from frame 35 to the next independent
mismatch at frame 36; Windows Savegame 014 and Linux Profile 003 Savegame 053
remain exact across their full recordings.

### Bow OUTOFVIEW handling preserves the Original switch fallthrough

Original's `EVENT_OUTOFVIEW` switch places the five active bow substates
(`OBSERVING_LOADING`, `OBSERVING`, `SHOOTING`, `LOADING`, and `AIMING`)
immediately before `_ANY_SWORDFIGHT_SUBSTATE_`. Unless the special
`enemy_seen_below` branch consumes the event, those labels deliberately fall
through the swordfight 360-degree test, then the moving-combat stare-vector
guard, and finally the common lost-enemy handler.

Rust previously sent these substates to the match default, which only rebuilt
the enemy list. An archer that genuinely lost its last target could remain in
`BOW_LOADING` instead of facing the missed NPC and entering the battle
overview. The explicit Rust arm now preserves every stage of the C++
fallthrough. This fixes Windows SuN1Sh1nE Profile 004 Savegame 024 frame 36
and advances it to the next independent RNG mismatch at frame 40. Windows
Savegame 014 and Linux Profile 003 Savegame 053 remain exact.

### ClearPatrol keeps the chief formation live through member callbacks

Original `RHArtificialIntelligence::ClearPatrol` walks the theoretical patrol
in order. For each member it clears `patrol_chief` and synchronously calls
`ForceReturnToDuty`; only after every nested member callback returns does it
clear the chief's theoretical, missed, and active patrol lists.

Rust previously cleared the chief lists before dispatching any returning-member
callback. Nested patrol/path decisions could therefore observe a formation that
Original still exposes, advancing a member an extra waypoint. Rust now
interleaves each member detach with its callback and clears the chief only after
the loop, matching the Original call boundary exercised by Linux Profile 001
Savegame 030.

### Waypoint scripts retain their enclosing Think stack

Original `RHArtificialIntelligence::ExecuteWaypointScript` invokes the
waypoint's `ReachPoint` VM from inside the route-arrival `Think`, then
recursively calls `Think(EVENT_AFTER_SCRIPT_GO_ON)` before the outer Think
returns. A native close-point `GoTo` made by that VM therefore sets the outer
call's `already_on_point` latch; the recursive Think resets the latch at its
own `StartThink` boundary instead of scheduling another reach-point event.

Rust must release the controller borrow before entering the VM and previously
also dropped the logical Think depth. The same close-point call was then
mistaken for an outside-Think call and queued an extra `EVENT_REACHPOINT`,
advancing a scripted patrol twice. Waypoint VM dispatch now retains a nested,
unwind-safe logical Think scope through `ReachPoint` and
`EVENT_AFTER_SCRIPT_GO_ON`. Linux Profile 001 Savegame 030 consequently passes
the former frame 30308 divergence and reaches its next independent mismatch at
frame 30378; Savegame 031 remains exact end to end.

### Non-enemy detection also runs for civilian NPCs

Original defines `RefreshDetection` on `RHElementActorNPC`; after the enemy
pass, the body, object, friend, missed-friend, and beggar passes therefore run
for both soldiers and civilians. Rust incorrectly required a soldier for the
entire post-enemy loop, leaving civilian detection state and callbacks stale.
The shared passes now operate on `NpcData`, and loaded FriendlyAI ownership is
backfilled for the corresponding civilian NPCs. Linux3 Profile 001 Savegame
030 matches through its former frame-30378 divergence and now reaches the next
independent mismatch at frame 30435.

### Legacy AI adoption restores both saved seek positions

Original saves serialize both `mposSeekPosition` and
`mposAlertSoldiersPoint`. The Linux-v48 decoder already read both values, but
the conversion and atomic-adoption layers omitted them, leaving loaded AI with
zero/default coordinates. Rust now carries both positions through the legacy
save plan and installs them in the live AI state. Linux3 Profile 001 Savegame
030 consequently advances from frame 30435 to its next independent mismatch at
frame 30465.

### Civilian alert reports launch their timer after changing state

Original's civilian `CALL_REPORT` handling enters the alert-report look state,
faces the report point, and only then launches the 30-frame timer. Rust launched
the timer before `SetState`; state replacement cancels active timers, so the
report never reached the same completion boundary. Rust now preserves the
Original ordering and uses the Original 3D `Face(RHposition)` operation at the
timer expiration boundary. Linux3 Profile 001 Savegame 030 advances from frame
30465 to frame 30495.

### Officer alert radius uses isometric max-norm distance

Original `AlertOfficer` calls `MaxNormDistance`, which stretches world Y by
`INVERSE_ASPECT_RATIO` before taking the max norm. Rust used raw map X/Y and
therefore admitted officers much farther away vertically on the isometric map.
The candidate scan now applies the same Y stretch and no longer writes
`alert_soldiers_point`, which Original `AlertOfficer` does not mutate. This
fixes Linux3 Profile 001 Savegame 030's frame-30495 branch and makes the entire
recording match exactly.

### SwitchToAlertPath closes its direct ReturnToDuty call synchronously

Original `SwitchToAlertPathByScript` installs the alert path and, for a
Default-state soldier, directly invokes the virtual `ReturnToDuty` method
before returning to the mission script. Rust previously queued an
`EVENT_RETURN_TO_DUTY`; besides resuming the VM too early, that translation
incorrectly routed the call through `FilterAIEvent`, which can reject it while
the soldier is moving. The native now yields to an engine-owned synchronous
barrier, invokes the enemy AI's `ReturnToDuty` method directly with a live
owner context, and closes its resulting AI/movement work before resuming the
VM. Nicouzouf Profile 001 Savegame 065 now consumes both patrol macro forecasts
at the Original call boundary and matches every recorded frame.

### Proud observers use live ally vision and 3D target distance

Original `GetNewPrimaryTarget` scores candidates with `Distance` and gates
them with `MaxNormDistance`; both operate on the full `RHposition`, stretching
Y for the isometric projection while retaining the elevation delta. Rust's
shared target selector previously used only projected X/Y. Original's
`IsTooProudToAttack` also asks each lower-pride observing soldier whether that
soldier detects the candidate, using the observer's current real view radius.
Rust substituted the deciding soldier's fixed standard radius. The selector
now retains Z in both norms, and the proud-observer branch uses the observing
soldier's live radius (while preserving the building gate). Linux3 Profile 001
Continue now stays in the proud-observer behavior at frame 471 and matches the
entire recording.

### Small Linux profiles match end to end

The complete `Savegame_linux` corpus currently consists of Profile 005
Continue/Restart and Profile 011 Savegame 000/Continue/Restart. All five
recordings decode and atomically adopt their Linux-v48 save payloads, then
match every recorded frame. These are tracked explicitly so later changes to
save adoption or mission restart do not silently regress the smaller profiles
while attention is focused on the larger save archives.

### `BeginSwordfight` evaluates its target at the live entity boundary

Original `RHArtificialMalignity::BeginSwordfight` reads the primary target's
current opponent list and action state immediately before issuing its Normal
priority `Stop`. Those values can change after the frame snapshot: in SuN1Sh1nE
Savegame 024, soldier 58 completed a start-running transition earlier in the
same creation-order walk. Rust tested the stale AI snapshot, skipped the Normal
stop, and its later Preference stop postponed different movement work. The
result first appeared as three missing combat RNG draws at frame 41. The AI now
emits a conditional stop intent and the engine applies both source gates
(`!IsSwordfighting` and Moving/MovingFast) against the live target when draining
that intent. The soldier-58 command/state and global RNG stream now match at
that boundary; the recording proceeds to an independent frame-41 divergence.

### Tower-guard completion preserves the synchronous battle boundary

SuN1Sh1nE Savegame 024 exposed three coupled omissions when tower guard 29's
Point command completed at frame 41. Original calls `TowerGuardCallAlert` and
then `BattleDecisions`; it does not force the guard directly into its Observe
state. The alert walks the stable same-camp soldier registry, measures
`SquareDistance` from world-horizontal `GetPosition()` coordinates (map Y plus
ground elevation), and synchronously delivers the cry even to inactive but
alive macro soldiers. Rust had iterated a HashMap and measured projected map Y,
therefore missing knight 152 on the higher level. The alert now follows the
camp snapshot order and source coordinate/body gates, so the knight enters the
expected Seeking/Turn state.

The subsequent battle decision also preserves two source orderings. It chooses
the guard's primary target from the personal Them list before appending targets
reported by nearby friends, and only friends admitted to `mlistUs` by the
360-degree detection gate may append their target. Rust previously injected
every attacking camp soldier and selected a nearer, unseen target, incorrectly
entering BowLoading instead of the source BowObservingLoading fallback. All
frame-41 state, command, direction, and RNG fields now match; the replay reaches
the next independent divergence at frame 53.

### `CALL_LOOKTHERE` faces the full authored position

At SuN1Sh1nE frame 53, soldiers 95 and 106 received the same positional look
hint while already facing sector 3. Original `Face(RHposition)` projects the
hint through `PositionToPoint3D`; its elevation contribution keeps sector 3 and
the idle `FaceTo` call completes synchronously. Rust used the explicitly 2D
face helper, derived sector 1, and launched two spurious Turn commands. The
standard look-there procedure now uses the shared 3D positional-face path, just
as its existing Focus operation already does. The replay advances to frame 58.

### Primary-target distance uses world Y as well as world Z

Original `GetNewPrimaryTarget` calls `Distance` and `MaxNormDistance` on the
actors' `GetPosition()` values. An actor's world-space Y is map Y plus its
elevation; after subtraction, the elevation delta is therefore present both in
the screen-plane Y component (before the inverse-aspect stretch) and separately
as the 3D Z component. Rust retained Z but measured the stretched component from
map Y alone. For actors on different levels this can reverse which enemy is
nearest. At SuN1Sh1nE frame 58, soldier 48 consequently selected the enemy at
`(357,1541)` and pointed to sector 10 instead of Original's enemy at `(520,1699)`
and sector 9. The shared primary-target selector now constructs the exact
world-space delta used by Original before applying either norm.

### Sword-strike areas use map coordinates, not world-ground coordinates

Original's lateral, half-circle, circle, and push victim collectors all build
their 2D strike geometry by subtracting `GetPositionMap()` values. The separate
`IsPossibleSwordStrikeVictim` gate retains its 3D belt-point reachability test;
elevation is not also folded into the strike vector's Y coordinate. Rust used
`ground_position()` for
three shared collectors, adding each actor's elevation to map Y and changing
both range and angular admission across sloped or stacked surfaces.
The circle-warning collector also now preserves Original's authored
attacker-minus-victim vector direction when deriving its approach tolerance;
the direction is observable there even though its length is symmetric.
All three collectors also retain Original's strict `MaxNorm() < 150` coarse
admission boundary rather than admitting an actor exactly 150 units away.

At SuN1Sh1nE frame 67 this admitted soldier 112 as a fourth victim of soldier
54's F sweep. Original hits only soldiers 103, 105, and 110. Interrupting the
extra victim's parry then triggered a synchronous swordfight reconsideration
and four unrecorded combat RNG draws. The hit, circle-warning, and push
collectors now retain map-space geometry exactly as the corresponding Original
methods do.

### Inactive scripted motion bypasses anti-collision

Original `RHSprite::PerformMotion` calls `UpdatePositionAntiCollision` only
when the owning actor is active. Inactive actors can still execute scripted
movement, but they commit the requested map increment directly and leave the
persistent deviation state untouched. Rust previously ran anti-collision for
those actors, which cleared a deviation restored from the save, snapped the
actor to the final waypoint, and later recomputed its facing from the
overshooting step. The movement driver now excludes inactive owners from both
ordinary and transition anti-collision, with the lower-level helper retaining
the same guard. Randomguy Profile 004 Savegame 031 and Linux2 Profile 002
Savegame 040 now match every recorded frame.

### Carried movement keeps its authored animation

Original movement dispatch preserves `WalkingWithCorpse` and
`WalkingCarryingOnShoulders` as the distance-producing animation. Rust
previously replaced either action with `WalkingUpright` whenever the actor's
state was merely `Moving`, doubling the per-frame distance in Linux3 Profile
001 Savegame 038. Both carrying actions now remain selected like the other
authored movement orders. The replay matches through the former frame-54407
boundary and reaches an independent soldier path divergence at frame 54445.

### Idle corpse carry preserves the carried surface

Original `WaitingWithCorpse` synchronizes the carried actor's animation and
display order but does not copy the carrier's obstacle. Rust copied the full
carrier surface every tick, so restoring a stationary Little John with no
obstacle erased his carried civilian's independently serialized obstacle and
225-unit elevation on the first frame. Idle corpse carry now retains that
surface while the existing moving and shoulder-carry synchronization remains
unchanged. Randomguy Profile 004 ExQuickSave now matches every recorded frame.

### Halt clears the goal of any detached selected actor command

Original `RHElementActor::SendCondolationCard` clears the sprite map goal when
the stopped element is the actor's selected `mpSequenceElement`; that rule is
not limited to movement elements. This matters when an AI calls `FaceTo` twice:
the first Turn can still own a running-to-waiting transition, and the second
Turn's leading `Halt` clears that selected Turn's inherited movement goal before
the replacement Turn is launched.

Rust's synchronous Halt boundary previously tracked only `active_movement`, so
it missed selected generic commands whose front order happened to be a movement
transition. Halt now snapshots the exact selected sequence element and clears
the goal only when `Stop(PREFERENCE)` actually detaches it. A movement element
rewritten in place to its exit transition remains live and therefore retains
its goal as before. That selected-element cleanup also invalidates queued
replacement snapshots of the old goal: once a later selected command has
cleared the goal, a delayed `MoveWaiting` path request must not resurrect it.
Linux3 Profile 001 Savegame 008 and the retained-goal Linux2 Profile 002
QuickSave both match every recorded frame; nicouzouf Profile 001 Savegame 024
passes the later path-request goal boundary exposed by the walking-style order
fix.

### Settled unconscious sword holds do not turn

Original `RHElementActorHuman::Execute` handles
`BEING_UNCONSCIOUS_SWORD` by calling `PerformAction`, applying the lying and
waiting-sword states on `START`, and then holding the order while the human is
unconscious. Unlike active sword-combat animations, this arm never calls
`Turn()`.

Rust's generic per-animation Turn table incorrectly included this settled hold.
An unconscious soldier restored with body direction 2 and direction goal 3
therefore rotated on the first replay frame even though Original retained
direction 2. Removing the hold from that table makes Linux3 Profile 001
Savegame 036 match every recorded frame.

### A soldier called by an officer approaches at walking speed

Original's `SUBSTATE_SEEKING_SOLDIER_CALLED_BY_OFFICER` timer calls
`GoNear(Position(mpAntagonist), 40)` without `GOTO_RUN`. This is intentionally
different from the separate return-to-officer path, which explicitly supplies
that flag. Rust had conflated the two paths and launched a running transition,
advancing six pixels where Original's walking transition advanced four.

The called-soldier path now preserves Original's default walking mode. Linux3
Profile 001 Savegame 038 matches through the former frame-54445 boundary and
reaches an independent officer-conversation progression divergence at frame
54506.

### Speech completion uses the concrete sound-manager resolution

Original queues `Say` requests into `RHSound`; the following sound Hourglass
selects a concrete speech entry, obtains its decoded length, and later invokes
`SoundIsFinished` from the fixed 25 Hz parity clock. A random speech group can
resolve to different lengths (or a zero-length gap), and forced variants do not
consume an audio RNG draw. The previous Rust path scheduled the maximum length
at `Say` time, while an attempted replay repair could not distinguish speech
selection from unrelated FX draws at the shared cache callsite.

Schema 12 records each ordered Pass-1 exclamation resolution with stable actor
identity, full and low-word exclamation IDs, selected variant/entry, and the
concrete decoded duration in frames. Ordinary play reports the same host
boundary to the simulation while retaining Original's separate Pass-3 sample
selection and audio RNG consumption. Rust drains matured callbacks before the
boundary events, settles zero-length callbacks inline, preserves FIFO, and
cancels unresolved requests on both `StopExclamation` paths. A fresh 250-frame
Savegame 038 capture records four resolutions and replaces the former
frame-54506 max-duration diagnosis; strict FIFO exposes an earlier latent
conversation-order difference at frame 54501 (Rust has already queued soldier 186's exclamation 77
ahead of Original soldier 172's exclamation 74). That next issue is AI
conversation scheduling, not sound-duration reconstruction.

### EnterSwordfight instruction preserves a postponed movement goal

Original `RHElementActor::Stop` retains an in-progress movement long enough to
play its transition to waiting. A stronger `EnterSwordfight` element can then
postpone that movement, leaving the sprite's existing map goal authoritative
until the raising-sword order actually executes. Rust previously cleared the
goal while merely translating/instructing `TransitionRaisingSword`, one actor
execution boundary too early.

EnterSwordfight instruction now only queues the order and marks the element in
progress; sprite destination side effects remain owned by order execution.
Linux2 Profile 002 Savegame 042 consequently preserves soldier 151's
`(768, 1796)` movement goal through the former frame-6987 boundary.

### AI position during door passage uses the committed gate side

Original `RHArtificialIntelligence::Position(actor)` does not expose an
actor's interpolating sprite coordinate while its selected command is
`PassDoor`. It reports the exact committed `point_in` or `point_out`, including
that side's sector and layer. This affects self-relative AI geometry, patrol
formation distance gates, and queued stimuli even though rendering continues
to show the actor moving along the door rail.

Rust now uses its shared, owner-slot AI entity view for both AI self position
and every `RefreshPatrol` actor snapshot. Final door-pass completion also
commits the exact gate endpoint before same-slot AI callbacks run. Linux2
Profile 002 Savegame 002 therefore skips the same obsolete frame-504 patrol
coordinate as Original, retains only the frame-512 update while BUSY, and
matches all 250 recorded frames exactly through frame 726.

### Carrying movement restores the PC waiting state on arrival

Ordinary `RHElementActor` walking leaves `MOVING` intact when motion terminates
and relies on an optional end transition to restore the waiting state. The PC
overrides for `WALKING_WITH_CORPSE` and `WALKING_CARRYING_ON_SHOULDERS` are
explicit exceptions: their `RHMOTION_TERMINATED` arms set `WAITING` directly.
Rust now applies that specialized completion behavior even for a
`NO_TRANSITIONS` movement. Linux3 Profile 001 Savegame 038 consequently
restores Little John's waiting state at the former frame-54511 divergence.

### Officer-directed searches retain the undefined direction default

When an instructed soldier acknowledges the officer, Original calls
`SeekArea` without its optional direction argument. Its default is
`UNDEFINED_DIRECTION`, causing the personal seek point to select its look
direction from the global RNG stream. Rust passed direction zero explicitly,
which silently selected a fixed direction and skipped the authoritative draw.
The translated handler now preserves the omitted-argument behavior for every
officer-directed search.

### VIP PC sword wounds apply the amulet coma save before speech classification

Original sword damage calls `GetWounded` virtually. For a VIP PC whose lethal
hit can consume an amulet, `RHElementActorPC::GetWounded` establishes the
5-life-point coma state through `SetLifePoints` before applying maximum
concussion. That `SetLifePoints` call emits `HERO_HURT` when the drop exceeds
twenty; the PC is neither dead nor unconscious at that callback boundary.
Rust previously let the shared damage primitive reach zero, emitted
`HERO_DIE`, and only then applied its existing coma save during post-damage
handling. Sword damage now closes the virtual PC wound boundary immediately,
preserves the 5-HP state for all later death classification, and emits the
same hurt expression before the unconscious speech gate. Linux3 Profile 003
Savegame 066 consequently matches the Original exclamation resolution at
frame 38597 and every recorded frame through EOF.

### Position authorization rejects boxes outside the map

Original `RHFastFindGrid::IsPositionAutorized` rejects a bounding box that
does not intersect the level's `mboxMap` before it queries motion lines. Rust
previously performed only the line query, so an actor wholly outside the map
could appear authorized. The later pathfinder extraction would then choose a
different nearby waypoint instead of the command-level extraction immediately
snapping the actor to an authorized position.

Rust now preserves the map-bounds gate. This restores the one-frame mounted
soldier correction shared by 13 recordings: Nescafe Profiles 001–003 Restart
and Profile 002 Continue; SuN1Sh1nE Profile 004 QuickSave and Savegame 038;
Linux3 Profile 001 Savegame 022 and Profile 003 Savegame 071; nicouzouf Profile
001 Savegames 014, 051, 057, and 075; and randomguy Profile 004 Restart. All
clear the former extraction boundary. Twelve match through recorded EOF;
Linux3 Profile 003 Savegame 071 advances to an independent frame-4601
`LookLeft` / AI-substate divergence.

## Frozen full-corpus audit at `2a3e842df`

The first no-unknown audit froze the debug parity runner built from commit
`2a3e842df` (SHA-256
`081fe21e80a2ffcb78863fb98f83d282273406ac2dc419c6b327ec28592adf1d`) before
launching parallel workers. This prevents later diagnostic edits or rebuilds
from changing semantics halfway through the count. The workers replayed all
103 previously unaudited recordings and the 14 then-known failures. Earlier
complete exact results supply the remaining classifications; the frozen runner
also rechecked the ten small-Linux/Linux3 Profile 002 traces and representative
Linux3 Profile 003 Savegame 053.

- 419 recordings match every recorded frame exactly.
- 26 recordings have a reproduced divergence or authoritative RNG/entity-map
  assertion.
- Zero recordings remain unaudited.

The shard totals are: 233 pass / 19 fail for the 252 Windows, nicouzouf, and
randomguy recordings; 46 / 2 for Linux2 Profile 002; 49 / 5 for Linux3
Profile 001; and 91 / 0 across the small Linux profiles plus Linux3 Profiles
002 and 003. The resulting corpus-wide exact rate is 94.2 percent.

The 26 frozen-run failures are assigned by shared cause rather than archive:

- **AI/sequence/Turn/state ordering (12):** Linux3 Profile 001 Continue,
  Savegames 008, 030, and 036; Linux2 Profile 002 Savegame 002; nicouzouf
  Savegames 024, 039, 041, 049, and 076; randomguy Savegames 030 and 038.
- **Movement/path/geometry (7):** Linux2 Profile 002 Savegame 040; Linux3
  Profile 001 Savegame 038; nicouzouf Savegames 053, 059, and 065; randomguy
  ExQuickSave and Savegame 031.
- **Combat/RNG ordering (4):** SuN1Sh1nE Savegame 024 and nicouzouf Savegames
  045, 054, and 063.
- **Runtime entity mapping (3):** randomguy Savegames 020, 029, and 032.

These counts describe the frozen `2a3e842df` baseline. Later commits may move
a recording to a new independent divergence before the next release-mode full
audit updates the totals.

The audit artifacts and individual logs live under
`.codex-tmp/full-corpus-audit-2a3e842df/`. Future large audits should use a
frozen release-mode runner after first proving one known-green trace matches
under both debug and release builds.

## Coverage limits

### Civilian macro flag cleanup follows the recursive command boundary

Original `CMD_RUN` and `CMD_WALK` recurse into the next macro command before
masking combat-only movement flags from a civilian's persistent patrol flags.
That ordering matters when the nested command finishes the macro and calls
`GoTo`: `muwLastGotoFlags` snapshots the raw flags first, while the movement
itself and the persistent flags are sanitized afterward. Rust's flattened VM
previously sanitized before the nested path completion. The two opcodes now
retain the Original recursive boundary; focused coverage checks both the raw
last-command snapshot and the masked emitted movement.

### Pending: patrol macro opcodes need synchronous engine continuations

Original `CMD_PATROL_START` clears the stopped flag, optionally says the
officer remark, calls `InitializePatrol()` synchronously, and only then
recurses into the next macro opcode. Rust currently clears the live patrol,
sets `needs_patrol_reinit`, and immediately continues the VM; the formation is
not rebuilt until a later patrol-coordination tick. A following
`CMD_PATROL_DIRECTION` consequently broadcasts to an empty patrol instead of
the freshly admitted and distance-sorted members.

`CMD_PATROL_DIRECTION` has the same boundary problem on its own. Original
iterates the live patrol and calls each member's
`GetInstructedPatrolDirection` before `ExecuteNextMacroCommand` returns to the
next opcode. Rust queues `direction_broadcast` but continues consuming macro
bytes, while the engine drains the broadcast only after the VM returns. The
existing engine comment promises the Original ordering, but the controller
does not actually yield there.

The general fix should add explicit engine-facing macro continuation barriers:
yield after `PATROL_START`, rebuild the patrol from the current owner-boundary
views, then resume the VM; yield after `PATROL_DIRECTION`, deliver and drain
the member calls in patrol order, then resume again. This is deliberately not
worked around with an extra timer or replay-specific ordering rule.

### Pending: patrol-path macro assignments need typed return-to-duty barriers

Original `AssignNewPatrolPath` begins with `BreakMacro` and, for an accepted
assignment, synchronously calls `Think(EVENT_RETURN_TO_DUTY)` when the AI is
not script-locked and its current state is `Default`. Consequently
`CMD_STAY_HERE` does not merely mutate the path and arrange work for a later
tick: its complete virtual `Think` boundary, including the concrete soldier or
civilian `ReturnToDuty` implementation and any recursively generated movement
events, closes before the opcode returns. Rust currently records a deferred
self stimulus in the controller outbox and returns from the macro VM first.

`CMD_CHANGE_WAY` has an additional shipped oddity. After
`AssignNewPatrolPath(index)` returns, Original explicitly calls `BreakMacro`
again and then calls the virtual `ReturnToDuty` unconditionally. On the normal
unlocked/default path this means the assignment's synchronous
`EVENT_RETURN_TO_DUTY` handling is followed by a second direct
`ReturnToDuty`; when script-locked or outside `Default`, the direct call still
happens even though the assignment did not dispatch the event. Rust currently
performs only the assignment and omits that second typed call entirely.

The general repair belongs in the same engine continuation mechanism as the
patrol-opcode fix: suspend the macro VM at these owner boundaries, drain the
real subtype-specific `Think`/`ReturnToDuty` work (and all synchronous child
effects) in Original order, then finish the opcode. Collapsing the calls into
`return_to_duty_common_stuff`, adding a timer, or special-casing a replay would
lose soldier/civilian virtual behavior and would not be equivalent.

### Invalid patrol assignments retain the Original's partial mutation

`AssignNewPatrolPath(index)` calls `BreakMacro` and sets
`mbHasPatrolPath = true` before checking whether the authored index is greater
than the hiking-path count. On rejection it returns without reinitializing the
path, leaving the flag set over the retained path state. Rust previously
validated first and skipped the flag write. The decoder remains strict about
structural corruption, but the runtime helper now preserves this shipped
engine error-path ordering for any authored or scripted invalid index.

### Friend-check fields preserve legacy integer narrowing

Relative `CheckForSync` indices are encoded around 1000. Original computes
`current + ((SWORD)encoded - 1000)` and then narrows the result to `UWORD`, so
a relative waypoint before zero wraps modulo 65,536. Rust previously clamped
negative results to zero. Both the pure synchronization branch and the later
wait setup now use the Original mixed-signedness conversion.

The authored wait duration similarly becomes
`mubNumberOfLooks = frames / interval + 1`: the integer result narrows to an
8-bit field rather than saturating at 255. Rust now retains that conversion
and no longer invents a one-look fallback for the zero/division error case.
Focused tests cover ordinary relative offsets, underflow, signed 16-bit input,
and the 8-bit look-count boundary.

### Panic seek-point scoring uses 16-bit distance arithmetic

Original `GetNearestSeekPointToFlee` stores both each candidate score and the
running minimum in `UWORD`. Its `+1000` sector-change and `+5000` directed-
panic penalties therefore wrap modulo 65,536; they do not saturate a 32-bit
score. The minimum starts at the `0xffff` sentinel and updates only for a
strictly smaller score, so a candidate scoring exactly `0xffff` is not
selected. Rust now preserves those rules, with coverage for both the sentinel
and penalty overflow.

### Nearby seek-point membership uses narrowed distances

Original `SetPosOnNearSeekPoint` converts the computed distance limit and
each candidate's MaxNorm distance from floating point to `UWORD` before the
strict comparison. It then adds the cross-layer penalty as wrapping 16-bit
arithmetic. Rust previously compared the floating-point values directly, so
ordinary fractional actor positions could change the candidate list and thus
the modulus applied to the global RNG draw. Rust now preserves the truncation
and wrapping rules; focused coverage checks both fractional membership and
layer-penalty overflow.

### Door selection preserves UWORD scoring and the infinite sentinel

Original `GetNearestDoor` narrows MaxNorm to `UWORD`, applies `+500` for a
sector change and `+300` for a layer change with 16-bit wrapping arithmetic,
and compares strictly against an initial `0xffff` minimum. Rust's panic and
arrow-reserve door searches had widened this to saturating `u32`, while the
indoor battle staging search compared raw floats and accepted its first
candidate unconditionally. The clean arrow-reserve and battle-staging paths
now share the legacy score helper and strict sentinel rule. The remaining
panic-door copy in `engine/ai/mod.rs` is intentionally pending while that
shared file has unrelated in-flight edits.

The arrow-reserve path also now enforces the Original's Lacklandist-only
dangerous-house check: an otherwise-best door is rejected when its interior
occupant list contains a PC, without advancing the running minimum.

A clean baseline proves exact parity only for the state fields serialized by the
recorder and the behaviors exercised by this session. When a divergence depends
on unrecorded state, extend the neutral trace schema rather than guessing from a
downstream symptom. New recordings should keep the same resolved-command,
mission-start, synchronous-path, global-RNG-stream contract unless this document
explicitly introduces and motivates another profile.
