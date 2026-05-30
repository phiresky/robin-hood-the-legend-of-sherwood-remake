//! Main-menu "Select Player" screen.
//!
//! Shows the global [`PlayerProfileManager`] roster, lets the player
//! pick an entry to set as active, create a new profile (name +
//! difficulty), delete, or rename via a small modal backed by the SDL3
//! text-input pipeline (IME composition, dead keys, non-ASCII
//! keyboards).

use crate::gfx_types::Keycode;

use crate::geo2d;
use crate::gfx_types::GameEvent;
use crate::ingame_menu::layout::{
    MENU_H, MENU_W, MenuRect, MenuTransform, align_bottom_right, dim_screen, draw_background,
    draw_screen_background, enter_modal_gpu_phase, render_text_virt,
};
use crate::ingame_menu::resources::{
    IngameMenuResources, MT_BTN_CANCEL, MT_BTN_DELETE, MT_BTN_NEW, MT_BTN_OK, MT_BTN_RENAME,
    MT_BTN_SELECT, MT_MSG_REALLY_DELETE_PLAYER, MT_STR_ANONYMOUS, MT_STR_DIFFICULTY_EASY,
    MT_STR_DIFFICULTY_HARD, MT_STR_DIFFICULTY_LEVEL, MT_STR_DIFFICULTY_MEDIUM, MT_STR_NAME,
    MT_TTL_NEW_PLAYER,
};
use crate::ingame_menu::widget_bridge::{self, ModalCursor, ModalInputState};
use crate::ingame_menu::yesno::show_yesno;
use crate::player_profile::{DifficultyLevel, PlayerProfile, PlayerProfileManager};
use crate::renderer::Renderer;
use crate::resource_ids;
use crate::ui_screens::MAX_PLAYER_NAME_LENGTH;
use crate::widget::{WidgetInput, WidgetInputField};
use robin_engine::sprite::BBox;

/// Maximum number of player profiles that can coexist on disk.
const MAX_PROFILES: usize = 10;

const LIST_RECT: MenuRect = MenuRect {
    x: 30,
    y: 72,
    w: 440,
    h: 340,
};

const ID_SELECT: u32 = 0;
const ID_NEW: u32 = 1;
const ID_RENAME: u32 = 2;
const ID_DELETE: u32 = 3;
const ID_CLOSE: u32 = 4;

