//! Recoverable save-store admission. Failed admission never constructs an empty
//! writable store, and Retry only repeats the store's validated open operation.

use serde::{Deserialize, Serialize};

use crate::host::ApplicationContext;
use crate::ingame_menu::layout::{self, MenuRect, MenuTransform};
use crate::ingame_menu::resources::{
    IngameMenuResources, MT_BTN_CANCEL, MT_BTN_OK, MT_BTN_QUIT_GAME,
};
use crate::ingame_menu::widget_bridge::{self, ModalCursor, ModalInputState};
use crate::renderer::Renderer;
use crate::savegame::SaveGameManager;
use crate::window::GameWindow;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("Cannot open save store: {detail}")]
pub struct SaveStoreOpenError {
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryChoice {
    Retry,
    Cancel,
    Exit,
}

/// Runtime ownership is deliberately not serializable.
pub enum OpenedSaveStore {
    Ready(SaveGameManager),
    Cancelled,
    ExitRequested,
}

pub fn try_open(context: &ApplicationContext) -> Result<SaveGameManager, SaveStoreOpenError> {
    SaveGameManager::open_for_context(context).map_err(|detail| SaveStoreOpenError { detail })
}

/// The same controller is exercised by tests with real filesystem admission.
/// Only `Ok(manager)` grants access; cancelling an error cannot manufacture one.
fn retry<T>(open: impl FnOnce() -> Result<T, String>) -> Result<T, SaveStoreOpenError> {
    open().map_err(|detail| SaveStoreOpenError { detail })
}

fn recovery_choice(event: &crate::gfx_types::GameEvent) -> Option<RecoveryChoice> {
    use crate::gfx_types::{GameEvent, Keycode};
    match event {
        GameEvent::Quit => Some(RecoveryChoice::Exit),
        GameEvent::KeyDown {
            keycode: Keycode::Escape,
            ..
        } => Some(RecoveryChoice::Cancel),
        GameEvent::KeyDown {
            keycode: Keycode::Return | Keycode::KpEnter,
            ..
        } => Some(RecoveryChoice::Retry),
        _ => None,
    }
}

/// Small existing-assets dialog. It yields every frame, handles native/browser
/// close events and uses existing localized Cancel/Quit action labels.
async fn choose_recovery(
    context: &ApplicationContext,
    window: &mut GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    cursor: Option<&ModalCursor<'_>>,
    error: &SaveStoreOpenError,
) -> RecoveryChoice {
    let mut input = ModalInputState::new();
    let mut scroll = 0;
    loop {
        context.poll_leaderboard_receipts();
        let (events, transform) = layout::poll_events_with_transform(window, renderer);
        let mut choice = None;
        for event in events {
            input.update_from_event(&event, transform);
            scroll_diagnostic(&mut scroll, &event);
            if choice != Some(RecoveryChoice::Exit) {
                if let Some(action) = recovery_choice(&event) {
                    choice = Some(action);
                }
            }
        }
        let (width, height) = resources.button_dimensions();
        let labels = [
            // No Retry token exists in the original game's menu table.
            // TODO(i18n): add a translated recovery Retry action.
            "Retry".to_string(),
            resources.menu_text.get(MT_BTN_CANCEL),
            resources.menu_text.get(MT_BTN_QUIT_GAME),
        ];
        let mut frame = widget_bridge::make_button_frame(&[
            (0, &labels[0], 70, 360, width, height),
            (1, &labels[1], 260, 360, width, height),
            (2, &labels[2], 450, 360, width, height),
        ]);
        let events = frame.process_input(&input.as_widget_input());
        input.end_frame();
        if choice != Some(RecoveryChoice::Exit)
            && let Some(id) = widget_bridge::find_activated(&events)
        {
            choice = Some(match id {
                0 => RecoveryChoice::Retry,
                1 => RecoveryChoice::Cancel,
                2 => RecoveryChoice::Exit,
                _ => unreachable!("recovery button id"),
            });
        }
        if let Some(choice) = choice {
            if choice == RecoveryChoice::Exit {
                window.close_requested = true;
            }
            return choice;
        }
        layout::enter_modal_gpu_phase(renderer);
        layout::dim_screen(renderer);
        layout::draw_fallback_panel(
            renderer,
            transform,
            &MenuRect {
                x: 35,
                y: 70,
                w: 570,
                h: 340,
            },
        );
        // The engine has no localized Retry/save-recovery sentence yet. Other
        // labels reuse its localized Cancel/Quit entries; diagnostics are
        // intentionally exact backend text, not a guessed repair suggestion.
        // TODO(i18n): give recovery guidance its own translated menu-text entry.
        let message = format!(
            "Saves are unavailable. Repair the reported problem, then select {}. {} leaves this launch without saving. No files will be reset. Scroll with Up/Down or mouse wheel.\n\n{error}",
            labels[0], labels[1]
        );
        draw_diagnostic(renderer, resources, transform, &message, &mut scroll);
        widget_bridge::draw_frame_buttons(renderer, resources, transform, &frame);
        if let Some(cursor) = cursor {
            cursor.draw(renderer, transform, &input);
        }
        renderer.present();
        crate::window::sleep_ui_frame().await;
    }
}

fn scroll_diagnostic(scroll: &mut usize, event: &crate::gfx_types::GameEvent) {
    use crate::gfx_types::{GameEvent, Keycode};
    match event {
        GameEvent::KeyDown {
            keycode: Keycode::Down,
            ..
        } => *scroll = scroll.saturating_add(1),
        GameEvent::KeyDown {
            keycode: Keycode::Up,
            ..
        } => *scroll = scroll.saturating_sub(1),
        GameEvent::MouseWheel(dy) if *dy < 0 => *scroll = scroll.saturating_add(1),
        GameEvent::MouseWheel(dy) if *dy > 0 => *scroll = scroll.saturating_sub(1),
        _ => {}
    }
}

fn draw_diagnostic(
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    transform: MenuTransform,
    message: &str,
    scroll: &mut usize,
) {
    let font = resources
        .menu_text_font_any()
        .expect("save recovery requires menu text font");
    let line_height = i32::from(font.height()).max(1);
    let visible = (250 / line_height).max(1) as usize;
    // This wrapper splits overlong individual path components at character
    // boundaries as well as wrapping normal words. Every diagnostic line can
    // be reached; a long backend error is never silently truncated.
    let wrapped = layout::wrap_text_for_box_font(font, message, 530, usize::MAX);
    *scroll = (*scroll).min(wrapped.lines.len().saturating_sub(visible));
    for (index, line) in wrapped.lines.iter().skip(*scroll).take(visible).enumerate() {
        layout::render_text_virt_font(
            renderer,
            font,
            transform,
            line,
            55,
            90 + index as i32 * line_height,
        );
    }
}

/// Nonblocking acknowledgement used by both picker scheduling adapters.
/// This runtime owner is not deserialized; its diagnostic is plain data.
pub(crate) struct ErrorNotice {
    message: String,
    scroll: usize,
    input: ModalInputState,
}

impl ErrorNotice {
    pub(crate) fn new(message: String) -> Self {
        Self {
            message,
            scroll: 0,
            input: ModalInputState::new(),
        }
    }

