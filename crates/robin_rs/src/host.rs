//! Host state — non-sim, non-serialized per-client transient.
//!
//! Lived as `robin_engine::engine::Host` during the early Rust port,
//! moved to robin_rs once engine code stopped depending on it. Each
//! client owns one `Host`; rollback snapshots ignore it (each client
//! reconstructs its own from hardware context). Engine reaches host
//! state only through input parameters and `SideEffects` outputs.

use robin_assets::frame_holder::FrameHolder;
use robin_assets::keyconfig::KeyConfig;
use robin_assets::shipping_datadir::ShippingDatadir;
use robin_engine::coordinates::{MapPoint, ScreenPoint, WorldPoint3D};
use robin_engine::element::{EntityId, TrajectoryPoint};
use robin_engine::engine::{
    DrawOrder, FadeToBlack, GroundMarkSpriteData, InputState, PendingBgBlit, SideEffects,
};
use robin_engine::game_operation::GameCode;
use robin_engine::geo2d::Vec2D;
use robin_engine::markers::GroundMark;
use std::collections::HashMap;
use std::sync::Arc;

use crate::bg_cache::BackgroundDecal;
use crate::draw_manager::DrawManager;
use crate::mouse_way::MouseWay;
use crate::pc_info_overlay::PcInfoOverlay;
use crate::sound::SoundManager;

const PANNEL_HEIGHT: f32 = robin_engine::engine::PANNEL_HEIGHT;
const DISPLAY_INFO_SAMPLES: usize = 16;

/// Deferred PrintScreen request, including the modifier branch that was
/// active when the key edge fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PrintScreenRequest {
    Plain,
    Median3x3,
    WideSnapshot,
}

/// Host-only titbit-like previews emitted by cursor/hover code.
///
/// These are intentionally not inserted into `Engine::titbit_manager`:
/// they are local UI feedback and must not affect rollback state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HostTitbitPreview {
    JumpHelperGhost {
        position: WorldPoint3D,
        layer: u16,
        sector_dir: u16,
        display_order: f32,
    },
}

impl HostTitbitPreview {
    #[inline]
    pub fn display_order(self) -> f32 {
        match self {
            Self::JumpHelperGhost { display_order, .. } => display_order,
        }
    }
}

/// Host-local viewport state. This is deliberately outside
/// `robin_engine`: screen size, mouse anchoring, render culling, and
/// local scroll/zoom are presentation concerns and may differ on every
/// multiplayer peer.
#[derive(Debug, Clone)]
pub struct ViewportState {
    pub view_position: MapPoint,
    pub old_view_position: MapPoint,
    pub zoom_factor: f32,
    pub old_zoom_factor: f32,
    pub screen_size: Vec2D,
    pub level_size: Vec2D,
}

impl ViewportState {
    pub fn new(screen_width: f32, screen_height: f32) -> Self {
        Self {
            view_position: MapPoint::ZERO,
            old_view_position: MapPoint::ZERO,
            zoom_factor: 1.0,
            old_zoom_factor: 1.0,
            screen_size: robin_engine::geo2d::pt(screen_width, screen_height),
            level_size: robin_engine::geo2d::pt(0.0, 0.0),
        }
    }

    pub fn set_screen_size(&mut self, width: f32, height: f32) {
        self.screen_size = robin_engine::geo2d::pt(width, height);
        self.clip_view();
    }

    pub fn set_level_size(&mut self, width: f32, height: f32) {
        self.level_size = robin_engine::geo2d::pt(width, height);
        self.clip_view();
    }

    pub fn center_on_point(&mut self, point: MapPoint) {
        self.view_position = MapPoint::new(
            (point.x - self.screen_size.x / (2.0 * self.zoom_factor)).floor(),
            (point.y - self.screen_size.y / (2.0 * self.zoom_factor)).floor(),
        );
        self.clip_view();
    }

    pub fn sound_listen_point(&self) -> MapPoint {
        MapPoint::new(
            self.view_position.x + self.screen_size.x * 0.5 / self.zoom_factor,
            self.view_position.y + (self.screen_size.y - PANNEL_HEIGHT) * 0.5 / self.zoom_factor,
        )
    }

    pub fn scroll_by(&mut self, delta: Vec2D) {
        self.old_view_position = self.view_position;
        self.view_position.x += delta.x / self.zoom_factor;
        self.view_position.y += delta.y / self.zoom_factor;
        self.clip_view();
    }

