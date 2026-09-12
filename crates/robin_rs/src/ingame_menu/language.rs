//! Runtime language selector for validated installed packs.

use crate::gfx_types::{GameEvent, Keycode};
use crate::host::ApplicationContext;
use crate::localization::{LanguageChange, LanguageSelection, PortTextKey};
use crate::renderer::Renderer;
use crate::widget::FrameWnd;

use super::layout::{
    MenuTransform, align_bottom_right, dim_screen, draw_screen_background, enter_modal_gpu_phase,
    render_text_virt_font,
};
use super::resources::{IngameMenuResources, MT_BTN_CANCEL};
use super::widget_bridge::{self, ModalCursor, ModalInputState, ModalScreenIo};

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
    let Some(mut state) =
        LanguageModalState::new(application_context, event_pump, renderer, resources)
    else {
        return false;
    };
    let mut io = ModalScreenIo {
        window: event_pump,
        renderer,
        resources,
        cursor: cursor.as_ref(),
    };
    loop {
        if let Some(changed) = state.tick(application_context, &mut io) {
            return changed;
        }
        crate::window::sleep_ui_frame().await;
    }
}

/// Language choices and transient errors live across frames, not in the IO bundle.
pub struct LanguageModalState {
    choices: Vec<(String, LanguageSelection)>,
    selected: usize,
    original_selection: LanguageSelection,
    active_locale: Option<String>,
    title: String,
    rows_per_column: usize,
    row_h: i32,
    btn_w: i32,
    frame: FrameWnd,
    input: ModalInputState,
    error_message: Option<String>,
    result: Option<bool>,
}

