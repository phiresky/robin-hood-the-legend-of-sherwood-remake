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

use crate::gfx_types::{GameEvent, Keycode};
use crate::host::ApplicationContext;
use crate::ingame_menu::IngameMenuResources;
use crate::ingame_menu::layout::{
    MENU_H, MENU_W, align_bottom_right, draw_screen_background, enter_modal_gpu_phase,
    render_text_virt_font,
};
use crate::ingame_menu::resources::{MT_BTN_BACK, MT_BTN_SHOW_MOVIES};
use crate::ingame_menu::widget_bridge::{self, ModalInputState, ModalScreenIo};
use crate::renderer::Renderer;
use crate::ui::UiState;
use crate::widget::FrameWnd;

const ID_INTRO: u32 = 0;
const ID_OUTRO: u32 = 1;
const ID_OK: u32 = 2;

/// Display the movies menu. Returns once the player picks Back / Escape.
pub(crate) async fn show_movies(
    application_context: &ApplicationContext,
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
) {
    let mut state = MoviesModalState::new(application_context, resources);
    let mut io = ModalScreenIo {
        window: event_pump,
        renderer,
        resources,
        cursor: None,
    };
    loop {
        if state.tick(application_context, &mut io).await {
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
        let outro_enabled = application_context
            .with_active_profile(|profile| profile.progression >= 100)
            .unwrap_or_else(|error| panic!("Show Movies requires an active profile: {error}"));

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
        let event_pump = &mut *io.window;
        let renderer = &mut *io.renderer;
        let resources = io.resources;
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
        let (events, transform) =
            crate::ingame_menu::layout::poll_events_with_transform(event_pump, renderer);
        for event in events {
            self.input_state.update_from_event(&event, transform);
            match event {
                GameEvent::Quit
                | GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => {
                    activated = Some(ID_OK);
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Up,
                    ..
                } => super::move_keyboard_selection(&frame, &mut self.keyboard_selection, -1),
                GameEvent::KeyDown {
                    keycode: Keycode::Down,
                    ..
                } => super::move_keyboard_selection(&frame, &mut self.keyboard_selection, 1),
                GameEvent::KeyDown {
                    keycode: Keycode::Return,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::KpEnter,
                    ..
                } => {
                    activated = Some(self.keyboard_selection);
                }
                _ => {}
            }
        }

        let widget_input = self.input_state.as_widget_input();
        let widget_events = frame.process_input(&widget_input);
        self.input_state.end_frame();

        for w in frame.widgets() {
            if w.base().state != UiState::Default && w.base().enabled {
                self.keyboard_selection = w.id();
            }
        }
        if let Some(id) = widget_bridge::find_activated(&widget_events) {
            activated = Some(id);
        }

        if let Some(id) = activated {
            match id {
                ID_INTRO => {
                    if let Err(e) = crate::video_player::play_video(
                        application_context,
                        event_pump,
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
                        event_pump,
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

        renderer.present();

        false
    }
}
