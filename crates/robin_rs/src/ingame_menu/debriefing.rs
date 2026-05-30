//! Post-mission debriefing screen.
//!
//! A 496x463 parchment window showing the mission title and debriefing
//! body text.
//!
//! Buttons are driven by the [`crate::widget`] system via the
//! [`super::widget_bridge`].

use crate::gfx_types::Keycode;

use crate::gfx_types::GameEvent;
use crate::renderer::Renderer;

use super::layout::{
    MENU_H, MENU_W, MenuTransform, TextAlign, TooltipState, dim_screen, draw_background,
    enter_modal_gpu_phase, render_text_in_box,
};
use super::resources::{
    IngameMenuResources, MT_BTN_LOAD, MT_BTN_OK, MT_BTN_RESTART, MT_INFOBULLE_BUTTON_OK,
    MT_INFOBULLE_BUTTON_RECOMMENCER, MT_STR_DB_S06, MT_STR_DB_S07, MT_STR_DB_S08, MT_STR_DB_S09,
    MT_STR_DB_S10, MT_STR_DB_S11, MT_STR_DB_S13, MT_STR_DB_S17, MT_STR_DB_S18, MT_TTL_MISSION_LOST,
    MT_TTL_MISSION_WON, MenuText,
};
use super::widget_bridge::{self, ModalCursor, ModalInputState};
use robin_engine::mission_stat::MissionStat;

/// Virtual window geometry.
pub const WIN_W: i32 = 496;
pub const WIN_H: i32 = 463;

const TITLE_X: i32 = 50;
const TITLE_Y: i32 = 50;
const TITLE_W: i32 = 400;
// 400x150 title box with default left/top-aligned text placement.
// Keeps long localised titles wrapping inside the box instead of
// overflowing.
const TITLE_H: i32 = 150;

const BODY_X: i32 = 50;
const BODY_Y: i32 = 90;
const BODY_W: i32 = 400;
const BODY_H: i32 = 285;

const OK_BTN_Y: i32 = 384;

const BTN_OK: u32 = 0;
const BTN_RESTART: u32 = 1;
const BTN_LOAD: u32 = 2;

/// Which bitmap font to render the body text with.  The free-text body
/// page uses `PopupScroll`; the mission-stat panel uses `Debrief`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyFont {
    PopupScroll,
    Debrief,
}

/// What happened when the player dismissed the debriefing window.
///
/// `Ok { text_remaining }` is always empty for [`show_debriefing`]
/// callers — the entry point paginates body text internally.
///
/// `LoadAttempt`: the player clicked the Load button.  The caller is
/// expected to run the save-load picker; if a slot is selected it
/// should queue the load, and if the picker is cancelled it must
/// re-enter the debriefing via [`show_debriefing`] passing
/// `body_remaining` for `body` and the same `stat`, with
/// `start_at_stat` set to `was_on_stat` so the same page the player
/// was looking at when they clicked Load is re-shown.
///
/// `EmergencyEnd` is set when the menu is force-closed by an external
/// event — the trigger is `GameEvent::Quit` (the window close button
/// / Alt-F4).  Surfaced as a distinct outcome so the outer session
/// loop can propagate `GameCode::Quit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebriefingOutcome {
    Ok {
        text_remaining: String,
    },
    Restart,
    LoadAttempt {
        /// The body text the player was viewing when they clicked
        /// Load.  Used by the caller to re-enter `show_debriefing` if
        /// the picker is cancelled — feed this back as the new `body`.
        body_remaining: String,
        /// `true` if Load was clicked from the stat panel rather than
        /// the body page.  On picker cancel, the caller passes this
        /// back as `start_at_stat` so the body pagination is skipped
        /// and the stat panel is the first thing shown again.
        was_on_stat: bool,
    },
    EmergencyEnd,
}

