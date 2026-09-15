//! One-frame mission-end leaderboard presentation.
//!
//! Network, replay export, and signing are advanced only through the
//! frame-polled controller. This screen owns input/render state and never
//! awaits or pauses multiplayer simulation.

use crate::gfx_types::{GameEvent, Keycode};
use crate::ingame_menu::widget_bridge::ModalScreenIo;
use crate::leaderboard_mission_end::{
    BoardLoadState, MissionEndLeaderboardAction, MissionEndLeaderboardController,
    MissionEndLeaderboardEvent, MissionSubmissionState,
};
use crate::renderer::Renderer;
use crate::widget::FrameWnd;
use robin_run_protocol::{BoardMetricValueV2, TickDurationV1};

use super::layout::{
    MENU_W, MenuRect, MenuTransform, TextAlign, VAlign, draw_screen_background,
    render_clipped_text_in_box_font, render_text_virt_font,
};
use super::resources::IngameMenuResources;
use super::widget_bridge::{self, ModalInputState, ScreenFrame, ScreenKey};

const TAB_ID_BASE: u32 = 100;
const SUBMIT_ID: u32 = 200;
const ALWAYS_SUBMIT_ID: u32 = 201;
const RETRY_ID: u32 = 202;
const CLOSE_ID: u32 = 203;
const TAB_Y: i32 = 55;
const TAB_H: i32 = 28;
const TABLE_X: i32 = 28;
const TABLE_Y: i32 = 105;
const TABLE_W: i32 = 584;
const TABLE_H: i32 = 250;
const MAX_VISIBLE_ROWS: usize = 8;

pub struct MissionEndLeaderboardScreen {
    controller: MissionEndLeaderboardController,
    frame: FrameWnd,
    input: ModalInputState,
    ui_error: Option<String>,
}

impl MissionEndLeaderboardScreen {
    pub fn new(
        controller: MissionEndLeaderboardController,
        resources: &IngameMenuResources,
    ) -> Self {
        let mut screen = Self {
            controller,
            frame: FrameWnd::default(),
            input: ModalInputState::new(),
            ui_error: None,
        };
        screen.frame.enabled = true;
        screen.frame.input_enabled = true;
        screen.rebuild_widgets(resources);
        screen
    }

    pub fn controller(&self) -> &MissionEndLeaderboardController {
        &self.controller
    }

    pub fn controller_mut(&mut self) -> &mut MissionEndLeaderboardController {
        &mut self.controller
    }

    /// Recover the controller when presentation is dismissed. Cooperative
    /// mission owners keep polling it while a consented submission remains in
    /// flight instead of cancelling network/co-sign work or blocking play.
    pub fn into_controller(self) -> MissionEndLeaderboardController {
        self.controller
    }

    /// Advance and render exactly one UI frame. A returned event is consumed
    /// by the outer cooperative task; `None` keeps the overlay alive.
    pub fn tick(&mut self, io: &mut ModalScreenIo<'_, '_>) -> Option<MissionEndLeaderboardEvent> {
        self.controller.poll();
        if !self.controller.is_visible() {
            return self.apply(MissionEndLeaderboardAction::Close);
        }
        self.sync_widget_enabled();
        let screen = ScreenFrame::begin(io, &mut self.input);
        if self.controller.needs_registration() {
            let handle = self.controller.registration_handle();
            let cancelled = {
                let mut slot = robin_util::sync::lock(&handle);
                if let Some(registration) = slot.as_mut() {
                    for event in &screen.events {
                        registration.event(event, screen.transform, (48, 88));
                        if registration.cancelled {
                            break;
                        }
                    }
                    registration.cancelled
                } else {
                    false
                }
            };
            self.input.end_frame();
            if cancelled {
                return self.apply(MissionEndLeaderboardAction::Close);
            }
            self.render(io, &screen);
            return None;
        }
        let mut outcome = None;
        // Actions apply in event order: a tab switch reads the tab selected
        // by the previous event.
        for event in &screen.events {
            let action = match ScreenKey::from_event(event) {
                Some(ScreenKey::Quit | ScreenKey::Cancel) => {
                    Some(MissionEndLeaderboardAction::Close)
                }
                Some(ScreenKey::Confirm | ScreenKey::Next) => None,
                None => match event {
                    GameEvent::KeyDown {
                        keycode: Keycode::Left,
                        ..
                    } => self.adjacent_tab(-1),
                    GameEvent::KeyDown {
                        keycode: Keycode::Right,
                        ..
                    } => self.adjacent_tab(1),
                    _ => None,
                },
            };
            if let Some(action) = action {
                outcome = self.apply(action).or(outcome);
            }
        }
        let (_, activated) = ScreenFrame::dispatch(&mut self.input, &mut self.frame);
        if let Some(id) = activated
            && let Some(action) = self.widget_action(id)
        {
            outcome = self.apply(action).or(outcome);
        }

        self.render(io, &screen);
        outcome
    }

