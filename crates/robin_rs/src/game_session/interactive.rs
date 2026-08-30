//! Focused owners for the native mission driver's process resources.
//!
//! These values deliberately do not implement serde. They own GPU, window,
//! input-device, resource-cache, and widget objects whose lifetime is one
//! interactive process session; deterministic persistence remains in the
//! engine snapshot owned by [`super::runtime::MissionWorld`].

use super::modal_state::ActiveModal;
use super::render::RenderContext;
use super::runtime::MissionRuntime;
use super::setup::{
    LoadedInteractiveResources, MissionProcessResources, MissionSprites, load_mission_sprites,
    setup_input_and_camera,
};
use super::tick::tick_audio;
use crate::audio_backend::KiraAudioBackend;
use crate::console_overlay::ConsoleOverlay;
use crate::corner_hud::{CornerButtonSprites, CornerHudLayout, CornerTooltipTracker};
use crate::game::Game;
use crate::host::Host;
use crate::hud_text::HudFonts;
use crate::ingame_menu::{IngameMenuResources, PauseMenu};
use crate::input::ThreadedInput;
use crate::input_translator::InputTranslator;
use crate::key_config::KeyConfig;
use crate::menu::CampaignMapState;
use crate::renderer::Renderer;
use crate::sherwood_hud::{
    SherwoodButtonEnable, SherwoodButtonSprites, SherwoodHudLayout, SherwoodTooltipTracker,
};
use crate::stature_hud::{StatureHudLayout, StatureSprites, StatureTooltipTracker};
use crate::ui_panel::{BlazonTooltipTracker, PcActionTooltipTracker, RequirementsTooltipTracker};
use crate::zoom_hud::{ZoomButtonSprites, ZoomHudLayout, ZoomTooltipTracker};
use robin_assets::res_descr::LevelDescriptors;
use robin_assets::resource_manager::ResourceManager;
use robin_engine::coordinates::ScreenBBox;
use robin_engine::engine::{Engine, SpatialPresentationSnapshot};
use robin_engine::engine_manager::EngineManager;
use robin_engine::graphic_config::TextureScaleMode;
use robin_engine::profiles::MissionLocation;
use robin_engine::sound_cache::SampleLoader;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Raw input state and the mission-specific action translator.
pub(super) struct MissionInput {
    pub(super) threaded: ThreadedInput,
    pub(super) translator: InputTranslator,
}

impl MissionInput {
    pub(super) fn new(threaded: ThreadedInput, translator: InputTranslator) -> Self {
        Self {
            threaded,
            translator,
        }
    }

    pub(super) fn resize(&mut self, width: u32, height: u32, key_config: &KeyConfig) {
        let width = width as f32;
        let height = height as f32;
        self.threaded
            .set_clipping(ScreenBBox::from_coords(0.0, 0.0, width, height));
        self.translator = InputTranslator::new(width, height);
        self.translator.load_bindings_from_keyconfig(key_config);
        self.translator.install_hud_dead_zones();
    }

    pub(super) fn reset_after_modal(&mut self) {
        self.threaded.reset_input_state();
        self.translator.reset_state();
        self.threaded.queue_mouse_motion_resync();
    }

    pub(super) fn reset_after_engine_request(&mut self) {
        self.threaded.reset_input_state();
        self.translator.reset_state();
    }
}

/// Native audio device plus mission-lifetime sample source and RNG.
pub(super) struct MissionAudio {
    pub(super) backend: Option<KiraAudioBackend>,
    pub(super) sample_loader: Box<SampleLoader>,
    pub(super) sound_rng: fastrand::Rng,
}

impl MissionAudio {
    pub(super) fn new(
        backend: Option<KiraAudioBackend>,
        sample_loader: Box<SampleLoader>,
        sound_rng: fastrand::Rng,
    ) -> Self {
        Self {
            backend,
            sample_loader,
            sound_rng,
        }
    }

