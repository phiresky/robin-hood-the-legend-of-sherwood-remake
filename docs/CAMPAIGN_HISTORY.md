# Campaign history and mission selection

The original campaign serializes mutable mission status, aggregate campaign
values, and at most the three most recently played mission pointers. It does
not preserve a debriefing record. Relevant reference points are:

- `original-code/RHCampaign.cpp`: mission accessibility, ARES progression,
  mission ageing, and the three-entry `last played` list.
- `original-code/RHgame.cpp`: mission duration and terminal debriefing update
  order.
- `original-code/RHMissionStat.h`: the debriefing counters which used to be
  discarded after the screen closed.
- `original-code/RHMenuCampaignMap.cpp`: location selection and the required
  mission-description confirmation before launch.

Every Rust-created campaign owns an append-only `MissionAttemptHistory` for
every mission. Recording is part of the campaign schema, not a gameplay
option. Native attempts freeze outcome, debriefing stats, duration, rules,
achievement results, wall-clock completion time, and whether the launch was
ordinary campaign progression or an isolated history replay. Derived totals
and bests are calculated from the immutable records, never serialized as a
second source of truth.

Achievement calculations and awards deliberately remain distinct parts of the
same canonical record. The deterministic terminal boundary freezes raw results;
the host then attaches an exactly-once eligibility attestation addressed by
campaign-run id and attempt sequence. Campaign, mission, and lifetime badge
unions derive only from attested eligible attempts. Blocked replay playback,
headless, custom, cheated, or disabled runs therefore remain auditable without
awarding an icon.

At synchronization, native records are also promoted into a versioned
`ProfileCampaignHistory` owned by the player profile, outside replaceable save
slots and campaign resets. Promotion is idempotent using the deterministic
campaign-run id plus attempt sequence. The tree and modal exhibit grid show
current-campaign and lifetime mission counts separately.

Earlier Rust campaign, replay, and player-profile history schemas are not
migrated. They fail closed at their schema/version boundary so absent evidence
cannot silently become invented defaults. Only an Original C++ save may be
adopted. Its Won/Lost mission status and ordered three-entry recent-mission list
are converted to explicitly incomplete import records. Missing duration,
rules, timestamps, achievements, and statistics remain absent, and recent
launches have an `Unknown` outcome because the C++ save does not preserve it.
The imported recent list feeds the native attempt log once; it is not retained
as a second compatibility storage lane.

Completed missions can be launched from history. Such a launch is practice:
the normal pre-selection campaign checkpoint is restored at the terminal
boundary, so ARES, score, money, gang state, inventory, and mission ageing
receive no second reward. The attempt record, including newly calculated
per-mission badges, is then appended to the restored campaign. This also makes
failed and interrupted practice attempts visible without corrupting campaign
progression.

`CampaignPresentationMode` selects among the original map, a prerequisite
graph, and the Sherwood Hall of Deeds modal exhibit grid. Arrow keys navigate
between exhibits; Enter inspects/launches an available mission or starts an
isolated replay of a completed one. Classic Map exposes the same history and
practice flow through its History & Practice action. The freely walkable,
world-space Hall remains deferred.
