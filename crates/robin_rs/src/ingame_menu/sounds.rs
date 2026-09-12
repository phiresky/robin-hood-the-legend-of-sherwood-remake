//! Sound settings sub-screen.
//!
//! Radio buttons for audio mode/resolution, five volume sliders, plus
//! OK / Cancel.
//!
//! Everything (radios, sliders, OK/Cancel) is driven by the
//! [`crate::widget`] system and rendered through the bridge.  The
//! sliders use `WidgetSlider` with `step_count = 10` so they snap to
//! the config's 0..9 tick range and only emit
//! `UiMsg::WidgetSliderTrack` on tick transitions.

use crate::gfx_types::Keycode;
use robin_engine::coordinates::ScreenBBox;
use robin_engine::sound_cache::SampleLoader;

use crate::gfx_types::GameEvent;
use crate::options_model::SoundSetting;
use crate::renderer::Renderer;
use crate::sound::{AudioBackend, SoundManager};
use crate::ui::{UiEvent, UiMsg, UiState};
use crate::widget::{FrameWnd, Widget, WidgetSlider};
use robin_engine::sound_config::SoundConfig;

use super::layout::{
    MenuRect, MenuTransform, align_bottom_right, align_on_first_widget, dim_screen,
    draw_screen_background, draw_slider, enter_modal_gpu_phase, render_text_virt_font,
};
use super::resources::{
    IngameMenuResources, MT_BTN_CANCEL, MT_BTN_OK, MT_STR_SOUND_3D, MT_STR_SOUND_COMMENT_FREQUENCY,
    MT_STR_SOUND_EAX, MT_STR_SOUND_RES_HIGH, MT_STR_SOUND_RES_LOW, MT_STR_SOUND_STEREO,
    MT_STR_SOUND_VOL_COMMENT, MT_STR_SOUND_VOL_DIALOGUE, MT_STR_SOUND_VOL_FX,
    MT_STR_SOUND_VOL_MUSIC, MT_TTL_SOUNDS,
};
use super::widget_bridge::{self, ModalCursor, ModalInputState};

// Widget ID ranges.
const ID_MODE_BASE: u32 = 100; // Stereo=100, EAX=101
const ID_RES_BASE: u32 = 200; // High=200, Low=201
const ID_OK: u32 = 300;
const ID_CANCEL: u32 = 301;
const ID_SLIDER_BASE: u32 = 400; // 400..404 — one per volume slider

/// Config's discrete volume range: 0..=9 inclusive, so 10 ticks.
const SLIDER_STEPS: u32 = 10;
const SLIDER_MAX: u16 = 9;

const SOUND_SLIDERS: [(SoundSetting, usize); 5] = [
    (SoundSetting::FxVolume, MT_STR_SOUND_VOL_FX),
    (SoundSetting::DialogueVolume, MT_STR_SOUND_VOL_DIALOGUE),
    (SoundSetting::MusicVolume, MT_STR_SOUND_VOL_MUSIC),
    (SoundSetting::CommentVolume, MT_STR_SOUND_VOL_COMMENT),
    (
        SoundSetting::CommentFrequency,
        MT_STR_SOUND_COMMENT_FREQUENCY,
    ),
];

/// Display the sounds sub-screen.  Returns `true` on OK when anything changed.
///
/// `sound` / `audio_backend` / `sample_loader` are threaded in so the
/// menu can play the slider "tick" sounds
/// (`RHWIDGETNOISY_SLIDER << 16 | *`) as the user hovers, drags, and
/// releases a volume slider. When any of them is `None` (e.g. the main-
/// menu entry path has no live `SoundManager`), the slider is silent.
#[allow(clippy::too_many_arguments)]
pub async fn show_sounds(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    cursor: Option<ModalCursor<'_>>,
    config: &mut SoundConfig,
    mut sound: Option<&mut SoundManager>,
    mut audio_backend: Option<&mut dyn AudioBackend>,
    sample_loader: Option<&SampleLoader>,
) -> bool {
    let transform = MenuTransform::centered(
        renderer.screen_width() as i32,
        renderer.screen_height() as i32,
    );
    let input_state = ModalInputState::from_window(event_pump, transform);
    let mut screen = SoundsScreen::new(resources, config, input_state, sound.as_deref());
    while !screen.done {
        screen.tick(
            widget_bridge::ModalScreenIo {
                window: event_pump,
                renderer,
                resources,
                cursor: cursor.as_ref(),
            },
            &mut sound,
            &mut audio_backend,
            sample_loader,
        );
        // Preserve the original final-frame presentation and sleep on close.
        crate::window::sleep_ui_frame().await;
    }
    screen.finish(config)
}

