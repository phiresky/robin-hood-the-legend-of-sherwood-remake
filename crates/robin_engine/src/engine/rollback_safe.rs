//! Cross-crate `Engine` facade that makes the
//! "mutations-only-inside-the-tick" invariant mechanical.
//!
//! Downstream crates only ever see the [`Engine`] wrapper defined here.
//! It gives read-only access to the underlying [`EngineInner`] via
//! `Deref`; there is no `DerefMut` and no accessor returning
//! `&mut EngineInner`, so the only way to mutate simulation state from
//! outside `robin_engine` is through an explicit method on this type.
//!
//! Each exposed mutator is either:
//!
//! * a tick call (`apply_command(s)`, `perform_hourglass`) — the normal
//!   per-frame sim-state mutation point,
//! * a one-shot setup / level-load / lifecycle hook, or
//! * a drain of a side-effect queue filled during the tick and consumed
//!   host-side.
//!
//! Anything that doesn't fit one of those buckets should be pushed into
//! the sim via `PlayerCommand` / a dedicated tick path, not added here.

use std::ops::Deref;

use super::{
    ConsoleResponse, DevState, EngineError, EngineInner, InputState, LevelAssets, PendingLevelData,
    SideEffects,
};
use crate::campaign::Campaign;
use crate::element::EntityId;
use crate::minimap::HitMask;
use crate::player_command::{PlayerCommand, PlayerInput};

/// Parallel runtime array whose length must match the loaded level geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotGridComponent {
    Lines,
    Sectors,
    Masks,
}

impl std::fmt::Display for SnapshotGridComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Lines => "lines",
            Self::Sectors => "sectors",
            Self::Masks => "masks",
        })
    }
}

/// A decoded snapshot is incompatible with the already-loaded mission.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotRestoreError {
    #[error(
        "snapshot fast-grid {component} runtime length {snapshot_len} does not match loaded level length {level_len}"
    )]
    FastGridLengthMismatch {
        component: SnapshotGridComponent,
        snapshot_len: usize,
        level_len: usize,
    },
    #[error("snapshot world invariant failed: {detail}")]
    WorldInvariantViolation { detail: String },
    #[error("snapshot order invariant failed: {detail}")]
    OrderInvariantViolation { detail: String },
}

/// Cross-crate owner of the simulation engine.
///
/// Downstream crates get `&EngineInner` via `Deref` and may only mutate
/// through the methods below.  There is no `DerefMut`, no accessor
/// returning `&mut EngineInner`, and `EngineInner::new` is
/// `pub(crate)`, so no alternative construction path leaks out either.
///
/// Internally (inside `robin_engine`) code still uses `EngineInner`
/// directly — the safety invariant is between the crate and its
/// downstream consumers, not a per-module check.
#[derive(Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash)]
#[serde(transparent)]
pub struct Engine {
    inner: EngineInner,
}

/// Level-load parameters for [`EngineArgs::level`].
///
/// The host is responsible for pre-loading the mission binaries
/// ([`crate::engine::level_loading::load_mission_for_campaign`]) and
/// pre-decoding the background bitmap (via the host-side
/// `pre_decode_background_map` helper) *before* calling
/// [`Engine::new`].  This lets the constructor size the grid
/// (`set_level_size`), ingest motion data, and run AI init with a
/// fully-populated `fast_grid` — instead of the previous split-init
/// pattern where `map_bbox` was zero and every patrol path failed
/// `TestIfPathIsFine`.
pub struct LevelLoadArgs<'a> {
    pub assets: &'a mut LevelAssets,
    pub level_directory: &'a str,
    pub progress: &'a mut dyn FnMut(f32),
    /// Pre-parsed mission + proto-level binaries.  See
    /// [`crate::engine::level_loading::load_mission_for_campaign`].
    pub loaded: crate::level_data::LoadedLevel,
    /// Background bitmap pixel dimensions, derived from the host's
    /// pre-decoded `PreDecodedBackground`.  Drives
    /// `FastFindGrid::size_map` and `CameraState::set_level_size` —
    /// both need real dims so `is_position_authorized` /
    /// `TestIfPathIsFine` work during `init_ai`.
    pub bg_pixel_dims: (f32, f32),
}

/// Ground-mark sprite metadata: sprite half-diagonal (half-width,
/// half-height) in world pixels and per-frame `(w, h)` sizes, used to
/// build the marker's move-box and on-screen culling rectangle.
///
/// `per_frame_offsets` is the per-frame `(x_min, y_min)` of the opaque
/// region, recorded when each frame is auto-cropped against the
/// `0x07C0` colour key.  The on-screen test adds this to the sprite's
/// top-left before testing the AABB, so plumbing it keeps the cull
/// rectangle aligned with the opaque pixels instead of biased by the
/// transparent border of the uncropped surface.
///
/// Host pre-computes this from the `RHID_GROUND_FOCUS` resource in
/// DEFAULT.RES and hands it to [`Engine::new`] so the sim can place
/// destination markers during the very first tick.
#[derive(Default, Clone)]
pub struct GroundMarkSpriteData {
    pub half_w: f32,
    pub half_h: f32,
    pub frame_sizes: Vec<(u16, u16)>,
    pub per_frame_offsets: Vec<(i16, i16)>,
}