/// Per-page outcome from [`show_one_page`].  The Load button click is
/// surfaced as `LoadClicked` so the surrounding pagination loop in
/// [`show_debriefing`] can run the save-game picker and then either
/// propagate a real [`DebriefingOutcome::Load`] (slot picked) or
/// continue the loop (picker cancelled).
#[derive(Debug, Clone, PartialEq, Eq)]
enum PageOutcome {
    Ok { text_remaining: String },
    Restart,
    LoadClicked,
    EmergencyEnd,
}

/// Resolve the debriefing title from the menu text table.
fn debriefing_title(resources: &IngameMenuResources, won: bool) -> String {
    let id = if won {
        MT_TTL_MISSION_WON
    } else {
        MT_TTL_MISSION_LOST
    };
    resources.menu_text.get(id)
}

/// Display the debriefing window, paginating through the body text.
///
///   1. Render the body text, paging on overflow until the body is
///      exhausted.
///   2. If the player didn't click Load, render the mission stat
///      panel as a follow-up page.
///
/// When `stat` is `Some`, the stat panel is shown as a follow-up page
/// after the body pagination completes (and only if Load wasn't
/// clicked).  Pass `None` to skip the stat panel — the cheat path
/// that displays the full debriefing vector doesn't render the stat
/// panel, so that caller passes `None`.
#[allow(clippy::too_many_arguments)]
pub async fn show_debriefing(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    mut cursor: Option<ModalCursor<'_>>,
    body: &str,
    stat: Option<&MissionStat>,
    mission_length_seconds: u32,
    won: bool,
    restart_allowed: bool,
    quick_load_scancode: Option<u16>,
    // Restart only triggers a load request when a restart snapshot
    // exists; when the snapshot is missing the body window closes and
    // the stat panel still shows.  The caller probes the save-manager
    // up front and passes the result here so the Restart click can
    // short-circuit to "skip body, show stat" instead of queueing a
    // no-op load request.
    restart_snapshot_exists: bool,
    // When `true`, skip the body pagination and start with the stat
    // panel.  Used by the caller to resume after a cancelled Load
    // picker on the stat phase, so the player stays on the page that
    // was visible when Load was clicked.
    start_at_stat: bool,
) -> DebriefingOutcome {
    let mut state = DebriefingModalState::new(
        resources,
        body.to_string(),
        stat,
        mission_length_seconds,
        won,
        restart_allowed,
        quick_load_scancode,
        restart_snapshot_exists,
        start_at_stat,
    );
    loop {
        if let Some(outcome) = state.tick(
            event_pump,
            renderer,
            resources,
            cursor.as_mut().map(|c| c.reborrow()),
        ) {
            return outcome;
        }
        crate::window::sleep_ms(16).await;
    }
}

enum DebriefingPhase {
    Body,
    Stat,
    Done,
}

/// One-frame state for a full debriefing flow: paginated body text
/// followed by an optional mission-stat page.
pub struct DebriefingModalState {
    title: String,
    remaining: String,
    stat_text: Option<String>,
    phase: DebriefingPhase,
    restart_allowed: bool,
    restart_snapshot_exists: bool,
    active_quick_load: Option<u16>,
    current_page: Option<DebriefingPageState>,
}

