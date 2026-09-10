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
    MenuTransform, dim_screen, draw_fallback_rect, draw_screen_background, enter_modal_gpu_phase,
    render_text_virt_font,
};
use super::resources::{
    IngameMenuResources, MT_BTN_CANCEL, MT_BTN_OK, MT_STR_ALPHA_VISION_FIELD,
    MT_STR_BCKGND_ANIMATIONS, MT_STR_EFFECT_ANIMATIONS, MT_STR_RES, MT_STR_RES_HIGH,
    MT_STR_RES_LOW, MT_STR_RES_MEDIUM, MT_STR_SPECIAL_FX, MT_STR_TRANSPARENT_SHADOWS,
    MT_TTL_GRAPHICS,
};
use super::widget_bridge::{self, ModalCursor, ModalInputState};

// Widget ID ranges: resolution 100..102, widescreen 150, options 200..209,
// scaling 400.., ok/cancel 300..301.
const ID_RES_BASE: u32 = 100;
const RESOLUTIONS: [(usize, f32, f32); 3] = [
    (MT_STR_RES_LOW, 640.0, 480.0),
    (MT_STR_RES_MEDIUM, 800.0, 600.0),
    (MT_STR_RES_HIGH, 1024.0, 768.0),
];
const ID_RES_LAST: u32 = ID_RES_BASE + RESOLUTIONS.len() as u32 - 1;
const ID_ADAPTIVE_WIDESCREEN: u32 = 150;
const ID_OPT_BASE: u32 = 200;
const OPTION_COUNT: u32 = 10;
const ID_OK: u32 = 300;
const ID_CANCEL: u32 = 301;
const ID_SCALE_BASE: u32 = 400;
const ID_EFFECT_BASE: u32 = 500;
const ID_PAGE_BASE: u32 = 600;
const COLUMN_W: i32 = 280;
const PARAMETER_ROW_H: i32 = 40;
const OPTION_START_Y: i32 = 112;
const OPTION_SPACING: i32 = 6;
const PRESET_LIST_X: i32 = 330;
const PRESET_LIST_Y: i32 = 150;
const PRESET_LIST_W: i32 = COLUMN_W;
const PRESET_LIST_ROW_H: i32 = 28;
const PRESET_LIST_ROWS: usize = 4;