    pub fn zoom_by(&mut self, factor: f32, mouse_screen: Option<ScreenPoint>) {
        let next = (self.zoom_factor * factor).clamp(0.5, 2.0);
        if (next - self.zoom_factor).abs() < f32::EPSILON {
            return;
        }
        let anchor = mouse_screen.unwrap_or_else(|| {
            ScreenPoint::new(self.screen_size.x * 0.5, self.screen_size.y * 0.5)
        });
        let before = self.screen_to_map_unchecked(anchor);
        self.old_zoom_factor = self.zoom_factor;
        self.zoom_factor = next;
        self.view_position = MapPoint::new(
            before.x - anchor.x / self.zoom_factor,
            before.y - anchor.y / self.zoom_factor,
        );
        self.clip_view();
    }

    pub fn screen_to_map(&self, screen_pt: ScreenPoint) -> Option<MapPoint> {
        let map_pt = self.screen_to_map_unchecked(screen_pt);
        if map_pt.x > 0.0
            && map_pt.y > 0.0
            && map_pt.x <= self.level_size.x
            && map_pt.y <= self.level_size.y
        {
            Some(map_pt)
        } else {
            None
        }
    }

    pub fn screen_to_map_unchecked(&self, screen_pt: ScreenPoint) -> MapPoint {
        MapPoint::new(
            self.view_position.x + screen_pt.x / self.zoom_factor,
            self.view_position.y + screen_pt.y / self.zoom_factor,
        )
    }

    pub fn map_to_screen(&self, map_pt: MapPoint) -> Option<ScreenPoint> {
        let screen_pt = self.map_to_screen_unclamped(map_pt);
        if screen_pt.x >= 0.0
            && screen_pt.y >= 0.0
            && screen_pt.x <= self.screen_size.x
            && screen_pt.y <= self.screen_size.y
        {
            Some(screen_pt)
        } else {
            None
        }
    }

    pub fn map_to_screen_unclamped(&self, map_pt: MapPoint) -> ScreenPoint {
        ScreenPoint::new(
            (map_pt.x - self.view_position.x) * self.zoom_factor,
            (map_pt.y - self.view_position.y) * self.zoom_factor,
        )
    }

    pub fn visible_rect(&self) -> geo::Rect<f32> {
        let w = self.screen_size.x / self.zoom_factor;
        let h = (self.screen_size.y - PANNEL_HEIGHT) / self.zoom_factor;
        geo::Rect::new(
            self.view_position.to_geo(),
            robin_engine::geo2d::pt(self.view_position.x + w, self.view_position.y + h),
        )
    }

    fn clip_view(&mut self) {
        if self.view_position.x < 0.0 {
            self.view_position.x = 0.0;
        }
        if self.view_position.y < 0.0 {
            self.view_position.y = 0.0;
        }
        if self.level_size.x > 0.0 {
            let max_x = (self.level_size.x - self.screen_size.x / self.zoom_factor).max(0.0);
            self.view_position.x = self.view_position.x.min(max_x);
        }
        if self.level_size.y > 0.0 {
            let max_y = (self.level_size.y
                - (self.screen_size.y - PANNEL_HEIGHT) / self.zoom_factor)
                .max(0.0);
            self.view_position.y = self.view_position.y.min(max_y);
        }
    }
}

impl Default for ViewportState {
    fn default() -> Self {
        Self::new(1024.0, 768.0)
    }
}

/// Rendering, input, audio, and transient per-frame state that does
/// **not** participate in the deterministic simulation snapshot.
#[derive(Default)]
pub struct Host {
    // ── Rendering / GPU surfaces ─────────────────────────────────
    pub map_surface: u32,
    pub minimap_corner_surfaces: Vec<u32>,
    pub minimap_corner_size: Vec2D,
    /// Per-`DotType` dot sprite `(surface, width, height)`. Indexed by
    /// `DotType as usize`. Populated at mission start; empty until then.
    pub minimap_dot_surfaces: Vec<(u32, u16, u16)>,
    pub ground_mark_surfaces: Vec<(u32, u16, u16)>,
    pub viewport: ViewportState,
    pub engine_display: robin_engine::engine::HostDisplayState,

    // ── Input ────────────────────────────────────────────────────
    pub input: InputState,

