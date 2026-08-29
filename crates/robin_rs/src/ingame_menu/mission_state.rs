//! Mission won / lost / quit transient popup.
//!
//! Creates a 629x480 frame window with the small menu background
//! inside, plays a scaling open/close transition from the source
//! button position, and on open launches a blocking Yes/No
//! confirmation prompt.  Returns `true` when the player confirms.

use crate::renderer::Renderer;

use super::layout::{
    MENU_H, MENU_W, MenuTransform, TextAlign, dim_screen, draw_background, enter_modal_gpu_phase,
    render_text_in_box,
};
use super::resources::{IngameMenuResources, MT_TTL_MISSION_LOST, MT_TTL_MISSION_WON};
use super::widget_bridge::ModalCursor;
use super::yesno::YesNoModalState;

const TRANSITION_SPEED: f32 = 1.5;
const INV_TRANSITION_SPEED: f32 = 1.0 / TRANSITION_SPEED;

/// Popup small-background dimensions.
const POPUP_W: i32 = 400;
const POPUP_H: i32 = 200;

/// Text region inside the popup.
const TEXT_X: i32 = 25;
const TEXT_Y: i32 = 50;
const TEXT_W: i32 = 350;
const TEXT_H: i32 = 70;

/// Show the mission state popup.
///
/// - `message` is the body text (e.g. "Really abandon this mission?").
/// - `won` selects the Mission Won vs Mission Lost title.
/// - `source_button`: the on-screen rectangle the popup zooms out from
///   (the "start mission" / "quit mission" widget the player clicked).
///   Pass the full screen rect when there is no source button.
///
/// Returns `true` if the player confirmed the prompt.
pub async fn show_mission_state_popup(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    mut cursor: Option<ModalCursor<'_>>,
    message: &str,
    won: bool,
    source_button: Option<(i32, i32, i32, i32)>,
) -> bool {
    let mut state =
        MissionStatePopupState::new(renderer, resources, message.to_string(), won, source_button);
    loop {
        if let Some(result) = state.tick(
            event_pump,
            renderer,
            resources,
            cursor.as_mut().map(|c| c.reborrow()),
        ) {
            return result;
        }
        crate::window::sleep_ms(16).await;
    }
}

#[derive(Copy, Clone)]
enum TransitionDirection {
    Open,
    Close,
}

enum MissionStatePopupPhase {
    Opening(MissionStateTransition),
    Confirming(Box<YesNoModalState>),
    Closing(MissionStateTransition),
    Done,
}

/// One-frame state for the mission won/lost popup.
pub struct MissionStatePopupState {
    message: String,
    won: bool,
    source_button: Option<(i32, i32, i32, i32)>,
    phase: MissionStatePopupPhase,
}

impl MissionStatePopupState {
    pub fn new(
        renderer: &Renderer,
        resources: &IngameMenuResources,
        message: String,
        won: bool,
        source_button: Option<(i32, i32, i32, i32)>,
    ) -> Self {
        let phase = MissionStatePopupPhase::Opening(MissionStateTransition::new(
            renderer,
            resources,
            &message,
            won,
            source_button,
            TransitionDirection::Open,
        ));
        Self {
            message,
            won,
            source_button,
            phase,
        }
    }

    pub fn tick(
        &mut self,
        event_pump: &mut crate::window::GameWindow,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<ModalCursor<'_>>,
    ) -> Option<bool> {
        match &mut self.phase {
            MissionStatePopupPhase::Opening(transition) => {
                if transition.tick(event_pump, renderer, resources, cursor) {
                    self.phase = MissionStatePopupPhase::Confirming(Box::new(
                        YesNoModalState::new(event_pump, renderer, resources, self.message.clone()),
                    ));
                }
                None
            }
            MissionStatePopupPhase::Confirming(yesno) => {
                if let Some(confirmed) =
                    yesno.tick(event_pump, renderer, resources, cursor.as_ref())
                {
                    if confirmed {
                        self.phase = MissionStatePopupPhase::Done;
                        Some(true)
                    } else {
                        self.phase = MissionStatePopupPhase::Closing(MissionStateTransition::new(
                            renderer,
                            resources,
                            &self.message,
                            self.won,
                            self.source_button,
                            TransitionDirection::Close,
                        ));
                        None
                    }
                } else {
                    None
                }
            }
            MissionStatePopupPhase::Closing(transition) => {
                if transition.tick(event_pump, renderer, resources, cursor) {
                    self.phase = MissionStatePopupPhase::Done;
                    Some(false)
                } else {
                    None
                }
            }
            MissionStatePopupPhase::Done => Some(false),
        }
    }
}

struct MissionStateTransition {
    transform: MenuTransform,
    src_vx: i32,
    src_vy: i32,
    src_vw: i32,
    src_vh: i32,
    final_x: i32,
    final_y: i32,
    counter: f32,
    title: String,
    message: String,
    direction: TransitionDirection,
}