/// Minimap corner-button widget setup: corner-sprite dimensions plus
/// the pixel-level hit mask built from frame 1 of `RHMAP_CORNER`.
/// Engine uses the canonical director view for this legacy minimap
/// setup; local widget placement lives host-side.
pub struct MinimapWidgetSetup {
    pub corner_size: crate::coordinates::ScreenSize,
    pub button_hit_mask: Option<HitMask>,
}

/// Arguments for [`Engine::new`].
///
/// Every field is required: a live `Engine` is defined as
/// "fully initialised for mission play", so construction requires the
/// host to already have loaded mission binaries, pre-decoded the
/// background bitmap, and gathered HUD/widget sprite metadata.  Test
/// and save-restore code paths that want a bare engine construct
/// `EngineInner` directly (it stays `pub(crate)` for that internal
/// use) or pass a test-fixture level through this same path.
pub struct EngineArgs<'a> {
    pub campaign: Campaign,
    pub level: LevelLoadArgs<'a>,
    /// Sprite metadata for the destination-marker ground mark (read
    /// from `RHID_GROUND_FOCUS`).  Used by `add_mark` to offset the
    /// click position and by the per-frame animation tick.  `None`
    /// when the host didn't find the resource (leaves the marker
    /// disabled).
    pub ground_mark_sprite: Option<GroundMarkSpriteData>,
    /// Per-row frame counts for the titbit sprite table.  Indexed by
    /// `SpriteRow` discriminant.  Used by `TitbitManager::num_frames_for_row`
    /// during animation.  Host pre-computes from DEFAULT.RES.  Empty
    /// when the resource is absent.
    pub titbit_row_frame_counts: Vec<u16>,
    /// Initial RNG seed.  Applied as the *first* mutation inside
    /// `Engine::new`, before any setup that draws from the engine's
    /// PRNG (entity spawn, AI init, mission script `StartUp`).  In
    /// single-player this is `0` (the historical default); in
    /// multiplayer it's the host-negotiated `mp_mission_seed`; under
    /// `--replay` it's the recording's header seed.  Threading the
    /// final seed through the constructor — instead of restoring it
    /// post-`Engine::new` — guarantees the engine's frame-0 state is
    /// a deterministic function of `EngineArgs` alone, with no
    /// SP↔MP-host divergence from RNG-consuming work between the
    /// two restore points.
    pub rng_seed: u64,
    /// AI GoldenEye cheat flag.  Set on the engine before any AI
    /// init runs.  Threaded as a constructor param (rather than a
    /// post-init `SetGoldenEyeMode` dispatch) so the local engine's
    /// frame-0 state matches what `InitialSnapshot` captures and
    /// what joining peers adopt — bypassing the
    /// `dispatch_local_command` wire-delay path that previously
    /// caused MP-host's frame-0 hash to lag the recording by
    /// `INPUT_DELAY_FRAMES`.
    pub goldeneye: bool,
}

impl Engine {
    /// Reattach immutable level assets that are intentionally outside
    /// serialized engine snapshots. Used by multiplayer snapshot adopt
    /// and save/load restore after decoding an `Engine`.
    pub fn attach_level_assets(&mut self, assets: &LevelAssets) {
        self.inner.attach_level_assets(assets);
    }

    /// Create a fully-initialised engine for mission play.
    ///
    /// The host is expected to have:
    ///
    /// 1. Built `Campaign` + selected the current mission.
    /// 2. Loaded the mission binaries via
    ///    [`crate::engine::level_loading::load_mission_for_campaign`].
    /// 3. Pre-decoded the background bitmap (host-side helper) and
    ///    recorded its pixel dimensions.
    /// 4. Optionally pre-decoded the minimap bitmap.
    ///
    /// With those in hand, this constructor runs every step the old
    /// split `Engine::new` + `apply_level_bitmaps_loaded` pair used to
    /// do — `initialize_from_campaign` (entity spawn, mission script),
    /// `set_level_size`, `consume_pending_motion_data` (pathfinder
    /// graph + grid sector registration), `initialize` (AI init, which
    /// now sees a real `map_bbox` + half-diagonals table), mission
    /// script `StartUp::Initialize`, and — for Sherwood —
    /// `apply_production_sector_data`.
    ///
    /// Returns `Err` only when mission data fails to ingest.
    pub fn new(args: EngineArgs) -> Result<Self, EngineError> {
        Self::new_preserving_campaign(args).map_err(|(error, _campaign)| error)
    }

