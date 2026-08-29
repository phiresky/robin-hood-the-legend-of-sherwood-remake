# Per-mission achievement campaign envelopes

## Decision summary

- **Recommendation:** MERGE.
- **Implementation status:** Implemented on `codex/accepted-04-campaign-envelope` from
  accepted feature-03 commit `f46d5a557`; final validation evidence is recorded
  below.
- **Verification confidence:** High for typed aggregation, same-run isolation,
  replay retroactivity, reset retention, and incomplete-import behavior; medium
  for graphical layout until a shipping-data campaign is inspected manually.
- **Release dependency:** Satisfied by the accepted mandatory full-fidelity
  campaign-history foundation at `f46d5a557`. This feature deliberately does
  not restore aggregate-only Rust history or global achievement-bit
  compatibility.

## Accepted scope

The four existing mission achievements remain independent per-mission facts:
**Clean Hands**, **Ghost**, **Pile-o-Bones**, and **All Enemies Stashed**. The
accepted campaign/lifetime semantics are:

- Clean Hands and Ghost use `AllRequiredMissions`. A campaign badge is earned
  only after campaign completion and only when every mission in that completed
  path has eligible, host-attested evidence for the badge.
- Pile-o-Bones and All Enemies Stashed use `AnyMissionOnce`. One eligible
  mission permanently satisfies the lifetime envelope; repetition on every
  mission is not required.
- A successful practice replay can add a badge missed on an earlier attempt.
  It can fill a mission already in the completed path, but cannot enlarge the
  frozen path or combine evidence from different campaign run IDs.
- Per-mission icons remain visible independently of aggregate campaign status.
  Lifetime mission icons and aggregate awards survive replacement/reset of the
  current campaign slot.
- Incomplete Original-save evidence stays explicitly `Unverifiable`; import
  never converts an aggregate or global bit into mission evidence.

The owner confirmed that the required set is successful ordinary `Campaign`
attempts in the completed path. Failed, lost, and interrupted attempts remain
in full history but never expand the envelope. `HistoryReplay` attempts fill
but do not expand it. Playable Historical, Attack, Rescue, Ambush, and Tactical
profiles participate; HQ, Pseudo, and End profiles do not.

## What the player gets

Every won mission can show its four evidence-backed badge icons in the classic
map and campaign-progress presentations. The same views now show two separate
aggregate rows:

- **Campaign** reports the replaceable current run.
- **Lifetime** reports durable profile history across campaign resets.

Each aggregate reports a typed state (`IN PROGRESS`, `N/A`, `MISSING`, or
`MET`) and a concrete ratio when a requirement set exists. Any-once badges use
only `0/1` and `1/1`; they never display a misleading mission total such as
`3/1`. Aggregate status is kept separate from mission icons, so an incomplete
Original import is not presented as either an earned badge or a fabricated
failure.

After campaign completion, replaying one missed mission in the same campaign
can change Clean Hands or Ghost from missing to met. Progression and rewards are
rolled back, while the new attempt, attestation, and badge remain. Replacing the
campaign keeps lifetime icons and envelopes, but an archived lifetime record
does not falsely make a reset campaign node launchable: cross-reset replay is
not implemented by this change.

## Configuration and compatibility

Achievement calculation and full attempt evidence are mandatory storage, not a
lossy preference. Existing presentation and gameplay controls remain
independent:

| Surface | Default for a fresh profile | Compatibility / parity behavior |
| --- | --- | --- |
| Campaign badge presentation | On | A settings document without this field defaults it off; hiding it never erases evidence |
| Debrief evidence | On | Presentation only |
| Clean Hands, Ghost, Pile-o-Bones, and stash live trackers | Individually off | Presentation only |
| Detailed XP and speedrun tracker | Individually off | Presentation only |
| NPC-caused deaths invalidate Clean Hands | Off | Independent deterministic gameplay rule; Original parity keeps it off |

The aggregation policy is part of the achievement definition, not a user
preference: Clean Hands/Ghost are all-required and Pile/Stash are any-once in
every presentation. Disabling badge presentation leaves canonical evidence in
place so re-enabling it is lossless.

