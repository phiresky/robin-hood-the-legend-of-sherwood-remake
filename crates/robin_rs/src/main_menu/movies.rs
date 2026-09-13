//! Main-menu "Show Movies" entry.
//!
//! Displays `RHID_MENU_BACKGROUND_2` chrome, the "Show Movies" title in
//! the `MissionTitle` font, and three buttons: Play Intro, Play Outro
//! (gated on the active profile's progression == 100), and OK / Back.
//!
//! Dedicated sprite packs `RHID_INTRO` / `RHID_OUTRO` aren't loaded into
//! [`IngameMenuResources`] yet, so the Intro / Outro buttons render as
//! standard menu buttons with localised text labels instead. Tracked as
//! a deliberate deviation from the original game's menu behavior.

use crate::application::require;
use crate::gfx_types::{GameEvent, Keycode};
use crate::host::ApplicationContext;
use crate::ingame_menu::IngameMenuResources;
use crate::ingame_menu::layout::{
    MENU_H, MENU_W, align_bottom_right, draw_screen_background, enter_modal_gpu_phase,
    render_text_virt_font,
};
use crate::ingame_menu::resources::{MT_BTN_BACK, MT_BTN_SHOW_MOVIES};
use crate::ingame_menu::widget_bridge::{
    self, ModalInputState, ModalScreenIo, ScreenFrame, ScreenKey,
};
use crate::ui::UiState;
use crate::widget::FrameWnd;

const ID_INTRO: u32 = 0;
const ID_OUTRO: u32 = 1;
const ID_OK: u32 = 2;

/// Display the movies menu. Returns once the player picks Back / Escape.
///
/// The frame loop stays here instead of `run_modal` because a tick awaits
/// video playback.
pub(crate) async fn show_movies(
    application_context: &ApplicationContext,
    io: &mut ModalScreenIo<'_, '_>,
) {
    let mut state = MoviesModalState::new(application_context, io.resources);
    loop {
        if state.tick(application_context, io).await {
            return;
        }
        crate::window::sleep_ui_frame().await;
    }
}

/// Live input and movie selection persist across frames; this is not serialized.
struct MoviesModalState {
    title: String,
    back: String,
    outro_enabled: bool,
    intro_label: String,
    outro_label: String,
    btn_w: i32,
    btn_h: i32,
    outro_y: i32,
    ok_position: (i32, i32),
    input_state: ModalInputState,
    keyboard_selection: u32,
}

impl MoviesModalState {
    fn new(application_context: &ApplicationContext, resources: &IngameMenuResources) -> Self {
        let title = resources.menu_text.get(MT_BTN_SHOW_MOVIES);
        let back = resources.menu_text.get(MT_BTN_BACK);

        // Outro stays out of the focus group until the player has finished
        // the campaign (progression < 100).
        let outro_enabled = require(
            application_context.with_active_profile(|profile| profile.progression >= 100),
            "Show Movies screen",
        );

        // Localised labels for the Intro / Outro buttons. The original game
        // leaves the label empty and relies on the sprite to convey meaning;
        // until the dedicated sprite packs are loaded, fall back to text
        // labels so the buttons are distinguishable.
        let intro_label = "Play Intro".to_string();
        let outro_label = "Play Outro".to_string();

        let (btn_w, btn_h) = resources.button_dimensions();

        // Intro at (110, 80); Outro at +30 below.
        const INTRO_Y: i32 = 80;
        const OUTRO_SPACING: i32 = 30;
        let outro_y = INTRO_Y + btn_h + OUTRO_SPACING;

        // OK button: bottom-right via `align_bottom_right`.
        let ok_layout = align_bottom_right(&[(&back, true)], btn_w, btn_h).remove(0);

        let input_state = ModalInputState::new();
        let keyboard_selection: u32 = ID_INTRO;

        Self {
            title,
            back,
            outro_enabled,
            intro_label,
            outro_label,
            btn_w,
            btn_h,
            outro_y,
            input_state,
            keyboard_selection,
            ok_position: (ok_layout.x, ok_layout.y),
        }
    }

