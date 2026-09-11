//! Post-mission debriefing screen.
//!
//! A 496x463 parchment window showing the mission title and debriefing
//! body text.
//!
//! Buttons are driven by the [`crate::widget`] system via the
//! [`super::widget_bridge`].

use crate::gfx_types::Keycode;
use crate::scroll_view::ScrollView;
use robin_engine::coordinates as engine_coordinates;
#[cfg(test)]
use robin_engine::mission_stat as engine_mission_stat;
use robin_engine::mission_stat::MissionStat;
#[cfg(test)]
use robin_engine::pc_status as engine_pc_status;

use crate::gfx_types::GameEvent;
use crate::native_font::Font;
use crate::renderer::Renderer;
use crate::widget::FrameWnd;

use super::layout::{
    MENU_H, MENU_W, MenuTransform, TextAlign, TooltipState, WrappedLine, dim_screen,
    draw_background, enter_modal_gpu_phase, render_text_in_box_font,
};
use super::resources::{
    IngameMenuResources, MT_BTN_LOAD, MT_INFOBULLE_BUTTON_OK, MT_INFOBULLE_BUTTON_RECOMMENCER,
    MT_STR_DB_S06, MT_STR_DB_S07, MT_STR_DB_S08, MT_STR_DB_S09, MT_STR_DB_S10, MT_STR_DB_S11,
    MT_STR_DB_S13, MT_STR_DB_S17, MT_STR_DB_S18, MT_TTL_MISSION_LOST, MT_TTL_MISSION_WON, MenuText,
};
use super::widget_bridge::{self, ModalCursor, ModalInputState};

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
/// `Ok` completes the flow; the complete body is available through scrolling.
///
/// `LoadAttempt`: the player clicked the Load button.  The caller is
/// expected to run the save-load picker; if a slot is selected it
/// should queue the load, and if the picker is cancelled it must
/// re-enter the debriefing via [`show_debriefing`] passing
/// `body` for `body` and the same `stat`, with
/// `start_at_stat` set to `was_on_stat` so the same page the player
/// was looking at when they clicked Load is re-shown.
///
/// `EmergencyEnd` is set when the menu is force-closed by an external
/// event — the trigger is `GameEvent::Quit` (the window close button
/// / Alt-F4).  Surfaced as a distinct outcome so the outer session
/// loop can propagate `GameCode::Quit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebriefingOutcome {
    Ok,
    Restart,
    LoadAttempt {
        /// The body text the player was viewing when they clicked
        /// Load.  Used by the caller to re-enter `show_debriefing` if
        /// the picker is cancelled — feed this back as the new `body`.
        body: String,
        /// `true` if Load was clicked from the stat panel rather than
        /// the body page.  On picker cancel, the caller passes this
        /// back as `start_at_stat` so the body page is skipped
        /// and the stat panel is the first thing shown again.
        was_on_stat: bool,
    },
    EmergencyEnd,
}

/// Outcome of one scrollable body or statistics page.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PageOutcome {
    Ok,
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

/// Display the debriefing window.
///
///   1. Render the complete, scrollable body text.
///   2. If the player didn't click Load, render the mission stat
///      panel as a follow-up page.
///
/// When `stat` is `Some`, the stat panel is shown as a follow-up page
/// after the body page completes (and only if Load wasn't
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
    quick_load_key: Option<winit::keyboard::KeyCode>,
    // Restart only triggers a load request when a restart snapshot
    // exists; when the snapshot is missing the body window closes and
    // the stat panel still shows.  The caller probes the save-manager
    // up front and passes the result here so the Restart click can
    // short-circuit to "skip body, show stat" instead of queueing a
    // no-op load request.
    restart_snapshot_exists: bool,
    // When `true`, skip the body page and start with the stat
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
        quick_load_key,
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
        crate::window::sleep_ui_frame().await;
    }
}

enum DebriefingPhase {
    Body,
    Stat,
    Done,
}

