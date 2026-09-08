//! Runtime language selector for validated installed packs.

use crate::gfx_types::{GameEvent, Keycode};
use crate::host::ApplicationContext;
use crate::localization::{LanguageSelection, PortTextKey};
use crate::renderer::Renderer;
use crate::widget::FrameWnd;

use super::layout::{
    MenuTransform, align_bottom_right, dim_screen, draw_screen_background, enter_modal_gpu_phase,
    render_text_virt_font,
};
use super::resources::{IngameMenuResources, MT_BTN_CANCEL};
use super::widget_bridge::{self, ModalCursor, ModalInputState};

const ID_LANGUAGE_BASE: u32 = 4_000;
const ID_APPLY: u32 = 4_100;
const ID_CANCEL: u32 = 4_101;

/// Select and commit a language. A failed commit stays on this screen and
/// displays the concrete error; the old locale remains installed.
pub async fn show_language(
    application_context: &ApplicationContext,
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    cursor: Option<ModalCursor<'_>>,
) -> bool {
    let packs = application_context
        .installed_languages()
        .unwrap_or_else(|error| panic!("Language screen lost its ApplicationContext: {error}"));
    if packs.len() < 2 {
        tracing::warn!("Language screen opened without two validated language packs");
        return false;
    }
    let preferences = application_context
        .localization_preferences()
        .unwrap_or_else(|error| panic!("Language screen lost its preferences: {error}"));
    let active_locale = application_context
        .active_locale()
        .unwrap_or_else(|error| panic!("Language screen lost its active locale: {error}"));

    let mut choices = Vec::with_capacity(packs.len() + 1);
    choices.push((
        application_context
            .port_text(PortTextKey::Automatic)
            .unwrap_or_else(|error| panic!("Language screen lost localized text: {error}"))
            .to_owned(),
        LanguageSelection::Auto,
    ));
    choices.extend(packs.iter().map(|pack| {
        (
            pack.native_name.clone(),
            LanguageSelection::Locale(pack.locale.clone()),
        )
    }));

    let mut selected = match &preferences.selection {
        LanguageSelection::Auto => 0,
        LanguageSelection::Locale(locale) => choices
            .iter()
            .position(|(_, selection)| {
                matches!(selection, LanguageSelection::Locale(candidate) if candidate == locale)
            })
            .unwrap_or(0),
    };

    let transform = MenuTransform::centered(
        renderer.screen_width() as i32,
        renderer.screen_height() as i32,
    );
    let (btn_w, btn_h) = resources.button_dimensions();
    let row_h = btn_h.max(25);
    let rows_per_column = choices.len().div_ceil(2).max(1);

    let apply_label = application_context
        .port_text(PortTextKey::Apply)
        .unwrap_or_else(|error| panic!("Language screen lost localized text: {error}"));
    let cancel_label = resources.menu_text.get(MT_BTN_CANCEL);
    let bottom = align_bottom_right(&[(apply_label, true), (&cancel_label, true)], btn_w, btn_h);

    let mut frame = FrameWnd::default();
    frame.enabled = true;
    frame.input_enabled = true;
    for (index, _) in choices.iter().enumerate() {
        let column = index / rows_per_column;
        let row = index % rows_per_column;
        frame.add_widget_absolute(widget_bridge::make_button(
            ID_LANGUAGE_BASE + index as u32,
            "",
            30 + column as i32 * 300,
            70 + row as i32 * (row_h + 3),
            btn_w.max(260),
            row_h,
        ));
    }
    frame.add_widget_absolute(widget_bridge::make_button(
        ID_APPLY,
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

    let title = application_context
        .port_text(PortTextKey::Language)
        .unwrap_or_else(|error| panic!("Language screen lost localized text: {error}"));
    let mut error_message: Option<String> = None;
    let mut input = ModalInputState::new();
    input.seed_mouse_from_window(event_pump, transform);

    loop {
        let mut apply = false;
        let mut cancel = false;
        for event in event_pump.poll_events() {
            input.update_from_event(&event, transform);
            match event {
                GameEvent::Quit
                | GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                } => cancel = true,
                GameEvent::KeyDown {
                    keycode: Keycode::Return | Keycode::KpEnter,
                    ..
                } => apply = true,
                _ => {}
            }
        }
        let events = frame.process_input(&input.as_widget_input());
        input.end_frame();
        if let Some(id) = widget_bridge::find_activated(&events) {
            match id {
                ID_APPLY => apply = true,
                ID_CANCEL => cancel = true,
                id if (ID_LANGUAGE_BASE..ID_LANGUAGE_BASE + choices.len() as u32).contains(&id) => {
                    selected = (id - ID_LANGUAGE_BASE) as usize;
                    error_message = None;
                }
                _ => {}
            }
        }

        if cancel {
            return false;
        }
        if apply {
            match application_context.set_language(choices[selected].1.clone()) {
                Ok(change) => {
                    return change.previous_locale != change.active_locale
                        || preferences.selection != choices[selected].1;
                }
                Err(error) => {
                    tracing::error!("Language switch rejected: {error}");
                    error_message = Some(error);
                }
            }
        }

        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);
        if let Some(bg) = resources.menu_bg[2] {
            draw_screen_background(renderer, &bg);
        }

        if let Some(font) = resources.title_font_any() {
            let x = (490 - font.text_width(title)) / 2;
            render_text_virt_font(renderer, font, transform, title, x, 20);
        }
        for (index, (label, selection)) in choices.iter().enumerate() {
            let Some(widget) = frame.widget(ID_LANGUAGE_BASE + index as u32) else {
                continue;
            };
            widget_bridge::draw_widget_radio(
                renderer,
                resources,
                transform,
                widget,
                selected == index,
            );
            if let Some(font) = resources.list_font(false, selected == index) {
                let column = index / rows_per_column;
                let row = index % rows_per_column;
                render_text_virt_font(
                    renderer,
                    font,
                    transform,
                    label,
                    36 + column as i32 * 300,
                    70 + row as i32 * (row_h + 3) + (row_h - font.height() as i32) / 2,
                );
            }
            if matches!(selection, LanguageSelection::Locale(locale) if Some(locale.as_str()) == active_locale.as_deref())
                && selected != index
            {
                // The selected radio is authoritative; this marker only makes
                // the currently active pack visible while browsing choices.
                if let Some(font) = resources.list_font(false, false) {
                    let column = index / rows_per_column;
                    let row = index % rows_per_column;
                    render_text_virt_font(
                        renderer,
                        font,
                        transform,
                        "•",
                        30 + column as i32 * 300 + btn_w.max(260) - 14,
                        70 + row as i32 * (row_h + 3),
                    );
                }
            }
        }

        if let Some(error) = error_message.as_deref()
            && let Some(font) = resources.list_font(false, false)
        {
            render_text_virt_font(renderer, font, transform, error, 30, 405);
        }
        for id in [ID_APPLY, ID_CANCEL] {
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
}