    /// Which seat in the simulation is driven by *this* process.
    ///
    /// Live input pipelines (mouse, keyboard, gamepad) stamp every
    /// outgoing [`robin_engine::player_command::PlayerCommand`] with
    /// this id before queueing it for replay/dispatch, so the seat
    /// tag is data-driven rather than baked into a constant.
    ///
    /// - Single-player or headful host: `PlayerId::HOST` (= seat 0).
    /// - Headless host: no input pipeline runs, so this is unused.
    /// - Remote peer: the join-order id assigned by the host on
    ///   connect (e.g. `PlayerId(2)` for the third-joined peer).
    ///
    /// Distinct from [`PlayerId::HOST`], which is the *seat* id seat 0
    /// occupies in the sim — that's identical on every machine in
    /// the session.  `local_seat` varies per machine and never
    /// participates in serialization or rollback hashes.
    pub local_seat: robin_engine::player_command::PlayerId,

    /// Multiplayer transport session (server or client).  `None` in
    /// single-player; populated when `--server` / `--connect` is set.
    /// The game loop drains `net.incoming` each frame to fold peer
    /// inputs into the engine's command batch, and pushes locally-
    /// produced inputs into `net.outgoing` for the server to stamp +
    /// broadcast.  Never serialised — purely host transport state.
    pub net: Option<crate::multiplayer::NetChannels>,

    /// Multiplayer-negotiated mission RNG seed.  `Some` when this
    /// process is part of an active multiplayer session — the server
    /// picks the seed at session start and broadcasts it via the
    /// `Welcome` handshake; the host uses its own picked seed.  After
    /// `Engine::new` the mission code calls
    /// [`robin_engine::engine::EngineInner::restore_rng_from_seed`]
    /// with this value so every machine in the session simulates the
    /// same sequence of rolls.  `None` in single-player keeps the
    /// engine's hardcoded seed (currently 0).
    pub mp_mission_seed: Option<u64>,

    /// Back-to-front entity draw order.  Host-cached derived state —
    /// recomputed from [`Engine::compute_display_order`] once per frame
    /// after the tick, before the input-dispatch and render passes.
    /// Consumed by the render loop (iteration), input hit-test
    /// (`find_focusable_entity`), and titbit Z flush (depth lookup).
    /// Not sim state: never serialized, never hashed.
    pub draw_order: DrawOrder,

    /// Ping-pong animation phase for the PC selection ring.  Advanced
    /// once per frame inside `Game::run_engine_tick`, gated on the same
    /// `should_run_hourglass` check as the sim tick (so pause / console
    /// freeze the ring).  Only `SelectionMarkRenderer` reads it —
    /// purely cosmetic, lives host-side.
    pub selection_mark: robin_engine::markers::SelectionMark,

    /// Entity whose vision cone is currently displayed as an overlay.
    /// Set when the player alt-hovers an NPC (or an ally via a cheat).
    ///
    /// UI-mode state: read by the render-phase vision-cone overlay,
    /// the alt-key UI handler, and the console cheats that target
    /// "the NPC you're currently looking at" (Honolulu, Morpheus,
    /// Hades, LastManStanding).  Not sim state: nothing inside the
    /// tick reads it, so it's excluded from the rollback hash by
    /// virtue of living on Host.
    pub selected_view_element: Option<EntityId>,

    // ── Trajectory preview (transient) ───────────────────────────
    pub valid_trajectory: bool,
    pub trajectory_preview_points: Vec<TrajectoryPoint>,
    pub trajectory_preview_start: WorldPoint3D,
    /// Shooter layer captured alongside `trajectory_preview_points`.
    /// Passed to `GroundMark::add_mark` by the ground-mark drop driver
    /// as the layer argument for the trajectory display.
    pub trajectory_preview_layer: u16,
    /// Set by the trajectory-preview computation when the projected
    /// shot will miss (arrows/stones) or the net will crumple
    /// (Easy-mode nets).  Read by the trajectory-preview renderer to
    /// swap the arc colour from cyan (default) to pink (crumpled).
    pub net_crumpled: bool,
    pub time_no_mouse_move: u32,
    pub mouse_map_prev: MapPoint,
    /// Rolling counter for the once-every-10-frames ground-mark drop
    /// performed by `DisplayTrajectory`.  Incremented each frame the
    /// trajectory-preview is valid.
    pub trajectory_mark_count: u16,
    /// Host-local destination markers emitted by the trajectory-preview
    /// hover path. Real move-command markers stay engine-owned; preview
    /// markers are per-seat UI feedback and must not affect rollback.
    pub trajectory_ground_mark: GroundMark,
    /// Host-local titbit-like hover preview.  Currently only the
    /// helper-needed jump ghost from the original mouse-hover path.
    pub host_titbit_preview: Option<HostTitbitPreview>,

