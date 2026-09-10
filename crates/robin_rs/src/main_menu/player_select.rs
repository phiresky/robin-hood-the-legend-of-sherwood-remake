//! Main-menu "Select Player" screen.
//!
//! Shows the application-owned [`PlayerProfileManager`] roster, lets the player
//! pick an entry to set as active, create a new profile (name +
//! difficulty), delete, or rename via a small modal backed by the winit
//! text-input pipeline (IME composition, dead keys, non-ASCII
//! keyboards).

use super::difficulty_to_string;
use crate::gfx_types::Keycode;
use robin_engine::sprite::BBox;

use crate::gfx_types::GameEvent;
use crate::host::ApplicationContext;
use crate::ingame_menu::layout::{
    MENU_H, MENU_W, MenuRect, MenuTransform, align_bottom_right, dim_screen, draw_background,
    draw_fallback_panel, draw_screen_background, enter_modal_gpu_phase, render_text_virt_font,
};
use crate::ingame_menu::resources::{
    IngameMenuResources, MT_BTN_CANCEL, MT_BTN_DELETE, MT_BTN_NEW, MT_BTN_OK, MT_BTN_RENAME,
    MT_BTN_SELECT, MT_INFOBULLE_BUTTON_NO, MT_INFOBULLE_BUTTON_YES, MT_MSG_REALLY_DELETE_PLAYER,
    MT_PORT_BTN_DIFFICULTY, MT_PORT_DIFF_AUTO_HEAL, MT_PORT_DIFF_BLIP_RANGE, MT_PORT_DIFF_CARNAGE,
    MT_PORT_DIFF_ENEMY_FIGHTING, MT_PORT_DIFF_ENEMY_HEALTH, MT_PORT_DIFF_ENEMY_IQ,
    MT_PORT_DIFF_ENEMY_SHOOTING, MT_PORT_DIFF_FRIENDLY_FIRE, MT_PORT_DIFF_GUARD_NOISE_SENSITIVITY,
    MT_PORT_DIFF_GUARD_VIEW_ANGLE, MT_PORT_DIFF_GUARD_VIEW_DISTANCE, MT_PORT_DIFF_LARGE_CAPACITY,
    MT_PORT_DIFF_LEGACY_SCRIPT, MT_PORT_DIFF_NET_PREVIEW, MT_PORT_DIFF_PUNCH,
    MT_PORT_DIFF_REACTION_TIME, MT_PORT_DIFF_SMALL_CAPACITY, MT_PORT_DIFF_SPECIAL_DELAY,
    MT_PORT_STR_DIFFICULTY_CUSTOM, MT_PORT_STR_DIFFICULTY_HELP, MT_PORT_STR_DIFFICULTY_LEGENDARY,
    MT_PORT_TTL_ADVANCED_DIFFICULTY, MT_STR_ANONYMOUS, MT_STR_DIFFICULTY_EASY,
    MT_STR_DIFFICULTY_HARD, MT_STR_DIFFICULTY_LEVEL, MT_STR_DIFFICULTY_MEDIUM, MT_STR_NAME,
    MT_TTL_NEW_PLAYER,
};
use crate::ingame_menu::widget_bridge::{self, ModalCursor, ModalInputState};
use crate::ingame_menu::yesno::show_yesno;
use crate::renderer::Renderer;
use crate::ui::{MouseButtons, UiKeyboard, UiState};
use crate::ui_screens::MAX_PLAYER_NAME_LENGTH;
use crate::widget::{FrameWnd, Widget, WidgetInput, WidgetInputField};
use robin_engine::player_profile::{
    DifficultyLevel, DifficultyRules, LegacyDifficultyLevel, PlayerProfile,
};
use robin_engine::resource_ids;
use serde::{Deserialize, Serialize};

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
const ID_DIFFICULTY: u32 = 5;

