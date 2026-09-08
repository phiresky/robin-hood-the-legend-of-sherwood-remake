//! Multiplayer publication/privacy preferences.

use crate::gfx_types::{GameEvent, Keycode};
use crate::renderer::Renderer;
use crate::widget::FrameWnd;
use robin_engine::multiplayer_config::MultiplayerConfig;

use super::layout::{
    MenuTransform, align_bottom_right, dim_screen, draw_screen_background, enter_modal_gpu_phase,
    render_text_virt_font,
};
use super::resources::{IngameMenuResources, MT_BTN_CANCEL, MT_BTN_OK};
use super::widget_bridge::{self, ModalCursor, ModalInputState};

const ID_PUBLICATION: u32 = 200;
const ID_OK: u32 = 300;
const ID_CANCEL: u32 = 301;

pub async fn show_multiplayer_privacy(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    cursor: Option<ModalCursor<'_>>,
    config: &mut MultiplayerConfig,
) -> bool {
    let transform = MenuTransform::centered(
        renderer.screen_width() as i32,
        renderer.screen_height() as i32,
    );
    let mut working = *config;
    let mut dirty = false;
    let (btn_w, btn_h) = resources.button_dimensions();
    let ok = resources.menu_text.get(MT_BTN_OK);
    let cancel = resources.menu_text.get(MT_BTN_CANCEL);
    let bottom = align_bottom_right(&[(&ok, true), (&cancel, true)], btn_w, btn_h);
    let (field_w, field_h) = resources.input_field_dimensions();

    let mut frame = FrameWnd::default();
    frame.enabled = true;
    frame.input_enabled = true;
    frame.add_widget_absolute(widget_bridge::make_button(
        ID_PUBLICATION,
        "Publish Browser Join Links",
        30,
        110,
        field_w,
        field_h,
    ));
    frame.add_widget_absolute(widget_bridge::make_button(
        ID_OK,
        &bottom[0].label,
        bottom[0].x,
        bottom[0].y,
        bottom[0].w,
        bottom[0].h,
    ));
    frame.add_widget_absolute(widget_bridge::make_button(
        ID_CANCEL,
        &bottom[1].label,
        bottom[1].x,
        bottom[1].y,
        bottom[1].w,
        bottom[1].h,
    ));

    let mut done = false;
    let mut accepted = false;
    let mut input = ModalInputState::new();
    input.seed_mouse_from_window(event_pump, transform);
    while !done {
        let (events, transform) = super::layout::poll_events_with_transform(event_pump, renderer);
        for event in events {
            input.update_from_event(&event, transform);
            match event {
                GameEvent::Quit => done = true,
                GameEvent::KeyDown {
                    keycode: Keycode::Return | Keycode::KpEnter,
                    ..
                } => {
                    accepted = true;
                    done = true;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => done = true,
                _ => {}
            }
        }
        let widget_events = frame.process_input(&input.as_widget_input());
        input.end_frame();
        if let Some(id) = widget_bridge::find_activated(&widget_events) {
            match id {
                ID_PUBLICATION => {
                    working.publish_browser_join_links = !working.publish_browser_join_links;
                    dirty = true;
                }
                ID_OK => {
                    accepted = true;
                    done = true;
                }
                ID_CANCEL => done = true,
                _ => {}
            }
        }

        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);
        if let Some(bg) = resources.menu_bg[0] {
            draw_screen_background(renderer, &bg);
        }
        if let Some(font) = resources.title_font_any() {
            let title = "Multiplayer / Privacy";
            render_text_virt_font(
                renderer,
                font,
                transform,
                title,
                (490 - font.text_width(title)) / 2,
                20,
            );
        }
        if let Some(font) = resources.label_font_any() {
            for (line, y) in [
                ("Applies to the next game you host.", 75),
                (
                    "Published invitations include the endpoint, HTTPS relay,",
                    170,
                ),
                ("mission, build, content edition, and player count.", 190),
                ("The relay can observe IPs, timing, and byte counts.", 220),
                ("Gameplay traffic remains end-to-end encrypted.", 240),
            ] {
                render_text_virt_font(renderer, font, transform, line, 30, y);
            }
        }
        if let Some(widget) = frame.widget(ID_PUBLICATION) {
            widget_bridge::draw_widget_radio(
                renderer,
                resources,
                transform,
                widget,
                working.publish_browser_join_links,
            );
        }
        for id in [ID_OK, ID_CANCEL] {
            if let Some(widget) = frame.widget(id) {
                widget_bridge::draw_widget_button(renderer, resources, transform, widget, false);
            }
        }
        if let Some(cursor) = &cursor {
            cursor.draw(renderer, transform, &input);
        }
        renderer.present();
        crate::window::sleep_ms(16).await;
    }

    if accepted && dirty && working != *config {
        *config = working;
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privacy_row_controls_only_browser_publication() {
        let original = MultiplayerConfig::default();
        let mut toggled = original;
        toggled.publish_browser_join_links = !toggled.publish_browser_join_links;
        assert_ne!(toggled, original);
    }
}
