# Campaign history and mission selection

The original campaign serializes mutable mission status, aggregate campaign
values, and at most the three most recently played mission pointers. It does
not preserve a debriefing record. Relevant reference points are:

- Original-game campaign behavior: mission accessibility, ARES progression,
  mission ageing, and the three-entry `last played` list.
- Original-game debriefing behavior: mission duration, update order, and counters.
- Original-game campaign-map behavior: location selection and required
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
cannot silently become invented defaults. Only an original-game save may be
adopted. Its Won/Lost mission status and ordered three-entry recent-mission list
are converted to explicitly incomplete import records. Missing duration,
rules, timestamps, achievements, and statistics remain absent, and recent
launches have an `Unknown` outcome because the legacy save does not preserve it.
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

During any mission, including before reaching Sherwood, **Escape → Campaign
Manager** opens the same progress tree and Hall of Deeds. Tab switches views;
arrows or a click select a mission and display its history and badges. Mission
launching is disabled in this pause-side view. Escape returns to the pause
menu; the active mission, campaign checkpoint, and rewards are unchanged.

The main menu also has a **Campaign Manager** entry, using the same browse-only
UI. It reads the selected player's latest resumable checkpoint; players without
one see a fresh campaign and their lifetime history. Escape returns to the main
menu. Viewing the campaign does not start a mission or apply a save.


## Campaign manager UI and offscreen captures

The campaign manager uses a fixed **1024×768** canvas. Both entry points and
the Sherwood history screen share its renderer. The tree keeps fixed-size cards
and follows the selected stage/row; Hall of Deeds shows twelve missions per page.
Click once to select and read details. Enter, the Inspect button, or a double-click
opens an available mission in Sherwood; the main-menu and pause-menu views remain
browse-only. Arrow keys navigate, Page Up/Down or the mouse wheel change pages,
and the Previous/Next buttons work with mouse or touch. Tab switches tree/gallery;
A or the Achievements tab shows campaign and lifetime badge progress. Back/Escape
returns to the originating screen.

An opt-in screenshot test uses the real production renderer with an offscreen
wgpu texture. It creates no window and needs neither a display server nor Xvfb.
Use a full-game legacy data directory (the shipping loader currently rejects
absolute datadir paths in this test setup). From the worktree root:

    RUST_LOG=error \
    ROBINHOOD_DATA_DIR=/absolute/path/to/datadirs/fullgame_gog \
    ROBIN_UI_CAPTURE_DIR=target/campaign-ui \
    cargo test -p robin_rs --lib campaign_map::capture_tests::capture_campaign_ui -- --ignored --exact --nocapture

Run this capture test alone: it sets its process working directory to the worktree
root for install-resource lookup. The PNG matrix includes the first, middle, and
last selections of the real campaign in tree/gallery/achievement views at 1024×768.
Names, fonts, and graph structure come from the supplied data; mission statuses
are a presentation-only fixture covering all five states. No save is applied or
written. A wgpu adapter is required; Vulkan software rendering also works when
provided by the host. Captures fail explicitly if data, fonts, or the adapter are
missing. The ordinary tests check all 62 selections for non-overlapping cards,
viewport visibility, and matching pointer hit targets.