/// One-frame state for a full debriefing flow: scrollable body text
/// followed by an optional mission-stat page.
pub struct DebriefingModalState {
    title: String,
    body: String,
    stat_text: Option<String>,
    phase: DebriefingPhase,
    restart_allowed: bool,
    restart_snapshot_exists: bool,
    active_quick_load: Option<winit::keyboard::KeyCode>,
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
        quick_load_key: Option<winit::keyboard::KeyCode>,
        restart_snapshot_exists: bool,
        start_at_stat: bool,
    ) -> Self {
        let stat_text =
            stat.map(|s| format_mission_stat_text(s, mission_length_seconds, &resources.menu_text));
        Self {
            title: debriefing_title(resources, won),
            body,
            stat_text,
            phase: if start_at_stat {
                DebriefingPhase::Stat
            } else {
                DebriefingPhase::Body
            },
            restart_allowed,
            restart_snapshot_exists,
            active_quick_load: restart_allowed.then_some(quick_load_key).flatten(),
            current_page: None,
        }
    }

    /// Scripted replay batches begin on the body and await one recorded result;
    /// physical buttons must not advance or finish them ahead of that result.
    pub(crate) fn render_scripted_replay_wait(
        &mut self,
        event_pump: &mut crate::window::GameWindow,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
    ) {
        assert!(
            matches!(self.phase, DebriefingPhase::Body),
            "scripted replay debriefing advanced without recorded control"
        );
        let page = self.current_page.get_or_insert_with(|| {
            DebriefingPageState::new(
                event_pump,
                renderer,
                resources,
                self.title.clone(),
                self.body.clone(),
                self.restart_allowed,
                self.restart_snapshot_exists,
                BodyFont::PopupScroll,
                self.active_quick_load,
            )
        });
        let (_, transform) = super::layout::poll_events_with_transform(event_pump, renderer);
        page.transform = transform;
        let (font, lines) = page.prepare_body(resources);
        page.render(renderer, resources, cursor, font, &lines);
        renderer.present();
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
                        self.body.clone(),
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
                self.finish_page(outcome)
            }
            DebriefingPhase::Stat => {
                let Some(stat_text) = self.stat_text.as_ref() else {
                    self.phase = DebriefingPhase::Done;
                    return Some(DebriefingOutcome::Ok);
                };
                if self.current_page.is_none() {
                    self.current_page = Some(DebriefingPageState::new(
                        event_pump,
                        renderer,
                        resources,
                        self.title.clone(),
                        stat_text.clone(),
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
                self.finish_page(outcome)
            }
            DebriefingPhase::Done => Some(DebriefingOutcome::Ok),
        }
    }

    /// Settle one actual scrollable page. The body is never consumed in pieces:
    /// keep it intact so cancelling Load can reconstruct the same phase.
    fn finish_page(&mut self, outcome: PageOutcome) -> Option<DebriefingOutcome> {
        self.current_page = None;
        match outcome {
            PageOutcome::Ok => {
                self.phase = match self.phase {
                    DebriefingPhase::Body => DebriefingPhase::Stat,
                    DebriefingPhase::Stat => DebriefingPhase::Done,
                    DebriefingPhase::Done => unreachable!("completed debriefing has no page"),
                };
                None
            }
            PageOutcome::Restart => Some(DebriefingOutcome::Restart),
            PageOutcome::LoadClicked => Some(DebriefingOutcome::LoadAttempt {
                body: self.body.clone(),
                was_on_stat: matches!(self.phase, DebriefingPhase::Stat),
            }),
            PageOutcome::EmergencyEnd => Some(DebriefingOutcome::EmergencyEnd),
        }
    }
}

