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

The Rust extension keeps the original campaign behavior as the classic mode,
but adds an append-only `MissionAttemptHistory` to every campaign mission.
Native attempts freeze outcome, debriefing stats, duration, rules, achievement
results, wall-clock completion time, and whether the launch was ordinary
campaign progression or an isolated history replay. Derived totals and bests
are calculated from the immutable records, never serialized as a second source
of truth.

Achievement calculations and awards deliberately remain distinct parts of the
same canonical record. The deterministic terminal boundary freezes raw results;
the host then attaches an exactly-once eligibility attestation addressed by
campaign-run id and attempt sequence. Campaign, mission, and lifetime badge
unions derive only from attested eligible attempts. Blocked replay playback,
headless, custom, cheated, or disabled runs therefore remain auditable without
awarding an icon. The older achievement-only vector/global fields are read only
as migration compatibility for saves written before general attempt history.

At synchronization, native records are also promoted into a versioned
`ProfileCampaignHistory` owned by the player profile, outside replaceable save
slots and campaign resets. Promotion is idempotent using the deterministic
campaign-run id plus attempt sequence. The tree/museum shows current-campaign
and lifetime mission counts separately. A pre-history player profile keeps its
old score, ransom, preserved-lives percentage, play time, and progression in a
typed legacy aggregate; those totals are not misrepresented as a made-up
mission attempt.

An aggregate-only campaign is migrated by creating one explicitly incomplete
record for each mission whose old status is Won or Lost. Missing duration,
rules, timestamps, achievements, and statistics remain `None`; migration does
not manufacture zeroes. The migration is idempotent and runs for Original save
adoption and current-version JSON payloads missing the history fields.

Completed missions can be launched from history. Such a launch is practice:
the normal pre-selection campaign checkpoint is restored at the terminal
boundary, so ARES, score, money, gang state, inventory, and mission ageing
receive no second reward. The attempt record, including newly calculated
per-mission badges, is then appended to the restored campaign. This also makes
failed and interrupted practice attempts visible without corrupting campaign
progression.

`CampaignPresentationMode` selects among the original map, a prerequisite
graph, and the Sherwood Hall of Deeds. The latter is a deterministic walkable
exhibit grid using the existing campaign background, location art, and Richard
flag art as its visitor marker. Arrow keys walk between exhibits; Enter
inspects/launches an available mission or starts an isolated replay of a
completed one. Both new presentations share the exact same mission-selection
command and mission-description/return flow as the classic map.
