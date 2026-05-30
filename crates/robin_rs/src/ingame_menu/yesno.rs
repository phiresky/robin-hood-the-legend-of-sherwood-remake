//! Modal Yes/No confirmation dialog.
//!
//! A 400x200 window using `RHID_MENU_BACKGROUND_SMALL` with the message
//! inside a `(25,50)..(375,120)` label, the Yes / No buttons centred
//! horizontally at y=130 with 18px spacing, and shortcuts binding
//! Return / Numpad Enter → Yes and Escape → No.
//!
//! Buttons are driven by the [`crate::widget`] system: a [`FrameWnd`]
//! holds two [`WidgetButton`]s whose state machines handle hover, push
//! and select transitions.  The bridge module renders them using the
//! existing sprite pipeline.

use crate::gfx_types::Keycode;
use robin_engine::coordinates as engine_coordinates;
use robin_engine::sprite::BBox;

use crate::gfx_types::GameEvent;
use crate::renderer::Renderer;

use super::layout::{
    MenuTransform, TextAlign, TooltipState, VAlign, dim_screen, draw_background,
    enter_modal_gpu_phase, render_text_in_box_aligned,
};
use super::resources::{
    IngameMenuResources, MT_BTN_CANCEL, MT_BTN_OK, MT_INFOBULLE_BUTTON_NO, MT_INFOBULLE_BUTTON_YES,
};
use super::widget_bridge::{self, ModalCursor, ModalInputState};

/// Virtual window geometry.
pub const WIN_W: i32 = 400;
pub const WIN_H: i32 = 200;

/// Message label bounding box `(25,50)..(375,120)`.
const MSG_X: i32 = 25;
const MSG_Y: i32 = 50;
const MSG_W: i32 = 350; // 375 - 25
const MSG_H: i32 = 70; // 120 - 50

/// Horizontal spacing between the Yes / No buttons.
const BUTTON_GAP: i32 = 18;

/// Widget IDs for the two buttons.
const ID_YES: u32 = 0;
const ID_NO: u32 = 1;

/// Display the modal confirmation dialog.  Returns `true` if the player
/// chose Yes (or pressed Return / Numpad Enter), `false` if the player
/// chose No (or pressed Escape / closed the window).
pub async fn show_yesno(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    cursor: Option<ModalCursor<'_>>,
    message: &str,
) -> bool {
    let mut state = YesNoModalState::new(event_pump, renderer, resources, message.to_string());
    loop {
        if let Some(result) = state.tick(event_pump, renderer, resources, cursor.as_ref()) {
            return result;
        }
        crate::window::sleep_ms(16).await;
    }
}

/// One-frame state for the standard yes/no modal.
pub struct YesNoModalState {
    message: String,
    frame: crate::widget::FrameWnd,
    input_state: ModalInputState,
    tooltip: TooltipState,
    transform: MenuTransform,
    win_x: i32,
    win_y: i32,
}

impl YesNoModalState {
    pub fn new(
        event_pump: &crate::window::GameWindow,
        renderer: &Renderer,
        resources: &IngameMenuResources,
        message: String,
    ) -> Self {
        let sw = renderer.screen_width() as i32;
        let sh = renderer.screen_height() as i32;
        let transform = MenuTransform::centered(sw, sh);

        let (btn_w, btn_h) = resources.button_dimensions();
        let win_x = (super::layout::MENU_W - WIN_W) / 2;
        let win_y = (super::layout::MENU_H - WIN_H) / 2;
        let yes_label = resources.menu_text.get(MT_BTN_OK);
        let no_label = resources.menu_text.get(MT_BTN_CANCEL);
        let n = 2i32;
        let total_w = n * btn_w + (n - 1) * BUTTON_GAP;
        let start_x = win_x + (WIN_W - total_w) / 2;
        let btn_y = win_y + 130;

        let mut frame = widget_bridge::make_button_frame(&[
            (ID_YES, &yes_label, start_x, btn_y, btn_w, btn_h),
            (
                ID_NO,
                &no_label,
                start_x + btn_w + BUTTON_GAP,
                btn_y,
                btn_w,
                btn_h,
            ),
        ]);

        let yes_tooltip = resources.menu_text.get(MT_INFOBULLE_BUTTON_YES);
        let no_tooltip = resources.menu_text.get(MT_INFOBULLE_BUTTON_NO);
        if let Some(w) = frame.widget_mut(ID_YES) {
            w.base_mut().set_tooltip_text(&yes_tooltip);
        }
        if let Some(w) = frame.widget_mut(ID_NO) {
            w.base_mut().set_tooltip_text(&no_tooltip);
        }

        let mut input_state = ModalInputState::new();
        input_state.seed_mouse_from_sdl(event_pump, transform);

        Self {
            message,
            frame,
            input_state,
            tooltip: TooltipState::new(),
            transform,
            win_x,
            win_y,
        }
    }

