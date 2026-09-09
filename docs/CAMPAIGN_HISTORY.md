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
campaign-run id plus attempt sequence. Campaign shows progress and mission
badges from the current save. Hall of Deeds shows all recorded attempts,
permanent mission badges, and best results including archived attempts. Its
details label the current campaign's mission status separately; an archived
win does not unlock that mission in a new or earlier save.

Achievements shows each player-wide award once, with an Earned/Not earned
status and current-campaign progress underneath. Incomplete historical evidence
is explicitly marked unverified. Earned awards survive loading an older save.
Clean Hands and Ghost require their mission badge on every required mission
within one completed campaign; evidence from different campaign runs cannot be
combined into that award. Pile-o-Bones and All Enemies Stashed require one
qualifying mission. This uses the existing aggregation policies and does not
introduce a campaign picker or change award eligibility.

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
and follows the selected stage. A primary prerequisite route runs across the top
and stays visible while paging through branches underneath. Other story missions,
ambushes, and optional tactical missions are grouped by their prerequisite stage.
Training, campaign events, and the epilogue have distinct labels; the scripted
epilogue follows the finale. Gallery order follows those same story stages.
Unused `Impossible_mission` field slots are omitted unless they have prior
results, which remain inspectable without offering a nonexistent map. Field
mission completion totals exclude campaign events and the epilogue.
These are presentation rules, not new campaign prerequisites or unlock rules.
TODO: Replace legacy filename conventions for tutorial/outro/unused slots with
content-authored presentation metadata when available.

Hall of Deeds shows twelve entries per page.
Click once to select; double-click opens Mission Details, including locked missions.
Enter or the Inspect button opens an available mission in Sherwood; the main-menu and pause-menu views remain
browse-only. Arrow keys navigate, Page Up/Down or the mouse wheel change pages,
and the Previous/Next buttons work with mouse or touch. Tab switches tree/gallery;
A or the Achievements tab shows permanent awards and current campaign progress. Back/Escape
returns to the originating screen.

**D / Mission Details** replaces the Requirements tab. It combines original
localized mission text (the same short description and full briefing used by the
game) with actual entry restrictions and the complete recorded play history.
Missing briefings are explicitly identified. The mouse wheel scrolls the column
under the pointer independently; Page Up/Down scrolls that column by a screenful
(defaulting to the briefing column). Scroll indicators show each position. Left/Right
selects another mission. Back/Escape returns to the mission cards.

The right-hand list contains distinct plays from both the active save and the
player's permanent archive, newest first. Every play retains its outcome,
recorded date, duration, and campaign/practice designation; unavailable imported
values remain unknown. Scroll over the right column to browse every play;
Up/Down moves the selection and keeps the selected play visible.
Click selects a play; double-click, Enter, or Watch selected replay opens its
recording in a separate desktop game window, preserving the paused live session.
The viewer uses the normal replay loader and disables its RPC listener.

Native terminal recordings acquire an atomic host-only link under
`robin_hood/replays/attempts/`, keyed by campaign run and attempt sequence.
A background scan also indexes compatible older local JSONL recordings using
the embedded campaign and terminal command. Results appear without reopening
the screen. Missing, deleted, or unrecorded plays stay in history
with Recording unavailable. Unsupported recordings report the replay loader's
compatibility error. Browser recording persistence/viewer handoff and
indexing compact imports are not implemented; the browser still displays history.
Watching a recording uses normal playback eligibility and cannot earn awards.

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
last selections, an archived-record example, and the most crowded branch in
tree/gallery/achievement/mission-details views at 1024×768. Names, fonts, graph
structure, current availability, and lock reasons come from the supplied data;
archived records and achievement summaries are presentation-only fixtures.
The tool also writes mission-profiles.json and campaign-graph.json for inspecting
layout metadata. No save is applied or written. A wgpu adapter is required; Vulkan software rendering also works when
provided by the host. Captures fail explicitly if data, fonts, or the adapter are
missing. The ordinary tests check all 62 selections for non-overlapping cards,
viewport visibility, and matching pointer hit targets.
