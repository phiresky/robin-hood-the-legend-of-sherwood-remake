//! Save / Load slot picker.
//!
//! A 640x480 window with a list of save slots, a name-entry field (shown
//! in Save mode — empty when the "< New Save >" pseudo-row is selected,
//! prefilled with the slot's existing name when an existing slot is
//! selected so the user can edit in place and overwrite), a thumbnail
//! preview of the selected existing slot, and Load/Save, Delete, and
//! Cancel buttons.
//!
//! Character input is driven by SDL3's text-input subsystem: `SDL_StartTextInput`
//! is enabled for the lifetime of a Save-mode picker so that IME composition,
//! dead keys, and non-ASCII keyboard layouts all work. Non-character keys
//! (Backspace, Enter, Escape, arrows) are still handled off `KeyDown`.
//!
//! In Save mode the name-entry state is owned by a `WidgetInputField`,
//! kept in `SelectedEditable` the whole time the modal is up. SDL
//! text-input events feed straight into the widget's caret-aware insert
//! path; Backspace routes through `WidgetInputField::backspace` so the
//! caret and edit buffer stay in sync with no local bookkeeping.

use crate::gfx_types::Keycode;

use crate::geo2d;
use crate::gfx_types::GameEvent;
use crate::renderer::Renderer;
use crate::savegame::SaveGameManager;
use crate::sound::{AudioBackend, SoundManager};
use crate::ui::{MouseButtons, UiKeyboard, UiState};
use crate::widget::{FrameWnd, TextFromCaretSide, WidgetInput, WidgetInputField, WidgetPicture};
use jiff::{Timestamp, tz::TimeZone};
use robin_engine::profiles::ProfileManager;
use robin_engine::sound_cache::SampleLoader;
use robin_engine::sprite::BBox;

use super::layout::{
    MenuRect, MenuTransform, align_bottom_right, dim_screen, draw_background,
    draw_screen_background, enter_modal_gpu_phase, render_text_virt, render_text_virt_font,
};
use super::resources::{
    IngameMenuResources, MT_BTN_CANCEL, MT_BTN_DELETE, MT_BTN_LOAD, MT_BTN_SAVE,
    MT_MSG_REALLY_DELETE_SAVEGAME, MT_MSG_REALLY_OVERWRITE_SAVEGAME,
};
use super::widget_bridge::{self, ModalCursor, ModalInputState};
use super::yesno::show_yesno;

/// Which flavour of slot picker to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveLoadMode {
    Save,
    Load,
}

/// Outcome of the picker — the caller turns this into a `SaveLoadRequest`.
#[derive(Debug, Clone, Copy)]
pub enum SaveLoadOutcome {
    /// User cancelled; no action.
    Cancel,
    /// User accepted the save or load, referring to `saves[slot]` on the
    /// save manager at the time the outcome was produced.
    Slot(usize),
}

const INPUT_RECT: MenuRect = MenuRect {
    x: 30,
    y: 440,
    w: 420,
    h: 28,
};
const LOAD_LIST_RECT: MenuRect = MenuRect {
    x: 30,
    y: 10,
    w: 420,
    h: 450,
};
const SAVE_LIST_RECT: MenuRect = MenuRect {
    x: 30,
    y: 10,
    w: 420,
    h: 420,
};
const THUMB_RECT: MenuRect = MenuRect {
    x: 460,
    y: 0,
    w: 180,
    h: 135,
};
const DETAIL_ROW_HEIGHT: i32 = 36;

/// Longest allowed save name — passed to the input field as its
/// max-length cap.
const MAX_NAME_LEN: usize = 45;

const ID_LOAD_SAVE: u32 = 0;
const ID_DELETE: u32 = 1;
const ID_CANCEL: u32 = 2;