    pub(super) fn tick(
        &mut self,
        manager: &mut EngineManager,
        host: &mut Host,
    ) -> Option<robin_engine::engine::SoundBoundary> {
        if let Some(backend) = self.backend.as_mut() {
            return tick_audio(
                manager,
                host,
                backend,
                &*self.sample_loader,
                &mut self.sound_rng,
            );
        }
        None
    }
}

/// Resource archives and decoded UI data shared by presentation and modals.
pub(super) struct MissionResources {
    pub(super) text: ResourceManager,
    pub(super) cursor: ResourceManager,
    pub(super) level_descriptors: Option<LevelDescriptors>,
    pub(super) hud_fonts: Option<HudFonts>,
    pub(super) short_briefing_strings: HashMap<u32, String>,
    pub(super) menu: Option<IngameMenuResources>,
}

/// Stateful menus and overlays which survive across interactive frames.
pub(super) struct MissionUi {
    pub(super) pause_menu: Option<PauseMenu>,
    pub(super) active_modal: Option<ActiveModal>,
    pub(super) console_overlay: ConsoleOverlay,
    pub(super) campaign_map: CampaignMapState,
    pub(super) restart_allowed: bool,
}

impl MissionUi {
    pub(super) fn new(restart_allowed: bool) -> Self {
        Self {
            pause_menu: None,
            active_modal: None,
            console_overlay: ConsoleOverlay::new(),
            // The map model itself is populated lazily when the overlay is
            // first raised, so this empty state always reflects live campaign
            // data rather than mission-bootstrap data.
            campaign_map: CampaignMapState::new(),
            restart_allowed,
        }
    }

    /// Close the pause surface before another blocking modal takes ownership.
    /// Session callbacks remain outside this component and resume sound/time
    /// immediately after this returns `true`.
    pub(super) fn close_pause(
        &mut self,
        input: &mut MissionInput,
        presentation: &mut MissionPresentation,
    ) -> bool {
        if self.pause_menu.take().is_none() {
            return false;
        }
        presentation.renderer.clear_frozen_scene();
        input.reset_after_modal();
        true
    }
}

/// HUD textures, layouts, enable state, and hover trackers.
pub(super) struct MissionHud {
    pub(super) sherwood_enable: SherwoodButtonEnable,
    pub(super) sherwood_sprites: SherwoodButtonSprites,
    pub(super) sherwood_layout: SherwoodHudLayout,
    pub(super) zoom_sprites: ZoomButtonSprites,
    pub(super) zoom_layout: ZoomHudLayout,
    pub(super) zoom_tooltip: ZoomTooltipTracker,
    pub(super) corner_sprites: CornerButtonSprites,
    pub(super) corner_layout: CornerHudLayout,
    pub(super) corner_tooltip: CornerTooltipTracker,
    pub(super) stature_sprites: StatureSprites,
    pub(super) stature_layout: StatureHudLayout,
    pub(super) requirements_tooltip: RequirementsTooltipTracker,
    pub(super) blazon_tooltip: BlazonTooltipTracker,
    pub(super) stature_tooltip: StatureTooltipTracker,
    pub(super) sherwood_tooltip: SherwoodTooltipTracker,
    pub(super) pc_action_tooltip: PcActionTooltipTracker,
    pub(super) last_cursor_id: i32,
}

impl MissionHud {
    pub(super) fn resize(&mut self, width: u32, height: u32) {
        self.sherwood_layout =
            SherwoodHudLayout::for_resolution(width, height, &self.sherwood_sprites);
        self.zoom_layout = ZoomHudLayout::for_resolution(width, height, &self.zoom_sprites);
        self.corner_layout = CornerHudLayout::for_resolution(width, height, &self.corner_sprites);
        self.stature_layout =
            StatureHudLayout::for_resolution(width, height, &self.stature_sprites);
    }
}

/// GPU renderer and renderer-side mission caches.
pub(super) struct MissionPresentation {
    pub(super) renderer: Renderer,
    pub(super) sprites: MissionSprites,
}