impl DebriefingModalState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        resources: &IngameMenuResources,
        body: String,
        stat: Option<&MissionStat>,
        mission_length_seconds: u32,
        won: bool,
        restart_allowed: bool,
        quick_load_scancode: Option<u16>,
        restart_snapshot_exists: bool,
        start_at_stat: bool,
    ) -> Self {
        let stat_text =
            stat.map(|s| format_mission_stat_text(s, mission_length_seconds, &resources.menu_text));
        Self {
            title: debriefing_title(resources, won),
            remaining: body,
            stat_text,
            phase: if start_at_stat {
                DebriefingPhase::Stat
            } else {
                DebriefingPhase::Body
            },
            restart_allowed,
            restart_snapshot_exists,
            active_quick_load: restart_allowed.then_some(quick_load_scancode).flatten(),
            current_page: None,
        }
    }

    pub fn tick(
        &mut self,
        event_pump: &mut crate::window::GameWindow,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<ModalCursor<'_>>,
    ) -> Option<DebriefingOutcome> {
        match self.phase {
            DebriefingPhase::Body => {
                if self.current_page.is_none() {
                    self.current_page = Some(DebriefingPageState::new(
                        event_pump,
                        renderer,
                        resources,
                        self.title.clone(),
                        self.remaining.clone(),
                        self.restart_allowed,
                        self.restart_snapshot_exists,
                        BodyFont::PopupScroll,
                        self.active_quick_load,
                    ));
                }
                let outcome = self
                    .current_page
                    .as_mut()
                    .and_then(|page| page.tick(event_pump, renderer, resources, cursor));
                let outcome = outcome?;
                self.current_page = None;
                match outcome {
                    PageOutcome::Ok { text_remaining } => {
                        if text_remaining.is_empty() || text_remaining == self.remaining {
                            self.phase = DebriefingPhase::Stat;
                        } else {
                            self.remaining = text_remaining;
                        }
                    }
                    PageOutcome::Restart => return Some(DebriefingOutcome::Restart),
                    PageOutcome::LoadClicked => {
                        return Some(DebriefingOutcome::LoadAttempt {
                            body_remaining: self.remaining.clone(),
                            was_on_stat: false,
                        });
                    }
                    PageOutcome::EmergencyEnd => return Some(DebriefingOutcome::EmergencyEnd),
                }
                None
            }
            DebriefingPhase::Stat => {
                let Some(stat_text) = self.stat_text.clone() else {
                    self.phase = DebriefingPhase::Done;
                    return Some(DebriefingOutcome::Ok {
                        text_remaining: String::new(),
                    });
                };
                if self.current_page.is_none() {
                    self.current_page = Some(DebriefingPageState::new(
                        event_pump,
                        renderer,
                        resources,
                        self.title.clone(),
                        stat_text,
                        self.restart_allowed,
                        self.restart_snapshot_exists,
                        BodyFont::Debrief,
                        self.active_quick_load,
                    ));
                }
                let outcome = self
                    .current_page
                    .as_mut()
                    .and_then(|page| page.tick(event_pump, renderer, resources, cursor));
                let outcome = outcome?;
                self.current_page = None;
                match outcome {
                    PageOutcome::Ok { .. } => self.phase = DebriefingPhase::Done,
                    PageOutcome::Restart => return Some(DebriefingOutcome::Restart),
                    PageOutcome::LoadClicked => {
                        return Some(DebriefingOutcome::LoadAttempt {
                            body_remaining: self.remaining.clone(),
                            was_on_stat: true,
                        });
                    }
                    PageOutcome::EmergencyEnd => return Some(DebriefingOutcome::EmergencyEnd),
                }
                None
            }
            DebriefingPhase::Done => Some(DebriefingOutcome::Ok {
                text_remaining: String::new(),
            }),
        }
    }
}

/// Replace printf placeholders in `template` with successive values from
/// `values`.  Handles `%u` / `%lu` / `%i` / `%d` (integer) and `%s` /
/// `%ls` (string) — these are the specs that appear in the menu-text
/// resource templates we consume.
///
/// If `template` has fewer placeholders than `values.len()`, extra
/// values are dropped silently; if it has more, the extras are left
/// as-is in the output.
fn substitute_printf(template: &str, values: &[&str]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut idx = 0;
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        // Lookahead at the conversion spec.
        let mut spec = String::new();
        if let Some(&'l') = chars.peek() {
            spec.push('l');
            chars.next();
        }
        match chars.peek() {
            Some(&'u') | Some(&'i') | Some(&'d') | Some(&'s') => {
                spec.push(*chars.peek().unwrap());
                chars.next();
                if idx < values.len() {
                    out.push_str(values[idx]);
                    idx += 1;
                } else {
                    out.push('%');
                    out.push_str(&spec);
                }
            }
            _ => {
                out.push('%');
                out.push_str(&spec);
            }
        }
    }
    out
}

