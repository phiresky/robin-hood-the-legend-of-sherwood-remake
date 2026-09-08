//! Cooperative pause-side screens.
//!
//! Unlike the compatibility `show_*` menu helpers, these states advance at
//! most one UI frame per mission frame.  The mission driver therefore keeps
//! servicing networking, HTTP control, replay bookkeeping, and simulation
//! while a local side screen is open.

use crate::gfx_types::{GameEvent, Keycode};
use crate::ingame_menu::layout::{
    MenuTransform, dim_screen, draw_screen_background, enter_modal_gpu_phase,
    poll_events_with_transform, render_text_virt_font, wrap_text_for_box_font,
};
use crate::ingame_menu::resources::{
    IngameMenuResources, MT_BTN_BACK, MT_BTN_CANCEL, MT_BTN_DELETE, MT_BTN_GRAPHICS, MT_BTN_LOAD,
    MT_BTN_OK, MT_BTN_SAVE, MT_BTN_SHORTCUTS, MT_BTN_SOUNDS, MT_MSG_REALLY_DELETE_SAVEGAME,
    MT_MSG_REALLY_OVERWRITE_SAVEGAME, MT_TTL_GRAPHICS, MT_TTL_OPTIONS, MT_TTL_SOUNDS,
};
use crate::ingame_menu::widget_bridge::{self, ModalCursor, ModalInputState};
use crate::ingame_menu::{SaveLoadMode, YesNoModalState};
use crate::key_config::{KeyConfig, REAL_KEY_COUNT};
use crate::options_model::{
    GraphicsSetting, adjust_graphics_setting, available_graphics_settings, graphics_setting_label,
    toggle_label,
};
use crate::options_model::{SoundSetting, sound_eq};
#[cfg(test)]
use crate::options_model::{graphic_eq, graphics_settings_for_retroarch_availability};
use crate::renderer::Renderer;
use crate::save_file::GameSaveFile;
use crate::savegame::SaveGameManager;
use crate::sound::{AudioBackend, SoundManager};
use crate::widget::FrameWnd;
use robin_engine::gameplay_config::GameplayConfig;
use robin_engine::graphic_config::GraphicConfig;
use robin_engine::multiplayer_config::MultiplayerConfig;
use robin_engine::profiles::ProfileManager;
use robin_engine::sound_cache::SampleLoader;
use robin_engine::sound_config::SoundConfig;
use serde::{Deserialize, Serialize};
use winit::keyboard::KeyCode;

const BUTTON_X: i32 = 330;
const BUTTON_Y: i32 = 36;
const BUTTON_GAP: i32 = 2;
const MAX_PAGE_BUTTONS: usize = 8;
const OPTIONS_SETTINGS_PER_PAGE: usize = 12;
const OPTIONS_SETTING_ROW_START_Y: i32 = 112;
const SPELLFORGE_CONTENT_BUTTON_Y: i32 = 350;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OptionRowAction {
    Enter(OptionsPage),
    AdjustGraphics(GraphicsSetting),
    AdjustSound(SoundSetting),
    AdjustGameplay(crate::ingame_menu::gameplay::GameplaySetting),
    #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
    AdjustMultiplayerPrivacy,
    Rebind(u16),
    ShortcutPreset(u8),
    PreviousPage,
    NextPage,
    ManageSpellforgeContent,
    AcceptPage,
    CancelPage,
    #[cfg(all(
        feature = "dialogs",
        any(target_os = "windows", target_os = "linux", target_os = "macos")
    ))]
    ChangeDataDir,
    Finish,
}

impl OptionRowAction {
    fn is_adjustment(self) -> bool {
        match self {
            Self::AdjustGraphics(_) | Self::AdjustSound(_) | Self::AdjustGameplay(_) => true,
            #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
            Self::AdjustMultiplayerPrivacy => true,
            _ => false,
        }
    }