/// Display the Select Player screen.  Mutates the global
/// [`PlayerProfileManager`] when the player confirms a selection,
/// creates, or deletes a profile.  Returns once the player closes the
/// dialog via Select/Escape — there's no outcome to carry back to
/// the caller; the active profile lives on the global manager.
pub(crate) async fn show_select_player(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    mut cursor: Option<ModalCursor<'_>>,
) {
    let sw = renderer.screen_width() as i32;
    let sh = renderer.screen_height() as i32;
    let transform = MenuTransform::centered(sw, sh);

    let (btn_w, btn_h) = resources.button_dimensions();

    let select_label = resources.menu_text.get(MT_BTN_SELECT);
    let new_label = resources.menu_text.get(MT_BTN_NEW);
    let rename_label = resources.menu_text.get(MT_BTN_RENAME);
    let delete_label = resources.menu_text.get(MT_BTN_DELETE);

    // Bottom-right button row, spacing=2.
    let bottom_labels: &[(&str, bool)] = &[
        (&select_label, true),
        (&new_label, true),
        (&rename_label, true),
        (&delete_label, true),
    ];
    let bottom_buttons = align_bottom_right(bottom_labels, btn_w, btn_h);
    let btn_positions: [(u32, &str, i32, i32); 4] = [
        (
            ID_SELECT,
            &select_label,
            bottom_buttons[0].x,
            bottom_buttons[0].y,
        ),
        (ID_NEW, &new_label, bottom_buttons[1].x, bottom_buttons[1].y),
        (
            ID_RENAME,
            &rename_label,
            bottom_buttons[2].x,
            bottom_buttons[2].y,
        ),
        (
            ID_DELETE,
            &delete_label,
            bottom_buttons[3].x,
            bottom_buttons[3].y,
        ),
    ];
    let (profile_field_w, profile_field_h) = resources.input_field_dimensions();

    // Track the highlighted row locally; `active_index` on the manager
    // only changes when the player commits via Select or double-click.
    let mut selected: Option<usize> = profiles_snapshot().and_then(|(_, active)| active);

    let mut input_state = ModalInputState::new();
    input_state.seed_mouse_from_sdl(event_pump, transform);

    let mut frame = crate::widget::FrameWnd::default();
    frame.enabled = true;
    frame.input_enabled = true;
    for (id, label, x, y) in &btn_positions {
        frame.add_widget_absolute(widget_bridge::make_button_enabled(
            *id, label, true, *x, *y, btn_w, btn_h,
        ));
    }

    loop {
        // Refresh the profile snapshot each frame: profile count and
        // the active-index can change when the player creates / deletes
        // entries. The button frame itself stays alive across frames so
        // mouse-down state is still present when the matching mouse-up
        // arrives.
        let (profiles, _active) = match profiles_snapshot() {
            Some(snap) => snap,
            None => {
                // No global manager — there's nothing sensible to show.
                tracing::warn!(
                    "Select Player: global PlayerProfileManager not initialised; closing"
                );
                return;
            }
        };
        // Button enablement is purely a function of the profile count.
        // Selection state is enforced inside the handlers (see
        // `selected.is_some()` guards below) rather than at the
        // button-arming layer.
        let has_profile = !profiles.is_empty();
        let can_select = has_profile;
        let can_new = profiles.len() < MAX_PROFILES;
        let can_rename = has_profile;
        let can_delete = has_profile;

        set_button_enabled(&mut frame, ID_SELECT, can_select);
        set_button_enabled(&mut frame, ID_NEW, can_new);
        set_button_enabled(&mut frame, ID_RENAME, can_rename);
        set_button_enabled(&mut frame, ID_DELETE, can_delete);

        // ── Events ──────────────────────────────────────────────
        let mut activated: Option<u32> = None;
        for event in event_pump.poll_events() {
            input_state.update_from_event(&event, transform);
            match event {
                GameEvent::Quit
                | GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => {
                    activated = Some(ID_CLOSE);
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Return,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::KpEnter,
                    ..
                } if can_select && selected.is_some() => {
                    activated = Some(ID_SELECT);
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Up,
                    ..
                } => {
                    selected = match selected {
                        Some(i) if i > 0 => Some(i - 1),
                        Some(_) => Some(0),
                        None if !profiles.is_empty() => Some(0),
                        None => None,
                    };
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Down,
                    ..
                } => {
                    selected = match selected {
                        Some(i) if i + 1 < profiles.len() => Some(i + 1),
                        Some(i) => Some(i),
                        None if !profiles.is_empty() => Some(0),
                        None => None,
                    };
                }
                GameEvent::MouseUp(x, y, 1) => {
                    let (vx, vy) = transform.from_screen(x, y);
                    if let Some(row) = profile_row_at(vx, vy, profile_field_w, profile_field_h)
                        && row < profiles.len()
                    {
                        selected = Some(row);
                    }
                }
                // A double-click on a profile row commits that profile
                // as active and closes the menu.  SDL3 reports the click
                // counter via the 4th tuple element of `MouseDown`.
                GameEvent::MouseDown(x, y, 1, clicks) if clicks >= 2 => {
                    let (vx, vy) = transform.from_screen(x, y);
                    if let Some(row) = profile_row_at(vx, vy, profile_field_w, profile_field_h)
                        && row < profiles.len()
                    {
                        selected = Some(row);
                        activated = Some(ID_SELECT);
                    }
                }
                _ => {}
            }
        }

        let widget_input = input_state.as_widget_input();
        let widget_events = frame.process_input(&widget_input);
        input_state.end_frame();
        if let Some(id) = widget_bridge::find_activated(&widget_events) {
            activated = Some(id);
        }

        if let Some(id) = activated {
            match id {
                ID_CLOSE => break,
                ID_SELECT => {
                    if let Some(idx) = selected {
                        commit_active(idx);
                    }
                    break;
                }
                ID_NEW if can_new => {
                    // New-player flow: name prompt + difficulty radios.
                    if let Some((name, difficulty)) = show_new_player_prompt(
                        event_pump,
                        renderer,
                        resources,
                        default_new_player_name(),
                        cursor.as_mut().map(|c| c.reborrow()),
                    )
                    .await
                    {
                        // Substitute the localised "Anonymous" string
                        // when the raw input is literally empty —
                        // whitespace names (e.g. "   ") pass through
                        // unchanged.
                        let final_name = if name.is_empty() {
                            resources.menu_text.get(MT_STR_ANONYMOUS)
                        } else {
                            name
                        };
                        let screen_dims = (
                            renderer.screen_width() as u32,
                            renderer.screen_height() as u32,
                        );
                        let idx = create_new_profile(final_name, difficulty, Some(screen_dims));
                        selected = idx;
                    }
                }
                ID_RENAME => {
                    // The original switches the selected input-field
                    // widget into inline edit mode; we render the list
                    // as flat rows, so we surface a small modal backed
                    // by the same text-input pipeline instead.
                    if let Some(idx) = selected
                        && let Some(profile) = profiles.get(idx)
                        && let Some(new_name) = show_rename_prompt(
                            event_pump,
                            renderer,
                            resources,
                            profile.name.clone(),
                            cursor.as_mut().map(|c| c.reborrow()),
                        )
                        .await
                    {
                        rename_profile(idx, new_name);
                    }
                }
                ID_DELETE => {
                    if let Some(idx) = selected {
                        let msg = resources.menu_text.get(MT_MSG_REALLY_DELETE_PLAYER);
                        if show_yesno(
                            event_pump,
                            renderer,
                            resources,
                            cursor.as_mut().map(|c| c.reborrow()),
                            &msg,
                        )
                        .await
                        {
                            delete_profile(idx);
                            // Clamp selection against the shrunken list.
                            let new_len = profile_count();
                            selected = if new_len == 0 {
                                None
                            } else {
                                Some(idx.min(new_len - 1))
                            };
                        }
                    }
                }
                _ => {}
            }
        }

        // ── Render ──────────────────────────────────────────────
        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);

        if let Some(bg) = resources.menu_bg[2] {
            draw_screen_background(renderer, &bg);
        }

        // Profile fields: ten input fields at `(30, 72)`, aligned
        // vertically with two pixels of spacing.
        for (i, profile) in profiles.iter().enumerate() {
            if i >= MAX_PROFILES {
                break;
            }
            let row_y = LIST_RECT.y + i as i32 * (profile_field_h + 2);
            let is_selected = selected == Some(i);
            if let Some(surf) = resources.input_field_surface(is_selected) {
                widget_bridge::draw_picture_surface_rect(
                    renderer,
                    transform,
                    surf,
                    LIST_RECT.x,
                    row_y,
                    profile_field_w,
                    profile_field_h,
                    0,
                    0,
                    profile_field_w,
                    profile_field_h,
                    true,
                );
            } else {
                draw_fallback_rect(
                    renderer,
                    transform,
                    &MenuRect {
                        x: LIST_RECT.x,
                        y: row_y,
                        w: profile_field_w,
                        h: profile_field_h,
                    },
                );
            }
            let Some(font) = resources.label_font() else {
                continue;
            };
            let label = format_profile_row(profile, resources);
            render_text_virt(
                renderer,
                font,
                transform,
                &label,
                LIST_RECT.x + 10,
                row_y + 10,
            );
        }

        widget_bridge::draw_frame_buttons(renderer, resources, transform, &frame);
        if let Some(cursor) = cursor.as_mut() {
            cursor.cursor.advance_animation();
            cursor.draw(renderer, transform, &input_state);
        }

        renderer.present();
        crate::window::sleep_ms(16).await;
    }
}

