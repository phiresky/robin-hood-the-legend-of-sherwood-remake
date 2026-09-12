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
use super::widget_bridge::{self, ModalCursor, ModalInputState, ModalScreenIo};

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
    let mut state = MultiplayerPrivacyModalState::new(event_pump, renderer, resources, config);
    let mut io = ModalScreenIo {
        window: event_pump,
        renderer,
        resources,
        cursor: cursor.as_ref(),
    };
    loop {
        let done = state.tick(&mut io);
        crate::window::sleep_ui_frame().await;
        if done {
            return state.commit(config);
        }
    }
}

/// Retained widget/input and staged preference state for one modal frame.
pub struct MultiplayerPrivacyModalState {
    working: MultiplayerConfig,
    dirty: bool,
    frame: FrameWnd,
    input: ModalInputState,
    accepted: bool,
    done: bool,
}

impl MultiplayerPrivacyModalState {
    pub fn new(
        event_pump: &crate::window::GameWindow,
        renderer: &Renderer,
        resources: &IngameMenuResources,
        config: &MultiplayerConfig,
    ) -> Self {
        let transform = MenuTransform::centered(
            renderer.screen_width() as i32,
            renderer.screen_height() as i32,
        );
        let working = *config;
        let dirty = false;
        let (btn_w, btn_h) = resources.button_dimensions();
        let ok = resources.menu_text.get(MT_BTN_OK);
        let cancel = resources.menu_text.get(MT_BTN_CANCEL);
        let bottom = align_bottom_right(&[(&ok, true), (&cancel, true)], btn_w, btn_h);
        let (field_w, field_h) = resources.input_field_dimensions();

        let mut frame = FrameWnd::interactive();
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

        let input = ModalInputState::from_window(event_pump, transform);

        Self {
            working,
            dirty,
            frame,
            input,
            accepted: false,
            done: false,
        }
    }

    /// Poll and draw exactly one frame; the caller owns pacing.
    pub fn tick(&mut self, io: &mut ModalScreenIo<'_, '_>) -> bool {
        let event_pump = &mut *io.window;
        let renderer = &mut *io.renderer;
        let resources = io.resources;
        let cursor = io.cursor;
        if self.done {
            return true;
        }
        let (events, transform) = super::layout::poll_events_with_transform(event_pump, renderer);
        for event in events {
            self.input.update_from_event(&event, transform);
            match event {
                GameEvent::Quit => self.done = true,
                GameEvent::KeyDown {
                    keycode: Keycode::Return | Keycode::KpEnter,
                    ..
                } => {
                    self.accepted = true;
                    self.done = true;
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => self.done = true,
                _ => {}
            }
        }
        let widget_events = self.input.process_frame(&mut self.frame);
        if let Some(id) = widget_bridge::find_activated(&widget_events) {
            self.activate(id);
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
        if let Some(widget) = self.frame.widget(ID_PUBLICATION) {
            widget_bridge::draw_widget_radio(
                renderer,
                resources,
                transform,
                widget,
                self.working.publish_browser_join_links,
            );
        }
        for id in [ID_OK, ID_CANCEL] {
            if let Some(widget) = self.frame.widget(id) {
                widget_bridge::draw_widget_button(renderer, resources, transform, widget, false);
            }
        }
        if let Some(cursor) = cursor {
            cursor.draw(renderer, transform, &self.input);
        }
        renderer.present();

        self.done
    }

    fn activate(&mut self, id: u32) {
        match id {
            ID_PUBLICATION => {
                self.working.publish_browser_join_links = !self.working.publish_browser_join_links;
                self.dirty = true;
            }
            ID_OK => {
                self.accepted = true;
                self.done = true;
            }
            ID_CANCEL => self.done = true,
            _ => {}
        }
    }

    /// Publish staged changes only after acceptance and an actual difference.
    pub fn commit(self, target: &mut MultiplayerConfig) -> bool {
        if self.accepted && self.dirty && self.working != *target {
            *target = self.working;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(config: MultiplayerConfig) -> MultiplayerPrivacyModalState {
        MultiplayerPrivacyModalState {
            working: config,
            dirty: false,
            frame: FrameWnd::interactive(),
            input: ModalInputState::new(),
            accepted: false,
            done: false,
        }
    }

    #[test]
    fn staged_publication_requires_acceptance_and_a_real_change() {
        for (actions, changed) in [
            (vec![ID_PUBLICATION, ID_OK], true),
            (vec![ID_PUBLICATION, ID_CANCEL], false),
            (vec![ID_PUBLICATION, ID_PUBLICATION, ID_OK], false),
            (vec![ID_OK], false),
        ] {
            let original = MultiplayerConfig::default();
            let mut target = original;
            let mut state = state(original);
            for action in actions {
                state.activate(action);
                assert_eq!(target, original, "editing must not publish staged changes");
            }
            assert!(state.done);
            assert_eq!(state.commit(&mut target), changed);
            assert_eq!(target != original, changed);
        }
    }

    #[test]
    fn privacy_row_controls_only_browser_publication() {
        let original = MultiplayerConfig::default();
        let mut toggled = original;
        toggled.publish_browser_join_links = !toggled.publish_browser_join_links;
        assert_ne!(toggled, original);
    }
}