    fn is_fixed_page_action(self) -> bool {
        matches!(
            self,
            Self::ManageSpellforgeContent | Self::AcceptPage | Self::CancelPage
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OptionRow {
    pub(super) action: OptionRowAction,
    pub(super) label: String,
    pub(super) help: Option<String>,
    pub(super) enabled: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct OptionsPager {
    page: usize,
}

impl OptionsPager {
    pub(super) fn page_count(total_settings: usize) -> usize {
        total_settings.div_ceil(OPTIONS_SETTINGS_PER_PAGE).max(1)
    }

    pub(super) fn visible_range(self, total_settings: usize) -> std::ops::Range<usize> {
        let start = self
            .page
            .min(Self::page_count(total_settings) - 1)
            .saturating_mul(OPTIONS_SETTINGS_PER_PAGE);
        start..(start + OPTIONS_SETTINGS_PER_PAGE).min(total_settings)
    }

    pub(super) fn can_move_previous(self) -> bool {
        self.page > 0
    }

    pub(super) fn can_move_next(self, total_settings: usize) -> bool {
        self.page + 1 < Self::page_count(total_settings)
    }

    pub(super) fn move_by(&mut self, delta: i32, total_settings: usize) -> bool {
        let last_page = Self::page_count(total_settings) - 1;
        let next_page = if delta < 0 {
            self.page.saturating_sub(1)
        } else if delta > 0 {
            (self.page + 1).min(last_page)
        } else {
            self.page
        };
        let changed = next_page != self.page;
        self.page = next_page;
        changed
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct OptionsTaskResult {
    pub(super) profile_id: u32,
    pub(super) graphic_config: GraphicConfig,
    /// Authoritative mission-facing values used for command dispatch.
    pub(super) gameplay_config: GameplayConfig,
    /// Profile-facing values. A multiplayer client retains every future-host
    /// rule preference while current host-owned rows are displayed read-only.
    pub(super) profile_gameplay_config: GameplayConfig,
    pub(super) multiplayer_config: MultiplayerConfig,
    /// Mission-facing sound values. `amount_of_speaking` is authoritative
    /// simulation state; the remaining values are local presentation.
    pub(super) sound_config: SoundConfig,
    /// Profile-facing sound values. A multiplayer client retains its own
    /// future-host speech-frequency preference.
    pub(super) profile_sound_config: SoundConfig,
    pub(super) key_config: KeyConfig,
    pub(super) custom_key_config: KeyConfig,
    pub(super) changed: bool,
    pub(super) resolution_changed: bool,
    pub(super) key_config_changed: bool,
    pub(super) original_amount_of_speaking: u16,
    pub(super) original_gameplay_config: GameplayConfig,
}

pub(super) enum UiTaskOutcome {
    ReturnToPause,
    OptionsAccepted(OptionsTaskResult),
    SaveLoadSelected {
        mode: SaveLoadMode,
        filename: String,
        mission_id: u32,
    },
    QuickLoadAccepted {
        slot: usize,
        mission_id: u32,
        save: Box<GameSaveFile>,
    },
    QuickLoadCancelled,
    QuitMissionRequested,
    ExitRequested,
    MissionEndLeaderboardFinished(
        Option<crate::leaderboard_mission_end::MissionEndLeaderboardController>,
    ),
}

pub(super) enum ActiveUiTask {
    Options(OptionsTaskState),
    SaveLoad(SaveLoadTaskState),
    Quit(YesNoModalState),
    QuickLoad(QuickLoadTaskState),
    MissionEndLeaderboard(super::leaderboard_runtime::MissionEndLeaderboardTaskState),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum UiTaskKind {
    Options,
    SaveLoad,
    QuitConfirmation,
    QuickLoadConfirmation,
    MissionEndLeaderboard,
}

impl ActiveUiTask {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn tick(
        &mut self,
        application_context: &crate::host::ApplicationContext,
        window: &mut crate::window::GameWindow,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
        save_manager: &mut SaveGameManager,
        profiles: Option<&ProfileManager>,
        sound: Option<&mut SoundManager>,
        audio_backend: Option<&mut dyn AudioBackend>,
        sample_loader: Option<&SampleLoader>,
    ) -> Option<UiTaskOutcome> {
        match self {
            Self::Options(state) => state.tick(
                application_context,
                window,
                renderer,
                resources,
                cursor,
                sound,
                audio_backend,
                sample_loader,
            ),
            Self::SaveLoad(state) => state.tick(
                window,
                renderer,
                resources,
                cursor,
                save_manager,
                profiles,
                sound,
                audio_backend,
                sample_loader,
            ),
            Self::Quit(state) => {
                let events = window.poll_events();
                let exit_requested = events.iter().any(|event| matches!(event, GameEvent::Quit));
                let result = state.handle_events(&events);
                state.render_overlay(renderer, resources, cursor);
                renderer.present();
                if exit_requested {
                    return Some(UiTaskOutcome::ExitRequested);
                }
                result.map(|yes| {
                    if yes {
                        UiTaskOutcome::QuitMissionRequested
                    } else {
                        UiTaskOutcome::ReturnToPause
                    }
                })
            }
            Self::QuickLoad(state) => state.tick(window, renderer, resources, cursor),
            Self::MissionEndLeaderboard(state) => {
                match state.tick(window, renderer, resources, cursor) {
                    super::leaderboard_runtime::MissionEndLeaderboardTaskProgress::Pending => None,
                    super::leaderboard_runtime::MissionEndLeaderboardTaskProgress::Finished => {
                        Some(UiTaskOutcome::MissionEndLeaderboardFinished(None))
                    }
                    super::leaderboard_runtime::MissionEndLeaderboardTaskProgress::Detach(
                        controller,
                    ) => Some(UiTaskOutcome::MissionEndLeaderboardFinished(Some(
                        controller,
                    ))),
                }
            }
        }
    }

    pub(super) fn is_mission_end_leaderboard(&self) -> bool {
        matches!(self, Self::MissionEndLeaderboard(_))
    }

    pub(super) fn owns_presentation(&self) -> bool {
        match self {
            Self::MissionEndLeaderboard(state) => state.owns_presentation(),
            Self::Options(_) | Self::SaveLoad(_) | Self::Quit(_) | Self::QuickLoad(_) => true,
        }
    }

    pub(super) fn kind(&self) -> UiTaskKind {
        match self {
            Self::Options(_) => UiTaskKind::Options,
            Self::SaveLoad(_) => UiTaskKind::SaveLoad,
            Self::Quit(_) => UiTaskKind::QuitConfirmation,
            Self::QuickLoad(_) => UiTaskKind::QuickLoadConfirmation,
            Self::MissionEndLeaderboard(_) => UiTaskKind::MissionEndLeaderboard,
        }
    }

    /// Resolve a local task without accepting a destructive or mutating
    /// action. HTTP automation uses this before stepping when auto-dismiss is
    /// enabled; callers still own pause-menu restoration and GPU cleanup.
    pub(super) fn auto_dismiss(&mut self) -> UiTaskOutcome {
        match self {
            Self::QuickLoad(_) => UiTaskOutcome::QuickLoadCancelled,
            Self::Options(_) | Self::SaveLoad(_) | Self::Quit(_) => UiTaskOutcome::ReturnToPause,
            Self::MissionEndLeaderboard(_) => {
                panic!("mission-end leaderboard auto-dismiss must preserve background work")
            }
        }
    }

    pub(super) fn cleanup(&mut self) {
        if let Self::SaveLoad(state) = self {
            state.cleanup();
        }
    }
}

impl UiTaskKind {
    /// Require the per-request opt-in before HTTP stepping cancels a local
    /// pause-side task. These tasks are presentation state rather than
    /// authoritative [`robin_engine::player_command::ModalKind`] values, so a
    /// typed gameplay-modal dismissal cannot stand in for `auto_dismiss`.
    pub(super) fn require_http_auto_dismiss(
        self,
        policy: &crate::http_server::StepModalPolicy,
    ) -> Result<(), String> {
        if self == Self::MissionEndLeaderboard {
            return Err(
                "mission-end leaderboard cannot be discarded by HTTP stepping; dismiss it in the game"
                    .to_owned(),
            );
        }
        if policy.auto_dismiss {
            return Ok(());
        }
        Err(format!(
            "blocked by local UI task {self:?}; retry with auto_dismiss=true or dismiss it in the game"
        ))
    }
}

pub(super) struct QuickLoadTaskState {
    dialog: YesNoModalState,
    slot: usize,
    mission_id: u32,
    save: Option<Box<GameSaveFile>>,
}

impl QuickLoadTaskState {
    pub(super) fn new(
        window: &crate::window::GameWindow,
        renderer: &Renderer,
        resources: &IngameMenuResources,
        message: String,
        slot: usize,
        mission_id: u32,
        save: GameSaveFile,
    ) -> Self {
        Self {
            dialog: YesNoModalState::new(window, renderer, resources, message),
            slot,
            mission_id,
            save: Some(Box::new(save)),
        }
    }

    fn tick(
        &mut self,
        window: &mut crate::window::GameWindow,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
    ) -> Option<UiTaskOutcome> {
        let events = window.poll_events();
        if events.iter().any(|event| matches!(event, GameEvent::Quit)) {
            self.dialog.render_overlay(renderer, resources, cursor);
            renderer.present();
            return Some(UiTaskOutcome::ExitRequested);
        }
        let result = self.dialog.handle_events(&events);
        self.dialog.render_overlay(renderer, resources, cursor);
        renderer.present();
        result.map(|accepted| {
            if accepted {
                UiTaskOutcome::QuickLoadAccepted {
                    slot: self.slot,
                    mission_id: self.mission_id,
                    save: self
                        .save
                        .take()
                        .expect("accepted QuickLoad must retain its decoded payload"),
                }
            } else {
                UiTaskOutcome::QuickLoadCancelled
            }
        })
    }
}

pub(super) use crate::options_model::OptionsPage;

pub(super) struct OptionsTaskState {
    profile_id: u32,
    controller: crate::options_model::OptionsController,
    original_gameplay: GameplayConfig,
    original_profile_gameplay: GameplayConfig,
    original_multiplayer: MultiplayerConfig,
    original_profile_sound: SoundConfig,
    original_keys: Vec<Option<KeyCode>>,
    original_amount_of_speaking: u16,
    frame: FrameWnd,
    rows: Vec<OptionRow>,
    selected: usize,
    pager: OptionsPager,
    input: ModalInputState,
    transform: MenuTransform,
    shortcut_scroll: usize,
    rebinding: Option<u16>,
    shortcut_dirty: bool,
    shortcut_reserved: bool,
    can_3d_sound: bool,
    host_gameplay_rules_editable: bool,
    localized_gameplay: crate::ingame_menu::gameplay::LocalizedGameplayText,
    spellforge_content:
        Option<crate::ingame_menu::spellforge_content::SpellforgeContentSettingsState>,
}

impl OptionsTaskState {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        application_context: &crate::host::ApplicationContext,
        window: &crate::window::GameWindow,
        renderer: &Renderer,
        resources: &IngameMenuResources,
        profile_id: u32,
        graphic: GraphicConfig,
        gameplay: GameplayConfig,
        profile_gameplay: GameplayConfig,
        multiplayer: MultiplayerConfig,
        sound: SoundConfig,
        profile_sound: SoundConfig,
        keys: KeyConfig,
        custom_keys: KeyConfig,
        can_3d_sound: bool,
        host_gameplay_rules_editable: bool,
    ) -> Self {
        let transform = MenuTransform::centered(
            renderer.screen_width() as i32,
            renderer.screen_height() as i32,
        );
        let mut input = ModalInputState::new();
        input.seed_mouse_from_window(window, transform);
        let original_keys = key_vec(&keys);
        let mut state = Self {
            profile_id,
            original_gameplay: gameplay,
            original_profile_gameplay: profile_gameplay,
            original_multiplayer: multiplayer,
            original_profile_sound: profile_sound,
            original_keys,
            original_amount_of_speaking: sound.amount_of_speaking,
            controller: crate::options_model::OptionsController::new(
                graphic,
                sound,
                gameplay,
                multiplayer,
                keys,
                custom_keys,
            ),
            frame: FrameWnd::default(),
            rows: Vec::new(),
            selected: 0,
            pager: OptionsPager::default(),
            input,
            transform,
            shortcut_scroll: 0,
            rebinding: None,
            shortcut_dirty: false,
            shortcut_reserved: false,
            can_3d_sound,
            host_gameplay_rules_editable,
            localized_gameplay:
                crate::ingame_menu::gameplay::LocalizedGameplayText::from_application_context(
                    application_context,
                ),
            spellforge_content: None,
        };
        state.rebuild_frame(resources);
        state
    }

    #[allow(clippy::too_many_arguments)]
    fn tick(
        &mut self,
        application_context: &crate::host::ApplicationContext,
        window: &mut crate::window::GameWindow,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
        mut sound_manager: Option<&mut SoundManager>,
        audio_backend: Option<&mut dyn AudioBackend>,
        sample_loader: Option<&SampleLoader>,
    ) -> Option<UiTaskOutcome> {
        if let Some(content) = self.spellforge_content.as_mut() {
            let outcome = content.tick(application_context, window, renderer, resources, cursor);
            match outcome {
                crate::ingame_menu::spellforge_content::SpellforgeContentSettingsOutcome::Pending => {
                    return None;
                }
                crate::ingame_menu::spellforge_content::SpellforgeContentSettingsOutcome::Closed => {
                    self.spellforge_content = None;
                    self.transform = MenuTransform::centered(
                        renderer.screen_width() as i32,
                        renderer.screen_height() as i32,
                    );
                    self.input.seed_mouse_from_window(window, self.transform);
                    self.render(renderer, resources, cursor);
                    renderer.present();
                    return None;
                }
                crate::ingame_menu::spellforge_content::SpellforgeContentSettingsOutcome::ExitRequested => {
                    return Some(UiTaskOutcome::ExitRequested);
                }
            }
        }

        let (events, transform) = poll_events_with_transform(window, renderer);
        self.transform = transform;
        if events.iter().any(|event| matches!(event, GameEvent::Quit)) {
            self.render(renderer, resources, cursor);
            renderer.present();
            return Some(UiTaskOutcome::ExitRequested);
        }

        if self.controller.page == OptionsPage::Shortcuts && self.rebinding.is_some() {
            for event in &events {
                match event {
                    GameEvent::KeyDown {
                        physical_key: Some(key),
                        ..
                    } if is_reserved_key(*key) => {
                        // Match the legacy shortcuts picker: a reserved key
                        // is rejected without abandoning the active row, so
                        // the player can immediately try another binding.
                        self.shortcut_reserved = true;
                        self.rebuild_frame(resources);
                    }
                    GameEvent::KeyDown {
                        physical_key: Some(key),
                        ..
                    } => {
                        let row = self.rebinding.take().expect("rebind row must exist");
                        assign_key(&mut self.controller.keys, row, *key);
                        self.controller.keys.key_type = 1;
                        self.shortcut_dirty = true;
                        self.shortcut_reserved = false;
                        self.rebuild_frame(resources);
                    }
                    _ => {}
                }
            }
        } else {
            for event in &events {
                self.input.update_from_event(event, self.transform);
                match event {
                    GameEvent::KeyDown {
                        keycode: Keycode::Escape,
                        ..
                    } => {
                        if let Some(outcome) = self.cancel_or_leave(resources) {
                            self.render(renderer, resources, cursor);
                            renderer.present();
                            return Some(outcome);
                        }
                    }
                    GameEvent::KeyDown {
                        keycode: Keycode::Up,
                        ..
                    } => self.selected = self.selected.saturating_sub(1),
                    GameEvent::KeyDown {
                        keycode: Keycode::Down,
                        ..
                    } => self.selected = (self.selected + 1).min(self.rows.len().saturating_sub(1)),
                    GameEvent::KeyDown {
                        keycode: Keycode::Left,
                        ..
                    } => self.adjust_selected(-1, resources),
                    GameEvent::KeyDown {
                        keycode: Keycode::Right,
                        ..
                    } => self.adjust_selected(1, resources),
                    GameEvent::KeyDown {
                        keycode: Keycode::Return | Keycode::KpEnter,
                        ..
                    } => {
                        if let Some(outcome) = self.activate(
                            self.selected,
                            application_context,
                            window,
                            renderer,
                            resources,
                        ) {
                            self.render(renderer, resources, cursor);
                            renderer.present();
                            return Some(outcome);
                        }
                    }
                    GameEvent::MouseWheel(delta)
                        if self.controller.page == OptionsPage::Shortcuts =>
                    {
                        let max =
                            REAL_KEY_COUNT as usize - MAX_PAGE_BUTTONS.min(REAL_KEY_COUNT as usize);
                        if *delta > 0 {
                            self.shortcut_scroll = self.shortcut_scroll.saturating_sub(1);
                        } else if *delta < 0 {
                            self.shortcut_scroll = (self.shortcut_scroll + 1).min(max);
                        }
                        self.rebuild_frame(resources);
                    }
                    GameEvent::MouseWheel(delta) if self.controller.page != OptionsPage::Hub => {
                        if *delta > 0 {
                            self.change_options_page(-1, resources);
                        } else if *delta < 0 {
                            self.change_options_page(1, resources);
                        }
                    }
                    _ => {}
                }
            }

            let widget_input = self.input.as_widget_input();
            let widget_events = self.frame.process_input(&widget_input);
            self.input.end_frame();
            play_button_noise(
                &widget_events,
                sound_manager.as_deref_mut(),
                audio_backend,
                sample_loader,
            );
            if let Some(id) = widget_bridge::find_activated(&widget_events)
                && let Some(outcome) = self.activate(
                    id as usize,
                    application_context,
                    window,
                    renderer,
                    resources,
                )
            {
                self.render(renderer, resources, cursor);
                renderer.present();
                return Some(outcome);
            }
        }

        self.render(renderer, resources, cursor);
        renderer.present();
        None
    }

    fn activate(
        &mut self,
        index: usize,
        application_context: &crate::host::ApplicationContext,
        window: &crate::window::GameWindow,
        renderer: &Renderer,
        resources: &IngameMenuResources,
    ) -> Option<UiTaskOutcome> {
        self.selected = index.min(self.rows.len().saturating_sub(1));
        let Some(row) = self.rows.get(self.selected).cloned() else {
            return None;
        };
        if !row.enabled {
            return None;
        }
        match row.action {
            OptionRowAction::Enter(page) => self.enter_page(page, resources),
            OptionRowAction::AdjustGraphics(_)
            | OptionRowAction::AdjustSound(_)
            | OptionRowAction::AdjustGameplay(_) => self.adjust_selected(1, resources),
            #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
            OptionRowAction::AdjustMultiplayerPrivacy => self.adjust_selected(1, resources),
            OptionRowAction::Rebind(index) => {
                self.rebinding = Some(index);
                self.shortcut_reserved = false;
                self.rebuild_frame(resources);
            }
            OptionRowAction::ShortcutPreset(preset) => {
                use crate::options_model::{
                    ShortcutPolicy, ShortcutPreset, select_shortcut_preset,
                };
                let preset = match preset {
                    0 => ShortcutPreset::Default,
                    1 => ShortcutPreset::Alternate,
                    2 => ShortcutPreset::Custom,
                    _ => panic!("unknown shortcut preset {preset}"),
                };
                select_shortcut_preset(
                    &mut self.controller.keys,
                    &mut self.controller.custom_keys,
                    &mut self.shortcut_dirty,
                    preset,
                    ShortcutPolicy::Cooperative,
                );
                self.shortcut_reserved = false;
                self.rebuild_frame(resources);
            }
            OptionRowAction::PreviousPage => self.change_options_page(-1, resources),
            OptionRowAction::NextPage => self.change_options_page(1, resources),
            OptionRowAction::ManageSpellforgeContent => {
                self.spellforge_content = Some(
                    crate::ingame_menu::spellforge_content::SpellforgeContentSettingsState::new(
                        application_context,
                        window,
                        renderer,
                        resources,
                    ),
                );
            }
            OptionRowAction::AcceptPage => self.accept_page(resources),
            OptionRowAction::CancelPage => self.restore_page(resources),
            #[cfg(all(
                feature = "dialogs",
                any(target_os = "windows", target_os = "linux", target_os = "macos")
            ))]
            OptionRowAction::ChangeDataDir => {
                crate::datadir_locator::change_datadir_interactive();
            }
            OptionRowAction::Finish => return Some(self.finish()),
        }
        None
    }

    fn adjust_selected(&mut self, delta: i32, resources: &IngameMenuResources) {
        let Some(action) = self.rows.get(self.selected).map(|row| row.action) else {
            return;
        };
        match action {
            OptionRowAction::AdjustGraphics(setting) => {
                if !adjust_graphics_setting(&mut self.controller.graphic.working, setting, delta) {
                    return;
                }
            }
            OptionRowAction::AdjustSound(setting) => {
                if !crate::options_model::adjust_sound_setting(
                    &mut self.controller.sound.working,
                    setting,
                    delta,
                    self.can_3d_sound,
                    self.host_gameplay_rules_editable,
                ) {
                    return;
                }
            }
            OptionRowAction::AdjustGameplay(setting)
                if gameplay_setting_editable(setting, self.host_gameplay_rules_editable) =>
            {
                crate::ingame_menu::gameplay::apply_setting(&mut self.controller.gameplay, setting)
            }
            #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
            OptionRowAction::AdjustMultiplayerPrivacy => {
                self.controller.multiplayer.publish_browser_join_links =
                    !self.controller.multiplayer.publish_browser_join_links;
            }
            _ => return,
        }
        self.rebuild_frame(resources);
    }

    fn change_options_page(&mut self, delta: i32, resources: &IngameMenuResources) {
        let total_settings = self
            .all_rows(resources)
            .into_iter()
            .filter(|row| row.action.is_adjustment())
            .count();
        if self.pager.move_by(delta, total_settings) {
            self.selected = 0;
            self.rebuild_frame(resources);
        }
    }

    fn enter_page(&mut self, page: OptionsPage, resources: &IngameMenuResources) {
        self.controller.enter_page(page);
        self.selected = 0;
        self.pager = OptionsPager::default();
        self.rebinding = None;
        self.shortcut_dirty = false;
        self.shortcut_reserved = false;
        self.rebuild_frame(resources);
    }

    fn accept_page(&mut self, resources: &IngameMenuResources) {
        if self.controller.page == OptionsPage::Shortcuts {
            promote_shortcut_edits(
                &self.controller.keys,
                &mut self.controller.custom_keys,
                &mut self.shortcut_dirty,
            );
        }
        self.controller.accept_page(false);
        self.selected = 0;
        self.pager = OptionsPager::default();
        self.rebinding = None;
        self.shortcut_dirty = false;
        self.shortcut_reserved = false;
        self.rebuild_frame(resources);
    }

    fn restore_page(&mut self, resources: &IngameMenuResources) {
        self.controller.cancel_page();
        self.selected = 0;
        self.pager = OptionsPager::default();
        self.rebinding = None;
        self.shortcut_dirty = false;
        self.shortcut_reserved = false;
        self.rebuild_frame(resources);
    }

    fn cancel_or_leave(&mut self, resources: &IngameMenuResources) -> Option<UiTaskOutcome> {
        if self.controller.page == OptionsPage::Hub {
            Some(self.finish())
        } else {
            self.restore_page(resources);
            None
        }
    }

    fn finish(&self) -> UiTaskOutcome {
        let resolution_changed = self.controller.graphic.resolution_changed();
        let key_config_changed = key_vec(&self.controller.keys) != self.original_keys;
        let profile_gameplay = profile_gameplay_after_options(
            self.controller.gameplay,
            self.original_profile_gameplay,
            self.host_gameplay_rules_editable,
        );
        let profile_sound = profile_sound_after_options(
            self.controller.sound.working,
            self.original_profile_sound,
            self.host_gameplay_rules_editable,
        );
        let changed = self.controller.graphic.changed()
            || self.controller.gameplay != self.original_gameplay
            || profile_gameplay != self.original_profile_gameplay
            || self.controller.multiplayer != self.original_multiplayer
            || self.controller.sound.changed()
            || !sound_eq(&profile_sound, &self.original_profile_sound);
        UiTaskOutcome::OptionsAccepted(OptionsTaskResult {
            profile_id: self.profile_id,
            graphic_config: self.controller.graphic.working.clone(),
            gameplay_config: self.controller.gameplay,
            profile_gameplay_config: profile_gameplay,
            multiplayer_config: self.controller.multiplayer,
            sound_config: self.controller.sound.working,
            profile_sound_config: profile_sound,
            key_config: self.controller.keys.clone(),
            custom_key_config: self.controller.custom_keys.clone(),
            changed,
            resolution_changed,
            key_config_changed,
            original_amount_of_speaking: self.original_amount_of_speaking,
            original_gameplay_config: self.original_gameplay,
        })
    }

    fn all_rows(&self, resources: &IngameMenuResources) -> Vec<OptionRow> {
        let row = |action, label: String, enabled| OptionRow {
            action,
            label,
            help: None,
            enabled,
        };
        match self.controller.page {
            OptionsPage::Hub => {
                let mut rows = vec![
                    row(
                        OptionRowAction::Enter(OptionsPage::Graphics),
                        resources.menu_text.get(MT_BTN_GRAPHICS),
                        true,
                    ),
                    row(
                        OptionRowAction::Enter(OptionsPage::Sounds),
                        resources.menu_text.get(MT_BTN_SOUNDS),
                        true,
                    ),
                    row(
                        OptionRowAction::Enter(OptionsPage::Shortcuts),
                        resources.menu_text.get(MT_BTN_SHORTCUTS),
                        true,
                    ),
                    row(
                        OptionRowAction::Enter(OptionsPage::Gameplay),
                        "Gameplay".to_string(),
                        true,
                    ),
                ];
                #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
                rows.push(row(
                    OptionRowAction::Enter(OptionsPage::MultiplayerPrivacy),
                    "Multiplayer / Privacy".to_string(),
                    true,
                ));
                #[cfg(all(
                    feature = "dialogs",
                    any(target_os = "windows", target_os = "linux", target_os = "macos")
                ))]
                rows.push(row(
                    OptionRowAction::ChangeDataDir,
                    "Game Data Folder".to_string(),
                    true,
                ));
                rows.push(row(
                    OptionRowAction::Finish,
                    resources.menu_text.get(MT_BTN_BACK),
                    true,
                ));
                rows
            }
            OptionsPage::Graphics => {
                let preset = crate::shader_preset::retroarch_presets()
                    .iter()
                    .find(|preset| preset.id == self.controller.graphic.working.shader_preset)
                    .map(|preset| preset.label.as_str())
                    .unwrap_or("Default");
                let mut rows = available_graphics_settings()
                    .into_iter()
                    .map(|setting| {
                        row(
                            OptionRowAction::AdjustGraphics(setting),
                            graphics_setting_label(
                                &self.controller.graphic.working,
                                preset,
                                setting,
                            ),
                            true,
                        )
                    })
                    .collect::<Vec<_>>();
                rows.extend(options_footer_rows(resources));
                rows
            }
            OptionsPage::Sounds => {
                let labels = vec![
                    toggle_label("3D Sound", self.controller.sound.working.sound_3d),
                    toggle_label("8-bit Sound", self.controller.sound.working.sound_8bit),
                    format!("FX Volume: {}", self.controller.sound.working.fx_volume),
                    format!(
                        "Dialogue Volume: {}",
                        self.controller.sound.working.dialogue_volume
                    ),
                    format!(
                        "Music Volume: {}",
                        self.controller.sound.working.music_volume
                    ),
                    format!(
                        "Comment Volume: {}",
                        self.controller.sound.working.exclamation_volume
                    ),
                    format!(
                        "Comment Frequency: {}",
                        self.controller.sound.working.amount_of_speaking
                    ),
                ];
                let mut rows = labels
                    .into_iter()
                    .zip(SoundSetting::ALL)
                    .map(|(label, setting)| {
                        row(
                            OptionRowAction::AdjustSound(setting),
                            label,
                            (setting != SoundSetting::ThreeDimensional || self.can_3d_sound)
                                && (!setting.requires_host_authority()
                                    || self.host_gameplay_rules_editable),
                        )
                    })
                    .collect::<Vec<_>>();
                rows.extend(options_footer_rows(resources));
                rows
            }
            OptionsPage::Gameplay => {
                let mut rows = crate::ingame_menu::gameplay::GameplaySetting::ALL
                    .into_iter()
                    .map(|setting| {
                        let index = setting.index();
                        let base_label = self.localized_gameplay.option_label(index);
                        let label = if setting
                            == crate::ingame_menu::gameplay::GameplaySetting::CampaignPresentation
                        {
                            format!(
                                "Campaign Presentation: {}",
                                self.controller.gameplay.campaign_presentation.label()
                            )
                        } else {
                            toggle_label(base_label, setting.is_selected(&self.controller.gameplay))
                        };
                        let mut row = row(
                            OptionRowAction::AdjustGameplay(setting),
                            label,
                            gameplay_setting_editable(setting, self.host_gameplay_rules_editable),
                        );
                        row.help = Some(self.localized_gameplay.option_tooltip(index).to_string());
                        row
                    })
                    .collect::<Vec<_>>();
                rows.push(row(
                    OptionRowAction::ManageSpellforgeContent,
                    self.localized_gameplay.manage_content().to_string(),
                    true,
                ));
                rows.extend(options_footer_rows(resources));
                rows
            }
            #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
            OptionsPage::MultiplayerPrivacy => {
                let mut rows = vec![row(
                    OptionRowAction::AdjustMultiplayerPrivacy,
                    toggle_label(
                        "Publish Browser Join Links",
                        self.controller.multiplayer.publish_browser_join_links,
                    ),
                    true,
                )];
                rows.extend(options_footer_rows(resources));
                rows
            }
            OptionsPage::Shortcuts => {
                let visible = MAX_PAGE_BUTTONS.min(REAL_KEY_COUNT as usize);
                let mut rows = (0..visible)
                    .map(|offset| {
                        let index = self.shortcut_scroll + offset;
                        let action = KEY_ACTIONS.get(index).copied().unwrap_or("Unknown");
                        let key = self.controller.keys.get_key_by_index(index as u16);
                        let label = if self.rebinding == Some(index as u16) {
                            if self.shortcut_reserved {
                                format!("{action}: <Reserved key>")
                            } else {
                                format!("{action}: <Press a key>")
                            }
                        } else {
                            format!(
                                "{action}: {}",
                                key.map_or_else(|| "None".into(), |key| format!("{key:?}"))
                            )
                        };
                        row(OptionRowAction::Rebind(index as u16), label, true)
                    })
                    .collect::<Vec<_>>();
                rows.extend([
                    row(
                        OptionRowAction::ShortcutPreset(0),
                        "Default 1".to_string(),
                        true,
                    ),
                    row(
                        OptionRowAction::ShortcutPreset(1),
                        "Default 2".to_string(),
                        true,
                    ),
                    row(
                        OptionRowAction::ShortcutPreset(2),
                        "User Defined".to_string(),
                        true,
                    ),
                    row(
                        OptionRowAction::AcceptPage,
                        resources.menu_text.get(MT_BTN_OK),
                        true,
                    ),
                    row(
                        OptionRowAction::CancelPage,
                        resources.menu_text.get(MT_BTN_CANCEL),
                        true,
                    ),
                ]);
                rows
            }
        }
    }

