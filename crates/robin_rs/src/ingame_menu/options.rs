//! Options hub screen.
//!
//! A 640x480 window using `RHID_MENU_BACKGROUND_2` as its background,
//! with a title at `(0,0,500,480)`, a hardware info label at
//! `(0,100,500,480)` and the buttons Graphics / Sounds / Shortcuts /
//! Gameplay / Back aligned bottom-right with spacing 2.  Escape maps to
//! Back.
//!
//! Buttons are driven by the [`crate::widget`] system via the
//! [`super::widget_bridge`].

use crate::gfx_types::Keycode;
use robin_engine::sound_cache::SampleLoader;

use crate::gfx_types::GameEvent;
use crate::hardware::Hardware;
use crate::key_config::KeyConfig;
use crate::renderer::Renderer;
use crate::sound::{AudioBackend, SoundManager};
use crate::widget::FrameWnd;
use robin_engine::gameplay_config::GameplayConfig;
use robin_engine::graphic_config::GraphicConfig;
use robin_engine::sound_config::SoundConfig;

use super::gameplay::show_gameplay;
use super::graphics::show_graphics;
use super::layout::{
    MenuTransform, align_bottom_right, dim_screen, draw_screen_background, enter_modal_gpu_phase,
    render_text_virt,
};
use super::resources::{
    IngameMenuResources, MT_BTN_BACK, MT_BTN_GRAPHICS, MT_BTN_SHORTCUTS, MT_BTN_SOUNDS,
    MT_STR_MEGA_BYTES, MT_STR_MEGA_HERZS, MT_STR_MEMORY, MT_STR_PROCESSOR, MT_TTL_OPTIONS,
};
use super::shortcuts::show_shortcuts;
use super::sounds::show_sounds;
use super::widget_bridge::{self, ModalCursor, ModalInputState};

/// Outcome of the options hub.
#[derive(Debug, Clone, Copy, Default)]
pub struct OptionsOutcome {
    pub changed: bool,
    pub resolution_changed: bool,
    /// Set when the keyboard-shortcuts sub-screen accepted edits.
    /// Callers must react by reloading the input translator's bindings
    /// and refreshing derived UI-shortcut state.
    pub key_config_changed: bool,
}

const BUTTON_GRAPHICS: u32 = 0;
const BUTTON_SOUNDS: u32 = 1;
const BUTTON_SHORTCUTS: u32 = 2;
const BUTTON_GAMEPLAY: u32 = 3;
/// Desktop only: re-select the game data folder (see
/// [`crate::datadir_locator`]).
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
const BUTTON_GAME_DATA: u32 = 4;
const BUTTON_BACK: u32 = 5;