/// Format "HH:MM" (no seconds).
fn seconds_to_hms(total: u32) -> String {
    let h = total / 3600;
    let m = (total % 3600) / 60;
    format!("{h:02}:{m:02}")
}

/// Build the stat-panel body text from a [`MissionStat`] and the menu
/// text table.  Assembly order:
///
/// ```text
///   Money               [S06 if collected != 0] [S18 if bonus|soldier != 0]
///   (blank line)
///   Soldiers            [S07]
///   New peasants        [S08]
///   New PCs             "<name> S09" per PC joined
///   Killed              [S10] [S17 if killed_allied != 0]
///   (blank line)
///   Score               [S11]
///   Length              [S13]
/// ```
pub fn format_mission_stat_text(
    stat: &MissionStat,
    mission_length_seconds: u32,
    menu_text: &MenuText,
) -> String {
    let mut out = String::new();

    // Money.
    if stat.collected_money != 0 {
        let s = substitute_printf(
            &menu_text.get(MT_STR_DB_S06),
            &[&stat.collected_money.to_string()],
        );
        out.push_str(&s);
        out.push('\n');
    }
    if stat.bonus_money != 0 || stat.soldier_money != 0 {
        let total = stat.bonus_money + stat.soldier_money;
        let s = substitute_printf(
            &menu_text.get(MT_STR_DB_S18),
            &[
                &total.to_string(),
                &stat.bonus_money.to_string(),
                &stat.soldier_money.to_string(),
            ],
        );
        out.push_str(&s);
        out.push('\n');
    }
    out.push('\n');

    // Soldier count (always).
    let s = substitute_printf(
        &menu_text.get(MT_STR_DB_S07),
        &[
            &stat.living_soldier_count.to_string(),
            &stat.total_soldier_count.to_string(),
        ],
    );
    out.push_str(&s);
    out.push('\n');

    // New members (peasants + PCs).
    let s = substitute_printf(
        &menu_text.get(MT_STR_DB_S08),
        &[&stat.total_new_members().to_string()],
    );
    out.push_str(&s);
    out.push('\n');

    // PCs who joined the gang — "<name> S09" per joined PC.
    // Renames performed by the script (PROP_NAME) are captured as a
    // SPECIAL_PEASANT slot id alongside the profile-name fallback, so
    // we resolve the localized override here against the same
    // menu-text table the rest of the screen already uses.
    let joined_suffix = menu_text.get(MT_STR_DB_S09);
    for entry in &stat.pc_names {
        let display = match entry.name_override {
            Some(slot) => {
                let resolved = menu_text.get(slot.menu_text_id());
                if resolved.is_empty() {
                    entry.fallback.clone()
                } else {
                    resolved
                }
            }
            None => entry.fallback.clone(),
        };
        out.push_str(&display);
        out.push(' ');
        out.push_str(&joined_suffix);
        out.push('\n');
    }

    // Killed (peasants always; allied only if non-zero).
    let s = substitute_printf(
        &menu_text.get(MT_STR_DB_S10),
        &[&stat.killed_peasant_count.to_string()],
    );
    out.push_str(&s);
    out.push('\n');
    if stat.killed_allied_count != 0 {
        let s = substitute_printf(
            &menu_text.get(MT_STR_DB_S17),
            &[&stat.killed_allied_count.to_string()],
        );
        out.push_str(&s);
        out.push('\n');
    }
    out.push('\n');

    // Score + length.
    let s = substitute_printf(
        &menu_text.get(MT_STR_DB_S11),
        &[&stat.added_score.to_string()],
    );
    out.push_str(&s);
    out.push('\n');
    let length_str = seconds_to_hms(mission_length_seconds);
    let s = substitute_printf(&menu_text.get(MT_STR_DB_S13), &[&length_str]);
    out.push_str(&s);
    out.push('\n');

    out
}