    /// Create a fully-initialised engine while preserving ownership of the
    /// supplied campaign if mission ingestion fails.
    ///
    /// `EngineArgs` consumes its campaign. Callers which are themselves
    /// ownership boundaries (notably mission bootstrap) must use this variant
    /// so an initialization error cannot silently drop the one live campaign.
    /// [`Engine::new`] remains the convenience wrapper for callers which do
    /// not need to recover that value.
    pub fn new_preserving_campaign(
        args: EngineArgs,
    ) -> Result<Self, (EngineError, crate::campaign::Campaign)> {
        let mut inner = EngineInner::new();
        // Seed the PRNG and apply engine-global cheat flags FIRST,
        // before any setup that might draw from the RNG or branch on
        // the cheat flag.  See `EngineArgs::rng_seed` /
        // `EngineArgs::goldeneye` docs for the rationale.
        inner.restore_rng_from_seed(args.rng_seed);
        inner.set_golden_eye_mode(args.goldeneye);
        inner.install_campaign(args.campaign);
        if let Some(gm) = args.ground_mark_sprite {
            inner.set_ground_mark_sprite_data(
                gm.half_w,
                gm.half_h,
                gm.frame_sizes,
                gm.per_frame_offsets,
            );
        }
        if !args.titbit_row_frame_counts.is_empty() {
            inner.set_titbit_row_frame_counts(args.titbit_row_frame_counts);
        }
        let LevelLoadArgs {
            assets,
            level_directory,
            progress,
            loaded,
            bg_pixel_dims,
        } = args.level;
        // The proto-level (motion sectors) loads before the mission
        // file (beam-mes / soldiers / civilians).  We thread
        // `bg_pixel_dims` into `initialize_from_campaign`, which calls
        // `set_level_size` + `consume_pending_motion_data` mid-load
        // (right after the proto data is stashed in constructor-local
        // pending data, but before any entity that references a sector
        // spawns) so that beam-me sector validation and downstream
        // sector-handle resolution see the populated grid.
        let mut pending = PendingLevelData::default();
        if let Err(error) = inner.with_sim_rng(|inner| {
            inner.initialize_from_campaign(
                assets,
                &mut pending,
                loaded,
                level_directory,
                bg_pixel_dims,
                progress,
            )
        }) {
            let campaign = inner.take_campaign();
            return Err((error, campaign));
        }
        inner.populate_sector_gates_from_doors();
        inner.resolve_patch_mask_refs(&mut pending);
        // AI init runs HERE — after pathfinder + grid are fully
        // populated, so `TestIfPathIsFine` / `is_position_authorized`
        // see real `map_bbox` + motion lines and patrol paths validate
        // correctly.
        inner.initialize(assets);
        assets.level_grid = inner.world.fast_grid.level.clone();

        // Mission script StartUp::Initialize — `hiking_paths` was
        // just populated by the level loader.
        inner.with_sim_rng(|inner| {
            inner.initialize_mission_script_with(assets, 0, &assets.hiking_paths)
        });

        // Sherwood-only: spawn production bonuses at the registered
        // points.
        let is_sherwood = inner
            .campaign()
            .and_then(|c| c.current_mission_idx.map(|i| (c, i)))
            .map(|(c, i)| {
                c.missions[i].profile(&assets.profile_manager).location
                    == crate::profiles::MissionLocation::Sherwood
            })
            .unwrap_or(false);
        if is_sherwood {
            inner.with_sim_rng(|inner| {
                inner.apply_production_sector_data(assets);
                // Fire the "production-sector data is ready" hook
                // (`SendMessage(0, 1001)`) the Sherwood StartUp script
                // listens for on fresh Sherwood entry.  The LevelLoad twin
                // is handled via the post-load fixup path; this arm covers
                // fresh entry only.
                inner.dispatch_startup_message(assets, 1001, 0, 0);
            });
        }
        inner
            .world
            .validate_level_attachments(assets, inner.script_domains.zones.scripts.len());
        Ok(Self { inner })
    }

    /// Test-only shortcut: build an `Engine` with an empty fixture
    /// level.  Equivalent to the old
    /// `Engine::new(EngineArgs { ..Default::default() })` spelling
    /// that disappeared when `Engine::new` went RAII.
    ///
    /// Used from unit tests that want an engine for serde round-trip,
    /// command-pipeline, or HUD testing without loading a real
    /// mission from disk.  Not suitable for anything that touches the
    /// pathfinder, motion grid, or AI — the fixture level has no
    /// entities, no motion data, and no pathfinder graph.
    pub fn new_for_test(
        screen_width: f32,
        screen_height: f32,
        campaign: Campaign,
        assets: &mut LevelAssets,
    ) -> Result<Self, super::EngineError> {
        Self::new_for_test_with_level_size(screen_width, screen_height, campaign, assets, 0.0, 0.0)
    }

