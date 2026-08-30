//! Modal Yes/No confirmation dialog.
//!
//! A 400x200 window using `RHID_MENU_BACKGROUND_SMALL` with the message
//! word-wrapped inside a `(25,50)..(375,120)` label, the round Yes / No
//! wax-seal buttons (`RHID_OK` / `RHID_CANCEL`) centred horizontally at
//! y=130 with 18px spacing, and shortcuts binding Return / Numpad Enter
//! → Yes and Escape → No.
//!
//! Buttons are driven by the [`crate::widget`] system: a [`FrameWnd`]
//! holds two [`WidgetButton`]s whose state machines handle hover, push
//! and select transitions.  The bridge module renders them using the
//! existing sprite pipeline.

use robin_engine::coordinates as engine_coordinates;
use robin_engine::sprite::BBox;
use serde::{Deserialize, Serialize};
use winit::keyboard::KeyCode;

use crate::focus_manager::{FrameButtonFocusManager, GroupOrientation};
use crate::gfx_types::GameEvent;
use crate::renderer::Renderer;
use crate::ui::{UiEvent, UiMsg};

use super::layout::{
    MenuTransform, TextAlign, TooltipState, VAlign, dim_screen, draw_background,
    enter_modal_gpu_phase, render_text_in_box_aligned,
};
use super::resources::{IngameMenuResources, MT_INFOBULLE_BUTTON_NO, MT_INFOBULLE_BUTTON_YES};
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

