//! Blazon-purchase confirmation modal.
//!
//! A 400x200 window shown on top of the mission-description modal when
//! the player clicks the "Convert money → blazons" button.  The window
//! shows the current ransom, the blazon price and a "Do you want to
//! buy a blazon?" confirmation with Buy (`RHID_OK`) and Quit
//! (`RHID_CANCEL`) seal buttons centred horizontally at y=135 with
//! 20px spacing.
//!
//! The purchase bookkeeping — deducting gold, incrementing the blazon
//! count, bumping the price, and running the follow-on update-blazons
//! logic — lives at the caller (`mission_description::show_mission_description`).
//! This module just renders the confirmation and reports the outcome.
//!
//! Shortcut bindings:
//!   Return / Numpad Enter → Buy
//!   Escape               → Quit

use crate::gfx_types::Keycode;
use robin_engine::sprite::BBox;

use crate::gfx_types::GameEvent;
use crate::renderer::Renderer;
use crate::ui_screens::BuyBlazonsScreen;
use crate::widget::FrameWnd;
use robin_engine::resource_ids;

use super::layout::{
    MENU_H, MENU_W, MenuTransform, TextAlign, VAlign, dim_screen, draw_background,
    enter_modal_gpu_phase, render_text_in_box_aligned_font,
};
use super::resources::{
    IngameMenuResources, MT_MSG_BUY_BLAZON, MT_STR_BLAZON_PRICE, MT_STR_RANSOM,
};
use super::widget_bridge::{self, ModalCursor, ModalInputState};

/// Virtual window geometry.
pub const WIN_W: i32 = 400;
pub const WIN_H: i32 = 200;

/// Message label geometry: bounding box (50, 50)..(350, 125).
const MSG_X: i32 = 50;
const MSG_Y: i32 = 50;
const MSG_W: i32 = 300; // 350 - 50
const MSG_H: i32 = 75; // 125 - 50

/// Common y for every button in the centred bottom row.
const BUTTON_ROW_Y: i32 = 135;

/// Horizontal gap between buttons in the centred row.
const BUTTON_GAP: i32 = 20;

const ID_BUY: u32 = 0;
const ID_QUIT: u32 = 1;

/// Outcome reported back to the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuyBlazonsOutcome {
    /// Player clicked Buy (or Enter) while the button was enabled — the
    /// caller should apply the purchase to campaign state.
    Bought,
    /// Player clicked Quit, pressed Escape, or closed the window.
    Cancelled,
}

/// One-frame driver for the blazon purchase confirmation.
pub struct BuyBlazonsModalState {
    screen: BuyBlazonsScreen,
    frame: FrameWnd,
    message: String,
    input_state: ModalInputState,
    focused_index: usize,
    buy_x: i32,
    quit_x: i32,
    btn_y: i32,
    btn_w: i32,
    btn_h: i32,
}

impl BuyBlazonsModalState {
    pub fn new(
        event_pump: &crate::window::GameWindow,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        mission_index: usize,
        blazon_price: u32,
        ransom: u32,
    ) -> Self {
        let screen = BuyBlazonsScreen::new(mission_index, blazon_price, ransom);
        let transform = MenuTransform::centered(
            renderer.screen_width() as i32,
            renderer.screen_height() as i32,
        );
        let win_x = (MENU_W - WIN_W) / 2;
        let win_y = (MENU_H - WIN_H) / 2;
        let (ok_w, ok_h) = resources.ok_button_dimensions();
        let (cancel_w, cancel_h) = resources.cancel_button_dimensions();
        let btn_w = ok_w.max(cancel_w);
        let btn_h = ok_h.max(cancel_h);
        let total_w = 2 * btn_w + BUTTON_GAP;
        let buy_x = win_x + (WIN_W - total_w) / 2;
        let quit_x = buy_x + btn_w + BUTTON_GAP;
        let btn_y = win_y + BUTTON_ROW_Y;

        let mut frame = FrameWnd::default();
        frame.enabled = true;
        frame.input_enabled = true;
        frame.add_widget_absolute(widget_bridge::make_button_with_resource(
            ID_BUY,
            "",
            screen.can_buy(),
            resource_ids::RHID_OK,
            buy_x,
            btn_y,
            btn_w,
            btn_h,
        ));
        frame.add_widget_absolute(widget_bridge::make_button_with_resource(
            ID_QUIT,
            "",
            true,
            resource_ids::RHID_CANCEL,
            quit_x,
            btn_y,
            btn_w,
            btn_h,
        ));
        if let Some(w) = frame.widget_mut(ID_BUY) {
            w.base_mut().set_tooltip_text(
                &resources
                    .menu_text
                    .get(super::resources::MT_INFOBULLE_BUTTON_OK),
            );
        }
        if let Some(w) = frame.widget_mut(ID_QUIT) {
            w.base_mut().set_tooltip_text(
                &resources
                    .menu_text
                    .get(super::resources::MT_INFOBULLE_BUTTON_CANCEL),
            );
        }
        widget_bridge::attach_alpha_masks(&mut frame, resources, renderer);

        let mut input_state = ModalInputState::new();
        input_state.seed_mouse_from_window(event_pump, transform);
        Self {
            screen,
            frame,
            message: compose_message(resources, ransom, blazon_price),
            input_state,
            focused_index: 0,
            buy_x,
            quit_x,
            btn_y,
            btn_w,
            btn_h,
        }
    }