/// Display the in-game options hub.
///
/// `sound` / `audio_backend` / `sample_loader` are threaded into the
/// Sounds sub-screen so volume-slider interactions play the
/// `RHWIDGETNOISY_SLIDER` tick sounds.  After the Sounds sub-screen
/// returns with changes, `sound.apply_volumes` runs.  Pass `None` for
/// audio args from contexts with no live audio (e.g. the main-menu
/// entry path).
#[allow(clippy::too_many_arguments)]
pub async fn show_options(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    mut cursor: Option<ModalCursor<'_>>,
    graphic_config: &mut GraphicConfig,
    gameplay_config: &mut GameplayConfig,
    sound_config: &mut SoundConfig,
    key_config: &mut KeyConfig,
    custom_key_config: &mut KeyConfig,
    sherwood_trading_editable: bool,
    mut sound: Option<&mut SoundManager>,
    mut audio_backend: Option<&mut dyn AudioBackend>,
    sample_loader: Option<&SampleLoader>,
) -> OptionsOutcome {
    let mut outcome = OptionsOutcome::default();
    let mut input_state = ModalInputState::new();

    // Outer re-display loop: on a resolution change the menu destroys
    // itself and the outer loop re-enters at the new screen size. Here
    // we re-layout everything against the fresh
    // `renderer.screen_width()`/`screen_height()` on every iteration.
    loop {
        let sw = renderer.screen_width() as i32;
        let sh = renderer.screen_height() as i32;
        let transform = MenuTransform::centered(sw, sh);

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
        ];
        #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
        entries.push((BUTTON_GAME_DATA, "Game Data Folder"));
        entries.push((BUTTON_BACK, &back_label));
        let labels: Vec<(&str, bool)> = entries.iter().map(|&(_, label)| (label, true)).collect();
        let menu_buttons = align_bottom_right(&labels, btn_w, btn_h);

        let mut frame = FrameWnd::default();
        frame.enabled = true;
        frame.input_enabled = true;
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

        let mut done = false;
        let mut re_display = false;
        input_state.seed_mouse_from_window(event_pump, transform);

        while !done {
            let (events, transform) =
                super::layout::poll_events_with_transform(event_pump, renderer);
            for event in events {
                input_state.update_from_event(&event, transform);
                match event {
                    GameEvent::Quit => done = true,
                    // Escape → Back.  No Return/KpEnter accelerator
                    // since there's no input field.
                    GameEvent::KeyDown {
                        keycode: Keycode::Escape,
                        ..
                    } => {
                        done = true;
                    }
                    _ => {}
                }
            }

            let widget_input = input_state.as_widget_input();
            let events = frame.process_input(&widget_input);
            input_state.end_frame();

            if let Some(id) = widget_bridge::find_activated(&events) {
                match id {
                    BUTTON_GRAPHICS => {
                        let (changed, resolution_changed) = show_graphics(
                            event_pump,
                            renderer,
                            resources,
                            cursor.as_mut().map(|c| c.reborrow()),
                            graphic_config,
                        )
                        .await;
                        outcome.changed |= changed;
                        if resolution_changed {
                            // Apply the selected 4:3 scale reference and
                            // aspect policy together. This keeps pointer
                            // conversion aligned while the Options dialog
                            // rebuilds itself; the caller still owns engine,
                            // HUD, and input-cache propagation on return.
                            outcome.resolution_changed = true;
                            event_pump.set_logical_resolution_policy(graphic_config);
                            renderer.sync_window_size(event_pump);
                            re_display = true;
                            done = true;
                        }
                    }
                    BUTTON_SOUNDS => {
                        // Explicit reborrow: `Option<&mut dyn Trait>::as_deref_mut` infers
                        // the returned reference's lifetime against the outer `&mut dyn`,
                        // which the borrow checker won't accept across loop iterations.
                        // `as_mut().map(|b| &mut **b as &mut dyn _)` re-expresses the
                        // reborrow with the local `&mut` as the source lifetime, which
                        // NLL happily shortens.
                        let backend_reborrow: Option<&mut dyn AudioBackend> = audio_backend
                            .as_mut()
                            .map(|b| &mut **b as &mut dyn AudioBackend);
                        let changed = show_sounds(
                            event_pump,
                            renderer,
                            resources,
                            cursor.as_mut().map(|c| c.reborrow()),
                            sound_config,
                            sound.as_deref_mut(),
                            backend_reborrow,
                            sample_loader,
                        )
                        .await;
                        outcome.changed |= changed;
                        // When the sub-screen accepts edits, push the
                        // new settings through `apply_sound_settings`
                        // so slider/toggle changes take effect
                        // immediately rather than at the next mission
                        // load. The Rust port lacks a kira device
                        // close/open round-trip but still updates
                        // `use_3d_sound`, invalidates the sample cache,
                        // and re-activates source pendings when the 3D
                        // toggle changed.
                        if changed && let Some(s) = sound.as_deref_mut() {
                            let backend_for_apply: Option<&mut dyn AudioBackend> = audio_backend
                                .as_mut()
                                .map(|b| &mut **b as &mut dyn AudioBackend);
                            if let Some(b) = backend_for_apply {
                                s.apply_sound_settings(false, b, sound_config, None);
                            } else {
                                s.apply_volumes(sound_config);
                            }
                        }
                    }
                    BUTTON_SHORTCUTS => {
                        let backend_reborrow: Option<&mut dyn AudioBackend> = audio_backend
                            .as_mut()
                            .map(|b| &mut **b as &mut dyn AudioBackend);
                        let changed = show_shortcuts(
                            event_pump,
                            renderer,
                            resources,
                            cursor.as_mut().map(|c| c.reborrow()),
                            key_config,
                            custom_key_config,
                            sound.as_deref_mut(),
                            backend_reborrow,
                            sample_loader,
                        )
                        .await;
                        // Shortcut edits do not propagate to the outer
                        // changed flag. Only persist the dedicated
                        // `KeyConfigStore` path here so editing only
                        // shortcuts does not spuriously mark the
                        // graphic/sound profile dirty.
                        outcome.key_config_changed |= changed;
                    }
                    BUTTON_GAMEPLAY => {
                        let changed = show_gameplay(
                            event_pump,
                            renderer,
                            resources,
                            cursor.as_mut().map(|c| c.reborrow()),
                            gameplay_config,
                            sherwood_trading_editable,
                        )
                        .await;
                        outcome.changed |= changed;
                    }
                    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
                    BUTTON_GAME_DATA => {
                        // Opens the native folder picker; the modal loop is
                        // frozen while the OS dialog is up, which is fine —
                        // both are modal. The new folder is remembered and
                        // applies on the next launch (resources from the
                        // old datadir are already loaded).
                        crate::datadir_locator::change_datadir_interactive();
                    }
                    BUTTON_BACK => done = true,
                    _ => {}
                }
            }

            enter_modal_gpu_phase(renderer);
            dim_screen(renderer);

            if let Some(bg) = resources.menu_bg[2] {
                draw_screen_background(renderer, &bg);
            }

            if let Some(font) = resources.title_font() {
                render_text_virt(renderer, font, transform, &title, 20, 20);
            }
            if let Some(font) = resources.label_font() {
                let mut y = 120;
                for line in info.lines() {
                    render_text_virt(renderer, font, transform, line, 40, y);
                    y += font.height() as i32 + 4;
                }
            }

            widget_bridge::draw_frame_buttons(renderer, resources, transform, &frame);

            if let Some(c) = &cursor {
                c.draw(renderer, transform, &input_state);
            }

            renderer.present();
            crate::window::sleep_ms(16).await;
        }

        if !re_display {
            break;
        }
    }

    outcome
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