    // ── Assets that live only on the host side ───────────────────
    /// Decoded sprite frame bank. Host-only because `FrameHolder`
    /// lives in `robin_assets`, which depends on `robin_engine` — so
    /// engine's `LevelAssets` can't carry it. Shared via `Arc` so
    /// `Engine::clone` stays cheap.
    pub frame_holder: Arc<FrameHolder>,

    /// Shipping-datadir handle. Host-only (asset-layer type). Holds the
    /// path/resource layout for the currently-loaded shipping build so
    /// the resource manager can resolve relative lookups.
    pub shipping: Option<Arc<ShippingDatadir>>,

    /// Active key bindings for the current player profile. Host-only —
    /// `KeyConfig` lives in `robin_assets` (depends on `robin_engine`),
    /// so it can't be a field on `PlayerProfile` itself.
    pub key_config: KeyConfig,

    /// User's custom key bindings (the "User Defined" slot in the
    /// shortcuts menu). The active set is whatever the user picked
    /// last (preset or custom), while this slot preserves their
    /// personal bindings so the User Defined button can restore them.
    pub custom_key_config: KeyConfig,

    /// SDL scancode bound to the `DisplayMap` shortcut.  The game loop
    /// reads this on each frame and emits a minimap-toggle command on
    /// key release.  Zero means no accelerator bound.  Lives host-side
    /// — the engine has no reason to know which key the UI is bound to.
    pub minimap_fast_key: u16,

    // ── Host-side managers ───────────────────────────────────────
    /// Audio playback manager (samples, music, listen point).
    pub sound: SoundManager,
    /// Immediate-mode draw helper (line segments, ellipses, gauges).
    pub draw_manager: DrawManager,
    /// PC info hover popup (HP, equipment). Populated from sim's
    /// `SideEffects.overlay`.
    pub pc_info_overlay: PcInfoOverlay,
    /// Mouse gesture / way-point tracker for "draw-path-to-target"
    /// movement. Pure host UI state.
    pub mouse_way: MouseWay,

    /// Gamepad / joystick state. Carries edge-detection, the QA macro
    /// timer, and the in-progress swordfight-gesture buffer across
    /// frames.  Updated from SDL3 controller events each frame.
    pub gamepad: crate::gamepad::GamePadState,

    // ── Pixel-level fade (script opcode `FADE_TO_BLACK`) ─────────
    /// Active fade-to-black ramp driven by the `FADE_TO_BLACK` script
    /// opcode.  When set, the renderer draws a black overlay with a
    /// per-frame alpha ramp — alpha climbs from 0→255 over `speed`
    /// frames (fade out), then falls 255→0 over the next `speed`
    /// frames (fade back in).
    pub fade_to_black: Option<FadeToBlack>,

    /// Last tick's `SideEffects.skip_render` decision. Read by the
    /// per-frame render loop in `game_session` to short-circuit the
    /// GPU pass when fast-forward mode wants to skip.
    pub skip_render: bool,

    /// Sound-source indices the engine asked the host to (re)play this
    /// tick. Drained by `host.sound.hourglass`.
    pub pending_play_delayed_sources: Vec<usize>,

    /// `(position, zoom)` set when a tick emitted a `ResumeAllSources`
    /// command. Drained by game_session before the sound hourglass
    /// runs, since it needs `&engine.sound_sim.sources`.
    pub pending_resume_all_sources: Option<(MapPoint, f32)>,

    /// Sound-source indices the engine asked the host to activate
    /// this tick (from `SoundCommand::ActivateSource`). Drained by
    /// game_session before the sound hourglass runs — needs
    /// `&mut engine.sound_sim.sources`.
    pub pending_activate_sources: Vec<usize>,

    /// Actor ids whose current/queued exclamation should be stopped
    /// before the sound hourglass starts new pending speech. Drained by
    /// game_session where the SDL backend is available.
    pub pending_stop_exclamations: Vec<u32>,

    /// Actor ids whose currently playing exclamation channel should be
    /// stopped without deleting speech queued later in the same tick.
    /// This preserves the StopExclamation-then-PlayExclamation
    /// sequence: the old line is cut off, the new emergency/death
    /// line still reaches the hourglass.
    pub pending_stop_exclamation_channels: Vec<u32>,