    pub fn tick(
        &mut self,
        event_pump: &mut crate::window::GameWindow,
        renderer: &mut Renderer,
        resources: &IngameMenuResources,
        cursor: Option<ModalCursor<'_>>,
    ) -> Option<BuyBlazonsOutcome> {
        if !self.screen.can_buy() {
            return Some(BuyBlazonsOutcome::Cancelled);
        }
        let transform = MenuTransform::centered(
            renderer.screen_width() as i32,
            renderer.screen_height() as i32,
        );
        let win_x = (MENU_W - WIN_W) / 2;
        let win_y = (MENU_H - WIN_H) / 2;
        let mut keyboard_activation = None;
        for event in event_pump.poll_events() {
            self.input_state.update_from_event(&event, transform);
            match event {
                GameEvent::Quit => self.screen.on_quit(),
                GameEvent::KeyDown {
                    keycode: Keycode::Tab | Keycode::Right | Keycode::Left,
                    ..
                } => self.focused_index = (self.focused_index + 1) % 2,
                GameEvent::KeyDown {
                    keycode: Keycode::Return,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::KpEnter,
                    ..
                } => {
                    keyboard_activation = Some(if self.focused_index == 0 {
                        ID_BUY
                    } else {
                        ID_QUIT
                    });
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => self.screen.on_quit(),
                _ => {}
            }
        }

        let widget_input = self.input_state.as_widget_input();
        let events = self.frame.process_input(&widget_input);
        self.input_state.end_frame();
        if let Some(id) = widget_bridge::find_activated(&events).or(keyboard_activation) {
            match id {
                ID_BUY => self.screen.on_buy(),
                ID_QUIT => self.screen.on_quit(),
                _ => {}
            }
        }

        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);
        if let Some(bg) = resources.menu_bg_small {
            draw_background(renderer, transform, &bg, win_x, win_y, WIN_W, WIN_H);
        } else {
            let (sx, sy) = transform.to_screen(win_x, win_y);
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
        let font = resources.debrief_font_any();
        if let Some(font) = font {
            let _ = render_text_in_box_aligned_font(
                renderer,
                font,
                transform,
                &self.message,
                win_x + MSG_X,
                win_y + MSG_Y,
                MSG_W,
                MSG_H,
                TextAlign::Center,
                VAlign::Top,
            );
        }
        widget_bridge::draw_frame_buttons(renderer, resources, transform, &self.frame);
        let (focused_x, focused_y) = if self.focused_index == 0 {
            (self.buy_x, self.btn_y)
        } else {
            (self.quit_x, self.btn_y)
        };
        let (sx, sy) = transform.to_screen(focused_x, focused_y);
        let (ex, ey) = transform.to_screen(focused_x + self.btn_w, focused_y + self.btn_h);
        renderer.draw_rect_outline_screen(
            sx - 1,
            sy - 1,
            ex + 1,
            ey + 1,
            Renderer::create_color_16(255, 220, 80),
        );
        if let Some(cursor) = &cursor {
            cursor.draw(renderer, transform, &self.input_state);
        }
        renderer.present();

        self.screen.closed.then_some(if self.screen.purchased {
            BuyBlazonsOutcome::Bought
        } else {
            BuyBlazonsOutcome::Cancelled
        })
    }
}

/// Compose the confirmation message using the localised menu-text entries.
///
/// Three lines separated by "\n":
///     STR_RANSOM        (e.g. "Ransom: 120")
///     STR_BLAZON_PRICE  (e.g. "Blazon price: 30")
///     MSG_BUY_BLAZON    (e.g. "Do you want to buy a blazon?")
fn compose_message(resources: &IngameMenuResources, ransom: u32, blazon_price: u32) -> String {
    let ransom_str = resources
        .menu_text
        .get(MT_STR_RANSOM)
        .replacen("%d", &ransom.to_string(), 1);
    let price_str =
        resources
            .menu_text
            .get(MT_STR_BLAZON_PRICE)
            .replacen("%d", &blazon_price.to_string(), 1);
    let confirm_str = resources.menu_text.get(MT_MSG_BUY_BLAZON);
    format!("{ransom_str}\n{price_str}\n{confirm_str}")
}

/// Display the blazon-purchase confirmation modal.
///
/// `mission_index` is threaded through into the returned
/// [`BuyBlazonsScreen`]-style outcome so the caller can look the mission
/// up again in `Campaign::missions` without the renderer needing a
/// borrow on it.  `blazon_price` and `ransom` are snapshotted at open
/// time — the modal closes on the first Buy / Quit rather than
/// refreshing in place, so polling live values would be wasted.
pub async fn show_buy_blazons(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    cursor: Option<ModalCursor<'_>>,
    mission_index: usize,
    blazon_price: u32,
    ransom: u32,
) -> BuyBlazonsOutcome {
    let mut screen = BuyBlazonsScreen::new(mission_index, blazon_price, ransom);

    // Defensive auto-close when ransom is below the blazon price.
    // The parent's Convert-Money button is enable-gated on
    // affordability, so we normally never hit this — but bail
    // defensively in case state drifts between the enable check and
    // the modal opening.
    if !screen.can_buy() {
        return BuyBlazonsOutcome::Cancelled;
    }

    let sw = renderer.screen_width() as i32;
    let sh = renderer.screen_height() as i32;
    let transform = MenuTransform::centered(sw, sh);

    let win_x = (MENU_W - WIN_W) / 2;
    let win_y = (MENU_H - WIN_H) / 2;

    // Buy uses the round `RHID_OK` seal; Quit uses `RHID_CANCEL` (the
    // "X" variant).  Both buttons get identical widths when centred;
    // take the max intrinsic size so both sprites render at their
    // native dimensions.
    let (ok_w, ok_h) = resources.ok_button_dimensions();
    let (cancel_w, cancel_h) = resources.cancel_button_dimensions();
    let btn_w = ok_w.max(cancel_w);
    let btn_h = ok_h.max(cancel_h);
    let total_w = 2 * btn_w + BUTTON_GAP;
    let buy_x = win_x + (WIN_W - total_w) / 2;
    let quit_x = buy_x + btn_w + BUTTON_GAP;
    let btn_y = win_y + BUTTON_ROW_Y;

    let mut frame = FrameWnd::default();
    frame.enabled = true;
    frame.input_enabled = true;
    frame.add_widget_absolute(widget_bridge::make_button_with_resource(
        ID_BUY,
        "",
        screen.can_buy(),
        resource_ids::RHID_OK,
        buy_x,
        btn_y,
        btn_w,
        btn_h,
    ));
    frame.add_widget_absolute(widget_bridge::make_button_with_resource(
        ID_QUIT,
        "",
        true,
        resource_ids::RHID_CANCEL,
        quit_x,
        btn_y,
        btn_w,
        btn_h,
    ));
    // Tooltips.
    if let Some(w) = frame.widget_mut(ID_BUY) {
        w.base_mut().set_tooltip_text(
            &resources
                .menu_text
                .get(super::resources::MT_INFOBULLE_BUTTON_OK),
        );
    }
    if let Some(w) = frame.widget_mut(ID_QUIT) {
        w.base_mut().set_tooltip_text(
            &resources
                .menu_text
                .get(super::resources::MT_INFOBULLE_BUTTON_CANCEL),
        );
    }

    // Bake the round-seal `RHID_OK` / `RHID_CANCEL` opacity masks so
    // the transparent corners around each seal don't capture clicks.
    widget_bridge::attach_alpha_masks(&mut frame, resources, renderer);

    let message = compose_message(resources, ransom, blazon_price);

    let mut input_state = ModalInputState::new();
    input_state.seed_mouse_from_window(event_pump, transform);

    // Keyboard focus index: 0 = Buy, 1 = Quit.  Tab / Left / Right
    // cycle between them and Enter activates whichever is focused.
    let mut focused_index: usize = 0;
    let mut keyboard_activation: Option<u32> = None;

    while !screen.closed {
        let (events, transform) = super::layout::poll_events_with_transform(event_pump, renderer);
        for event in events {
            input_state.update_from_event(&event, transform);
            match event {
                GameEvent::Quit => screen.on_quit(),
                GameEvent::KeyDown {
                    keycode: Keycode::Tab | Keycode::Right,
                    ..
                } => {
                    focused_index = (focused_index + 1) % 2;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Left,
                    ..
                } => {
                    focused_index = (focused_index + 1) % 2;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Return,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::KpEnter,
                    ..
                } => {
                    // Enter / Numpad-Enter fires the focus-manager
                    // shortcut bound to Buy.  Once focus moves onto
                    // Quit, Enter activates Quit instead — Return
                    // always selects the focused widget.
                    keyboard_activation = Some(if focused_index == 0 { ID_BUY } else { ID_QUIT });
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => screen.on_quit(),
                _ => {}
            }
        }

        let widget_input = input_state.as_widget_input();
        let events = frame.process_input(&widget_input);
        input_state.end_frame();

        let activated_id = widget_bridge::find_activated(&events).or(keyboard_activation);
        keyboard_activation = None;
        if let Some(id) = activated_id {
            match id {
                ID_BUY => screen.on_buy(),
                ID_QUIT => screen.on_quit(),
                _ => {}
            }
        }

        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);

        // Window background: `RHID_MENU_BACKGROUND_SMALL`.
        if let Some(bg) = resources.menu_bg_small {
            draw_background(renderer, transform, &bg, win_x, win_y, WIN_W, WIN_H);
        } else {
            let (sx, sy) = transform.to_screen(win_x, win_y);
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

        // Message text — centred horizontally inside the 300x75 box.
        // Use the "Debrief" font, falling back through the popup font
        // chain when it's missing.
        let font = resources.debrief_font_any();
        if let Some(font) = font {
            let _ = render_text_in_box_aligned_font(
                renderer,
                font,
                transform,
                &message,
                win_x + MSG_X,
                win_y + MSG_Y,
                MSG_W,
                MSG_H,
                TextAlign::Center,
                VAlign::Top,
            );
        }

        widget_bridge::draw_frame_buttons(renderer, resources, transform, &frame);

        // Keyboard-focus outline around the currently-focused button.
        let (focused_x, focused_y) = if focused_index == 0 {
            (buy_x, btn_y)
        } else {
            (quit_x, btn_y)
        };
        let (sx, sy) = transform.to_screen(focused_x, focused_y);
        let (ex, ey) = transform.to_screen(focused_x + btn_w, focused_y + btn_h);
        renderer.draw_rect_outline_screen(
            sx - 1,
            sy - 1,
            ex + 1,
            ey + 1,
            Renderer::create_color_16(255, 220, 80),
        );

        if let Some(c) = &cursor {
            c.draw(renderer, transform, &input_state);
        }

        renderer.present();
        crate::window::sleep_ui_frame().await;
    }

    if screen.purchased {
        BuyBlazonsOutcome::Bought
    } else {
        BuyBlazonsOutcome::Cancelled
    }
}
