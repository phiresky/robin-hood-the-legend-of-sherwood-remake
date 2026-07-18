//! Focused owners for the native mission driver's process resources.
//!
//! These values deliberately do not implement serde. They own GPU, window,
//! input-device, resource-cache, and widget objects whose lifetime is one
//! interactive process session; deterministic persistence remains in the
//! engine snapshot owned by [`super::runtime::MissionWorld`].

use super::modal_state::ActiveModal;
use super::render::RenderContext;
use super::runtime::MissionRuntime;
use super::setup::MissionSprites;
use super::tick::tick_audio;
use crate::Host;
use crate::audio_backend::KiraAudioBackend;
use crate::console_overlay::ConsoleOverlay;
use crate::corner_hud::{CornerButtonSprites, CornerHudLayout, CornerTooltipTracker};
use crate::game::Game;
use crate::hud_text::HudFonts;
use crate::ingame_menu::{IngameMenuResources, PauseMenu};
use crate::input::ThreadedInput;
use crate::input_translator::InputTranslator;
use crate::menu::CampaignMapState;
use crate::renderer::Renderer;
use crate::resource_manager::ResourceManager;
use crate::sherwood_hud::{
    SherwoodButtonEnable, SherwoodButtonSprites, SherwoodHudLayout, SherwoodTooltipTracker,
};
use crate::sound_cache::SampleLoader;
use crate::stature_hud::{StatureHudLayout, StatureSprites, StatureTooltipTracker};
use crate::ui_panel::{BlazonTooltipTracker, PcActionTooltipTracker, RequirementsTooltipTracker};
use crate::zoom_hud::{ZoomButtonSprites, ZoomHudLayout, ZoomTooltipTracker};
use robin_assets::res_descr::LevelDescriptors;
use robin_engine::coordinates::ScreenBBox;
use robin_engine::engine_manager::EngineManager;
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

    pub(super) fn resize(
        &mut self,
        width: u32,
        height: u32,
        key_config: &robin_assets::keyconfig::KeyConfig,
    ) {
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

    pub(super) fn tick(&mut self, manager: &mut EngineManager, host: &mut Host) {
        if let Some(backend) = self.backend.as_mut() {
            tick_audio(
                manager,
                host,
                backend,
                &*self.sample_loader,
                &mut self.sound_rng,
            );
        }
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
        host.frame_holder_mut().apply_arno_law(shadow_color);
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
/// It intentionally has no frame-loop method: callbacks, the window/event
/// pump, and frame phase ordering remain in `run_mission`.
pub(super) struct InteractiveFrontend {
    pub(super) input: MissionInput,
    pub(super) audio: MissionAudio,
    pub(super) resources: MissionResources,
    pub(super) ui: MissionUi,
    pub(super) hud: MissionHud,
    pub(super) presentation: MissionPresentation,
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
    use super::{MissionUi, RenderViewState};

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
}