    fn rebuild_widgets(&mut self, resources: &IngameMenuResources) {
        self.frame.clear_widgets();
        let board_count = i32::try_from(self.controller.boards().len()).unwrap_or(1);
        let gap = 8;
        let tab_w = ((TABLE_W - gap * (board_count - 1)) / board_count).clamp(100, 190);
        for (index, board) in self.controller.boards().iter().enumerate() {
            let index_i32 = i32::try_from(index).unwrap_or(0);
            self.frame.add_widget_absolute(widget_bridge::make_button(
                TAB_ID_BASE + u32::try_from(index).unwrap_or(0),
                &board.label,
                TABLE_X + index_i32 * (tab_w + gap),
                TAB_Y,
                tab_w,
                TAB_H,
            ));
        }
        let (button_w, button_h) = resources.button_dimensions();
        let bottom_y = 420;
        self.frame
            .add_widget_absolute(widget_bridge::make_button_enabled(
                SUBMIT_ID,
                "Submit this run",
                false,
                26,
                bottom_y,
                button_w,
                button_h,
            ));
        self.frame.add_widget_absolute(widget_bridge::make_button(
            ALWAYS_SUBMIT_ID,
            "Always submit won runs",
            185,
            bottom_y,
            button_w.max(180),
            button_h,
        ));
        self.frame
            .add_widget_absolute(widget_bridge::make_button_enabled(
                RETRY_ID, "Retry", false, 400, bottom_y, 90, button_h,
            ));
        self.frame.add_widget_absolute(widget_bridge::make_button(
            CLOSE_ID, "Continue", 500, bottom_y, 110, button_h,
        ));
    }

    fn sync_widget_enabled(&mut self) {
        if let Some(widget) = self.frame.widget_mut(SUBMIT_ID) {
            widget.base_mut().enabled = matches!(
                self.controller.submission_state(),
                MissionSubmissionState::AwaitingConsent
            );
        }
        if let Some(widget) = self.frame.widget_mut(RETRY_ID) {
            widget.base_mut().enabled = matches!(
                (
                    self.controller.board_state(),
                    self.controller.submission_state()
                ),
                (BoardLoadState::Failed(_), _) | (_, MissionSubmissionState::Failed(_))
            );
        }
    }

    fn adjacent_tab(&self, direction: i32) -> Option<MissionEndLeaderboardAction> {
        let boards = self.controller.boards();
        if boards.len() < 2 {
            return None;
        }
        let current = boards
            .iter()
            .position(|board| board.tab == self.controller.selected_tab())
            .unwrap_or(0);
        let next = (i32::try_from(current).ok()? + direction)
            .rem_euclid(i32::try_from(boards.len()).ok()?);
        Some(MissionEndLeaderboardAction::SelectTab(
            boards[usize::try_from(next).ok()?].tab,
        ))
    }

    fn widget_action(&self, id: u32) -> Option<MissionEndLeaderboardAction> {
        if let Some(index) = id.checked_sub(TAB_ID_BASE)
            && let Some(board) = self.controller.boards().get(index as usize)
        {
            return Some(MissionEndLeaderboardAction::SelectTab(board.tab));
        }
        match id {
            SUBMIT_ID => Some(MissionEndLeaderboardAction::SubmitThisRun),
            ALWAYS_SUBMIT_ID => Some(MissionEndLeaderboardAction::SetAlwaysSubmitRuns(
                !self.controller.preferences().always_submit_eligible_runs,
            )),
            RETRY_ID => Some(
                if matches!(
                    self.controller.submission_state(),
                    MissionSubmissionState::Failed(_)
                ) {
                    MissionEndLeaderboardAction::RetrySubmission
                } else {
                    MissionEndLeaderboardAction::RetryBoard
                },
            ),
            CLOSE_ID => Some(MissionEndLeaderboardAction::Close),
            _ => None,
        }
    }

    fn apply(&mut self, action: MissionEndLeaderboardAction) -> Option<MissionEndLeaderboardEvent> {
        match self.controller.apply_action(action) {
            Ok(MissionEndLeaderboardEvent::Closed) => Some(MissionEndLeaderboardEvent::Closed),
            Ok(
                MissionEndLeaderboardEvent::None | MissionEndLeaderboardEvent::PreferencesChanged,
            ) => {
                self.ui_error = None;
                None
            }
            Err(error) => {
                self.ui_error = Some(error.to_string());
                None
            }
        }
    }

