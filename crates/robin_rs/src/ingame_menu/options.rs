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

use super::gameplay::show_gameplay;
use super::graphics::show_graphics;
use super::language::show_language;
use super::layout::{
    MenuTransform, align_bottom_right, draw_screen_background, render_text_virt_font,
};
use super::leaderboard_settings::show_leaderboard_settings;
#[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
use super::multiplayer_privacy::show_multiplayer_privacy;
use super::resources::{
    MT_BTN_BACK, MT_BTN_GRAPHICS, MT_BTN_SHORTCUTS, MT_BTN_SOUNDS, MT_STR_MEGA_BYTES,
    MT_STR_MEGA_HERZS, MT_STR_MEMORY, MT_STR_PROCESSOR, MT_TTL_OPTIONS,
};
use super::shortcuts::show_shortcuts;
use super::sounds::show_sounds;
use super::widget_bridge::{
    self, ModalInputState, ModalScreenIo, ScreenAudio, ScreenFrame, ScreenKey,
};

/// Outcome of the options hub.
#[derive(Debug, Clone, Copy, Default)]
pub struct OptionsOutcome {
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
/// The frame loop stays here instead of `run_modal` because a tick awaits
/// nested sub-screens.
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
        targets.allow_language_switching,
        io,
        controller,
        OptionsOutcome::default(),
        ModalInputState::new(),
    );
    loop {
        while !state.done {
            state
                .tick(
                    application_context,
                    io,
                    targets.sherwood_trading_editable,
                    &mut audio,
                )
                .await;
            // Closing frames were always drawn and paced before committing edits.
            crate::window::sleep_ui_frame().await;
        }
        if state.outcome.language_changed || !state.re_display {
            break;
        }
        // Rebuild only after resolution changes, keeping edits and live input.
        state = OptionsModalState::new(
            application_context,
            targets.allow_language_switching,
            io,
            state.controller,
            state.outcome,
            state.input_state,
        );
    }

    *targets.graphic = state.controller.graphic.working;
    *targets.sound = state.controller.sound.working;
    *targets.gameplay = state.controller.gameplay;
    *targets.multiplayer = state.controller.multiplayer;
    *targets.keys = state.controller.keys;
    *targets.custom_keys = state.controller.custom_keys;
    state.outcome
}

/// Owns one options layout and its staged edits across nested sub-screen awaits.
struct OptionsModalState {
    controller: OptionsController,
    outcome: OptionsOutcome,
    input_state: ModalInputState,
    frame: FrameWnd,
    title: String,
    info: String,
    done: bool,
    re_display: bool,
}

impl OptionsModalState {
    fn new(
        application_context: &crate::host::ApplicationContext,
        allow_language_switching: bool,
        io: &ModalScreenIo<'_, '_>,
        controller: OptionsController,
        outcome: OptionsOutcome,
        mut input_state: ModalInputState,
    ) -> Self {
        let resources = io.resources;
        let transform = MenuTransform::for_renderer(io.renderer);

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
        let re_display = false;
        input_state.seed_mouse_from_window(io.window, transform);

        Self {
            controller,
            outcome,
            input_state,
            frame,
            title,
            info,
            done,
            re_display,
        }
    }