struct DebriefingPageState {
    title: String,
    body: String,
    restart_snapshot_exists: bool,
    body_font: BodyFont,
    quick_load_scancode: Option<u16>,
    transform: MenuTransform,
    virt_x: i32,
    virt_y: i32,
    frame: crate::widget::FrameWnd,
    input_state: ModalInputState,
    tooltip: TooltipState,
    text_remaining: String,
}

impl DebriefingPageState {
    #[allow(clippy::too_many_arguments)]
    fn new(
        event_pump: &crate::window::GameWindow,
        renderer: &Renderer,
        resources: &IngameMenuResources,
        title: String,
        body: String,
        restart_allowed: bool,
        restart_snapshot_exists: bool,
        body_font: BodyFont,
        quick_load_scancode: Option<u16>,
    ) -> Self {
        let sw = renderer.screen_width() as i32;
        let sh = renderer.screen_height() as i32;
        let transform = MenuTransform::centered(sw, sh);
        let virt_x = (MENU_W - WIN_W) / 2;
        let virt_y = (MENU_H - WIN_H) / 2;
        let (btn_w, btn_h) = resources.button_dimensions();
        let ok_label = resources.menu_text.get(MT_BTN_OK);
        let restart_label = resources.menu_text.get(MT_BTN_RESTART);
        let load_label = resources.menu_text.get(MT_BTN_LOAD);

        let mut frame = crate::widget::FrameWnd::default();
        frame.enabled = true;
        frame.input_enabled = true;
        let btn_y = virt_y + OK_BTN_Y;
        let restart_x = virt_x + 50;
        let load_x = virt_x + 100;
        let ok_x = virt_x + (WIN_W - btn_w) / 2;
        frame.add_widget_absolute(widget_bridge::make_button(
            BTN_OK, &ok_label, ok_x, btn_y, btn_w, btn_h,
        ));
        if restart_allowed {
            frame.add_widget_absolute(widget_bridge::make_button(
                BTN_RESTART,
                &restart_label,
                restart_x,
                btn_y,
                btn_w,
                btn_h,
            ));
            frame.add_widget_absolute(widget_bridge::make_button(
                BTN_LOAD,
                &load_label,
                load_x,
                btn_y,
                btn_w,
                btn_h,
            ));
        }

        let ok_tooltip = resources.menu_text.get(MT_INFOBULLE_BUTTON_OK);
        let restart_tooltip = resources.menu_text.get(MT_INFOBULLE_BUTTON_RECOMMENCER);
        let load_tooltip = resources.menu_text.get(MT_BTN_LOAD);
        if let Some(w) = frame.widget_mut(BTN_OK) {
            w.base_mut().set_tooltip_text(&ok_tooltip);
        }
        if let Some(w) = frame.widget_mut(BTN_RESTART) {
            w.base_mut().set_tooltip_text(&restart_tooltip);
        }
        if let Some(w) = frame.widget_mut(BTN_LOAD) {
            w.base_mut().set_tooltip_text(&load_tooltip);
        }

        let mut input_state = ModalInputState::new();
        input_state.seed_mouse_from_sdl(event_pump, transform);
        Self {
            title,
            body,
            restart_snapshot_exists,
            body_font,
            quick_load_scancode,
            transform,
            virt_x,
            virt_y,
            frame,
            input_state,
            tooltip: TooltipState::new(),
            text_remaining: String::new(),
        }
    }