The profile campaign-history schema advances to carry completed campaign
envelopes. Under the accepted feature-03 storage contract, old native Rust
aggregate/history/global-bit compatibility is removed rather than guessed.
Only Original C++ saves import through the explicit incomplete-evidence path.
An all-Original import does not receive a fabricated Rust campaign run ID:
profile promotion remains deferred until the first native terminal supplies a
real run identity, at which point imported and native attempts can be archived
together.
Campaign history remains at accepted schema v2; the profile history advances to
schema v3. Because tracker state and its deterministic configuration are now
serialized, native saves advance to v58, replays to v18, and multiplayer to
protocol v25. Old native Rust schemas fail closed under the accepted storage
contract rather than guessing a migration. The player-command variant is still
appended so pre-existing variant indices do not move.

## Implementation map

- `crates/robin_engine/src/achievement.rs`
  - `AchievementAggregationPolicy` and stable policy mapping on
    `AchievementId`;
  - `AchievementAggregationStatus`, progress/input records, summary, and the
    shared `aggregate_achievement` evaluator.
- `crates/robin_engine/src/profiles.rs`
  - `MissionType::supports_mission_achievements` defines playable mission
    boundaries without achievement-name conditionals.
- `crates/robin_engine/src/mission.rs`
  - host-attested mission badge evidence and explicit incomplete-evidence
    classification.
- `crates/robin_engine/src/campaign.rs`
  - completed-path membership, Original completion boundary, campaign
    aggregation, and before/after aggregate unlock reporting on exact attempt
    attestation.
- `crates/robin_engine/src/campaign_history.rs`
  - `LifetimeCampaignAchievementEnvelope` freezes run ID, completion sequence,
    and sorted required mission IDs;
  - profile promotion, same-run lifetime evaluation, schema validation, and
    honest Original-import handling.
- `crates/robin_engine/src/player_profile.rs`
  - lifetime awards derive from typed profile history rather than obsolete
    global bits.
- `crates/robin_engine/src/engine/achievements.rs`,
  `engine/rollback_safe.rs`, and
  `crates/robin_rs/src/game_session/terminal_debriefing.rs`
  - pass canonical profile metadata into exact-attempt attestation.
- `crates/robin_rs/src/achievement_hud.rs`
  - aggregate presentation and honest incomplete-import summaries.
- `crates/robin_rs/src/campaign_progress.rs` and `campaign_map.rs`
  - current/lifetime rows, lifetime per-mission icon union, and replayability
    kept scoped to the current campaign.
- `crates/robin_engine/src/gameplay_config.rs`, `engine/global_options.rs`, and
  `player_command.rs`
  - independent presentation toggles plus the deterministic NPC-caused-death
    Clean Hands rule.
- `crates/robin_rs/src/save_file.rs`, `crates/robin_engine/src/replay.rs`, and
  `multiplayer.rs`
  - explicit fail-closed schema/protocol boundaries for the new deterministic
    tracker fields and command.
- `crates/robin_rs/src/ingame_menu/gameplay.rs` and
  `game_session/render.rs`
  - settings controls and optional live tracker rendering.

## Determinism and platform impact

- **Saves/profile storage:** Raw results and attestations remain the evidence
  source. A completed profile envelope stores only deterministic identities and
  membership; it does not duplicate mutable counters or awarded unions.
- **Replays:** History-replay attempts already carry the same frozen result and
  attestation. A replay can fill only a mission ID in the same run envelope.
  Replay-playback blockers continue to prevent an observed replay from awarding
  a local profile.
- **Rollback:** Aggregation is derived after an exact terminal attempt. The
  tracker and NPC-caused-death policy are deterministic rollback state; profile
  promotion remains host-side.
- **Multiplayer:** The authoritative host attests the same exact attempt key and
  computes aggregation from the canonical profile catalogue. No client-local
  badge union can create an award.
- **External verification:** Stable per-mission evidence remains available to
  a verifier. The profile UI envelope is not a substitute for authoritative
  replay resimulation.
- **Browser/native/Android:** Shared engine and renderer code is used on every
  platform. The envelope round-trips through both JSON and bitcode, while
  browser profile persistence remains JSON-envelope based.