/// Live modal owner: keyboard capture and widget interaction state cannot be
/// restored from serialization. Edited configuration remains ordinary data.
struct SoundsScreen {
    edit: crate::options_model::SoundEdit,
    dirty: bool,
    frame: FrameWnd,
    slider_rects: [MenuRect; SOUND_SLIDERS.len()],
    slider_labels: [String; SOUND_SLIDERS.len()],
    title: String,
    done: bool,
    accepted: bool,
    input_state: ModalInputState,
    noisy_tracker: widget_bridge::NoisyTracker,
    slider_events: Vec<UiEvent>,
    button_events: Vec<UiEvent>,
}

impl SoundsScreen {
    fn new(
        resources: &IngameMenuResources,
        config: &SoundConfig,
        input_state: ModalInputState,
        sound: Option<&SoundManager>,
    ) -> Self {
        let edit = crate::options_model::SoundEdit::new(*config);
        let dirty = false;

        // - The EAX/3D radio's label is `MT_STR_SOUND_EAX` when the
        //   backend supports EAX, otherwise `MT_STR_SOUND_3D`.
        // - The radio is enabled only when the backend can do 3D sound;
        //   on a 2D-only backend (today's kira) the user can't pick EAX.
        // When `sound` is `None` (main-menu options entry without a live
        // backend), default to "no 3D / no EAX" so a sound-disabled boot
        // still presents a coherent UI.
        let (can_3d, can_eax) = sound
            .as_ref()
            .map(|s| (s.can_3d_sound(), s.can_eax_sound()))
            .unwrap_or((false, false));
        let mode_label_id = if can_eax {
            MT_STR_SOUND_EAX
        } else {
            MT_STR_SOUND_3D
        };

        let (btn_w, btn_h) = resources.button_dimensions();
        let ok_label = resources.menu_text.get(MT_BTN_OK);
        let cancel_label = resources.menu_text.get(MT_BTN_CANCEL);
        let bottom_labels: &[(&str, bool)] = &[(&ok_label, true), (&cancel_label, true)];
        let bottom = align_bottom_right(bottom_labels, btn_w, btn_h);

        // ── Stereo / EAX radios at (30,70) ────────────────────────────
        let (field_w, field_h) = resources.input_field_dimensions();
        let mut mode_layout = vec![
            super::layout::MenuButton {
                label: resources.menu_text.get(MT_STR_SOUND_STEREO),
                enabled: true,
                x: 30,
                y: 70,
                w: field_w,
                h: field_h,
            },
            super::layout::MenuButton {
                label: resources.menu_text.get(mode_label_id),
                enabled: can_3d,
                x: 30,
                y: 0,
                w: field_w,
                h: field_h,
            },
        ];
        align_on_first_widget(&mut mode_layout, 2);

        // ── High/Low resolution radios at (30,170) ────────────────────
        let mut res_layout = vec![
            super::layout::MenuButton {
                label: resources.menu_text.get(MT_STR_SOUND_RES_HIGH),
                enabled: true,
                x: 30,
                y: 170,
                w: field_w,
                h: field_h,
            },
            super::layout::MenuButton {
                label: resources.menu_text.get(MT_STR_SOUND_RES_LOW),
                enabled: true,
                x: 30,
                y: 0,
                w: field_w,
                h: field_h,
            },
        ];
        align_on_first_widget(&mut res_layout, 2);

        // Build FrameWnd with radios, OK/Cancel, and five volume sliders.
        let mut frame = FrameWnd::interactive();

        for (i, mb) in mode_layout.iter().enumerate() {
            // Honour the per-button `enabled` flag so a 3D-incapable
            // backend renders the EAX radio greyed out.
            frame.add_widget_absolute(widget_bridge::make_button_enabled(
                ID_MODE_BASE + i as u32,
                &mb.label,
                mb.enabled,
                mb.x,
                mb.y,
                mb.w,
                mb.h,
            ));
        }
        for (i, mb) in res_layout.iter().enumerate() {
            frame.add_widget_absolute(widget_bridge::make_button(
                ID_RES_BASE + i as u32,
                &mb.label,
                mb.x,
                mb.y,
                mb.w,
                mb.h,
            ));
        }
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

        // ── Slider widgets ────────────────────────────────────────────
        // Same virtual rects the pre-widget version drew at; now they drive
        // hit-testing + drag state through `WidgetSlider`.
        let slider_rects: [MenuRect; SOUND_SLIDERS.len()] = std::array::from_fn(|index| MenuRect {
            x: 30,
            y: 290 + index as i32 * 40,
            w: 200,
            h: 16,
        });
        let slider_labels = SOUND_SLIDERS.map(|(_, label)| resources.menu_text.get(label));
        for (i, rect) in slider_rects.iter().enumerate() {
            let mut slider = WidgetSlider::new(ID_SLIDER_BASE + i as u32);
            slider.base.bbox = ScreenBBox::from_coords(
                rect.x as f32,
                rect.y as f32,
                (rect.x + rect.w) as f32,
                (rect.y + rect.h) as f32,
            );
            slider.set_range(0.0, SLIDER_MAX as f32);
            slider.set_step_count(SLIDER_STEPS);
            slider.set_value(slider_value(&edit.working, i) as f32);
            frame.add_widget_absolute(Widget::Slider(slider));
        }

        let title = resources.menu_text.get(MT_TTL_SOUNDS);

        let done = false;
        let accepted = false;
        // Per-widget noise-tracking state. Kept alive across frames so
        // repeat events in the same widget state stay silent; resets on
        // state change.
        let noisy_tracker = widget_bridge::NoisyTracker::new();
        let slider_events = Vec::new();
        let button_events = Vec::new();

        Self {
            edit,
            dirty,
            frame,
            slider_rects,
            slider_labels,
            title,
            done,
            accepted,
            input_state,
            noisy_tracker,
            slider_events,
            button_events,
        }
    }