impl LanguageModalState {
    pub fn new(
        application_context: &ApplicationContext,
        event_pump: &crate::window::GameWindow,
        renderer: &Renderer,
        resources: &IngameMenuResources,
    ) -> Option<Self> {
        let packs = application_context
            .installed_languages()
            .unwrap_or_else(|error| panic!("Language screen lost its ApplicationContext: {error}"));
        if packs.len() < 2 {
            tracing::warn!("Language screen opened without two validated language packs");
            return None;
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

        let selected = match &preferences.selection {
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
        let bottom =
            align_bottom_right(&[(apply_label, true), (&cancel_label, true)], btn_w, btn_h);

        let mut frame = FrameWnd::interactive();
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
        let error_message: Option<String> = None;
        let input = ModalInputState::from_window(event_pump, transform);

        Some(Self {
            choices,
            selected,
            original_selection: preferences.selection,
            active_locale,
            title: title.to_owned(),
            rows_per_column,
            row_h,
            btn_w,
            frame,
            input,
            error_message,
            result: None,
        })
    }

    /// Run one frame; successful commits and cancellation stop before presentation.
    pub fn tick(
        &mut self,
        application_context: &ApplicationContext,
        io: &mut ModalScreenIo<'_, '_>,
    ) -> Option<bool> {
        if self.result.is_some() {
            return self.result;
        }
        let event_pump = &mut *io.window;
        let renderer = &mut *io.renderer;
        let resources = io.resources;
        let cursor = io.cursor;
        let mut apply = false;
        let mut cancel = false;
        let (events, transform) = super::layout::poll_events_with_transform(event_pump, renderer);
        for event in events {
            self.input.update_from_event(&event, transform);
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
        let events = self.input.process_frame(&mut self.frame);
        if let Some(id) = widget_bridge::find_activated(&events) {
            match id {
                ID_APPLY => apply = true,
                ID_CANCEL => cancel = true,
                id if (ID_LANGUAGE_BASE..ID_LANGUAGE_BASE + self.choices.len() as u32)
                    .contains(&id) =>
                {
                    self.selected = (id - ID_LANGUAGE_BASE) as usize;
                    self.error_message = None;
                }
                _ => {}
            }
        }

        if cancel {
            self.result = Some(false);
            return self.result;
        }
        if apply {
            let change = application_context.set_language(self.choices[self.selected].1.clone());
            if let Some(changed) = self.finish_language_change(change) {
                return Some(changed);
            }
        }

        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);
        if let Some(bg) = resources.menu_bg[2] {
            draw_screen_background(renderer, &bg);
        }

        if let Some(font) = resources.title_font_any() {
            let title = self.title.as_str();
            let x = (490 - font.text_width(title)) / 2;
            render_text_virt_font(renderer, font, transform, title, x, 20);
        }
        for (index, (label, selection)) in self.choices.iter().enumerate() {
            let Some(widget) = self.frame.widget(ID_LANGUAGE_BASE + index as u32) else {
                continue;
            };
            widget_bridge::draw_widget_radio(
                renderer,
                resources,
                transform,
                widget,
                self.selected == index,
            );
            if let Some(font) = resources.list_font(false, self.selected == index) {
                let column = index / self.rows_per_column;
                let row = index % self.rows_per_column;
                render_text_virt_font(
                    renderer,
                    font,
                    transform,
                    label,
                    36 + column as i32 * 300,
                    70 + row as i32 * (self.row_h + 3) + (self.row_h - font.height() as i32) / 2,
                );
            }
            if matches!(selection, LanguageSelection::Locale(locale) if Some(locale.as_str()) == self.active_locale.as_deref())
                && self.selected != index
            {
                // The selected radio is authoritative; this marker only makes
                // the currently active pack visible while browsing choices.
                if let Some(font) = resources.list_font(false, false) {
                    let column = index / self.rows_per_column;
                    let row = index % self.rows_per_column;
                    render_text_virt_font(
                        renderer,
                        font,
                        transform,
                        "•",
                        30 + column as i32 * 300 + self.btn_w.max(260) - 14,
                        70 + row as i32 * (self.row_h + 3),
                    );
                }
            }
        }

        if let Some(error) = self.error_message.as_deref()
            && let Some(font) = resources.list_font(false, false)
        {
            render_text_virt_font(renderer, font, transform, error, 30, 405);
        }
        for id in [ID_APPLY, ID_CANCEL] {
            if let Some(widget) = self.frame.widget(id) {
                widget_bridge::draw_widget_button(renderer, resources, transform, widget, false);
            }
        }
        if let Some(cursor) = cursor {
            cursor.draw(renderer, transform, &self.input);
        }
        renderer.present();

        None
    }

    fn finish_language_change(&mut self, change: Result<LanguageChange, String>) -> Option<bool> {
        match change {
            Ok(change) => {
                self.result = Some(
                    change.previous_locale != change.active_locale
                        || self.original_selection != self.choices[self.selected].1,
                );
            }
            Err(error) => {
                tracing::error!("Language switch rejected: {error}");
                self.error_message = Some(error);
            }
        }
        self.result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> LanguageModalState {
        LanguageModalState {
            choices: vec![("Automatic".into(), LanguageSelection::Auto)],
            selected: 0,
            original_selection: LanguageSelection::Auto,
            active_locale: Some("en".into()),
            title: "Language".into(),
            rows_per_column: 1,
            row_h: 25,
            btn_w: 260,
            frame: FrameWnd::interactive(),
            input: ModalInputState::new(),
            error_message: None,
            result: None,
        }
    }

    #[test]
    fn rejected_language_commit_keeps_screen_open_for_retry() {
        let mut state = state();
        assert_eq!(
            state.finish_language_change(Err("unavailable".into())),
            None
        );
        assert_eq!(state.error_message.as_deref(), Some("unavailable"));
        assert_eq!(state.active_locale.as_deref(), Some("en"));
        assert_eq!(state.result, None);
        assert_eq!(
            state.finish_language_change(Ok(LanguageChange {
                previous_locale: Some("en".into()),
                active_locale: Some("de".into()),
                generation: 1,
            })),
            Some(true)
        );
    }

    #[test]
    fn unchanged_locale_still_reports_an_explicit_preference_change() {
        for (selection, changed) in [
            (LanguageSelection::Auto, false),
            (LanguageSelection::Locale("en".into()), true),
        ] {
            let mut state = state();
            state.choices[0].1 = selection;
            assert_eq!(
                state.finish_language_change(Ok(LanguageChange {
                    previous_locale: Some("en".into()),
                    active_locale: Some("en".into()),
                    generation: 1,
                })),
                Some(changed)
            );
        }
    }
}