    fn tick(
        &mut self,
        event_pump: &mut crate::window::GameWindow,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<ModalCursor<'_>>,
    ) -> Option<PageOutcome> {
        let mut outcome = None;
        for event in event_pump.poll_events() {
            self.input_state.update_from_event(&event, self.transform);
            match event {
                GameEvent::Quit => outcome = Some(PageOutcome::EmergencyEnd),
                GameEvent::KeyDown {
                    keycode: Keycode::Return,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::KpEnter,
                    ..
                } => {
                    outcome = Some(PageOutcome::Ok {
                        text_remaining: self.text_remaining.clone(),
                    });
                }
                GameEvent::KeyDown { scancode, .. }
                    if Some(scancode) == self.quick_load_scancode =>
                {
                    outcome = Some(PageOutcome::LoadClicked);
                }
                _ => {}
            }
        }

        let widget_input = self.input_state.as_widget_input();
        let events = self.frame.process_input(&widget_input);
        self.input_state.end_frame();
        if let Some(id) = widget_bridge::find_activated(&events) {
            outcome = Some(match id {
                BTN_OK => PageOutcome::Ok {
                    text_remaining: self.text_remaining.clone(),
                },
                BTN_RESTART => {
                    if self.restart_snapshot_exists {
                        PageOutcome::Restart
                    } else {
                        tracing::warn!(
                            "Debriefing Restart clicked but no restart snapshot exists; \
                             falling through to stat panel"
                        );
                        PageOutcome::Ok {
                            text_remaining: String::new(),
                        }
                    }
                }
                BTN_LOAD => PageOutcome::LoadClicked,
                _ => PageOutcome::Ok {
                    text_remaining: String::new(),
                },
            });
        }

        self.render(renderer, resources, cursor.as_ref());
        renderer.present();
        outcome
    }

    fn render(
        &mut self,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
    ) {
        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);

        if let Some(bg) = resources.parchment_huge {
            draw_background(
                renderer,
                self.transform,
                &bg,
                self.virt_x,
                self.virt_y,
                WIN_W,
                WIN_H,
            );
        }

        if let Some(font) = resources.title_font() {
            render_text_in_box(
                renderer,
                font,
                self.transform,
                &self.title,
                self.virt_x + TITLE_X,
                self.virt_y + TITLE_Y,
                TITLE_W,
                TITLE_H,
                TextAlign::Center,
            );
        }

        let body_font_ref = match self.body_font {
            BodyFont::PopupScroll => resources.popup_font(),
            BodyFont::Debrief => resources.debrief_font(),
        };
        if let Some(font) = body_font_ref {
            self.text_remaining = render_text_in_box(
                renderer,
                font,
                self.transform,
                &self.body,
                self.virt_x + BODY_X,
                self.virt_y + BODY_Y,
                BODY_W,
                BODY_H,
                TextAlign::Justified,
            );
        }

        widget_bridge::draw_frame_buttons(renderer, resources, self.transform, &self.frame);

        let mouse_pt = robin_engine::coordinates::ScreenPoint::new(
            self.input_state.virt_x,
            self.input_state.virt_y,
        );
        self.tooltip.update(&self.frame, mouse_pt);
        if let Some(font) = body_font_ref {
            self.tooltip
                .draw(renderer, font, self.transform, &self.frame, mouse_pt);
        }