    /// Patch-effect `BlitToMap` / `RestoreBackground` requests handed
    /// off by the last tick's `SideEffects`.  Drained in
    /// `pre_render_engine_setup` where `&mut LevelAssets` + `&mut
    /// Renderer` are available — see `robin_rs::blit_to_map`.
    pub pending_bg_blits: Vec<PendingBgBlit>,

    // ── Host-owned UI-request queues (drained at host UI sites) ──
    /// Dialogue IDs pushed by `StartDialog` script commands.  Accumulated
    /// from every tick's `SideEffects.pending_dialogues`.  Drained by
    /// the game session when it's ready to display.
    pub pending_dialogues: Vec<i32>,
    /// Popup-scroll text IDs pushed by `DisplayPopupText` /
    /// `DisplayAllPopupTexts`.  Accumulated from every tick.  Drained
    /// by the game session through `RHMenuPopupScroll::DisplaySingle`.
    pub pending_popup_texts: Vec<i32>,
    /// Debriefing text IDs pushed by the `DisplayAllDebriefings` cheat.
    pub pending_debriefings: Vec<robin_engine::player_command::DebriefingTextId>,
    /// Set when a tick fired `DisplaySherwoodReport`.
    pub pending_sherwood_report: bool,
    /// Set when a tick fired `DisplayConsole`.
    pub pending_show_console: bool,
    /// Set when a tick fired a silent win (ambush/tactical). Drained
    /// in `Game::perform_hourglass_*` to flip the Sherwood
    /// start/quit-mission widgets.
    pub pending_silent_win_widget_swap: bool,
    /// Set when a tick fired the first-time mission-won banner
    /// trigger.  Host drains it in `Game::perform_hourglass_*`, flips
    /// `quit_mission_enabled = false`, and defers the blocking popup
    /// to the main loop via [`Self::pending_mission_state_popup`].
    pub pending_mission_state_notice: bool,
    /// Deferred blocking popup request for the first-time mission-won
    /// banner.  The main game loop blocks on `show_mission_state_popup`
    /// here, which requires `&mut crate::window::GameWindow` + `&mut Renderer` — neither
    /// is in scope inside `apply_side_effects`, so we park the flag and
    /// drive the popup from the same site that drives the end-of-mission
    /// debriefing popup.
    pub pending_mission_state_popup: bool,
    /// Set when a tick consumed `SimpleMessage::ResetInput`. Drained
    /// before the next input-translation pass so the input-translator's
    /// pressed-key cache and UI modifier latches are reset and the
    /// cursor is resynced — dropping any key-down edges queued while a
    /// modal was displaying.
    pub pending_reset_input: bool,
    /// Set in the `reset_input` branch of [`Self::apply_side_effects`];
    /// the caller drains it in the next pass through the game loop
    /// where [`robin_engine::engine::DevState`] is in scope, and
    /// applies the swap `info_displayed = fps_cheat; fps_cheat =
    /// false`. The FPS-cheat flag lives on `DevState::debug.fps_display`
    /// (game-session owned), which is unreachable from
    /// `apply_side_effects` itself — hence this two-stage hand-off.
    pub pending_fps_cheat_promote: bool,
    /// Set when the PrintScreen keybind fires.  Drained in the render
    /// loop after `render_frame` (before `present()`) which reads back
    /// the composited frame and writes it to disk as `screen%03u.png`
    /// in the save directory. Ctrl requests a wide snapshot; Shift
    /// applies the historical 3x3 median filter to the captured frame.
    pub pending_print_screen: Option<PrintScreenRequest>,

    /// Debug-info overlay toggle. Toggled by the bound `RequestInfo`
    /// / `DisplayInfo` key (typically `Home`); read by the per-frame
    /// debug-overlay renderer.  Not serialized — debug state, not sim
    /// state.
    pub info_displayed: bool,
    /// Rolling frame-duration samples used by the DisplayInfo overlay.
    pub display_info_frame_samples: [u32; DISPLAY_INFO_SAMPLES],
    pub display_info_sample_cursor: usize,
    pub display_info_last_tick_ms: u32,
    pub display_info_max_pending_sounds: usize,

    /// Slow-motion pacing toggle. Toggled by `MSG_SLOW_MOTION` (the
    /// bound SlowMotion key — Pause by default).  Consumed by the
    /// frame pacing block at the bottom of `run_mission`: when set
    /// (and neither console nor engine fast-forward are active), the
    /// 40 ms frame target is multiplied by 10.
    pub slow_motion: bool,