    /// Variant of [`Engine::new_for_test`] that lets the caller set
    /// non-zero map dimensions — needed by tests that touch the
    /// cutscene camera's zoom / scroll clamps, which key off `level_size`.
    pub fn new_for_test_with_level_size(
        _screen_width: f32,
        _screen_height: f32,
        campaign: Campaign,
        assets: &mut LevelAssets,
        map_width: f32,
        map_height: f32,
    ) -> Result<Self, super::EngineError> {
        use crate::mission::Mission;
        use crate::profiles::MissionProfile;

        let mut campaign = campaign;

        // `initialize_from_campaign` expects `current_mission_idx`,
        // `campaign.missions[idx]`, and `profiles.missions[profile_idx]`
        // all to resolve.  When the caller hasn't populated any of
        // those (the common test case), plant a minimal fixture entry
        // at index 0.  We mutate `assets.profile_manager` via
        // `Arc::make_mut` so callers that share the same profiles Arc
        // pick up the fixture.
        if assets.profile_manager.missions.is_empty() {
            let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
            profiles.missions.push(MissionProfile::default());
        }
        if campaign.missions.is_empty() {
            campaign.missions.push(Mission {
                profile_idx: Some(0),
                ..Mission::default()
            });
        }
        if campaign.current_mission_idx.is_none() {
            campaign.current_mission_idx = Some(0);
        }

        let loaded = crate::level_data::LoadedLevel::empty_for_test();
        Self::new(EngineArgs {
            campaign,
            level: LevelLoadArgs {
                assets,
                level_directory: "",
                progress: &mut |_| {},
                loaded,
                bg_pixel_dims: (map_width, map_height),
            },
            ground_mark_sprite: None,
            titbit_row_frame_counts: Vec::new(),
            rng_seed: 0,
            goldeneye: false,
        })
    }

    // ── Tick ────────────────────────────────────────────────────────

    /// The per-frame simulation tick. The ONLY per-frame sim-state
    /// mutation point; rollback replay re-runs this on a cloned engine
    /// and must see bit-identical results.
    pub fn perform_hourglass(
        &mut self,
        display: &mut super::HostDisplayState,
        assets: &LevelAssets,
        dev: &mut DevState,
    ) -> SideEffects {
        self.inner.perform_hourglass(display, assets, dev)
    }

    /// One-shot lifecycle stage matching `RHGame::GameLoop`: dispatch
    /// mission `PostInitialize` after the first host refresh and sound
    /// hourglass.  Replay drivers must run this after reconstructing
    /// frame zero as well.
    pub fn perform_post_initialize(
        &mut self,
        display: &mut super::HostDisplayState,
        assets: &LevelAssets,
    ) -> Option<SideEffects> {
        self.inner.perform_post_initialize(display, assets)
    }

    /// Apply one player command.  Commands are the only host → sim
    /// channel that mutates serialised state outside `perform_hourglass`.
    pub fn apply_command(
        &mut self,
        display: &mut super::HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        cmd: &PlayerCommand,
    ) {
        self.inner.apply_command(display, input, assets, cmd);
    }

    /// Apply a batch of player commands, as used by the replay driver
    /// and the rollback checker.
    pub fn apply_commands(
        &mut self,
        display: &mut super::HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        cmds: &[PlayerInput],
    ) {
        self.inner.apply_commands(display, input, assets, cmds);
    }

    /// Apply a batch of locally-sourced commands (live single-player
    /// host pipeline). See [`EngineInner::apply_local_commands`].
    pub fn apply_local_commands(
        &mut self,
        display: &mut super::HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        cmds: &[PlayerCommand],
    ) {
        self.inner
            .apply_local_commands(display, input, assets, cmds);
    }

    /// Fire the `DIES IRAE` (`EZEKIEL_2517`) cheat if active: instakill
    /// the target when the host's alt-hover gesture lands on a live
    /// human.  Returns `true` when the cheat consumed the gesture (the
    /// host should NOT then set `host.selected_view_element`).
    ///
    /// This is a cheat shortcut — the instakill is a sim-state mutation
    /// that bypasses the normal command recording.  Rollback replay of
    /// frames spanning the cheat activation may desync for one
    /// window; acceptable since EZEKIEL is a dev-only toggle rarely
    /// triggered during normal play.
    pub fn try_ezekiel_instakill(&mut self, id: EntityId) -> bool {
        self.inner.try_ezekiel_instakill(id)
    }

    /// Host-side entry point for injecting a `SimpleMessage` onto the
    /// engine messenger.  Used by UI sites that forward messages (console
    /// hide, switch-task, …); the drain handler in
    /// `perform_hourglass_inner` is the sole consumer.
    pub fn send_simple_message(&mut self, msg: crate::messenger::SimpleMessage) {
        self.inner.send_simple_message(msg);
    }

    // ── Setup / lifecycle ──────────────────────────────────────────

    pub fn install_campaign(&mut self, campaign: Campaign) {
        self.inner.install_campaign(campaign);
    }

    /// Mutable access to the mission script's `GameHost`. Exposed
    /// so the host's Lua scripting layer (`robin_rs::lua_session`)
    /// can drive custom-mission Lua events against the same
    /// `GameHost` the `.scb` VM uses. Only safe to call from
    /// script-event windows (right after `swap_engine_state`
    /// install, before swap-out) — see the doc on
    /// [`EngineInner::mission_script_game_host_mut`].
    pub fn mission_script_game_host_mut(&mut self) -> Option<&mut crate::natives::GameHost> {
        self.inner.mission_script_game_host_mut()
    }

