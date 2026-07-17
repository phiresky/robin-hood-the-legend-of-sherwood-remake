//! Focused owners for the native mission driver's process resources.
//!
//! These values deliberately do not implement serde. They own GPU, SDL,
//! input-device, resource-cache, and widget objects whose lifetime is one
//! interactive process session; deterministic persistence remains in the
//! engine snapshot owned by [`super::runtime::MissionWorld`].

use super::modal_state::ActiveModal;
use super::setup::MissionSprites;
use crate::console_overlay::ConsoleOverlay;
use crate::corner_hud::{CornerButtonSprites, CornerHudLayout, CornerTooltipTracker};
use crate::hud_text::HudFonts;
use crate::ingame_menu::{IngameMenuResources, PauseMenu};
use crate::input::ThreadedInput;
use crate::input_translator::InputTranslator;
use crate::menu::CampaignMapState;
use crate::renderer::Renderer;
use crate::resource_manager::ResourceManager;
use crate::sdl_audio::SdlMixerBackend;
use crate::sherwood_hud::{
    SherwoodButtonEnable, SherwoodButtonSprites, SherwoodHudLayout, SherwoodTooltipTracker,
};
use crate::sound_cache::SampleLoader;
use crate::stature_hud::{StatureHudLayout, StatureSprites, StatureTooltipTracker};
use crate::ui_panel::{BlazonTooltipTracker, PcActionTooltipTracker, RequirementsTooltipTracker};
use crate::zoom_hud::{ZoomButtonSprites, ZoomHudLayout, ZoomTooltipTracker};
use robin_assets::res_descr::LevelDescriptors;
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
}

/// Native audio device plus mission-lifetime sample source and RNG.
pub(super) struct MissionAudio {
    pub(super) backend: Option<SdlMixerBackend>,
    pub(super) sample_loader: Box<SampleLoader>,
    pub(super) sound_rng: fastrand::Rng,
}

impl MissionAudio {
    pub(super) fn new(
        backend: Option<SdlMixerBackend>,
        sample_loader: Box<SampleLoader>,
        sound_rng: fastrand::Rng,
    ) -> Self {
        Self {
            backend,
            sample_loader,
            sound_rng,
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

/// GPU renderer and renderer-side mission caches.
pub(super) struct MissionPresentation {
    pub(super) renderer: Renderer,
    pub(super) sprites: MissionSprites,
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

#[cfg(test)]
mod tests {
    use super::MissionUi;

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
}
