//! Cooperative pause-side screens.
//!
//! Unlike the compatibility `show_*` menu helpers, these states advance at
//! most one UI frame per mission frame.  The mission driver therefore keeps
//! servicing networking, HTTP control, replay bookkeeping, and simulation
//! while a local side screen is open.

use crate::gfx_types::{GameEvent, Keycode};
use crate::ingame_menu::layout::{
    MenuTransform, dim_screen, draw_screen_background, enter_modal_gpu_phase, render_text_virt_font,
};
use crate::ingame_menu::resources::{
    IngameMenuResources, MT_BTN_BACK, MT_BTN_CANCEL, MT_BTN_DELETE, MT_BTN_GRAPHICS, MT_BTN_LOAD,
    MT_BTN_OK, MT_BTN_SAVE, MT_BTN_SHORTCUTS, MT_BTN_SOUNDS, MT_MSG_REALLY_DELETE_SAVEGAME,
    MT_MSG_REALLY_OVERWRITE_SAVEGAME, MT_TTL_GRAPHICS, MT_TTL_OPTIONS, MT_TTL_SOUNDS,
};
use crate::ingame_menu::widget_bridge::{self, ModalCursor, ModalInputState};
use crate::ingame_menu::{SaveLoadMode, YesNoModalState};
use crate::key_config::{KeyConfig, REAL_KEY_COUNT};
use crate::renderer::Renderer;
use crate::save_file::GameSaveFile;
use crate::savegame::SaveGameManager;
use crate::sound::{AudioBackend, SoundManager};
use crate::widget::FrameWnd;
use robin_engine::gameplay_config::GameplayConfig;
use robin_engine::graphic_config::{GraphicConfig, TextureScaleMode};
use robin_engine::profiles::ProfileManager;
use robin_engine::sound_cache::SampleLoader;
use robin_engine::sound_config::SoundConfig;
use serde::{Deserialize, Serialize};
use winit::keyboard::KeyCode;

const BUTTON_X: i32 = 330;
const BUTTON_Y: i32 = 36;
const BUTTON_GAP: i32 = 2;
const MAX_PAGE_BUTTONS: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct OptionsTaskResult {
    pub(super) profile_id: u32,
    pub(super) graphic_config: GraphicConfig,
    pub(super) gameplay_config: GameplayConfig,
    pub(super) sound_config: SoundConfig,
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
}

pub(super) enum ActiveUiTask {
    Options(OptionsTaskState),
    SaveLoad(SaveLoadTaskState),
    Quit(YesNoModalState),
    QuickLoad(QuickLoadTaskState),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum UiTaskKind {
    Options,
    SaveLoad,
    QuitConfirmation,
    QuickLoadConfirmation,
}

impl ActiveUiTask {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn tick(
        &mut self,
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
        }
    }

    pub(super) fn is_quick_load(&self) -> bool {
        matches!(self, Self::QuickLoad(_))
    }

    pub(super) fn kind(&self) -> UiTaskKind {
        match self {
            Self::Options(_) => UiTaskKind::Options,
            Self::SaveLoad(_) => UiTaskKind::SaveLoad,
            Self::Quit(_) => UiTaskKind::QuitConfirmation,
            Self::QuickLoad(_) => UiTaskKind::QuickLoadConfirmation,
        }
    }