    /// Run a host-side mission-script extension against the live `GameHost`
    /// while the Engine-owned simulation RNG is installed.
    ///
    /// Spellforge startup is outside the normal engine tick but its native
    /// shims can still draw from `sim_rng`; using this boundary advances the
    /// one authoritative stream instead of panicking for lack of a scope or
    /// inventing a second RNG. The closure must not retain the host reference.
    pub fn with_mission_script_game_host_and_rng<R>(
        &mut self,
        f: impl FnOnce(
            Option<(
                &mut crate::natives::GameHost,
                &mut crate::natives::ScriptState,
                &crate::natives::AttachedScriptBindings,
                crate::natives::NativeQueryViews<'_>,
            )>,
        ) -> R,
    ) -> R {
        self.inner.with_sim_rng(|inner| {
            let queries = native_query_views!(inner);
            f(inner.mission_script.as_mut().map(|script| {
                (
                    &mut script.game_host,
                    &mut script.state,
                    &script.bindings,
                    queries,
                )
            }))
        })
    }

    pub fn take_campaign(&mut self) -> Option<Campaign> {
        self.inner.take_campaign()
    }

    /// Run a console-cheat input and return the dispatch response
    /// directly.  Console cheats are dev escape-hatches outside the
    /// command pipeline (not replay-tracked), so the response is
    /// transient UI text — no rollback-hash concern with returning it.
    ///
    /// `selected_view_element` is the host's alt-hover UI selection —
    /// cheats that operate on "the NPC you're currently viewing" read
    /// (and sometimes clear) it.
    pub fn run_console_command(
        &mut self,
        assets: &LevelAssets,
        dev: &mut DevState,
        selected_view_element: &mut Option<EntityId>,
        input: &str,
    ) -> ConsoleResponse {
        self.inner
            .run_console_command(assets, dev, selected_view_element, input)
    }

    /// Run a console-cheat input with the dev cheat set forced on, even
    /// if the console is currently in `use_final` mode.  Intended for
    /// out-of-band cheat entry points (HTTP RPC, debug overlays) whose
    /// caller contract is "always reach the full dev command table".
    pub fn run_cheat_string(
        &mut self,
        assets: &LevelAssets,
        dev: &mut DevState,
        selected_view_element: &mut Option<EntityId>,
        input: &str,
    ) -> ConsoleResponse {
        self.inner
            .run_cheat_string(assets, dev, selected_view_element, input)
    }

    pub fn restore_rng_from_seed(&mut self, seed: u64) {
        self.inner.restore_rng_from_seed(seed);
    }

    /// Invoke a script `NativeFn` from outside the VM (HTTP-RPC, debug
    /// tooling).  See [`EngineInner::call_external_native`] for the full
    /// contract — performs the same swap-engine-state /
    /// `sync_game_host_post_script` dance script callbacks use, so any
    /// queued side-effects (camera, dialog, sequences, sound, deferred
    /// game-logic) are drained as if a script had made the call.
    pub fn call_external_native(
        &mut self,
        assets: &LevelAssets,
        native_name: &str,
        args: &[i32],
    ) -> Result<i32, String> {
        self.inner.call_external_native(assets, native_name, args)
    }

    /// Like [`Self::call_external_native`], but with an explicit
    /// `script_this` override (restored after the call).
    pub fn call_external_native_with_this(
        &mut self,
        assets: &LevelAssets,
        native_name: &str,
        args: &[i32],
        this_actor: Option<i32>,
    ) -> Result<i32, String> {
        self.inner
            .call_external_native_with_this(assets, native_name, args, this_actor)
    }

    /// Refresh render-only patch door highlight flags.
    ///
    /// `GameHost.patches` is intentionally outside the rollback hash,
    /// so this is allowed from the cursor/render path. Sim-visible
    /// mouse effects must still travel through `PlayerCommand`.
    pub fn refresh_selected_patch_display_doors(&mut self, selected_patch_idx: Option<u32>) {
        self.inner
            .refresh_selected_patch_display_doors(selected_patch_idx);
    }

    pub fn doors(&self) -> &[crate::gate::Door] {
        &self.inner.script_domains.interactables.doors
    }

    pub fn patches(&self) -> &[crate::patch::Patch] {
        &self.inner.script_domains.interactables.patches
    }

    // ── Per-frame drains ────
    // Patch-effect bg blits now travel through `SideEffects`
    // (`apply_side_effects` moves them into `Host::pending_bg_blits`)
    // so the engine no longer owns the queue between tick and render.

    // `mission_script_game_host_mut` is no longer exposed — the
    // host-side callers go through `refresh_selected_patch_display_doors`
    // / `queue_update_information_bars` / `PlayerCommand::*` instead.

    // `campaign_mut` is no longer exposed — cross-crate callers use
    // the narrow methods below, or read through `campaign()` and
    // dispatch mutations via `PlayerCommand`.  `Campaign` is part of
    // the rollback hash; any future mutator added here must run on a
    // mission-lifecycle boundary (campaign map, save/load, quit) where
    // the sim is paused.

    /// Commit a blazon purchase on the owned campaign.  Pure menu-time
    /// operation — runs on the mission-description screen while the sim
    /// is paused.  Returns `true` when the Sherwood consume-cascade
    /// closed the buy screen (blazon mission fully funded), matching
    /// `Campaign::buy_blazon`.  `None` when no campaign is installed.
    pub fn campaign_buy_blazon(
        &mut self,
        mission_index: usize,
        profiles: &crate::profiles::ProfileManager,
    ) -> Option<bool> {
        self.inner
            .mission_domain
            .campaign
            .as_mut()
            .map(|c| c.buy_blazon(mission_index, profiles))
    }