/// Take a cheap snapshot of the global profile manager.
///
/// Returns `(profiles, active_index)` cloned out of the global lock so
/// the event loop doesn't hold it while rendering.
fn profiles_snapshot() -> Option<(Vec<PlayerProfile>, Option<usize>)> {
    let guard = PlayerProfileManager::global();
    guard
        .as_ref()
        .map(|mgr| (mgr.profiles.clone(), mgr.active_index))
}

fn profile_row_at(vx: i32, vy: i32, field_w: i32, field_h: i32) -> Option<usize> {
    if vx < LIST_RECT.x || vx >= LIST_RECT.x + field_w || vy < LIST_RECT.y {
        return None;
    }
    let stride = field_h + 2;
    if stride <= 0 {
        return None;
    }
    let rel_y = vy - LIST_RECT.y;
    let row = rel_y / stride;
    if row < 0 || row as usize >= MAX_PROFILES || rel_y % stride >= field_h {
        return None;
    }
    Some(row as usize)
}

fn profile_count() -> usize {
    PlayerProfileManager::global()
        .as_ref()
        .map(|mgr| mgr.profile_count())
        .unwrap_or(0)
}

fn set_button_enabled(frame: &mut crate::widget::FrameWnd, id: u32, enabled: bool) {
    let Some(widget) = frame.widget_mut(id) else {
        panic!("Select Player: missing button widget {id}");
    };
    if widget.base().enabled != enabled {
        widget.set_enable(enabled);
    }
}