/// Labels for the scaling radio column.
fn scale_modes() -> &'static [TextureScaleMode] {
    crate::shader_preset::available_texture_scale_modes()
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

    let mut edit = crate::options_model::GraphicsEdit::new(config.clone());
    let mut dirty = false;
    let retroarch_presets = crate::shader_preset::retroarch_presets();
    if edit.working.shader_preset.is_empty()
        && let Some(preset) = retroarch_presets.first()
    {
        edit.working.shader_preset = preset.id.clone();
    }
    let mut preset_scroll = preset_index(retroarch_presets, &edit.working.shader_preset)
        .unwrap_or(0)
        .saturating_sub(PRESET_LIST_ROWS / 2);

    let ok_label = resources.menu_text.get(MT_BTN_OK);
    let cancel_label = resources.menu_text.get(MT_BTN_CANCEL);
    let scale_modes = scale_modes();
    let row_h = resources.button_dimensions().1;
    let scale_x = 330;
    let scale_btn_w = COLUMN_W;
    let effect_y = OPTION_START_Y;
    let parameter_y = PRESET_LIST_Y;
    let mut page = 0;
    let mut frame = FrameWnd::default();
    frame.enabled = true;
    frame.input_enabled = true;
    let mut add = |id, label: &str, x, y, width, height| {
        let label = super::gameplay::fit_button_label(resources, label, true, width);
        frame.add_widget_absolute(widget_bridge::make_button(id, &label, x, y, width, height));
    };
    for (i, label) in ["Display", "Scaling", "Effects & Tuning"]
        .iter()
        .enumerate()
    {
        add(
            ID_PAGE_BASE + i as u32,
            label,
            30 + i as i32 * 196,
            55,
            188,
            row_h,
        );
    }
    for (i, &(label_id, _, _)) in RESOLUTIONS.iter().enumerate() {
        let label = resources.menu_text.get(label_id);
        add(
            ID_RES_BASE + i as u32,
            &label,
            30,
            OPTION_START_Y + i as i32 * (row_h + OPTION_SPACING),
            COLUMN_W,
            row_h,
        );
    }
    add(
        ID_ADAPTIVE_WIDESCREEN,
        "Adaptive Widescreen",
        30,
        OPTION_START_Y + RESOLUTIONS.len() as i32 * (row_h + OPTION_SPACING),
        COLUMN_W,
        row_h,
    );
    let option_labels = [
        resources.menu_text.get(MT_STR_ALPHA_VISION_FIELD),
        resources.menu_text.get(MT_STR_TRANSPARENT_SHADOWS),
        resources.menu_text.get(MT_STR_EFFECT_ANIMATIONS),
        resources.menu_text.get(MT_STR_BCKGND_ANIMATIONS),
        "Fog/Night All Sprites".into(),
        "Native Refresh Rate".into(),
        "Mission Countdown".into(),
        "Dynamic Ambience Visuals".into(),
        "Diplomacy Colors (neutral = amber)".into(),
        "Quick-Action Cursor Pulse".into(),
    ];
    assert_eq!(option_labels.len(), OPTION_COUNT as usize);
    for (i, label) in option_labels.iter().enumerate() {
        let (x, y) = option_position(i, row_h);
        add(ID_OPT_BASE + i as u32, label, x, y, COLUMN_W, row_h);
    }
    for (i, mode) in scale_modes.iter().enumerate() {
        let (x, y) = scaling_position(i, scale_modes.len(), row_h);
        add(
            ID_SCALE_BASE + i as u32,
            mode.label(),
            x,
            y,
            COLUMN_W,
            row_h,
        );
    }
    for (i, effect) in TextureEffect::ALL.iter().enumerate() {
        add(
            ID_EFFECT_BASE + i as u32,
            effect.label(),
            30,
            effect_y + i as i32 * (row_h + OPTION_SPACING),
            COLUMN_W,
            row_h,
        );
    }
    add(ID_OK, &ok_label, 330, 472 - row_h, 134, row_h);
    add(ID_CANCEL, &cancel_label, 476, 472 - row_h, 134, row_h);

    let title = resources.menu_text.get(MT_TTL_GRAPHICS);
    let res_label = resources.menu_text.get(MT_STR_RES);
    let fx_label = resources.menu_text.get(MT_STR_SPECIAL_FX);

    let mut done = false;
    let mut accepted = false;
    let mut parameter_page_effect = false;
    let mut parameter_status = String::new();
    let mut input_state = ModalInputState::new();
    input_state.seed_mouse_from_window(event_pump, transform);

    while !done {
        let (events, transform) = super::layout::poll_events_with_transform(event_pump, renderer);
        for event in events {
            input_state.update_from_event(&event, transform);
            match event {
                GameEvent::Quit => done = true,
                GameEvent::MouseDown(x, y, 1, _) if page == 2 => {
                    let (vx, vy) = transform.from_screen(x, y);
                    let row_count =
                        parameter_rows(&edit.working, parameter_page_effect).len() as i32;
                    if (edit.working.scale_mode != TextureScaleMode::RetroArch
                        || parameter_page_effect)
                        && (scale_x..scale_x + scale_btn_w).contains(&vx)
                        && (parameter_y..parameter_y + row_count * PARAMETER_ROW_H).contains(&vy)
                    {
                        let row = ((vy - parameter_y) / PARAMETER_ROW_H) as usize;
                        adjust_parameter(
                            &mut edit.working,
                            parameter_page_effect,
                            row,
                            vx >= scale_x + scale_btn_w / 2,
                        );
                        dirty = true;
                    } else if page == 2
                        && edit.working.scale_mode == TextureScaleMode::RetroArch
                        && !parameter_page_effect
                        && (PRESET_LIST_X..PRESET_LIST_X + PRESET_LIST_W).contains(&vx)
                        && (PRESET_LIST_Y
                            ..PRESET_LIST_Y + PRESET_LIST_ROW_H * PRESET_LIST_ROWS as i32)
                            .contains(&vy)
                    {
                        let row = ((vy - PRESET_LIST_Y) / PRESET_LIST_ROW_H) as usize;
                        let index = preset_scroll + row;
                        if let Some(preset) = retroarch_presets.get(index) {
                            select_builtin_preset(
                                &mut edit.working,
                                &preset.id,
                                &mut parameter_status,
                            );
                            dirty = true;
                        }
                    }
                }
                GameEvent::MouseWheel(delta)
                    if page == 2
                        && edit.working.scale_mode == TextureScaleMode::RetroArch
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
                } if page == 2 && edit.working.scale_mode == TextureScaleMode::RetroArch => {
                    match pick_retroarch_preset().await {
                        Ok(Some(path)) => {
                            let selected = path.to_string_lossy().to_string();
                            match renderer.validate_retroarch_preset(&selected) {
                                Ok(()) => {
                                    edit.working.shader_preset = selected;
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
                    if page == 2
                        && edit.working.scale_mode == TextureScaleMode::RetroArch
                        && !parameter_page_effect =>
                {
                    let current = preset_index(retroarch_presets, &edit.working.shader_preset)
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
                        select_builtin_preset(&mut edit.working, &preset.id, &mut parameter_status);
                        preset_scroll = keep_visible(next, preset_scroll, retroarch_presets.len());
                        dirty = true;
                    }
                }
                _ => {}
            }
        }

        for widget in frame.widgets_mut() {
            let active = widget_page(widget.id()).is_none_or(|owner| owner == page);
            let base = widget.base_mut();
            base.enabled = active;
            if !active {
                base.state = crate::ui::UiState::Default;
            }
        }
        let widget_input = input_state.as_widget_input();
        let events = frame.process_input(&widget_input);
        input_state.end_frame();

        if let Some(id) = widget_bridge::find_activated(&events) {
            match id {
                id if (ID_PAGE_BASE..ID_PAGE_BASE + 3).contains(&id) => page = id - ID_PAGE_BASE,
                ID_OK => {
                    accepted = true;
                    done = true;
                }
                ID_CANCEL => done = true,
                id if (ID_RES_BASE..=ID_RES_LAST).contains(&id) => {
                    apply_resolution(&mut edit.working, (id - ID_RES_BASE) as usize);
                    dirty = true;
                }
                ID_ADAPTIVE_WIDESCREEN => {
                    crate::options_model::adjust_graphics_setting(
                        &mut edit.working,
                        crate::options_model::GraphicsSetting::AdaptiveWidescreen,
                        1,
                    );
                    dirty = true;
                }
                id if (ID_OPT_BASE..ID_OPT_BASE + OPTION_COUNT).contains(&id) => {
                    apply_option_toggle(&mut edit.working, (id - ID_OPT_BASE) as usize);
                    dirty = true;
                }
                id if (ID_SCALE_BASE..ID_SCALE_BASE + scale_modes.len() as u32).contains(&id) => {
                    edit.working.scale_mode = scale_modes[(id - ID_SCALE_BASE) as usize];
                    if edit.working.scale_mode == TextureScaleMode::RetroArch {
                        let index = preset_index(retroarch_presets, &edit.working.shader_preset)
                            .unwrap_or(0);
                        preset_scroll = keep_visible(index, preset_scroll, retroarch_presets.len());
                    }
                    dirty = true;
                }
                id if (ID_EFFECT_BASE..ID_EFFECT_BASE + TextureEffect::ALL.len() as u32)
                    .contains(&id) =>
                {
                    edit.working.texture_effect =
                        TextureEffect::ALL[(id - ID_EFFECT_BASE) as usize];
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

        if let Some(font) = resources.title_font_any() {
            let tw = font.text_width(&title);
            render_text_virt_font(renderer, font, transform, &title, (640 - tw) / 2, 20);
        }
        if let Some(font) = resources.label_font_any() {
            if page == 0 {
                render_text_virt_font(renderer, font, transform, &res_label, 30, 90);
                render_text_virt_font(renderer, font, transform, &fx_label, 330, 90);
                render_text_virt_font(
                    renderer,
                    font,
                    transform,
                    &fx_label,
                    30,
                    option_position(7, row_h).1 - 22,
                );
            } else if page == 1 {
                render_text_virt_font(renderer, font, transform, "Scaling", 30, 90);
            }
            if page == 2 {
                render_text_virt_font(
                    renderer,
                    font,
                    transform,
                    "Texture effect",
                    30,
                    effect_y - 22,
                );
                if page == 2
                    && edit.working.scale_mode == TextureScaleMode::RetroArch
                    && !parameter_page_effect
                {
                    render_text_virt_font(
                        renderer,
                        font,
                        transform,
                        "Preset",
                        PRESET_LIST_X,
                        PRESET_LIST_Y - 18,
                    );
                    render_text_virt_font(
                        renderer,
                        font,
                        transform,
                        "Press I to import a preset",
                        PRESET_LIST_X,
                        PRESET_LIST_Y + PRESET_LIST_ROW_H * PRESET_LIST_ROWS as i32 + 2,
                    );
                } else {
                    let page = if parameter_page_effect {
                        "Effect parameters (Tab)"
                    } else {
                        "Upscaler parameters (Tab)"
                    };
                    render_text_virt_font(
                        renderer,
                        font,
                        transform,
                        page,
                        scale_x,
                        parameter_y - 24,
                    );
                }
            }
            if page == 2 && !parameter_status.is_empty() {
                let status = fit_label(font, &parameter_status, 580);
                render_text_virt_font(renderer, font, transform, &status, 30, 410);
            }
        }

        // Render only controls owned by the active page.
        if page == 0 {
            for i in 0..RESOLUTIONS.len() as u32 {
                if let Some(w) = frame.widget(ID_RES_BASE + i) {
                    widget_bridge::draw_widget_radio(
                        renderer,
                        resources,
                        transform,
                        w,
                        is_resolution_selected(&edit.working, i as usize),
                    );
                }
            }
            if let Some(w) = frame.widget(ID_ADAPTIVE_WIDESCREEN) {
                widget_bridge::draw_widget_radio(
                    renderer,
                    resources,
                    transform,
                    w,
                    edit.working.adaptive_widescreen,
                );
            }
            for i in 0..OPTION_COUNT {
                if let Some(w) = frame.widget(ID_OPT_BASE + i) {
                    widget_bridge::draw_widget_radio(
                        renderer,
                        resources,
                        transform,
                        w,
                        is_option_selected(&edit.working, i as usize),
                    );
                }
            }
        }
        if page == 1 {
            for (i, mode) in scale_modes.iter().enumerate() {
                if let Some(w) = frame.widget(ID_SCALE_BASE + i as u32) {
                    widget_bridge::draw_widget_radio(
                        renderer,
                        resources,
                        transform,
                        w,
                        edit.working.scale_mode == *mode,
                    );
                }
            }
        }
        if page == 2 {
            for (i, effect) in TextureEffect::ALL.iter().enumerate() {
                if let Some(w) = frame.widget(ID_EFFECT_BASE + i as u32) {
                    widget_bridge::draw_widget_radio(
                        renderer,
                        resources,
                        transform,
                        w,
                        edit.working.texture_effect == *effect,
                    );
                }
            }

            if page == 2
                && edit.working.scale_mode == TextureScaleMode::RetroArch
                && !parameter_page_effect
            {
                draw_preset_list(
                    renderer,
                    resources,
                    transform,
                    retroarch_presets,
                    preset_scroll,
                    &edit.working.shader_preset,
                );
            } else {
                draw_parameter_panel(
                    renderer,
                    resources,
                    transform,
                    &edit.working,
                    parameter_page_effect,
                    scale_x,
                    parameter_y,
                    scale_btn_w,
                );
            }
        }
        for i in 0..3 {
            widget_bridge::draw_widget_button(
                renderer,
                resources,
                transform,
                frame
                    .widget(ID_PAGE_BASE + i)
                    .expect("graphics page button"),
                page == i,
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
        crate::window::sleep_ui_frame().await;
    }

    let outcome = edit.commit(accepted && dirty, config);
    if outcome.0 {
        renderer.apply_upscale_config(config);
    }
    outcome
}

fn widget_page(id: u32) -> Option<u32> {
    match id {
        ID_ADAPTIVE_WIDESCREEN | ID_RES_BASE..=ID_RES_LAST | ID_OPT_BASE..=209 => Some(0),
        ID_SCALE_BASE..=499 => Some(1),
        ID_EFFECT_BASE..=599 => Some(2),
        _ => None,
    }
}

fn scaling_position(index: usize, count: usize, row_h: i32) -> (i32, i32) {
    let rows = count.div_ceil(2).max(1);
    assert!(rows <= 8, "scaling choices need another page");
    (
        if index < rows { 30 } else { 330 },
        OPTION_START_Y + (index % rows) as i32 * (row_h + OPTION_SPACING),
    )
}

fn option_position(index: usize, row_h: i32) -> (i32, i32) {
    assert!(
        index < OPTION_COUNT as usize,
        "invalid graphics option index"
    );
    if index < 7 {
        (
            330,
            OPTION_START_Y + index as i32 * (row_h + OPTION_SPACING),
        )
    } else {
        (
            30,
            OPTION_START_Y
                + 4 * (row_h + OPTION_SPACING)
                + 24
                + (index - 7) as i32 * (row_h + OPTION_SPACING),
        )
    }
}

/// Selecting a bundled preset invalidates feedback about a previous import.
fn select_builtin_preset(config: &mut GraphicConfig, preset_id: &str, status: &mut String) {
    preset_id.clone_into(&mut config.shader_preset);
    status.clear();
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
    } else if index - scroll >= PRESET_LIST_ROWS {
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

    let Some(font) = resources.label_font_any() else {
        return;
    };
    for (row, preset) in presets
        .iter()
        .skip(scroll)
        .take(PRESET_LIST_ROWS)
        .enumerate()
    {
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
        render_text_virt_font(renderer, font, transform, &label, PRESET_LIST_X + 4, y + 1);
    }
}

fn fit_label<'a>(
    font: &crate::native_font::Font,
    label: &'a str,
    max_w: i32,
) -> std::borrow::Cow<'a, str> {
    fit_label_by(label, max_w, |candidate| font.text_width(candidate))
}

fn fit_label_by(
    label: &str,
    max_w: i32,
    measure: impl Fn(&str) -> i32,
) -> std::borrow::Cow<'_, str> {
    if measure(label) <= max_w {
        return std::borrow::Cow::Borrowed(label);
    }
    let mut out = String::with_capacity(label.len() + 3);
    out.push_str(label);
    out.push_str("...");
    // Preserve the existing scalar-at-a-time removal and whole-candidate
    // measurement; kerning means character widths cannot simply be added.
    while out.len() > 3 && measure(&out) > max_w {
        out.truncate(out.len() - 3);
        out.pop();
        out.push_str("...");
    }
    // Legacy graphics labels retain the ellipsis even if it cannot fit.
    std::borrow::Cow::Owned(out)
}

fn parameter_rows(
    config: &GraphicConfig,
    effect_page: bool,
) -> impl ExactSizeIterator<Item = (crate::options_model::GraphicsSetting, &'static str, u8)> {
    use crate::options_model::GraphicsSetting::*;
    let rows = [
        (
            UpscaleStrength,
            "Strength",
            config.upscale_parameters.strength,
        ),
        (
            UpscaleEdgeThreshold,
            "Edge threshold",
            config.upscale_parameters.edge_threshold,
        ),
        (
            UpscaleArtifactRemoval,
            "Artifact removal",
            config.upscale_parameters.artifact_removal,
        ),
        (
            EffectScanlines,
            "Scanlines",
            config.texture_effect_parameters.scanlines,
        ),
        (
            EffectPhosphorMask,
            "Phosphor mask",
            config.texture_effect_parameters.phosphor_mask,
        ),
        (EffectBloom, "Bloom", config.texture_effect_parameters.bloom),
        (
            EffectCurvature,
            "Curvature",
            config.texture_effect_parameters.curvature,
        ),
        (
            EffectTemporalFlicker,
            "Temporal flicker",
            config.texture_effect_parameters.temporal_flicker,
        ),
    ];
    let (start, count) = if effect_page { (3, 5) } else { (0, 3) };
    rows.into_iter().skip(start).take(count)
}

fn adjust_parameter(config: &mut GraphicConfig, effect_page: bool, index: usize, increase: bool) {
    let (setting, _, _) = parameter_rows(config, effect_page)
        .nth(index)
        .expect("graphics parameter row must exist");
    crate::options_model::adjust_graphics_setting(config, setting, if increase { 1 } else { -1 });
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
    let Some(font) = resources.label_font_any() else {
        return;
    };
    for (row, (_, label, value)) in parameter_rows(config, effect_page).enumerate() {
        let row_y = y + row as i32 * PARAMETER_ROW_H;
        render_text_virt_font(
            renderer,
            font,
            transform,
            &format!("{label}: {value:3}%"),
            x,
            row_y,
        );
        let bar_x = x;
        let (screen_x, screen_y) = transform.to_screen(bar_x, row_y + 22);
        let bar_w = width - 8;
        let filled = (bar_w * i32::from(value)) / 100;
        renderer.fill_screen(
            Some(&engine_sprite::BBox::from_coords(
                screen_x as f32,
                screen_y as f32,
                (screen_x + bar_w) as f32,
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

#[cfg(all(
    feature = "dialogs",
    any(target_os = "windows", target_os = "linux", target_os = "macos")
))]
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

#[cfg(not(all(
    feature = "dialogs",
    any(target_os = "windows", target_os = "linux", target_os = "macos")
)))]
async fn pick_retroarch_preset() -> Result<Option<std::path::PathBuf>, String> {
    Err(
        "RetroArch preset import is unavailable in this build; rebuild a native client with `--features dialogs`"
            .to_string(),
    )
}

fn apply_resolution(config: &mut GraphicConfig, idx: usize) {
    let (_, width, height) = RESOLUTIONS[idx];
    config.set_resolution(width, height);
}

fn is_resolution_selected(config: &GraphicConfig, idx: usize) -> bool {
    let (_, want_x, want_y) = RESOLUTIONS[idx];
    (config.resolution_x - want_x).abs() < 0.5 && (config.resolution_y - want_y).abs() < 0.5
}

const ORIGINAL_TOGGLES: [crate::options_model::GraphicsSetting; 10] = {
    use crate::options_model::GraphicsSetting::*;
    [
        AlphaVisionField,
        TransparentShadows,
        EffectAnimations,
        BackgroundAnimations,
        FogNightAllSprites,
        NativeRefreshPresentation,
        MissionCountdown,
        DynamicAmbienceVisuals,
        DiplomacyVisuals,
        QuickActionCursorPulse,
    ]
};

fn apply_option_toggle(config: &mut GraphicConfig, idx: usize) {
    crate::options_model::adjust_graphics_setting(config, ORIGINAL_TOGGLES[idx], 1);
}

fn is_option_selected(config: &GraphicConfig, idx: usize) -> bool {
    match idx {
        0 => !config.framed_view_cone,
        1 => config.display_shadow,
        2 => config.display_titbits,
        3 => config.display_anim,
        4 => config.apply_fog_to_all_sprites,
        5 => config.native_refresh_presentation,
        6 => config.show_mission_countdown,
        7 => config.dynamic_ambience_visuals,
        8 => config.diplomacy_visuals,
        9 => config.quick_action_cursor_pulse,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphics_pages_keep_all_choices_above_the_footer() {
        let row_h = 34;
        let modes = scale_modes();
        for (index, _) in modes.iter().enumerate() {
            let (x, y) = scaling_position(index, modes.len(), row_h);
            assert!(x >= 30 && x + COLUMN_W <= 610);
            assert!(y >= OPTION_START_Y && y + row_h <= 426);
            for previous in 0..index {
                let (other_x, other_y) = scaling_position(previous, modes.len(), row_h);
                assert!(x != other_x || y - other_y >= row_h + OPTION_SPACING);
            }
            assert_eq!(widget_page(ID_SCALE_BASE + index as u32), Some(1));
        }
        for index in 0..OPTION_COUNT {
            assert_eq!(widget_page(ID_OPT_BASE + index), Some(0));
        }
        for index in 0..TextureEffect::ALL.len() {
            assert_eq!(widget_page(ID_EFFECT_BASE + index as u32), Some(2));
        }
        for id in [ID_OK, ID_CANCEL, ID_PAGE_BASE, ID_PAGE_BASE + 2] {
            assert_eq!(widget_page(id), None);
        }
        const { assert!(PRESET_LIST_Y + 5 * PARAMETER_ROW_H < 410) };
    }

    #[test]
    fn timed_ambience_graphics_options_are_independently_reachable() {
        let mut config = GraphicConfig::default();
        let adaptive_widescreen = config.adaptive_widescreen;
        let native_refresh_index = 5;

        apply_option_toggle(&mut config, native_refresh_index);

        assert!(!config.native_refresh_presentation);
        assert_eq!(config.adaptive_widescreen, adaptive_widescreen);
        assert!(!is_option_selected(&config, native_refresh_index));

        apply_option_toggle(&mut config, 6);
        assert!(!config.show_mission_countdown);
        assert!(config.dynamic_ambience_visuals);
        apply_option_toggle(&mut config, 7);
        assert!(!config.dynamic_ambience_visuals);
        apply_option_toggle(&mut config, 8);
        assert!(!config.diplomacy_visuals);
        assert!(config.quick_action_cursor_pulse);
        apply_option_toggle(&mut config, 9);
        assert!(!config.quick_action_cursor_pulse);
        assert_eq!(OPTION_COUNT, 10);
    }

    #[test]
    fn all_graphics_option_rows_fit_and_keep_stable_mappings_at_640x480() {
        let row_h = 34;
        for index in 0..OPTION_COUNT as usize {
            let (x, y) = option_position(index, row_h);
            assert!(x >= 30 && x + COLUMN_W <= 610);
            assert!(y + row_h <= 426);
            if x == 30 {
                assert!(y >= 296, "toggles must sit below the resolution controls");
            }
        }

        let mut config = GraphicConfig::default();
        let before = config.clone();
        apply_option_toggle(&mut config, 9);
        assert!(!config.quick_action_cursor_pulse);
        assert_eq!(
            config.native_refresh_presentation,
            before.native_refresh_presentation
        );
        assert_eq!(config.show_mission_countdown, before.show_mission_countdown);
        assert_eq!(
            config.dynamic_ambience_visuals,
            before.dynamic_ambience_visuals
        );
        assert_eq!(config.diplomacy_visuals, before.diplomacy_visuals);
        assert!(!is_option_selected(&config, 9));
    }

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

#[test]
fn preset_scroll_preserves_visibility_without_offset_overflow() {
    for total in 1usize..32 {
        let max_scroll = total.saturating_sub(PRESET_LIST_ROWS);
        for index in 0..total {
            for scroll in 0..=max_scroll {
                let expected = if index < scroll {
                    index
                } else if index >= scroll + PRESET_LIST_ROWS {
                    index.saturating_sub(PRESET_LIST_ROWS - 1).min(max_scroll)
                } else {
                    scroll.min(max_scroll)
                };
                let actual = keep_visible(index, scroll, total);
                assert_eq!(actual, expected);
                assert!(actual <= index && index - actual < PRESET_LIST_ROWS);
                assert!(actual <= max_scroll);
            }
        }
    }
    assert_eq!(keep_visible(0, 0, 0), 0);
    assert_eq!(
        keep_visible(usize::MAX - 1, usize::MAX - 2, usize::MAX),
        usize::MAX - PRESET_LIST_ROWS
    );
}

#[test]
fn selecting_builtin_presets_clears_stale_import_feedback() {
    let mut config = GraphicConfig::default();
    config.shader_preset = "/tmp/imported.slangp".into();
    for message in [
        "Imported preset validated",
        "Import failed: invalid preset",
        "",
    ] {
        let mut status = message.to_owned();
        select_builtin_preset(&mut config, "bundled-preset", &mut status);
        assert_eq!(config.shader_preset, "bundled-preset");
        assert!(status.is_empty());
    }
}

#[test]
fn graphics_label_fitting_preserves_scalar_boundaries_and_ellipsis_policy() {
    let measure = |value: &str| value.chars().count() as i32;
    assert!(matches!(
        fit_label_by("é🏹", 2, measure),
        std::borrow::Cow::Borrowed("é🏹")
    ));
    assert_eq!(fit_label_by("é🏹罗宾AB", 5, measure), "é🏹...");
    assert_eq!(fit_label_by("abcdef", 3, measure), "...");
    assert_eq!(fit_label_by("abcdef", 0, measure), "...");
    assert_eq!(fit_label_by("", -1, measure), "...");
    assert_eq!(
        fit_label_by("abcdef", 2, |value| if value == "abcd..." { 2 } else { 10 }),
        "abcd..."
    );
}

#[test]
fn every_parameter_row_edits_only_its_displayed_value() {
    let mut config = GraphicConfig::default();
    config.upscale_parameters.strength = 5;
    config.upscale_parameters.edge_threshold = 10;
    config.upscale_parameters.artifact_removal = 15;
    config.texture_effect_parameters.scanlines = 20;
    config.texture_effect_parameters.phosphor_mask = 25;
    config.texture_effect_parameters.bloom = 30;
    config.texture_effect_parameters.curvature = 35;
    config.texture_effect_parameters.temporal_flicker = 40;
    let values = |config: &GraphicConfig| {
        parameter_rows(config, false)
            .chain(parameter_rows(config, true))
            .map(|(_, _, value)| value)
            .collect::<Vec<_>>()
    };
    assert_eq!(values(&config), [5, 10, 15, 20, 25, 30, 35, 40]);
    for (effect_page, offset, labels) in [
        (
            false,
            0,
            &["Strength", "Edge threshold", "Artifact removal"][..],
        ),
        (
            true,
            3,
            &[
                "Scanlines",
                "Phosphor mask",
                "Bloom",
                "Curvature",
                "Temporal flicker",
            ][..],
        ),
    ] {
        assert_eq!(parameter_rows(&config, effect_page).len(), labels.len());
        assert_eq!(
            parameter_rows(&config, effect_page)
                .map(|(_, label, _)| label)
                .collect::<Vec<_>>(),
            labels
        );
        for index in 0..labels.len() {
            let mut edited = config.clone();
            adjust_parameter(&mut edited, effect_page, index, true);
            let mut expected = values(&config);
            expected[offset + index] += 5;
            assert_eq!(values(&edited), expected);
        }
    }
}

#[test]
fn resolution_rows_share_labels_values_selection_and_page_ownership() {
    assert_eq!(
        RESOLUTIONS,
        [
            (MT_STR_RES_LOW, 640.0, 480.0),
            (MT_STR_RES_MEDIUM, 800.0, 600.0),
            (MT_STR_RES_HIGH, 1024.0, 768.0),
        ]
    );
    let mut config = GraphicConfig::default();
    for (index, &(_, width, height)) in RESOLUTIONS.iter().enumerate() {
        apply_resolution(&mut config, index);
        assert_eq!((config.resolution_x, config.resolution_y), (width, height));
        for other in 0..RESOLUTIONS.len() {
            assert_eq!(is_resolution_selected(&config, other), index == other);
        }
        assert_eq!(widget_page(ID_RES_BASE + index as u32), Some(0));
        config.resolution_x = width + 0.49;
        assert!(is_resolution_selected(&config, index));
        config.resolution_x = width + 0.5;
        assert!(!is_resolution_selected(&config, index));
    }
}