    async fn tick(
        &mut self,
        application_context: &ApplicationContext,
        io: &mut ModalScreenIo<'_, '_>,
    ) -> bool {
        const INTRO_X: i32 = 110;
        const INTRO_Y: i32 = 80;
        // Build the frame fresh each frame so state changes are picked up
        // (matches the pattern other in-place sub-menus use).
        let mut frame = FrameWnd::interactive();
        frame.add_widget_absolute(widget_bridge::make_button_enabled(
            ID_INTRO,
            &self.intro_label,
            true,
            INTRO_X,
            INTRO_Y,
            self.btn_w,
            self.btn_h,
        ));
        frame.add_widget_absolute(widget_bridge::make_button_enabled(
            ID_OUTRO,
            &self.outro_label,
            self.outro_enabled,
            INTRO_X,
            self.outro_y,
            self.btn_w,
            self.btn_h,
        ));
        frame.add_widget_absolute(widget_bridge::make_button_enabled(
            ID_OK,
            &self.back,
            true,
            self.ok_position.0,
            self.ok_position.1,
            self.btn_w,
            self.btn_h,
        ));

        // ── Events ──────────────────────────────────────────────
        let mut activated: Option<u32> = None;
        let screen = ScreenFrame::begin(io, &mut self.input_state);
        // One ordered pass: Up/Down before Return changes what Return activates.
        for event in &screen.events {
            match ScreenKey::from_event(event) {
                Some(ScreenKey::Quit | ScreenKey::Cancel) => activated = Some(ID_OK),
                Some(ScreenKey::Confirm) => activated = Some(self.keyboard_selection),
                Some(ScreenKey::Next) => {}
                None => match event {
                    GameEvent::KeyDown {
                        keycode: Keycode::Up,
                        ..
                    } => super::move_keyboard_selection(&frame, &mut self.keyboard_selection, -1),
                    GameEvent::KeyDown {
                        keycode: Keycode::Down,
                        ..
                    } => super::move_keyboard_selection(&frame, &mut self.keyboard_selection, 1),
                    _ => {}
                },
            }
        }

        let (_, widget_activated) = ScreenFrame::dispatch(&mut self.input_state, &mut frame);

        for w in frame.widgets() {
            if w.base().state != UiState::Default && w.base().enabled {
                self.keyboard_selection = w.id();
            }
        }
        if let Some(id) = widget_activated {
            activated = Some(id);
        }

        if let Some(id) = activated {
            match id {
                ID_INTRO => {
                    if let Err(e) = crate::video_player::play_video(
                        application_context,
                        io.window,
                        "Data/Cinematics/Intro.ogg",
                    )
                    .await
                    {
                        tracing::warn!("Intro video error: {e}");
                    }
                }
                ID_OUTRO if self.outro_enabled => {
                    if let Err(e) = crate::video_player::play_video(
                        application_context,
                        io.window,
                        "Data/Cinematics/Outro.ogg",
                    )
                    .await
                    {
                        tracing::warn!("Outro video error: {e}");
                    }
                }
                ID_OK => return true,
                _ => {}
            }
        }

        // ── Render ──────────────────────────────────────────────
        // No dim: the movies menu paints its own opaque background, so it
        // enters the modal phase directly instead of `ScreenFrame::begin_draw`.
        let transform = screen.transform;
        let renderer = &mut *io.renderer;
        let resources = io.resources;
        enter_modal_gpu_phase(renderer);

        if let Some(bg) = resources.menu_bg[2] {
            draw_screen_background(renderer, &bg);
        } else {
            // No `RHID_MENU_BACKGROUND_2` available — fall back to dim so
            // we at least get visible button chrome.
            renderer.render_gpu_rect(0, 0, MENU_W, MENU_H, [0, 0, 0, 255]);
        }

        // Title — centre the string horizontally inside the 0..500 column,
        // matching the original layout's title label box.
        if let Some(font) = resources.title_font_any() {
            let tw = font.text_width(&self.title);
            let x = (500 - tw) / 2;
            render_text_virt_font(renderer, font, transform, &self.title, x, 20);
        }

        for widget in frame.widgets() {
            let kb_highlight =
                widget.id() == self.keyboard_selection && widget.base().state == UiState::Default;
            widget_bridge::draw_widget_button(renderer, resources, transform, widget, kb_highlight);
        }

        screen.finish(io, &self.input_state);

        false
    }
}