/// Append template text, replacing printf placeholders with successive values from
/// `values`.  Handles `%u` / `%lu` / `%i` / `%d` (integer) and `%s` /
/// `%ls` (string) — these are the specs that appear in the menu-text
/// resource templates we consume.
///
/// If `template` has fewer placeholders than `values.len()`, extra
/// values are dropped silently; if it has more, the extras are left
/// as-is in the output.
fn append_printf(out: &mut String, template: &str, values: &[&str]) {
    let mut values = values.iter();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let long = chars.next_if_eq(&'l').is_some();
        match chars.peek().copied() {
            Some(spec @ ('u' | 'i' | 'd' | 's')) => {
                chars.next();
                if let Some(value) = values.next() {
                    out.push_str(value);
                } else {
                    out.push('%');
                    if long {
                        out.push('l');
                    }
                    out.push(spec);
                }
            }
            _ => {
                out.push('%');
                if long {
                    out.push('l');
                }
            }
        }
    }
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

    // The legacy unsigned field accumulates signed changes, including spending.
    // Interpret its net value as signed instead of displaying a wrapped loss.
    if stat.collected_money != 0 {
        append_printf(
            &mut out,
            &menu_text.get(MT_STR_DB_S06),
            &[&(stat.collected_money as i32).to_string()],
        );
        out.push('\n');
    }
    if stat.bonus_money != 0 || stat.soldier_money != 0 {
        let total = stat.bonus_money + stat.soldier_money;
        append_printf(
            &mut out,
            &menu_text.get(MT_STR_DB_S18),
            &[
                &total.to_string(),
                &stat.bonus_money.to_string(),
                &stat.soldier_money.to_string(),
            ],
        );
        out.push('\n');
    }
    out.push('\n');

    // Soldier count (always).
    append_printf(
        &mut out,
        &menu_text.get(MT_STR_DB_S07),
        &[
            &stat.living_soldier_count.to_string(),
            &stat.total_soldier_count.to_string(),
        ],
    );
    out.push('\n');

    // New members (peasants + PCs).
    append_printf(
        &mut out,
        &menu_text.get(MT_STR_DB_S08),
        &[&stat.total_new_members().to_string()],
    );
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
    append_printf(
        &mut out,
        &menu_text.get(MT_STR_DB_S10),
        &[&stat.killed_peasant_count.to_string()],
    );
    out.push('\n');
    if stat.killed_allied_count != 0 {
        append_printf(
            &mut out,
            &menu_text.get(MT_STR_DB_S17),
            &[&stat.killed_allied_count.to_string()],
        );
        out.push('\n');
    }
    out.push('\n');

    // Score + length.
    append_printf(
        &mut out,
        &menu_text.get(MT_STR_DB_S11),
        &[&stat.added_score.to_string()],
    );
    out.push('\n');
    let length_str = seconds_to_hms(mission_length_seconds);
    append_printf(&mut out, &menu_text.get(MT_STR_DB_S13), &[&length_str]);
    out.push('\n');

    out
}