/// One fixed-tick-late, host-only presentation state. The working engine is
/// always a clone of the current authoritative tick: animation/gameplay state
/// remains at 25 Hz while only spatial transforms are sampled between the two
/// adjacent snapshots. It is never serialized into saves or rollback state.
pub(super) struct NativeRefreshInterpolation {
    previous: Option<SpatialPresentationSnapshot>,
    current: Option<SpatialPresentationSnapshot>,
    working: Option<Engine>,
    previous_camera: Option<CameraPresentationPose>,
    current_camera: Option<CameraPresentationPose>,
    latest_frame: Option<u32>,
    segment_started_at_ms: u32,
    segment_active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct CameraPresentationPose {
    view_position: robin_engine::coordinates::MapPoint,
    old_view_position: robin_engine::coordinates::MapPoint,
    zoom_factor: f32,
    old_zoom_factor: f32,
}

impl CameraPresentationPose {
    pub(super) fn capture(host: &Host) -> Self {
        Self {
            view_position: host.viewport.view_position,
            old_view_position: host.viewport.old_view_position,
            zoom_factor: host.viewport.zoom_factor,
            old_zoom_factor: host.viewport.old_zoom_factor,
        }
    }

    pub(super) fn apply(self, host: &mut Host) {
        host.viewport.view_position = self.view_position;
        host.viewport.old_view_position = self.old_view_position;
        host.viewport.zoom_factor = self.zoom_factor;
        host.viewport.old_zoom_factor = self.old_zoom_factor;
    }

    fn interpolate(previous: Self, current: Self, alpha: f32) -> Self {
        let finite = previous.view_position.x.is_finite()
            && previous.view_position.y.is_finite()
            && previous.zoom_factor.is_finite()
            && current.view_position.x.is_finite()
            && current.view_position.y.is_finite()
            && current.zoom_factor.is_finite()
            && previous.zoom_factor > 0.0
            && current.zoom_factor > 0.0;
        if !finite {
            tracing::warn!(
                ?previous,
                ?current,
                "invalid camera interpolation endpoint; snapping"
            );
            return current;
        }
        // Camera follows can inherit an entity teleport, while menu/script
        // camera jumps do not carry an engine-side transition marker at all.
        // Either kind must snap instead of sweeping across unrelated map
        // space. Ordinary edge scroll and touch inertia remain far below this
        // distance during one 40 ms fixed tick.
        const MAX_CONTINUOUS_MAP_DISTANCE_PER_TICK: f32 = 128.0;
        let camera_dx = current.view_position.x - previous.view_position.x;
        let camera_dy = current.view_position.y - previous.view_position.y;
        if camera_dx.abs().max(camera_dy.abs()) > MAX_CONTINUOUS_MAP_DISTANCE_PER_TICK {
            return current;
        }
        let lerp = |a: f32, b: f32| a + (b - a) * alpha;
        Self {
            view_position: robin_engine::coordinates::MapPoint::new(
                lerp(previous.view_position.x, current.view_position.x),
                lerp(previous.view_position.y, current.view_position.y),
            ),
            old_view_position: robin_engine::coordinates::MapPoint::new(
                lerp(previous.view_position.x, current.view_position.x),
                lerp(previous.view_position.y, current.view_position.y),
            ),
            zoom_factor: lerp(previous.zoom_factor, current.zoom_factor),
            old_zoom_factor: lerp(previous.zoom_factor, current.zoom_factor),
        }
    }
}

impl NativeRefreshInterpolation {
    fn new() -> Self {
        Self {
            previous: None,
            current: None,
            working: None,
            previous_camera: None,
            current_camera: None,
            latest_frame: None,
            segment_started_at_ms: 0,
            segment_active: false,
        }
    }