- **Failure behavior:** Invalid zero/duplicate IDs, noncanonical ordering,
  conflicting envelopes, foreign/stale attempt keys, and unsupported profile
  schemas fail explicitly. Missing imported evidence produces `Unverifiable`,
  never a default boolean. An Original-only import without a native run ID is
  retained in the campaign rather than being assigned a synthetic identity.

## Original-game and research basis

Achievements and campaign envelopes are a port extension; the Original has no
achievement subsystem. Original source establishes the campaign boundaries the
extension must respect:

- `original-code/RHCampaign.cpp`, `RHCampaign::GetProgression` counts won
  missions and returns 100 immediately when H12 is won; optional catalogue
  exhaustion is not the completion rule.
- `original-code/RHMission.h`, `RHMission::IsDone` treats any non-available
  status as done, which is too broad for an achievement badge that exists only
  on successful attempts.
- `original-code/RHProfileManager.h` defines Historical, Attack, Rescue,
  Ambush, HQ, Pseudo, Tactical, and End mission profile classes. The extension
  explicitly excludes the navigation/non-run classes HQ, Pseudo, and End.

`docs/CAMPAIGN_HISTORY.md` and the accepted feature-03 implementation define
the append-only attempt, exact attestation, and practice-return contract used
here. The policy split itself is the owner's accepted post-port design.

## Verification evidence

Post-rebase verification on the exact Feature-03 base passed:

- `cargo check -p robin_engine` and `cargo check -p robin_rs`;
- `cargo test -p robin_engine achievement`: 21 passed;
- `cargo test -p robin_engine campaign::tests::`: 42 passed;
- `cargo test -p robin_engine campaign_history::tests::`: 15 passed;
- `cargo test -p robin_engine player_profile::tests::`: 17 passed;
- `cargo test -p robin_rs achievement_hud`: 5 passed;
- `cargo test -p robin_rs campaign_progress::tests::`: 5 passed;
- save/replay/network version guard tests: 3 passed;
- full `cargo test -p robin_engine`: 4,046 unit tests plus 14 integration
  tests passed; one documentation example remained intentionally ignored;
- full `cargo test -p robin_rs`: 1,037 library tests, 9 converter tests, and
  13 integration tests passed; one ffmpeg/libopus-dependent converter test
  remained intentionally ignored;
- `cargo build --bin robin` passed;
- `cargo fmt --all` and `git diff --check` passed.

The focused coverage includes:

- stable typed policy mapping and codec behavior;
- all-required versus any-once status and normalized 0/1 progress;
- campaign completion gating and all required mission badges;
- failed, lost, and interrupted attempts remaining in history without
  expanding the required won-mission set;
- an eligible practice replay filling a missed all-required badge without
  expanding the path;
- lifetime persistence across campaign replacement/reset;
- no cross-run union of partial Clean Hands/Ghost evidence;
- profile-envelope JSON and bitcode round trips;
- incomplete current-campaign and Original-save evidence remaining
  unverifiable;
- invalid/duplicate envelope data rejection;
- lifetime per-mission icons surviving reset without unlocking an archived
  replay;
- typed UI states and suppression of fabricated zero counters for incomplete
  imports;
- public engine-facade capability accounting and fail-closed native
  save/replay/network version guards.

No graphical shipping-data launch has been claimed.

## Risks and limitations

- Ambush and Tactical runs currently count as canonical playable missions. If
  product intent uses “canonical” to mean story-path profiles only, this one
  typed mission-category predicate must be narrowed.
- Lifetime icons survive reset, but cross-reset launch/replay is intentionally
  not exposed because the active engine cannot safely attach a new attempt to
  an archived campaign snapshot. Supporting that later requires a typed
  archived-run launch flow, not a UI-only selectable flag.
- The aggregate rows share the campaign-map header area with existing content.
  Automated layout tests cover the data but a shipping-data visual inspection
  is still required at supported aspect ratios and locales.
- Existing badge glyphs are code-native fallbacks. Replacement art remains an
  aesthetic follow-up, not missing achievement evidence.

## Merge recommendation

**MERGE.**
The implementation cleanly separates per-mission facts from campaign/lifetime
policy, freezes completed paths, prevents cross-run evidence mixing, supports
retroactive same-campaign practice, and keeps incomplete imports honest. The
owner has confirmed that only won missions on the completed path expand the
all-required envelope.
