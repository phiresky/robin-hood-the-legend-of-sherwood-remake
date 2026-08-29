//! Main-menu "Custom Missions" entry.
//!
//! Scans the local mods directory (typically `datadirs/mods/`), shows a
//! scrollable list of `(mod, version, .rhm)` rows with a side detail
//! pane, and returns a [`CustomMissionLaunch`] describing the choice.
//!
//! Selected mod is mounted as a non-destructive overlay by the caller
//! (`main_entry`) via [`crate::mod_pack::mount_for_launch`] just before
//! the session starts; this picker is purely a UI for choosing.
//!
//! Lua / Spellforge runtime support is handled by a separate agent —
//! this picker only cares about discovery and the proto-level filename
//! peeked out of each `.rhm` header.

use robin_engine::sprite::BBox;
use std::path::{Path, PathBuf};

use crate::gfx_types::{GameEvent, Keycode};
use crate::ingame_menu::IngameMenuResources;
use crate::ingame_menu::layout::{
    MENU_W, MenuTransform, TextAlign, VAlign, align_bottom_right, dim_screen,
    enter_modal_gpu_phase, render_text_in_box_aligned, render_text_virt,
};
use crate::ingame_menu::widget_bridge::{self, ModalCursor, ModalInputState};
use crate::mod_pack::{MissionEntry, MissionStatus, enumerate_missions, scan_mods_dir};
use crate::renderer::Renderer;
use crate::ui::MouseButtons;
use crate::widget::FrameWnd;

/// What the picker returns when the player chooses to launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomMissionChoice {
    /// A downloaded mod pack: the caller mounts its zip as an overlay
    /// via [`crate::mod_pack::mount_for_launch`] before the session.
    Mod(CustomMissionLaunch),
    /// A hackable JSON level shipped in an always-mounted overlay
    /// datadir (see `ModDetails::hackable_missions`): launched directly
    /// by mission filename, no mount step.
    Hackable { mission: String, title: String },
}

/// Launch description for a mod-pack (zip) custom mission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomMissionLaunch {
    pub slug: String,
    pub mod_title: String,
    pub version_zip: PathBuf,
    pub rhm_basename: String,
    /// Proto-level (.rhp) filename pulled from the `.rhm` header.
    pub map_filename: String,
    pub requires_spellforge: bool,
}

// ── Layout (640×480 virtual menu) ────────────────────────────────

const TITLE_Y: i32 = 8;
const LIST_X: i32 = 14;
const LIST_Y: i32 = 36;
const LIST_W: i32 = 380;
const LIST_H: i32 = 380;
const DETAIL_X: i32 = 408;
const DETAIL_Y: i32 = 36;
const DETAIL_W: i32 = 218;
const DETAIL_H: i32 = 380;
const ROW_HEIGHT: i32 = 20;

const ID_PLAY: u32 = 0;
const ID_CANCEL: u32 = 1;