    pub(super) fn prepare_fixed_tick(
        &mut self,
        authoritative: &Engine,
        camera: CameraPresentationPose,
        started_at_ms: u32,
        enabled: bool,
    ) {
        if !enabled {
            self.clear();
            return;
        }

        let frame = authoritative.frame_counter();
        if self.latest_frame == Some(frame) {
            // Paused/locked simulation can retain one engine frame across
            // several host frames while touch, keyboard, or scripted camera
            // input still moves the host-only viewport. Start a camera-only
            // segment without replaying the already completed world segment.
            if self.current_camera != Some(camera) {
                self.previous = self.current.clone();
                self.previous_camera = self.current_camera.replace(camera);
                self.working = Some(authoritative.clone());
                self.segment_started_at_ms = started_at_ms;
                self.segment_active = true;
            }
            return;
        }
        let current = authoritative.spatial_presentation_snapshot();
        let sequential = self
            .latest_frame
            .is_some_and(|previous| previous.wrapping_add(1) == frame);
        if sequential {
            self.previous = self.current.replace(current);
            self.previous_camera = self.current_camera.replace(camera);
            self.working = Some(authoritative.clone());
            self.segment_started_at_ms = started_at_ms;
            self.segment_active = true;
        } else {
            self.previous = Some(current.clone());
            self.current = Some(current);
            self.previous_camera = Some(camera);
            self.current_camera = Some(camera);
            self.working = Some(authoritative.clone());
            self.segment_started_at_ms = started_at_ms;
            self.segment_active = false;
        }
        self.latest_frame = Some(frame);
    }

    pub(super) fn sample(&mut self, now_ms: u32) -> Option<CameraPresentationPose> {
        let alpha = if self.segment_active {
            now_ms.wrapping_sub(self.segment_started_at_ms) as f32
                / robin_engine::engine::FRAME_TIME_MS as f32
        } else {
            1.0
        }
        .clamp(0.0, 1.0);
        let previous = self.previous.as_ref()?;
        let current = self.current.as_ref()?;
        self.working
            .as_mut()?
            .apply_spatial_presentation(previous, current, alpha);
        Some(CameraPresentationPose::interpolate(
            self.previous_camera?,
            self.current_camera?,
            alpha,
        ))
    }

    pub(super) fn engine(&self) -> Option<&Engine> {
        self.working.as_ref()
    }

    pub(super) fn clear(&mut self) {
        self.previous = None;
        self.current = None;
        self.working = None;
        self.previous_camera = None;
        self.current_camera = None;
        self.latest_frame = None;
        self.segment_active = false;
    }
}

/// Renderer settings resolved before the loading screen is created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct MissionRendererConfig {
    pub(super) scale_mode: TextureScaleMode,
    pub(super) shader_preset: String,
    pub(super) native_refresh_presentation: bool,
}

/// Renderer-only stage. This value can only be constructed after the loading
/// screen owner has been consumed, making the GPU ownership handoff explicit.
pub(super) struct InteractiveRendererAssembly {
    renderer: Renderer,
}

impl InteractiveRendererAssembly {
    pub(super) fn new_after_loading_screen(
        window: &mut crate::window::GameWindow,
        config: MissionRendererConfig,
    ) -> Self {
        window.set_native_refresh_presentation(config.native_refresh_presentation);
        let render_w = window.width as u16;
        let render_h = window.height as u16;
        window.set_logical_size(u32::from(render_w), u32::from(render_h));
        let mut renderer = Renderer::new(window, render_w, render_h, config.scale_mode);
        renderer.set_shader_preset(config.shader_preset);
        Self { renderer }
    }

    /// Upload the predecoded map resources before any HUD/input frontend is
    /// assembled. Engine dimensions were already established during level
    /// construction; this stage only transfers host presentation data.
    pub(super) fn upload_maps(
        &mut self,
        engine: &robin_engine::engine::Engine,
        host: &mut Host,
        background: Option<robin_engine::engine::level_loading::PreDecodedBackground>,
        minimap: Option<robin_engine::engine::level_loading::PreDecodedMinimap>,
    ) {
        if let Some(decoded) = background {
            crate::level_loading_host::apply_background_map(
                engine,
                host,
                &mut self.renderer,
                decoded,
            );
        }
        if let Some(map) = minimap.map(|decoded| {
            crate::level_loading_host::apply_minimap(host, &mut self.renderer, decoded)
        }) {
            host.engine_display.setup_minimap_map(
                map.hit_mask,
                map.map_size,
                map.saved_position,
                f32::from(self.renderer.screen_width()),
                f32::from(self.renderer.screen_height()),
            );
        }
    }