    fn rebuild_frame(&mut self, resources: &IngameMenuResources) {
        let all_rows = self.all_rows(resources);
        self.rows = if matches!(
            self.controller.page,
            OptionsPage::Hub | OptionsPage::Shortcuts
        ) {
            all_rows
        } else {
            let (settings, footer): (Vec<_>, Vec<_>) = all_rows
                .into_iter()
                .partition(|row| !row.action.is_fixed_page_action());
            let total_settings = settings.len();
            self.pager.page = self
                .pager
                .page
                .min(OptionsPager::page_count(total_settings) - 1);
            let mut visible = settings[self.pager.visible_range(total_settings)].to_vec();
            visible.push(OptionRow {
                action: OptionRowAction::PreviousPage,
                label: "Previous Page".to_string(),
                help: Some("Show the previous settings page.".to_string()),
                enabled: self.pager.can_move_previous(),
            });
            visible.push(OptionRow {
                action: OptionRowAction::NextPage,
                label: "Next Page".to_string(),
                help: Some("Show the next settings page.".to_string()),
                enabled: self.pager.can_move_next(total_settings),
            });
            visible.extend(footer);
            visible
        };
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        let (button_w, button_h) = resources.button_dimensions();
        let setting_button_w = button_w.min(290);
        let row_h = if self.controller.page == OptionsPage::Shortcuts && self.rows.len() > 12 {
            27
        } else {
            button_h.min(34)
        };
        let mut frame = FrameWnd::default();
        frame.enabled = true;
        frame.input_enabled = true;
        let mut settings_seen = 0usize;
        for (index, row) in self.rows.iter().enumerate() {
            let (x, y, width, height) = match row.action {
                action if action.is_adjustment() => {
                    let setting = settings_seen;
                    settings_seen += 1;
                    (
                        if setting < 6 { 30 } else { 320 },
                        OPTIONS_SETTING_ROW_START_Y
                            + i32::try_from(setting % 6).expect("option row fits i32")
                                * (row_h + BUTTON_GAP),
                        setting_button_w,
                        row_h,
                    )
                }
                OptionRowAction::PreviousPage => (30, 388, button_w, row_h),
                OptionRowAction::NextPage => (30, 388 + row_h + BUTTON_GAP, button_w, row_h),
                OptionRowAction::ManageSpellforgeContent => {
                    (320, SPELLFORGE_CONTENT_BUTTON_Y, setting_button_w, row_h)
                }
                OptionRowAction::AcceptPage if self.controller.page != OptionsPage::Shortcuts => {
                    (640 - button_w, 388, button_w, row_h)
                }
                OptionRowAction::CancelPage if self.controller.page != OptionsPage::Shortcuts => {
                    (640 - button_w, 388 + row_h + BUTTON_GAP, button_w, row_h)
                }
                _ => (
                    BUTTON_X,
                    BUTTON_Y + index as i32 * (row_h + BUTTON_GAP),
                    button_w,
                    row_h,
                ),
            };
            let display_label = crate::ingame_menu::gameplay::fit_button_label(
                resources,
                &row.label,
                row.enabled,
                width,
            );
            frame.add_widget_absolute(widget_bridge::make_button_enabled(
                index as u32,
                &display_label,
                row.enabled,
                x,
                y,
                width,
                height,
            ));
        }
        self.frame = frame;
    }

