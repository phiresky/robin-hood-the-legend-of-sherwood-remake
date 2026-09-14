//! Options hub screen.
//!
//! A 640x480 window using `RHID_MENU_BACKGROUND_2` as its background,
//! with a title at `(0,0,500,480)`, a hardware info label at
//! `(0,100,500,480)` and its category buttons aligned bottom-right with
//! spacing 2. Escape maps to Back.
//!
//! Buttons are driven by the [`crate::widget`] system via the
//! [`super::widget_bridge`].

use crate::application::require;

use crate::hardware::Hardware;
use crate::key_config::KeyConfig;
use crate::options_model::{OptionsController, OptionsPage};
use crate::widget::FrameWnd;
use robin_engine::gameplay_config::GameplayConfig;
use robin_engine::graphic_config::GraphicConfig;
use robin_engine::multiplayer_config::MultiplayerConfig;
use robin_engine::sound_config::SoundConfig;

use super::layout::{
    MenuTransform, align_bottom_right, draw_screen_background, render_text_virt_font,
};
use super::resources::{
    MT_BTN_BACK, MT_BTN_GRAPHICS, MT_BTN_SHORTCUTS, MT_BTN_SOUNDS, MT_STR_MEGA_BYTES,
    MT_STR_MEGA_HERZS, MT_STR_MEMORY, MT_STR_PROCESSOR, MT_TTL_OPTIONS,
};
use super::widget_bridge::{
    self, ModalInputState, ModalScreenIo, ScreenAudio, ScreenFrame, ScreenKey,
};

/// Outcome of the options hub.
#[derive(Debug, Clone, Copy, Default)]
pub struct OptionsOutcome {
    pub exit_requested: bool,
    pub changed: bool,
    pub resolution_changed: bool,
    /// Set when the keyboard-shortcuts sub-screen accepted edits.
    /// Callers must react by reloading the input translator's bindings
    /// and refreshing derived UI-shortcut state.
    pub key_config_changed: bool,
    /// The locale lookup generation changed. Callers must discard every
    /// eager localized presentation cache before drawing another menu frame.
    pub language_changed: bool,
}

const SCREEN: &str = "Options screen";
const BUTTON_GRAPHICS: u32 = 0;
const BUTTON_SOUNDS: u32 = 1;
const BUTTON_SHORTCUTS: u32 = 2;
const BUTTON_GAMEPLAY: u32 = 3;
#[cfg(all(not(target_arch = "wasm32"), feature = "multiplayer"))]
const BUTTON_MULTIPLAYER_PRIVACY: u32 = 4;
/// Desktop only: re-select the game data folder (see
/// [`crate::datadir_locator`]).
#[cfg(all(
    feature = "dialogs",
    any(target_os = "windows", target_os = "linux", target_os = "macos")
))]
const BUTTON_GAME_DATA: u32 = 5;
const BUTTON_BACK: u32 = 6;
const BUTTON_LANGUAGE: u32 = 7;
const BUTTON_LEADERBOARDS: u32 = 8;

fn language_option_visible(allow_language_switching: bool, selector_visible: bool) -> bool {
    allow_language_switching && selector_visible
}

/// Live configuration slots and scope flags edited by the options hub.
///
/// Borrowed for one modal run, so it is deliberately not serializable; the
/// staged edits live in [`OptionsController`] until the hub closes.
pub struct OptionsTargets<'a> {
    /// Offer the language selector (main menu only).
    pub allow_language_switching: bool,
    /// Whether the gameplay page may edit Sherwood trading.
    pub sherwood_trading_editable: bool,
    pub graphic: &'a mut GraphicConfig,
    pub gameplay: &'a mut GameplayConfig,
    pub multiplayer: &'a mut MultiplayerConfig,
    pub sound: &'a mut SoundConfig,
    pub keys: &'a mut KeyConfig,
    pub custom_keys: &'a mut KeyConfig,
}