fn commit_active(idx: usize) {
    let mut guard = PlayerProfileManager::global();
    let Some(mgr) = guard.as_mut() else {
        panic!("Select Player commit: global PlayerProfileManager missing")
    };
    if idx < mgr.profile_count() {
        mgr.set_active(idx);
        if let Err(err) = mgr.save() {
            tracing::error!("Select Player: failed to persist active profile change: {err:#}");
        }
    }
}

/// Return a default profile name that doesn't collide with an existing
/// one ("Player", "Player 2", ...).  Pre-fills the "New player" dialog
/// so the user can accept it unchanged, and so the OK button is
/// immediately usable rather than relying on the empty→Anonymous
/// fallback.
fn default_new_player_name() -> String {
    let guard = PlayerProfileManager::global();
    let Some(mgr) = guard.as_ref() else {
        return "Player".to_string();
    };
    if !mgr.has_profile("Player") {
        return "Player".to_string();
    }
    let mut n = 2;
    loop {
        let candidate = format!("Player {n}");
        if !mgr.has_profile(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

fn create_new_profile(
    name: String,
    difficulty: DifficultyLevel,
    screen_dims: Option<(u32, u32)>,
) -> Option<usize> {
    let mut guard = PlayerProfileManager::global();
    let Some(mgr) = guard.as_mut() else {
        panic!("Select Player create: global PlayerProfileManager missing")
    };
    // Creation only inserts; it does not promote the new profile to
    // active and does not save.  The user must press Select (or
    // double-click) to commit and persist.  Pass the live window
    // dimensions so the new profile inherits them when no other profile
    // is active (the "screen open" arm of profile creation).
    let idx = mgr.create_profile_with_screen_dims(name, difficulty, screen_dims);
    Some(idx)
}

fn rename_profile(idx: usize, new_name: String) {
    let mut guard = PlayerProfileManager::global();
    let Some(mgr) = guard.as_mut() else {
        panic!("Select Player rename: global PlayerProfileManager missing")
    };
    if idx >= mgr.profile_count() {
        return;
    }
    // Trim the entered text, fall back to "Robin" when empty, then
    // unconditionally promote the renamed profile to active and save.
    // The "last-edited profile wins" side effect is part of the
    // contract — Select isn't required for rename to take effect on
    // the active slot.
    let trimmed = new_name.trim();
    let final_name = if trimmed.is_empty() { "Robin" } else { trimmed };
    mgr.profiles[idx].name = final_name.to_string();
    mgr.set_active(idx);
    if let Err(err) = mgr.save() {
        tracing::error!("Select Player: failed to persist rename: {err:#}");
    }
}

fn delete_profile(idx: usize) {
    let mut guard = PlayerProfileManager::global();
    let Some(mgr) = guard.as_mut() else {
        panic!("Select Player delete: global PlayerProfileManager missing")
    };
    if idx >= mgr.profile_count() {
        return;
    }
    // `delete_profile` itself wipes `<save_directory>/Profile_NNN`.
    mgr.delete_profile(idx);
    // Unconditionally promote index 0 to active whenever any profile
    // remains — regardless of whether the deleted one was the active
    // one.
    if mgr.profile_count() > 0 {
        mgr.set_active(0);
    }
    if let Err(err) = mgr.save() {
        tracing::error!("Select Player: failed to persist deletion: {err:#}");
    }
}

/// Format a profile row as `"<Name> / <Difficulty> / <Progression>%"`.
fn format_profile_row(profile: &PlayerProfile, resources: &IngameMenuResources) -> String {
    let marker = if profile.active { "> " } else { "  " };
    format!(
        "{marker}{name} — {difficulty} / {progression}%",
        name = profile.name,
        difficulty = difficulty_label(resources, profile.difficulty),
        progression = profile.progression,
    )
}

/// Returns the localised difficulty label via the menu-text table.
fn difficulty_label(resources: &IngameMenuResources, d: DifficultyLevel) -> String {
    let id = match d {
        DifficultyLevel::Easy => MT_STR_DIFFICULTY_EASY,
        DifficultyLevel::Medium => MT_STR_DIFFICULTY_MEDIUM,
        DifficultyLevel::Hard => MT_STR_DIFFICULTY_HARD,
    };
    resources.menu_text.get(id)
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

fn point_in_rect(px: i32, py: i32, x: i32, y: i32, w: i32, h: i32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

// ═══════════════════════════════════════════════════════════════════
// Name-entry modal (shared between New Player and Rename)
// ═══════════════════════════════════════════════════════════════════

/// New-player window geometry: 496×463.
const NEW_PLAYER_PROMPT_W: i32 = 496;
const NEW_PLAYER_PROMPT_H: i32 = 463;
const RENAME_PROMPT_W: i32 = 420;
const RENAME_PROMPT_H: i32 = 220;
const RENAME_PROMPT_INPUT_W: i32 = 340;
const RENAME_PROMPT_INPUT_H: i32 = 28;

/// Display a name-entry modal pre-filled with `initial`.
///
/// Returns `Some(name)` on OK / Enter (trimmed; the caller decides what
/// to do with an empty string), `None` on Cancel / Escape.
pub(crate) async fn show_rename_prompt(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    initial: String,
    cursor: Option<ModalCursor<'_>>,
) -> Option<String> {
    run_name_prompt(
        event_pump,
        renderer,
        resources,
        &resources.menu_text.get(MT_BTN_RENAME),
        initial,
        None,
        cursor,
    )
    .await
    .map(|(name, _diff)| name)
}

/// Display the new-player modal with a name input and a difficulty
/// radio row, pre-filled with `initial_name` and Medium.
pub(crate) async fn show_new_player_prompt(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    initial_name: String,
    cursor: Option<ModalCursor<'_>>,
) -> Option<(String, DifficultyLevel)> {
    run_name_prompt(
        event_pump,
        renderer,
        resources,
        &resources.menu_text.get(MT_TTL_NEW_PLAYER),
        initial_name,
        Some(DifficultyLevel::Medium),
        cursor,
    )
    .await
}

const PROMPT_ID_OK: u32 = 0;
const PROMPT_ID_CANCEL: u32 = 1;
const PROMPT_ID_DIFF_EASY: u32 = 10;
const PROMPT_ID_DIFF_MEDIUM: u32 = 11;
const PROMPT_ID_DIFF_HARD: u32 = 12;

async fn run_name_prompt(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    title: &str,
    initial: String,
    initial_difficulty: Option<DifficultyLevel>,
    mut cursor: Option<ModalCursor<'_>>,
) -> Option<(String, DifficultyLevel)> {
    let sw = renderer.screen_width() as i32;
    let sh = renderer.screen_height() as i32;
    let transform = MenuTransform::centered(sw, sh);

    let is_new_player = initial_difficulty.is_some();
    let (win_w, win_h) = if is_new_player {
        (NEW_PLAYER_PROMPT_W, NEW_PLAYER_PROMPT_H)
    } else {
        (RENAME_PROMPT_W, RENAME_PROMPT_H)
    };
    let win_x = (MENU_W - win_w) / 2;
    let win_y = (MENU_H - win_h) / 2;

    let ok_label = resources.menu_text.get(MT_BTN_OK);
    let cancel_label = resources.menu_text.get(MT_BTN_CANCEL);

    let (input_x, input_y, input_w, input_h) = if is_new_player {
        let (field_w, field_h) = resources.input_field_dimensions();
        (win_x + 35, win_y + 130, field_w, field_h)
    } else {
        (
            win_x + (win_w - RENAME_PROMPT_INPUT_W) / 2,
            win_y + 55,
            RENAME_PROMPT_INPUT_W,
            RENAME_PROMPT_INPUT_H,
        )
    };

    let diff_easy_label = resources.menu_text.get(MT_STR_DIFFICULTY_EASY);
    let diff_medium_label = resources.menu_text.get(MT_STR_DIFFICULTY_MEDIUM);
    let diff_hard_label = resources.menu_text.get(MT_STR_DIFFICULTY_HARD);
    let diff_labels: [(u32, &str, DifficultyLevel); 3] = [
        (PROMPT_ID_DIFF_EASY, &diff_easy_label, DifficultyLevel::Easy),
        (
            PROMPT_ID_DIFF_MEDIUM,
            &diff_medium_label,
            DifficultyLevel::Medium,
        ),
        (PROMPT_ID_DIFF_HARD, &diff_hard_label, DifficultyLevel::Hard),
    ];
    let (diff_btn_w, diff_btn_h) = resources.radio_dimensions();
    let diff_btn_gap = 25;
    let diff_total_w = 3 * diff_btn_w + 2 * diff_btn_gap;
    let diff_row_x = win_x + (win_w - diff_total_w) / 2;
    let diff_row_y = win_y + 250;

    let (ok_w, ok_h) = resources.ok_button_dimensions();
    let (cancel_w, cancel_h) = resources.cancel_button_dimensions();
    let ok_cancel_gap = if is_new_player { 20 } else { 18 };
    let confirm_total_w = ok_w + cancel_w + ok_cancel_gap;
    let confirm_row_x = win_x + (win_w - confirm_total_w) / 2;
    let confirm_row_y = if is_new_player {
        win_y + 370
    } else {
        win_y + win_h - ok_h.max(cancel_h) - 14
    };

    // Name-entry state lives on a `WidgetInputField` kept in
    // `SelectedEditable`.  The pre-fill is trimmed to the max length
    // so an oversized initial doesn't prevent editing; caret sits at
    // the end so the user types after the existing name.
    const PROMPT_ID_INPUT: u32 = 100;
    let trimmed_initial: String = initial.chars().take(MAX_PLAYER_NAME_LENGTH).collect();
    let mut input_widget = WidgetInputField::new(PROMPT_ID_INPUT);
    input_widget.set_max_length(MAX_PLAYER_NAME_LENGTH);
    input_widget.set_text(&trimmed_initial);
    input_widget.enter_edit_mode();
    let mut caret_timer: u32 = 0;
    let mut difficulty = initial_difficulty.unwrap_or(DifficultyLevel::Medium);
    let mut input_state = ModalInputState::new();
    input_state.seed_mouse_from_sdl(event_pump, transform);
    let empty_keyboard = crate::ui::UiKeyboard::default();

    crate::window::start_text_input();
    let outcome = loop {
        // Build the widget frame each iteration — difficulty selection
        // is reflected on the radio buttons via their enabled-but-pressed
        // style (the "selected" sub-picture).  The OK and Cancel buttons
        // sit below.
        let mut frame = crate::widget::FrameWnd::default();
        frame.enabled = true;
        frame.input_enabled = true;

        // OK / Cancel buttons.
        // OK is not gated on non-empty input; it is always clickable,
        // and the empty-input case is replaced with the "Anonymous"
        // string at the call site.
        frame.add_widget_absolute(widget_bridge::make_button_with_resource(
            PROMPT_ID_OK,
            if is_new_player { "" } else { &ok_label },
            true,
            resource_ids::RHID_OK,
            confirm_row_x,
            confirm_row_y,
            ok_w,
            ok_h,
        ));
        frame.add_widget_absolute(widget_bridge::make_button_with_resource(
            PROMPT_ID_CANCEL,
            if is_new_player { "" } else { &cancel_label },
            true,
            resource_ids::RHID_CANCEL,
            confirm_row_x + ok_w + ok_cancel_gap,
            confirm_row_y,
            cancel_w,
            cancel_h,
        ));

        // Difficulty radios (only when this modal is the "New Player" variant).
        if initial_difficulty.is_some() {
            for (i, (id, _label, level)) in diff_labels.iter().enumerate() {
                let x = diff_row_x + i as i32 * (diff_btn_w + diff_btn_gap);
                let mut widget = widget_bridge::make_button_with_resource(
                    *id,
                    "",
                    true,
                    resource_ids::RHID_RADIO,
                    x,
                    diff_row_y,
                    diff_btn_w,
                    diff_btn_h,
                );
                if *level == difficulty
                    && let crate::widget::Widget::Button(button) = &mut widget
                {
                    let _ = button.set_group_selected(true);
                }
                frame.add_widget_absolute(widget);
            }
        }

        // ── Events ──────────────────────────────────────────────
        let mut activated: Option<u32> = None;
        let mut confirmed = false;
        let mut cancelled = false;
        for event in event_pump.poll_events() {
            input_state.update_from_event(&event, transform);
            match event {
                GameEvent::Quit
                | GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => {
                    cancelled = true;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Return,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::KpEnter,
                    ..
                } => {
                    confirmed = true;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Backspace,
                    ..
                } => {
                    input_widget.backspace();
                    caret_timer = 0;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Delete,
                    ..
                } => {
                    input_widget.delete_char();
                    caret_timer = 0;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Left,
                    ..
                } => {
                    input_widget.move_caret_left();
                    caret_timer = 0;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Right,
                    ..
                } => {
                    input_widget.move_caret_right();
                    caret_timer = 0;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Home,
                    ..
                } => {
                    input_widget.move_caret_home();
                    caret_timer = 0;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::End,
                    ..
                } => {
                    input_widget.move_caret_end();
                    caret_timer = 0;
                }
                GameEvent::TextInput { .. } => {
                    // Text input flows through the widget below via
                    // `ModalInputState::as_widget_input().text_input`;
                    // reset the caret blink so the insertion is visible.
                    caret_timer = 0;
                }
                GameEvent::MouseUp(x, y, 1) => {
                    let (vx, vy) = transform.from_screen(x, y);
                    if point_in_rect(vx, vy, confirm_row_x, confirm_row_y, ok_w, ok_h) {
                        confirmed = true;
                    } else if point_in_rect(
                        vx,
                        vy,
                        confirm_row_x + ok_w + ok_cancel_gap,
                        confirm_row_y,
                        cancel_w,
                        cancel_h,
                    ) {
                        cancelled = true;
                    } else if initial_difficulty.is_some() {
                        for (i, (_id, _label, level)) in diff_labels.iter().enumerate() {
                            let radio_x = diff_row_x + i as i32 * (diff_btn_w + diff_btn_gap);
                            if point_in_rect(
                                vx,
                                vy,
                                radio_x - 8,
                                diff_row_y - 8,
                                diff_btn_w + 16,
                                diff_btn_h + 38,
                            ) {
                                difficulty = *level;
                                break;
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        let widget_input = input_state.as_widget_input();
        let widget_events = frame.process_input(&widget_input);
        // Feed the accumulated text-input stream to the widget so its
        // caret-aware insert path handles max-length + control-char
        // filtering. Stub keyboard keeps its special-key branches silent
        // (Backspace/caret nav handled above at the modal level to
        // avoid double-firing on key release).
        let field_input = WidgetInput {
            mouse_position: widget_input.mouse_position,
            mouse_z: widget_input.mouse_z,
            mouse_button: crate::ui::MouseButtons::empty(),
            keyboard: &empty_keyboard,
            text_input: widget_input.text_input,
            capture: None,
        };
        let _field_events = input_widget.process_input(&field_input);
        if input_widget.base.state != crate::ui::UiState::SelectedEditable {
            input_widget.enter_edit_mode();
        }
        input_state.end_frame();
        if let Some(id) = widget_bridge::find_activated(&widget_events) {
            activated = Some(id);
        }

        if let Some(id) = activated {
            match id {
                PROMPT_ID_OK => confirmed = true,
                PROMPT_ID_CANCEL => cancelled = true,
                PROMPT_ID_DIFF_EASY => difficulty = DifficultyLevel::Easy,
                PROMPT_ID_DIFF_MEDIUM => difficulty = DifficultyLevel::Medium,
                PROMPT_ID_DIFF_HARD => difficulty = DifficultyLevel::Hard,
                _ => {}
            }
        }

        if confirmed {
            // Return the raw input; only a *literal* empty string gets
            // replaced with the "Anonymous" placeholder, and whitespace
            // passes through unchanged. Don't trim here — the caller
            // decides on the empty→Anonymous substitution.
            break Some((input_widget.edit_text.clone(), difficulty));
        }
        if cancelled {
            break None;
        }

        // ── Render ──────────────────────────────────────────────
        enter_modal_gpu_phase(renderer);
        if !is_new_player {
            dim_screen(renderer);
        }

        let bg = if is_new_player {
            resources.parchment_huge
        } else {
            resources.menu_bg_small
        };
        if let Some(bg) = bg {
            draw_background(renderer, transform, &bg, win_x, win_y, win_w, win_h);
        } else {
            draw_fallback_rect(
                renderer,
                transform,
                &MenuRect {
                    x: win_x,
                    y: win_y,
                    w: win_w,
                    h: win_h,
                },
            );
        }

        if let Some(font) = resources.title_font() {
            let tw = font.text_width(title);
            render_text_virt(
                renderer,
                font,
                transform,
                title,
                win_x + (win_w - tw) / 2,
                if is_new_player {
                    win_y + 45
                } else {
                    win_y + 18
                },
            );
        }

        if is_new_player && let Some(font) = resources.popup_font() {
            let name = resources.menu_text.get(MT_STR_NAME);
            let difficulty = resources.menu_text.get(MT_STR_DIFFICULTY_LEVEL);
            let name_w = font.text_width(&name);
            let difficulty_w = font.text_width(&difficulty);
            render_text_virt(
                renderer,
                font,
                transform,
                &name,
                win_x + (win_w - name_w) / 2,
                win_y + 100,
            );
            render_text_virt(
                renderer,
                font,
                transform,
                &difficulty,
                win_x + (win_w - difficulty_w) / 2,
                win_y + 200,
            );
        }

        // Input field background + editable text.
        let input_rect = MenuRect {
            x: input_x,
            y: input_y,
            w: input_w,
            h: input_h,
        };
        if let Some(surf) = resources.input_field_selected_surface() {
            let (x, y) = transform.to_screen(input_rect.x, input_rect.y);
            let src = BBox::new(
                geo2d::pt(0.0, 0.0),
                geo2d::pt(input_rect.w as f32, input_rect.h as f32),
            );
            let dst = BBox::new(
                geo2d::pt(x as f32, y as f32),
                geo2d::pt((x + input_rect.w) as f32, (y + input_rect.h) as f32),
            );
            renderer.blit_to_screen(
                surf,
                Some(&src),
                Some(&dst),
                crate::renderer::BLIT_SOURCE_TRANSPARENT,
            );
        } else {
            draw_fallback_rect(renderer, transform, &input_rect);
        }

        // `caret_timer` ticks every frame (~60 Hz); toggle caret every
        // ~500 ms (matches the save/load picker's timing). The caret
        // is inserted at its char offset inside the buffer — widget
        // `caret_offset` tracks chars, so we split on char boundaries.
        let show_caret = (caret_timer / 30).is_multiple_of(2);
        let display = if show_caret {
            let text = &input_widget.edit_text;
            let byte_idx = text
                .char_indices()
                .nth(input_widget.caret_offset)
                .map(|(b, _)| b)
                .unwrap_or(text.len());
            let (head, tail) = text.split_at(byte_idx);
            format!("{head}|{tail}")
        } else {
            input_widget.edit_text.clone()
        };
        if let Some(font) = resources.edit_field_font() {
            render_text_virt(
                renderer,
                font,
                transform,
                &display,
                input_rect.x + 8,
                input_rect.y + 6,
            );
        }

        widget_bridge::draw_frame_buttons(renderer, resources, transform, &frame);
        if is_new_player && let Some(font) = resources.popup_font() {
            let label_box_w = 100;
            let label_y = diff_row_y + diff_btn_h + 5;
            for (i, (_id, label, _level)) in diff_labels.iter().enumerate() {
                let radio_x = diff_row_x + i as i32 * (diff_btn_w + diff_btn_gap);
                let box_x = radio_x + (diff_btn_w - label_box_w) / 2;
                let text_w = font.text_width(label);
                render_text_virt(
                    renderer,
                    font,
                    transform,
                    label,
                    box_x + (label_box_w - text_w) / 2,
                    label_y,
                );
            }
        }
        if let Some(cursor) = cursor.as_mut() {
            cursor.cursor.advance_animation();
            cursor.draw(renderer, transform, &input_state);
        }

        renderer.present();
        caret_timer = caret_timer.wrapping_add(1);
        crate::window::sleep_ms(16).await;
    };
    crate::window::stop_text_input();
    outcome
}