/// Display the Select Player screen. Mutates the application-owned profile
/// manager when the player confirms a selection,
/// creates, or deletes a profile.  Returns once the player closes the
/// dialog via Select/Escape — there's no outcome to carry back to
/// the caller; the active profile lives on `application_context`.
pub(crate) async fn show_select_player(
    application_context: &ApplicationContext,
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
    let difficulty_button_label = resources.menu_text.get_port(MT_PORT_BTN_DIFFICULTY);

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
    let mut selected: Option<usize> = profiles_snapshot(application_context).1;
    let mut status = String::new();

    let mut input_state = ModalInputState::new();
    input_state.seed_mouse_from_window(event_pump, transform);

    let mut frame = FrameWnd::default();
    frame.enabled = true;
    frame.input_enabled = true;
    for (id, label, x, y) in &btn_positions {
        frame.add_widget_absolute(widget_bridge::make_button_enabled(
            *id, label, true, *x, *y, btn_w, btn_h,
        ));
    }
    // Difficulty editing is intentionally exposed only from this
    // between-mission profile screen. It cannot mutate a running simulation.
    frame.add_widget_absolute(widget_bridge::make_button_enabled(
        ID_DIFFICULTY,
        difficulty_button_label,
        true,
        LIST_RECT.x,
        418,
        btn_w,
        btn_h,
    ));

    loop {
        // Refresh the profile snapshot each frame: profile count and
        // the active-index can change when the player creates / deletes
        // entries. The button frame itself stays alive across frames so
        // mouse-down state is still present when the matching mouse-up
        // arrives.
        let (profiles, _active) = profiles_snapshot(application_context);
        // Button enablement is purely a function of the profile count.
        // Selection state is enforced inside the handlers (see
        // `selected.is_some()` guards below) rather than at the
        // button-arming layer.
        let has_profile = !profiles.is_empty();
        let can_select = has_profile;
        let can_new = profiles.len() < MAX_PROFILES;
        let can_rename = has_profile;
        // A completed application context must always have a final active
        // profile before menus, saves, or sessions can continue.
        let can_delete = can_delete_profile(profiles.len());

        set_button_enabled(&mut frame, ID_SELECT, can_select);
        set_button_enabled(&mut frame, ID_NEW, can_new);
        set_button_enabled(&mut frame, ID_RENAME, can_rename);
        set_button_enabled(&mut frame, ID_DELETE, can_delete);
        set_button_enabled(&mut frame, ID_DIFFICULTY, has_profile);

        // ── Events ──────────────────────────────────────────────
        let mut activated: Option<u32> = None;
        let (events, transform) =
            crate::ingame_menu::layout::poll_events_with_transform(event_pump, renderer);
        for event in events {
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
                // as active and closes the menu. The window layer reports the click
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
                        commit_active(application_context, idx);
                    }
                    break;
                }
                ID_NEW if can_new => {
                    // New-player flow: name prompt + difficulty radios.
                    if let Some((name, mut difficulty)) = show_new_player_prompt(
                        event_pump,
                        renderer,
                        resources,
                        default_new_player_name(application_context),
                        cursor.as_mut().map(|c| c.reborrow()),
                    )
                    .await
                    {
                        if matches!(difficulty, DifficultyLevel::Custom(_)) {
                            let Some(advanced) = show_difficulty_prompt(
                                event_pump,
                                renderer,
                                resources,
                                difficulty,
                                cursor.as_mut().map(|c| c.reborrow()),
                            )
                            .await
                            else {
                                continue;
                            };
                            difficulty = advanced;
                        }
                        // Substitute the localised "Anonymous" string
                        // when the raw input is literally empty —
                        // whitespace names (e.g. "   ") pass through
                        // unchanged.
                        let final_name = if name.is_empty() {
                            resources.menu_text.get(MT_STR_ANONYMOUS)
                        } else {
                            name
                        };
                        // Persist the selected 4:3 scale reference, not the
                        // transient aspect-adapted canvas dimensions.
                        let screen_dims = application_context
                            .with_active_profile(|profile| {
                                (
                                    profile.graphic_config.resolution_x.round() as u32,
                                    profile.graphic_config.resolution_y.round() as u32,
                                )
                            })
                            .unwrap_or_else(|error| {
                                panic!("new-player flow lost the active profile: {error}")
                            });
                        let idx = create_new_profile(
                            application_context,
                            final_name,
                            difficulty,
                            Some(screen_dims),
                        );
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
                        rename_profile(application_context, idx, new_name);
                    }
                }
                ID_DIFFICULTY => {
                    if let Some(idx) = selected
                        && let Some(profile) = profiles.get(idx)
                        && let Some(difficulty) = show_difficulty_prompt(
                            event_pump,
                            renderer,
                            resources,
                            profile.difficulty,
                            cursor.as_mut().map(|c| c.reborrow()),
                        )
                        .await
                    {
                        set_profile_difficulty(application_context, idx, difficulty);
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
                            match delete_profile(application_context, idx) {
                                Ok(true) => {
                                    status.clear();
                                    // Clamp selection against the shrunken list.
                                    let new_len = profile_count(application_context);
                                    selected = if new_len == 0 {
                                        None
                                    } else {
                                        Some(idx.min(new_len - 1))
                                    };
                                }
                                Ok(false) => {}
                                Err(error) => {
                                    status = format!("Player deletion failed: {error}");
                                }
                            }
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
                widget_bridge::draw_menu_surface_rect(
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
                draw_fallback_panel(
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
            let Some(font) = resources.label_font_any() else {
                continue;
            };
            let label = format_profile_row(profile, resources);
            render_text_virt_font(
                renderer,
                font,
                transform,
                &label,
                LIST_RECT.x + 10,
                row_y + 10,
            );
        }

        if !status.is_empty()
            && let Some(font) = resources.label_font_any()
        {
            render_text_virt_font(renderer, font, transform, &status, 30, 420);
        }

        widget_bridge::draw_frame_buttons(renderer, resources, transform, &frame);
        if let Some(cursor) = cursor.as_mut() {
            cursor.cursor.advance_ui_animation();
            cursor.draw(renderer, transform, &input_state);
        }

        renderer.present();
        crate::window::sleep_ui_frame().await;
    }
}

/// Only the profile fields needed by the roster and its edit dialogs.
/// Campaign history and configuration stay with the application owner.
#[derive(Debug, Serialize, Deserialize)]
struct PlayerProfileRow {
    name: String,
    difficulty: DifficultyLevel,
    progression: u32,
    active: bool,
}

impl From<&PlayerProfile> for PlayerProfileRow {
    fn from(profile: &PlayerProfile) -> Self {
        Self {
            name: profile.name.clone(),
            difficulty: profile.difficulty,
            progression: profile.progression,
            active: profile.active,
        }
    }
}

/// Project the roster under the context lock; rendering and dialogs use only
/// these owned rows after the lock is released.
fn profiles_snapshot(
    application_context: &ApplicationContext,
) -> (Vec<PlayerProfileRow>, Option<usize>) {
    application_context
        .with_player_profiles(|mgr| {
            (
                mgr.profiles.iter().map(PlayerProfileRow::from).collect(),
                mgr.active_index,
            )
        })
        .unwrap_or_else(|error| panic!("Select Player lost its ApplicationContext: {error}"))
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

fn profile_count(application_context: &ApplicationContext) -> usize {
    application_context
        .with_player_profiles(|mgr| mgr.profile_count())
        .unwrap_or_else(|error| panic!("Select Player lost its ApplicationContext: {error}"))
}

fn can_delete_profile(profile_count: usize) -> bool {
    profile_count > 1
}

fn set_button_enabled(frame: &mut crate::widget::FrameWnd, id: u32, enabled: bool) {
    let Some(widget) = frame.widget_mut(id) else {
        panic!("Select Player: missing button widget {id}");
    };
    if widget.base().enabled != enabled {
        widget.set_enable(enabled);
    }
}

fn commit_active(application_context: &ApplicationContext, idx: usize) {
    let profile_id = application_context
        .update_and_retain_player_profiles(|mgr| {
            if idx < mgr.profile_count() {
                mgr.set_active(idx);
                return Some(mgr.profiles[idx].id);
            }
            None
        })
        .unwrap_or_else(|error| panic!("Select Player commit failed: {error}"))
        .log_persistence_error("Select Player: failed to persist active profile change");
    if let Some(profile_id) = profile_id {
        application_context
            .with_key_configs_mut(|store| {
                store.entry_or_default(profile_id);
            })
            .unwrap_or_else(|error| panic!("Select Player key setup failed: {error}"));
    }
}

/// Return a default profile name that doesn't collide with an existing
/// one ("Player", "Player 2", ...).  Pre-fills the "New player" dialog
/// so the user can accept it unchanged, and so the OK button is
/// immediately usable rather than relying on the empty→Anonymous
/// fallback.
fn default_new_player_name(application_context: &ApplicationContext) -> String {
    application_context
        .with_player_profiles(|mgr| {
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
        })
        .unwrap_or_else(|error| panic!("Select Player name generation failed: {error}"))
}

fn create_new_profile(
    application_context: &ApplicationContext,
    name: String,
    difficulty: DifficultyLevel,
    screen_dims: Option<(u32, u32)>,
) -> Option<usize> {
    // Creation only inserts; it does not promote the new profile to
    // active and does not save.  The user must press Select (or
    // double-click) to commit and persist.  Pass the live window
    // dimensions so the new profile inherits them when no other profile
    // is active (the "screen open" arm of profile creation).
    let (idx, profile_id) = application_context
        .with_player_profiles_mut(|mgr| {
            let idx = mgr.create_profile_with_screen_dims(name, difficulty, screen_dims);
            (idx, mgr.profiles[idx].id)
        })
        .unwrap_or_else(|error| panic!("Select Player create failed: {error}"));
    application_context
        .with_key_configs_mut(|store| {
            store.configs.insert(
                profile_id,
                crate::key_config_store::ProfileKeyConfig::fresh(),
            );
        })
        .unwrap_or_else(|error| panic!("Select Player key setup failed: {error}"));
    Some(idx)
}

fn rename_profile(application_context: &ApplicationContext, idx: usize, new_name: String) {
    // Trim the entered text, fall back to "Robin" when empty, then
    // unconditionally promote the renamed profile to active and save.
    // The "last-edited profile wins" side effect is part of the
    // contract — Select isn't required for rename to take effect on
    // the active slot.
    let trimmed = new_name.trim();
    let final_name = if trimmed.is_empty() { "Robin" } else { trimmed };
    application_context
        .update_and_retain_player_profiles(|mgr| {
            if idx >= mgr.profile_count() {
                return;
            }
            mgr.profiles[idx].name = final_name.to_string();
            mgr.set_active(idx);
        })
        .unwrap_or_else(|error| panic!("Select Player rename failed: {error}"))
        .log_persistence_error("Select Player: failed to persist rename");
}

fn delete_profile(application_context: &ApplicationContext, idx: usize) -> Result<bool, String> {
    application_context.delete_player_profile(idx)
}

/// Format a profile row as `"<Name> / <Difficulty> / <Progression>%"`.
fn format_profile_row(profile: &PlayerProfileRow, resources: &IngameMenuResources) -> String {
    let marker = if profile.active { "> " } else { "  " };
    format!(
        "{marker}{name} — {difficulty} / {progression}%",
        name = profile.name,
        difficulty = difficulty_to_string(&resources.menu_text, profile.difficulty),
        progression = profile.progression,
    )
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
const DIFFICULTY_LABEL_W: i32 = 112;
const DIFFICULTY_COLUMN_W: i32 = 120;
const DIFFICULTY_TOP_ROW_Y: i32 = 225;
const DIFFICULTY_BOTTOM_ROW_Y: i32 = 300;
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
const PROMPT_ID_DIFF_LEGENDARY: u32 = 13;
const PROMPT_ID_DIFF_CUSTOM: u32 = 14;

/// Keep the original Easy/Medium/Hard row intact and place the two port-added
/// modes on a second row. Five labels in the original single-row width overlap
/// in the localized popup font.
fn new_player_difficulty_positions(win_x: i32, win_y: i32, button_w: i32) -> [(i32, i32); 5] {
    let row_positions = |count: i32, y: i32| {
        let row_x = win_x + (NEW_PLAYER_PROMPT_W - count * DIFFICULTY_COLUMN_W) / 2;
        (0..count)
            .map(|column| {
                (
                    row_x + column * DIFFICULTY_COLUMN_W + (DIFFICULTY_COLUMN_W - button_w) / 2,
                    win_y + y,
                )
            })
            .collect::<Vec<_>>()
    };

    let top = row_positions(3, DIFFICULTY_TOP_ROW_Y);
    let bottom = row_positions(2, DIFFICULTY_BOTTOM_ROW_Y);
    [top[0], top[1], top[2], bottom[0], bottom[1]]
}

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
    let diff_legendary_label = resources
        .menu_text
        .get_port(MT_PORT_STR_DIFFICULTY_LEGENDARY);
    let diff_custom_label = resources.menu_text.get_port(MT_PORT_STR_DIFFICULTY_CUSTOM);
    let diff_labels: [(u32, &str, DifficultyLevel); 5] = [
        (PROMPT_ID_DIFF_EASY, &diff_easy_label, DifficultyLevel::Easy),
        (
            PROMPT_ID_DIFF_MEDIUM,
            &diff_medium_label,
            DifficultyLevel::Medium,
        ),
        (PROMPT_ID_DIFF_HARD, &diff_hard_label, DifficultyLevel::Hard),
        (
            PROMPT_ID_DIFF_LEGENDARY,
            diff_legendary_label,
            DifficultyLevel::Legendary,
        ),
        (
            PROMPT_ID_DIFF_CUSTOM,
            diff_custom_label,
            DifficultyLevel::Custom(DifficultyRules::MEDIUM),
        ),
    ];
    let (diff_btn_w, diff_btn_h) = resources.radio_dimensions();
    let diff_positions = new_player_difficulty_positions(win_x, win_y, diff_btn_w);

    let (ok_w, ok_h) = resources.ok_button_dimensions();
    let (cancel_w, cancel_h) = resources.cancel_button_dimensions();
    let ok_cancel_gap = if is_new_player { 20 } else { 18 };
    let confirm_total_w = ok_w + cancel_w + ok_cancel_gap;
    let confirm_row_x = win_x + (win_w - confirm_total_w) / 2;
    let confirm_row_y = if is_new_player {
        win_y + 390
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
    let mut caret_started_at_ms = crate::window::process_uptime_ms();
    let mut difficulty = initial_difficulty.unwrap_or(DifficultyLevel::Medium);
    let mut input_state = ModalInputState::new();
    input_state.seed_mouse_from_window(event_pump, transform);
    let empty_keyboard = UiKeyboard::default();

    crate::window::start_text_input();
    let outcome = loop {
        // Build the widget frame each iteration — difficulty selection
        // is reflected on the radio buttons via their enabled-but-pressed
        // style (the "selected" sub-picture).  The OK and Cancel buttons
        // sit below.
        let mut frame = FrameWnd::default();
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
            for ((id, _label, level), (x, y)) in diff_labels.iter().zip(diff_positions) {
                let mut widget = widget_bridge::make_button_with_resource(
                    *id,
                    "",
                    true,
                    resource_ids::RHID_RADIO,
                    x,
                    y,
                    diff_btn_w,
                    diff_btn_h,
                );
                if *level == difficulty
                    && let Widget::Button(button) = &mut widget
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
        let (events, transform) =
            crate::ingame_menu::layout::poll_events_with_transform(event_pump, renderer);
        for event in events {
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
                    caret_started_at_ms = crate::window::process_uptime_ms();
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Delete,
                    ..
                } => {
                    input_widget.delete_char();
                    caret_started_at_ms = crate::window::process_uptime_ms();
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Left,
                    ..
                } => {
                    input_widget.move_caret_left();
                    caret_started_at_ms = crate::window::process_uptime_ms();
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Right,
                    ..
                } => {
                    input_widget.move_caret_right();
                    caret_started_at_ms = crate::window::process_uptime_ms();
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Home,
                    ..
                } => {
                    input_widget.move_caret_home();
                    caret_started_at_ms = crate::window::process_uptime_ms();
                }
                GameEvent::KeyDown {
                    keycode: Keycode::End,
                    ..
                } => {
                    input_widget.move_caret_end();
                    caret_started_at_ms = crate::window::process_uptime_ms();
                }
                GameEvent::TextInput { .. } => {
                    // Text input flows through the widget below via
                    // `ModalInputState::as_widget_input().text_input`;
                    // reset the caret blink so the insertion is visible.
                    caret_started_at_ms = crate::window::process_uptime_ms();
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
                        for ((_id, _label, level), (radio_x, radio_y)) in
                            diff_labels.iter().zip(diff_positions)
                        {
                            if point_in_rect(
                                vx,
                                vy,
                                radio_x - 8,
                                radio_y - 8,
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
            mouse_button: MouseButtons::empty(),
            keyboard: &empty_keyboard,
            text_input: widget_input.text_input,
            capture: None,
        };
        let _field_events = input_widget.process_input(&field_input);
        if input_widget.base.state != UiState::SelectedEditable {
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
                PROMPT_ID_DIFF_LEGENDARY => difficulty = DifficultyLevel::Legendary,
                PROMPT_ID_DIFF_CUSTOM => {
                    difficulty = DifficultyLevel::Custom(DifficultyRules::MEDIUM)
                }
                _ => {}
            }
        }

        if confirmed {
            // Return the raw input; only a *literal* empty string gets
            // replaced with the "Anonymous" placeholder, and whitespace
            // passes through unchanged. Don't trim here — the caller
            // decides on the empty→Anonymous substitution.
            break Some((input_widget.edit_text, difficulty));
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
            draw_fallback_panel(
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

        if let Some(font) = resources.title_font_any() {
            let tw = font.text_width(title);
            render_text_virt_font(
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

        if is_new_player && let Some(font) = resources.popup_font_any() {
            let name = resources.menu_text.get(MT_STR_NAME);
            let difficulty = resources.menu_text.get(MT_STR_DIFFICULTY_LEVEL);
            let name_w = font.text_width(&name);
            let difficulty_w = font.text_width(&difficulty);
            render_text_virt_font(
                renderer,
                font,
                transform,
                &name,
                win_x + (win_w - name_w) / 2,
                win_y + 100,
            );
            render_text_virt_font(
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
            let src = BBox::from_coords(0.0, 0.0, input_rect.w as f32, input_rect.h as f32);
            let dst = BBox::from_coords(
                x as f32,
                y as f32,
                (x + input_rect.w) as f32,
                (y + input_rect.h) as f32,
            );
            renderer
                .draw_surface(
                    surf,
                    Some(&src),
                    Some(&dst),
                    crate::renderer::BLIT_SOURCE_TRANSPARENT,
                )
                .expect("live selected input surface");
        } else {
            draw_fallback_panel(renderer, transform, &input_rect);
        }

        // Toggle the caret every ~500 ms regardless of display refresh rate
        // (matching the save/load picker's timing). The caret
        // is inserted at its char offset inside the buffer — widget
        // `caret_offset` tracks chars, so we split on char boundaries.
        let caret_elapsed_ms = crate::window::process_uptime_ms().wrapping_sub(caret_started_at_ms);
        let show_caret = (caret_elapsed_ms / 500).is_multiple_of(2);
        if let Some(font) = resources.edit_field_font_any() {
            let display = player_name_display(
                &input_widget.edit_text,
                input_widget.caret_offset,
                show_caret,
            );
            render_text_virt_font(
                renderer,
                font,
                transform,
                &display,
                input_rect.x + 8,
                input_rect.y + 6,
            );
        }

        widget_bridge::draw_frame_buttons(renderer, resources, transform, &frame);
        if is_new_player && let Some(font) = resources.popup_font_any() {
            for ((_id, label, _level), (radio_x, radio_y)) in diff_labels.iter().zip(diff_positions)
            {
                let box_x = radio_x + (diff_btn_w - DIFFICULTY_LABEL_W) / 2;
                let text_w = font.text_width(label);
                render_text_virt_font(
                    renderer,
                    font,
                    transform,
                    label,
                    box_x + (DIFFICULTY_LABEL_W - text_w) / 2,
                    radio_y + diff_btn_h + 5,
                );
            }
        }
        if let Some(cursor) = cursor.as_mut() {
            cursor.cursor.advance_ui_animation();
            cursor.draw(renderer, transform, &input_state);
        }

        renderer.present();
        crate::window::sleep_ui_frame().await;
    };
    crate::window::stop_text_input();
    outcome
}

fn player_name_display(text: &str, caret: usize, show_caret: bool) -> std::borrow::Cow<'_, str> {
    if !show_caret {
        return std::borrow::Cow::Borrowed(text);
    }
    let byte_idx = text
        .char_indices()
        .nth(caret)
        .map(|(byte, _)| byte)
        .unwrap_or(text.len());
    let (head, tail) = text.split_at(byte_idx);
    std::borrow::Cow::Owned(format!("{head}|{tail}"))
}

#[test]
fn player_name_display_borrows_plain_text_and_places_caret_at_unicode_boundaries() {
    let text = String::from("aé界🙂");
    let plain = player_name_display(&text, 2, false);
    assert!(matches!(plain, std::borrow::Cow::Borrowed(_)));
    assert_eq!(plain.as_ptr(), text.as_ptr());
    assert_eq!(plain, text);
    for (caret, expected) in [
        (0, "|aé界🙂"),
        (1, "a|é界🙂"),
        (2, "aé|界🙂"),
        (3, "aé界|🙂"),
        (4, "aé界🙂|"),
        (usize::MAX, "aé界🙂|"),
    ] {
        assert_eq!(player_name_display(&text, caret, true), expected);
    }
    assert_eq!(player_name_display("", 0, true), "|");
    assert_eq!(player_name_display("", 0, false), "");
}

const ADVANCED_RULE_COUNT: usize = 18;

fn preset_index(difficulty: DifficultyLevel) -> usize {
    match difficulty {
        DifficultyLevel::Easy => 0,
        DifficultyLevel::Medium => 1,
        DifficultyLevel::Hard => 2,
        DifficultyLevel::Legendary => 3,
        DifficultyLevel::Custom(_) => 4,
    }
}

fn preset_at(index: usize, custom_rules: DifficultyRules) -> DifficultyLevel {
    match index {
        0 => DifficultyLevel::Easy,
        1 => DifficultyLevel::Medium,
        2 => DifficultyLevel::Hard,
        3 => DifficultyLevel::Legendary,
        4 => DifficultyLevel::Custom(custom_rules),
        _ => panic!("difficulty preset index {index} is out of range"),
    }
}

fn step_value(value: &mut u16, direction: i8, step: u16, min: u16, max: u16) {
    *value = if direction < 0 {
        value.saturating_sub(step).max(min)
    } else {
        value.saturating_add(step).min(max)
    };
}

fn adjust_custom_rule(rules: &mut DifficultyRules, row: usize, direction: i8) {
    match row {
        0 => {
            rules.legacy_level = match (rules.legacy_level, direction < 0) {
                (LegacyDifficultyLevel::Easy, true) => LegacyDifficultyLevel::Easy,
                (LegacyDifficultyLevel::Easy, false) => LegacyDifficultyLevel::Medium,
                (LegacyDifficultyLevel::Medium, true) => LegacyDifficultyLevel::Easy,
                (LegacyDifficultyLevel::Medium, false) => LegacyDifficultyLevel::Hard,
                (LegacyDifficultyLevel::Hard, true) => LegacyDifficultyLevel::Medium,
                (LegacyDifficultyLevel::Hard, false) => LegacyDifficultyLevel::Hard,
            }
        }
        1 => step_value(&mut rules.enemy_fighting_percent, direction, 10, 25, 400),
        2 => step_value(&mut rules.enemy_shooting_percent, direction, 10, 25, 400),
        3 => step_value(&mut rules.enemy_iq_percent, direction, 10, 25, 400),
        4 => step_value(&mut rules.enemy_life_points_percent, direction, 10, 25, 400),
        5 => step_value(&mut rules.reaction_time_percent, direction, 5, 10, 400),
        6 => step_value(
            &mut rules.hostile_soldier_view_distance_percent,
            direction,
            5,
            25,
            200,
        ),
        7 => step_value(
            &mut rules.hostile_soldier_view_angle_percent,
            direction,
            5,
            25,
            200,
        ),
        8 => step_value(
            &mut rules.hostile_soldier_noise_sensitivity_percent,
            direction,
            5,
            25,
            200,
        ),
        9 => step_value(
            &mut rules.blip_detection_range_percent,
            direction,
            5,
            10,
            200,
        ),
        10 => step_value(&mut rules.carnage_percent, direction, 10, 25, 400),
        11 => step_value(&mut rules.six_capacity, direction, 1, 0, 99),
        12 => step_value(&mut rules.twelve_capacity, direction, 1, 0, 99),
        13 => step_value(
            &mut rules.pc_auto_heal_interval_frames,
            direction,
            10,
            0,
            3600,
        ),
        14 => rules.accurate_net_preview = !rules.accurate_net_preview,
        15 => rules.protect_allies_from_pc_arrows = !rules.protect_allies_from_pc_arrows,
        16 => step_value(&mut rules.special_strike_base_frames, direction, 1, 0, 60),
        17 => step_value(
            &mut rules.pc_punch_concussion_percent,
            direction,
            10,
            25,
            400,
        ),
        _ => panic!("advanced difficulty row {row} is out of range"),
    }
    rules
        .validate()
        .expect("difficulty editor produced invalid rules");
}

fn advanced_rule_label_id(row: usize) -> &'static str {
    match row {
        0 => MT_PORT_DIFF_LEGACY_SCRIPT,
        1 => MT_PORT_DIFF_ENEMY_FIGHTING,
        2 => MT_PORT_DIFF_ENEMY_SHOOTING,
        3 => MT_PORT_DIFF_ENEMY_IQ,
        4 => MT_PORT_DIFF_ENEMY_HEALTH,
        5 => MT_PORT_DIFF_REACTION_TIME,
        6 => MT_PORT_DIFF_GUARD_VIEW_DISTANCE,
        7 => MT_PORT_DIFF_GUARD_VIEW_ANGLE,
        8 => MT_PORT_DIFF_GUARD_NOISE_SENSITIVITY,
        9 => MT_PORT_DIFF_BLIP_RANGE,
        10 => MT_PORT_DIFF_CARNAGE,
        11 => MT_PORT_DIFF_SMALL_CAPACITY,
        12 => MT_PORT_DIFF_LARGE_CAPACITY,
        13 => MT_PORT_DIFF_AUTO_HEAL,
        14 => MT_PORT_DIFF_NET_PREVIEW,
        15 => MT_PORT_DIFF_FRIENDLY_FIRE,
        16 => MT_PORT_DIFF_SPECIAL_DELAY,
        17 => MT_PORT_DIFF_PUNCH,
        _ => panic!("advanced difficulty row {row} is out of range"),
    }
}

fn advanced_rule_value(
    resources: &IngameMenuResources,
    rules: DifficultyRules,
    row: usize,
) -> String {
    let percent = |value: u16| format!("{value}%");
    let yes_no = |value: bool| {
        resources.menu_text.get(if value {
            MT_INFOBULLE_BUTTON_YES
        } else {
            MT_INFOBULLE_BUTTON_NO
        })
    };
    match row {
        0 => difficulty_to_string(
            &resources.menu_text,
            match rules.legacy_level {
                LegacyDifficultyLevel::Easy => DifficultyLevel::Easy,
                LegacyDifficultyLevel::Medium => DifficultyLevel::Medium,
                LegacyDifficultyLevel::Hard => DifficultyLevel::Hard,
            },
        ),
        1 => percent(rules.enemy_fighting_percent),
        2 => percent(rules.enemy_shooting_percent),
        3 => percent(rules.enemy_iq_percent),
        4 => percent(rules.enemy_life_points_percent),
        5 => percent(rules.reaction_time_percent),
        6 => percent(rules.hostile_soldier_view_distance_percent),
        7 => percent(rules.hostile_soldier_view_angle_percent),
        8 => percent(rules.hostile_soldier_noise_sensitivity_percent),
        9 => percent(rules.blip_detection_range_percent),
        10 => percent(rules.carnage_percent),
        11 => rules.six_capacity.to_string(),
        12 => rules.twelve_capacity.to_string(),
        13 if rules.pc_auto_heal_interval_frames == 0 => yes_no(false),
        13 => format!("{} frames", rules.pc_auto_heal_interval_frames),
        14 => yes_no(rules.accurate_net_preview),
        15 => yes_no(rules.protect_allies_from_pc_arrows),
        16 => format!("{} frames", rules.special_strike_base_frames),
        17 => percent(rules.pc_punch_concussion_percent),
        _ => panic!("advanced difficulty row {row} is out of range"),
    }
}

/// Edit a profile's difficulty between missions. Presets are immutable views
/// of their rules; selecting Custom makes every simulation-affecting field
/// editable and stores the resolved values in the profile.
async fn show_difficulty_prompt(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    initial: DifficultyLevel,
    mut cursor: Option<ModalCursor<'_>>,
) -> Option<DifficultyLevel> {
    const PANEL: MenuRect = MenuRect {
        x: 20,
        y: 14,
        w: 600,
        h: 452,
    };
    const PRESET_X: i32 = 39;
    const PRESET_Y: i32 = 58;
    const PRESET_W: i32 = 106;
    const PRESET_H: i32 = 30;
    const PRESET_GAP: i32 = 6;
    const RULE_X: i32 = 68;
    const RULE_Y: i32 = 96;
    const RULE_W: i32 = 504;
    const RULE_H: i32 = 16;

    let transform = MenuTransform::centered(
        renderer.screen_width() as i32,
        renderer.screen_height() as i32,
    );
    let preset_labels: [String; 5] = std::array::from_fn(|index| {
        difficulty_to_string(&resources.menu_text, preset_at(index, initial.rules()))
    });
    let mut custom_rules = initial.rules();
    let mut difficulty = initial;
    let mut focused_rule = 0usize;
    let mut input_state = ModalInputState::new();
    input_state.seed_mouse_from_window(event_pump, transform);
    let (ok_w, ok_h) = resources.ok_button_dimensions();
    let (cancel_w, cancel_h) = resources.cancel_button_dimensions();
    let ok_x = PANEL.x + PANEL.w / 2 - ok_w - 8;
    let cancel_x = PANEL.x + PANEL.w / 2 + 8;
    let button_y = PANEL.y + PANEL.h - ok_h.max(cancel_h) - 10;

    loop {
        let mut confirmed = false;
        let mut cancelled = false;
        let (events, transform) =
            crate::ingame_menu::layout::poll_events_with_transform(event_pump, renderer);
        for event in events {
            input_state.update_from_event(&event, transform);
            match event {
                GameEvent::Quit
                | GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => cancelled = true,
                GameEvent::KeyDown {
                    keycode: Keycode::Return | Keycode::KpEnter,
                    ..
                } => confirmed = true,
                GameEvent::KeyDown {
                    keycode: Keycode::Up,
                    ..
                } if matches!(difficulty, DifficultyLevel::Custom(_)) => {
                    focused_rule = focused_rule.saturating_sub(1);
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Down,
                    ..
                } if matches!(difficulty, DifficultyLevel::Custom(_)) => {
                    focused_rule = (focused_rule + 1).min(ADVANCED_RULE_COUNT - 1);
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Left,
                    ..
                } if matches!(difficulty, DifficultyLevel::Custom(_)) => {
                    adjust_custom_rule(&mut custom_rules, focused_rule, -1);
                    difficulty = DifficultyLevel::Custom(custom_rules);
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Right,
                    ..
                } if matches!(difficulty, DifficultyLevel::Custom(_)) => {
                    adjust_custom_rule(&mut custom_rules, focused_rule, 1);
                    difficulty = DifficultyLevel::Custom(custom_rules);
                }
                GameEvent::MouseUp(x, y, 1) => {
                    let (vx, vy) = transform.from_screen(x, y);
                    if point_in_rect(vx, vy, ok_x, button_y, ok_w, ok_h) {
                        confirmed = true;
                    } else if point_in_rect(vx, vy, cancel_x, button_y, cancel_w, cancel_h) {
                        cancelled = true;
                    } else if (PRESET_Y..PRESET_Y + PRESET_H).contains(&vy) {
                        for index in 0..preset_labels.len() {
                            let x = PRESET_X + index as i32 * (PRESET_W + PRESET_GAP);
                            if point_in_rect(vx, vy, x, PRESET_Y, PRESET_W, PRESET_H) {
                                difficulty = preset_at(index, custom_rules);
                                break;
                            }
                        }
                    } else if matches!(difficulty, DifficultyLevel::Custom(_))
                        && (RULE_X..RULE_X + RULE_W).contains(&vx)
                        && vy >= RULE_Y
                    {
                        let row = ((vy - RULE_Y) / RULE_H) as usize;
                        if row < ADVANCED_RULE_COUNT {
                            focused_rule = row;
                            let direction = if vx < RULE_X + RULE_W / 2 { -1 } else { 1 };
                            adjust_custom_rule(&mut custom_rules, row, direction);
                            difficulty = DifficultyLevel::Custom(custom_rules);
                        }
                    }
                }
                _ => {}
            }
        }
        input_state.end_frame();

        if confirmed {
            return Some(match difficulty {
                DifficultyLevel::Custom(_) => DifficultyLevel::custom(custom_rules)
                    .expect("difficulty editor confirmed invalid custom rules"),
                preset => preset,
            });
        }
        if cancelled {
            return None;
        }

        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);
        draw_fallback_panel(renderer, transform, &PANEL);

        if let Some(font) = resources.title_font_any() {
            let title = resources
                .menu_text
                .get_port(MT_PORT_TTL_ADVANCED_DIFFICULTY);
            render_text_virt_font(
                renderer,
                font,
                transform,
                title,
                PANEL.x + (PANEL.w - font.text_width(title)) / 2,
                PANEL.y + 13,
            );
        }

        if let Some(font) = resources.popup_font_any() {
            for (index, label) in preset_labels.iter().enumerate() {
                let x = PRESET_X + index as i32 * (PRESET_W + PRESET_GAP);
                let marker = if index == preset_index(difficulty) {
                    "[x]"
                } else {
                    "[ ]"
                };
                let text = format!("{marker} {label}");
                render_text_virt_font(renderer, font, transform, &text, x + 4, PRESET_Y + 7);
            }

            let rules = difficulty.rules();
            for row in 0..ADVANCED_RULE_COUNT {
                let marker =
                    if matches!(difficulty, DifficultyLevel::Custom(_)) && row == focused_rule {
                        ">"
                    } else {
                        " "
                    };
                let label = resources.menu_text.get_port(advanced_rule_label_id(row));
                let value = advanced_rule_value(resources, rules, row);
                let text = format!("{marker} {label}: {value}");
                render_text_virt_font(
                    renderer,
                    font,
                    transform,
                    &text,
                    RULE_X,
                    RULE_Y + row as i32 * RULE_H,
                );
            }
            if matches!(difficulty, DifficultyLevel::Custom(_)) {
                let help = resources.menu_text.get_port(MT_PORT_STR_DIFFICULTY_HELP);
                render_text_virt_font(renderer, font, transform, help, RULE_X, 390);
            }
        }

        let mut frame = FrameWnd::default();
        frame.enabled = true;
        frame.input_enabled = false;
        frame.add_widget_absolute(widget_bridge::make_button_with_resource(
            PROMPT_ID_OK,
            "",
            true,
            resource_ids::RHID_OK,
            ok_x,
            button_y,
            ok_w,
            ok_h,
        ));
        frame.add_widget_absolute(widget_bridge::make_button_with_resource(
            PROMPT_ID_CANCEL,
            "",
            true,
            resource_ids::RHID_CANCEL,
            cancel_x,
            button_y,
            cancel_w,
            cancel_h,
        ));
        widget_bridge::draw_frame_buttons(renderer, resources, transform, &frame);

        if let Some(cursor) = cursor.as_mut() {
            cursor.cursor.advance_animation();
            cursor.draw(renderer, transform, &input_state);
        }
        renderer.present();
        crate::window::sleep_ms(16).await;
    }
}

fn set_profile_difficulty(
    application_context: &ApplicationContext,
    idx: usize,
    difficulty: DifficultyLevel,
) {
    difficulty
        .rules()
        .validate()
        .expect("difficulty dialog returned invalid rules");
    application_context
        .update_and_retain_player_profiles(|profiles| {
            let profile = profiles.profiles.get_mut(idx).unwrap_or_else(|| {
                panic!("difficulty profile index {idx} disappeared during editing")
            });
            profile.difficulty = difficulty;
        })
        .unwrap_or_else(|error| panic!("Select Player difficulty update failed: {error}"))
        .log_persistence_error("Select Player: failed to persist difficulty");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key_config_store::KeyConfigStore;
    use crate::spellforge_trust::{
        SpellforgeTrustKey, SpellforgeTrustMetadata, SpellforgeTrustStore,
    };
    use robin_engine::engine::GlobalOptions;
    use robin_engine::player_profile::PlayerProfileManager;

    fn trust_key(value: u8) -> SpellforgeTrustKey {
        SpellforgeTrustKey {
            full_mod_sha256: [value; 32],
            package_sha256: Some([value.wrapping_add(1); 32]),
        }
    }

    fn trust_metadata() -> SpellforgeTrustMetadata {
        SpellforgeTrustMetadata {
            mission: "Mission".into(),
            title: "Mission".into(),
            claimed_author: "Author".into(),
            version: "1".into(),
            source_url: "https://example.invalid/mod".into(),
            license: "CC0".into(),
            host_endpoint_id: "endpoint-public-key".into(),
            package_vm_abi: Some("spellforge-v1-sha256:00".into()),
            compressed_bytes: 123,
        }
    }

    #[test]
    fn roster_snapshot_preserves_order_active_flags_and_dialog_fields() {
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().to_string_lossy().into_owned();
        let mut profiles = PlayerProfileManager::new(root_path.clone());
        let first = profiles.create_profile("Robin".into(), DifficultyLevel::Medium);
        let second = profiles.create_profile("Marian".into(), DifficultyLevel::Hard);
        profiles.profiles[first].progression = 12;
        profiles.profiles[second].progression = 34;
        profiles.set_active(second);
        let mut keys = KeyConfigStore::new(root_path.clone());
        for profile in &profiles.profiles {
            keys.entry_or_default(profile.id);
        }
        let context = ApplicationContext::complete(
            crate::player_profile_store::PlayerProfileStore::for_directory(&root_path),
            GlobalOptions::default(),
            profiles,
            keys,
            None,
        )
        .unwrap();
        let (rows, active) = profiles_snapshot(&context);
        assert_eq!(active, Some(second));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[first].name, "Robin");
        assert_eq!(rows[first].difficulty, DifficultyLevel::Medium);
        assert_eq!(rows[first].progression, 12);
        assert!(!rows[first].active);
        assert_eq!(rows[second].name, "Marian");
        assert_eq!(rows[second].difficulty, DifficultyLevel::Hard);
        assert_eq!(rows[second].progression, 34);
        assert!(rows[second].active);
    }

    #[test]
    fn profile_rows_are_independent_of_later_profile_changes() {
        let mut profile = PlayerProfile::new(1, "Robin".into(), DifficultyLevel::Medium);
        profile.progression = 42;
        profile.active = true;
        let row = PlayerProfileRow::from(&profile);
        profile.name.clear();
        profile.difficulty = DifficultyLevel::Hard;
        profile.progression = 0;
        profile.active = false;
        assert_eq!(row.name, "Robin");
        assert_eq!(row.difficulty, DifficultyLevel::Medium);
        assert_eq!(row.progression, 42);
        assert!(row.active);
    }

    #[test]
    fn deleting_the_final_profile_is_refused() {
        assert!(!can_delete_profile(1));
        assert!(can_delete_profile(2));

        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().to_string_lossy().into_owned();
        let mut profiles = PlayerProfileManager::new(root_path.clone());
        let index = profiles.create_profile("Robin".into(), DifficultyLevel::Medium);
        profiles.set_active(index);

        let mut keys = KeyConfigStore::new(root_path.clone());
        keys.entry_or_default(profiles.profiles[index].id);
        let context = ApplicationContext::complete(
            crate::player_profile_store::PlayerProfileStore::for_directory(&root_path),
            GlobalOptions::default(),
            profiles,
            keys,
            None,
        )
        .unwrap();

        assert!(!delete_profile(&context, 0).unwrap());

        context
            .with_player_profiles(|profiles| {
                assert_eq!(profiles.profile_count(), 1);
                assert_eq!(profiles.active_index, Some(0));
                assert_eq!(profiles.get_active().unwrap().name, "Robin");
            })
            .unwrap();
        assert!(context.active_key_configs().is_ok());
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn failed_profile_deletion_does_not_publish_staged_profiles() {
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().to_string_lossy().into_owned();
        let mut profiles = PlayerProfileManager::new(root_path.clone());
        let first = profiles.create_profile("Robin".into(), DifficultyLevel::Medium);
        profiles.create_profile("Marian".into(), DifficultyLevel::Hard);
        profiles.set_active(first);
        let context = ApplicationContext::complete(
            crate::player_profile_store::PlayerProfileStore::for_directory(&root_path),
            GlobalOptions::default(),
            profiles,
            KeyConfigStore::new(root_path),
            None,
        )
        .unwrap();
        let before = serde_json::to_value(context.player_profiles_snapshot().unwrap()).unwrap();
        let config = context.sim_config();
        // Reject the profile store's atomic rename without changing permissions.
        let saves = root.path().join("Profile_000");
        std::fs::create_dir(&saves).unwrap();
        std::fs::write(saves.join("QuickSave.json"), b"retained save").unwrap();
        std::fs::create_dir(root.path().join("profiles.json")).unwrap();
        assert!(
            delete_profile(&context, first)
                .unwrap_err()
                .contains("persist player deletion")
        );
        assert_eq!(
            serde_json::to_value(context.player_profiles_snapshot().unwrap()).unwrap(),
            before
        );
        assert_eq!(context.sim_config(), config);
        assert_eq!(
            std::fs::read(saves.join("QuickSave.json")).unwrap(),
            b"retained save"
        );
        assert!(!root.path().join(".deleted-Profile_000").exists());
    }

    #[test]
    fn deleting_a_profile_durably_revokes_its_exact_content_grants() {
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().to_string_lossy().into_owned();
        let mut profiles = PlayerProfileManager::new(root_path.clone());
        let first = profiles.create_profile("Robin".into(), DifficultyLevel::Medium);
        let second = profiles.create_profile("Marian".into(), DifficultyLevel::Medium);
        profiles.set_active(first);
        let deleted_id = profiles.profiles[first].id;
        let remaining_id = profiles.profiles[second].id;

        let mut keys = KeyConfigStore::new(root_path.clone());
        keys.entry_or_default(deleted_id);
        keys.entry_or_default(remaining_id);
        let context = ApplicationContext::complete(
            crate::player_profile_store::PlayerProfileStore::for_directory(&root_path),
            GlobalOptions::default(),
            profiles,
            keys,
            None,
        )
        .unwrap();
        context
            .grant_spellforge_content_trust(trust_key(7), trust_metadata(), 10)
            .unwrap();

        assert!(delete_profile(&context, first).unwrap());
        context
            .with_spellforge_trust(|store| {
                assert!(store.grants_for_profile(deleted_id).is_empty());
            })
            .unwrap();
        let reloaded = SpellforgeTrustStore::load(&root_path).unwrap();
        assert!(reloaded.grants_for_profile(deleted_id).is_empty());
        assert_eq!(context.active_profile_snapshot().unwrap().id, remaining_id);
        assert!(!context.is_spellforge_content_trusted(trust_key(7)).unwrap());
    }

    #[test]
    fn unavailable_trust_persistence_blocks_profile_deletion() {
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().to_string_lossy().into_owned();
        let mut profiles = PlayerProfileManager::new(root_path.clone());
        let first = profiles.create_profile("Robin".into(), DifficultyLevel::Medium);
        let second = profiles.create_profile("Marian".into(), DifficultyLevel::Medium);
        profiles.set_active(first);

        let mut keys = KeyConfigStore::new(root_path.clone());
        keys.entry_or_default(profiles.profiles[first].id);
        keys.entry_or_default(profiles.profiles[second].id);
        let context = ApplicationContext::complete(
            crate::player_profile_store::PlayerProfileStore::for_directory(&root_path),
            GlobalOptions::default(),
            profiles,
            keys,
            None,
        )
        .unwrap();
        context
            .with_spellforge_trust_mut(|store| {
                *store = SpellforgeTrustStore::unavailable(root_path, "corrupt trust store");
            })
            .unwrap();

        let error = delete_profile(&context, first).unwrap_err();
        assert!(error.contains("persistence is unavailable"), "{error}");
        context
            .with_player_profiles(|profiles| {
                assert_eq!(profiles.profile_count(), 2);
                assert_eq!(profiles.get_active().unwrap().name, "Robin");
            })
            .unwrap();
    }

    #[test]
    fn custom_editor_steps_every_numeric_rule_within_valid_ranges() {
        let mut rules = DifficultyRules::MEDIUM;
        for row in 0..ADVANCED_RULE_COUNT {
            adjust_custom_rule(&mut rules, row, 1);
            rules.validate().unwrap();
            adjust_custom_rule(&mut rules, row, -1);
            rules.validate().unwrap();
        }
    }

    #[test]
    fn advanced_preset_index_roundtrips_all_fixed_presets() {
        for (index, expected) in [
            DifficultyLevel::Easy,
            DifficultyLevel::Medium,
            DifficultyLevel::Hard,
            DifficultyLevel::Legendary,
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(preset_at(index, DifficultyRules::MEDIUM), expected);
            assert_eq!(preset_index(expected), index);
        }
    }

    #[test]
    fn new_player_difficulty_rows_keep_label_columns_separate() {
        // The shipping RHID_RADIO frames are 44 px wide. The layout must use
        // that full width rather than the old clipped 32 px hit/draw box.
        let radio_width = 44;
        let positions = new_player_difficulty_positions(72, 8, radio_width);

        assert_eq!(positions[0].1, positions[1].1);
        assert_eq!(positions[1].1, positions[2].1);
        assert_eq!(positions[3].1, positions[4].1);
        assert!(positions[3].1 > positions[0].1);

        for row in [&positions[0..3], &positions[3..5]] {
            for pair in row.windows(2) {
                let left_center = pair[0].0 + 16;
                let right_center = pair[1].0 + 16;
                assert!(right_center - left_center >= DIFFICULTY_LABEL_W);
            }
        }

        for (x, _) in positions {
            let label_x = x + (radio_width - DIFFICULTY_LABEL_W) / 2;
            assert!(label_x >= 72);
            assert!(label_x + DIFFICULTY_LABEL_W <= 72 + NEW_PLAYER_PROMPT_W);
        }
    }

    #[test]
    fn profile_difficulty_change_refreshes_future_simulation_config() {
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().to_string_lossy().into_owned();
        let mut profiles = PlayerProfileManager::new(root_path.clone());
        let index = profiles.create_profile("Robin".into(), DifficultyLevel::Medium);
        profiles.set_active(index);
        let mut keys = KeyConfigStore::new(root_path.clone());
        keys.entry_or_default(profiles.profiles[index].id);
        let context = ApplicationContext::complete(
            crate::player_profile_store::PlayerProfileStore::for_directory(&root_path),
            GlobalOptions::default(),
            profiles,
            keys,
            None,
        )
        .unwrap();

        set_profile_difficulty(&context, index, DifficultyLevel::Legendary);

        assert_eq!(context.sim_config().difficulty, DifficultyLevel::Legendary);
        assert_eq!(
            context.active_profile_snapshot().unwrap().difficulty,
            DifficultyLevel::Legendary
        );
    }
}