/// Display the in-game options hub.
///
/// `audio` is threaded into the Sounds sub-screen so volume-slider
/// interactions play the `RHWIDGETNOISY_SLIDER` tick sounds.  After the
/// Sounds sub-screen returns with changes, `sound.apply_volumes` runs.
/// Pass `None` services from contexts with no live audio.
///
/// This adapter owns frame pacing; missions drive the same presenter from
/// their cooperative UI task.
pub async fn show_options(
    application_context: &crate::host::ApplicationContext,
    io: &mut ModalScreenIo<'_, '_>,
    targets: OptionsTargets<'_>,
    mut audio: ScreenAudio<'_>,
) -> OptionsOutcome {
    let controller = OptionsController::new(
        targets.graphic.clone(),
        *targets.sound,
        *targets.gameplay,
        *targets.multiplayer,
        targets.keys.clone(),
        targets.custom_keys.clone(),
    );

    let mut state = OptionsModalState::new(
        application_context,
        io.window,
        io.renderer,
        io.resources,
        controller,
        OptionsScope {
            allow_language_switching: targets.allow_language_switching,
            sherwood_trading_editable: targets.sherwood_trading_editable,
            host_gameplay_rules_editable: true,
            apply_live_preferences: true,
        },
    );
    while state.tick(application_context, io, &mut audio).is_none() {
        crate::window::sleep_ui_frame().await;
    }

    *targets.graphic = state.controller.graphic.working;
    *targets.sound = state.controller.sound.working;
    *targets.gameplay = state.controller.gameplay;
    *targets.multiplayer = state.controller.multiplayer;
    *targets.keys = state.controller.keys;
    *targets.custom_keys = state.controller.custom_keys;
    state.outcome
}

/// The owner decides which rules and locale may be changed; presentation is shared.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OptionsScope {
    pub(crate) allow_language_switching: bool,
    pub(crate) sherwood_trading_editable: bool,
    pub(crate) host_gameplay_rules_editable: bool,
    /// Main-menu page acceptance applies presentation immediately. Missions
    /// commit through their owner so task preemption discards every staged edit.
    pub(crate) apply_live_preferences: bool,
}

enum OptionsChild {
    Graphics(super::graphics::GraphicsScreen),
    Sounds(super::sounds::SoundsScreen),
    Shortcuts(super::shortcuts::ShortcutsScreen),
    Gameplay(super::gameplay::GameplayScreenState),
    Language(super::language::LanguageModalState),
    Leaderboards(
        super::leaderboard_settings::LeaderboardSettingsModalState,
        crate::leaderboard_preferences::LeaderboardPreferences,
    ),
    #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
    Multiplayer(super::multiplayer_privacy::MultiplayerPrivacyModalState),
}

/// One resumable presenter for the main menu and cooperative mission driver.
pub(crate) struct OptionsModalState {
    pub(crate) controller: OptionsController,
    pub(crate) outcome: OptionsOutcome,
    scope: OptionsScope,
    input_state: ModalInputState,
    frame: FrameWnd,
    title: String,
    info: String,
    done: bool,
    child: Option<OptionsChild>,
    content: Option<super::spellforge_content::SpellforgeContentSettingsState>,
}

