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
    MENU_W, MenuTransform, align_bottom_right, dim_screen, elide_text_to_width_by,
    enter_modal_gpu_phase, render_text_virt_font, wrap_text_for_box_font,
};
use crate::ingame_menu::widget_bridge::{self, ModalCursor, ModalInputState};
use crate::mod_pack::{MissionEntry, MissionStatus, enumerate_missions, scan_mission_roots};
use crate::renderer::Renderer;
use crate::scroll_view::ScrollView;
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
    pub claimed_author: String,
    pub version: String,
    pub source_url: String,
    pub license: String,
    pub version_zip: PathBuf,
    /// Exact logical installed source selected by native discovery. Runtime
    /// root paths are never persisted; saves/replays keep only its relative
    /// locator. Canonical browser-distributed launches use no installed source.
    pub installed_source: Option<crate::mission_asset_launch::InstalledMissionSource>,
    /// Browser launchers may already hold the fetched archive in memory.
    /// Native discovery leaves this `None` and reads `version_zip`.
    pub version_zip_bytes: Option<std::sync::Arc<[u8]>>,
    /// Exact selected `.rhm` path inside `version_zip`. This disambiguates
    /// multilingual archives that contain same-basename missions.
    pub rhm_zip_entry: String,
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
// Leave room below both panes for the Play/Cancel button stack.
const LIST_H: i32 = 360;
const DETAIL_X: i32 = 408;
const DETAIL_Y: i32 = 36;
const DETAIL_W: i32 = 218;
const DETAIL_H: i32 = LIST_H;
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
    files: &robin_engine::sbfile::SbFileSystem,
) -> Option<CustomMissionChoice> {
    let mods = scan_mission_roots(mods_root, crate::main_entry::overlay_mods_dir().as_deref());
    let entries = enumerate_missions(&mods, files);
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
    let mut list_view = ScrollView::new(
        [LIST_X + 4, LIST_Y + 4, LIST_W - 8, LIST_H - 8],
        ROW_HEIGHT,
        resources,
    );
    list_view.set_total(entries.len());
    list_view.reveal(selected);
    let visible_rows = list_view.visible_count();
    let font = resources
        .menu_text_font_any()
        .expect("custom missions requires a body font");
    let detail_line_h = (font.height() as i32).max(12) + 2;
    let mut detail_view = ScrollView::new(
        [DETAIL_X + 8, DETAIL_Y + 8, DETAIL_W - 16, DETAIL_H - 16],
        detail_line_h,
        resources,
    );
    let mut detail_lines =
        mission_detail_lines(font, &entries[selected], detail_view.content_width() - 4);
    detail_view.set_total(detail_lines.len());

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
        // ── Events ──────────────────────────────────────────────
        let mut activated: Option<u32> = None;
        let (events, transform) =
            crate::ingame_menu::layout::poll_events_with_transform(event_pump, renderer);
        for event in events {
            let previous_selected = selected;
            input_state.update_from_event(&event, transform);
            let pointer = (input_state.virt_x as i32, input_state.virt_y as i32);
            if list_view.handle_event(&event, transform, pointer)
                || detail_view.handle_event(&event, transform, pointer)
            {
                continue;
            }
            match event {
                GameEvent::Quit => activated = Some(ID_CANCEL),
                GameEvent::KeyDown { keycode, .. } => {
                    if let Some(action) =
                        mission_key_action(keycode, &mut selected, &entries, visible_rows)
                    {
                        activated = Some(action);
                    }
                }
                GameEvent::MouseUp(x, y, 1) => {
                    let (vx, vy) = transform.from_screen(x, y);
                    if let Some(target) = list_view.row_at(vx, vy) {
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
                _ => {}
            }
            if selected != previous_selected {
                detail_lines =
                    mission_detail_lines(font, &entries[selected], detail_view.content_width() - 4);
                detail_view.set_total(detail_lines.len());
                detail_view.reset();
            }
            // Only keyboard navigation reveals selection; wheel/drag scrolling
            // must remain independent of the currently selected mission.
            if matches!(
                event,
                GameEvent::KeyDown {
                    keycode: Keycode::Up
                        | Keycode::Down
                        | Keycode::PageUp
                        | Keycode::PageDown
                        | Keycode::Home
                        | Keycode::End,
                    ..
                }
            ) {
                list_view.reveal(selected);
            }
        }

        // Selection may have changed during this event batch. Update enablement
        // before widget processing and drawing, without resetting button state.
        frame
            .widget_mut(ID_PLAY)
            .expect("picker always has a Play button")
            .base_mut()
            .enabled = entries[selected].status.is_ok();
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
                        #[cfg(not(target_arch = "wasm32"))]
                        let installed_source =
                            match crate::mission_asset_launch::locate_installed_mission_source(
                                &e.version_zip,
                                mods_root,
                                crate::main_entry::overlay_mods_dir().as_deref(),
                            ) {
                                Ok(source) => Some(source),
                                Err(error) => {
                                    tracing::error!(
                                        archive = %e.version_zip.display(),
                                        "CustomMission: cannot retain installed source: {error}"
                                    );
                                    continue;
                                }
                            };
                        #[cfg(target_arch = "wasm32")]
                        let installed_source = None;
                        return Some(CustomMissionChoice::Mod(CustomMissionLaunch {
                            slug: e.mod_slug.clone(),
                            mod_title: e.mod_title.clone(),
                            claimed_author: e.author.clone(),
                            version: e.version_label.clone(),
                            source_url: e.source_url.clone(),
                            license: e.license.clone(),
                            version_zip: e.version_zip.clone(),
                            installed_source,
                            version_zip_bytes: None,
                            rhm_zip_entry: e.rhm_zip_entry.clone(),
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
            renderer, resources, transform, &entries, selected, &list_view,
        );
        draw_detail_pane(renderer, resources, transform, &detail_lines, &detail_view);
        widget_bridge::draw_frame_buttons(renderer, resources, transform, &frame);
        cursor.draw(renderer, transform, &input_state);
        renderer.present();
        crate::window::sleep_ui_frame().await;
    }
}

/// Apply keys against the current selection, including changes from earlier
/// events in this frame. The picker only runs with a nonempty entry list.
fn mission_key_action(
    key: Keycode,
    selected: &mut usize,
    entries: &[MissionEntry],
    visible_rows: usize,
) -> Option<u32> {
    match key {
        Keycode::Escape => return Some(ID_CANCEL),
        Keycode::Up => *selected = selected.saturating_sub(1),
        Keycode::Down => *selected = selected.saturating_add(1).min(entries.len() - 1),
        Keycode::PageUp => *selected = selected.saturating_sub(visible_rows),
        Keycode::PageDown => {
            *selected = selected.saturating_add(visible_rows).min(entries.len() - 1)
        }
        Keycode::Home => *selected = 0,
        Keycode::End => *selected = entries.len() - 1,
        Keycode::Return | Keycode::KpEnter if entries[*selected].status.is_ok() => {
            return Some(ID_PLAY);
        }
        _ => {}
    }
    None
}

fn draw_title(renderer: &mut Renderer, resources: &IngameMenuResources, transform: MenuTransform) {
    let Some(font) = resources
        .edit_field_font_any()
        .or_else(|| resources.menu_text_font_any())
    else {
        return;
    };
    let title = "Custom Missions";
    let tw = font.text_width(title);
    let x = (MENU_W - tw) / 2;
    render_text_virt_font(renderer, font, transform, title, x, TITLE_Y);
}

fn draw_list(
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    transform: MenuTransform,
    entries: &[MissionEntry],
    selected: usize,
    view: &ScrollView,
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

    for idx in view.visible_range() {
        let row_y = view.row_y(idx);
        let e = &entries[idx];
        let is_selected = idx == selected;
        let broken = !e.status.is_ok();

        // Selection highlight: thin fill bar so the eye can find the
        // current row even when the font choice is the same as
        // neighbours.
        if is_selected {
            let (sx0, sy0) = transform.to_screen(LIST_X + 2, row_y - 2);
            let (sx1, sy1) = transform.to_screen(
                LIST_X + 4 + view.content_width() - 2,
                row_y + ROW_HEIGHT - 4,
            );
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
        let Some(font) = resources.menu_text_font_any() else {
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
        // the detail pane. Keeps complete graphemes and adds an ellipsis if it
        // doesn't fit.
        let row_text_w = view.content_width() - 12;
        let label = elide_text_to_width_by(&label, row_text_w, false, |text| font.text_width(text));
        // Visual highlight of the selected row already drawn above; the
        // text colour is the same for selected/unselected, matching the
        // main menu profile info block.
        render_text_virt_font(renderer, font, transform, &label, LIST_X + 10, row_y);
    }
    view.draw_scrollbar(renderer, transform, resources);
}

fn draw_detail_pane(
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    transform: MenuTransform,
    lines: &[String],
    view: &ScrollView,
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
    let Some(font) = resources.menu_text_font_any() else {
        return;
    };

    for row in view.visible_range() {
        render_text_virt_font(
            renderer,
            font,
            transform,
            &lines[row],
            DETAIL_X + 10,
            view.row_y(row),
        );
    }
    view.draw_scrollbar(renderer, transform, resources);
}

fn mission_detail_lines(
    font: &crate::native_font::Font,
    entry: &MissionEntry,
    width: i32,
) -> Vec<String> {
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

    let mut wrapped = Vec::new();
    for line in lines {
        wrapped.extend(wrap_text_for_box_font(font, &line, width, usize::MAX).lines);
    }
    if !entry.description.trim().is_empty() {
        wrapped.push(String::new());
        wrapped.extend(wrap_text_for_box_font(font, &entry.description, width, usize::MAX).lines);
    }
    wrapped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mission_entry() -> MissionEntry {
        MissionEntry {
            mod_slug: String::new(),
            mod_title: "Multi-team Demos".into(),
            author: "Robin Rust port".into(),
            source_url: String::new(),
            license: String::new(),
            description: "Ten multi-team combat and diplomacy demos. ".repeat(30) + "END",
            map: "Open Battlefield".into(),
            requires_spellforge: false,
            version_label: "MultiTeamAllPcsCircle".into(),
            version_zip: PathBuf::new(),
            rhm_zip_entry: String::new(),
            rhm_basename: "MultiTeamAllPcsCircle".into(),
            hackable: true,
            status: MissionStatus::Broken {
                reason: "Unable to locate level MultiTeamAllPcsCircle.level".into(),
            },
            preview_image: None,
        }
    }

    #[test]
    fn keyboard_activation_uses_selection_from_previous_events() {
        let broken = mission_entry();
        let mut ready = mission_entry();
        ready.status = MissionStatus::Ok {
            map_filename: "demo.rhp".into(),
        };
        let entries = [broken, ready];
        let mut selected = 0;
        for enter in [Keycode::Return, Keycode::KpEnter] {
            assert_eq!(mission_key_action(enter, &mut selected, &entries, 5), None);
            assert_eq!(
                mission_key_action(Keycode::Down, &mut selected, &entries, 5),
                None
            );
            assert_eq!(selected, 1);
            assert_eq!(
                mission_key_action(enter, &mut selected, &entries, 5),
                Some(ID_PLAY)
            );
            assert_eq!(
                mission_key_action(Keycode::Up, &mut selected, &entries, 5),
                None
            );
            assert_eq!(selected, 0);
            assert_eq!(mission_key_action(enter, &mut selected, &entries, 5), None);
        }
        assert_eq!(
            mission_key_action(Keycode::Escape, &mut selected, &entries, 5),
            Some(ID_CANCEL)
        );
    }

    #[test]
    fn keyboard_navigation_stays_within_entries() {
        let entries = [mission_entry(), mission_entry(), mission_entry()];
        for page in [0, 1, 2, 10, usize::MAX] {
            for initial in 0..entries.len() {
                for (key, expected) in [
                    (Keycode::Up, initial.saturating_sub(1)),
                    (Keycode::Down, (initial + 1).min(2)),
                    (Keycode::PageUp, initial.saturating_sub(page)),
                    (Keycode::PageDown, initial.saturating_add(page).min(2)),
                    (Keycode::Home, 0),
                    (Keycode::End, 2),
                ] {
                    let mut selected = initial;
                    assert_eq!(mission_key_action(key, &mut selected, &entries, page), None);
                    assert_eq!(selected, expected);
                }
            }
        }
    }

    #[test]
    fn long_mission_details_wrap_without_dropping_status_or_description() {
        let name = [0; 32];
        let font = crate::native_font::Font::TrueType(crate::font::TrueTypeFont::from_parts(
            &name,
            18,
            0,
            0,
            &name,
            0,
            include_bytes!("../../../../assets/core-datadir/Data/Interface/Fonts/arial.ttf"),
        ));
        let entry = mission_entry();
        let width = 180;
        let lines = mission_detail_lines(&font, &entry, width);
        assert!(lines.len() > ((DETAIL_H - 16) / (font.height() as i32 + 2)) as usize);
        assert!(lines.iter().all(|line| font.text_width(line) <= width));
        let text = lines.join(" ");
        assert!(text.contains("Status:"));
        assert!(text.ends_with("END"));
    }
}
