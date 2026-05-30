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

use crate::gfx_types::GameEvent;
use crate::renderer::Renderer;
use crate::sound::{AudioBackend, SoundManager};
use crate::sound_config::SoundConfig;
use crate::ui::{UiEvent, UiMsg};
use crate::widget::{Widget, WidgetSlider};
use robin_engine::coordinates::ScreenBBox;
use robin_engine::sound_cache::SampleLoader;

use super::layout::{
    MenuRect, MenuTransform, align_bottom_right, align_on_first_widget, dim_screen,
    draw_screen_background, draw_slider, enter_modal_gpu_phase, render_text_virt,
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
    let sw = renderer.screen_width() as i32;
    let sh = renderer.screen_height() as i32;
    let transform = MenuTransform::centered(sw, sh);

    let mut working = *config;
    let mut dirty = false;

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
    let mut frame = crate::widget::FrameWnd::default();
    frame.enabled = true;
    frame.input_enabled = true;

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
    let slider_w = 200i32;
    let slider_h = 16i32;
    let slider_rects = [
        MenuRect {
            x: 30,
            y: 290,
            w: slider_w,
            h: slider_h,
        },
        MenuRect {
            x: 30,
            y: 330,
            w: slider_w,
            h: slider_h,
        },
        MenuRect {
            x: 30,
            y: 370,
            w: slider_w,
            h: slider_h,
        },
        MenuRect {
            x: 30,
            y: 410,
            w: slider_w,
            h: slider_h,
        },
        MenuRect {
            x: 30,
            y: 450,
            w: slider_w,
            h: slider_h,
        },
    ];
    let slider_labels = [
        resources.menu_text.get(MT_STR_SOUND_VOL_FX),
        resources.menu_text.get(MT_STR_SOUND_VOL_DIALOGUE),
        resources.menu_text.get(MT_STR_SOUND_VOL_MUSIC),
        resources.menu_text.get(MT_STR_SOUND_VOL_COMMENT),
        resources.menu_text.get(MT_STR_SOUND_COMMENT_FREQUENCY),
    ];
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
        slider.set_value(slider_value(&working, i) as f32);
        frame.add_widget_absolute(Widget::Slider(slider));
    }

    let title = resources.menu_text.get(MT_TTL_SOUNDS);

    let mut done = false;
    let mut accepted = false;
    let mut input_state = ModalInputState::new();
    input_state.seed_mouse_from_sdl(event_pump, transform);
    // Per-widget noise-tracking state. Kept alive across frames so
    // repeat events in the same widget state stay silent; resets on
    // state change.
    let mut noisy_tracker = widget_bridge::NoisyTracker::new();

    while !done {
        for event in event_pump.poll_events() {
            input_state.update_from_event(&event, transform);
            match event {
                GameEvent::Quit => done = true,
                GameEvent::KeyDown {
                    keycode: Keycode::Return,
                    ..
                }
                | GameEvent::KeyDown {
                    keycode: Keycode::KpEnter,
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

        let widget_input = input_state.as_widget_input();
        let events = frame.process_input(&widget_input);
        input_state.end_frame();

        // Apply slider value updates + slider activations to the working
        // config.  Track events carry the new tick value via
        // `UiEventData::SliderPosition`; activate events (drag release)
        // only signal that dragging ended.
        for ev in &events {
            if !is_slider_id(ev.origin_widget_id) {
                continue;
            }
            let idx = (ev.origin_widget_id - ID_SLIDER_BASE) as usize;
            if matches!(ev.msg_type, UiMsg::WidgetSliderTrack)
                && let Some(Widget::Slider(s)) = frame.widget(ev.origin_widget_id)
            {
                let new_val = s.tick_index().min(SLIDER_MAX as u32) as u16;
                if new_val != slider_value(&working, idx) {
                    store_slider_value(&mut working, idx, new_val);
                    dirty = true;
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
        let (slider_events, button_events): (Vec<_>, Vec<_>) = events
            .iter()
            .cloned()
            .partition(|e| is_slider_id(e.origin_widget_id));

        // Buttons: use the widget's own state for gating.
        for e in &button_events {
            let state = frame
                .widget(e.origin_widget_id)
                .map(|w| w.base().state)
                .unwrap_or(crate::ui::UiState::Default);
            let backend: Option<&mut dyn AudioBackend> = audio_backend
                .as_mut()
                .map(|b| &mut **b as &mut dyn AudioBackend);
            dispatch_noise(
                std::slice::from_ref(e),
                widget_bridge::WIDGET_NOISY_BUTTON,
                sound.as_deref_mut(),
                backend,
                sample_loader,
                Some(&mut noisy_tracker),
                state,
            );
        }
        for e in &slider_events {
            let state = frame
                .widget(e.origin_widget_id)
                .map(|w| w.base().state)
                .unwrap_or(crate::ui::UiState::Default);
            let backend: Option<&mut dyn AudioBackend> = audio_backend
                .as_mut()
                .map(|b| &mut **b as &mut dyn AudioBackend);
            dispatch_noise(
                std::slice::from_ref(e),
                widget_bridge::WIDGET_NOISY_SLIDER,
                sound.as_deref_mut(),
                backend,
                sample_loader,
                Some(&mut noisy_tracker),
                state,
            );
        }

        // Button activations drive the radio / OK / Cancel state — but
        // only react to real buttons, not slider `WidgetActivated`
        // (drag release), which `find_activated` would otherwise
        // return first.
        if let Some(id) = button_events
            .iter()
            .find(|e| e.msg_type == UiMsg::WidgetActivated)
            .map(|e| e.origin_widget_id)
        {
            match id {
                ID_OK => {
                    accepted = true;
                    done = true;
                }
                ID_CANCEL => done = true,
                id if id == ID_MODE_BASE => {
                    working.sound_3d = false;
                    dirty = true;
                }
                id if id == ID_MODE_BASE + 1 => {
                    working.sound_3d = true;
                    dirty = true;
                }
                id if id == ID_RES_BASE => {
                    working.sound_8bit = false;
                    dirty = true;
                }
                id if id == ID_RES_BASE + 1 => {
                    working.sound_8bit = true;
                    dirty = true;
                }
                _ => {}
            }
        }

        enter_modal_gpu_phase(renderer);
        dim_screen(renderer);

        if let Some(bg) = resources.menu_bg[0] {
            draw_screen_background(renderer, &bg);
        }

        if let Some(font) = resources.title_font() {
            let tw = font.text_width(&title);
            render_text_virt(renderer, font, transform, &title, (460 - tw) / 2, 20);
        }
        if let Some(font) = resources.label_font() {
            for (i, label) in slider_labels.iter().enumerate() {
                render_text_virt(
                    renderer,
                    font,
                    transform,
                    label,
                    slider_rects[i].x,
                    slider_rects[i].y - 20,
                );
            }
        }

        // Radio buttons with config-driven selected state.
        for i in 0..2u32 {
            if let Some(w) = frame.widget(ID_MODE_BASE + i) {
                let selected = (i == 0 && !working.sound_3d) || (i == 1 && working.sound_3d);
                widget_bridge::draw_widget_radio(renderer, resources, transform, w, selected);
            }
        }
        for i in 0..2u32 {
            if let Some(w) = frame.widget(ID_RES_BASE + i) {
                let selected = (i == 0 && !working.sound_8bit) || (i == 1 && working.sound_8bit);
                widget_bridge::draw_widget_radio(renderer, resources, transform, w, selected);
            }
        }

        // Sliders — reuse the existing `draw_slider` thumb/track
        // renderer, reading the live value off the widget (which may
        // be mid-drag, so `working` lags until the widget publishes
        // a track event).
        for (i, rect) in slider_rects.iter().enumerate() {
            let value = frame
                .widget(ID_SLIDER_BASE + i as u32)
                .and_then(|w| match w {
                    Widget::Slider(s) => Some(s.tick_index().min(SLIDER_MAX as u32) as u16),
                    _ => None,
                })
                .unwrap_or_else(|| slider_value(&working, i));
            draw_slider(renderer, resources, transform, rect, value, SLIDER_MAX);
        }

        // OK / Cancel as regular buttons.
        if let Some(w) = frame.widget(ID_OK) {
            widget_bridge::draw_widget_button(renderer, resources, transform, w, false);
        }
        if let Some(w) = frame.widget(ID_CANCEL) {
            widget_bridge::draw_widget_button(renderer, resources, transform, w, false);
        }

        if let Some(c) = &cursor {
            c.draw(renderer, transform, &input_state);
        }

        renderer.present();
        crate::window::sleep_ms(16).await;
    }

    // The `dirty` flag is set on every widget event — even a click on
    // the already-selected radio. Any accepted+dirty exit triggers
    // sound-settings re-apply in the caller, regardless of whether the
    // working config differs field-for-field from the original.
    if accepted && dirty {
        *config = working;
        true
    } else {
        false
    }
}

fn is_slider_id(id: u32) -> bool {
    (ID_SLIDER_BASE..ID_SLIDER_BASE + 5).contains(&id)
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
        _ => 0,
    }
}

fn store_slider_value(config: &mut SoundConfig, idx: usize, value: u16) {
    let clamped = value.min(SLIDER_MAX);
    match idx {
        0 => config.fx_volume = clamped,
        1 => config.dialogue_volume = clamped,
        2 => config.music_volume = clamped,
        3 => config.exclamation_volume = clamped,
        4 => config.amount_of_speaking = clamped,
        _ => {}
    }
}