    /// One-frame "a UI widget stole input focus" latch.  Set by
    /// `MSG_UI_HAS_FOCUS` during the frame's message dispatch and
    /// cleared every frame by `BringDownState`.  The sole consumer
    /// (`RHDISPLAY_INITZOOM`) is itself unported, so this field
    /// currently only tracks the flag for future consumers.  Not sim
    /// state — purely transient per-frame input gating.
    pub ui_focus: bool,

    /// Deferred console-overlay output lines produced by host-side work
    /// that can't reach the overlay directly. Drained by the overlay
    /// at the start of each frame via
    /// [`crate::console_overlay::ConsoleOverlay::drain_pending_host_output`].
    pub pending_console_output: Vec<String>,

    // ── Persistent background decals ─
    /// Per-FX-entity persistent background decals replacing the legacy
    /// `BlitToMap` bake/restore surface pipeline. A queued `BlitToMap`
    /// inserts or replaces the entity's decal; a queued restore removes it.
    pub background_decals: HashMap<EntityId, BackgroundDecal>,
    /// Stable draw order for [`Self::background_decals`], preserving the
    /// order in which patch effects became permanent.
    pub background_decal_order: Vec<EntityId>,

    /// Lua interpreter for custom Spellforge missions, populated when
    /// the player launches a mod that ships a `.lua` script. `None`
    /// for vanilla campaigns and for Vanilla-tagged mods — the
    /// engine's `.scb` mission script still runs normally either way.
    /// Drained by the game-session frame loop to fire `Timer`,
    /// `CheckVictoryCondition`, etc. on the script.
    pub lua_session: Option<crate::lua_session::LuaSession>,
}

impl Host {
    pub fn new(screen_width: f32, screen_height: f32) -> Self {
        Self {
            viewport: ViewportState::new(screen_width, screen_height),
            input: InputState {
                has_focus: true,
                ..Default::default()
            },
            // Pick up the shipping datadir the entry-point installed in
            // `robin_assets::shipping_datadir::install_global`.  Every
            // `host.shipping.as_deref()` caller (text/cursor resource
            // attach, script/level loaders, mission-script lookup,
            // etc.) relies on this being populated — without it, a
            // wasm build with a shipping datadir still ends up going
            // through the disk-I/O fallback, which fails because no
            // filesystem is visible inside the worker.
            shipping: robin_assets::shipping_datadir::global().cloned(),
            ..Default::default()
        }
    }

    /// Mutable access to the frame holder during loading.
    pub fn frame_holder_mut(&mut self) -> &mut FrameHolder {
        Arc::make_mut(&mut self.frame_holder)
    }

    /// Clear persistent decals that belonged to the previous level.
    pub fn clear_background_decals(&mut self) {
        self.background_decals.clear();
        self.background_decal_order.clear();
    }

    /// Reset host-side transient state after a save-load.  Mirrors the
    /// engine-side [`robin_engine::engine::Engine::restore`] fix-ups
    /// for the host half: a stale mid-drag rubber-band, a stale
    /// `focused_entity_id` pointing at a now-missing entity, or a
    /// UI-request queue partially drained before the load would all
    /// survive into the restored session without this wipe.  Called by
    /// [`crate::save_file::GameSaveFile::apply_to`] and by any future
    /// caller that swaps engines under a live host.
    ///
    /// Does NOT touch `SoundManager` — callers replace that wholesale
    /// from the save payload.
    pub fn post_load_reset(&mut self) {
        // Clear mouse/keyboard input state — otherwise a mid-drag
        // quick-load would leave the rubber-band box active with stale
        // screen coordinates, or keep a stale `focused_entity_id` that
        // no longer exists in the reloaded entity array.
        self.input = InputState::default();

        // Per-frame scratch flags that live host-side.
        self.valid_trajectory = false;
        self.host_titbit_preview = None;
        self.trajectory_ground_mark.clear();
        self.selected_view_element = None;
        // Restart the PC selection ring animation so every selected PC
        // comes back with a clean frame 0 (matches the pre-move
        // `Engine::restore` cleanup).
        self.selection_mark = robin_engine::markers::SelectionMark::default();

        // Drop any UI-request queues that were in flight before the
        // load.  They live host-side now — accumulated from per-tick
        // `SideEffects.pending_*` by `Host::apply_side_effects`.
        self.pending_dialogues.clear();
        self.pending_popup_texts.clear();
        self.pending_debriefings.clear();
        self.pending_sherwood_report = false;
        self.pending_show_console = false;
        self.pending_silent_win_widget_swap = false;
        self.pending_mission_state_notice = false;
        self.pending_mission_state_popup = false;
        self.pending_console_output.clear();
        self.pending_reset_input = false;
        self.pending_fps_cheat_promote = false;
        self.pending_print_screen = None;
    }