    /// Reset the campaign's `last_pseudo_mission_status` flag after the
    /// campaign-map host has displayed the pseudo-mission debriefing.
    /// Runs on a mission-lifecycle boundary (sim paused) — `Campaign` is
    /// part of the rollback hash.  No-op when no campaign is installed.
    pub fn campaign_reset_last_pseudo_mission_status(&mut self) {
        if let Some(c) = self.inner.mission_domain.campaign.as_mut() {
            c.reset_last_pseudo_mission_status();
        }
    }

    /// Reset the campaign's `MissionLength` accumulator to 0 before
    /// the mission begins.
    pub fn campaign_reset_mission_length(&mut self) {
        if let Some(c) = self.inner.mission_domain.campaign.as_mut() {
            c.set_value(crate::campaign::CampaignValue::MissionLength, 0);
        }
    }

    /// Queue the `UpdateInformationBars` script-host command so the
    /// next tick rebuilds the blazon / requirements widgets against
    /// the current campaign state.  Called inline from
    /// `DisplayCampaignMap` post-commit and from the options-menu
    /// resolution-change handler.
    pub fn queue_update_information_bars(&mut self) {
        self.inner.queue_update_information_bars();
    }

    /// Push the active player profile's `GraphicConfig` through the
    /// shadow polygon and every live element so a graphics-options
    /// change takes effect immediately.
    ///
    /// Today neither effect needs explicit propagation: the
    /// shadow-polygon renderer reads the framed-view-cone flag live
    /// from the active profile at draw time (no cached function
    /// pointer), and element shadow rendering is not currently
    /// per-element-cached either.  The method is provided as a
    /// callable surface so the `DisplayMenu` re-entry path has a
    /// single hook — when the framed-view-cone shadow path or
    /// per-element shadow caching is wired up, the implementation
    /// here is the single point that needs to fan out the new config.
    pub fn change_detail_level(&mut self) {
        // Read-only access today (the active profile already supplies
        // GraphicConfig wherever rendering needs it).  Logged at debug
        // so the call shows up in replay traces alongside the
        // resolution-change events that surround it.
        tracing::debug!(
            "Engine::change_detail_level — graphics config refreshed (no cached state to invalidate)"
        );
    }

    pub fn is_peasant_name_registered(&self, name: &str) -> bool {
        self.inner.is_peasant_name_registered(name)
    }

