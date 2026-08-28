//! Main-menu "Show Movies" entry.
//!
//! Displays `RHID_MENU_BACKGROUND_2` chrome, the "Show Movies" title in
//! the `MissionTitle` font, and three buttons: Play Intro, Play Outro
//! (gated on the active profile's progression == 100), and OK / Back.
//!
//! Dedicated sprite packs `RHID_INTRO` / `RHID_OUTRO` aren't loaded into
//! [`IngameMenuResources`] yet, so the Intro / Outro buttons render as
//! standard menu buttons with localised text labels instead. Tracked as
//! a deliberate deviation in `parity-audit/RHMenuMovies-01.md`.

use crate::gfx_types::{GameEvent, Keycode};
use crate::host::ApplicationContext;
use crate::ingame_menu::IngameMenuResources;
use crate::ingame_menu::layout::{
    MENU_H, MENU_W, MenuTransform, align_bottom_right, draw_screen_background,
    enter_modal_gpu_phase, render_text_virt,
};
use crate::ingame_menu::resources::{MT_BTN_BACK, MT_BTN_SHOW_MOVIES};
use crate::ingame_menu::widget_bridge::{self, ModalInputState};
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
    let title = resources.menu_text.get(MT_BTN_SHOW_MOVIES);
    let back = resources.menu_text.get(MT_BTN_BACK);

    // Outro stays out of the focus group until the player has finished
    // the campaign (progression < 100).
    let outro_enabled = application_context
        .active_profile_snapshot()
        .unwrap_or_else(|error| panic!("Show Movies requires an active profile: {error}"))
        .progression
        >= 100;

    // Localised labels for the Intro / Outro buttons. The original game
    // leaves the label empty and relies on the sprite to convey meaning;
    // until the dedicated sprite packs are loaded, fall back to text
    // labels so the buttons are distinguishable.
    let intro_label = "Play Intro".to_string();
    let outro_label = "Play Outro".to_string();

    let (btn_w, btn_h) = resources.button_dimensions();

    // Intro at (110, 80); Outro at +30 below.
    const INTRO_X: i32 = 110;
    const INTRO_Y: i32 = 80;
    const OUTRO_SPACING: i32 = 30;
    let outro_y = INTRO_Y + btn_h + OUTRO_SPACING;

    // OK button: bottom-right via `align_bottom_right`.
    let ok_layout = &align_bottom_right(&[(&back, true)], btn_w, btn_h)[0];

    let mut input_state = ModalInputState::new();
    let mut keyboard_selection: u32 = ID_INTRO;

    loop {
        let sw = renderer.screen_width() as i32;
        let sh = renderer.screen_height() as i32;
        let transform = MenuTransform::centered(sw, sh);

        // Build the frame fresh each frame so state changes are picked up
        // (matches the pattern other in-place sub-menus use).
        let mut frame = FrameWnd::default();
        frame.enabled = true;
        frame.input_enabled = true;
        frame.add_widget_absolute(widget_bridge::make_button_enabled(
            ID_INTRO,
            &intro_label,
            true,
            INTRO_X,
            INTRO_Y,
            btn_w,
            btn_h,
        ));
        frame.add_widget_absolute(widget_bridge::make_button_enabled(
            ID_OUTRO,
            &outro_label,
            outro_enabled,
            INTRO_X,
            outro_y,
            btn_w,
            btn_h,
        ));
        frame.add_widget_absolute(widget_bridge::make_button_enabled(
            ID_OK,
            &back,
            true,
            ok_layout.x,
            ok_layout.y,
            btn_w,
            btn_h,
        ));

        // ── Events ──────────────────────────────────────────────
        let mut activated: Option<u32> = None;
        for event in event_pump.poll_events() {
            input_state.update_from_event(&event, transform);
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
                } => move_keyboard_selection(&frame, &mut keyboard_selection, -1),
                GameEvent::KeyDown {
                    keycode: Keycode::Down,
                    ..
                } => move_keyboard_selection(&frame, &mut keyboard_selection, 1),
                GameEvent::KeyDown {
                    keycode: Keycode::Return,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::KpEnter,
                    ..
                } => {
                    activated = Some(keyboard_selection);
                }
                _ => {}
            }
        }

        let widget_input = input_state.as_widget_input();
        let widget_events = frame.process_input(&widget_input);
        input_state.end_frame();

        for w in frame.widgets() {
            if w.base().state != UiState::Default && w.base().enabled {
                keyboard_selection = w.id();
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
                ID_OUTRO if outro_enabled => {
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
                ID_OK => return,
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
            renderer.render_gpu_rect(0, 0, MENU_W, MENU_H, 0, 0, 0, 255);
        }

        // Title — centre the string horizontally inside the 0..500 column,
        // matching the original layout's title label box.
        if let Some(font) = resources.title_font() {
            let tw = font.text_width(&title);
            let x = (500 - tw) / 2;
            render_text_virt(renderer, font, transform, &title, x, 20);
        }

        for widget in frame.widgets() {
            let kb_highlight =
                widget.id() == keyboard_selection && widget.base().state == UiState::Default;
            widget_bridge::draw_widget_button(renderer, resources, transform, widget, kb_highlight);
        }

        renderer.present();
        crate::window::sleep_ui_frame().await;
    }
}

fn move_keyboard_selection(frame: &FrameWnd, selection: &mut u32, direction: i32) {
    let len = frame.widget_count() as i32;
    if len == 0 {
        return;
    }
    let mut idx = *selection as i32;
    for _ in 0..len {
        idx = (idx + direction).rem_euclid(len);
        if let Some(w) = frame.widget_at(idx as usize)
            && w.base().enabled
        {
            *selection = idx as u32;
            break;
        }
    }
}