impl MissionStateTransition {
    fn new(
        renderer: &Renderer,
        resources: &IngameMenuResources,
        message: &str,
        won: bool,
        source_button: Option<(i32, i32, i32, i32)>,
        direction: TransitionDirection,
    ) -> Self {
        let sw = renderer.screen_width() as i32;
        let sh = renderer.screen_height() as i32;
        let transform = MenuTransform::centered(sw, sh);

        // Final (fully-open) virtual rectangle, centred in the menu.
        let final_x = (MENU_W - POPUP_W) / 2;
        let final_y = (MENU_H - POPUP_H) / 2;

        // Start (collapsed) rectangle — the source button in screen pixels
        // converted back to virtual coordinates.  When no source button is
        // supplied, fall back to a 1x1 point at the final centre so the
        // animation still runs for code paths (e.g. mission end) that do
        // not have a triggering button.
        let (src_vx, src_vy, src_vw, src_vh) = source_button
            .map(|(sx, sy, sw_btn, sh_btn)| {
                let (vx, vy) = transform.from_screen(sx, sy);
                (vx, vy, sw_btn, sh_btn)
            })
            .unwrap_or((final_x + POPUP_W / 2, final_y + POPUP_H / 2, 1, 1));

        let counter: f32 = match direction {
            TransitionDirection::Open => TRANSITION_SPEED,
            TransitionDirection::Close => 0.05,
        };

        let title_id = if won {
            MT_TTL_MISSION_WON
        } else {
            MT_TTL_MISSION_LOST
        };
        let title = resources.menu_text.get(title_id);

        Self {
            transform,
            src_vx,
            src_vy,
            src_vw,
            src_vh,
            final_x,
            final_y,
            counter,
            title,
            message: message.to_string(),
            direction,
        }
    }

    fn tick(
        &mut self,
        event_pump: &mut crate::window::GameWindow,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        mut cursor: Option<ModalCursor<'_>>,
    ) -> bool {
        // Exponential decay:
        //   open:  counter *= INV_TRANSITION_SPEED
        //   close: counter *= TRANSITION_SPEED
        match self.direction {
            TransitionDirection::Open => {
                self.counter *= INV_TRANSITION_SPEED;
                if self.counter < 0.03 {
                    return true;
                }
            }
            TransitionDirection::Close => {
                self.counter *= TRANSITION_SPEED;
                if self.counter >= 0.8 {
                    return true;
                }
            }
        }

        // Drain pending input so the OS doesn't think the window is hung, and
        // keep the transition centred if the presentation aspect changes.
        let (_events, transform) = super::layout::poll_events_with_transform(event_pump, renderer);
        self.transform = transform;

        // Interpolate bounds: destination = source + (final - source) * (1 - counter)
        let t = (1.0_f32 - self.counter.min(1.0)).clamp(0.0, 1.0);
        let tl_x = self.src_vx + ((self.final_x - self.src_vx) as f32 * t) as i32;
        let tl_y = self.src_vy + ((self.final_y - self.src_vy) as f32 * t) as i32;
        let br_x = (self.src_vx + self.src_vw)
            + ((self.final_x + POPUP_W - (self.src_vx + self.src_vw)) as f32 * t) as i32;
        let br_y = (self.src_vy + self.src_vh)
            + ((self.final_y + POPUP_H - (self.src_vy + self.src_vh)) as f32 * t) as i32;
        let cur_w = (br_x - tl_x).max(1);
        let cur_h = (br_y - tl_y).max(1);

        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);

        if let Some(bg) = resources.menu_bg_small {
            draw_background(renderer, self.transform, &bg, tl_x, tl_y, cur_w, cur_h);
        }

        // Title + body draw every frame in both directions.  We don't
        // bake a snapshot of the fully-open popup; instead we scale the
        // text-box origin/size in proportion to the current popup rect
        // so the body stays inside the shrinking frame.
        let scale_x = cur_w as f32 / POPUP_W as f32;
        let scale_y = cur_h as f32 / POPUP_H as f32;
        if let Some(font) = resources.title_font() {
            let tw = font.text_width(&self.title);
            let tx = tl_x + (cur_w - tw) / 2;
            let ty = tl_y + (20.0 * scale_y) as i32;
            super::layout::render_text_virt(renderer, font, self.transform, &self.title, tx, ty);
        }
        if let Some(font) = resources.popup_font() {
            let _ = render_text_in_box(
                renderer,
                font,
                self.transform,
                &self.message,
                tl_x + (TEXT_X as f32 * scale_x) as i32,
                tl_y + (TEXT_Y as f32 * scale_y) as i32,
                (TEXT_W as f32 * scale_x) as i32,
                (TEXT_H as f32 * scale_y) as i32,
                TextAlign::Center,
            );
        }

        if let Some(c) = cursor.as_mut() {
            let (mx, my) = event_pump.cursor_pos();
            c.cursor
                .render(renderer, mx as f32, my as f32, c.opacity, c.shadow_color);
        }

        renderer.present();
        false
    }
}