    fn tick(
        &mut self,
        io: widget_bridge::ModalScreenIo<'_, '_>,
        sound: &mut Option<&mut SoundManager>,
        audio_backend: &mut Option<&mut dyn AudioBackend>,
        sample_loader: Option<&SampleLoader>,
    ) {
        let widget_bridge::ModalScreenIo {
            window: event_pump,
            renderer,
            resources,
            cursor,
        } = io;
        let (events, transform) = super::layout::poll_events_with_transform(event_pump, renderer);
        for event in events {
            self.input_state.update_from_event(&event, transform);
            match event {
                GameEvent::Quit => self.done = true,
                GameEvent::KeyDown {
                    keycode: Keycode::Return,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::KpEnter,
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

        let widget_input = self.input_state.as_widget_input();
        let events = self.frame.process_input(&widget_input);
        self.input_state.end_frame();

        // Apply slider value updates + slider activations to the edit.working
        // config.  Track events carry the new tick value via
        // `UiEventData::SliderPosition`; activate events (drag release)
        // only signal that dragging ended.
        for ev in &events {
            if !is_slider_id(ev.origin_widget_id) {
                continue;
            }
            let idx = (ev.origin_widget_id - ID_SLIDER_BASE) as usize;
            if matches!(ev.msg_type, UiMsg::WidgetSliderTrack)
                && let Some(Widget::Slider(s)) = self.frame.widget(ev.origin_widget_id)
            {
                let new_val = s.tick_index().min(SLIDER_MAX as u32) as u16;
                if new_val != slider_value(&self.edit.working, idx) {
                    store_slider_value(&mut self.edit.working, idx, new_val);
                    self.dirty = true;
                }
            }
        }

        // Dispatch menu sounds.  Buttons + sliders share the one event
        // stream — partition by widget ID so each `play_widget_noise`
        // call sees only its own events (otherwise the first-match
        // behaviour would cross-wire the two noisy banks).  Each
        // dispatch passes the current widget's `UiState` and the
        // shared `NoisyTracker` so the state-gate applies per-widget:
        // a sound plays at most once per (widget, state) pair.
        partition_widget_events(events, &mut self.slider_events, &mut self.button_events);

        // Observe buttons even on silent mouse-leave frames to rearm hover.
        if let (Some(snd), Some(loader)) = (sound.as_deref_mut(), sample_loader) {
            let backend: Option<&mut dyn AudioBackend> = audio_backend
                .as_mut()
                .map(|b| &mut **b as &mut dyn AudioBackend);
            widget_bridge::play_frame_widget_noise(
                &self.button_events,
                &self.frame,
                widget_bridge::WIDGET_NOISY_BUTTON,
                snd,
                backend,
                loader,
                &mut self.noisy_tracker,
            );
        }
        for e in &self.slider_events {
            let state = self
                .frame
                .widget(e.origin_widget_id)
                .map(|w| w.base().state)
                .unwrap_or(UiState::Default);
            let backend: Option<&mut dyn AudioBackend> = audio_backend
                .as_mut()
                .map(|b| &mut **b as &mut dyn AudioBackend);
            dispatch_noise(
                std::slice::from_ref(e),
                widget_bridge::WIDGET_NOISY_SLIDER,
                sound.as_deref_mut(),
                backend,
                sample_loader,
                Some(&mut self.noisy_tracker),
                state,
            );
        }

        // Button activations drive the radio / OK / Cancel state — but
        // only react to real buttons, not slider `WidgetActivated`
        // (drag release), which `find_activated` would otherwise
        // return first.
        if let Some(id) = self
            .button_events
            .iter()
            .find(|e| e.msg_type == UiMsg::WidgetActivated)
            .map(|e| e.origin_widget_id)
        {
            match id {
                ID_OK => {
                    self.accepted = true;
                    self.done = true;
                }
                ID_CANCEL => self.done = true,
                id if id == ID_MODE_BASE => {
                    self.edit.working.sound_3d = false;
                    self.dirty = true;
                }
                id if id == ID_MODE_BASE + 1 => {
                    self.edit.working.sound_3d = true;
                    self.dirty = true;
                }
                id if id == ID_RES_BASE => {
                    self.edit.working.sound_8bit = false;
                    self.dirty = true;
                }
                id if id == ID_RES_BASE + 1 => {
                    self.edit.working.sound_8bit = true;
                    self.dirty = true;
                }
                _ => {}
            }
        }

        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);

        if let Some(bg) = resources.menu_bg[0] {
            draw_screen_background(renderer, &bg);
        }

        if let Some(font) = resources.title_font_any() {
            let tw = font.text_width(&self.title);
            render_text_virt_font(renderer, font, transform, &self.title, (460 - tw) / 2, 20);
        }
        if let Some(font) = resources.label_font_any() {
            for (i, label) in self.slider_labels.iter().enumerate() {
                render_text_virt_font(
                    renderer,
                    font,
                    transform,
                    label,
                    self.slider_rects[i].x,
                    self.slider_rects[i].y - 20,
                );
            }
        }

        // Radio buttons with config-driven selected state.
        for i in 0..2u32 {
            if let Some(w) = self.frame.widget(ID_MODE_BASE + i) {
                let selected = (i == 0 && !self.edit.working.sound_3d)
                    || (i == 1 && self.edit.working.sound_3d);
                widget_bridge::draw_widget_radio(renderer, resources, transform, w, selected);
            }
        }
        for i in 0..2u32 {
            if let Some(w) = self.frame.widget(ID_RES_BASE + i) {
                let selected = (i == 0 && !self.edit.working.sound_8bit)
                    || (i == 1 && self.edit.working.sound_8bit);
                widget_bridge::draw_widget_radio(renderer, resources, transform, w, selected);
            }
        }

        // Sliders — reuse the existing `draw_slider` thumb/track
        // renderer, reading the live value off the widget (which may
        // be mid-drag, so `edit.working` lags until the widget publishes
        // a track event).
        for (i, rect) in self.slider_rects.iter().enumerate() {
            let value = self
                .frame
                .widget(ID_SLIDER_BASE + i as u32)
                .and_then(|w| match w {
                    Widget::Slider(s) => Some(s.tick_index().min(SLIDER_MAX as u32) as u16),
                    _ => None,
                })
                .unwrap_or_else(|| slider_value(&self.edit.working, i));
            draw_slider(renderer, resources, transform, rect, value, SLIDER_MAX);
        }

        // OK / Cancel as regular buttons.
        if let Some(w) = self.frame.widget(ID_OK) {
            widget_bridge::draw_widget_button(renderer, resources, transform, w, false);
        }
        if let Some(w) = self.frame.widget(ID_CANCEL) {
            widget_bridge::draw_widget_button(renderer, resources, transform, w, false);
        }

        if let Some(c) = &cursor {
            c.draw(renderer, transform, &self.input_state);
        }

        renderer.present();
    }

    fn finish(self, config: &mut SoundConfig) -> bool {
        // The `dirty` flag is set on every widget event — even a click on
        // the already-selected radio. Any accepted+dirty exit triggers
        // sound-settings re-apply in the caller, regardless of whether the
        // edit.working config differs field-for-field from the original.
        self.edit.commit(self.accepted && self.dirty, config)
    }
}

fn partition_widget_events(
    events: Vec<UiEvent>,
    sliders: &mut Vec<UiEvent>,
    buttons: &mut Vec<UiEvent>,
) {
    sliders.clear();
    buttons.clear();
    for event in events {
        if is_slider_id(event.origin_widget_id) {
            sliders.push(event);
        } else {
            buttons.push(event);
        }
    }
}

#[cfg(test)]
mod screen_state_tests {
    use super::*;

    #[test]
    fn sound_screen_finish_retains_dirty_click_reapply_and_cancel_policy() {
        for accepted in [false, true] {
            for dirty in [false, true] {
                let mut config = SoundConfig::default();
                let original = config;
                let mut edit = crate::options_model::SoundEdit::new(config);
                edit.working.sound_8bit = !config.sound_8bit;
                let expected = edit.working;
                let screen = SoundsScreen {
                    edit,
                    dirty,
                    frame: FrameWnd::interactive(),
                    slider_rects: [MenuRect {
                        x: 0,
                        y: 0,
                        w: 200,
                        h: 16,
                    }; SOUND_SLIDERS.len()],
                    slider_labels: std::array::from_fn(|_| String::new()),
                    title: String::new(),
                    done: true,
                    accepted,
                    input_state: ModalInputState::new(),
                    noisy_tracker: widget_bridge::NoisyTracker::new(),
                    slider_events: Vec::new(),
                    button_events: Vec::new(),
                };
                assert_eq!(screen.finish(&mut config), accepted && dirty);
                assert_eq!(
                    serde_json::to_value(config).unwrap(),
                    serde_json::to_value(if accepted && dirty {
                        expected
                    } else {
                        original
                    })
                    .unwrap()
                );
            }
        }
    }
}

fn is_slider_id(id: u32) -> bool {
    (ID_SLIDER_BASE..ID_SLIDER_BASE + SOUND_SLIDERS.len() as u32).contains(&id)
}

/// Forward to [`widget_bridge::play_widget_noise_tracked`] only when
/// the caller supplied a live `SoundManager` + `SampleLoader`.
/// Extracted so that each call inside the main loop fully releases
/// its borrow of the `sound` / `audio_backend` slots at the
/// `}`-boundary, which lets the borrow-checker accept multiple
/// back-to-back dispatches (buttons + sliders) within the same
/// iteration.
#[allow(clippy::too_many_arguments)]
fn dispatch_noise(
    events: &[UiEvent],
    noisy_id: u32,
    sound: Option<&mut SoundManager>,
    audio_backend: Option<&mut dyn AudioBackend>,
    sample_loader: Option<&SampleLoader>,
    tracker: Option<&mut widget_bridge::NoisyTracker>,
    current_state: crate::ui::UiState,
) {
    if let (Some(snd), Some(loader)) = (sound, sample_loader) {
        widget_bridge::play_widget_noise_tracked(
            events,
            noisy_id,
            snd,
            audio_backend,
            loader,
            tracker,
            current_state,
            false,
        );
    }
}

fn slider_value(config: &SoundConfig, idx: usize) -> u16 {
    match idx {
        0 => config.fx_volume,
        1 => config.dialogue_volume,
        2 => config.music_volume,
        3 => config.exclamation_volume,
        4 => config.amount_of_speaking,
        _ => panic!("invalid sound slider index {idx}"),
    }
}

fn store_slider_value(config: &mut SoundConfig, idx: usize, value: u16) {
    *crate::options_model::sound_value_mut(config, SOUND_SLIDERS[idx].0) = value.min(SLIDER_MAX);
}

#[test]
fn sound_slider_order_matches_numeric_settings_and_widget_ids() {
    let mut config = SoundConfig::default();
    for (index, (setting, _)) in SOUND_SLIDERS.iter().enumerate() {
        store_slider_value(&mut config, index, index as u16 + 1);
        assert_eq!(slider_value(&config, index), index as u16 + 1);
        assert_eq!(
            *crate::options_model::sound_value_mut(&mut config, *setting),
            index as u16 + 1
        );
        assert!(is_slider_id(ID_SLIDER_BASE + index as u32));
    }
    assert!(!is_slider_id(ID_SLIDER_BASE - 1));
    assert!(!is_slider_id(ID_SLIDER_BASE + SOUND_SLIDERS.len() as u32));
    for index in 0..SOUND_SLIDERS.len() {
        store_slider_value(&mut config, index, u16::MAX);
        assert_eq!(slider_value(&config, index), SLIDER_MAX);
    }
}

#[test]
#[should_panic(expected = "invalid sound slider index")]
fn invalid_sound_slider_index_is_not_a_zero_volume() {
    slider_value(&SoundConfig::default(), SOUND_SLIDERS.len());
}

#[test]
fn event_partition_reuses_buffers_and_moves_payloads_in_order() {
    let payload = String::from("owned event payload");
    let pointer = payload.as_ptr();
    let mut sliders = Vec::with_capacity(4);
    let mut buttons = Vec::with_capacity(4);
    let slider_pointer = sliders.as_ptr();
    let button_pointer = buttons.as_ptr();
    let mut events: Vec<_> = [ID_OK, ID_SLIDER_BASE + 1, ID_CANCEL, ID_SLIDER_BASE]
        .into_iter()
        .map(|id| UiEvent {
            msg_type: UiMsg::WidgetActivated,
            origin_widget_id: id,
            data: None,
        })
        .collect();
    events[1].data = Some(crate::ui::UiEventData::Text(payload));
    partition_widget_events(events, &mut sliders, &mut buttons);
    assert_eq!(
        sliders
            .iter()
            .map(|event| event.origin_widget_id)
            .collect::<Vec<_>>(),
        [ID_SLIDER_BASE + 1, ID_SLIDER_BASE]
    );
    assert_eq!(
        buttons
            .iter()
            .map(|event| event.origin_widget_id)
            .collect::<Vec<_>>(),
        [ID_OK, ID_CANCEL]
    );
    let Some(crate::ui::UiEventData::Text(payload)) = &sliders[0].data else {
        panic!("partition must preserve the payload");
    };
    assert_eq!(payload.as_ptr(), pointer);
    partition_widget_events(Vec::new(), &mut sliders, &mut buttons);
    assert!(sliders.is_empty() && buttons.is_empty());
    assert_eq!(sliders.as_ptr(), slider_pointer);
    assert_eq!(buttons.as_ptr(), button_pointer);
}