    pub fn tick(
        &mut self,
        event_pump: &mut crate::window::GameWindow,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
    ) -> Option<bool> {
        let mut result = None;
        for event in event_pump.poll_events() {
            self.input_state.update_from_event(&event, self.transform);
            match event {
                GameEvent::Quit => result = Some(false),
                GameEvent::KeyDown {
                    keycode: Keycode::Return,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::KpEnter,
                    ..
                } => result = Some(true),
                GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => result = Some(false),
                _ => {}
            }
        }

        let widget_input = self.input_state.as_widget_input();
        let events = self.frame.process_input(&widget_input);
        self.input_state.end_frame();
        if let Some(id) = widget_bridge::find_activated(&events) {
            match id {
                ID_YES => result = Some(true),
                ID_NO => result = Some(false),
                _ => {}
            }
        }

        self.render(renderer, resources, cursor);
        renderer.present();
        result
    }

    fn render(
        &mut self,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
    ) {
        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);

        if let Some(bg) = resources.menu_bg_small {
            draw_background(
                renderer,
                self.transform,
                &bg,
                self.win_x,
                self.win_y,
                WIN_W,
                WIN_H,
            );
        } else {
            let (sx, sy) = self.transform.to_screen(self.win_x, self.win_y);
            renderer.fill_screen(
                Some(&BBox::from_coords(
                    sx as f32,
                    sy as f32,
                    (sx + WIN_W) as f32,
                    (sy + WIN_H) as f32,
                )),
                Renderer::create_color_16(30, 25, 15),
            );
            renderer.draw_rect_outline_screen(
                sx,
                sy,
                sx + WIN_W,
                sy + WIN_H,
                Renderer::create_color_16(180, 160, 100),
            );
        }

        if let Some(font) = resources.popup_font() {
            let _ = render_text_in_box_aligned(
                renderer,
                font,
                self.transform,
                &self.message,
                self.win_x + MSG_X,
                self.win_y + MSG_Y,
                MSG_W,
                MSG_H,
                TextAlign::Center,
                VAlign::Center,
            );
        }

        widget_bridge::draw_frame_buttons(renderer, resources, self.transform, &self.frame);

        let mouse_pt =
            engine_coordinates::ScreenPoint::new(self.input_state.virt_x, self.input_state.virt_y);
        self.tooltip.update(&self.frame, mouse_pt);
        if let Some(font) = resources.popup_font() {
            self.tooltip
                .draw(renderer, font, self.transform, &self.frame, mouse_pt);
        }

        if let Some(c) = cursor {
            c.draw(renderer, self.transform, &self.input_state);
        }
    }
}

/// Formats the hardcoded English prompt `"Unable to find the file '%s' !
/// Would you like to try to load the file again ?"` (exact punctuation
/// and spacing preserved) and presents the standard Yes/No modal.
/// Returns `true` if the player wants to retry the load.
///
/// Reuses the caller's current frame plus the shared `dim_screen`
/// overlay. The original standalone `RHMenuYesNo::FileNotFound`
/// constructs a fresh menu screen and uses the same prompt; Rust exposes
/// the modal helper at the menu boundary so resource retry loops can opt
/// into it without coupling low-level loaders to UI state.
pub async fn show_file_not_found(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    cursor: Option<ModalCursor<'_>>,
    path: &str,
) -> bool {
    let message = format!(
        "Unable to find the file '{}' ! Would you like to try to load the file again ?",
        path
    );
    show_yesno(event_pump, renderer, resources, cursor, &message).await
}