    /// `true` means acknowledged; a close request also stays set on the window
    /// so the adapter can propagate it instead of consuming it as an OK click.
    pub(crate) fn tick(
        &mut self,
        window: &mut GameWindow,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
    ) -> bool {
        let (events, transform) = layout::poll_events_with_transform(window, renderer);
        let mut dismissed = false;
        for event in events {
            self.input.update_from_event(&event, transform);
            scroll_diagnostic(&mut self.scroll, &event);
            if let Some(choice) = recovery_choice(&event) {
                if choice == RecoveryChoice::Exit {
                    window.close_requested = true;
                }
                dismissed = true;
            }
        }
        let (width, height) = resources.button_dimensions();
        let label = resources.menu_text.get(MT_BTN_OK);
        let mut frame =
            widget_bridge::make_button_frame(&[(0, &label, (640 - width) / 2, 360, width, height)]);
        let events = frame.process_input(&self.input.as_widget_input());
        self.input.end_frame();
        if widget_bridge::find_activated(&events).is_some() {
            dismissed = true;
        }
        if dismissed {
            return true;
        }
        layout::enter_modal_gpu_phase(renderer);
        layout::dim_screen(renderer);
        layout::draw_fallback_panel(
            renderer,
            transform,
            &MenuRect {
                x: 35,
                y: 70,
                w: 570,
                h: 340,
            },
        );
        draw_diagnostic(
            renderer,
            resources,
            transform,
            &self.message,
            &mut self.scroll,
        );
        widget_bridge::draw_frame_buttons(renderer, resources, transform, &frame);
        if let Some(cursor) = cursor {
            cursor.draw(renderer, transform, &self.input);
        }
        renderer.present();
        false
    }
}

pub async fn open_with_recovery(
    context: &ApplicationContext,
    window: &mut GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    cursor: Option<&ModalCursor<'_>>,
) -> OpenedSaveStore {
    loop {
        match retry(|| SaveGameManager::open_for_context(context)) {
            Ok(store) => return OpenedSaveStore::Ready(store),
            Err(error) => {
                tracing::error!("{error}");
                match choose_recovery(context, window, renderer, resources, cursor, &error).await {
                    RecoveryChoice::Retry => {}
                    RecoveryChoice::Cancel => return OpenedSaveStore::Cancelled,
                    RecoveryChoice::Exit => return OpenedSaveStore::ExitRequested,
                }
            }
        }
    }
}

/// Direct graphical launches have no menu renderer yet. Allocate UI resources
/// only on admission failure; headless callers use `try_open` and return errors.
pub async fn open_for_launch(
    context: &ApplicationContext,
    window: &mut GameWindow,
) -> Result<OpenedSaveStore, String> {
    let original_error = match try_open(context) {
        Ok(store) => return Ok(OpenedSaveStore::Ready(store)),
        Err(error) => error,
    };
    tracing::error!("{original_error}");
    let profile = context
        .active_profile_snapshot()
        .map_err(|error| error.to_string())?;
    let (width, height) = window.logical_size();
    let mut renderer = Renderer::new(
        window,
        width as u16,
        height as u16,
        profile.graphic_config.scale_mode,
    );
    renderer.apply_upscale_config(&profile.graphic_config);
    let resources = IngameMenuResources::new(
        &mut renderer,
        context.shipping()?,
        context.preparation_files()?.clone(),
    )
    .ok_or_else(|| {
        format!("{original_error}; save recovery UI: Data/Interface/DEFAULT.RES unavailable")
    })?;
    Ok(open_with_recovery(context, window, &mut renderer, &resources, None).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn retry_requires_repaired_index_and_does_not_create_writable_fallback() {
        let directory = tempfile::tempdir().unwrap();
        let index = directory.path().join("saves.json");
        std::fs::write(&index, b"broken").unwrap();
        let attempt = || SaveGameManager::load_index(directory.path().to_str().unwrap());
        assert!(retry(attempt).is_err());
        assert!(retry(attempt).is_err());
        assert_eq!(std::fs::read(&index).unwrap(), b"broken");
        // External repair is a test action, not something the recovery UI does.
        std::fs::write(&index, br#"{"saves":[],"next_id":0}"#).unwrap();
        assert_eq!(retry(attempt).unwrap().count(), 0);
    }

    #[test]
    fn cancel_and_window_close_are_distinct_from_retry() {
        use crate::gfx_types::{GameEvent, Keycode};
        assert_eq!(
            recovery_choice(&GameEvent::Quit),
            Some(RecoveryChoice::Exit)
        );
        assert_eq!(
            recovery_choice(&GameEvent::KeyDown {
                keycode: Keycode::Escape,
                physical_key: None
            }),
            Some(RecoveryChoice::Cancel)
        );
        assert_eq!(
            recovery_choice(&GameEvent::KeyDown {
                keycode: Keycode::Return,
                physical_key: None
            }),
            Some(RecoveryChoice::Retry)
        );
    }
}