    /// Consume pre-loop process resources into the complete frontend assembly
    /// needed by the lost-Sherwood gate. HUD trackers are intentionally added
    /// only after that gate and campaign-entry setup have succeeded.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn assemble_process_frontend(
        mut self,
        window: &mut crate::window::GameWindow,
        host: &mut Host,
        game: &Game,
        engine: &mut robin_engine::engine::Engine,
        assets: &robin_engine::engine::LevelAssets,
        process: MissionProcessResources,
        decoded: LoadedInteractiveResources,
        short_briefing_strings: HashMap<u32, String>,
        args: &crate::main_entry::CliArgs,
        mission_idx: usize,
        location: MissionLocation,
    ) -> InteractiveFrontendAssembly {
        let MissionProcessResources {
            mut text,
            mut cursor,
            audio_backend,
        } = process;
        let sprites = load_mission_sprites(
            engine,
            host,
            assets,
            &mut self.renderer,
            &mut cursor,
            &mut text,
        );
        let mut timer = super::setup::PhaseTimer::new("process frontend");
        let sample_loader = crate::audio_backend::create_sample_loader(std::path::PathBuf::from(
            &game.global_options.sound_directory,
        ));
        let sound_rng = fastrand::Rng::new();
        let (threaded_input, input_translator) = setup_input_and_camera(
            engine,
            host,
            assets,
            args,
            window.width,
            window.height,
            mission_idx,
        );
        window.grab_mouse(true);
        timer.step("input + camera");

        let menu = IngameMenuResources::new(&mut self.renderer, host.shipping.as_deref());
        if menu.is_none() {
            tracing::error!(
                "In-game menu resources unavailable — pause actions require a successful reload"
            );
        }
        timer.step("in-game menu resources");

        InteractiveFrontendAssembly {
            input: MissionInput::new(threaded_input, input_translator),
            audio: MissionAudio::new(audio_backend, sample_loader, sound_rng),
            resources: MissionResources {
                text,
                cursor,
                level_descriptors: decoded.level_descriptors,
                hud_fonts: decoded.hud_fonts,
                short_briefing_strings,
                menu,
            },
            ui: MissionUi::new(location != MissionLocation::Sherwood),
            renderer: self.renderer,
            sprites,
            is_sherwood: game.is_sherwood,
        }
    }
}

/// Frontend state complete enough to drive the blocking pre-loop campaign
/// gate, but not yet promoted to a running mission.
pub(super) struct InteractiveFrontendAssembly {
    input: MissionInput,
    audio: MissionAudio,
    pub(super) resources: MissionResources,
    ui: MissionUi,
    pub(super) renderer: Renderer,
    pub(super) sprites: MissionSprites,
    pub(super) is_sherwood: bool,
}

