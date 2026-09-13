//! Multiplayer publication/privacy preferences.

use crate::widget::FrameWnd;
use robin_engine::multiplayer_config::MultiplayerConfig;

use super::layout::{align_bottom_right, draw_screen_background, render_text_virt_font};
use super::resources::{MT_BTN_CANCEL, MT_BTN_OK};
use super::widget_bridge::{self, ModalInputState, ModalScreenIo, ScreenFrame, ScreenKey};

const ID_PUBLICATION: u32 = 200;
const ID_OK: u32 = 300;
const ID_CANCEL: u32 = 301;

pub async fn show_multiplayer_privacy(
    io: &mut ModalScreenIo<'_, '_>,
    config: &mut MultiplayerConfig,
) -> bool {
    let mut state = MultiplayerPrivacyModalState::new(io, config);
    widget_bridge::run_modal(io, |io| state.tick(io)).await;
    state.commit(config)
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
    pub fn new(io: &ModalScreenIo<'_, '_>, config: &MultiplayerConfig) -> Self {
        let resources = io.resources;
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

        let input = ModalInputState::for_screen(io.window, io.renderer);

        Self {
            working,
            dirty,
            frame,
            input,
            accepted: false,
            done: false,
        }
    }

    /// Poll and draw exactly one frame; the caller owns pacing. The frame that
    /// closes the screen is still drawn and paced: `Some(())` is only reported
    /// on the following tick, before polling.
    pub fn tick(&mut self, io: &mut ModalScreenIo<'_, '_>) -> Option<()> {
        if self.done {
            return Some(());
        }
        let screen = ScreenFrame::begin(io, &mut self.input);
        for key in screen.keys() {
            match key {
                ScreenKey::Quit | ScreenKey::Cancel => self.done = true,
                ScreenKey::Confirm => {
                    self.accepted = true;
                    self.done = true;
                }
                ScreenKey::Next => {}
            }
        }
        let (_, activated) = ScreenFrame::dispatch(&mut self.input, &mut self.frame);
        if let Some(id) = activated {
            self.activate(id);
        }

        let transform = screen.transform;
        let renderer = &mut *io.renderer;
        let resources = io.resources;
        screen.begin_draw(renderer);
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
        screen.finish(io, &self.input);

        None
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