    fn render(&self, io: &mut ModalScreenIo<'_, '_>, screen: &ScreenFrame) {
        let renderer = &mut *io.renderer;
        let resources = io.resources;
        let transform = screen.transform;
        screen.begin_draw(renderer);
        if let Some(background) = resources.menu_bg[0] {
            draw_screen_background(renderer, &background);
        }
        if self.controller.needs_registration() {
            let handle = self.controller.registration_handle();
            if let Some(registration) = robin_util::sync::lock(&handle).as_mut() {
                let font = resources
                    .label_font_any()
                    .or_else(|| resources.title_font_any())
                    .expect("leaderboard registration needs a readable font");
                registration.draw(renderer, font, transform, (48, 88));
            }
            screen.finish(io, &self.input);
            return;
        }
        if let Some(font) = resources.title_font_any() {
            let title = match self.controller.outcome() {
                crate::leaderboard_mission_end::MissionEndOutcome::Won => {
                    "Mission complete - Leaderboards"
                }
                crate::leaderboard_mission_end::MissionEndOutcome::Lost => {
                    "Mission lost - Leaderboards"
                }
                crate::leaderboard_mission_end::MissionEndOutcome::Interrupted => {
                    "Mission ended - Leaderboards"
                }
            };
            render_text_virt_font(
                renderer,
                font,
                transform,
                title,
                (MENU_W - font.text_width(title)) / 2,
                18,
            );
        }
        for (index, widget) in self.frame.widgets().iter().enumerate() {
            if widget.id() == ALWAYS_SUBMIT_ID {
                widget_bridge::draw_widget_radio(
                    renderer,
                    resources,
                    transform,
                    widget,
                    self.controller.preferences().always_submit_eligible_runs,
                );
            } else {
                let selected = widget.id() >= TAB_ID_BASE
                    && widget.id() < TAB_ID_BASE + self.controller.boards().len() as u32
                    && self
                        .controller
                        .boards()
                        .get(index)
                        .is_some_and(|board| board.tab == self.controller.selected_tab());
                widget_bridge::draw_widget_button(renderer, resources, transform, widget, selected);
            }
        }
        if let Some(font) = resources.label_font_any() {
            self.render_board(renderer, font, transform);
            let status = self
                .ui_error
                .as_deref()
                .unwrap_or_else(|| submission_status(self.controller.submission_state()));
            render_clipped_text_in_box_font(
                renderer,
                font,
                transform,
                status,
                MenuRect {
                    x: TABLE_X,
                    y: 365,
                    w: TABLE_W,
                    h: 45,
                },
                TextAlign::Center,
                VAlign::Top,
            );
        }
        screen.finish(io, &self.input);
    }

    fn render_board(
        &self,
        renderer: &mut Renderer,
        font: &crate::native_font::Font,
        transform: MenuTransform,
    ) {
        match self.controller.board_state() {
            BoardLoadState::Loading => {
                render_text_virt_font(
                    renderer,
                    font,
                    transform,
                    "Loading leaderboard...",
                    TABLE_X + 10,
                    TABLE_Y + 20,
                );
            }
            BoardLoadState::Failed(error) => {
                render_clipped_text_in_box_font(
                    renderer,
                    font,
                    transform,
                    &format!("Leaderboard unavailable: {error}"),
                    MenuRect {
                        x: TABLE_X + 10,
                        y: TABLE_Y + 20,
                        w: TABLE_W - 20,
                        h: TABLE_H - 40,
                    },
                    TextAlign::Center,
                    VAlign::Top,
                );
            }
            BoardLoadState::Ready(page) if page.entries.is_empty() => {
                render_text_virt_font(
                    renderer,
                    font,
                    transform,
                    "No verified runs yet.",
                    TABLE_X + 10,
                    TABLE_Y + 20,
                );
            }
            BoardLoadState::Ready(page) => {
                for (row, entry) in page.entries.iter().take(MAX_VISIBLE_ROWS).enumerate() {
                    let y = TABLE_Y + 8 + i32::try_from(row).unwrap_or(0) * 28;
                    // Only the uploader can be named; every other recorded
                    // participant instance is anonymous.
                    let player = participant_text(
                        entry
                            .uploader
                            .iter()
                            .map(|participant| participant.username.as_str()),
                        u32::from(entry.participant_instance_count)
                            - u32::from(entry.uploader.is_some()),
                    );
                    let rank = format!("#{}", entry.rank);
                    let player = truncate_chars(&renderable_text(font, &player), 34);
                    let metric = metric_text(&entry.metric_value, self.controller.tick_duration());
                    render_text_virt_font(renderer, font, transform, &rank, TABLE_X + 10, y);
                    render_text_virt_font(renderer, font, transform, &player, TABLE_X + 65, y);
                    render_text_virt_font(
                        renderer,
                        font,
                        transform,
                        &metric,
                        TABLE_X + TABLE_W - 10 - font.text_width(&metric),
                        y,
                    );
                }
            }
        }
    }
}