    fn render(
        &mut self,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
    ) {
        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);
        if let Some(background) = resources.menu_bg[2] {
            draw_screen_background(renderer, &background);
        }
        let title = match self.controller.page {
            OptionsPage::Hub => resources.menu_text.get(MT_TTL_OPTIONS),
            OptionsPage::Graphics => resources.menu_text.get(MT_TTL_GRAPHICS),
            OptionsPage::Sounds => resources.menu_text.get(MT_TTL_SOUNDS),
            OptionsPage::Shortcuts => resources.menu_text.get(MT_BTN_SHORTCUTS),
            OptionsPage::Gameplay => "Gameplay".to_string(),
            #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
            OptionsPage::MultiplayerPrivacy => "Multiplayer / Privacy".to_string(),
        };
        if let Some(font) = resources.title_font_any() {
            render_text_virt_font(renderer, font, self.transform, &title, 20, 20);
        }
        if let Some(font) = resources.label_font_any() {
            let fallback_help = match self.controller.page {
                OptionsPage::Hub => "Select a settings page.",
                OptionsPage::Shortcuts => "Click a binding, then press a key. Mouse wheel scrolls.",
                #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
                OptionsPage::MultiplayerPrivacy => {
                    "Applies to the next hosted game; traffic remains end-to-end encrypted."
                }
                _ => "Click a value or use Left/Right. OK accepts; Cancel restores.",
            };
            let help = self
                .rows
                .get(self.selected)
                .and_then(|row| row.help.as_deref())
                .unwrap_or(fallback_help);
            let wrapped = wrap_text_for_box_font(font, help, 592, 2);
            let mut lines = wrapped.lines;
            if !wrapped.remaining.is_empty()
                && let Some(last) = lines.last_mut()
            {
                let marked = format!("{last}…");
                *last =
                    crate::ingame_menu::gameplay::elide_to_width_by(&marked, 592, |candidate| {
                        font.text_width(candidate)
                    });
            }
            for (line, y) in lines
                .iter()
                .zip((0..).map(|row| 62 + row * (font.height() as i32 + 2)))
            {
                render_text_virt_font(renderer, font, self.transform, line, 24, y);
            }
        }
        for (index, widget) in self.frame.widgets().iter().enumerate() {
            widget_bridge::draw_widget_button(
                renderer,
                resources,
                self.transform,
                widget,
                index == self.selected,
            );
        }
        if let Some(cursor) = cursor {
            cursor.draw(renderer, self.transform, &self.input);
        }
    }
}