impl OptionsModalState {
    pub(crate) fn new(
        application_context: &crate::host::ApplicationContext,
        window: &crate::window::GameWindow,
        renderer: &crate::renderer::Renderer,
        resources: &super::resources::IngameMenuResources,
        controller: OptionsController,
        scope: OptionsScope,
    ) -> Self {
        let allow_language_switching = scope.allow_language_switching;
        let mut input_state = ModalInputState::new();
        let transform = MenuTransform::for_renderer(renderer);

        let (btn_w, btn_h) = resources.button_dimensions();

        let graphics_label = resources.menu_text.get(MT_BTN_GRAPHICS);
        let sounds_label = resources.menu_text.get(MT_BTN_SOUNDS);
        let shortcuts_label = resources.menu_text.get(MT_BTN_SHORTCUTS);
        let back_label = resources.menu_text.get(MT_BTN_BACK);
        #[allow(unused_mut)]
        let mut entries: Vec<(u32, &str)> = vec![
            (BUTTON_GRAPHICS, &graphics_label),
            (BUTTON_SOUNDS, &sounds_label),
            (BUTTON_SHORTCUTS, &shortcuts_label),
            (BUTTON_GAMEPLAY, "Gameplay"),
            (BUTTON_LEADERBOARDS, "Leaderboards"),
        ];
        #[cfg(all(not(target_arch = "wasm32"), feature = "multiplayer"))]
        entries.push((BUTTON_MULTIPLAYER_PRIVACY, "Multiplayer / Privacy"));
        let language_label = require(
            application_context.port_text(crate::localization::PortTextKey::Language),
            SCREEN,
        );
        let selector_visible = allow_language_switching
            && require(application_context.language_selector_visible(), SCREEN);
        if language_option_visible(allow_language_switching, selector_visible) {
            entries.push((BUTTON_LANGUAGE, language_label));
        }
        #[cfg(all(
            feature = "dialogs",
            any(target_os = "windows", target_os = "linux", target_os = "macos")
        ))]
        entries.push((BUTTON_GAME_DATA, "Game Data Folder"));
        entries.push((BUTTON_BACK, &back_label));
        let labels: Vec<(&str, bool)> = entries.iter().map(|&(_, label)| (label, true)).collect();
        let menu_buttons = align_bottom_right(&labels, btn_w, btn_h);

        let mut frame = FrameWnd::interactive();
        for (i, mb) in menu_buttons.iter().enumerate() {
            frame.add_widget_absolute(widget_bridge::make_button(
                entries[i].0,
                &mb.label,
                mb.x,
                mb.y,
                mb.w,
                mb.h,
            ));
        }

        let title = resources.menu_text.get(MT_TTL_OPTIONS);
        let info = hardware_description(&resources.menu_text);

        let done = false;
        input_state.seed_mouse_from_window(window, transform);

        Self {
            controller,
            outcome: OptionsOutcome::default(),
            scope,
            input_state,
            frame,
            title,
            info,
            done,
            child: None,
            content: None,
        }
    }

    pub(crate) fn tick(
        &mut self,
        application_context: &crate::host::ApplicationContext,
        io: &mut ModalScreenIo<'_, '_>,
        audio: &mut ScreenAudio<'_>,
    ) -> Option<OptionsOutcome> {
        if self.done {
            return Some(self.outcome);
        }
        if let Some(content) = self.content.as_mut() {
            match content.tick(application_context, io) {
                super::spellforge_content::SpellforgeContentSettingsOutcome::Pending => {}
                super::spellforge_content::SpellforgeContentSettingsOutcome::Closed => {
                    self.content = None;
                    if let Some(OptionsChild::Gameplay(page)) = self.child.as_mut() {
                        page.resume_after_content(io);
                    }
                }
                super::spellforge_content::SpellforgeContentSettingsOutcome::ExitRequested => {
                    self.outcome.exit_requested = true;
                    self.done = true;
                }
            }
            return self.done.then_some(self.outcome);
        }
        if let Some(child) = self.child.take() {
            self.tick_child(child, application_context, io, audio);
            return self.done.then_some(self.outcome);
        }
        let screen = ScreenFrame::begin(io, &mut self.input_state);
        for key in screen.keys() {
            match key {
                // Escape → Back.  No Return/KpEnter accelerator
                // since there's no input field.
                ScreenKey::Quit => {
                    self.outcome.exit_requested = true;
                    self.done = true;
                }
                ScreenKey::Cancel => self.done = true,
                ScreenKey::Confirm | ScreenKey::Next => {}
            }
        }

        let (_, activated) = ScreenFrame::dispatch(&mut self.input_state, &mut self.frame);

        if let Some(id) = activated {
            if id == BUTTON_SHORTCUTS {
                self.controller.enter_page(OptionsPage::Shortcuts);
            }
            self.child = match id {
                BUTTON_GRAPHICS => Some(OptionsChild::Graphics(
                    super::graphics::GraphicsScreen::new(
                        io.resources,
                        &self.controller.graphic.working,
                        ModalInputState::for_screen(io.window, io.renderer),
                    ),
                )),
                BUTTON_SOUNDS => Some(OptionsChild::Sounds(
                    super::sounds::SoundsScreen::new(
                        io,
                        &self.controller.sound.working,
                        audio.sound.as_deref(),
                    )
                    .with_host_authority(self.scope.host_gameplay_rules_editable),
                )),
                BUTTON_SHORTCUTS => Some(OptionsChild::Shortcuts(
                    super::shortcuts::ShortcutsScreen::new(
                        io.resources,
                        &self.controller.keys,
                        ModalInputState::for_screen(io.window, io.renderer),
                    ),
                )),
                BUTTON_GAMEPLAY => Some(OptionsChild::Gameplay(
                    super::gameplay::GameplayScreenState::new(
                        application_context,
                        io,
                        &self.controller.gameplay,
                        self.scope.sherwood_trading_editable,
                    )
                    .with_host_authority(
                        self.scope.host_gameplay_rules_editable,
                        application_context,
                        io.resources,
                    ),
                )),
                BUTTON_LEADERBOARDS => match crate::leaderboard_preferences::load() {
                    Ok(preferences) => Some(OptionsChild::Leaderboards(
                        super::leaderboard_settings::LeaderboardSettingsModalState::new(
                            io,
                            &preferences,
                        ),
                        preferences,
                    )),
                    Err(error) => {
                        tracing::error!("failed to load leaderboard settings: {error}");
                        None
                    }
                },
                #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
                BUTTON_MULTIPLAYER_PRIVACY => Some(OptionsChild::Multiplayer(
                    super::multiplayer_privacy::MultiplayerPrivacyModalState::new(
                        io,
                        &self.controller.multiplayer,
                    ),
                )),
                BUTTON_LANGUAGE if self.scope.allow_language_switching => {
                    super::language::LanguageModalState::new(application_context, io)
                        .map(OptionsChild::Language)
                }
                #[cfg(all(
                    feature = "dialogs",
                    any(target_os = "windows", target_os = "linux", target_os = "macos")
                ))]
                BUTTON_GAME_DATA => {
                    crate::datadir_locator::change_datadir_interactive();
                    None
                }
                BUTTON_BACK => {
                    self.done = true;
                    None
                }
                _ => None,
            };
        }

        let transform = screen.transform;
        let renderer = &mut *io.renderer;
        let resources = io.resources;
        screen.begin_draw(renderer);

        if let Some(bg) = resources.menu_bg[2] {
            draw_screen_background(renderer, &bg);
        }

        if let Some(font) = resources.title_font_any() {
            render_text_virt_font(renderer, font, transform, &self.title, 20, 20);
        }
        if let Some(font) = resources.label_font_any() {
            let mut y = 120;
            for line in self.info.lines() {
                render_text_virt_font(renderer, font, transform, line, 40, y);
                y += font.height() as i32 + 4;
            }
        }

        widget_bridge::draw_frame_buttons(renderer, resources, transform, &self.frame);

        screen.finish(io, &self.input_state);
        self.done.then_some(self.outcome)
    }

    fn tick_child(
        &mut self,
        child: OptionsChild,
        application_context: &crate::host::ApplicationContext,
        io: &mut ModalScreenIo<'_, '_>,
        audio: &mut ScreenAudio<'_>,
    ) {
        match child {
            OptionsChild::Graphics(mut page) => {
                page.tick(io);
                if !page.done {
                    self.child = Some(OptionsChild::Graphics(page));
                    return;
                }
                self.outcome.exit_requested |= page.exit_requested;
                self.done |= page.exit_requested;
                let (changed, resolution_changed) =
                    page.finish(&mut self.controller.graphic.working);
                self.outcome.changed |= changed;
                self.outcome.resolution_changed |= resolution_changed;
                if changed && self.scope.apply_live_preferences {
                    io.renderer
                        .apply_upscale_config(&self.controller.graphic.working);
                }
                if resolution_changed && self.scope.apply_live_preferences {
                    io.window
                        .set_logical_resolution_policy(&self.controller.graphic.working);
                    io.renderer.sync_window_size(io.window);
                }
            }
            OptionsChild::Sounds(mut page) => {
                if page.tick(io, audio).is_none() {
                    self.child = Some(OptionsChild::Sounds(page));
                    return;
                }
                self.outcome.exit_requested |= page.exit_requested;
                self.done |= page.exit_requested;
                let changed = page.finish(&mut self.controller.sound.working);
                self.outcome.changed |= changed;
                if changed && self.scope.apply_live_preferences {
                    let ScreenAudio { sound, backend, .. } = audio.reborrow();
                    if let Some(sound) = sound {
                        if let Some(backend) = backend {
                            sound.apply_sound_settings(
                                false,
                                backend,
                                &self.controller.sound.working,
                                None,
                            );
                        } else {
                            sound.apply_volumes(&self.controller.sound.working);
                        }
                    }
                }
            }
            OptionsChild::Shortcuts(mut page) => {
                page.tick(io, &mut self.controller.custom_keys, audio);
                if !page.done {
                    self.child = Some(OptionsChild::Shortcuts(page));
                    return;
                }
                self.outcome.exit_requested |= page.exit_requested;
                self.done |= page.exit_requested;
                let accepted =
                    page.finish(&mut self.controller.keys, &mut self.controller.custom_keys);
                if accepted {
                    self.outcome.key_config_changed |=
                        self.controller.accept_page(false).keys_changed;
                } else {
                    self.controller.cancel_page();
                }
            }
            OptionsChild::Gameplay(mut page) => {
                let outcome = page.tick(application_context, io);
                if page.take_content_request() {
                    self.content = Some(
                        super::spellforge_content::SpellforgeContentSettingsState::new(
                            application_context,
                            io.window,
                            io.renderer,
                            io.resources,
                        ),
                    );
                    self.child = Some(OptionsChild::Gameplay(page));
                    return;
                }
                match outcome {
                    None => {
                        self.child = Some(OptionsChild::Gameplay(page));
                        return;
                    }
                    Some(super::ModalScreenOutcome::Accepted(config)) => {
                        self.outcome.changed |= config != self.controller.gameplay;
                        self.controller.gameplay = config;
                    }
                    Some(super::ModalScreenOutcome::ExitRequested) => {
                        self.outcome.exit_requested = true;
                        self.done = true;
                    }
                    Some(super::ModalScreenOutcome::Cancelled) => {}
                }
            }
            OptionsChild::Language(mut page) => match page.tick(application_context, io) {
                None => {
                    self.child = Some(OptionsChild::Language(page));
                    return;
                }
                Some(changed) => {
                    self.outcome.changed |= changed;
                    self.outcome.language_changed |= changed;
                    self.done |= changed;
                }
            },
            OptionsChild::Leaderboards(mut page, mut preferences) => {
                if page.tick(io).is_none() {
                    self.child = Some(OptionsChild::Leaderboards(page, preferences));
                    return;
                }
                self.outcome.exit_requested |= page.exit_requested;
                self.done |= page.exit_requested;
                if page.commit(&mut preferences)
                    && let Err(error) = crate::leaderboard_preferences::persist(&preferences)
                {
                    tracing::error!("failed to persist leaderboard settings: {error}");
                }
            }
            #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
            OptionsChild::Multiplayer(mut page) => {
                if page.tick(io).is_none() {
                    self.child = Some(OptionsChild::Multiplayer(page));
                    return;
                }
                self.outcome.exit_requested |= page.exit_requested;
                self.done |= page.exit_requested;
                self.outcome.changed |= page.commit(&mut self.controller.multiplayer);
            }
        }
        self.input_state
            .seed_mouse_from_window(io.window, MenuTransform::for_renderer(io.renderer));
    }
}

/// Build the hardware description line shown on the options hub.
fn hardware_description(text: &super::resources::MenuText) -> String {
    let processor = text.get(MT_STR_PROCESSOR);
    let memory = text.get(MT_STR_MEMORY);
    let mhz = text.get(MT_STR_MEGA_HERZS);
    let mb = text.get(MT_STR_MEGA_BYTES);
    let hw = Hardware::detect();
    let ident = hw.processor_identifier().to_string_lossy();
    // Speed and memory can be unknown (e.g. wasm); omit those parts
    // instead of showing an invented number.
    let mut description = format!("{processor} : {ident}");
    if let Some(speed) = hw.processor_speed() {
        description.push_str(&format!(", {speed} {mhz}"));
    }
    if let Some(memory_mb) = hw.physical_memory_mb() {
        description.push_str(&format!("\n{memory} : {memory_mb} {mb}"));
    }
    description
}

#[cfg(test)]
mod tests {
    use super::language_option_visible;

    #[test]
    fn language_option_respects_main_menu_only_scope() {
        assert!(language_option_visible(true, true));
        assert!(!language_option_visible(true, false));
        assert!(!language_option_visible(false, true));
        assert!(!language_option_visible(false, false));
    }
}