impl InteractiveFrontendAssembly {
    /// Add mission HUD ownership only after the lost-campaign gate and
    /// restart/Sherwood entry setup have completed.
    pub(super) fn finish(self, width: u32, height: u32) -> InteractiveFrontend {
        let Self {
            input,
            audio,
            resources,
            ui,
            mut renderer,
            sprites,
            ..
        } = self;
        let mut cursor = resources.cursor;

        let sherwood_sprites = SherwoodButtonSprites::load(&mut cursor, &mut renderer);
        let sherwood_layout = SherwoodHudLayout::for_resolution(width, height, &sherwood_sprites);
        let zoom_sprites = ZoomButtonSprites::load(&mut cursor, &mut renderer);
        let zoom_layout = ZoomHudLayout::for_resolution(width, height, &zoom_sprites);
        let corner_sprites = CornerButtonSprites::load(&mut cursor, &mut renderer);
        let corner_layout = CornerHudLayout::for_resolution(
            renderer.screen_width() as u32,
            renderer.screen_height() as u32,
            &corner_sprites,
        );
        let stature_sprites = StatureSprites::load(&mut cursor, &mut renderer);
        let stature_layout = StatureHudLayout::for_resolution(
            renderer.screen_width() as u32,
            renderer.screen_height() as u32,
            &stature_sprites,
        );
        let resources = MissionResources {
            cursor,
            ..resources
        };

        InteractiveFrontend {
            input,
            audio,
            resources,
            ui,
            hud: MissionHud {
                sherwood_enable: SherwoodButtonEnable::pre_commit(),
                sherwood_sprites,
                sherwood_layout,
                zoom_sprites,
                zoom_layout,
                zoom_tooltip: ZoomTooltipTracker::new(),
                corner_sprites,
                corner_layout,
                corner_tooltip: CornerTooltipTracker::new(),
                stature_sprites,
                stature_layout,
                requirements_tooltip: RequirementsTooltipTracker::new(),
                blazon_tooltip: BlazonTooltipTracker::new(),
                stature_tooltip: StatureTooltipTracker::new(),
                sherwood_tooltip: SherwoodTooltipTracker::new(),
                pc_action_tooltip: PcActionTooltipTracker::new(),
                last_cursor_id: robin_engine::resource_ids::RHMOUSE_DEFAULT,
            },
            presentation: MissionPresentation { renderer, sprites },
            native_refresh_interpolation: NativeRefreshInterpolation::new(),
        }
    }
}

/// Copy-only inputs for one short-lived render view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct RenderViewState {
    pub(super) shift_held: bool,
    pub(super) rewind_active: bool,
    pub(super) display_info_elapsed_secs: u32,
}

impl MissionPresentation {
    pub(super) fn render_context<'a>(
        &'a mut self,
        resources: &'a MissionResources,
        hud: &'a mut MissionHud,
        input: &'a MissionInput,
        ui: &'a mut MissionUi,
        game: &'a Game,
        state: RenderViewState,
    ) -> RenderContext<'a> {
        RenderContext {
            renderer: &mut self.renderer,
            cursor_renderer: &mut self.sprites.cursor_renderer,
            selection_mark_renderer: &mut self.sprites.selection_mark_renderer,
            titbit_renderer: &mut self.sprites.titbit_renderer,
            console_overlay: &mut ui.console_overlay,
            zoom_tooltip: &mut hud.zoom_tooltip,
            corner_tooltip: &mut hud.corner_tooltip,
            requirements_tooltip: &mut hud.requirements_tooltip,
            blazon_tooltip: &mut hud.blazon_tooltip,
            stature_tooltip: &mut hud.stature_tooltip,
            sherwood_tooltip: &mut hud.sherwood_tooltip,
            pc_action_tooltip: &mut hud.pc_action_tooltip,
            mouse_trail_renderer: self.sprites.mouse_trail_renderer.as_ref(),
            portrait_cache: &self.sprites.portrait_cache,
            menu_resources: resources.menu.as_ref(),
            hud_fonts: resources.hud_fonts.as_ref(),
            short_briefing_strings: &resources.short_briefing_strings,
            sherwood_layout: &hud.sherwood_layout,
            sherwood_sprites: &hud.sherwood_sprites,
            zoom_layout: &hud.zoom_layout,
            zoom_sprites: &hud.zoom_sprites,
            corner_layout: &hud.corner_layout,
            corner_sprites: &hud.corner_sprites,
            stature_layout: &hud.stature_layout,
            stature_sprites: &hud.stature_sprites,
            threaded_input: &input.threaded,
            game,
            pause_menu: ui.pause_menu.as_ref(),
            sherwood_enable: hud.sherwood_enable,
            shift_held: state.shift_held,
            rewind_active: state.rewind_active,
            display_info_elapsed_secs: state.display_info_elapsed_secs,
            draw_hud: true,
        }
    }

    pub(super) fn rebind_shadow_key(
        &mut self,
        resources: &mut MissionResources,
        host: &mut Host,
        gpu: &crate::window::GpuContext,
        shadow_color: u16,
    ) {
        host.rebind_frame_holder_shadow_color(shadow_color);
        self.sprites.selection_mark_renderer.load(
            &mut resources.cursor,
            &self.renderer,
            shadow_color,
        );
        self.sprites.titbit_renderer.load(
            &mut resources.cursor,
            gpu,
            shadow_color,
            self.renderer.scale_mode(),
        );
    }
}