struct DebriefingPageState {
    title: String,
    body: String,
    restart_snapshot_exists: bool,
    body_font: BodyFont,
    quick_load_key: Option<winit::keyboard::KeyCode>,
    transform: MenuTransform,
    virt_x: i32,
    virt_y: i32,
    frame: FrameWnd,
    input_state: ModalInputState,
    tooltip: TooltipState,
    scroll_view: ScrollView,
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
        quick_load_key: Option<winit::keyboard::KeyCode>,
    ) -> Self {
        let sw = renderer.screen_width() as i32;
        let sh = renderer.screen_height() as i32;
        let transform = MenuTransform::centered(sw, sh);
        let virt_x = (MENU_W - WIN_W) / 2;
        let virt_y = (MENU_H - WIN_H) / 2;
        // The original debriefing uses the round `RHID_OK` seal
        // (centred) plus dedicated `RHID_RESTART` / `RHID_LOAD` seal
        // sprites at fixed x = 50 / 100, all label-less.
        let (ok_w, ok_h) = resources.ok_button_dimensions();
        let (restart_w, restart_h) = resources.restart_button_dimensions();
        let (load_w, load_h) = resources.load_button_dimensions();

        let mut frame = FrameWnd::default();
        frame.enabled = true;
        frame.input_enabled = true;
        let btn_y = virt_y + OK_BTN_Y;
        let restart_x = virt_x + 50;
        let load_x = virt_x + 100;
        let ok_x = virt_x + (WIN_W - ok_w) / 2;
        frame.add_widget_absolute(widget_bridge::make_button_with_resource(
            BTN_OK,
            "",
            true,
            robin_engine::resource_ids::RHID_OK,
            ok_x,
            btn_y,
            ok_w,
            ok_h,
        ));
        if restart_allowed {
            frame.add_widget_absolute(widget_bridge::make_button_with_resource(
                BTN_RESTART,
                "",
                true,
                robin_engine::resource_ids::RHID_RESTART,
                restart_x,
                btn_y,
                restart_w,
                restart_h,
            ));
            frame.add_widget_absolute(widget_bridge::make_button_with_resource(
                BTN_LOAD,
                "",
                true,
                robin_engine::resource_ids::RHID_LOAD,
                load_x,
                btn_y,
                load_w,
                load_h,
            ));
        }
        // Per-pixel hit masks so the transparent corners around each
        // round seal don't capture clicks.
        widget_bridge::attach_alpha_masks(&mut frame, resources, renderer);

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

        let font = match body_font {
            BodyFont::PopupScroll => resources.popup_font_any(),
            BodyFont::Debrief => resources.debrief_font_any(),
        }
        .expect("debriefing body requires its font");
        let scroll_view = ScrollView::new(
            [virt_x + BODY_X, virt_y + BODY_Y, BODY_W, BODY_H],
            font.height() as i32,
            resources,
        );
        let mut input_state = ModalInputState::new();
        input_state.seed_mouse_from_window(event_pump, transform);
        Self {
            title,
            body,
            restart_snapshot_exists,
            body_font,
            quick_load_key,
            transform,
            virt_x,
            virt_y,
            frame,
            input_state,
            tooltip: TooltipState::new(),
            scroll_view,
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
        let (font, lines) = self.prepare_body(resources);
        self.scroll_view.set_total(lines.len());
        let (events, transform) = super::layout::poll_events_with_transform(event_pump, renderer);
        self.transform = transform;
        for event in events {
            self.input_state.update_from_event(&event, self.transform);
            if self.scroll_view.handle_event(
                &event,
                self.transform,
                (
                    self.input_state.virt_x as i32,
                    self.input_state.virt_y as i32,
                ),
            ) {
                continue;
            }
            match event {
                GameEvent::Quit => outcome = Some(PageOutcome::EmergencyEnd),
                GameEvent::KeyDown { keycode, .. }
                    if matches!(
                        keycode,
                        Keycode::Up
                            | Keycode::Down
                            | Keycode::PageUp
                            | Keycode::PageDown
                            | Keycode::Home
                            | Keycode::End
                    ) =>
                {
                    self.scroll_view.navigate(keycode);
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Return,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::KpEnter,
                    ..
                } => {
                    outcome = Some(PageOutcome::Ok);
                }
                GameEvent::KeyDown { physical_key, .. } if physical_key == self.quick_load_key => {
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
                BTN_OK => PageOutcome::Ok,
                BTN_RESTART => {
                    if self.restart_snapshot_exists {
                        PageOutcome::Restart
                    } else {
                        tracing::warn!(
                            "Debriefing Restart clicked but no restart snapshot exists; \
                             falling through to stat panel"
                        );
                        PageOutcome::Ok
                    }
                }
                BTN_LOAD => PageOutcome::LoadClicked,
                _ => PageOutcome::Ok,
            });
        }

        self.render(renderer, resources, cursor.as_ref(), font, &lines);
        renderer.present();
        outcome
    }

    /// Prepare once per frame so input bounds and drawing use the same lines.
    /// Resources are supplied each tick; do not cache across font/theme changes.
    fn prepare_body<'a>(&self, resources: &'a IngameMenuResources) -> (&'a Font, Vec<WrappedLine>) {
        let font = match self.body_font {
            BodyFont::PopupScroll => resources.popup_font_any(),
            BodyFont::Debrief => resources.debrief_font_any(),
        }
        .expect("debriefing body requires its font");
        let lines = super::layout::wrap_text_for_box_font(
            font,
            &self.body,
            self.scroll_view.content_width() - 4,
            usize::MAX,
        )
        .lines;
        (font, lines)
    }

    fn render(
        &mut self,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
        font: &Font,
        lines: &[WrappedLine],
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

        if let Some(font) = resources.title_font_any() {
            render_text_in_box_font(
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

        self.scroll_view.set_total(lines.len());
        for row in self.scroll_view.visible_range() {
            super::layout::render_text_virt_font(
                renderer,
                font,
                self.transform,
                &lines[row].text,
                self.virt_x + BODY_X,
                self.scroll_view.row_y(row),
            );
        }
        // Scrolling exposes the complete body; OK advances to statistics.
        self.scroll_view
            .draw_scrollbar(renderer, self.transform, resources);

        widget_bridge::draw_frame_buttons(renderer, resources, self.transform, &self.frame);

        let mouse_pt =
            engine_coordinates::ScreenPoint::new(self.input_state.virt_x, self.input_state.virt_y);
        self.tooltip.update(&self.frame, mouse_pt);
        self.tooltip
            .draw(renderer, font, self.transform, &self.frame, mouse_pt);

        if let Some(c) = cursor {
            c.draw(renderer, self.transform, &self.input_state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn substitute_printf(template: &str, values: &[&str]) -> String {
        let mut out = String::new();
        append_printf(&mut out, template, values);
        out
    }

    #[test]
    fn printf_append_preserves_existing_output_and_value_boundaries() {
        let mut out = String::from("prefix: ");
        append_printf(&mut out, "%s %u", &["first", "42"]);
        append_printf(&mut out, " / %lu", &["7"]);
        assert_eq!(out, "prefix: first 42 / 7");
    }

    fn flow(body: &str) -> DebriefingModalState {
        DebriefingModalState {
            title: String::new(),
            body: body.into(),
            stat_text: Some("statistics".into()),
            phase: DebriefingPhase::Body,
            restart_allowed: true,
            restart_snapshot_exists: true,
            active_quick_load: None,
            current_page: None,
        }
    }

    #[test]
    fn scrollable_body_advances_once_then_statistics_complete() {
        let body = "long body\n".repeat(100);
        let mut state = flow(&body);
        assert_eq!(state.finish_page(PageOutcome::Ok), None);
        assert!(matches!(state.phase, DebriefingPhase::Stat));
        assert_eq!(state.body, body);
        assert_eq!(state.finish_page(PageOutcome::Ok), None);
        assert!(matches!(state.phase, DebriefingPhase::Done));
    }

    #[test]
    fn load_attempt_preserves_complete_body_and_resume_phase() {
        let mut state = flow("first line\nlast line");
        for was_on_stat in [false, true] {
            assert_eq!(
                state.finish_page(PageOutcome::LoadClicked),
                Some(DebriefingOutcome::LoadAttempt {
                    body: "first line\nlast line".into(),
                    was_on_stat,
                })
            );
            // Cancelling the picker does not consume any body text or phase.
            assert_eq!(matches!(state.phase, DebriefingPhase::Stat), was_on_stat);
            state.finish_page(PageOutcome::Ok);
        }
    }

    #[test]
    fn restart_and_emergency_end_do_not_advance_the_page() {
        for phase in [DebriefingPhase::Body, DebriefingPhase::Stat] {
            let mut state = flow("body");
            state.phase = phase;
            let was_on_stat = matches!(state.phase, DebriefingPhase::Stat);
            assert_eq!(
                state.finish_page(PageOutcome::Restart),
                Some(DebriefingOutcome::Restart)
            );
            assert_eq!(
                state.finish_page(PageOutcome::EmergencyEnd),
                Some(DebriefingOutcome::EmergencyEnd)
            );
            assert_eq!(matches!(state.phase, DebriefingPhase::Stat), was_on_stat);
            assert_eq!(state.body, "body");
        }
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
    fn substitute_printf_preserves_unsupported_and_unfilled_syntax() {
        for template in ["%", "%l", "%x", "%lx", "%lls", "é %lu 界 %s", "%%"] {
            assert_eq!(substitute_printf(template, &[]), template);
        }
        assert_eq!(
            substitute_printf("%x %li %ld %s", &["1", "2", "é"]),
            "%x 1 2 é"
        );
        assert_eq!(substitute_printf("%u %ls %d", &["42"]), "42 %ls %d");
        assert_eq!(substitute_printf("%%u", &["42"]), "%42");
        assert_eq!(substitute_printf("%s", &["", "unused"]), "");
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
            pc_names: vec![engine_mission_stat::PcStatName::new(
                "Little John".into(),
                None,
            )],
            factions: Default::default(),
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
    fn format_mission_stat_displays_negative_net_money() {
        let stat = MissionStat {
            collected_money: (-50_i32) as u32,
            ..Default::default()
        };
        let text = format_mission_stat_text(&stat, 0, &MenuText::english_fallbacks_only());
        assert!(text.contains("You collected -50"));
        assert!(!text.contains("4294967246"));
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
                engine_mission_stat::PcStatName::new(
                    "Robin des bois".into(),
                    Some(engine_pc_status::SpecialPeasantName::A),
                ),
                engine_mission_stat::PcStatName::new("Little John".into(), None),
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
            pc_names: vec![engine_mission_stat::PcStatName::new(
                "Robin des bois".into(),
                Some(engine_pc_status::SpecialPeasantName::C),
            )],
            ..Default::default()
        };
        let text = format_mission_stat_text(&stat, 0, &menu_text);
        assert!(text.contains("Robin des bois"));
    }
}