fn gameplay_setting_editable(
    setting: crate::ingame_menu::gameplay::GameplaySetting,
    host_gameplay_rules_editable: bool,
) -> bool {
    !setting.requires_host_authority() || host_gameplay_rules_editable
}

fn profile_gameplay_after_options(
    working: GameplayConfig,
    original_profile: GameplayConfig,
    host_gameplay_rules_editable: bool,
) -> GameplayConfig {
    if host_gameplay_rules_editable {
        return working;
    }
    GameplayConfig {
        // These rows display the current host's authoritative values to
        // clients, but must not overwrite the client's preferences for a
        // future session they host themselves.
        fix_hard_reaction_times: original_profile.fix_hard_reaction_times,
        enable_unbinding: original_profile.enable_unbinding,
        clean_hands_npc_kills_invalidate: original_profile.clean_hands_npc_kills_invalidate,
        reusable_cloaks: original_profile.reusable_cloaks,
        item_gameplay: original_profile.item_gameplay,
        noise_distraction_feedback: original_profile.noise_distraction_feedback,
        sherwood_trading: original_profile.sherwood_trading,
        enable_timed_missions: original_profile.enable_timed_missions,
        enable_dynamic_ambience: original_profile.enable_dynamic_ambience,
        diplomacy: original_profile.diplomacy,
        npc_faction_wars: original_profile.npc_faction_wars,
        more_combat_gestures: original_profile.more_combat_gestures,
        gesture_quality_damage: original_profile.gesture_quality_damage,
        fog_of_war: original_profile.fog_of_war,
        ..working
    }
}

