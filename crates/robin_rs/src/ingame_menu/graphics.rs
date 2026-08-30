//! Graphics settings sub-screen — radio buttons for resolution and
//! visual toggles, plus OK / Cancel.
//!
//! Radio buttons and OK/Cancel are driven by the [`crate::widget`] system
//! via the [`super::widget_bridge`].

use crate::gfx_types::Keycode;
use robin_engine::graphic_config::{TextureEffect, TextureScaleMode};
use robin_engine::sprite as engine_sprite;

use crate::gfx_types::GameEvent;
use crate::renderer::Renderer;
use crate::widget::FrameWnd;
use robin_engine::graphic_config::GraphicConfig;

use super::layout::{
    MenuTransform, align_bottom_right, align_on_first_widget, dim_screen, draw_fallback_rect,
    draw_screen_background, enter_modal_gpu_phase, render_text_virt,
};
use super::resources::{
    IngameMenuResources, MT_BTN_CANCEL, MT_BTN_OK, MT_STR_ALPHA_VISION_FIELD,
    MT_STR_BCKGND_ANIMATIONS, MT_STR_EFFECT_ANIMATIONS, MT_STR_RES, MT_STR_RES_HIGH,
    MT_STR_RES_LOW, MT_STR_RES_MEDIUM, MT_STR_SPECIAL_FX, MT_STR_TRANSPARENT_SHADOWS,
    MT_TTL_GRAPHICS,
};
use super::widget_bridge::{self, ModalCursor, ModalInputState};

// Widget ID ranges: resolution 100..102, widescreen 150, options 200..204,
// scaling 400.., ok/cancel 300..301.
const ID_RES_BASE: u32 = 100;
const ID_ADAPTIVE_WIDESCREEN: u32 = 150;
const ID_OPT_BASE: u32 = 200;
const ID_OK: u32 = 300;
const ID_CANCEL: u32 = 301;
const ID_SCALE_BASE: u32 = 400;
const ID_EFFECT_BASE: u32 = 500;
const PRESET_LIST_X: i32 = 360;
const PRESET_LIST_Y: i32 = 350;
const PRESET_LIST_W: i32 = 240;
const PRESET_LIST_ROW_H: i32 = 15;
const PRESET_LIST_ROWS: usize = 4;

/// Labels for the scaling radio column.
fn scale_modes() -> Vec<TextureScaleMode> {
    TextureScaleMode::ALL
        .iter()
        .copied()
        .filter(|mode| {
            *mode != TextureScaleMode::RetroArch
                || crate::shader_preset::retroarch_runtime_available()
        })
        .collect()
}