/// Resolved result of the modal's widget event stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum YesNoChoice {
    Yes,
    No,
}

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
        crate::window::sleep_ui_frame().await;
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
    focus: FrameButtonFocusManager,
    choice: Option<YesNoChoice>,
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

        let win_x = (super::layout::MENU_W - WIN_W) / 2;
        let win_y = (super::layout::MENU_H - WIN_H) / 2;
        // Yes / No are the round wax-seal sprites (`RHID_OK` /
        // `RHID_CANCEL`) with no label, like the original dialog.  Both
        // get the max intrinsic size so they render at native
        // dimensions when centred as a pair.
        let (ok_w, ok_h) = resources.ok_button_dimensions();
        let (cancel_w, cancel_h) = resources.cancel_button_dimensions();
        let btn_w = ok_w.max(cancel_w);
        let btn_h = ok_h.max(cancel_h);
        let n = 2i32;
        let total_w = n * btn_w + (n - 1) * BUTTON_GAP;
        let start_x = win_x + (WIN_W - total_w) / 2;
        let btn_y = win_y + 130;

        let mut frame = crate::widget::FrameWnd::default();
        frame.enabled = true;
        frame.input_enabled = true;
        frame.add_widget_absolute(widget_bridge::make_button_with_resource(
            ID_YES,
            "",
            true,
            robin_engine::resource_ids::RHID_OK,
            start_x,
            btn_y,
            btn_w,
            btn_h,
        ));
        frame.add_widget_absolute(widget_bridge::make_button_with_resource(
            ID_NO,
            "",
            true,
            robin_engine::resource_ids::RHID_CANCEL,
            start_x + btn_w + BUTTON_GAP,
            btn_y,
            btn_w,
            btn_h,
        ));
        // Per-pixel hit masks so the transparent corners around each
        // round seal don't capture clicks.
        widget_bridge::attach_alpha_masks(&mut frame, resources, renderer);

        let yes_tooltip = resources.menu_text.get(MT_INFOBULLE_BUTTON_YES);
        let no_tooltip = resources.menu_text.get(MT_INFOBULLE_BUTTON_NO);
        if let Some(w) = frame.widget_mut(ID_YES) {
            w.base_mut().set_tooltip_text(&yes_tooltip);
        }
        if let Some(w) = frame.widget_mut(ID_NO) {
            w.base_mut().set_tooltip_text(&no_tooltip);
        }

        // Original provenance: RHMenuYesNo.cpp::Create registers both
        // buttons as non-navigable horizontal groupables, then binds Return
        // and Numpad Enter to Yes and Escape to No. Keeping the buttons
        // non-navigable is intentional: Left/Right must not change the
        // original shortcut-only dialog behavior.
        let mut focus = FrameButtonFocusManager::new(GroupOrientation::Horizontal);
        focus.add_button(&frame, ID_YES, false);
        focus.add_button(&frame, ID_NO, false);
        focus.add_shortcut(ID_YES, KeyCode::Enter);
        focus.add_shortcut(ID_YES, KeyCode::NumpadEnter);
        focus.add_shortcut(ID_NO, KeyCode::Escape);

        let mut input_state = ModalInputState::new();
        input_state.seed_mouse_from_window(event_pump, transform);
        // UiKeyboard's first refresh only establishes its baseline. Do that
        // before polling so a key pressed on the modal's first visible frame
        // is not swallowed.
        {
            let _ = input_state.as_widget_input();
        }
        input_state.end_frame();

        Self {
            message,
            frame,
            input_state,
            tooltip: TooltipState::new(),
            transform,
            win_x,
            win_y,
            focus,
            choice: None,
        }
    }

    pub fn tick(
        &mut self,
        event_pump: &mut crate::window::GameWindow,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
    ) -> Option<bool> {
        if let Some(choice) = self.choice {
            return Some(choice == YesNoChoice::Yes);
        }

        let (events, transform) = super::layout::poll_events_with_transform(event_pump, renderer);
        self.transform = transform;
        self.handle_events(&events);
        self.render_overlay(renderer, resources, cursor);
        renderer.present();
        self.result()
    }

    /// Advance the dialog from an event batch without drawing or presenting.
    ///
    /// Nested side screens use this split API so they can draw their picker
    /// first and then place the confirmation over it in the same frame.
    pub fn handle_events(&mut self, events: &[GameEvent]) -> Option<bool> {
        if self.choice.is_some() {
            return self.result();
        }
        for event in events {
            self.input_state.update_from_event(event, self.transform);
            if matches!(event, GameEvent::Quit) {
                self.resolve(YesNoChoice::No);
            }
        }
        self.process_widget_input();
        self.result()
    }

    /// Draw the dialog over the caller's current framebuffer without
    /// clearing or presenting it.
    pub fn render_overlay(
        &mut self,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<&ModalCursor<'_>>,
    ) {
        self.render(renderer, resources, cursor);
    }

    pub fn result(&self) -> Option<bool> {
        self.choice.map(|choice| choice == YesNoChoice::Yes)
    }

    fn process_widget_input(&mut self) {
        let events = {
            let widget_input = self.input_state.as_widget_input();
            let events = self.frame.process_input(&widget_input);
            let mouse_captured = widget_input
                .capture
                .is_some_and(|capture| capture.get().is_some());
            self.focus.process_input(
                &mut self.frame,
                events,
                widget_input.keyboard,
                mouse_captured,
            )
        };
        self.input_state.end_frame();
        self.apply_widget_events(&events);
    }

    fn apply_widget_events(&mut self, events: &[UiEvent]) {
        for event in events {
            if event.msg_type != UiMsg::WidgetActivated {
                continue;
            }
            match event.origin_widget_id {
                ID_YES => self.resolve(YesNoChoice::Yes),
                ID_NO => self.resolve(YesNoChoice::No),
                id => panic!("yes/no modal received activation from unknown widget {id}"),
            }
        }
    }

    /// Resolve at most once. Nested/modal callers may observe the state more
    /// than once while unwinding; a later cancel event must not overwrite an
    /// already-confirmed choice (or vice versa).
    fn resolve(&mut self, choice: YesNoChoice) {
        if self.choice.is_none() {
            self.choice = Some(choice);
        }
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
                // Top-origin with word-wrap: the original renders the
                // message with `SBSimpleTextRenderer::Centered` inside
                // the box, wrapping lines from the box top.
                VAlign::Top,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx_types::Keycode;

    fn modal_state() -> YesNoModalState {
        let frame = widget_bridge::make_button_frame(&[
            (ID_YES, "Yes", 0, 0, 80, 30),
            (ID_NO, "No", 100, 0, 80, 30),
        ]);
        let mut focus = FrameButtonFocusManager::new(GroupOrientation::Horizontal);
        focus.add_button(&frame, ID_YES, false);
        focus.add_button(&frame, ID_NO, false);
        focus.add_shortcut(ID_YES, KeyCode::Enter);
        focus.add_shortcut(ID_YES, KeyCode::NumpadEnter);
        focus.add_shortcut(ID_NO, KeyCode::Escape);

        let mut input_state = ModalInputState::new();
        {
            let _ = input_state.as_widget_input();
        }
        input_state.end_frame();

        YesNoModalState {
            message: "Continue?".to_string(),
            frame,
            input_state,
            tooltip: TooltipState::new(),
            transform: MenuTransform::centered(640, 480),
            win_x: 0,
            win_y: 0,
            focus,
            choice: None,
        }
    }

    fn key_event(keycode: Keycode, physical_key: KeyCode, down: bool) -> GameEvent {
        if down {
            GameEvent::KeyDown {
                keycode,
                physical_key: Some(physical_key),
            }
        } else {
            GameEvent::KeyUp {
                keycode,
                physical_key: Some(physical_key),
            }
        }
    }

    fn send_key(state: &mut YesNoModalState, keycode: Keycode, physical: KeyCode, down: bool) {
        let event = key_event(keycode, physical, down);
        state.input_state.update_from_event(&event, state.transform);
        state.process_widget_input();
    }

    #[test]
    fn return_focuses_then_confirms_on_release() {
        let mut state = modal_state();
        send_key(&mut state, Keycode::Return, KeyCode::Enter, true);
        assert_eq!(state.choice, None);
        assert_eq!(state.focus.focused_button(), Some(ID_YES));

        send_key(&mut state, Keycode::Return, KeyCode::Enter, false);
        assert_eq!(state.choice, Some(YesNoChoice::Yes));
    }

    #[test]
    fn escape_focuses_then_cancels_on_release() {
        let mut state = modal_state();
        send_key(&mut state, Keycode::Escape, KeyCode::Escape, true);
        assert_eq!(state.choice, None);
        assert_eq!(state.focus.focused_button(), Some(ID_NO));

        send_key(&mut state, Keycode::Escape, KeyCode::Escape, false);
        assert_eq!(state.choice, Some(YesNoChoice::No));
    }

    #[test]
    fn original_non_navigable_group_ignores_arrows() {
        let mut state = modal_state();
        send_key(&mut state, Keycode::Right, KeyCode::ArrowRight, true);
        assert_eq!(state.focus.focused_button(), None);
        assert_eq!(state.choice, None);
    }

    #[test]
    fn resolution_is_reentrant_and_first_choice_wins() {
        let mut state = modal_state();
        state.apply_widget_events(&[
            UiEvent {
                msg_type: UiMsg::WidgetActivated,
                origin_widget_id: ID_YES,
                data: None,
            },
            UiEvent {
                msg_type: UiMsg::WidgetActivated,
                origin_widget_id: ID_NO,
                data: None,
            },
        ]);
        state.resolve(YesNoChoice::No);
        assert_eq!(state.choice, Some(YesNoChoice::Yes));
    }
}