    /// Resolve a local task without accepting a destructive or mutating
    /// action. HTTP automation uses this before stepping when auto-dismiss is
    /// enabled; callers still own pause-menu restoration and GPU cleanup.
    pub(super) fn auto_dismiss(&mut self) -> UiTaskOutcome {
        match self {
            Self::QuickLoad(_) => UiTaskOutcome::QuickLoadCancelled,
            Self::Options(_) | Self::SaveLoad(_) | Self::Quit(_) => UiTaskOutcome::ReturnToPause,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OptionsPage {
    Hub,
    Graphics,
    Sounds,
    Shortcuts,
    Gameplay,
}

#[derive(Clone)]
enum PageSnapshot {
    None,
    Graphics(GraphicConfig),
    Sounds(SoundConfig),
    Shortcuts(KeyConfig, KeyConfig),
    Gameplay(GameplayConfig),
}

pub(super) struct OptionsTaskState {
    profile_id: u32,
    graphic: GraphicConfig,
    gameplay: GameplayConfig,
    sound: SoundConfig,
    keys: KeyConfig,
    custom_keys: KeyConfig,
    original_graphic: GraphicConfig,
    original_gameplay: GameplayConfig,
    original_sound: SoundConfig,
    original_keys: Vec<Option<KeyCode>>,
    original_amount_of_speaking: u16,
    page: OptionsPage,
    page_snapshot: PageSnapshot,
    frame: FrameWnd,
    labels: Vec<String>,
    selected: usize,
    input: ModalInputState,
    transform: MenuTransform,
    shortcut_scroll: usize,
    rebinding: Option<u16>,
    shortcut_dirty: bool,
    shortcut_reserved: bool,
    can_3d_sound: bool,
    sherwood_trading_editable: bool,
}

impl OptionsTaskState {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        window: &crate::window::GameWindow,
        renderer: &Renderer,
        resources: &IngameMenuResources,
        profile_id: u32,
        graphic: GraphicConfig,
        gameplay: GameplayConfig,
        sound: SoundConfig,
        keys: KeyConfig,
        custom_keys: KeyConfig,
        can_3d_sound: bool,
        sherwood_trading_editable: bool,
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
            original_graphic: graphic.clone(),
            original_gameplay: gameplay,
            original_sound: sound,
            original_keys,
            original_amount_of_speaking: sound.amount_of_speaking,
            graphic,
            gameplay,
            sound,
            keys,
            custom_keys,
            page: OptionsPage::Hub,
            page_snapshot: PageSnapshot::None,
            frame: FrameWnd::default(),
            labels: Vec::new(),
            selected: 0,
            input,
            transform,
            shortcut_scroll: 0,
            rebinding: None,
            shortcut_dirty: false,
            shortcut_reserved: false,
            can_3d_sound,
            sherwood_trading_editable,
        };
        state.rebuild_frame(resources);
        state
    }

    #[allow(clippy::too_many_arguments)]
    fn tick(
        &mut self,
        window: &mut crate::window::GameWindow,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
        mut sound_manager: Option<&mut SoundManager>,
        audio_backend: Option<&mut dyn AudioBackend>,
        sample_loader: Option<&SampleLoader>,
    ) -> Option<UiTaskOutcome> {
        let events = window.poll_events();
        if events.iter().any(|event| matches!(event, GameEvent::Quit)) {
            self.render(renderer, resources, cursor);
            renderer.present();
            return Some(UiTaskOutcome::ExitRequested);
        }

        if self.page == OptionsPage::Shortcuts && self.rebinding.is_some() {
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
                        assign_key(&mut self.keys, row, *key);
                        self.keys.key_type = 1;
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
                    } => {
                        self.selected = (self.selected + 1).min(self.labels.len().saturating_sub(1))
                    }
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
                        if let Some(outcome) = self.activate(self.selected, resources) {
                            self.render(renderer, resources, cursor);
                            renderer.present();
                            return Some(outcome);
                        }
                    }
                    GameEvent::MouseWheel(delta) if self.page == OptionsPage::Shortcuts => {
                        let max =
                            REAL_KEY_COUNT as usize - MAX_PAGE_BUTTONS.min(REAL_KEY_COUNT as usize);
                        if *delta > 0 {
                            self.shortcut_scroll = self.shortcut_scroll.saturating_sub(1);
                        } else if *delta < 0 {
                            self.shortcut_scroll = (self.shortcut_scroll + 1).min(max);
                        }
                        self.rebuild_frame(resources);
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
                && let Some(outcome) = self.activate(id as usize, resources)
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

    fn activate(&mut self, index: usize, resources: &IngameMenuResources) -> Option<UiTaskOutcome> {
        self.selected = index.min(self.labels.len().saturating_sub(1));
        match self.page {
            OptionsPage::Hub => match index {
                0 => self.enter_page(OptionsPage::Graphics, resources),
                1 => self.enter_page(OptionsPage::Sounds, resources),
                2 => self.enter_page(OptionsPage::Shortcuts, resources),
                3 => self.enter_page(OptionsPage::Gameplay, resources),
                4 => {
                    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
                    crate::datadir_locator::change_datadir_interactive();
                    #[cfg(not(any(
                        target_os = "windows",
                        target_os = "linux",
                        target_os = "macos"
                    )))]
                    return Some(self.finish());
                }
                #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
                5 => return Some(self.finish()),
                _ => {}
            },
            OptionsPage::Graphics => match index {
                0..=9 => self.adjust_selected(1, resources),
                10 => self.accept_page(resources),
                11 => self.restore_page(resources),
                _ => {}
            },
            OptionsPage::Sounds => match index {
                0..=6 => self.adjust_selected(1, resources),
                7 => self.accept_page(resources),
                8 => self.restore_page(resources),
                _ => {}
            },
            OptionsPage::Gameplay => match index {
                index if index < crate::ingame_menu::gameplay::OPTION_LABELS.len() => {
                    self.adjust_selected(1, resources)
                }
                index if index == crate::ingame_menu::gameplay::OPTION_LABELS.len() => {
                    self.accept_page(resources)
                }
                index if index == crate::ingame_menu::gameplay::OPTION_LABELS.len() + 1 => {
                    self.restore_page(resources)
                }
                _ => {}
            },
            OptionsPage::Shortcuts => {
                let visible = MAX_PAGE_BUTTONS.min(REAL_KEY_COUNT as usize);
                match index {
                    i if i < visible => {
                        self.rebinding = Some((self.shortcut_scroll + i) as u16);
                        self.shortcut_reserved = false;
                        self.rebuild_frame(resources);
                    }
                    i if i == visible => {
                        promote_shortcut_edits(
                            &self.keys,
                            &mut self.custom_keys,
                            &mut self.shortcut_dirty,
                        );
                        self.keys = KeyConfig::default_preset();
                        self.rebuild_frame(resources);
                    }
                    i if i == visible + 1 => {
                        promote_shortcut_edits(
                            &self.keys,
                            &mut self.custom_keys,
                            &mut self.shortcut_dirty,
                        );
                        self.keys = KeyConfig::alternate_preset();
                        self.rebuild_frame(resources);
                    }
                    i if i == visible + 2 => {
                        self.keys = self.custom_keys.clone();
                        self.keys.key_type = 1;
                        self.shortcut_dirty = false;
                        self.shortcut_reserved = false;
                        self.rebuild_frame(resources);
                    }
                    i if i == visible + 3 => self.accept_page(resources),
                    i if i == visible + 4 => self.restore_page(resources),
                    _ => {}
                }
            }
        }
        None
    }

    fn adjust_selected(&mut self, delta: i32, resources: &IngameMenuResources) {
        match (self.page, self.selected) {
            (OptionsPage::Graphics, 0) => {
                const MODES: &[(f32, f32)] = &[(640.0, 480.0), (800.0, 600.0), (1024.0, 768.0)];
                let current = MODES
                    .iter()
                    .position(|(w, h)| {
                        (self.graphic.resolution_x - w).abs() < 0.5
                            && (self.graphic.resolution_y - h).abs() < 0.5
                    })
                    .unwrap_or(0);
                let next = cycle_index(current, MODES.len(), delta);
                self.graphic.set_resolution(MODES[next].0, MODES[next].1);
            }
            (OptionsPage::Graphics, 1) => {
                self.graphic.framed_view_cone = !self.graphic.framed_view_cone
            }
            (OptionsPage::Graphics, 2) => {
                self.graphic.display_shadow = !self.graphic.display_shadow
            }
            (OptionsPage::Graphics, 3) => {
                self.graphic.display_titbits = !self.graphic.display_titbits
            }
            (OptionsPage::Graphics, 4) => self.graphic.display_anim = !self.graphic.display_anim,
            (OptionsPage::Graphics, 5) => {
                self.graphic.apply_fog_to_all_sprites = !self.graphic.apply_fog_to_all_sprites
            }
            (OptionsPage::Graphics, 6) => self.graphic.fullscreen = !self.graphic.fullscreen,
            (OptionsPage::Graphics, 7) => {
                self.graphic.hardware_cursor = !self.graphic.hardware_cursor
            }
            (OptionsPage::Graphics, 8) => {
                let all = TextureScaleMode::ALL;
                let current = all
                    .iter()
                    .position(|mode| *mode == self.graphic.scale_mode)
                    .unwrap_or(0);
                self.graphic.scale_mode = all[cycle_index(current, all.len(), delta)];
            }
            (OptionsPage::Graphics, 9) => {
                let presets = crate::shader_preset::retroarch_presets();
                if !presets.is_empty() {
                    let current = presets
                        .iter()
                        .position(|preset| preset.id == self.graphic.shader_preset)
                        .unwrap_or(0);
                    self.graphic.shader_preset = presets
                        [cycle_index(current, presets.len(), delta)]
                    .id
                    .clone();
                }
            }
            (OptionsPage::Sounds, 0) if self.can_3d_sound => {
                self.sound.sound_3d = !self.sound.sound_3d
            }
            (OptionsPage::Sounds, 1) => self.sound.sound_8bit = !self.sound.sound_8bit,
            (OptionsPage::Sounds, index @ 2..=6) => {
                let value = sound_value_mut(&mut self.sound, index - 2);
                *value = (*value as i32 + delta).clamp(0, 9) as u16;
            }
            (OptionsPage::Gameplay, index)
                if index < crate::ingame_menu::gameplay::OPTION_LABELS.len()
                    && (index != crate::ingame_menu::gameplay::SHERWOOD_TRADING_OPTION_INDEX
                        || self.sherwood_trading_editable) =>
            {
                crate::ingame_menu::gameplay::apply_option_toggle(&mut self.gameplay, index)
            }
            _ => return,
        }
        self.rebuild_frame(resources);
    }

    fn enter_page(&mut self, page: OptionsPage, resources: &IngameMenuResources) {
        self.page_snapshot = match page {
            OptionsPage::Graphics => PageSnapshot::Graphics(self.graphic.clone()),
            OptionsPage::Sounds => PageSnapshot::Sounds(self.sound),
            OptionsPage::Shortcuts => {
                PageSnapshot::Shortcuts(self.keys.clone(), self.custom_keys.clone())
            }
            OptionsPage::Gameplay => PageSnapshot::Gameplay(self.gameplay),
            OptionsPage::Hub => PageSnapshot::None,
        };
        self.page = page;
        self.selected = 0;
        self.rebinding = None;
        self.shortcut_dirty = false;
        self.shortcut_reserved = false;
        self.rebuild_frame(resources);
    }

    fn accept_page(&mut self, resources: &IngameMenuResources) {
        if self.page == OptionsPage::Shortcuts {
            promote_shortcut_edits(&self.keys, &mut self.custom_keys, &mut self.shortcut_dirty);
        }
        self.page = OptionsPage::Hub;
        self.page_snapshot = PageSnapshot::None;
        self.selected = 0;
        self.rebinding = None;
        self.shortcut_dirty = false;
        self.shortcut_reserved = false;
        self.rebuild_frame(resources);
    }

    fn restore_page(&mut self, resources: &IngameMenuResources) {
        match std::mem::replace(&mut self.page_snapshot, PageSnapshot::None) {
            PageSnapshot::Graphics(value) => self.graphic = value,
            PageSnapshot::Sounds(value) => self.sound = value,
            PageSnapshot::Shortcuts(active, custom) => {
                self.keys = active;
                self.custom_keys = custom;
            }
            PageSnapshot::Gameplay(value) => self.gameplay = value,
            PageSnapshot::None => {}
        }
        self.page = OptionsPage::Hub;
        self.selected = 0;
        self.rebinding = None;
        self.shortcut_dirty = false;
        self.shortcut_reserved = false;
        self.rebuild_frame(resources);
    }

    fn cancel_or_leave(&mut self, resources: &IngameMenuResources) -> Option<UiTaskOutcome> {
        if self.page == OptionsPage::Hub {
            Some(self.finish())
        } else {
            self.restore_page(resources);
            None
        }
    }

    fn finish(&self) -> UiTaskOutcome {
        let resolution_changed =
            (self.graphic.resolution_x - self.original_graphic.resolution_x).abs() > 0.5
                || (self.graphic.resolution_y - self.original_graphic.resolution_y).abs() > 0.5;
        let key_config_changed = key_vec(&self.keys) != self.original_keys;
        let changed = !graphic_eq(&self.graphic, &self.original_graphic)
            || self.gameplay != self.original_gameplay
            || !sound_eq(&self.sound, &self.original_sound);
        UiTaskOutcome::OptionsAccepted(OptionsTaskResult {
            profile_id: self.profile_id,
            graphic_config: self.graphic.clone(),
            gameplay_config: self.gameplay,
            sound_config: self.sound,
            key_config: self.keys.clone(),
            custom_key_config: self.custom_keys.clone(),
            changed,
            resolution_changed,
            key_config_changed,
            original_amount_of_speaking: self.original_amount_of_speaking,
            original_gameplay_config: self.original_gameplay,
        })
    }

    fn labels(&self, resources: &IngameMenuResources) -> Vec<String> {
        match self.page {
            OptionsPage::Hub => {
                let mut labels = vec![
                    resources.menu_text.get(MT_BTN_GRAPHICS),
                    resources.menu_text.get(MT_BTN_SOUNDS),
                    resources.menu_text.get(MT_BTN_SHORTCUTS),
                    "Gameplay".to_string(),
                ];
                #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
                labels.push("Game Data Folder".to_string());
                labels.push(resources.menu_text.get(MT_BTN_BACK));
                labels
            }
            OptionsPage::Graphics => {
                let preset = crate::shader_preset::retroarch_presets()
                    .iter()
                    .find(|preset| preset.id == self.graphic.shader_preset)
                    .map(|preset| preset.label.as_str())
                    .unwrap_or("Default");
                vec![
                    format!(
                        "Resolution: {}x{}",
                        self.graphic.resolution_x.round(),
                        self.graphic.resolution_y.round()
                    ),
                    toggle_label("Alpha Vision Field", !self.graphic.framed_view_cone),
                    toggle_label("Transparent Shadows", self.graphic.display_shadow),
                    toggle_label("Effect Animations", self.graphic.display_titbits),
                    toggle_label("Background Animations", self.graphic.display_anim),
                    toggle_label(
                        "Fog/Night All Sprites",
                        self.graphic.apply_fog_to_all_sprites,
                    ),
                    toggle_label("Fullscreen", self.graphic.fullscreen),
                    toggle_label("Hardware Cursor", self.graphic.hardware_cursor),
                    format!("Scaling: {}", self.graphic.scale_mode.label()),
                    format!("Shader Preset: {preset}"),
                    resources.menu_text.get(MT_BTN_OK),
                    resources.menu_text.get(MT_BTN_CANCEL),
                ]
            }
            OptionsPage::Sounds => vec![
                toggle_label("3D Sound", self.sound.sound_3d),
                toggle_label("8-bit Sound", self.sound.sound_8bit),
                format!("FX Volume: {}", self.sound.fx_volume),
                format!("Dialogue Volume: {}", self.sound.dialogue_volume),
                format!("Music Volume: {}", self.sound.music_volume),
                format!("Comment Volume: {}", self.sound.exclamation_volume),
                format!("Comment Frequency: {}", self.sound.amount_of_speaking),
                resources.menu_text.get(MT_BTN_OK),
                resources.menu_text.get(MT_BTN_CANCEL),
            ],
            OptionsPage::Gameplay => {
                let mut labels = crate::ingame_menu::gameplay::OPTION_LABELS
                    .iter()
                    .enumerate()
                    .map(|(index, label)| {
                        if index == 5 {
                            format!(
                                "Campaign Presentation: {}",
                                self.gameplay.campaign_presentation.label()
                            )
                        } else {
                            toggle_label(
                                label,
                                crate::ingame_menu::gameplay::is_option_selected(
                                    &self.gameplay,
                                    index,
                                ),
                            )
                        }
                    })
                    .collect::<Vec<_>>();
                labels.extend([
                    resources.menu_text.get(MT_BTN_OK),
                    resources.menu_text.get(MT_BTN_CANCEL),
                ]);
                labels
            }
            OptionsPage::Shortcuts => {
                let visible = MAX_PAGE_BUTTONS.min(REAL_KEY_COUNT as usize);
                let mut labels = (0..visible)
                    .map(|offset| {
                        let index = self.shortcut_scroll + offset;
                        let action = KEY_ACTIONS.get(index).copied().unwrap_or("Unknown");
                        let key = self.keys.get_key_by_index(index as u16);
                        if self.rebinding == Some(index as u16) {
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
                        }
                    })
                    .collect::<Vec<_>>();
                labels.extend([
                    "Default 1".to_string(),
                    "Default 2".to_string(),
                    "User Defined".to_string(),
                    resources.menu_text.get(MT_BTN_OK),
                    resources.menu_text.get(MT_BTN_CANCEL),
                ]);
                labels
            }
        }
    }

    fn rebuild_frame(&mut self, resources: &IngameMenuResources) {
        self.labels = self.labels(resources);
        self.selected = self.selected.min(self.labels.len().saturating_sub(1));
        let (button_w, button_h) = resources.button_dimensions();
        let row_h = if self.labels.len() > 12 {
            27
        } else {
            button_h.min(34)
        };
        let mut frame = FrameWnd::default();
        frame.enabled = true;
        frame.input_enabled = true;
        for (index, label) in self.labels.iter().enumerate() {
            let enabled = !matches!(
                (self.page, index),
                (OptionsPage::Sounds, 0) if !self.can_3d_sound
            ) && !(self.page == OptionsPage::Gameplay
                && index == crate::ingame_menu::gameplay::SHERWOOD_TRADING_OPTION_INDEX
                && !self.sherwood_trading_editable);
            let (x, y, width, height) = if self.page == OptionsPage::Gameplay {
                let option_count = crate::ingame_menu::gameplay::OPTION_LABELS.len();
                if index < option_count {
                    let rows = option_count.div_ceil(2);
                    (
                        if index < rows { 30 } else { 320 },
                        100 + i32::try_from(index % rows).expect("gameplay option row fits i32")
                            * (row_h + BUTTON_GAP),
                        button_w,
                        row_h,
                    )
                } else {
                    let (button_width, button_height) = resources.button_dimensions();
                    let bottom = crate::ingame_menu::layout::align_bottom_right(
                        &[
                            (&self.labels[option_count], true),
                            (&self.labels[option_count + 1], true),
                        ],
                        button_width,
                        button_height,
                    );
                    let button = &bottom[index - option_count];
                    (button.x, button.y, button.w, button.h)
                }
            } else {
                (
                    BUTTON_X,
                    BUTTON_Y + index as i32 * (row_h + BUTTON_GAP),
                    button_w,
                    row_h,
                )
            };
            frame.add_widget_absolute(widget_bridge::make_button_enabled(
                index as u32,
                label,
                enabled,
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
        let title = match self.page {
            OptionsPage::Hub => resources.menu_text.get(MT_TTL_OPTIONS),
            OptionsPage::Graphics => resources.menu_text.get(MT_TTL_GRAPHICS),
            OptionsPage::Sounds => resources.menu_text.get(MT_TTL_SOUNDS),
            OptionsPage::Shortcuts => resources.menu_text.get(MT_BTN_SHORTCUTS),
            OptionsPage::Gameplay => "Gameplay".to_string(),
        };
        if let Some(font) = resources.title_font_any() {
            render_text_virt_font(renderer, font, self.transform, &title, 20, 20);
        }
        if let Some(font) = resources.label_font_any() {
            let help = match self.page {
                OptionsPage::Hub => "Select a settings page.",
                OptionsPage::Shortcuts => "Click a binding, then press a key. Mouse wheel scrolls.",
                _ => "Click a value or use Left/Right. OK accepts; Cancel restores.",
            };
            render_text_virt_font(renderer, font, self.transform, help, 24, 76);
        }
        widget_bridge::draw_frame_buttons(renderer, resources, self.transform, &self.frame);
        if let Some(cursor) = cursor {
            cursor.draw(renderer, self.transform, &self.input);
        }
    }
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
    multiplayer_connected: bool,
}

impl SaveLoadTaskState {
    pub(super) fn new(
        window: &crate::window::GameWindow,
        renderer: &Renderer,
        resources: &IngameMenuResources,
        save_manager: &mut SaveGameManager,
        mission_id: u32,
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
                            save_manager.remove(slot);
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
                    if (30..450).contains(&vx) && (10..430).contains(&vy) {
                        let row = ((vy - 14) / 34).max(0) as usize + self.scroll;
                        self.select_row(row, save_manager, resources);
                    }
                }
                GameEvent::MouseWheel(delta) => {
                    let total = self.visible.len() + usize::from(self.mode == SaveLoadMode::Save);
                    let max = total.saturating_sub(12);
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
        if next >= self.scroll + 12 {
            self.scroll = next + 1 - 12;
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
            for offset in 0..12 {
                let row = self.scroll + offset;
                if row >= total {
                    break;
                }
                let (selected, label) = if self.mode == SaveLoadMode::Save && row == 0 {
                    (
                        self.selected == Some(SaveRow::New),
                        "< New Save >".to_string(),
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
                        save.text.clone(),
                    )
                };
                let prefix = if selected { "> " } else { "  " };
                render_text_virt_font(
                    renderer,
                    font,
                    self.transform,
                    &format!("{prefix}{label}"),
                    40,
                    42 + offset as i32 * 32,
                );
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

fn cycle_index(current: usize, len: usize, delta: i32) -> usize {
    if len == 0 {
        return 0;
    }
    (current as i32 + delta).rem_euclid(len as i32) as usize
}

fn sound_value_mut(config: &mut SoundConfig, index: usize) -> &mut u16 {
    match index {
        0 => &mut config.fx_volume,
        1 => &mut config.dialogue_volume,
        2 => &mut config.music_volume,
        3 => &mut config.exclamation_volume,
        4 => &mut config.amount_of_speaking,
        _ => panic!("sound value index {index} is out of range"),
    }
}

fn toggle_label(label: &str, selected: bool) -> String {
    format!("{} {label}", if selected { "[x]" } else { "[ ]" })
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
    let previous = config.get_index_for_key(key);
    if previous != 0xFFFF && previous != target {
        config.set_key_by_index(previous, None);
    }
    config.set_key_by_index(target, Some(key));
}

/// Persist direct row edits before leaving the custom binding set for a
/// preset. Merely selecting a preset must not overwrite the player's stored
/// custom bindings.
fn promote_shortcut_edits(active: &KeyConfig, custom: &mut KeyConfig, shortcut_dirty: &mut bool) {
    if *shortcut_dirty {
        *custom = active.clone();
        custom.key_type = 1;
        *shortcut_dirty = false;
    }
}

fn is_reserved_key(key: KeyCode) -> bool {
    matches!(
        key,
        KeyCode::PrintScreen
            | KeyCode::Escape
            | KeyCode::SuperLeft
            | KeyCode::SuperRight
            | KeyCode::ContextMenu
    )
}

fn graphic_eq(left: &GraphicConfig, right: &GraphicConfig) -> bool {
    left.display_anim == right.display_anim
        && left.display_shadow == right.display_shadow
        && left.framed_view_cone == right.framed_view_cone
        && left.display_titbits == right.display_titbits
        && (left.resolution_x - right.resolution_x).abs() < 0.5
        && (left.resolution_y - right.resolution_y).abs() < 0.5
        && left.fullscreen == right.fullscreen
        && left.hardware_cursor == right.hardware_cursor
        && left.scale_mode == right.scale_mode
        && left.shader_preset == right.shader_preset
        && left.apply_fog_to_all_sprites == right.apply_fog_to_all_sprites
}

fn sound_eq(left: &SoundConfig, right: &SoundConfig) -> bool {
    left.music_volume == right.music_volume
        && left.dialogue_volume == right.dialogue_volume
        && left.fx_volume == right.fx_volume
        && left.exclamation_volume == right.exclamation_volume
        && left.amount_of_speaking == right.amount_of_speaking
        && left.sound_3d == right.sound_3d
        && left.sound_8bit == right.sound_8bit
        && (left.master_volume - right.master_volume).abs() < f32::EPSILON
        && left.music_muted == right.music_muted
        && left.fx_muted == right.fx_muted
}

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