    // ── Test-only helpers (round-trip save/load tests) ────────────
    //
    // Gated behind the `test-helpers` Cargo feature so production
    // builds of the facade do not expose direct sim-state setters.
    // `robin_rs` enables the feature in its `[dev-dependencies]`
    // block so its round-trip tests compile.

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_set_mission_flags(&mut self, quit_won: bool, quit_lost: bool, mission_won: bool) {
        self.inner
            .test_set_mission_flags(quit_won, quit_lost, mission_won);
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_set_frame_counter(&mut self, frame: u32) {
        self.inner.test_set_frame_counter(frame);
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_set_engine_scalars(
        &mut self,
        cheat_used_flags: u32,
        speed: f32,
        speed_int: u16,
        lock_engine: bool,
        freeze_all: bool,
        script_globals: Vec<i32>,
    ) {
        self.inner.test_set_engine_scalars(
            cheat_used_flags,
            speed,
            speed_int,
            lock_engine,
            freeze_all,
            script_globals,
        );
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_set_mission_stat(&mut self, stat: crate::mission_stat::MissionStat) {
        self.inner.test_set_mission_stat(stat);
    }

    // ── Save / restore ────────────────────────────────────────────

    /// Build a fully-restored engine from `self` (the currently-live
    /// engine, loaded for the matching mission) and `saved` (a
    /// deserialised engine just read off disk).
    ///
    /// Wholesale sim-state replacement — legitimate because loading a
    /// save or rewinding is a deliberate, user-initiated discontinuity
    /// that also resets the rollback checker.  The consuming signature
    /// makes the replacement explicit at every call site: the old
    /// engine is moved in, a new one comes out.
    ///
    /// What survives from `self` is the host-populated static level
    /// data that isn't in the save payload (`Arc`'d level geometry /
    /// script bytecode). Everything else comes from `saved`, then runs
    /// through [`EngineInner::post_load_fixups`] so mid-drag / mid-zoom
    /// / mid-tick transient state from whichever session produced the
    /// save doesn't leak into the new session.
    ///
    /// Queues `UpdateInformationBars` on the script host so the next
    /// tick recomputes the HUD state to match the loaded mission.
    pub fn restore(&mut self, display: &mut super::HostDisplayState, saved: Engine) {
        // TODO(snapshot-api): migrate the downstream save-file facade to
        // `try_restore` and surface `SnapshotRestoreError` to the UI. That
        // caller is outside this invariant-hardening slice, so retain the
        // existing infallible facade while rejecting corruption loudly.
        self.try_restore(display, saved)
            .unwrap_or_else(|error| panic!("cannot restore engine snapshot: {error}"));
    }

    /// Fallible form of [`Self::restore`] for snapshot/network boundaries.
    ///
    /// Validation happens before `self` is mutated. In particular, malformed
    /// parallel fast-grid arrays are rejected rather than replaced with
    /// all-active values that were never present in the snapshot.
    pub fn try_restore(
        &mut self,
        display: &mut super::HostDisplayState,
        saved: Engine,
    ) -> Result<(), SnapshotRestoreError> {
        self.validate_snapshot_compatibility(&saved)?;

        // Consume the saved engine's inner into `self.inner`, keeping
        // the previous (already-loaded) `inner` as `prev` so its Arc'd
        // static level data can be transferred over.  Taking `&mut self`
        // instead of `self` preserves the RAII invariant that an
        // `Engine` always exists in a fully-initialised state — we
        // never need a `Default` shim to satisfy `std::mem::take`.
        let prev = std::mem::replace(&mut self.inner, saved.inner);
        let inner = &mut self.inner;

        // ── Transfer host display level data ─────────────────────
        // This lives outside the sim snapshot and is populated by the
        // loaded level assets, so keep it from the already-loaded
        // engine we're restoring into.
        inner.feedback.cutscene_camera.level_size = prev.feedback.cutscene_camera.level_size;
        // `source_durations` lives on `LevelAssets` now; the host
        // reinstates it alongside the rest of the level assets.

        inner
            .world
            .fast_grid
            .attach_level_grid(prev.world.fast_grid.level.clone());

        // Re-attach the script bytecode Arc to the deserialised mission
        // script. The concrete GameHost is now serialised on
        // MissionScript; `vm.host` is only a temporary call adapter.
        if let (Some(new_ms), Some(prev_ms)) =
            (inner.mission_script.as_mut(), prev.mission_script.as_ref())
        {
            new_ms
                .manager
                .attach_program(prev_ms.manager.program.clone());
            new_ms.bindings = prev_ms.bindings.clone();
        }
        inner.migrate_legacy_script_custom_values();

        // Rebuild `SequenceManager` lookup indices after replacing the
        // sequence list.
        inner.orders.sequence_manager.rebuild_indices();

        // ── Engine-owned transient reset + HUD refresh ───────────
        inner.post_load_fixups(display);
        inner.queue_update_information_bars();
        Ok(())
    }

    fn validate_snapshot_compatibility(&self, saved: &Engine) -> Result<(), SnapshotRestoreError> {
        if saved.inner.script_domains.zones.scripts.len()
            != self.inner.script_domains.zones.scripts.len()
        {
            return Err(SnapshotRestoreError::WorldInvariantViolation {
                detail: format!(
                    "snapshot script-zone runtime length {} does not match loaded world length {}",
                    saved.inner.script_domains.zones.scripts.len(),
                    self.inner.script_domains.zones.scripts.len(),
                ),
            });
        }
        let level = &self.inner.world.fast_grid.level;
        let lengths = [
            (
                SnapshotGridComponent::Lines,
                saved.inner.world.fast_grid.line_active.len(),
                level.lines.len(),
            ),
            (
                SnapshotGridComponent::Sectors,
                saved.inner.world.fast_grid.sector_active.len(),
                level.sectors.len(),
            ),
            (
                SnapshotGridComponent::Masks,
                saved.inner.world.fast_grid.mask_active.len(),
                level.masks.len(),
            ),
        ];

        // Original provenance: `original-code/RHfastfindgrid.cpp:8890-9115`
        // serializes runtime patch/door/sector state against the already
        // loaded grid and propagates failure. It never invents an all-active
        // replacement when the save and level topology disagree.
        for (component, snapshot_len, level_len) in lengths {
            if snapshot_len != level_len {
                return Err(SnapshotRestoreError::FastGridLengthMismatch {
                    component,
                    snapshot_len,
                    level_len,
                });
            }
        }
        saved
            .inner
            .world
            .validate_snapshot_compatibility(&self.inner.world)
            .map_err(|detail| SnapshotRestoreError::WorldInvariantViolation { detail })?;
        saved
            .inner
            .orders
            .validate_invariants()
            .map_err(|detail| SnapshotRestoreError::OrderInvariantViolation { detail })?;
        Ok(())
    }
}

impl Deref for Engine {
    type Target = EngineInner;

    fn deref(&self) -> &EngineInner {
        &self.inner
    }
}

// `Default for Engine` is intentionally not implemented: the RAII
// contract says an `Engine` exists only when it's a fully-initialised
// mission engine, and the required mission data can't be conjured
// from defaults.  Tests that want a blank engine should construct
// `EngineInner` directly (it stays `pub(crate)` for that use), or
// fabricate a test-fixture level and go through `Engine::new`.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_construction_returns_the_same_campaign_allocation() {
        let mut profiles = crate::profiles::ProfileManager::default();
        profiles
            .missions
            .push(crate::profiles::MissionProfile::default());
        profiles.soldiers.push(crate::profiles::SoldierProfile {
            filename: "missing-construction-test-sprite".to_owned(),
            profile_name: "missing-construction-test-profile".to_owned(),
            ..crate::profiles::SoldierProfile::default()
        });

        let mut campaign = crate::campaign::Campaign::default();
        campaign.missions.push(crate::mission::Mission {
            profile_idx: Some(0),
            ..crate::mission::Mission::default()
        });
        campaign.current_mission_idx = Some(0);
        let missions_ptr = campaign.missions.as_ptr();

        let mut assets = LevelAssets::new();
        assets.profile_manager = std::sync::Arc::new(profiles);
        let mut loaded = crate::level_data::LoadedLevel::empty_for_test();
        loaded.mission.soldiers.push(crate::level_data::RawSoldier {
            position_x: 0,
            position_y: 0,
            direction: 0,
            action: 0,
            obstacle_index: 0,
            sector: 0,
            layer: 0,
            material: 0,
            profile_number: 0,
            tower_guard: false,
            company_number: 0,
            drunk_level: 0,
            money: 0,
            subordinate_ids: Vec::new(),
            path_id: 0,
            alert_path_id: 0,
            script_class: None,
        });

        let result = Engine::new_preserving_campaign(EngineArgs {
            campaign,
            level: LevelLoadArgs {
                assets: &mut assets,
                level_directory: "",
                progress: &mut |_| {},
                loaded,
                bg_pixel_dims: (0.0, 0.0),
            },
            ground_mark_sprite: None,
            titbit_row_frame_counts: Vec::new(),
            rng_seed: 0,
            goldeneye: false,
        });

        let (error, returned) = match result {
            Ok(_) => panic!("missing sprite must fail construction"),
            Err(failure) => failure,
        };
        assert!(matches!(error, EngineError::ProfileSpriteLoadFailed { .. }));
        assert_eq!(returned.missions.as_ptr(), missions_ptr);
        assert_eq!(returned.current_mission_idx, Some(0));
    }

    /// `Engine::restore` must transfer host-owned level fields from
    /// the live engine to the deserialised one when those fields live
    /// outside the sim snapshot.
    ///
    /// This test plants distinctive host data on a source engine, runs
    /// it through serde, calls `restore`, and asserts every host field
    /// we seeded comes back. When a new host-owned field lands on
    /// `EngineInner`, extend both the transfer block in `Engine::restore`
    /// and the assertions below.
    #[test]
    fn restore_preserves_host_level_fields() {
        let mut source_inner = EngineInner::new();

        source_inner.feedback.cutscene_camera.level_size =
            crate::coordinates::MapSize::new(1234.0, 5678.0);

        let source = Engine {
            inner: source_inner,
        };

        let json = serde_json::to_string(&source).expect("serialize");
        let decoded: Engine = serde_json::from_str(&json).expect("deserialize");

        // `restore` mutates `source` in place so the RAII invariant
        // that `Engine` is always fully initialized holds without
        // needing a `Default` shim for `std::mem::take`.
        let mut restored = source;
        let mut display = crate::engine::HostDisplayState::default();
        restored.restore(&mut display, decoded);

        assert_eq!(
            restored.inner.feedback.cutscene_camera.level_size,
            crate::coordinates::MapSize::new(1234.0, 5678.0)
        );
    }

    #[test]
    fn try_restore_rejects_mismatched_runtime_lengths_without_mutating_live_engine() {
        let mut live_inner = EngineInner::new();
        live_inner.feedback.cutscene_camera.level_size =
            crate::coordinates::MapSize::new(1234.0, 5678.0);
        let mut live = Engine { inner: live_inner };

        let mut malformed_inner = EngineInner::new();
        malformed_inner.world.fast_grid.line_active.push(true);
        let malformed = Engine {
            inner: malformed_inner,
        };

        let mut display = crate::engine::HostDisplayState::default();
        let error = live.try_restore(&mut display, malformed).unwrap_err();
        assert_eq!(
            error,
            SnapshotRestoreError::FastGridLengthMismatch {
                component: SnapshotGridComponent::Lines,
                snapshot_len: 1,
                level_len: 0,
            }
        );
        assert_eq!(
            live.inner.feedback.cutscene_camera.level_size,
            crate::coordinates::MapSize::new(1234.0, 5678.0),
            "validation must happen before replacing the live engine"
        );
    }

    #[test]
    fn try_restore_rejects_world_parallel_mismatch_before_mutating_live_engine() {
        let mut live = Engine {
            inner: EngineInner::new(),
        };
        let mut malformed_inner = EngineInner::new();
        malformed_inner
            .script_domains
            .zones
            .scripts
            .push(crate::sector::ScriptSectorData::new());
        let malformed = Engine {
            inner: malformed_inner,
        };

        let mut display = crate::engine::HostDisplayState::default();
        let error = live.try_restore(&mut display, malformed).unwrap_err();
        assert_eq!(
            error,
            SnapshotRestoreError::WorldInvariantViolation {
                detail:
                    "snapshot script-zone runtime length 1 does not match loaded world length 0"
                        .to_owned(),
            }
        );
        assert!(live.inner.script_domains.zones.scripts.is_empty());
    }
}