/// Display the graphics sub-screen.  Returns `(options_changed, resolution_changed)`.
pub async fn show_graphics(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    cursor: Option<ModalCursor<'_>>,
    config: &mut GraphicConfig,
) -> (bool, bool) {
    let sw = renderer.screen_width() as i32;
    let sh = renderer.screen_height() as i32;
    let transform = MenuTransform::centered(sw, sh);

    let original = config.clone();
    let mut working = config.clone();
    let mut dirty = false;
    let retroarch_presets = crate::shader_preset::retroarch_presets();
    if working.shader_preset.is_empty()
        && let Some(preset) = retroarch_presets.first()
    {
        working.shader_preset = preset.id.clone();
    }
    let mut preset_scroll = preset_index(retroarch_presets, &working.shader_preset)
        .unwrap_or(0)
        .saturating_sub(PRESET_LIST_ROWS / 2);

    // ── OK / Cancel (bottom-right) ─────────────────────────────────
    let (btn_w, btn_h) = resources.button_dimensions();
    let ok_label = resources.menu_text.get(MT_BTN_OK);
    let cancel_label = resources.menu_text.get(MT_BTN_CANCEL);
    let bottom_labels: &[(&str, bool)] = &[(&ok_label, true), (&cancel_label, true)];
    let bottom = align_bottom_right(bottom_labels, btn_w, btn_h);

    // ── Resolution radio buttons (3 stacked from (30,100)) ────────
    let (field_w, field_h) = resources.input_field_dimensions();
    let res_low_label = resources.menu_text.get(MT_STR_RES_LOW);
    let res_med_label = resources.menu_text.get(MT_STR_RES_MEDIUM);
    let res_high_label = resources.menu_text.get(MT_STR_RES_HIGH);
    let mut res_layout = vec![
        super::layout::MenuButton {
            label: res_low_label,
            enabled: true,
            x: 30,
            y: 100,
            w: field_w,
            h: field_h,
        },
        super::layout::MenuButton {
            label: res_med_label,
            enabled: true,
            x: 30,
            y: 0,
            w: field_w,
            h: field_h,
        },
        super::layout::MenuButton {
            label: res_high_label,
            enabled: true,
            x: 30,
            y: 0,
            w: field_w,
            h: field_h,
        },
    ];
    align_on_first_widget(&mut res_layout, 2);
    let widescreen_button = super::layout::MenuButton {
        // Rust extension; shipped string tables predate widescreen support.
        label: "Adaptive Widescreen".to_string(),
        enabled: true,
        x: 30,
        y: 205,
        w: field_w,
        h: field_h,
    };

    // ── Option toggle buttons stacked below widescreen policy ─────
    let mut opt_layout = vec![
        super::layout::MenuButton {
            label: resources.menu_text.get(MT_STR_ALPHA_VISION_FIELD),
            enabled: true,
            x: 30,
            y: 270,
            w: field_w,
            h: field_h,
        },
        super::layout::MenuButton {
            label: resources.menu_text.get(MT_STR_TRANSPARENT_SHADOWS),
            enabled: true,
            x: 30,
            y: 0,
            w: field_w,
            h: field_h,
        },
        super::layout::MenuButton {
            label: resources.menu_text.get(MT_STR_EFFECT_ANIMATIONS),
            enabled: true,
            x: 30,
            y: 0,
            w: field_w,
            h: field_h,
        },
        super::layout::MenuButton {
            label: resources.menu_text.get(MT_STR_BCKGND_ANIMATIONS),
            enabled: true,
            x: 30,
            y: 0,
            w: field_w,
            h: field_h,
        },
        super::layout::MenuButton {
            // Rust extension; the original string table has no label for it.
            label: "Fog/Night All Sprites".to_string(),
            enabled: true,
            x: 30,
            y: 0,
            w: field_w,
            h: field_h,
        },
    ];
    align_on_first_widget(&mut opt_layout, 2);

    // ── Scaling radios (right column, stacked from (360, 100)) ─────
    // One row per supported [`TextureScaleMode`]. RetroArch is omitted when
    // this build cannot execute presets; selected shaders fail explicitly
    // instead of silently changing the requested presentation mode.
    // We use the OK/Cancel button width (not the full input-field
    // width) so the rows fit inside the menu frame without clipping
    // the right edge, and each row is shorter than a resolution button
    // so the column fits within the vertical space that's left.
    let scale_modes = scale_modes();
    let scale_btn_w = btn_w;
    let scale_btn_h = 13;
    let scale_row_spacing = 0;
    let scale_x = super::layout::MENU_W - 40 - scale_btn_w;
    let mut scale_layout: Vec<super::layout::MenuButton> = scale_modes
        .iter()
        .enumerate()
        .map(|(i, mode)| super::layout::MenuButton {
            label: mode.label().to_string(),
            enabled: true,
            x: scale_x,
            y: if i == 0 { 94 } else { 0 },
            w: scale_btn_w,
            h: scale_btn_h,
        })
        .collect();
    align_on_first_widget(&mut scale_layout, scale_row_spacing);

    let effect_y = 94 + scale_modes.len() as i32 * scale_btn_h + 8;
    let mut effect_layout: Vec<super::layout::MenuButton> = TextureEffect::ALL
        .iter()
        .map(|effect| super::layout::MenuButton {
            label: effect.label().to_string(),
            enabled: true,
            x: scale_x,
            y: if *effect == TextureEffect::None {
                effect_y
            } else {
                0
            },
            w: scale_btn_w,
            h: scale_btn_h,
        })
        .collect();
    align_on_first_widget(&mut effect_layout, 0);

    // Build the FrameWnd with all widgets.
    let mut frame = FrameWnd::default();
    frame.enabled = true;
    frame.input_enabled = true;

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
        ID_ADAPTIVE_WIDESCREEN,
        &widescreen_button.label,
        widescreen_button.x,
        widescreen_button.y,
        widescreen_button.w,
        widescreen_button.h,
    ));
    for (i, mb) in opt_layout.iter().enumerate() {
        frame.add_widget_absolute(widget_bridge::make_button(
            ID_OPT_BASE + i as u32,
            &mb.label,
            mb.x,
            mb.y,
            mb.w,
            mb.h,
        ));
    }
    for (i, mb) in scale_layout.iter().enumerate() {
        frame.add_widget_absolute(widget_bridge::make_button(
            ID_SCALE_BASE + i as u32,
            &mb.label,
            mb.x,
            mb.y,
            mb.w,
            mb.h,
        ));
    }
    for (i, mb) in effect_layout.iter().enumerate() {
        frame.add_widget_absolute(widget_bridge::make_button(
            ID_EFFECT_BASE + i as u32,
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

    let title = resources.menu_text.get(MT_TTL_GRAPHICS);
    let res_label = resources.menu_text.get(MT_STR_RES);
    let fx_label = resources.menu_text.get(MT_STR_SPECIAL_FX);

    let mut done = false;
    let mut accepted = false;
    let mut parameter_page_effect = false;
    let mut parameter_status = String::new();
    let parameter_y = effect_y + TextureEffect::ALL.len() as i32 * scale_btn_h + 14;
    let mut input_state = ModalInputState::new();
    input_state.seed_mouse_from_window(event_pump, transform);

    while !done {
        let (events, transform) = super::layout::poll_events_with_transform(event_pump, renderer);
        for event in events {
            input_state.update_from_event(&event, transform);
            match event {
                GameEvent::Quit => done = true,
                GameEvent::MouseDown(x, y, 1, _) => {
                    let (vx, vy) = transform.from_screen(x, y);
                    let row_count = if parameter_page_effect { 5 } else { 3 };
                    if (working.scale_mode != TextureScaleMode::RetroArch || parameter_page_effect)
                        && (scale_x..scale_x + scale_btn_w).contains(&vx)
                        && (parameter_y..parameter_y + row_count * 12).contains(&vy)
                    {
                        let row = ((vy - parameter_y) / 12) as usize;
                        adjust_parameter(
                            &mut working,
                            parameter_page_effect,
                            row,
                            vx >= scale_x + scale_btn_w / 2,
                        );
                        dirty = true;
                    } else if working.scale_mode == TextureScaleMode::RetroArch
                        && !parameter_page_effect
                        && (PRESET_LIST_X..PRESET_LIST_X + PRESET_LIST_W).contains(&vx)
                        && (PRESET_LIST_Y
                            ..PRESET_LIST_Y + PRESET_LIST_ROW_H * PRESET_LIST_ROWS as i32)
                            .contains(&vy)
                    {
                        let row = ((vy - PRESET_LIST_Y) / PRESET_LIST_ROW_H) as usize;
                        let index = preset_scroll + row;
                        if let Some(preset) = retroarch_presets.get(index) {
                            working.shader_preset = preset.id.clone();
                            parameter_status.clear();
                            dirty = true;
                        }
                    }
                }
                GameEvent::MouseWheel(delta)
                    if working.scale_mode == TextureScaleMode::RetroArch
                        && !parameter_page_effect =>
                {
                    if delta > 0 {
                        preset_scroll = preset_scroll.saturating_sub(delta as usize);
                    } else if delta < 0 {
                        preset_scroll = (preset_scroll + (-delta) as usize)
                            .min(retroarch_presets.len().saturating_sub(PRESET_LIST_ROWS));
                    }
                }
                GameEvent::KeyDown {
                    keycode: Keycode::Tab,
                    ..
                } => parameter_page_effect = !parameter_page_effect,
                GameEvent::KeyDown {
                    keycode: Keycode::Char(b'i'),
                    ..
                } if working.scale_mode == TextureScaleMode::RetroArch => {
                    match pick_retroarch_preset().await {
                        Ok(Some(path)) => {
                            let selected = path.to_string_lossy().to_string();
                            match renderer.validate_retroarch_preset(&selected) {
                                Ok(()) => {
                                    working.shader_preset = selected;
                                    parameter_status = "Imported preset validated".to_string();
                                    dirty = true;
                                }
                                Err(error) => parameter_status = format!("Import failed: {error}"),
                            }
                        }
                        Ok(None) => {}
                        Err(error) => parameter_status = error,
                    }
                }
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
                GameEvent::KeyDown { keycode, .. }
                    if working.scale_mode == TextureScaleMode::RetroArch
                        && !parameter_page_effect =>
                {
                    let current = preset_index(retroarch_presets, &working.shader_preset)
                        .unwrap_or(preset_scroll);
                    let next = match keycode {
                        Keycode::Up => current.saturating_sub(1),
                        Keycode::Down => {
                            (current + 1).min(retroarch_presets.len().saturating_sub(1))
                        }
                        Keycode::PageUp => current.saturating_sub(PRESET_LIST_ROWS),
                        Keycode::PageDown => (current + PRESET_LIST_ROWS)
                            .min(retroarch_presets.len().saturating_sub(1)),
                        Keycode::Home => 0,
                        Keycode::End => retroarch_presets.len().saturating_sub(1),
                        _ => current,
                    };
                    if next != current
                        && let Some(preset) = retroarch_presets.get(next)
                    {
                        working.shader_preset = preset.id.clone();
                        preset_scroll = keep_visible(next, preset_scroll, retroarch_presets.len());
                        dirty = true;
                    }
                }
                _ => {}
            }
        }

        let widget_input = input_state.as_widget_input();
        let events = frame.process_input(&widget_input);
        input_state.end_frame();

        if let Some(id) = widget_bridge::find_activated(&events) {
            match id {
                ID_OK => {
                    accepted = true;
                    done = true;
                }
                ID_CANCEL => done = true,
                id if (ID_RES_BASE..ID_RES_BASE + 3).contains(&id) => {
                    apply_resolution(&mut working, (id - ID_RES_BASE) as usize);
                    dirty = true;
                }
                ID_ADAPTIVE_WIDESCREEN => {
                    working.adaptive_widescreen = !working.adaptive_widescreen;
                    dirty = true;
                }
                id if (ID_OPT_BASE..ID_OPT_BASE + 5).contains(&id) => {
                    apply_option_toggle(&mut working, (id - ID_OPT_BASE) as usize);
                    dirty = true;
                }
                id if (ID_SCALE_BASE..ID_SCALE_BASE + scale_modes.len() as u32).contains(&id) => {
                    working.scale_mode = scale_modes[(id - ID_SCALE_BASE) as usize];
                    if working.scale_mode == TextureScaleMode::RetroArch {
                        let index =
                            preset_index(retroarch_presets, &working.shader_preset).unwrap_or(0);
                        preset_scroll = keep_visible(index, preset_scroll, retroarch_presets.len());
                    }
                    dirty = true;
                }
                id if (ID_EFFECT_BASE..ID_EFFECT_BASE + TextureEffect::ALL.len() as u32)
                    .contains(&id) =>
                {
                    working.texture_effect = TextureEffect::ALL[(id - ID_EFFECT_BASE) as usize];
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
            render_text_virt(renderer, font, transform, &title, (490 - tw) / 2, 20);
        }
        if let Some(font) = resources.label_font() {
            render_text_virt(renderer, font, transform, &res_label, 30, 80);
            render_text_virt(renderer, font, transform, &fx_label, 30, 250);
            render_text_virt(renderer, font, transform, "Scaling", scale_x, 80);
            render_text_virt(
                renderer,
                font,
                transform,
                "Texture effect",
                scale_x,
                effect_y - 12,
            );
            if working.scale_mode == TextureScaleMode::RetroArch && !parameter_page_effect {
                render_text_virt(
                    renderer,
                    font,
                    transform,
                    "Preset",
                    PRESET_LIST_X,
                    PRESET_LIST_Y - 18,
                );
                render_text_virt(
                    renderer,
                    font,
                    transform,
                    "Press I to import a native .slangp preset",
                    PRESET_LIST_X,
                    PRESET_LIST_Y + PRESET_LIST_ROW_H * PRESET_LIST_ROWS as i32 + 2,
                );
            } else {
                let page = if parameter_page_effect {
                    "Effect parameters (Tab)"
                } else {
                    "Upscaler parameters (Tab)"
                };
                render_text_virt(renderer, font, transform, page, scale_x, parameter_y - 12);
            }
            if !parameter_status.is_empty() {
                render_text_virt(renderer, font, transform, &parameter_status, 30, 410);
            }
        }

        // Render radio buttons with config-driven selected state.
        for i in 0..3u32 {
            if let Some(w) = frame.widget(ID_RES_BASE + i) {
                widget_bridge::draw_widget_radio(
                    renderer,
                    resources,
                    transform,
                    w,
                    is_resolution_selected(&working, i as usize),
                );
            }
        }
        if let Some(w) = frame.widget(ID_ADAPTIVE_WIDESCREEN) {
            widget_bridge::draw_widget_radio(
                renderer,
                resources,
                transform,
                w,
                working.adaptive_widescreen,
            );
        }
        for i in 0..5u32 {
            if let Some(w) = frame.widget(ID_OPT_BASE + i) {
                widget_bridge::draw_widget_radio(
                    renderer,
                    resources,
                    transform,
                    w,
                    is_option_selected(&working, i as usize),
                );
            }
        }
        for (i, mode) in scale_modes.iter().enumerate() {
            if let Some(w) = frame.widget(ID_SCALE_BASE + i as u32) {
                widget_bridge::draw_widget_radio(
                    renderer,
                    resources,
                    transform,
                    w,
                    working.scale_mode == *mode,
                );
            }
        }
        for (i, effect) in TextureEffect::ALL.iter().enumerate() {
            if let Some(w) = frame.widget(ID_EFFECT_BASE + i as u32) {
                widget_bridge::draw_widget_radio(
                    renderer,
                    resources,
                    transform,
                    w,
                    working.texture_effect == *effect,
                );
            }
        }

        if working.scale_mode == TextureScaleMode::RetroArch && !parameter_page_effect {
            draw_preset_list(
                renderer,
                resources,
                transform,
                retroarch_presets,
                preset_scroll,
                &working.shader_preset,
            );
        } else {
            draw_parameter_panel(
                renderer,
                resources,
                transform,
                &working,
                parameter_page_effect,
                scale_x,
                parameter_y,
                scale_btn_w,
            );
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

    if accepted && dirty {
        *config = working;
        let resolution_changed = (config.resolution_x - original.resolution_x).abs() > 0.5
            || (config.resolution_y - original.resolution_y).abs() > 0.5
            || config.adaptive_widescreen != original.adaptive_widescreen;
        renderer.apply_upscale_config(config);
        (true, resolution_changed)
    } else {
        (false, false)
    }
}

fn preset_index(
    presets: &[crate::shader_preset::RetroArchPresetInfo],
    selected: &str,
) -> Option<usize> {
    presets.iter().position(|preset| preset.id == selected)
}

fn keep_visible(index: usize, scroll: usize, total: usize) -> usize {
    let max_scroll = total.saturating_sub(PRESET_LIST_ROWS);
    if index < scroll {
        index
    } else if index >= scroll + PRESET_LIST_ROWS {
        index.saturating_sub(PRESET_LIST_ROWS - 1).min(max_scroll)
    } else {
        scroll.min(max_scroll)
    }
}

fn draw_preset_list(
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    transform: MenuTransform,
    presets: &[crate::shader_preset::RetroArchPresetInfo],
    scroll: usize,
    selected: &str,
) {
    let (sx, sy) = transform.to_screen(PRESET_LIST_X, PRESET_LIST_Y);
    draw_fallback_rect(
        renderer,
        sx,
        sy,
        PRESET_LIST_W,
        PRESET_LIST_ROW_H * PRESET_LIST_ROWS as i32,
        false,
    );

    let Some(font) = resources.label_font() else {
        return;
    };
    for row in 0..PRESET_LIST_ROWS {
        let Some(preset) = presets.get(scroll + row) else {
            break;
        };
        let y = PRESET_LIST_Y + row as i32 * PRESET_LIST_ROW_H;
        let is_selected = preset.id == selected;
        if is_selected {
            let (rx, ry) = transform.to_screen(PRESET_LIST_X + 1, y + 1);
            renderer.fill_screen(
                Some(&engine_sprite::BBox::from_coords(
                    rx as f32,
                    ry as f32,
                    (rx + PRESET_LIST_W - 2) as f32,
                    (ry + PRESET_LIST_ROW_H - 1) as f32,
                )),
                Renderer::create_color_16(80, 60, 35),
            );
        }
        let label = fit_label(font, &preset.label, PRESET_LIST_W - 8);
        render_text_virt(renderer, font, transform, &label, PRESET_LIST_X + 4, y + 1);
    }
}

fn fit_label(font: &crate::native_font::NativeFont, label: &str, max_w: i32) -> String {
    if font.text_width(label) <= max_w {
        return label.to_string();
    }
    let mut out = label.to_string();
    while !out.is_empty() && font.text_width(&format!("{out}...")) > max_w {
        out.pop();
    }
    format!("{out}...")
}

fn adjust_parameter(config: &mut GraphicConfig, effect_page: bool, index: usize, increase: bool) {
    let value = if effect_page {
        match index {
            0 => &mut config.texture_effect_parameters.scanlines,
            1 => &mut config.texture_effect_parameters.phosphor_mask,
            2 => &mut config.texture_effect_parameters.bloom,
            3 => &mut config.texture_effect_parameters.curvature,
            4 => &mut config.texture_effect_parameters.temporal_flicker,
            _ => return,
        }
    } else {
        match index {
            0 => &mut config.upscale_parameters.strength,
            1 => &mut config.upscale_parameters.edge_threshold,
            2 => &mut config.upscale_parameters.artifact_removal,
            _ => return,
        }
    };
    *value = if increase {
        value.saturating_add(5).min(100)
    } else {
        value.saturating_sub(5)
    };
}

#[allow(clippy::too_many_arguments)]
fn draw_parameter_panel(
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    transform: MenuTransform,
    config: &GraphicConfig,
    effect_page: bool,
    x: i32,
    y: i32,
    width: i32,
) {
    let Some(font) = resources.label_font() else {
        return;
    };
    let upscale_values = [
        ("Strength", config.upscale_parameters.strength),
        ("Edge threshold", config.upscale_parameters.edge_threshold),
        (
            "Artifact removal",
            config.upscale_parameters.artifact_removal,
        ),
    ];
    let effect_values = [
        ("Scanlines", config.texture_effect_parameters.scanlines),
        (
            "Phosphor mask",
            config.texture_effect_parameters.phosphor_mask,
        ),
        ("Bloom", config.texture_effect_parameters.bloom),
        ("Curvature", config.texture_effect_parameters.curvature),
        (
            "Temporal flicker",
            config.texture_effect_parameters.temporal_flicker,
        ),
    ];
    let values: &[(&str, u8)] = if effect_page {
        &effect_values
    } else {
        &upscale_values
    };
    for (row, (label, value)) in values.iter().enumerate() {
        let row_y = y + row as i32 * 12;
        render_text_virt(
            renderer,
            font,
            transform,
            &format!("{label}: {value:3}%"),
            x,
            row_y,
        );
        let bar_x = x + width - 72;
        let (screen_x, screen_y) = transform.to_screen(bar_x, row_y + 2);
        let filled = (68 * i32::from(*value)) / 100;
        renderer.fill_screen(
            Some(&engine_sprite::BBox::from_coords(
                screen_x as f32,
                screen_y as f32,
                (screen_x + 68) as f32,
                (screen_y + 7) as f32,
            )),
            Renderer::create_color_16(25, 20, 16),
        );
        if filled > 0 {
            renderer.fill_screen(
                Some(&engine_sprite::BBox::from_coords(
                    screen_x as f32,
                    screen_y as f32,
                    (screen_x + filled) as f32,
                    (screen_y + 7) as f32,
                )),
                Renderer::create_color_16(175, 125, 55),
            );
        }
    }
}

#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
async fn pick_retroarch_preset() -> Result<Option<std::path::PathBuf>, String> {
    let Some(file) = rfd::AsyncFileDialog::new()
        .add_filter("RetroArch shader preset", &["slangp"])
        .set_title("Import RetroArch shader preset")
        .pick_file()
        .await
    else {
        return Ok(None);
    };
    let path = file.path();
    if path.extension().and_then(|extension| extension.to_str()) != Some("slangp") {
        return Err(format!("{} is not a .slangp preset", path.display()));
    }
    std::fs::canonicalize(path)
        .map(Some)
        .map_err(|error| format!("cannot read imported preset {}: {error}", path.display()))
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
async fn pick_retroarch_preset() -> Result<Option<std::path::PathBuf>, String> {
    Err("RetroArch preset import is only available in native desktop builds".to_string())
}

fn apply_resolution(config: &mut GraphicConfig, idx: usize) {
    match idx {
        0 => config.set_resolution(640.0, 480.0),
        1 => config.set_resolution(800.0, 600.0),
        2 => config.set_resolution(1024.0, 768.0),
        _ => {}
    }
}

fn is_resolution_selected(config: &GraphicConfig, idx: usize) -> bool {
    let (want_x, want_y) = match idx {
        0 => (640.0, 480.0),
        1 => (800.0, 600.0),
        2 => (1024.0, 768.0),
        _ => return false,
    };
    (config.resolution_x - want_x).abs() < 0.5 && (config.resolution_y - want_y).abs() < 0.5
}

fn apply_option_toggle(config: &mut GraphicConfig, idx: usize) {
    match idx {
        0 => config.framed_view_cone = !config.framed_view_cone,
        1 => config.display_shadow = !config.display_shadow,
        2 => config.display_titbits = !config.display_titbits,
        3 => config.display_anim = !config.display_anim,
        4 => config.apply_fog_to_all_sprites = !config.apply_fog_to_all_sprites,
        _ => {}
    }
}

fn is_option_selected(config: &GraphicConfig, idx: usize) -> bool {
    match idx {
        0 => !config.framed_view_cone,
        1 => config.display_shadow,
        2 => config.display_titbits,
        3 => config.display_anim,
        4 => config.apply_fog_to_all_sprites,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameter_edits_are_quantized_and_saturating() {
        let mut config = GraphicConfig::default();
        config.upscale_parameters.strength = 98;
        adjust_parameter(&mut config, false, 0, true);
        assert_eq!(config.upscale_parameters.strength, 100);
        adjust_parameter(&mut config, false, 0, false);
        assert_eq!(config.upscale_parameters.strength, 95);

        config.texture_effect_parameters.curvature = 2;
        adjust_parameter(&mut config, true, 3, false);
        assert_eq!(config.texture_effect_parameters.curvature, 0);
    }

    #[test]
    fn every_effect_has_a_distinct_persisted_choice() {
        assert_eq!(TextureEffect::ALL.len(), 3);
        assert_eq!(TextureEffect::ALL[0], TextureEffect::None);
        assert_ne!(TextureEffect::ALL[1], TextureEffect::ALL[2]);
    }
}