    /// Apply the engine-local outputs of a tick.  Consumes the
    /// [`SideEffects`] struct by value so owned sub-vectors
    /// can be moved directly into host accumulators without clones.
    /// Returns the tick's game-state code.
    pub fn apply_side_effects(&mut self, fx: SideEffects) -> GameCode {
        if let Some(fade) = fx.fade_to_black {
            self.fade_to_black = fade;
        }
        if let Some(show) = fx.set_draw_hidden {
            self.input.draw_hidden = show;
        }
        if fx.invalidate_trajectory_preview {
            // `SelectAction` trajectory cleanup: clear the jumper and
            // jumped trajectories, the valid flag, and the projectile
            // arc.  We fold all four trajectory overlays (jump-line
            // preview, projectile arc, valid flag, crumpled-net tint)
            // into the single host-side preview since there is only
            // ever one visible arc; clearing them together here is an
            // immediate wipe before the next UpdateMouse frame.
            self.valid_trajectory = false;
            self.trajectory_preview_points.clear();
            self.trajectory_ground_mark.clear();
            self.trajectory_mark_count = 0;
            self.net_crumpled = false;
            self.host_titbit_preview = None;
        }
        if fx.reset_input {
            // MSG_RESET_INPUT clears the rubber-band selection flags
            // and suppresses any pending drag / click so a modal popup
            // / dialog entered from a sequence command doesn't leave
            // input state armed.  Also zeroes the per-frame modifier
            // cache and the swordfight mouse-way polyline (modifier
            // keys, drag, UI focus, info overlay, mouse-way).
            self.input.multi_selection_active = false;
            self.input.multi_unselection_active = false;
            self.input.draw_multi_selection = false;
            self.input.ignore_next_drag = false;
            self.input.ignore_next_left_click = false;
            self.input.is_dragging = false;
            self.input.is_alt = false;
            // Reset does the swap `info_displayed = fps_cheat;
            // fps_cheat = false`: the FPS-cheat flag is consumed and
            // promoted into `info_displayed`, so toggling the FPS
            // cheat arms the next reset to leave the debug-info
            // overlay visible.  The cheat flag lives on
            // `DevState::debug.fps_display`, which is not reachable
            // from here — hand off via `pending_fps_cheat_promote` for
            // the game-loop site that owns `&mut DevState` to apply.
            self.pending_fps_cheat_promote = true;
            self.ui_focus = false;
            self.mouse_way.clear();
            // Zero the no-mouse-move accumulator so the
            // hover-trajectory gate (`TIME_TRAJECTORY_DISPLAY`)
            // doesn't re-arm immediately after a modal dialog or task
            // switch.
            self.time_no_mouse_move = 0;
        }
        if fx.cancel_multi_selection {
            self.input.multi_selection_active = false;
            self.input.multi_unselection_active = false;
            self.input.draw_multi_selection = false;
        }
        if let Some(top_left) = fx.pending_minimap_position {
            // Write the new minimap top-left back to the active player
            // profile on every accepted move.  Persist via the global
            // `PlayerProfileManager` and save to disk; failures are
            // logged and otherwise tolerated (the sim has already
            // accepted the new position).
            use robin_engine::player_profile::PlayerProfileManager;
            let mut guard = PlayerProfileManager::global();
            if let Some(mgr) = guard.as_mut()
                && let Some(profile) = mgr.get_active_mut()
            {
                profile.minimap_x = top_left.x;
                profile.minimap_y = top_left.y;
                if let Err(e) = mgr.save() {
                    tracing::warn!("failed to persist minimap position to profile: {e}");
                }
            }
        }
        if fx.pending_swordfight_drag_ignore && self.input.is_dragging {
            // Selected PC left Swordfighting this tick; if a drag was
            // in flight, raise `IgnoreMouseEvent(true, true, true)` so
            // the drag doesn't bleed into a click-release or a
            // subsequent double-click.
            self.input.ignore_mouse_event(true, true, true);
        }
        self.skip_render = fx.skip_render;
        // Dispatch sim-emitted sound commands onto the SoundManager.
        // Most variants queue into `SoundManager::pending_sounds` and
        // are played out by `SoundManager::hourglass`; the two that
        // need access to `engine.sound_sim.sources` (ResumeAllSources,
        // ActivateSource) are stashed on host and drained by
        // game_session before the hourglass call.
        for cmd in fx.sounds {
            use robin_engine::engine::SoundCommand;
            match cmd {
                SoundCommand::StopExclamation { actor_id } => {
                    self.pending_stop_exclamations.push(actor_id.0);
                }
                SoundCommand::Exclamation {
                    group,
                    profile_id,
                    exclamation_id,
                    variant,
                    position,
                    actor_id,
                } => {
                    if let Some(actor_id) = actor_id {
                        let had_deferred_stop =
                            self.pending_stop_exclamations.contains(&actor_id.0);
                        if had_deferred_stop {
                            self.pending_stop_exclamations
                                .retain(|id| *id != actor_id.0);
                            self.sound.drop_pending_exclamations(actor_id.0);
                            self.pending_stop_exclamation_channels.push(actor_id.0);
                        }
                    }
                    self.sound.play_exclamation(
                        group,
                        profile_id,
                        exclamation_id,
                        variant,
                        position,
                        actor_id.map(|id| id.0),
                    );
                }
                SoundCommand::Fx {
                    fx_id,
                    position,
                    material,
                } => {
                    self.sound.queue_fx(fx_id, position, material);
                }
                SoundCommand::StrikeFx {
                    strike_kind,
                    weapon1,
                    weapon2,
                    position,
                } => {
                    self.sound
                        .queue_strike_fx(strike_kind, weapon1, weapon2, position);
                }
                SoundCommand::ImpactFx {
                    impact_kind,
                    weapon,
                    armor,
                    position,
                } => {
                    self.sound
                        .queue_impact_fx(impact_kind, weapon, armor, position);
                }
                SoundCommand::Jingle(jingle) => {
                    self.sound.queue_jingle(jingle);
                }
                SoundCommand::SetMusicMode(mode) => {
                    self.sound.set_music_mode(mode);
                }
                SoundCommand::ForceMusicMode(mode) => {
                    self.sound.force_music_mode(mode);
                }
                SoundCommand::SetListenPoint { .. } => {
                    // Local viewport state lives on Host now. The engine's
                    // shared cutscene-camera listener is still emitted for
                    // deterministic legacy plumbing, but native playback must
                    // use the actual viewport the player is looking at.
                    self.sync_sound_listener();
                }
                SoundCommand::PlayDelayedSource(idx) => {
                    self.pending_play_delayed_sources.push(idx);
                }
                SoundCommand::ResumeAllSources { position, zoom } => {
                    self.pending_resume_all_sources = Some((position, zoom));
                }
                SoundCommand::ActivateSource(idx) => {
                    self.pending_activate_sources.push(idx);
                }
            }
        }
        // Accumulate UI-request queues — the host drives the widgets
        // asynchronously so signals outlive a single tick.
        self.pending_dialogues.extend(fx.pending_dialogues);
        self.pending_popup_texts.extend(fx.pending_popup_texts);
        self.pending_debriefings.extend(fx.pending_debriefings);
        self.pending_sherwood_report |= fx.pending_sherwood_report;
        self.pending_show_console |= fx.pending_show_console;
        self.pending_silent_win_widget_swap |= fx.pending_silent_win_widget_swap;
        if fx.pending_mission_state_notice {
            self.pending_mission_state_notice = true;
            self.pending_mission_state_popup = true;
        }
        self.pending_reset_input |= fx.pending_reset_input;
        self.ui_focus |= fx.ui_has_focus;
        // Per-frame mark requests from sim-side Mark() calls (currently
        // `RHScript::AddPCToMissionTeam` → `EngineCommand::MarkPc`).
        // Accumulates with host-side mark sources (requirements-bar
        // hover, portrait guard hover); the render loop drains the
        // buffer right after the outline pass.
        self.input.marked_pc_ids.extend(fx.pending_mark_pc_ids);
        // Patch-effect background decal changes are accumulated across
        // frames until the next render pass drains them.
        self.pending_bg_blits.extend(fx.bg_blits);
        fx.code
    }

    pub fn sync_sound_listener(&mut self) {
        self.sound.set_listen_point(
            self.viewport.sound_listen_point(),
            self.viewport.zoom_factor,
        );
    }

    pub fn install_trajectory_ground_mark_sprite(&mut self, data: &GroundMarkSpriteData) {
        self.trajectory_ground_mark.set_sprite_data(
            data.half_w,
            data.half_h,
            data.frame_sizes.clone(),
            data.per_frame_offsets.clone(),
        );
    }
}