/// Top-level owner for the native-only half of an interactive mission.
///
/// Frame ordering is implemented on [`InteractiveMission`] in `flow`; this
/// component owns only mission-lifetime frontend state and never stores the
/// borrowed window, callbacks, profile manager, or CLI services.
pub(super) struct InteractiveFrontend {
    pub(super) input: MissionInput,
    pub(super) audio: MissionAudio,
    pub(super) resources: MissionResources,
    pub(super) ui: MissionUi,
    pub(super) hud: MissionHud,
    pub(super) presentation: MissionPresentation,
    pub(super) native_refresh_interpolation: NativeRefreshInterpolation,
}

/// Complete process owner returned by interactive mission bootstrap.
///
/// It deliberately does not implement serde because its frontend owns GPU,
/// input-device, audio, and resource-cache handles.
pub(super) struct InteractiveMission {
    pub(super) runtime: MissionRuntime,
    pub(super) frontend: InteractiveFrontend,
}

#[cfg(test)]
mod tests {
    use super::{CameraPresentationPose, MissionUi, RenderViewState};

    #[test]
    fn mission_ui_starts_with_all_blocking_surfaces_closed() {
        let ui = MissionUi::new(true);

        assert!(ui.pause_menu.is_none());
        assert!(ui.active_modal.is_none());
        assert!(ui.restart_allowed);
    }

    #[test]
    fn mission_ui_preserves_sherwood_restart_policy() {
        let ui = MissionUi::new(false);

        assert!(!ui.restart_allowed);
    }

    #[test]
    fn render_view_state_round_trips_for_diagnostics() {
        let expected = RenderViewState {
            shift_held: true,
            rewind_active: false,
            display_info_elapsed_secs: 42,
        };

        let json = serde_json::to_string(&expected).expect("render view state should serialize");
        let actual: RenderViewState =
            serde_json::from_str(&json).expect("render view state should deserialize");

        assert_eq!(actual, expected);
    }

    #[test]
    fn camera_presentation_interpolates_position_and_zoom() {
        let previous = CameraPresentationPose {
            view_position: robin_engine::coordinates::MapPoint::new(100.0, 200.0),
            old_view_position: robin_engine::coordinates::MapPoint::new(90.0, 190.0),
            zoom_factor: 0.5,
            old_zoom_factor: 1.0,
        };
        let current = CameraPresentationPose {
            view_position: robin_engine::coordinates::MapPoint::new(140.0, 120.0),
            old_view_position: robin_engine::coordinates::MapPoint::new(100.0, 200.0),
            zoom_factor: 1.5,
            old_zoom_factor: 0.5,
        };

        let sample = CameraPresentationPose::interpolate(previous, current, 0.25);

        assert_eq!(
            sample.view_position,
            robin_engine::coordinates::MapPoint::new(110.0, 180.0)
        );
        assert_eq!(sample.zoom_factor, 0.75);
    }

    #[test]
    fn camera_presentation_snaps_discontinuous_jumps() {
        let previous = CameraPresentationPose {
            view_position: robin_engine::coordinates::MapPoint::new(100.0, 200.0),
            old_view_position: robin_engine::coordinates::MapPoint::new(100.0, 200.0),
            zoom_factor: 1.0,
            old_zoom_factor: 1.0,
        };
        let current = CameraPresentationPose {
            view_position: robin_engine::coordinates::MapPoint::new(500.0, 200.0),
            old_view_position: robin_engine::coordinates::MapPoint::new(100.0, 200.0),
            zoom_factor: 2.0,
            old_zoom_factor: 1.0,
        };

        assert_eq!(
            CameraPresentationPose::interpolate(previous, current, 0.25),
            current
        );
    }
}