    async fn tick(
        &mut self,
        application_context: &crate::host::ApplicationContext,
        io: &mut ModalScreenIo<'_, '_>,
        sherwood_trading_editable: bool,
        audio: &mut ScreenAudio<'_>,
    ) {
        let screen = ScreenFrame::begin(io, &mut self.input_state);
        for key in screen.keys() {
            match key {
                // Escape → Back.  No Return/KpEnter accelerator
                // since there's no input field.
                ScreenKey::Quit | ScreenKey::Cancel => self.done = true,
                ScreenKey::Confirm | ScreenKey::Next => {}
            }
        }

        let (_, activated) = ScreenFrame::dispatch(&mut self.input_state, &mut self.frame);

        if let Some(id) = activated {
            match id {
                BUTTON_GRAPHICS => {
                    self.controller.enter_page(OptionsPage::Graphics);
                    let (changed, _resolution_changed) = show_graphics(
                        io.window,
                        io.renderer,
                        io.resources,
                        io.cursor,
                        &mut self.controller.graphic.working,
                    )
                    .await;
                    let effects = self.controller.accept_page(changed);
                    self.outcome.changed |= effects.profile_changed;
                    if effects.resolution_changed {
                        // Apply the selected 4:3 scale reference and
                        // aspect policy together. This keeps pointer
                        // conversion aligned while the Options dialog
                        // rebuilds itself; the caller still owns engine,
                        // HUD, and input-cache propagation on return.
                        self.outcome.resolution_changed = true;
                        io.window
                            .set_logical_resolution_policy(&self.controller.graphic.working);
                        io.renderer.sync_window_size(io.window);
                        self.re_display = true;
                        self.done = true;
                    }
                }
                BUTTON_SOUNDS => {
                    self.controller.enter_page(OptionsPage::Sounds);
                    let changed =
                        show_sounds(io, &mut self.controller.sound.working, audio.reborrow()).await;
                    let effects = self.controller.accept_page(changed);
                    self.outcome.changed |= effects.profile_changed;
                    // When the sub-screen accepts edits, push the
                    // new settings through `apply_sound_settings`
                    // so slider/toggle changes take effect
                    // immediately rather than at the next mission
                    // load. The Rust port lacks a kira device
                    // close/open round-trip but still updates
                    // `use_3d_sound`, invalidates the sample cache,
                    // and re-activates source pendings when the 3D
                    // toggle changed.
                    if changed
                        && let ScreenAudio {
                            sound: Some(s),
                            backend,
                            ..
                        } = audio.reborrow()
                    {
                        if let Some(b) = backend {
                            s.apply_sound_settings(false, b, &self.controller.sound.working, None);
                        } else {
                            s.apply_volumes(&self.controller.sound.working);
                        }
                    }
                }
                BUTTON_SHORTCUTS => {
                    self.controller.enter_page(OptionsPage::Shortcuts);
                    // TODO: pass `io` / `ScreenAudio` once the shortcuts
                    // screen is migrated to the ScreenFrame API.
                    let shortcut_audio = audio.reborrow();
                    let accepted = show_shortcuts(
                        io.window,
                        io.renderer,
                        io.resources,
                        io.cursor,
                        &mut self.controller.keys,
                        &mut self.controller.custom_keys,
                        shortcut_audio.sound,
                        shortcut_audio.backend,
                        shortcut_audio.sample_loader,
                    )
                    .await;
                    // Shortcut edits do not propagate to the outer
                    // changed flag. Only persist the dedicated
                    // `KeyConfigStore` path here so editing only
                    // shortcuts does not spuriously mark the
                    // graphic/sound profile dirty.
                    if accepted {
                        self.outcome.key_config_changed |=
                            self.controller.accept_page(false).keys_changed;
                    } else {
                        self.controller.cancel_page();
                    }
                }
                BUTTON_GAMEPLAY => {
                    self.controller.enter_page(OptionsPage::Gameplay);
                    let changed = show_gameplay(
                        application_context,
                        io.window,
                        io.renderer,
                        io.resources,
                        io.cursor,
                        &mut self.controller.gameplay,
                        sherwood_trading_editable,
                    )
                    .await;
                    self.outcome.changed |= self.controller.accept_page(changed).profile_changed;
                }
                BUTTON_LEADERBOARDS => match crate::leaderboard_preferences::load() {
                    Ok(mut preferences) => {
                        if show_leaderboard_settings(io, &mut preferences).await
                            && let Err(error) =
                                crate::leaderboard_preferences::persist(&preferences)
                        {
                            tracing::error!("failed to persist leaderboard settings: {error}");
                        }
                    }
                    Err(error) => {
                        tracing::error!("failed to load leaderboard settings: {error}");
                    }
                },
                #[cfg(all(not(target_arch = "wasm32"), feature = "multiplayer"))]
                BUTTON_MULTIPLAYER_PRIVACY => {
                    self.controller.enter_page(OptionsPage::MultiplayerPrivacy);
                    let changed =
                        show_multiplayer_privacy(io, &mut self.controller.multiplayer).await;
                    self.outcome.changed |= self.controller.accept_page(changed).profile_changed;
                }
                BUTTON_LANGUAGE => {
                    if show_language(application_context, io).await {
                        self.outcome.language_changed = true;
                        self.outcome.changed = true;
                        self.done = true;
                    }
                }
                #[cfg(all(
                    feature = "dialogs",
                    any(target_os = "windows", target_os = "linux", target_os = "macos")
                ))]
                BUTTON_GAME_DATA => {
                    // Opens the native folder picker; the modal loop is
                    // frozen while the OS dialog is up, which is fine —
                    // both are modal. The new folder is remembered and
                    // applies on the next launch (resources from the
                    // old datadir are already loaded).
                    crate::datadir_locator::change_datadir_interactive();
                }
                BUTTON_BACK => self.done = true,
                _ => {}
            }
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