/// Display the picker.  Returns `Some(CustomMissionChoice)` when the
/// player picked a launchable mission, `None` on cancel or when there's
/// no launchable content.
pub(crate) async fn show_custom_missions(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    cursor: ModalCursor<'_>,
    mods_root: &Path,
) -> Option<CustomMissionChoice> {
    let mut mods = scan_mods_dir(mods_root);
    // Overlay-shipped mods (repo `mods/`, e.g. hackable levels) may also
    // carry a `details.json`; list them alongside the downloaded packs.
    if let Some(overlay_root) = crate::main_entry::overlay_mods_dir()
        && overlay_root != mods_root
    {
        mods.extend(scan_mods_dir(&overlay_root));
        mods.sort_by(|a, b| a.details.title.cmp(&b.details.title));
    }
    let entries = enumerate_missions(&mods);
    if entries.is_empty() {
        tracing::info!(
            "Custom missions: no mods discovered under {} — picker would be empty, returning to main menu",
            mods_root.display()
        );
        return None;
    }

    let sw = renderer.screen_width() as i32;
    let sh = renderer.screen_height() as i32;
    let transform = MenuTransform::centered(sw, sh);

    let (btn_w, btn_h) = resources.button_dimensions();
    let play_label = "Play".to_string();
    let cancel_label = "Cancel".to_string();
    let labels: &[(&str, bool)] = &[(&play_label, false), (&cancel_label, true)];
    let positions = align_bottom_right(labels, btn_w, btn_h);
    let btn_positions: [(u32, String, i32, i32); 2] = [
        (ID_PLAY, play_label.clone(), positions[0].x, positions[0].y),
        (
            ID_CANCEL,
            cancel_label.clone(),
            positions[1].x,
            positions[1].y,
        ),
    ];
    // Default selection: first launchable row if any, otherwise the
    // first row (which will be broken — at least the user can read why).
    let mut selected: usize = entries.iter().position(|e| e.status.is_ok()).unwrap_or(0);
    let visible_rows = (LIST_H / ROW_HEIGHT).max(1) as usize;
    let mut scroll_offset: usize = 0;

    let mut input_state = ModalInputState::new();
    input_state.seed_mouse_from_window(event_pump, transform);

    // FrameWnd holds widget state (Focused/Pushed/Activated) across
    // frames — menu buttons take multiple ticks to traverse the state
    // machine, so the frame must be persistent. Rebuilding it every
    // iteration would reset every button to Default and clicks would
    // never register. Enablement is updated in-place each frame on the
    // existing widgets below.
    let mut frame = FrameWnd::default();
    frame.enabled = true;
    frame.input_enabled = true;
    for (id, label, x, y) in &btn_positions {
        frame.add_widget_absolute(widget_bridge::make_button_enabled(
            *id, label, true, *x, *y, btn_w, btn_h,
        ));
    }

    loop {
        // Ensure the selected row stays within the visible window. Done
        // before event polling so a keyboard arrow that just moved the
        // selection off-screen still triggers a scroll this frame.
        if selected < scroll_offset {
            scroll_offset = selected;
        } else if selected >= scroll_offset + visible_rows {
            scroll_offset = selected + 1 - visible_rows;
        }

        let play_enabled = entries[selected].status.is_ok();
        // Update the Play button's enabled flag in place — selecting a
        // broken row should grey the button without resetting its
        // hover/push state machine.
        if let Some(w) = frame.widget_mut(ID_PLAY) {
            w.base_mut().enabled = play_enabled;
        }

        // ── Events ──────────────────────────────────────────────
        let mut activated: Option<u32> = None;
        for event in event_pump.poll_events() {
            input_state.update_from_event(&event, transform);
            match event {
                GameEvent::Quit => activated = Some(ID_CANCEL),
                GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => activated = Some(ID_CANCEL),
                GameEvent::KeyDown {
                    keycode: Keycode::Up,
                    ..
                } => {
                    selected = selected.saturating_sub(1);
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Down,
                    ..
                } => {
                    if selected + 1 < entries.len() {
                        selected += 1;
                    }
                }
                GameEvent::KeyDown {
                    keycode: Keycode::PageUp,
                    ..
                } => {
                    selected = selected.saturating_sub(visible_rows);
                }
                GameEvent::KeyDown {
                    keycode: Keycode::PageDown,
                    ..
                } => {
                    selected = (selected + visible_rows).min(entries.len().saturating_sub(1));
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Home,
                    ..
                } => selected = 0,
                GameEvent::KeyDown {
                    keycode: Keycode::End,
                    ..
                } => selected = entries.len().saturating_sub(1),
                GameEvent::KeyDown {
                    keycode: Keycode::Return,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::KpEnter,
                    ..
                } if play_enabled => {
                    activated = Some(ID_PLAY);
                }
                GameEvent::MouseUp(x, y, 1) => {
                    let (vx, vy) = transform.from_screen(x, y);
                    if (LIST_X..LIST_X + LIST_W).contains(&vx)
                        && (LIST_Y..LIST_Y + LIST_H).contains(&vy)
                    {
                        let row_offset = ((vy - LIST_Y - 4) / ROW_HEIGHT).max(0) as usize;
                        let target = scroll_offset + row_offset;
                        if target < entries.len() {
                            // Single click selects; double click of the
                            // same row launches (when launchable).
                            let dbl = input_state
                                .buttons
                                .contains(MouseButtons::LEFT_DOUBLE_CLICK);
                            if target == selected && dbl && entries[selected].status.is_ok() {
                                activated = Some(ID_PLAY);
                            }
                            selected = target;
                        }
                    }
                }
                GameEvent::MouseWheel(dy) => {
                    let max_scroll = entries.len().saturating_sub(visible_rows);
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
        let events = frame.process_input(&widget_input);
        input_state.end_frame();
        if let Some(id) = widget_bridge::find_activated(&events) {
            activated = Some(id);
        }

        if let Some(id) = activated {
            match id {
                ID_CANCEL => return None,
                ID_PLAY => {
                    let e = &entries[selected];
                    if let MissionStatus::Ok { map_filename } = &e.status {
                        if e.hackable {
                            return Some(CustomMissionChoice::Hackable {
                                mission: e.rhm_basename.clone(),
                                title: e.mod_title.clone(),
                            });
                        }
                        return Some(CustomMissionChoice::Mod(CustomMissionLaunch {
                            slug: e.mod_slug.clone(),
                            mod_title: e.mod_title.clone(),
                            version_zip: e.version_zip.clone(),
                            rhm_basename: e.rhm_basename.clone(),
                            map_filename: map_filename.clone(),
                            requires_spellforge: e.requires_spellforge,
                        }));
                    }
                }
                _ => {}
            }
        }

        // ── Render ──────────────────────────────────────────────
        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);
        // Skip the wood/parchment menu background — both panes draw on
        // their own solid fills, so the busy menu artwork would only
        // bleed through the edges and fight the list/detail text. The
        // dimmed scene snapshot is enough of a backdrop.

        draw_title(renderer, resources, transform);
        draw_list(
            renderer,
            resources,
            transform,
            &entries,
            selected,
            scroll_offset,
            visible_rows,
        );
        draw_detail_pane(renderer, resources, transform, &entries[selected]);
        widget_bridge::draw_frame_buttons(renderer, resources, transform, &frame);
        cursor.draw(renderer, transform, &input_state);
        renderer.present();
        crate::window::sleep_ms(16).await;
    }
}

fn draw_title(renderer: &mut Renderer, resources: &IngameMenuResources, transform: MenuTransform) {
    let Some(font) = resources
        .edit_field_font()
        .or_else(|| resources.menu_text_font())
    else {
        return;
    };
    let title = "Custom Missions";
    let tw = font.text_width(title);
    let x = (MENU_W - tw) / 2;
    render_text_virt(renderer, font, transform, title, x, TITLE_Y);
}

fn draw_list(
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    transform: MenuTransform,
    entries: &[MissionEntry],
    selected: usize,
    scroll_offset: usize,
    visible_rows: usize,
) {
    // Solid dark backing so the row text isn't fighting the menu's wood
    // grate / parchment background.
    let (sx0, sy0) = transform.to_screen(LIST_X, LIST_Y);
    let (sx1, sy1) = transform.to_screen(LIST_X + LIST_W, LIST_Y + LIST_H);
    renderer.fill_screen(
        Some(&BBox::from_coords(
            sx0 as f32, sy0 as f32, sx1 as f32, sy1 as f32,
        )),
        Renderer::create_color_16(20, 15, 10),
    );
    renderer.draw_rect_outline_screen(sx0, sy0, sx1, sy1, Renderer::create_color_16(180, 160, 100));

    for row_offset in 0..visible_rows {
        let idx = scroll_offset + row_offset;
        if idx >= entries.len() {
            break;
        }
        let row_y = LIST_Y + 4 + row_offset as i32 * ROW_HEIGHT;
        let e = &entries[idx];
        let is_selected = idx == selected;
        let broken = !e.status.is_ok();

        // Selection highlight: thin fill bar so the eye can find the
        // current row even when the font choice is the same as
        // neighbours.
        if is_selected {
            let (sx0, sy0) = transform.to_screen(LIST_X + 2, row_y - 2);
            let (sx1, sy1) = transform.to_screen(LIST_X + LIST_W - 2, row_y + ROW_HEIGHT - 4);
            renderer.fill_screen(
                Some(&BBox::from_coords(
                    sx0 as f32, sy0 as f32, sx1 as f32, sy1 as f32,
                )),
                Renderer::create_color_16(60, 50, 30),
            );
        }

        // Use the same body font as the main menu's left-side profile
        // info block ("Difficulty level: Hard" etc.) — clean serif on
        // dark backdrop, consistent with the rest of the menu.
        let Some(font) = resources.menu_text_font() else {
            continue;
        };
        let tag = if e.requires_spellforge { "[SF] " } else { "" };
        let label = format!(
            "{tag}{title} — {rhm}  ({ver})",
            title = e.mod_title,
            rhm = e.rhm_basename,
            ver = e.version_label
        );
        let label = if broken {
            format!("{label}  — unavailable")
        } else {
            label
        };
        // Truncate so long labels don't bleed out of the list pane into
        // the detail pane. Drops trailing chars + adds an ellipsis if it
        // doesn't fit.
        let row_text_w = LIST_W - 20;
        let label = truncate_to_pixel_width(font, &label, row_text_w);
        // Visual highlight of the selected row already drawn above; the
        // text colour is the same for selected/unselected, matching the
        // main menu profile info block.
        let _ = is_selected;
        render_text_virt(renderer, font, transform, &label, LIST_X + 10, row_y);
    }
}

fn truncate_to_pixel_width(
    font: &crate::native_font::NativeFont,
    text: &str,
    max_w: i32,
) -> String {
    if max_w <= 0 {
        return String::new();
    }
    if font.text_width(text) <= max_w {
        return text.to_string();
    }
    let ellipsis = "…";
    let ellipsis_w = font.text_width(ellipsis);
    let budget = (max_w - ellipsis_w).max(0);
    let mut fit_end = 0usize;
    for (idx, _) in text.char_indices() {
        if font.text_width(&text[..idx]) > budget {
            break;
        }
        fit_end = idx;
    }
    let mut out = text[..fit_end].trim_end().to_string();
    out.push_str(ellipsis);
    out
}

fn draw_detail_pane(
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    transform: MenuTransform,
    entry: &MissionEntry,
) {
    let (sx0, sy0) = transform.to_screen(DETAIL_X, DETAIL_Y);
    let (sx1, sy1) = transform.to_screen(DETAIL_X + DETAIL_W, DETAIL_Y + DETAIL_H);
    renderer.fill_screen(
        Some(&BBox::from_coords(
            sx0 as f32, sy0 as f32, sx1 as f32, sy1 as f32,
        )),
        Renderer::create_color_16(20, 15, 10),
    );
    renderer.draw_rect_outline_screen(sx0, sy0, sx1, sy1, Renderer::create_color_16(180, 160, 100));

    // Same body font as the rows + the main menu's profile info block,
    // so the detail pane visually matches the rest of the menu.
    let Some(font) = resources.menu_text_font() else {
        return;
    };

    let mut y = DETAIL_Y + 8;
    let info_x = DETAIL_X + 8;
    let info_w = DETAIL_W - 16;
    let line_h = (font.height() as i32).max(12);

    let mut lines: Vec<String> = Vec::with_capacity(8);
    lines.push(entry.mod_title.clone());
    lines.push(format!("by {}", entry.author));
    lines.push(format!("Map: {}", entry.map));
    if entry.hackable {
        lines.push("Tag: Hackable level".to_string());
    } else if entry.requires_spellforge {
        lines.push("Tag: Spellforge".to_string());
    } else {
        lines.push("Tag: Vanilla".to_string());
    }
    lines.push(format!("Version: {}", entry.version_label));
    if !entry.rhm_basename.is_empty() {
        lines.push(format!("Mission: {}", entry.rhm_basename));
    }
    if let MissionStatus::Broken { reason } = &entry.status {
        lines.push(format!("Status: unavailable — {reason}"));
    }

    for line in lines {
        let _ = render_text_in_box_aligned(
            renderer,
            font,
            transform,
            &line,
            info_x,
            y,
            info_w,
            line_h * 2,
            TextAlign::Left,
            VAlign::Top,
        );
        y += line_h + 2;
    }

    // Blank gap before description.
    y += 4;

    if !entry.description.trim().is_empty() {
        // The remaining height drives how much description fits; the
        // wrap helper inside render_text_in_box_aligned clips
        // overflow rather than letting it escape the pane.
        let remaining_h = (DETAIL_Y + DETAIL_H) - y - 4;
        if remaining_h > line_h {
            let _ = render_text_in_box_aligned(
                renderer,
                font,
                transform,
                &entry.description,
                info_x,
                y,
                info_w,
                remaining_h,
                TextAlign::Left,
                VAlign::Top,
            );
        }
    }
}