fn submission_status(state: &MissionSubmissionState) -> &str {
    match state {
        MissionSubmissionState::WonMissionRequired => {
            "Failed and interrupted missions are never submitted."
        }
        MissionSubmissionState::Unavailable(reason) | MissionSubmissionState::Failed(reason) => {
            reason
        }
        MissionSubmissionState::AwaitingConsent => {
            "This won run is eligible. Submit it, or continue without uploading."
        }
        MissionSubmissionState::Submitting => {
            "Signing and uploading the replay for server verification..."
        }
        MissionSubmissionState::Queued(_) => {
            "Submitted. Verification continues on the server; you may continue playing."
        }
    }
}

fn metric_text(value: &BoardMetricValueV2, tick_duration: TickDurationV1) -> String {
    match value {
        BoardMetricValueV2::OriginalScore { points } => format!("{points} pts"),
        BoardMetricValueV2::FastestSuccess {
            active_simulation_ticks,
        } => {
            let micros = u128::from(*active_simulation_ticks)
                .saturating_mul(u128::from(tick_duration.numerator_micros))
                / u128::from(tick_duration.denominator.max(1));
            let total_seconds = micros / 1_000_000;
            format!("{}:{:02}", total_seconds / 60, total_seconds % 60)
        }
    }
}

fn truncate_chars(value: &str, maximum: usize) -> String {
    let Some((end, _)) = value.char_indices().nth(maximum) else {
        return value.to_owned();
    };
    if maximum <= 3 {
        return ".".repeat(maximum);
    }
    let prefix_end = value[..end]
        .char_indices()
        .nth(maximum - 3)
        .expect("truncated prefix contains the requested scalar count")
        .0;
    let mut output = String::with_capacity(prefix_end + 3);
    output.push_str(&value[..prefix_end]);
    output.push_str("...");
    output
}

fn renderable_text(font: &crate::native_font::Font, value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_whitespace() || font.character_width(ch) > 0 {
                ch
            } else {
                '?'
            }
        })
        .collect()
}

fn participant_text<'a>(
    names: impl Iterator<Item = &'a str>,
    anonymous_participant_instance_count: u32,
) -> String {
    use std::fmt::Write;
    let mut output = String::new();
    let mut separator = "";
    for name in names {
        output.push_str(separator);
        output.push_str(name);
        separator = ", ";
    }
    if anonymous_participant_instance_count > 0 {
        output.push_str(separator);
        let count = anonymous_participant_instance_count;
        write!(
            &mut output,
            "{count} anonymous{}",
            if count == 1 { "" } else { " participants" }
        )
        .expect("writing to a String cannot fail");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_labels_use_score_and_authoritative_tick_fraction() {
        let tick = TickDurationV1 {
            numerator_micros: 40_000,
            denominator: 1,
        };
        assert_eq!(
            metric_text(&BoardMetricValueV2::OriginalScore { points: 1234 }, tick),
            "1234 pts"
        );
        assert_eq!(
            metric_text(
                &BoardMetricValueV2::FastestSuccess {
                    active_simulation_ticks: 2_250,
                },
                tick
            ),
            "1:30"
        );
    }

    #[test]
    fn long_public_names_are_bounded_by_unicode_scalars() {
        let value = "Marian🏹".repeat(10);
        let shortened = truncate_chars(&value, 12);
        assert_eq!(shortened.chars().count(), 12);
        assert!(shortened.ends_with("..."));
        assert_eq!(truncate_chars("Robin", 2), "..");
        assert_eq!(truncate_chars("Robin", 0), "");
    }

    #[test]
    fn named_and_anonymous_multiplayer_participants_are_all_represented() {
        assert_eq!(
            participant_text(["Robin", "Marian"].into_iter(), 2),
            "Robin, Marian, 2 anonymous participants"
        );
        assert_eq!(
            participant_text(std::iter::empty::<&str>(), 1),
            "1 anonymous"
        );
    }
}

#[test]
fn leaderboard_text_preserves_empty_names_and_short_scalar_limits() {
    assert_eq!(participant_text([].into_iter(), 0), "");
    assert_eq!(
        participant_text(["", "Robin", ""].into_iter(), 1),
        ", Robin, , 1 anonymous"
    );
    assert_eq!(
        participant_text([].into_iter(), 2),
        "2 anonymous participants"
    );
    assert_eq!(
        participant_text(["罗宾", "Marian"].into_iter(), 0),
        "罗宾, Marian"
    );
    for maximum in 0..=8 {
        let source = "é🏹罗宾AB";
        let expected = match maximum {
            0 => "",
            1 => ".",
            2 => "..",
            3 => "...",
            4 => "é...",
            5 => "é🏹...",
            _ => source,
        };
        assert_eq!(truncate_chars(source, maximum), expected);
        assert_eq!(truncate_chars("", maximum), "");
    }
}