/// Display the save/load picker. `mission_id` is recorded onto any new
/// slot created in Save mode so headers stay consistent.
///
/// In Save mode the first row is a pseudo "< New Save >" entry. Selecting
/// it and confirming creates a fresh slot on `save_manager`. Selecting an
/// existing slot and confirming triggers an overwrite prompt first.
///
/// `sound` / `audio_backend` / `sample_loader` drive the input-field
/// noisy events — focus-sound and activation-sound are played through
/// [`WidgetInputField::play_noise`] each frame. All three are optional:
/// when any is `None` the modal is silent, matching the main-menu Load
/// path which doesn't thread audio.
#[allow(clippy::too_many_arguments)]
pub async fn show_save_load(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    mut cursor: Option<ModalCursor<'_>>,
    save_manager: &mut SaveGameManager,
    mission_id: u32,
    profiles: Option<&ProfileManager>,
    mode: SaveLoadMode,
    mut sound: Option<&mut SoundManager>,
    mut audio_backend: Option<&mut dyn AudioBackend>,
    sample_loader: Option<&SampleLoader>,
) -> SaveLoadOutcome {
    let sw = renderer.screen_width() as i32;
    let sh = renderer.screen_height() as i32;
    let transform = MenuTransform::centered(sw, sh);
    let list_rect = match mode {
        SaveLoadMode::Load => LOAD_LIST_RECT,
        SaveLoadMode::Save => SAVE_LIST_RECT,
    };
    let (btn_w, btn_h) = resources.button_dimensions();
    let load_save_label = resources.menu_text.get(match mode {
        SaveLoadMode::Save => MT_BTN_SAVE,
        SaveLoadMode::Load => MT_BTN_LOAD,
    });
    let delete_label = resources.menu_text.get(MT_BTN_DELETE);
    let cancel_label = resources.menu_text.get(MT_BTN_CANCEL);

    // Bottom-right stack of buttons.
    let bottom_labels: &[(&str, bool)] = &[
        (&load_save_label, false),
        (&delete_label, false),
        (&cancel_label, true),
    ];
    let bottom_buttons = align_bottom_right(bottom_labels, btn_w, btn_h);
    let btn_positions: [(u32, &str, i32, i32); 3] = [
        (
            ID_LOAD_SAVE,
            &load_save_label,
            bottom_buttons[0].x,
            bottom_buttons[0].y,
        ),
        (
            ID_DELETE,
            &delete_label,
            bottom_buttons[1].x,
            bottom_buttons[1].y,
        ),
        (
            ID_CANCEL,
            &cancel_label,
            bottom_buttons[2].x,
            bottom_buttons[2].y,
        ),
    ];

    let mut selected: Option<ListRow> = match mode {
        // Default the Save mode selection to the "new save" pseudo-row so
        // the action button is enabled out of the gate.
        SaveLoadMode::Save => Some(ListRow::New),
        SaveLoadMode::Load => None,
    };

    // Snapshot of visible save indices. Filter depends on mode (Load
    // hides only Continue/Restart; Save hides every special slot).
    // Sort before rebuilding the list so the entries display in
    // chronological order rather than insertion order.
    save_manager.sort_by_time();
    let mut visible = collect_visible_slots(save_manager, mode);
    let visible_rows = (list_rect.h / DETAIL_ROW_HEIGHT).max(1) as usize;
    let mut scroll_offset: usize = 0;

    // Name-entry state lives on a `WidgetInputField` kept in
    // `SelectedEditable` for the duration of the Save-mode dialog. SDL
    // text input flows straight into the widget's caret-aware insert
    // path each frame. The widget is resynced via `set_text` whenever
    // the list selection changes — empty when the "< New Save >"
    // pseudo-row is selected, prefilled with the slot's display text on
    // an existing slot.
    const ID_INPUT_FIELD: u32 = 1000;
    let mut input_widget = WidgetInputField::new(ID_INPUT_FIELD);
    input_widget.set_max_length(MAX_NAME_LEN);
    // Give the widget a bbox so the state machine's bookkeeping stays
    // sane — not used for hit-testing because we never leave edit mode.
    input_widget.base.bbox = robin_engine::coordinates::ScreenBBox::from_coords(
        INPUT_RECT.x as f32,
        INPUT_RECT.y as f32,
        (INPUT_RECT.x + INPUT_RECT.w) as f32,
        (INPUT_RECT.y + INPUT_RECT.h) as f32,
    );
    if mode == SaveLoadMode::Save {
        input_widget.enter_edit_mode();
    }
    let mut caret_timer: u32 = 0;

    // Enable SDL text input for the lifetime of a Save-mode picker so
    // IME composition and non-ASCII layouts work. Load mode stays quiet.
    if mode == SaveLoadMode::Save {
        crate::window::start_text_input();
    }

    // Thumbnail preview state: a WidgetPicture owns the alternate-surface
    // handle; the metadata cache tracks which slot the surface was
    // built for so we only rebuild on selection change.
    let mut thumb_widget = WidgetPicture::new(u32::MAX);
    let mut thumb_cache: Option<ThumbnailCache> = None;

    let mut input_state = ModalInputState::new();
    input_state.seed_mouse_from_sdl(event_pump, transform);

    // Stub keyboard fed into the input-field widget so its special-key
    // branches (Backspace / Delete / Left / Right / Home / End / Tab /
    // Up / Down / Enter / Escape) stay silent. The modal handles those
    // at the `GameEvent::KeyDown` level and drives the widget via the
    // public caret / backspace helpers — otherwise the release-edge
    // `KeyPressed` transitions would double-fire with the modal's
    // press-edge handling. Kept outside the loop so we don't pay the
    // `Vec` reallocation on every frame.
    let empty_keyboard = UiKeyboard::default();

    let outcome = loop {
        // Build (or rebuild) the widget frame. Save mode accepts an
        // empty name and fills a default label on confirmation.
        let action_enabled = match (mode, selected) {
            (SaveLoadMode::Save, Some(_)) => true,
            (SaveLoadMode::Load, Some(ListRow::Existing(_))) => true,
            _ => false,
        };
        let delete_enabled = matches!(selected, Some(ListRow::Existing(_)));
        let mut frame = FrameWnd::default();
        frame.enabled = true;
        frame.input_enabled = true;
        for (id, label, x, y) in &btn_positions {
            let enabled = match *id {
                ID_LOAD_SAVE => action_enabled,
                ID_DELETE => delete_enabled,
                _ => true,
            };
            frame.add_widget_absolute(widget_bridge::make_button_enabled(
                *id, label, enabled, *x, *y, btn_w, btn_h,
            ));
        }

        // In Save mode the input is always editable. Load mode never shows it.
        let input_editable = mode == SaveLoadMode::Save;

        // ── Event loop ──────────────────────────────────────────
        let mut activated: Option<u32> = None;
        for event in event_pump.poll_events() {
            input_state.update_from_event(&event, transform);
            match event {
                GameEvent::Quit => {
                    activated = Some(ID_CANCEL);
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => {
                    activated = Some(ID_CANCEL);
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Up,
                    ..
                } => {
                    let new_sel = previous_row(selected, mode, visible.len());
                    if new_sel != selected {
                        selected = new_sel;
                        sync_input_for_selection(
                            &mut input_widget,
                            selected,
                            mode,
                            &visible,
                            save_manager,
                        );
                        caret_timer = 0;
                    }
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Down,
                    ..
                } => {
                    let new_sel = next_row(selected, mode, visible.len());
                    if new_sel != selected {
                        selected = new_sel;
                        sync_input_for_selection(
                            &mut input_widget,
                            selected,
                            mode,
                            &visible,
                            save_manager,
                        );
                        caret_timer = 0;
                    }
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Return,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::KpEnter,
                    ..
                } if action_enabled => {
                    activated = Some(ID_LOAD_SAVE);
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Backspace,
                    ..
                } if input_editable => {
                    input_widget.backspace();
                    caret_timer = 0;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Delete,
                    ..
                } if input_editable => {
                    input_widget.delete_char();
                    caret_timer = 0;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Left,
                    ..
                } if input_editable => {
                    input_widget.move_caret_left();
                    caret_timer = 0;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Right,
                    ..
                } if input_editable => {
                    input_widget.move_caret_right();
                    caret_timer = 0;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Home,
                    ..
                } if input_editable => {
                    input_widget.move_caret_home();
                    caret_timer = 0;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::End,
                    ..
                } if input_editable => {
                    input_widget.move_caret_end();
                    caret_timer = 0;
                }
                GameEvent::TextInput { .. } if input_editable => {
                    // Text input is consumed by the widget below via
                    // `ModalInputState::as_widget_input().text_input`
                    // after it's been accumulated. Reset the caret
                    // blink so the insertion stays visible.
                    caret_timer = 0;
                }
                // Row selection + double-click activation fire on the
                // release edge. Double-click detection uses SDL3's
                // native counter, tracked for us on MouseDown/MouseUp by
                // `ModalInputState::update_from_event` above.
                GameEvent::MouseUp(x, y, 1) => {
                    let (vx, vy) = transform.from_screen(x, y);
                    if list_rect.contains_virt(vx, vy) {
                        let row_offset =
                            ((vy - list_rect.y - 4) / DETAIL_ROW_HEIGHT).max(0) as usize;
                        let new_selection = row_at(mode, scroll_offset + row_offset, visible.len());
                        if new_selection != selected {
                            selected = new_selection;
                            sync_input_for_selection(
                                &mut input_widget,
                                selected,
                                mode,
                                &visible,
                                save_manager,
                            );
                            caret_timer = 0;
                        }
                        if input_state
                            .buttons
                            .contains(MouseButtons::LEFT_DOUBLE_CLICK)
                            && selected.is_some()
                        {
                            // Match the action-enable rules used by the
                            // explicit button / Enter-key path.
                            let action_enabled_now = match (mode, selected) {
                                (SaveLoadMode::Save, Some(_)) => true,
                                (SaveLoadMode::Load, Some(ListRow::Existing(_))) => true,
                                _ => false,
                            };
                            if action_enabled_now {
                                activated = Some(ID_LOAD_SAVE);
                            }
                        }
                    } else if let Some(id) = hit_button(
                        vx,
                        vy,
                        &btn_positions,
                        btn_w,
                        btn_h,
                        action_enabled,
                        delete_enabled,
                    ) {
                        activated = Some(id);
                    }
                }
                GameEvent::MouseWheel(dy) => {
                    let total = total_rows(mode, visible.len());
                    let max_scroll = total.saturating_sub(visible_rows);
                    if dy > 0 {
                        scroll_offset = scroll_offset.saturating_sub(1);
                    } else if dy < 0 {
                        scroll_offset = (scroll_offset + 1).min(max_scroll);
                    }
                }
                _ => {}
            }
        }

        let widget_input = input_state.as_widget_input();
        let mouse_virt = widget_input.mouse_position;
        let widget_events = frame.process_input(&widget_input);
        let mut field_events: Vec<crate::ui::UiEvent> = Vec::new();
        if input_editable {
            // Feed the text-input buffer straight to the input widget
            // so the caret-aware insert path in `process_input_editable`
            // handles composition, max-length, and control-char filter.
            // Build a dedicated `WidgetInput` so the mouse/button state
            // for the button frame doesn't accidentally drive state
            // transitions on the field (which we force to stay
            // `SelectedEditable` regardless).
            let field_input = WidgetInput {
                mouse_position: widget_input.mouse_position,
                mouse_z: widget_input.mouse_z,
                mouse_button: MouseButtons::empty(),
                keyboard: &empty_keyboard,
                text_input: widget_input.text_input,
                capture: None,
            };
            field_events = input_widget.process_input(&field_input);
            // If the state machine fell out of edit mode for any reason
            // (shouldn't happen here but be defensive), put it back so
            // subsequent frames still accept text input.
            if input_widget.base.state != UiState::SelectedEditable {
                input_widget.enter_edit_mode();
            }
        }
        input_state.end_frame();

        // Play menu sounds for any noisy events emitted this frame.
        // Buttons use `WIDGET_NOISY_BUTTON`; the input field uses
        // `WIDGET_NOISY_INPUTFIELD`. Each routed through its own bank so
        // the first-match behaviour of `play_widget_noise` doesn't
        // cross-wire them.
        if let (Some(snd), Some(loader)) = (sound.as_deref_mut(), sample_loader) {
            let backend: Option<&mut dyn AudioBackend> = audio_backend
                .as_deref_mut()
                .map(|b| b as &mut dyn AudioBackend);
            widget_bridge::play_widget_noise(
                &widget_events,
                widget_bridge::WIDGET_NOISY_BUTTON,
                snd,
                backend,
                loader,
            );
        }
        if !field_events.is_empty()
            && let (Some(snd), Some(loader)) = (sound.as_deref_mut(), sample_loader)
        {
            let backend: Option<&mut dyn AudioBackend> = audio_backend
                .as_deref_mut()
                .map(|b| b as &mut dyn AudioBackend);
            WidgetInputField::play_noise(&field_events, snd, backend, loader);
        }

        if let Some(id) = widget_bridge::find_activated(&widget_events) {
            activated = Some(id);
        }

        if let Some(id) = activated {
            match id {
                ID_CANCEL => break SaveLoadOutcome::Cancel,
                ID_LOAD_SAVE => match (mode, selected) {
                    (SaveLoadMode::Save, Some(ListRow::New)) => {
                        let text = accepted_save_text(
                            &input_widget.edit_text,
                            selected,
                            save_manager,
                            &visible,
                            mission_id,
                            profiles,
                        );
                        let idx = save_manager.create(text, mission_id);
                        break SaveLoadOutcome::Slot(idx);
                    }
                    (SaveLoadMode::Save, Some(ListRow::Existing(v_idx))) => {
                        let slot = visible[v_idx];
                        let msg = resources.menu_text.get(MT_MSG_REALLY_OVERWRITE_SAVEGAME);
                        if show_yesno(
                            event_pump,
                            renderer,
                            resources,
                            cursor.as_mut().map(|c| c.reborrow()),
                            &msg,
                        )
                        .await
                        {
                            // Apply edited name to the slot before overwriting.
                            let new_text = accepted_save_text(
                                &input_widget.edit_text,
                                selected,
                                save_manager,
                                &visible,
                                mission_id,
                                profiles,
                            );
                            if !new_text.is_empty() {
                                save_manager
                                    .get_mut(slot)
                                    .expect("visible slot must exist")
                                    .text = new_text;
                            }
                            break SaveLoadOutcome::Slot(slot);
                        }
                    }
                    (SaveLoadMode::Load, Some(ListRow::Existing(v_idx))) => {
                        break SaveLoadOutcome::Slot(visible[v_idx]);
                    }
                    _ => {}
                },
                ID_DELETE => {
                    if let Some(ListRow::Existing(v_idx)) = selected {
                        let slot = visible[v_idx];
                        let msg = resources.menu_text.get(MT_MSG_REALLY_DELETE_SAVEGAME);
                        if show_yesno(
                            event_pump,
                            renderer,
                            resources,
                            cursor.as_mut().map(|c| c.reborrow()),
                            &msg,
                        )
                        .await
                        {
                            save_manager.remove(slot);
                            // Sort before rebuilding the list, including
                            // post-delete refreshes.
                            save_manager.sort_by_time();
                            visible = collect_visible_slots(save_manager, mode);
                            selected = None;
                            scroll_offset = 0;
                            sync_input_for_selection(
                                &mut input_widget,
                                selected,
                                mode,
                                &visible,
                                save_manager,
                            );
                            if let Some(old) = thumb_cache.take() {
                                renderer.delete_surface(old.surface_id);
                                thumb_widget.reset_alternate_picture();
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Refresh the thumbnail cache so the preview tracks the
        // currently selected existing slot. Off-slot selections drop
        // the cached surface; changing slot rebuilds it.
        sync_thumbnail_cache(
            &mut thumb_cache,
            &mut thumb_widget,
            selected,
            &visible,
            save_manager,
            renderer,
            mode,
        );

        // ── Render ──────────────────────────────────────────────
        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);

        if let Some(bg) = resources.menu_bg[3] {
            draw_screen_background(renderer, &bg);
        }

        // Input field — only drawn in Save + New mode.
        if input_editable {
            draw_input_field(renderer, resources, transform, &input_widget, caret_timer);
        }

        // Rows.
        let total = total_rows(mode, visible.len());
        let scrollbar_w = list_scrollbar_width(resources);
        let needs_scrollbar = total > visible_rows && scrollbar_w > 0;
        // Row area matches the old +10 left padding; mirror it on the
        // right and keep text out from under the scrollbar.
        let row_area_x = list_rect.x + 10;
        let row_area_w = list_rect.w - 20 - if needs_scrollbar { scrollbar_w } else { 0 };

        // Per-row hover → `list_focused` font; the renderer picks
        // between default / focused / selected per row flags. The
        // mouse position was snapshotted before `end_frame()` in
        // virtual menu coords, so we hit-test against the active list
        // rect directly.
        let hovered_row = if list_rect.contains_virt(mouse_virt.x as i32, mouse_virt.y as i32) {
            let row_offset =
                ((mouse_virt.y as i32 - list_rect.y - 4) / DETAIL_ROW_HEIGHT).max(0) as usize;
            row_at(mode, scroll_offset + row_offset, visible.len())
        } else {
            None
        };

        for row_offset in 0..visible_rows {
            let row_index = scroll_offset + row_offset;
            if row_index >= total {
                break;
            }
            let row = row_at_unchecked(mode, row_index, visible.len());
            let row_y = list_rect.y + 4 + row_offset as i32 * DETAIL_ROW_HEIGHT;
            let is_selected = selected == Some(row);
            let is_focused = hovered_row == Some(row);
            let label = row_label(row, save_manager, &visible);
            let detail = row_detail(row, save_manager, &visible);

            let Some(font) = resources.list_font(is_focused, is_selected) else {
                continue;
            };
            let fitted = truncate_to_pixel_width(font, &label, row_area_w);
            if !fitted.is_empty() {
                render_text_virt_font(renderer, font, transform, &fitted, row_area_x, row_y);
            }
            let detail_fitted = truncate_to_pixel_width(font, &detail, row_area_w);
            if !detail_fitted.is_empty() {
                render_text_virt_font(
                    renderer,
                    font,
                    transform,
                    &detail_fitted,
                    row_area_x,
                    row_y + 16,
                );
            }
        }

        if needs_scrollbar {
            draw_listbox_scrollbar(
                renderer,
                transform,
                resources,
                list_rect.x + list_rect.w - scrollbar_w,
                list_rect.y,
                scrollbar_w,
                list_rect.h,
                scroll_offset,
                visible_rows,
                total,
            );
        }

        // Thumbnail preview.
        draw_preview(
            renderer,
            transform,
            selected,
            &visible,
            thumb_cache.as_ref(),
            &thumb_widget,
        );

        // Buttons.
        widget_bridge::draw_frame_buttons(renderer, resources, transform, &frame);

        if let Some(c) = &cursor {
            c.draw(renderer, transform, &input_state);
        }

        renderer.present();
        caret_timer = caret_timer.wrapping_add(1);
        crate::window::sleep_ms(16).await;
    };

    // Make sure the cached thumbnail surface is returned to the renderer
    // pool before we unwind.
    if let Some(cache) = thumb_cache {
        renderer.delete_surface(cache.surface_id);
        thumb_widget.reset_alternate_picture();
    }
    if mode == SaveLoadMode::Save {
        crate::window::stop_text_input();
    }

    outcome
}

/// Tracks a loaded thumbnail so we don't rebuild the GPU surface on
/// every frame while the selection is stable.
struct ThumbnailCache {
    slot: usize,
    surface_id: u32,
    width: u16,
    height: u16,
}

fn sync_thumbnail_cache(
    cache: &mut Option<ThumbnailCache>,
    widget: &mut crate::widget::WidgetPicture,
    selected: Option<ListRow>,
    visible: &[usize],
    save_manager: &SaveGameManager,
    renderer: &mut Renderer,
    mode: SaveLoadMode,
) {
    // Save-mode never previews a thumbnail — the picture widget stays
    // disabled and the entire reload branch is gated on Load mode.
    let target_slot = match (mode, selected) {
        (SaveLoadMode::Load, Some(ListRow::Existing(v))) => visible.get(v).copied(),
        _ => None,
    };
    match (&*cache, target_slot) {
        (Some(c), Some(slot)) if c.slot == slot => {}
        (_, None) => {
            if let Some(old) = cache.take() {
                renderer.delete_surface(old.surface_id);
            }
            widget.reset_alternate_picture();
        }
        (_, Some(slot)) => {
            if let Some(old) = cache.take() {
                renderer.delete_surface(old.surface_id);
            }
            widget.reset_alternate_picture();
            if let Some(thumb) = save_manager.load_thumbnail(slot) {
                let id = renderer
                    .create_surface_from_rgb565(thumb.width, thumb.height, &thumb.pixels)
                    .expect("save thumbnail dimensions must match RGB565 payload");
                widget.set_alternate_picture(id);
                *cache = Some(ThumbnailCache {
                    slot,
                    surface_id: id,
                    width: thumb.width,
                    height: thumb.height,
                });
            }
        }
    }
}

fn draw_input_field(
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    transform: MenuTransform,
    input_widget: &WidgetInputField,
    caret_timer: u32,
) {
    // Use the menu's input-field sprite if loaded, otherwise fall back
    // to a simple outlined rect so layouts without DEFAULT.RES still
    // render something usable.
    if let Some(surf) = resources.input_field_surface(true) {
        widget_bridge::draw_picture_surface_rect(
            renderer,
            transform,
            surf,
            INPUT_RECT.x,
            INPUT_RECT.y,
            INPUT_RECT.w,
            INPUT_RECT.h,
            0,
            0,
            INPUT_RECT.w,
            INPUT_RECT.h,
            true,
        );
    } else {
        draw_fallback_rect(renderer, transform, &INPUT_RECT);
    }

    let Some(font) = resources.label_font() else {
        return;
    };

    // Split the text into visible-left and visible-right slices around
    // the caret using `WidgetInputField::get_text_from_caret`, with a
    // horizontal scroll offset so the caret stays inside the field when
    // the full string would overflow. Per-char advance is
    // `character_width(ch) + extra_spacing()`.
    let extra = font.extra_spacing();
    let char_advance =
        |ch: char| -> u32 { ((font.character_width(ch) as i32) + extra).max(0) as u32 };

    // Interior width — 6px padding on each side, matching the text-
    // origin offset used below.
    let interior_w = (INPUT_RECT.w - 12).max(0) as u32;

    // Pixel position of the caret measured from the start of the full text.
    let caret_pixel: u32 = input_widget
        .edit_text
        .chars()
        .take(input_widget.caret_offset)
        .map(char_advance)
        .sum();

    // If the caret runs past the right edge, shift everything left
    // by `interior_w - caret_pixel` (a negative offset) so the caret
    // sits flush against the right edge.
    let scroll_offset: i32 = if caret_pixel >= interior_w {
        interior_w as i32 - caret_pixel as i32
    } else {
        0
    };
    let left_budget = (caret_pixel as i32 + scroll_offset).max(0) as u32;
    let right_budget = (interior_w as i32 - caret_pixel as i32 - scroll_offset).max(0) as u32;

    let left_text =
        input_widget.get_text_from_caret(TextFromCaretSide::Left, left_budget, char_advance);
    let right_text =
        input_widget.get_text_from_caret(TextFromCaretSide::Right, right_budget, char_advance);

    // Render the buffer plus a blinking caret. `caret_timer` ticks once
    // per frame (~60 Hz); the caret toggles every ~500 ms. We don't
    // have a dedicated caret sprite yet, so this inlines a `|` character
    // at the caret position.
    let show_caret = (caret_timer / 30).is_multiple_of(2);
    let display = if show_caret {
        format!("{left_text}|{right_text}")
    } else {
        format!("{left_text}{right_text}")
    };
    render_text_virt(
        renderer,
        font,
        transform,
        &display,
        INPUT_RECT.x + 6,
        INPUT_RECT.y + 6,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_preview(
    renderer: &mut Renderer,
    transform: MenuTransform,
    selected: Option<ListRow>,
    visible: &[usize],
    thumb_cache: Option<&ThumbnailCache>,
    thumb_widget: &crate::widget::WidgetPicture,
) {
    let slot = match selected {
        Some(ListRow::Existing(v)) => match visible.get(v) {
            Some(&s) => s,
            None => return,
        },
        _ => return,
    };

    // Thumbnail image. The original RHMenuLoadSave disables the picture
    // widget when there is no selected save or no thumbnail file; it
    // does not draw a placeholder frame or metadata panel.
    if let Some(cache) = thumb_cache
        && cache.slot == slot
    {
        let mut widget = thumb_widget.clone();
        widget
            .base
            .set_position(robin_engine::coordinates::ScreenBBox::from_coords(
                (THUMB_RECT.x + 4) as f32,
                (THUMB_RECT.y + 4) as f32,
                (THUMB_RECT.x + THUMB_RECT.w - 4) as f32,
                (THUMB_RECT.y + THUMB_RECT.h - 4) as f32,
            ));
        widget_bridge::draw_picture_alternate_surface(
            renderer,
            transform,
            &widget,
            i32::from(cache.width),
            i32::from(cache.height),
            true,
        );
    }
}

fn draw_fallback_rect(renderer: &mut Renderer, transform: MenuTransform, rect: &MenuRect) {
    let (sx, sy) = transform.to_screen(rect.x, rect.y);
    renderer.fill_screen(
        Some(&BBox::new(
            geo2d::pt(sx as f32, sy as f32),
            geo2d::pt((sx + rect.w) as f32, (sy + rect.h) as f32),
        )),
        Renderer::create_color_16(30, 25, 15),
    );
    renderer.draw_rect_outline_screen(
        sx,
        sy,
        sx + rect.w,
        sy + rect.h,
        Renderer::create_color_16(180, 160, 100),
    );
}

fn list_scrollbar_width(resources: &IngameMenuResources) -> i32 {
    resources.list_scrollbar[0].map_or(0, |s| s.width)
}

#[allow(clippy::too_many_arguments)]
fn draw_listbox_scrollbar(
    renderer: &mut Renderer,
    transform: MenuTransform,
    resources: &IngameMenuResources,
    track_x: i32,
    track_y: i32,
    track_w: i32,
    track_h: i32,
    scroll_offset: usize,
    visible_rows: usize,
    total_rows: usize,
) {
    let slices = &resources.list_scrollbar;
    let (Some(back_start), Some(back_fill), Some(back_end)) = (slices[0], slices[1], slices[2])
    else {
        return;
    };
    let (Some(thumb_start), Some(thumb_fill), Some(thumb_end)) = (slices[3], slices[4], slices[5])
    else {
        return;
    };

    let start_h = back_start.height.min(track_h);
    let end_h = back_end.height.min(track_h - start_h);
    let fill_y = track_y + start_h;
    let fill_h = (track_h - start_h - end_h).max(0);
    draw_background(
        renderer,
        transform,
        &back_start,
        track_x,
        track_y,
        track_w,
        start_h,
    );
    if fill_h > 0 {
        draw_background(
            renderer, transform, &back_fill, track_x, fill_y, track_w, fill_h,
        );
    }
    draw_background(
        renderer,
        transform,
        &back_end,
        track_x,
        track_y + track_h - end_h,
        track_w,
        end_h,
    );

    let total = total_rows.max(1) as f32;
    let before_ratio = (scroll_offset as f32 / total).clamp(0.0, 1.0);
    let knob_ratio = (visible_rows as f32 / total).clamp(0.0, 1.0);
    let usable = (track_h - 2).max(0) as f32;
    let thumb_top = track_y + 1 + (usable * before_ratio) as i32;
    let thumb_bot = track_y + 1 + (usable * (before_ratio + knob_ratio)) as i32;
    let thumb_h = (thumb_bot - thumb_top).max(thumb_start.height + thumb_end.height);

    let thumb_start_h = thumb_start.height.min(thumb_h);
    let thumb_end_h = thumb_end.height.min(thumb_h - thumb_start_h);
    let thumb_fill_h = (thumb_h - thumb_start_h - thumb_end_h).max(0);
    draw_background(
        renderer,
        transform,
        &thumb_start,
        track_x,
        thumb_top,
        track_w,
        thumb_start_h,
    );
    if thumb_fill_h > 0 {
        draw_background(
            renderer,
            transform,
            &thumb_fill,
            track_x,
            thumb_top + thumb_start_h,
            track_w,
            thumb_fill_h,
        );
    }
    draw_background(
        renderer,
        transform,
        &thumb_end,
        track_x,
        thumb_top + thumb_h - thumb_end_h,
        track_w,
        thumb_end_h,
    );
}

/// One row in the slot list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListRow {
    /// The synthetic "New Save" row at the top of Save-mode lists.
    New,
    /// An existing slot at `visible[idx]`.
    Existing(usize),
}

fn total_rows(mode: SaveLoadMode, n_visible: usize) -> usize {
    match mode {
        SaveLoadMode::Save => n_visible + 1,
        SaveLoadMode::Load => n_visible,
    }
}

fn row_at(mode: SaveLoadMode, index: usize, n_visible: usize) -> Option<ListRow> {
    if index >= total_rows(mode, n_visible) {
        return None;
    }
    Some(row_at_unchecked(mode, index, n_visible))
}

fn row_at_unchecked(mode: SaveLoadMode, index: usize, _n_visible: usize) -> ListRow {
    match mode {
        SaveLoadMode::Save => {
            if index == 0 {
                ListRow::New
            } else {
                ListRow::Existing(index - 1)
            }
        }
        SaveLoadMode::Load => ListRow::Existing(index),
    }
}

fn previous_row(current: Option<ListRow>, mode: SaveLoadMode, n_visible: usize) -> Option<ListRow> {
    let total = total_rows(mode, n_visible);
    if total == 0 {
        return None;
    }
    let cur_idx = match current {
        Some(row) => row_index(row, mode),
        None => return row_at(mode, 0, n_visible),
    };
    let new_idx = cur_idx.saturating_sub(1);
    row_at(mode, new_idx, n_visible)
}

fn next_row(current: Option<ListRow>, mode: SaveLoadMode, n_visible: usize) -> Option<ListRow> {
    let total = total_rows(mode, n_visible);
    if total == 0 {
        return None;
    }
    let cur_idx = match current {
        Some(row) => row_index(row, mode),
        None => return row_at(mode, 0, n_visible),
    };
    let new_idx = (cur_idx + 1).min(total - 1);
    row_at(mode, new_idx, n_visible)
}

fn row_index(row: ListRow, mode: SaveLoadMode) -> usize {
    match (mode, row) {
        (SaveLoadMode::Save, ListRow::New) => 0,
        (SaveLoadMode::Save, ListRow::Existing(v)) => v + 1,
        (SaveLoadMode::Load, ListRow::Existing(v)) => v,
        (SaveLoadMode::Load, ListRow::New) => 0, // shouldn't happen
    }
}

/// Build the listbox row label. The original menu adds only
/// `RHSaveGame::GetText()` to the list box.
fn row_label(row: ListRow, save_manager: &SaveGameManager, visible: &[usize]) -> String {
    match row {
        ListRow::New => "< New Save >".to_string(),
        ListRow::Existing(v_idx) => {
            let slot = visible[v_idx];
            match save_manager.get(slot) {
                Some(s) => s.text.clone(),
                None => format!("<invalid slot {slot}>"),
            }
        }
    }
}

fn row_detail(row: ListRow, save_manager: &SaveGameManager, visible: &[usize]) -> String {
    match row {
        ListRow::New => "Name optional - creates a new save slot".to_string(),
        ListRow::Existing(v_idx) => {
            let slot = visible[v_idx];
            let Some(save) = save_manager.get(slot) else {
                return String::new();
            };

            let mut parts = vec![format!("Saved {}", format_saved_time(&save.timestamp))];
            if !save.mission_name.is_empty() && save.text.trim() != save.mission_name.trim() {
                parts.push(save.mission_name.clone());
            }
            if let Some(progress) = save.campaign_progress {
                parts.push(format!("{progress}% campaign"));
            }
            if let (Some(done), Some(total)) = (save.missions_done, save.missions_total) {
                parts.push(format!("{done}/{total} missions"));
            }
            if let Some(gang) = save.gang_size {
                parts.push(format!("Gang {gang}"));
            }
            if let Some(ransom) = save.ransom {
                parts.push(format!("Ransom {ransom}"));
            }
            if let Some(blazons) = save.blazons {
                parts.push(format!("Blazons {blazons}"));
            }
            if let Some(amulets) = save.amulets {
                parts.push(format!("Amulets {amulets}"));
            }
            parts.join(" | ")
        }
    }
}

fn accepted_save_text(
    input_text: &str,
    selected: Option<ListRow>,
    save_manager: &SaveGameManager,
    visible: &[usize],
    mission_id: u32,
    profiles: Option<&ProfileManager>,
) -> String {
    let trimmed = input_text.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    if let Some(ListRow::Existing(v_idx)) = selected
        && let Some(slot) = visible.get(v_idx).and_then(|&slot| save_manager.get(slot))
        && !slot.text.trim().is_empty()
    {
        return slot.text.clone();
    }
    default_save_text(save_manager, mission_id, profiles)
}

fn default_save_text(
    save_manager: &SaveGameManager,
    mission_id: u32,
    profiles: Option<&ProfileManager>,
) -> String {
    mission_display_name(mission_id, profiles)
        .unwrap_or_else(|| format!("Save {}", save_manager.count() + 1))
}

fn format_saved_time(timestamp: &str) -> String {
    let Some(zoned) = timestamp
        .parse::<i64>()
        .ok()
        .and_then(|secs| Timestamp::from_second(secs).ok())
        .map(|ts| ts.to_zoned(TimeZone::system()))
    else {
        return "unknown".to_string();
    };
    zoned.strftime("%Y-%m-%d %H:%M").to_string()
}

fn mission_display_name(mission_id: u32, profiles: Option<&ProfileManager>) -> Option<String> {
    let profiles = profiles?;
    profiles
        .missions
        .iter()
        .find(|mission| mission.id == mission_id)
        .map(|mission| mission.mission_name.clone())
        .filter(|name| !name.trim().is_empty())
}

fn hit_button(
    vx: i32,
    vy: i32,
    btn_positions: &[(u32, &str, i32, i32); 3],
    btn_w: i32,
    btn_h: i32,
    action_enabled: bool,
    delete_enabled: bool,
) -> Option<u32> {
    for (id, _, x, y) in btn_positions {
        if vx < *x || vx >= *x + btn_w || vy < *y || vy >= *y + btn_h {
            continue;
        }
        let enabled = match *id {
            ID_LOAD_SAVE => action_enabled,
            ID_DELETE => delete_enabled,
            _ => true,
        };
        if enabled {
            return Some(*id);
        }
    }
    None
}

/// Truncate `text` to the longest prefix that fits in `max_w` pixels
/// when rendered with `font`. Oversize text gets an ASCII ellipsis so
/// clipped metadata is visibly abbreviated instead of looking like a
/// broken string.
fn truncate_to_pixel_width(font: &crate::native_font::Font, text: &str, max_w: i32) -> String {
    if max_w <= 0 {
        return String::new();
    }
    if font.text_width(text) <= max_w {
        return text.to_string();
    }

    let ellipsis = "...";
    let ellipsis_w = font.text_width(ellipsis);
    if ellipsis_w > max_w {
        return String::new();
    }

    let budget = max_w - ellipsis_w;
    // `text` doesn't fit in full — scan prefix-by-prefix for the
    // longest one that does.  `char_indices()` yields byte offsets at
    // the *start* of each char, so `text[..idx]` is the prefix with
    // `idx` excluded.
    let mut fit_end = 0;
    for (idx, _) in text.char_indices() {
        if font.text_width(&text[..idx]) > budget {
            return format!("{}{}", &text[..fit_end], ellipsis);
        }
        fit_end = idx;
    }
    format!("{}{}", &text[..fit_end], ellipsis)
}

/// Resync the input-field widget to the current selection. In Save
/// mode, an existing-slot selection prefills the widget with that
/// slot's display text (so the user can edit in place and overwrite);
/// the New pseudo-row clears it. Load mode clears unconditionally —
/// the field isn't shown.
///
/// `set_text` leaves the widget in `SelectedEditable` (it only touches
/// the buffer + caret) so subsequent text input keeps flowing through.
fn sync_input_for_selection(
    input_widget: &mut WidgetInputField,
    selection: Option<ListRow>,
    mode: SaveLoadMode,
    visible: &[usize],
    save_manager: &SaveGameManager,
) {
    if mode != SaveLoadMode::Save {
        input_widget.set_text("");
        return;
    }
    match selection {
        Some(ListRow::Existing(v_idx)) => {
            let slot = visible[v_idx];
            let save = save_manager
                .get(slot)
                .expect("visible slot must resolve to a save");
            input_widget.set_text(&save.text);
        }
        _ => input_widget.set_text(""),
    }
    // Park the caret at the end so the user is typing after the
    // prefilled name, not in the middle of it.
    input_widget.caret_offset = input_widget.edit_text.chars().count();
}

/// Collect the indices of user-visible saves for the given picker mode.
///
/// - **Load**: hides only Continue and Restart. QuickSave / ExQuickSave /
///   Sherwood are still loadable by the player.
/// - **Save**: hides *any* special slot so the player can't overwrite
///   the auto-managed Continue/QuickSave/etc. entries by hand.
fn collect_visible_slots(save_manager: &SaveGameManager, mode: SaveLoadMode) -> Vec<usize> {
    (0..save_manager.count())
        .filter(|&i| {
            let save = save_manager
                .get(i)
                .expect("index from 0..count() must resolve");
            match mode {
                SaveLoadMode::Load => !save.is_continue() && !save.is_restart(),
                SaveLoadMode::Save => save.special.is_none(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_new_save_name_gets_default_label() {
        let save_manager = SaveGameManager::new("/tmp/test_saves".into());
        let text = accepted_save_text("", Some(ListRow::New), &save_manager, &[], 123, None);
        assert_eq!(text, "Save 1");
    }

    #[test]
    fn empty_existing_save_name_preserves_slot_label() {
        let mut save_manager = SaveGameManager::new("/tmp/test_saves".into());
        let slot = save_manager.create("Existing Slot".into(), 123);
        let visible = [slot];

        assert_eq!(
            accepted_save_text(
                "   ",
                Some(ListRow::Existing(0)),
                &save_manager,
                &visible,
                123,
                None
            ),
            "Existing Slot"
        );
        assert_eq!(
            accepted_save_text(
                " Renamed ",
                Some(ListRow::Existing(0)),
                &save_manager,
                &visible,
                123,
                None
            ),
            "Renamed"
        );
    }
}