fn profile_sound_after_options(
    mut working: SoundConfig,
    original_profile: SoundConfig,
    host_gameplay_rules_editable: bool,
) -> SoundConfig {
    if !host_gameplay_rules_editable {
        // Speech frequency participates in deterministic chorus suppression;
        // clients display the host value read-only without adopting it.
        working.amount_of_speaking = original_profile.amount_of_speaking;
    }
    working
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SaveRow {
    New,
    Existing(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SaveConfirmation {
    Overwrite(String),
    Delete(String),
}

pub(super) struct SaveLoadTaskState {
    mode: SaveLoadMode,
    mission_id: u32,
    visible: Vec<String>,
    selected: Option<SaveRow>,
    scroll: usize,
    name: String,
    input: ModalInputState,
    transform: MenuTransform,
    buttons: FrameWnd,
    confirmation: Option<(SaveConfirmation, YesNoModalState)>,
    text_input_active: bool,
    detailed_metadata: bool,
    local_time_zone: Option<jiff::tz::TimeZone>,
    clock_error_reported: bool,
    multiplayer_connected: bool,
}

impl SaveLoadTaskState {
    pub(super) fn new(
        window: &crate::window::GameWindow,
        renderer: &Renderer,
        resources: &IngameMenuResources,
        save_manager: &mut SaveGameManager,
        mission_id: u32,
        detailed_metadata: bool,
        mode: SaveLoadMode,
        multiplayer_connected: bool,
    ) -> Self {
        save_manager.sort_by_time();
        let visible = visible_save_filenames(save_manager, mode, multiplayer_connected);
        let selected = (mode == SaveLoadMode::Save).then_some(SaveRow::New);
        let transform = MenuTransform::centered(
            renderer.screen_width() as i32,
            renderer.screen_height() as i32,
        );
        let mut input = ModalInputState::new();
        input.seed_mouse_from_window(window, transform);
        let buttons = save_buttons(resources, mode, selected);
        let text_input_active = mode == SaveLoadMode::Save;
        if text_input_active {
            crate::window::start_text_input();
        }
        Self {
            mode,
            mission_id,
            visible,
            selected,
            scroll: 0,
            name: String::new(),
            input,
            transform,
            buttons,
            confirmation: None,
            text_input_active,
            detailed_metadata,
            local_time_zone: jiff::tz::TimeZone::try_system()
                .inspect_err(|error| tracing::warn!("Save menu local time is unavailable: {error}"))
                .ok(),
            clock_error_reported: false,
            multiplayer_connected,
        }
    }

    fn tick(
        &mut self,
        window: &mut crate::window::GameWindow,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
        save_manager: &mut SaveGameManager,
        profiles: Option<&ProfileManager>,
        sound_manager: Option<&mut SoundManager>,
        audio_backend: Option<&mut dyn AudioBackend>,
        sample_loader: Option<&SampleLoader>,
    ) -> Option<UiTaskOutcome> {
        let events = window.poll_events();
        let exit_requested = events.iter().any(|event| matches!(event, GameEvent::Quit));

        if self.confirmation.is_some() {
            if exit_requested {
                // Keep the topmost confirmation visible for any screenshot
                // captured on the final frame instead of exposing the picker
                // underneath while the native window is closing.
                self.render(renderer, resources, None, save_manager);
                let (_, dialog) = self.confirmation.as_mut().expect("confirmation exists");
                dialog.render_overlay(renderer, resources, cursor);
                renderer.present();
                return Some(UiTaskOutcome::ExitRequested);
            }
            let result = {
                let (_, dialog) = self.confirmation.as_mut().expect("confirmation exists");
                dialog.handle_events(&events)
            };
            self.render(renderer, resources, None, save_manager);
            {
                let (_, dialog) = self.confirmation.as_mut().expect("confirmation exists");
                dialog.render_overlay(renderer, resources, cursor);
            }
            renderer.present();
            if let Some(yes) = result {
                let (action, _) = self
                    .confirmation
                    .take()
                    .expect("resolved confirmation exists");
                if yes {
                    match action {
                        SaveConfirmation::Overwrite(filename) => {
                            let slot = save_manager
                                .find_by_filename(&filename)
                                .expect("confirmed overwrite slot disappeared");
                            let text = accepted_name(
                                &self.name,
                                save_manager,
                                slot,
                                self.mission_id,
                                profiles,
                            );
                            save_manager
                                .get_mut(slot)
                                .expect("overwrite slot exists")
                                .text = text;
                            return Some(UiTaskOutcome::SaveLoadSelected {
                                mode: self.mode,
                                filename,
                                mission_id: self.mission_id,
                            });
                        }
                        SaveConfirmation::Delete(filename) => {
                            let slot = save_manager
                                .find_by_filename(&filename)
                                .expect("confirmed delete slot disappeared");
                            if let Err(error) = save_manager.remove(slot) {
                                tracing::error!(
                                    "Delete save failed (cleanup may be pending): {error:#}"
                                );
                            }
                            save_manager.sort_by_time();
                            self.visible = visible_save_filenames(
                                save_manager,
                                self.mode,
                                self.multiplayer_connected,
                            );
                            self.selected =
                                (self.mode == SaveLoadMode::Save).then_some(SaveRow::New);
                            self.name.clear();
                            self.scroll = 0;
                            self.buttons = save_buttons(resources, self.mode, self.selected);
                        }
                    }
                }
            }
            return None;
        }

        if exit_requested {
            self.render(renderer, resources, cursor, save_manager);
            renderer.present();
            return Some(UiTaskOutcome::ExitRequested);
        }

        let mut activated = None;
        for event in &events {
            self.input.update_from_event(event, self.transform);
            match event {
                GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => activated = Some(2),
                GameEvent::KeyDown {
                    keycode: Keycode::Up,
                    ..
                } => self.move_selection(-1, save_manager, resources),
                GameEvent::KeyDown {
                    keycode: Keycode::Down,
                    ..
                } => self.move_selection(1, save_manager, resources),
                GameEvent::KeyDown {
                    keycode: Keycode::Return | Keycode::KpEnter,
                    ..
                } => activated = Some(0),
                GameEvent::KeyDown {
                    keycode: Keycode::Backspace,
                    ..
                } if self.mode == SaveLoadMode::Save => {
                    self.name.pop();
                }
                GameEvent::TextInput { text } if self.mode == SaveLoadMode::Save => {
                    for ch in text.chars().filter(|ch| !ch.is_control()) {
                        if self.name.chars().count() < 45 {
                            self.name.push(ch);
                        }
                    }
                }
                GameEvent::MouseUp(x, y, 1) => {
                    let (vx, vy) = self.transform.from_screen(*x, *y);
                    if (30..450).contains(&vx) && (38..430).contains(&vy) {
                        let row = ((vy - 42) / self.row_height()).max(0) as usize + self.scroll;
                        self.select_row(row, save_manager, resources);
                    }
                }
                GameEvent::MouseWheel(delta) => {
                    let total = self.visible.len() + usize::from(self.mode == SaveLoadMode::Save);
                    let max = total.saturating_sub(self.visible_rows());
                    if *delta > 0 {
                        self.scroll = self.scroll.saturating_sub(1);
                    } else if *delta < 0 {
                        self.scroll = (self.scroll + 1).min(max);
                    }
                }
                _ => {}
            }
        }
        let widget_input = self.input.as_widget_input();
        let widget_events = self.buttons.process_input(&widget_input);
        self.input.end_frame();
        play_button_noise(&widget_events, sound_manager, audio_backend, sample_loader);
        if let Some(id) = widget_bridge::find_activated(&widget_events) {
            activated = Some(id);
        }

        let outcome = match activated {
            Some(2) => Some(UiTaskOutcome::ReturnToPause),
            Some(1) => {
                if let Some(filename) = self.selected_filename().map(str::to_owned) {
                    let message = resources.menu_text.get(MT_MSG_REALLY_DELETE_SAVEGAME);
                    self.confirmation = Some((
                        SaveConfirmation::Delete(filename),
                        YesNoModalState::new(window, renderer, resources, message),
                    ));
                }
                None
            }
            Some(0) => self.accept(window, renderer, resources, save_manager, profiles),
            _ => None,
        };

        self.render(renderer, resources, cursor, save_manager);
        renderer.present();
        outcome
    }

    fn accept(
        &mut self,
        window: &crate::window::GameWindow,
        renderer: &Renderer,
        resources: &IngameMenuResources,
        save_manager: &mut SaveGameManager,
        profiles: Option<&ProfileManager>,
    ) -> Option<UiTaskOutcome> {
        match (self.mode, self.selected) {
            (SaveLoadMode::Save, Some(SaveRow::New)) => {
                let text = if self.name.trim().is_empty() {
                    mission_name(self.mission_id, profiles)
                        .unwrap_or_else(|| format!("Save {}", save_manager.count() + 1))
                } else {
                    self.name.trim().to_string()
                };
                let slot = save_manager.create(text, self.mission_id);
                let filename = save_manager
                    .get(slot)
                    .expect("new save slot exists")
                    .filename
                    .clone();
                Some(UiTaskOutcome::SaveLoadSelected {
                    mode: self.mode,
                    filename,
                    mission_id: self.mission_id,
                })
            }
            (SaveLoadMode::Save, Some(SaveRow::Existing(_))) => {
                let filename = self.selected_filename()?.to_owned();
                let message = resources.menu_text.get(MT_MSG_REALLY_OVERWRITE_SAVEGAME);
                self.confirmation = Some((
                    SaveConfirmation::Overwrite(filename),
                    YesNoModalState::new(window, renderer, resources, message),
                ));
                None
            }
            (SaveLoadMode::Load, Some(SaveRow::Existing(_))) => {
                Some(UiTaskOutcome::SaveLoadSelected {
                    mode: self.mode,
                    filename: self.selected_filename()?.to_owned(),
                    mission_id: self.mission_id,
                })
            }
            _ => None,
        }
    }

    fn selected_filename(&self) -> Option<&str> {
        let SaveRow::Existing(index) = self.selected? else {
            return None;
        };
        self.visible.get(index).map(String::as_str)
    }

    fn select_row(
        &mut self,
        row: usize,
        save_manager: &SaveGameManager,
        resources: &IngameMenuResources,
    ) {
        let next = if self.mode == SaveLoadMode::Save {
            if row == 0 {
                Some(SaveRow::New)
            } else {
                self.visible
                    .get(row - 1)
                    .map(|_| SaveRow::Existing(row - 1))
            }
        } else {
            self.visible.get(row).map(|_| SaveRow::Existing(row))
        };
        if next != self.selected {
            self.selected = next;
            self.sync_name(save_manager);
            self.buttons = save_buttons(resources, self.mode, self.selected);
        }
    }

    fn move_selection(
        &mut self,
        delta: i32,
        save_manager: &SaveGameManager,
        resources: &IngameMenuResources,
    ) {
        let total = self.visible.len() + usize::from(self.mode == SaveLoadMode::Save);
        if total == 0 {
            return;
        }
        let current = match self.selected {
            Some(SaveRow::New) => 0,
            Some(SaveRow::Existing(index)) => index + usize::from(self.mode == SaveLoadMode::Save),
            None => 0,
        };
        let next = (current as i32 + delta).clamp(0, total as i32 - 1) as usize;
        self.select_row(next, save_manager, resources);
        if next < self.scroll {
            self.scroll = next;
        }
        let visible_rows = self.visible_rows();
        if next >= self.scroll + visible_rows {
            self.scroll = next + 1 - visible_rows;
        }
    }

    fn sync_name(&mut self, save_manager: &SaveGameManager) {
        self.name = self
            .selected_filename()
            .and_then(|filename| save_manager.find_by_filename(filename))
            .and_then(|slot| save_manager.get(slot))
            .map(|save| save.text.clone())
            .unwrap_or_default();
    }

    fn render(
        &mut self,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
        save_manager: &SaveGameManager,
    ) {
        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);
        if let Some(background) = resources.menu_bg[3] {
            draw_screen_background(renderer, &background);
        }
        if let Some(font) = resources.title_font_any() {
            let title = if self.mode == SaveLoadMode::Save {
                "Save Game"
            } else {
                "Load Game"
            };
            render_text_virt_font(renderer, font, self.transform, title, 30, 8);
        }
        if let Some(font) = resources.label_font_any() {
            let total = self.visible.len() + usize::from(self.mode == SaveLoadMode::Save);
            let row_height = self.row_height();
            let now_unix = if self.detailed_metadata {
                match crate::save_file::unix_timestamp_now() {
                    Ok(now) => Some(now),
                    Err(error) => {
                        if !self.clock_error_reported {
                            tracing::warn!("Save menu relative time is unavailable: {error:#}");
                            self.clock_error_reported = true;
                        }
                        None
                    }
                }
            } else {
                None
            };
            for offset in 0..self.visible_rows() {
                let row = self.scroll + offset;
                if row >= total {
                    break;
                }
                let (selected, label, details) = if self.mode == SaveLoadMode::Save && row == 0 {
                    (
                        self.selected == Some(SaveRow::New),
                        "< New Save >".to_string(),
                        [
                            "Name optional - creates a new save slot".to_string(),
                            String::new(),
                        ],
                    )
                } else {
                    let index = row - usize::from(self.mode == SaveLoadMode::Save);
                    let filename = &self.visible[index];
                    let slot = save_manager
                        .find_by_filename(filename)
                        .expect("visible save disappeared");
                    let save = save_manager.get(slot).expect("visible save slot exists");
                    (
                        self.selected == Some(SaveRow::Existing(index)),
                        if save.is_autosave() {
                            format!("Autosave - {}", save.text)
                        } else {
                            save.text.clone()
                        },
                        crate::ingame_menu::save_load::cooperative_save_row_detail_lines(
                            save,
                            self.detailed_metadata,
                            now_unix,
                            self.local_time_zone.as_ref(),
                        ),
                    )
                };
                let prefix = if selected { "> " } else { "  " };
                let row_y = 42 + offset as i32 * row_height;
                render_text_virt_font(
                    renderer,
                    font,
                    self.transform,
                    &format!("{prefix}{label}"),
                    40,
                    row_y,
                );
                for (line_index, detail) in
                    details.iter().filter(|line| !line.is_empty()).enumerate()
                {
                    render_text_virt_font(
                        renderer,
                        font,
                        self.transform,
                        detail,
                        54,
                        row_y + 16 * (line_index as i32 + 1),
                    );
                }
            }
            if self.mode == SaveLoadMode::Save {
                render_text_virt_font(
                    renderer,
                    font,
                    self.transform,
                    &format!("Name: {}|", self.name),
                    34,
                    438,
                );
            }
        }
        widget_bridge::draw_frame_buttons(renderer, resources, self.transform, &self.buttons);
        if let Some(cursor) = cursor {
            cursor.draw(renderer, self.transform, &self.input);
        }
    }

    fn cleanup(&mut self) {
        if self.text_input_active {
            crate::window::stop_text_input();
            self.text_input_active = false;
        }
    }

    fn row_height(&self) -> i32 {
        if self.detailed_metadata { 52 } else { 36 }
    }

    fn visible_rows(&self) -> usize {
        if self.detailed_metadata { 7 } else { 10 }
    }
}

impl Drop for SaveLoadTaskState {
    fn drop(&mut self) {
        self.cleanup();
    }
}

fn save_buttons(
    resources: &IngameMenuResources,
    mode: SaveLoadMode,
    selected: Option<SaveRow>,
) -> FrameWnd {
    let labels = [
        resources.menu_text.get(if mode == SaveLoadMode::Save {
            MT_BTN_SAVE
        } else {
            MT_BTN_LOAD
        }),
        resources.menu_text.get(MT_BTN_DELETE),
        resources.menu_text.get(MT_BTN_CANCEL),
    ];
    let enabled = [
        matches!(
            (mode, selected),
            (SaveLoadMode::Save, Some(_)) | (SaveLoadMode::Load, Some(SaveRow::Existing(_)))
        ),
        matches!(selected, Some(SaveRow::Existing(_))),
        true,
    ];
    let (w, h) = resources.button_dimensions();
    let mut frame = FrameWnd::default();
    frame.enabled = true;
    frame.input_enabled = true;
    for (index, label) in labels.iter().enumerate() {
        frame.add_widget_absolute(widget_bridge::make_button_enabled(
            index as u32,
            label,
            enabled[index],
            640 - w - 10,
            480 - (3 - index as i32) * (h + 2) - 8,
            w,
            h,
        ));
    }
    frame
}

fn visible_save_filenames(
    save_manager: &SaveGameManager,
    mode: SaveLoadMode,
    multiplayer_connected: bool,
) -> Vec<String> {
    save_manager
        .saves
        .iter()
        .filter(|save| match mode {
            SaveLoadMode::Save => !save.is_special(),
            SaveLoadMode::Load => {
                !save.is_continue()
                    && !save.is_restart()
                    && !(multiplayer_connected && save.multiplayer_diagnostic)
            }
        })
        .map(|save| save.filename.clone())
        .collect()
}

fn accepted_name(
    input: &str,
    save_manager: &SaveGameManager,
    slot: usize,
    mission_id: u32,
    profiles: Option<&ProfileManager>,
) -> String {
    let trimmed = input.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    let existing = save_manager.get(slot).expect("accepted save slot exists");
    if !existing.text.trim().is_empty() {
        return existing.text.clone();
    }
    mission_name(mission_id, profiles)
        .unwrap_or_else(|| format!("Save {}", save_manager.count() + 1))
}

fn mission_name(mission_id: u32, profiles: Option<&ProfileManager>) -> Option<String> {
    profiles?
        .missions
        .iter()
        .find(|mission| mission.id == mission_id)
        .map(|mission| mission.mission_name.clone())
        .filter(|name| !name.trim().is_empty())
}

fn options_footer_rows(resources: &IngameMenuResources) -> [OptionRow; 2] {
    [
        OptionRow {
            action: OptionRowAction::AcceptPage,
            label: resources.menu_text.get(MT_BTN_OK),
            help: Some("Accept changes on this settings page.".to_string()),
            enabled: true,
        },
        OptionRow {
            action: OptionRowAction::CancelPage,
            label: resources.menu_text.get(MT_BTN_CANCEL),
            help: Some("Discard changes on this settings page.".to_string()),
            enabled: true,
        },
    ]
}

fn play_button_noise(
    events: &[crate::ui::UiEvent],
    sound_manager: Option<&mut SoundManager>,
    audio_backend: Option<&mut dyn AudioBackend>,
    sample_loader: Option<&SampleLoader>,
) {
    if let (Some(sound_manager), Some(sample_loader)) = (sound_manager, sample_loader) {
        widget_bridge::play_widget_noise(
            events,
            widget_bridge::WIDGET_NOISY_BUTTON,
            sound_manager,
            audio_backend,
            sample_loader,
        );
    }
}

fn key_vec(config: &KeyConfig) -> Vec<Option<KeyCode>> {
    let mut keys = vec![None; REAL_KEY_COUNT as usize];
    config.get_keys_array(&mut keys);
    keys
}

fn assign_key(config: &mut KeyConfig, target: u16, key: KeyCode) {
    crate::options_model::assign_shortcut(
        config,
        target,
        key,
        crate::options_model::ShortcutPolicy::Cooperative,
    );
}
fn promote_shortcut_edits(active: &KeyConfig, custom: &mut KeyConfig, dirty: &mut bool) {
    crate::options_model::promote_shortcut_edits(
        active,
        custom,
        dirty,
        crate::options_model::ShortcutPolicy::Cooperative,
    );
}
use crate::options_model::is_reserved_shortcut_key as is_reserved_key;

const KEY_ACTIONS: &[&str] = &[
    "Zoom In",
    "Zoom Out",
    "Scroll Up",
    "Scroll Down",
    "Scroll Left",
    "Scroll Right",
    "Minimap",
    "Character 1",
    "Character 2",
    "Character 3",
    "Character 4",
    "Character 5",
    "All Characters",
    "No Characters",
    "Crouch",
    "Stand Up",
    "Go Behind Buildings",
    "Toggle Outlines",
    "Action 1",
    "Action 2",
    "Action 3",
    "Move During Action",
    "Record Quick Action",
    "Start Quick Action",
    "Delete Quick Action",
    "Show View Cone",
    "Quick Save",
    "Quick Load",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_pager_covers_large_setting_sets_exactly_once() {
        let total = 45;
        assert_eq!(OptionsPager::page_count(total), 4);
        let covered = (0..OptionsPager::page_count(total))
            .flat_map(|page| OptionsPager { page }.visible_range(total))
            .collect::<Vec<_>>();
        assert_eq!(covered, (0..total).collect::<Vec<_>>());
        assert_eq!(OptionsPager { page: 0 }.visible_range(total), 0..12);
        assert_eq!(OptionsPager { page: 3 }.visible_range(total), 36..45);
    }

    #[test]
    fn options_pager_covers_every_integrated_gameplay_setting() {
        let total = crate::ingame_menu::gameplay::OPTION_LABELS.len();
        assert_eq!(total, 45, "update this contract when settings are added");
        assert_eq!(OptionsPager::page_count(total), 4);
        let covered = (0..OptionsPager::page_count(total))
            .flat_map(|page| OptionsPager { page }.visible_range(total))
            .collect::<Vec<_>>();
        assert_eq!(covered, (0..total).collect::<Vec<_>>());
        assert_eq!(OptionsPager { page: 3 }.visible_range(total), 36..45);
        assert!(
            OptionsPager { page: 3 }
                .visible_range(total)
                .contains(&crate::ingame_menu::gameplay::FOG_OF_WAR_OPTION_INDEX)
        );
    }

    #[test]
    fn content_manager_is_a_fixed_gameplay_action_not_a_fake_setting() {
        assert!(OptionRowAction::ManageSpellforgeContent.is_fixed_page_action());
        assert!(!matches!(
            OptionRowAction::ManageSpellforgeContent,
            OptionRowAction::AdjustGameplay(_)
        ));
        assert_eq!(
            crate::ingame_menu::gameplay::OPTION_LABELS.len(),
            45,
            "Manage Content must not consume a gameplay-setting index"
        );
    }

    #[test]
    fn cooperative_gameplay_layout_has_room_for_long_rows_and_fixed_actions() {
        let row_height = 34;
        let maximum_supported_help_font_height = 24;
        assert_eq!(
            62 + maximum_supported_help_font_height * 2 + 2,
            OPTIONS_SETTING_ROW_START_Y
        );
        for visible_index in 0..OPTIONS_SETTINGS_PER_PAGE {
            let column_index = visible_index % 6;
            let x = if visible_index < 6 { 30 } else { 320 };
            let y = OPTIONS_SETTING_ROW_START_Y + column_index as i32 * (row_height + BUTTON_GAP);
            assert!(x >= 0 && x < 640);
            assert!(y >= OPTIONS_SETTING_ROW_START_Y && y + row_height < 350);
        }
        let manage = (320, SPELLFORGE_CONTENT_BUTTON_Y, 290, row_height);
        assert!(manage.0 + manage.2 <= 640);
        assert!(manage.1 + manage.3 < 388);
    }

    #[test]
    fn cooperative_graphics_cursor_pulse_row_is_reachable_and_persistable() {
        let original = GraphicConfig::default();
        let settings = available_graphics_settings();
        let labels = settings
            .iter()
            .map(|setting| graphics_setting_label(&original, "Default", *setting))
            .collect::<Vec<_>>();
        let cursor_pulse_index = settings
            .iter()
            .position(|setting| *setting == GraphicsSetting::QuickActionCursorPulse)
            .expect("cursor pulse is exposed");
        assert_eq!(labels.len(), settings.len());
        assert_eq!(labels[cursor_pulse_index], "[x] Quick-Action Cursor Pulse");
        let containing_page = (0..OptionsPager::page_count(labels.len()))
            .find(|&page| {
                OptionsPager { page }
                    .visible_range(labels.len())
                    .contains(&cursor_pulse_index)
            })
            .expect("cursor pulse row is reachable through graphics pagination");
        assert_eq!(containing_page, 1);

        let mut accepted = original.clone();
        assert!(adjust_graphics_setting(
            &mut accepted,
            GraphicsSetting::QuickActionCursorPulse,
            1,
        ));
        assert!(!accepted.quick_action_cursor_pulse);
        assert!(
            !graphic_eq(&accepted, &original),
            "OK must report this presentation-only change for profile persistence"
        );

        let cancelled = original.clone();
        assert!(cancelled.quick_action_cursor_pulse);
        assert!(graphic_eq(&cancelled, &original));
    }

    #[test]
    fn cooperative_graphics_exposes_every_current_visual_control() {
        let original = GraphicConfig::default();
        let settings = graphics_settings_for_retroarch_availability(true);
        let labels = settings
            .iter()
            .map(|setting| graphics_setting_label(&original, "Default", *setting))
            .collect::<Vec<_>>();
        assert_eq!(labels.len(), settings.len());
        assert_eq!(OptionsPager::page_count(labels.len()), 3);
        let covered = (0..OptionsPager::page_count(labels.len()))
            .flat_map(|page| OptionsPager { page }.visible_range(labels.len()))
            .collect::<Vec<_>>();
        assert_eq!(covered, (0..settings.len()).collect::<Vec<_>>());

        for required in [
            GraphicsSetting::AdaptiveWidescreen,
            GraphicsSetting::NativeRefreshPresentation,
            GraphicsSetting::MissionCountdown,
            GraphicsSetting::DynamicAmbienceVisuals,
            GraphicsSetting::DiplomacyVisuals,
            GraphicsSetting::TextureEffect,
            GraphicsSetting::UpscaleStrength,
            GraphicsSetting::EffectTemporalFlicker,
        ] {
            let mut changed = original.clone();
            assert!(adjust_graphics_setting(&mut changed, required, 1));
            assert!(
                !graphic_eq(&changed, &original),
                "{required:?} was not detected for persistence",
            );
        }
    }

    #[test]
    fn portable_graphics_rows_hide_only_native_shader_presets() {
        let portable = graphics_settings_for_retroarch_availability(false);
        let native = graphics_settings_for_retroarch_availability(true);
        let current = available_graphics_settings();

        assert!(!portable.contains(&GraphicsSetting::ShaderPreset));
        assert!(native.contains(&GraphicsSetting::ShaderPreset));
        assert_eq!(native.len(), portable.len() + 1);
        assert!(GraphicsSetting::ALL.into_iter().all(|setting| {
            setting == GraphicsSetting::ShaderPreset
                || (portable.contains(&setting) && native.contains(&setting))
        }));
        assert_eq!(
            current.contains(&GraphicsSetting::ShaderPreset),
            crate::shader_preset::retroarch_runtime_available(),
        );
    }

    #[test]
    fn scaling_cycle_uses_only_modes_available_to_this_build() {
        let available = crate::shader_preset::available_texture_scale_modes();
        let mut config = GraphicConfig::default();

        for _ in 0..available.len() * 2 {
            assert!(adjust_graphics_setting(
                &mut config,
                GraphicsSetting::ScalingMode,
                1,
            ));
            assert!(available.contains(&config.scale_mode));
        }
        assert_eq!(
            available.contains(&robin_engine::graphic_config::TextureScaleMode::RetroArch),
            crate::shader_preset::retroarch_runtime_available(),
        );
    }

    #[test]
    fn multiplayer_client_sees_all_host_rules_read_only_and_retains_own_preferences() {
        use crate::ingame_menu::gameplay::GameplaySetting;
        for setting in GameplaySetting::ALL {
            assert_eq!(
                gameplay_setting_editable(setting, false),
                !setting.requires_host_authority(),
                "client editability drifted for {setting:?}",
            );
            assert!(gameplay_setting_editable(setting, true));
        }

        let client_profile = GameplayConfig::default();
        let host_values = GameplayConfig {
            fix_hard_reaction_times: false,
            enable_unbinding: false,
            clean_hands_npc_kills_invalidate: true,
            reusable_cloaks: false,
            item_gameplay: robin_engine::gameplay_config::ItemGameplayConfig::classic(),
            noise_distraction_feedback: false,
            sherwood_trading: false,
            enable_timed_missions: false,
            enable_dynamic_ambience: false,
            diplomacy: false,
            npc_faction_wars: false,
            more_combat_gestures: false,
            gesture_quality_damage: false,
            fog_of_war: false,
            ..client_profile
        };
        let persisted = profile_gameplay_after_options(host_values, client_profile, false);
        assert_eq!(persisted, client_profile);

        let hosted = profile_gameplay_after_options(host_values, client_profile, true);
        assert_eq!(hosted, host_values);
    }

    #[test]
    fn multiplayer_client_keeps_local_sound_but_not_host_speech_frequency() {
        assert!(SoundSetting::CommentFrequency.requires_host_authority());
        let profile = SoundConfig {
            fx_volume: 3,
            amount_of_speaking: 2,
            ..SoundConfig::default()
        };
        let host_display = SoundConfig {
            fx_volume: 7,
            amount_of_speaking: 8,
            ..profile
        };
        let persisted = profile_sound_after_options(host_display, profile, false);
        assert_eq!(persisted.fx_volume, 7);
        assert_eq!(persisted.amount_of_speaking, 2);
        assert_eq!(
            profile_sound_after_options(host_display, profile, true).amount_of_speaking,
            8,
        );
    }

    #[test]
    fn options_pager_stops_at_navigation_boundaries() {
        let mut pager = OptionsPager::default();
        assert!(!pager.can_move_previous());
        assert!(!pager.move_by(-1, 45));
        assert!(pager.move_by(1, 45));
        assert_eq!(pager.page, 1);
        assert!(pager.move_by(99, 45));
        assert_eq!(pager.page, 2, "one activation advances exactly one page");
        assert!(pager.move_by(1, 45));
        assert_eq!(pager.page, 3);
        assert!(!pager.can_move_next(45));
        assert!(!pager.move_by(1, 45));
        assert_eq!(pager.page, 3);
    }

    #[test]
    fn stable_save_filter_matches_legacy_picker_rules() {
        let mut manager = SaveGameManager::new("/tmp/pause-ui-task-test".into());
        manager.create_with_filename("Continue".into(), "Continue".into(), 1);
        manager.create_with_filename("QuickSave".into(), "Quick".into(), 1);
        manager.create_with_filename("Manual".into(), "Manual".into(), 1);
        assert_eq!(
            visible_save_filenames(&manager, SaveLoadMode::Save, false),
            ["Manual"]
        );
        assert_eq!(
            visible_save_filenames(&manager, SaveLoadMode::Load, false),
            ["QuickSave", "Manual"]
        );
        let manual = manager.find_by_filename("Manual").unwrap();
        manager.get_mut(manual).unwrap().multiplayer_diagnostic = true;
        assert_eq!(
            visible_save_filenames(&manager, SaveLoadMode::Load, true),
            ["QuickSave"]
        );
    }

    #[test]
    fn shortcut_assignment_clears_previous_owner() {
        let mut keys = KeyConfig::default_preset();
        let key = keys.get_key_by_index(0).expect("default ZoomIn binding");
        assign_key(&mut keys, 18, key);
        assert_eq!(keys.get_key_by_index(0), None);
        assert_eq!(keys.get_key_by_index(18), Some(key));
    }

    #[test]
    fn selecting_a_preset_preserves_stored_custom_bindings() {
        let mut active = KeyConfig::default_preset();
        let custom = KeyConfig::alternate_preset();
        let custom_before = key_vec(&custom);
        let mut custom = custom;
        let mut dirty = false;

        promote_shortcut_edits(&active, &mut custom, &mut dirty);
        active = KeyConfig::alternate_preset();

        assert_eq!(key_vec(&custom), custom_before);
        assert_eq!(key_vec(&active), key_vec(&KeyConfig::alternate_preset()));
    }

    #[test]
    fn direct_shortcut_edits_are_promoted_before_a_preset_switch() {
        let mut active = KeyConfig::default_preset();
        let mut custom = KeyConfig::alternate_preset();
        let replacement = KeyCode::F6;
        assign_key(&mut active, 0, replacement);
        let mut dirty = true;

        promote_shortcut_edits(&active, &mut custom, &mut dirty);

        assert_eq!(custom.get_key_by_index(0), Some(replacement));
        assert_eq!(custom.key_type, 1);
        assert!(!dirty);
    }

    #[test]
    fn strict_http_steps_preserve_every_pause_side_task_kind() {
        let strict = crate::http_server::StepModalPolicy {
            auto_dismiss: false,
            dismissals: Vec::new(),
            synchronized_multiplayer: false,
        };
        for kind in [
            UiTaskKind::Options,
            UiTaskKind::SaveLoad,
            UiTaskKind::QuitConfirmation,
            UiTaskKind::QuickLoadConfirmation,
        ] {
            let error = kind
                .require_http_auto_dismiss(&strict)
                .expect_err("strict HTTP stepping must not cancel local UI state");
            assert!(error.contains("blocked by local UI task"));
            assert!(error.contains(&format!("{kind:?}")));
        }
    }

    #[test]
    fn default_http_policy_auto_dismisses_every_pause_side_task_kind() {
        let default_policy = crate::http_server::StepModalPolicy::default();
        assert!(default_policy.auto_dismiss);
        for kind in [
            UiTaskKind::Options,
            UiTaskKind::SaveLoad,
            UiTaskKind::QuitConfirmation,
            UiTaskKind::QuickLoadConfirmation,
        ] {
            kind.require_http_auto_dismiss(&default_policy)
                .expect("default automation policy must dismiss local UI state");
        }
    }
}