        if let Some(c) = cursor {
            c.draw(renderer, self.transform, &self.input_state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pagination control flow is the interesting part — exercise it
    /// with a mock page-producer so we don't have to spin up a renderer.
    ///   - keep feeding `text_remaining` back in until it's empty or
    ///     identical to the input (defensive cycle break);
    ///   - short-circuit on Load / Restart.
    fn paginate<F>(body: &str, mut produce: F) -> PageOutcome
    where
        F: FnMut(&str) -> PageOutcome,
    {
        let mut remaining = body.to_string();
        loop {
            let outcome = produce(&remaining);
            match outcome {
                PageOutcome::Ok { text_remaining } => {
                    if text_remaining.is_empty() || text_remaining == remaining {
                        return PageOutcome::Ok {
                            text_remaining: String::new(),
                        };
                    }
                    remaining = text_remaining;
                }
                PageOutcome::Restart => return PageOutcome::Restart,
                PageOutcome::LoadClicked => return PageOutcome::LoadClicked,
                PageOutcome::EmergencyEnd => return PageOutcome::EmergencyEnd,
            }
        }
    }

    #[test]
    fn pagination_short_circuits_on_emergency_end() {
        let mut calls = 0;
        let result = paginate("line1\nline2", |_| {
            calls += 1;
            PageOutcome::EmergencyEnd
        });
        assert_eq!(result, PageOutcome::EmergencyEnd);
        assert_eq!(calls, 1);
    }

    #[test]
    fn pagination_exhausts_overflow() {
        // Three-page body: each page hands back the next page's text.
        let pages = std::cell::RefCell::new(vec![
            "page 2\npage 3".to_string(),
            "page 3".to_string(),
            String::new(),
        ]);
        let mut visited = Vec::new();
        let result = paginate("page 1\npage 2\npage 3", |input| {
            visited.push(input.to_string());
            let next = pages.borrow_mut().remove(0);
            PageOutcome::Ok {
                text_remaining: next,
            }
        });
        assert_eq!(
            result,
            PageOutcome::Ok {
                text_remaining: String::new()
            }
        );
        assert_eq!(visited.len(), 3);
        assert_eq!(visited[0], "page 1\npage 2\npage 3");
        assert_eq!(visited[1], "page 2\npage 3");
        assert_eq!(visited[2], "page 3");
    }

    #[test]
    fn pagination_short_circuits_on_load_clicked() {
        let mut calls = 0;
        let result = paginate("line1\nline2", |_| {
            calls += 1;
            PageOutcome::LoadClicked
        });
        assert_eq!(result, PageOutcome::LoadClicked);
        assert_eq!(calls, 1);
    }

    #[test]
    fn pagination_short_circuits_on_restart() {
        let mut calls = 0;
        let result = paginate("line1\nline2", |_| {
            calls += 1;
            PageOutcome::Restart
        });
        assert_eq!(result, PageOutcome::Restart);
        assert_eq!(calls, 1);
    }

    #[test]
    fn pagination_breaks_on_identical_remainder() {
        // If `render_text_in_box` somehow hands back the same body (e.g.
        // the box is too small for even one line), the defensive
        // `text_remaining == remaining` guard must stop the loop so we
        // don't spin forever.
        let mut calls = 0;
        let result = paginate("single page", |input| {
            calls += 1;
            PageOutcome::Ok {
                text_remaining: input.to_string(),
            }
        });
        assert_eq!(
            result,
            PageOutcome::Ok {
                text_remaining: String::new()
            }
        );
        assert_eq!(calls, 1);
    }

    #[test]
    fn substitute_printf_handles_common_specs() {
        assert_eq!(substitute_printf("Score: %u", &["42"]), "Score: 42");
        assert_eq!(substitute_printf("Score: %lu", &["42"]), "Score: 42");
        assert_eq!(substitute_printf("Score: %i", &["42"]), "Score: 42");
        assert_eq!(substitute_printf("Score: %d", &["42"]), "Score: 42");
        assert_eq!(
            substitute_printf("Time: %s", &["01:23:45"]),
            "Time: 01:23:45"
        );
        assert_eq!(
            substitute_printf("Time: %ls", &["01:23:45"]),
            "Time: 01:23:45"
        );
        assert_eq!(
            substitute_printf("Money %u (bonus %u, loot %u)", &["300", "100", "200"]),
            "Money 300 (bonus 100, loot 200)"
        );
    }

    #[test]
    fn substitute_printf_leaves_extras_untouched() {
        assert_eq!(substitute_printf("%u %u", &["42"]), "42 %u");
    }

    #[test]
    fn format_mission_stat_exercises_each_section() {
        let menu_text = MenuText::english_fallbacks_only();
        let stat = MissionStat {
            collected_money: 100,
            bonus_money: 50,
            soldier_money: 25,
            living_soldier_count: 3,
            total_soldier_count: 10,
            new_peasant_count: 2,
            killed_peasant_count: 1,
            killed_allied_count: 1,
            added_score: 500,
            pc_names: vec![robin_engine::mission_stat::PcStatName::new(
                "Little John".into(),
                None,
            )],
        };
        let text = format_mission_stat_text(&stat, 3725, &menu_text);
        // Money section (both lines present).
        assert!(text.contains("You collected 100"));
        assert!(text.contains("Found 75 gold pieces (bonuses: 50, soldiers: 25)"));
        // Soldier section.
        assert!(text.contains("3 of 10 enemy soldiers"));
        // Peasants + new members (2 peasants + 1 PC = 3).
        assert!(text.contains("3 new gang members"));
        // PC joined suffix.
        assert!(text.contains("Little John joined"));
        // Killed.
        assert!(text.contains("1 peasants were killed"));
        assert!(text.contains("1 allied soldiers"));
        // Score + length.
        assert!(text.contains("Score: 500"));
        assert!(text.contains("01:02"));
    }

    #[test]
    fn format_mission_stat_skips_zero_money_lines() {
        let menu_text = MenuText::english_fallbacks_only();
        let stat = MissionStat::default();
        let text = format_mission_stat_text(&stat, 0, &menu_text);
        // Money lines are conditional; none should appear when all zero.
        assert!(!text.contains("collected"));
        assert!(!text.contains("Found"));
        // Allied kill line is also conditional.
        assert!(!text.contains("allied"));
        // Soldier / peasants / score / length always render.
        assert!(text.contains("0 of 0 enemy"));
        assert!(text.contains("0 new gang members"));
        assert!(text.contains("Score: 0"));
        assert!(text.contains("00:00"));
    }

    #[test]
    fn format_mission_stat_resolves_pc_name_override() {
        // PROP_NAME-renamed PC: the SPECIAL_PEASANT slot resolves
        // through the menu-text table at render time.
        let mut menu_text = MenuText::english_fallbacks_only();
        // Slot 250/251/252 are the SPECIAL_PEASANT_A/B/C ids; inject
        // stand-in strings so the override path has something to
        // resolve to.  default_fallbacks() doesn't ship these (they
        // come from the localised `.sxt`), so we plug them in by
        // hand.
        let mut strings: Vec<String> = vec![String::new(); 253];
        strings[250] = "Aelfric".into();
        strings[251] = "Beornred".into();
        strings[252] = "Cuthbert".into();
        menu_text.replace_strings_for_test(strings);
        let stat = MissionStat {
            pc_names: vec![
                robin_engine::mission_stat::PcStatName::new(
                    "Robin des bois".into(),
                    Some(robin_engine::pc_status::SpecialPeasantName::A),
                ),
                robin_engine::mission_stat::PcStatName::new("Little John".into(), None),
            ],
            ..Default::default()
        };
        let text = format_mission_stat_text(&stat, 0, &menu_text);
        // The SPECIAL_PEASANT_A resolved string should appear, not the
        // raw "Robin des bois" profile-name fallback.
        assert!(text.contains("Aelfric"));
        assert!(!text.contains("Robin des bois"));
        // Override-less PCs still render via fallback.
        assert!(text.contains("Little John"));
    }

    #[test]
    fn format_mission_stat_falls_back_when_override_resolves_empty() {
        // Edge case: menu-text table is loaded but the SPECIAL_PEASANT
        // slot resolves to an empty string (e.g. the localized `.sxt`
        // is missing the entry).  The renderer should fall back to
        // the profile-name string rather than emit a blank.
        let menu_text = MenuText::english_fallbacks_only();
        let stat = MissionStat {
            pc_names: vec![robin_engine::mission_stat::PcStatName::new(
                "Robin des bois".into(),
                Some(robin_engine::pc_status::SpecialPeasantName::C),
            )],
            ..Default::default()
        };
        let text = format_mission_stat_text(&stat, 0, &menu_text);
        assert!(text.contains("Robin des bois"));
    }

    #[test]
    fn pagination_stops_immediately_on_empty_remainder() {
        let mut calls = 0;
        let result = paginate("short text", |_| {
            calls += 1;
            PageOutcome::Ok {
                text_remaining: String::new(),
            }
        });
        assert_eq!(
            result,
            PageOutcome::Ok {
                text_remaining: String::new()
            }
        );
        assert_eq!(calls, 1);
    }
}
