//! Virtual coordinate system, layout helpers, button and text rendering.
//!
//! Covers menu-window alignment (AlignBottomRight, CenterHorizontally,
//! AlignOnFirstWidget), button sprite dispatch for
//! normal/hover/pressed/selected state, and greedy word wrap with
//! orphan avoidance (used by the debriefing modal for the justified
//! mission summary).

use crate::native_font::{Font, NativeFont};
use crate::renderer::{BLIT_SOURCE_TRANSPARENT, Renderer};
use robin_engine::coordinates as engine_coordinates;
use robin_engine::sprite::BBox;

use super::resources::IngameMenuResources;

// ═══════════════════════════════════════════════════════════════════
// Virtual coordinate system
// ═══════════════════════════════════════════════════════════════════

/// Virtual menu width — every menu window is laid out at 640x480.
pub const MENU_W: i32 = 640;
/// Virtual menu height.
pub const MENU_H: i32 = 480;

/// Per-side horizontal inset applied inside a text box before wrapping
/// or rendering.
const KERNING_MARGIN: i32 = 2;

/// Button sub-picture indices — used as indices into a flagged picture
/// pack (BTTN resource), so slot 0 is the *disabled* sprite, not the
/// default.  `RHID_OK` (the small round seal) in particular doesn't
/// ship a disabled frame, so asking for slot 0 returns `None` and used
/// to fall through to the regular `RHID_MENU_BUTTON` placeholder,
/// producing a grey rectangle until the user hovered and we switched
/// to slot 2.
pub const BTN_STATE_DISABLED: usize = 0;
pub const BTN_STATE_NORMAL: usize = 1;
pub const BTN_STATE_HOVER: usize = 2;
pub const BTN_STATE_PRESSED: usize = 3;
/// Legacy alias kept for existing call sites (focus highlight).  The
/// selected state shares its sprite slot with the hover state.
pub const BTN_STATE_SELECTED: usize = BTN_STATE_HOVER;

/// Default button spacing.
pub const BUTTON_SPACING: i32 = 2;

/// Translation from 640x480 virtual menu coordinates to actual screen pixels.
#[derive(Copy, Clone, Debug)]
pub struct MenuTransform {
    pub origin_x: i32,
    pub origin_y: i32,
}

impl MenuTransform {
    /// Center the 640x480 menu window inside the active screen.
    pub fn centered(screen_w: i32, screen_h: i32) -> Self {
        Self {
            origin_x: (screen_w - MENU_W) / 2,
            origin_y: (screen_h - MENU_H) / 2,
        }
    }

    pub fn to_screen(self, x: i32, y: i32) -> (i32, i32) {
        (self.origin_x + x, self.origin_y + y)
    }

    pub fn from_screen(self, sx: i32, sy: i32) -> (i32, i32) {
        (sx - self.origin_x, sy - self.origin_y)
    }
}

/// Drain window events, synchronize native presentation/logical dimensions,
/// and return the transform that matches those events' pointer coordinates.
///
/// `GameWindow` applies the aspect policy while draining resize messages. Menu
/// loops must therefore refresh the renderer and their virtual transform before
/// interpreting events from that drain; doing it a frame later makes clicks
/// disagree with widgets immediately after a resize.
pub fn poll_events_with_transform(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
) -> (Vec<crate::gfx_types::GameEvent>, MenuTransform) {
    let events = event_pump.poll_events();
    renderer.sync_window_size(event_pump);
    let transform = MenuTransform::centered(
        i32::from(renderer.screen_width()),
        i32::from(renderer.screen_height()),
    );
    (events, transform)
}

// ═══════════════════════════════════════════════════════════════════
// Widget layout
// ═══════════════════════════════════════════════════════════════════

/// A menu button with label and hit-box in virtual 640x480 coordinates.
#[derive(Debug, Clone)]
pub struct MenuButton {
    pub label: String,
    pub enabled: bool,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl MenuButton {
    pub fn contains_virt(&self, vx: i32, vy: i32) -> bool {
        vx >= self.x && vx < self.x + self.w && vy >= self.y && vy < self.y + self.h
    }
}

/// Stacks the buttons flush to the bottom-right of the virtual 640x480
/// window with a fixed spacing.
pub fn align_bottom_right(labels: &[(&str, bool)], btn_w: i32, btn_h: i32) -> Vec<MenuButton> {
    align_bottom_right_in(
        labels,
        btn_w,
        btn_h,
        &MenuRect {
            x: 0,
            y: 0,
            w: MENU_W,
            h: MENU_H,
        },
    )
}

/// Stacks the buttons flush to the bottom-right inside an arbitrary
/// sub-rect.
pub fn align_bottom_right_in(
    labels: &[(&str, bool)],
    btn_w: i32,
    btn_h: i32,
    container: &MenuRect,
) -> Vec<MenuButton> {
    if labels.is_empty() {
        return Vec::new();
    }
    let n = labels.len() as i32;
    let total_h = n * btn_h + (n - 1) * BUTTON_SPACING;
    let start_x = container.x + container.w - btn_w;
    let start_y = container.y + container.h - total_h;
    labels
        .iter()
        .enumerate()
        .map(|(i, (label, enabled))| MenuButton {
            label: (*label).to_string(),
            enabled: *enabled,
            x: start_x,
            y: start_y + i as i32 * (btn_h + BUTTON_SPACING),
            w: btn_w,
            h: btn_h,
        })
        .collect()
}

/// Lays out buttons on one horizontal row, centred inside the 640x480
/// window at the shared y of the first entry.
pub fn center_horizontally(
    labels: &[(&str, bool)],
    btn_w: i32,
    btn_h: i32,
    spacing: i32,
    y: i32,
) -> Vec<MenuButton> {
    if labels.is_empty() {
        return Vec::new();
    }
    let n = labels.len() as i32;
    let total_w = n * btn_w + (n - 1) * spacing;
    let start_x = (MENU_W - total_w) / 2;
    labels
        .iter()
        .enumerate()
        .map(|(i, (label, enabled))| MenuButton {
            label: (*label).to_string(),
            enabled: *enabled,
            x: start_x + i as i32 * (btn_w + spacing),
            y,
            w: btn_w,
            h: btn_h,
        })
        .collect()
}

/// Stacks widgets vertically starting at the first entry's virtual
/// position.
pub fn align_on_first_widget(buttons: &mut [MenuButton], spacing: i32) {
    if buttons.is_empty() {
        return;
    }
    let start_x = buttons[0].x;
    let mut cur_y = buttons[0].y;
    for btn in buttons.iter_mut() {
        btn.x = start_x;
        btn.y = cur_y;
        cur_y += btn.h + spacing;
    }
}

/// Plain virtual rectangle used by sub-layout helpers.
#[derive(Copy, Clone, Debug)]
pub struct MenuRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl MenuRect {
    pub fn contains_virt(&self, vx: i32, vy: i32) -> bool {
        vx >= self.x && vx < self.x + self.w && vy >= self.y && vy < self.y + self.h
    }
}

// ═══════════════════════════════════════════════════════════════════
// Window background
// ═══════════════════════════════════════════════════════════════════

/// Colorize the entire screen with a dark teal tint before drawing an overlay.
///
/// Per-pixel HSV hue replacement with hue 150 (cyan-blue) and
/// brightness scale 0.2.
pub fn dim_screen(renderer: &mut Renderer) {
    renderer.colorize_framebuffer(150.0, 0.2);
}

/// Snapshot the prior gameplay frame so the modal can dim/tint and
/// overlay widgets on top of it.
///
/// The wgpu port snapshots the offscreen render target (which holds
/// the fully-composited scene — bg map + portraits + HUD + characters),
/// then `present()` draws the snapshot before walking the queued
/// draws. The snapshot is idempotent and persists until the gameplay
/// path resumes.
pub fn enter_modal_gpu_phase(renderer: &mut Renderer) {
    renderer.freeze_scene_for_modal();
    renderer.begin_ui_layer();
}

/// Blit a background surface into a virtual sub-rect at `(virt_x, virt_y)`
/// of size `(virt_w, virt_h)`.  Used by every modal to put up the parchment
/// / small / numbered background before rendering its contents.
pub fn draw_background(
    renderer: &mut Renderer,
    transform: MenuTransform,
    surface: &super::resources::MenuSurface,
    virt_x: i32,
    virt_y: i32,
    virt_w: i32,
    virt_h: i32,
) {
    let (sx, sy) = transform.to_screen(virt_x, virt_y);
    let src = BBox::from_coords(0.0, 0.0, surface.width as f32, surface.height as f32);
    let dst = BBox::from_coords(
        sx as f32,
        sy as f32,
        (sx + virt_w) as f32,
        (sy + virt_h) as f32,
    );
    renderer.blit_to_screen(surface.id, Some(&src), Some(&dst), BLIT_SOURCE_TRANSPARENT);
}

/// Blit a menu-screen background across the active screen.
///
/// This is the screen-level background layer: it covers the whole
/// screen rather than the centered 640x480 menu window.
pub fn draw_screen_background(renderer: &mut Renderer, surface: &super::resources::MenuSurface) {
    let src = BBox::from_coords(0.0, 0.0, surface.width as f32, surface.height as f32);
    let dst = BBox::from_coords(
        0.0,
        0.0,
        renderer.screen_width() as f32,
        renderer.screen_height() as f32,
    );
    renderer.blit_to_screen(surface.id, Some(&src), Some(&dst), 0);
}

// ═══════════════════════════════════════════════════════════════════
// Button drawing
// ═══════════════════════════════════════════════════════════════════

/// Pick the right button sprite for the current interaction state:
/// disabled → slot 0, pushed → slot 3, focused → slot 2, else slot 1.
pub fn button_sprite_state(enabled: bool, hovered: bool, pressed: bool) -> usize {
    if !enabled {
        BTN_STATE_DISABLED
    } else if pressed {
        BTN_STATE_PRESSED
    } else if hovered {
        BTN_STATE_HOVER
    } else {
        BTN_STATE_NORMAL
    }
}

/// Fallback button look when DEFAULT.RES is unavailable.
pub fn draw_fallback_rect(renderer: &mut Renderer, x: i32, y: i32, w: i32, h: i32, hovered: bool) {
    let bg = if hovered {
        Renderer::create_color_16(80, 60, 40)
    } else {
        Renderer::create_color_16(40, 35, 25)
    };
    renderer.fill_screen(
        Some(&BBox::from_coords(
            x as f32,
            y as f32,
            (x + w) as f32,
            (y + h) as f32,
        )),
        bg,
    );
    let border = Renderer::create_color_16(180, 160, 100);
    renderer.draw_rect_outline_screen(x, y, x + w, y + h, border);
}

/// Draw the shared dark panel/input fallback used when a menu bitmap is
/// unavailable.
pub fn draw_fallback_panel(renderer: &mut Renderer, transform: MenuTransform, rect: &MenuRect) {
    let (sx, sy) = transform.to_screen(rect.x, rect.y);
    renderer.fill_screen(
        Some(&BBox::from_coords(
            sx as f32,
            sy as f32,
            (sx + rect.w) as f32,
            (sy + rect.h) as f32,
        )),
        Renderer::create_color_16(30, 25, 15),
    );
    renderer.draw_rect_outline_screen(
        sx,
        sy,
        sx + rect.w,
        sy + rect.h,
        Renderer::create_color_16(180, 160, 100),
    );
}

/// Render a slider widget — a 0..10 horizontal track with a thumb
/// showing the current position.  The slider sprite pack typically has
/// the track + 10 thumb positions; we pick the frame matching `value`
/// (clamped to 0..=10) or fall back to a plain rect.
pub fn draw_slider(
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    transform: MenuTransform,
    rect: &MenuRect,
    value: u16,
    max: u16,
) {
    let (x, y) = transform.to_screen(rect.x, rect.y);
    // Track
    let bg = Renderer::create_color_16(25, 20, 10);
    renderer.fill_screen(
        Some(&BBox::from_coords(
            x as f32,
            (y + rect.h / 2 - 3) as f32,
            (x + rect.w) as f32,
            (y + rect.h / 2 + 3) as f32,
        )),
        bg,
    );
    renderer.draw_rect_outline_screen(
        x,
        y + rect.h / 2 - 3,
        x + rect.w,
        y + rect.h / 2 + 3,
        Renderer::create_color_16(180, 160, 100),
    );
    // Thumb
    let t = if max == 0 {
        0.0
    } else {
        (value as f32 / max as f32).clamp(0.0, 1.0)
    };
    let thumb_w = 12;
    let thumb_x = x + ((rect.w - thumb_w) as f32 * t) as i32;
    renderer.fill_screen(
        Some(&BBox::from_coords(
            thumb_x as f32,
            y as f32,
            (thumb_x + thumb_w) as f32,
            (y + rect.h) as f32,
        )),
        Renderer::create_color_16(220, 200, 140),
    );
    renderer.draw_rect_outline_screen(
        thumb_x,
        y,
        thumb_x + thumb_w,
        y + rect.h,
        Renderer::create_color_16(255, 240, 180),
    );
    // Attempt to overlay the slider-thumb sprite on top of the fallback
    // if the resource pack is available.  The sprite is centred on the
    // thumb and sized to its own dimensions.
    if let (Some(&Some(sprite)), true) = (resources.slider_frames.first(), resources.slider_w > 0) {
        let sw = resources.slider_w.min(thumb_w + 4);
        let sh = resources.slider_h.min(rect.h);
        let sx = thumb_x + (thumb_w - sw) / 2;
        let sy = y + (rect.h - sh) / 2;
        let src = BBox::from_coords(
            0.0,
            0.0,
            resources.slider_w as f32,
            resources.slider_h as f32,
        );
        let dst = BBox::from_coords(sx as f32, sy as f32, (sx + sw) as f32, (sy + sh) as f32);
        renderer.blit_to_screen(sprite, Some(&src), Some(&dst), BLIT_SOURCE_TRANSPARENT);
    }
}

// ═══════════════════════════════════════════════════════════════════
// Text rendering
// ═══════════════════════════════════════════════════════════════════

/// Render text at a virtual position.
pub fn render_text_virt(
    renderer: &mut Renderer,
    font: &NativeFont,
    transform: MenuTransform,
    text: &str,
    vx: i32,
    vy: i32,
) {
    let (sx, sy) = transform.to_screen(vx, vy);
    render_text_screen(renderer, font, text, sx, sy);
}

/// Render text directly at a screen position.
///
/// Builds a small ARGB8888 surface sized to the glyph bounding box and
/// blits it with `BLENDMODE_BLEND`.  Per-pixel alpha blend against the
/// destination so anti-aliased edges fade into the background
/// (parchment, menu frame, …) instead of being written verbatim and
/// leaving a green halo.
pub fn render_text_screen(renderer: &mut Renderer, font: &NativeFont, text: &str, x: i32, y: i32) {
    renderer.render_text_argb(font, text, x, y);
}

/// `Font`-polymorphic version of `render_text_virt`. Dispatches the
/// native variant through `Renderer::render_text_argb` (atlas + quads)
/// and the TrueType variant through `Renderer::render_text_truetype`
/// (per-string ARGB rasterise + one-shot upload). Used by list views
/// (`save_load`, `shortcuts`, `player_select`) where the resolved font
/// can be either kind depending on `manager.cfg`.
pub fn render_text_virt_font(
    renderer: &mut Renderer,
    font: &Font,
    transform: MenuTransform,
    text: &str,
    vx: i32,
    vy: i32,
) {
    let (sx, sy) = transform.to_screen(vx, vy);
    render_text_screen_font(renderer, font, text, sx, sy);
}

/// `Font`-polymorphic version of `render_text_screen`.
pub fn render_text_screen_font(renderer: &mut Renderer, font: &Font, text: &str, x: i32, y: i32) {
    match font {
        Font::Native(n) => renderer.render_text_argb(n, text, x, y),
        Font::TrueType(tt) => renderer.render_text_truetype(tt, text, x, y),
    }
}

/// Horizontal alignment for wrapped text.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    /// Right-anchored: line snaps to `box_x + box_w - line_width`.
    Right,
    Justified,
}

/// Vertical alignment inside a box.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VAlign {
    /// Anchor text to the top of the box (legacy default).
    Top,
    /// Baseline-aware vertical centre.  Uses `font.baseline()` so the
    /// visual centre of the letters (ignoring descenders) lines up
    /// with the box midline.
    Center,
}

/// Result of a single wrap pass: how much text fit, and what was left
/// over for the next page.  Used by the debriefing modal for
/// pagination.
#[derive(Debug, Clone, Default)]
pub struct WrapResult {
    /// The unrendered remainder (empty when everything fit).
    pub remaining: String,
    /// The lines actually produced by the wrap pass.
    pub lines: Vec<String>,
    /// For each line, `true` if it is the final line of its paragraph
    /// (i.e. the next boundary is a hard `\n` or end-of-text, not a
    /// greedy wrap break).  Justified alignment must leave these lines
    /// un-justified.
    pub paragraph_end: Vec<bool>,
}

/// Wrap text greedily, clipping to the supplied box height.
///
/// Implements orphan avoidance: if the final line of a paragraph would
/// contain a single ≤5-character word, the previous line's last word
/// is bumped down.
pub fn wrap_text(font: &NativeFont, text: &str, box_w: i32, max_lines: usize) -> WrapResult {
    let mut lines: Vec<String> = Vec::new();
    let mut paragraph_end: Vec<bool> = Vec::new();
    let mut consumed_chars = 0usize; // byte count of text consumed so far
    if max_lines == 0 || box_w <= 0 {
        return WrapResult {
            remaining: text.to_string(),
            lines,
            paragraph_end,
        };
    }

    let mut cursor = 0usize;
    for (paragraph_index, paragraph) in text.split('\n').enumerate() {
        if paragraph_index > 0 {
            cursor += 1; // the '\n' that split consumed
        }
        let words: Vec<&str> = paragraph.split_whitespace().collect();
        if words.is_empty() {
            if lines.len() < max_lines {
                lines.push(String::new());
                paragraph_end.push(true);
                consumed_chars = cursor + paragraph.len();
                continue;
            } else {
                break;
            }
        }

        // Use `sum_of_word_widths + (N-1) * space_w` as the effective
        // line width.  `font.text_width(line)` folds negative cross-char
        // kerning across space boundaries into the measurement, making
        // packed lines look like they fit when rendered word-by-word
        // they would overflow.
        let space_w = font.text_width(" ").max(1);
        let mut i = 0;
        let mut last_word_end = 0usize; // byte offset (within paragraph) past the last word we packed
        while i < words.len() {
            if lines.len() >= max_lines {
                break;
            }
            let mut line = String::from(words[i]);
            let mut cur_w = font.text_width(words[i]);
            let mut j = i + 1;
            while j < words.len() {
                let next_w = font.text_width(words[j]);
                let candidate_w = cur_w + space_w + next_w;
                if candidate_w > box_w {
                    break;
                }
                line.push(' ');
                line.push_str(words[j]);
                cur_w = candidate_w;
                j += 1;
            }

            // Orphan avoidance for non-final lines: if the next line would
            // contain a single ≤5-character word, bump the last word of
            // this line down so they sit together.
            if j < words.len() && j + 1 == words.len() && words[j].len() <= 5 && j > i + 1 {
                j -= 1;
                line = words[i..j].join(" ");
            }

            let is_para_end = j == words.len();
            lines.push(line);
            paragraph_end.push(is_para_end);
            // Byte offset within the paragraph of the end of the last word
            // we just packed — needed so that when we bail early due to
            // `max_lines`, `consumed_chars` reflects actual progress, not
            // the whole paragraph.
            let last_word = words[j - 1];
            let last_word_ptr = last_word.as_ptr() as usize - paragraph.as_ptr() as usize;
            last_word_end = last_word_ptr + last_word.len();
            i = j;
        }

        // Track consumed bytes.  If we bailed out with unfinished words in
        // this paragraph, advance only up through the last packed word so
        // the remainder picks up cleanly on the next page.
        let finished_paragraph = i >= words.len();
        if finished_paragraph {
            consumed_chars = cursor + paragraph.len();
            cursor += paragraph.len();
        } else {
            consumed_chars = cursor + last_word_end;
            cursor += paragraph.len();
        }

        if lines.len() >= max_lines {
            break;
        }
    }

    let remaining = if consumed_chars >= text.len() {
        String::new()
    } else {
        text[consumed_chars..].trim_start().to_string()
    };

    WrapResult {
        remaining,
        lines,
        paragraph_end,
    }
}

/// Returns `true` if any whitespace-separated word in `text` is wider
/// than `max_w`.  Used to gate the per-character wrap fallback for
/// narrow boxes / unspaced scripts.
fn any_word_wider_than(font: &NativeFont, text: &str, max_w: i32) -> bool {
    text.split_whitespace().any(|w| font.text_width(w) > max_w)
}

/// Per-character wrap fallback for boxes too narrow to fit any word
/// (e.g. the 60-px portrait name strip), or for unspaced scripts like
/// Japanese/Chinese.  Walks `text` character by character, breaks the
/// line when the next glyph would exceed `box_w`, and clips at
/// `max_lines` — characters that don't fit are returned as the
/// `remaining` string.
pub fn wrap_text_per_char(
    font: &NativeFont,
    text: &str,
    box_w: i32,
    max_lines: usize,
) -> WrapResult {
    let mut lines: Vec<String> = Vec::new();
    let mut paragraph_end: Vec<bool> = Vec::new();
    if max_lines == 0 || box_w <= 0 {
        return WrapResult {
            remaining: text.to_string(),
            lines,
            paragraph_end,
        };
    }

    let mut current = String::new();
    let mut current_w = 0i32;
    let mut consumed_bytes = 0usize;

    let push_line = |lines: &mut Vec<String>,
                     pe: &mut Vec<bool>,
                     line: &mut String,
                     w: &mut i32,
                     is_para_end: bool| {
        lines.push(std::mem::take(line));
        pe.push(is_para_end);
        *w = 0;
    };

    let chars = text.char_indices().peekable();
    for (idx, ch) in chars {
        if ch == '\n' {
            push_line(
                &mut lines,
                &mut paragraph_end,
                &mut current,
                &mut current_w,
                true,
            );
            consumed_bytes = idx + ch.len_utf8();
            if lines.len() >= max_lines {
                break;
            }
            continue;
        }
        let mut buf = [0u8; 4];
        let cw = font.text_width(ch.encode_utf8(&mut buf));
        if current_w + cw > box_w && !current.is_empty() {
            push_line(
                &mut lines,
                &mut paragraph_end,
                &mut current,
                &mut current_w,
                false,
            );
            consumed_bytes = idx;
            if lines.len() >= max_lines {
                break;
            }
        }
        current.push(ch);
        current_w += cw;
    }

    if !current.is_empty() && lines.len() < max_lines {
        lines.push(std::mem::take(&mut current));
        paragraph_end.push(true);
        consumed_bytes = text.len();
    }

    let remaining = if consumed_bytes >= text.len() {
        String::new()
    } else {
        text[consumed_bytes..].to_string()
    };

    WrapResult {
        remaining,
        lines,
        paragraph_end,
    }
}

/// Measure the height needed to render `text` inside a box of the
/// given width and height, matching the shared wrap path used by
/// [`render_text_in_box`].
///
/// Routes through the same [`wrap_text`] call as the renderer, so the
/// measurement can't drift from what actually gets drawn: both clip to
/// `box_h / line_h` lines, both apply the same greedy wrap and orphan
/// avoidance.
pub fn measure_text_height_in_box(font: &NativeFont, text: &str, box_w: i32, box_h: i32) -> i32 {
    if text.is_empty() || box_w <= 0 || box_h <= 0 {
        return 0;
    }
    let line_h = font.height() as i32;
    if line_h <= 0 {
        return 0;
    }
    let max_lines = (box_h / line_h) as usize;
    let wrap = wrap_text(font, text, box_w, max_lines);
    wrap.lines.len() as i32 * line_h
}

pub fn measure_text_height_in_box_font(font: &Font, text: &str, box_w: i32, box_h: i32) -> i32 {
    if text.is_empty() || box_w <= 0 || box_h <= 0 {
        return 0;
    }
    let line_h = font.height() as i32;
    if line_h <= 0 {
        return 0;
    }
    let max_lines = (box_h / line_h) as usize;
    let wrap = wrap_text_for_box_font(font, text, box_w, max_lines);
    wrap.lines.len() as i32 * line_h
}

/// Render text inside a virtual box, with horizontal alignment and a
/// configurable maximum height in lines.  Returns the unrendered
/// remainder for callers implementing pagination.  Defaults to
/// `VAlign::Top`; use [`render_text_in_box_aligned`] to pick a
/// different vertical alignment.
#[allow(clippy::too_many_arguments)]
pub fn render_text_in_box(
    renderer: &mut Renderer,
    font: &NativeFont,
    transform: MenuTransform,
    text: &str,
    box_x: i32,
    box_y: i32,
    box_w: i32,
    box_h: i32,
    align: TextAlign,
) -> String {
    render_text_in_box_aligned(
        renderer,
        font,
        transform,
        text,
        box_x,
        box_y,
        box_w,
        box_h,
        align,
        VAlign::Top,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn render_text_in_box_font(
    renderer: &mut Renderer,
    font: &Font,
    transform: MenuTransform,
    text: &str,
    box_x: i32,
    box_y: i32,
    box_w: i32,
    box_h: i32,
    align: TextAlign,
) -> String {
    render_text_in_box_aligned_font(
        renderer,
        font,
        transform,
        text,
        box_x,
        box_y,
        box_w,
        box_h,
        align,
        VAlign::Top,
    )
}

/// Widget state for the popup-scroll text renderer's 4-state font
/// table.  The popup-scroll text widget is never interactive in-game
/// so only `Default` is ever hit, but the API exposes all four for
/// completeness.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TextWidgetState {
    Disabled,
    Default,
    Focused,
    Selected,
}

/// 4-font table indexed by [`TextWidgetState`].  The popup-scroll
/// builder passes the same font for all four states, so the common
/// case is to construct via [`Self::uniform`].  We store borrows so
/// callers don't have to clone the native font handles.
#[derive(Copy, Clone)]
pub struct TextFontTable<'a> {
    pub disabled: Option<&'a NativeFont>,
    pub default: Option<&'a NativeFont>,
    pub focused: Option<&'a NativeFont>,
    pub selected: Option<&'a NativeFont>,
}

impl<'a> TextFontTable<'a> {
    /// Build a table where every state resolves to the same font.
    pub fn uniform(font: Option<&'a NativeFont>) -> Self {
        Self {
            disabled: font,
            default: font,
            focused: font,
            selected: font,
        }
    }

    /// Pick the font for the given widget state, falling back to
    /// `default` when the requested slot is unset.
    pub fn pick(&self, state: TextWidgetState) -> Option<&'a NativeFont> {
        let slot = match state {
            TextWidgetState::Disabled => self.disabled,
            TextWidgetState::Default => self.default,
            TextWidgetState::Focused => self.focused,
            TextWidgetState::Selected => self.selected,
        };
        slot.or(self.default)
    }
}

/// Render text inside a box with a rectangular carve-out in the
/// top-right corner, flowing text around the reserved area in two
/// passes:
///   1. **Beside the dropped initial** — narrower box on the left
///      (`box_w - drop_cap_w`), spanning `ceil(drop_cap_h / line_h)`
///      lines.
///   2. **Below the dropped initial** — full-width continuation for
///      whatever didn't fit in pass 1.
///
/// `drop_cap_w`/`drop_cap_h` of 0 disables the carve-out and delegates
/// directly to [`render_text_in_box`].  `fonts.pick(state)` selects
/// the font for the current widget state.
///
/// Returns the unrendered remainder for pagination, identical to
/// [`render_text_in_box`].
#[allow(clippy::too_many_arguments)]
pub fn render_text_in_box_with_drop_cap(
    renderer: &mut Renderer,
    fonts: &TextFontTable<'_>,
    state: TextWidgetState,
    transform: MenuTransform,
    text: &str,
    box_x: i32,
    box_y: i32,
    box_w: i32,
    box_h: i32,
    drop_cap_w: i32,
    drop_cap_h: i32,
    align: TextAlign,
) -> String {
    let Some(font) = fonts.pick(state) else {
        return text.to_string();
    };

    if drop_cap_w <= 0 || drop_cap_h <= 0 {
        return render_text_in_box(
            renderer, font, transform, text, box_x, box_y, box_w, box_h, align,
        );
    }

    let line_h = font.height() as i32;
    if line_h <= 0 {
        return text.to_string();
    }

    // Number of lines the carve-out spans: `drop_cap_h / line_h`,
    // rounded up.  The font's own line-spacing constant is folded into
    // `line_h` (we don't have it separately) — the small discrepancy
    // of a few pixels across many lines is covered by the ceil rule.
    let mut di_lines = drop_cap_h / line_h;
    if drop_cap_h % line_h != 0 {
        di_lines += 1;
    }
    let carveout_h = (di_lines * line_h).min(box_h);
    let narrow_w = (box_w - drop_cap_w).max(0);

    // Part 1: beside the drop cap (narrower left column).  If the drop
    // cap fills the full width, skip directly to the below-cap pass —
    // clamping the drop-cap width to the box width makes the first
    // render a no-op and pushes everything into the second pass.
    let remainder = if narrow_w > 0 && carveout_h > 0 {
        render_text_in_box(
            renderer, font, transform, text, box_x, box_y, narrow_w, carveout_h, align,
        )
    } else {
        text.to_string()
    };
    // Part 2: full width, below the carve-out.
    if remainder.is_empty() {
        return String::new();
    }
    let below_y = box_y + carveout_h;
    let below_h = (box_h - carveout_h).max(0);
    if below_h == 0 {
        return remainder;
    }
    render_text_in_box(
        renderer, font, transform, &remainder, box_x, below_y, box_w, below_h, align,
    )
}

/// Polymorphic-font version of [`render_text_in_box_with_drop_cap`].
#[allow(clippy::too_many_arguments)]
pub fn render_text_in_box_with_drop_cap_font(
    renderer: &mut Renderer,
    font: &Font,
    transform: MenuTransform,
    text: &str,
    box_x: i32,
    box_y: i32,
    box_w: i32,
    box_h: i32,
    drop_cap_w: i32,
    drop_cap_h: i32,
    align: TextAlign,
) -> String {
    if drop_cap_w <= 0 || drop_cap_h <= 0 {
        return render_text_in_box_font(
            renderer, font, transform, text, box_x, box_y, box_w, box_h, align,
        );
    }

    let line_h = font.height() as i32;
    if line_h <= 0 {
        return text.to_string();
    }
    let carveout_lines = (drop_cap_h + line_h - 1) / line_h;
    let carveout_h = (carveout_lines * line_h).min(box_h);
    let narrow_w = (box_w - drop_cap_w).max(0);
    let remainder = if narrow_w > 0 && carveout_h > 0 {
        render_text_in_box_font(
            renderer, font, transform, text, box_x, box_y, narrow_w, carveout_h, align,
        )
    } else {
        text.to_string()
    };
    if remainder.is_empty() {
        return String::new();
    }
    let below_y = box_y + carveout_h;
    let below_h = (box_h - carveout_h).max(0);
    if below_h == 0 {
        return remainder;
    }
    render_text_in_box_font(
        renderer, font, transform, &remainder, box_x, below_y, box_w, below_h, align,
    )
}

/// Like [`render_text_in_box`] but with an explicit vertical alignment.
///
/// `VAlign::Center` uses the font baseline (baseline at
/// `box_y + (box_h - baseline) / 2`), which biases the visual centre
/// of the letters against the box midline.
///
/// Only the boxed branch is implemented — the early-out at `box_w <=
/// 0` returns immediately and there's no zero-width / single-point
/// anchor path.  Every Rust caller passes a real box (widget rect or
/// layout-computed region), so the degenerate "zero-width refresh
/// box, single-point anchor" path never fires.  If a future caller
/// needs single-point right/centred anchoring, add a dedicated
/// `render_text_aligned_at_point` helper rather than resurrecting
/// the missing branch here.
#[allow(clippy::too_many_arguments)]
pub fn render_text_in_box_aligned(
    renderer: &mut Renderer,
    font: &NativeFont,
    transform: MenuTransform,
    text: &str,
    box_x: i32,
    box_y: i32,
    box_w: i32,
    box_h: i32,
    align: TextAlign,
    valign: VAlign,
) -> String {
    if text.is_empty() || box_w <= 0 || box_h <= 0 {
        return String::new();
    }
    let line_h = font.height() as i32;
    if line_h <= 0 {
        return String::new();
    }
    let max_lines = (box_h / line_h) as usize;

    // Inset the text rect by the kerning margin on each side before
    // wrapping and rendering, but only when the box is wide enough to
    // absorb the inset.
    let (inner_x, inner_w) = if box_w > 2 * KERNING_MARGIN {
        (box_x + KERNING_MARGIN, box_w - 2 * KERNING_MARGIN)
    } else {
        (box_x, box_w)
    };

    // VCentered mode forces single-line layout: set the wrap budget
    // to the full text width so the greedy wrap pass emits one
    // (overflowing) line per paragraph.
    let wrap_w = match valign {
        VAlign::Center => font.text_width(text).max(inner_w),
        VAlign::Top => inner_w,
    };
    // Per-character wrap fallback when any whole word is too wide for
    // the box.  Only applies to greedy word-wrap (VAlign::Top) —
    // VCentered forces a single line so the word-overflow doesn't
    // matter.
    let wrap = if valign == VAlign::Top && any_word_wider_than(font, text, inner_w) {
        wrap_text_per_char(font, text, inner_w, max_lines)
    } else {
        wrap_text(font, text, wrap_w, max_lines)
    };

    let baseline = font.baseline() as i32;
    // For top-origin rendering, `y` is the top of the glyph row.
    // Centring is anchored on the baseline, so translate through:
    // baseline_y = box_y + (box_h - baseline) / 2
    // glyph_top_y = baseline_y - baseline
    let first_line_y = match valign {
        VAlign::Top => box_y,
        VAlign::Center if box_h > line_h => {
            let total_h = wrap.lines.len() as i32 * line_h;
            // Centre the text block baseline-wise: the first line's top
            // sits such that its baseline lands at (box_h - baseline)/2.
            let baseline_in_box = (box_h - baseline) / 2;
            let first_baseline_y = box_y + baseline_in_box;
            let first_top = first_baseline_y - baseline;
            // Clamp: multi-line text that overflows the box reverts to
            // top alignment so we never render above `box_y`.
            if total_h >= box_h {
                box_y
            } else {
                first_top.max(box_y)
            }
        }
        VAlign::Center => box_y,
    };

    let mut y = first_line_y;
    for (idx, line) in wrap.lines.iter().enumerate() {
        let tw = font.text_width(line);
        let is_para_end = wrap.paragraph_end.get(idx).copied().unwrap_or(true);
        match align {
            TextAlign::Left => {
                render_text_virt(renderer, font, transform, line, inner_x, y);
            }
            TextAlign::Center => {
                render_text_virt(
                    renderer,
                    font,
                    transform,
                    line,
                    inner_x + (inner_w - tw) / 2,
                    y,
                );
            }
            TextAlign::Right => {
                render_text_virt(renderer, font, transform, line, inner_x + inner_w - tw, y);
            }
            TextAlign::Justified => {
                render_justified_line(
                    renderer,
                    font,
                    transform,
                    line,
                    inner_x,
                    y,
                    inner_w,
                    tw,
                    is_para_end,
                );
            }
        }
        y += line_h;
    }
    wrap.remaining
}

#[allow(clippy::too_many_arguments)]
pub fn render_text_in_box_aligned_font(
    renderer: &mut Renderer,
    font: &Font,
    transform: MenuTransform,
    text: &str,
    box_x: i32,
    box_y: i32,
    box_w: i32,
    box_h: i32,
    align: TextAlign,
    valign: VAlign,
) -> String {
    if text.is_empty() || box_w <= 0 || box_h <= 0 {
        return String::new();
    }
    let line_h = font.height() as i32;
    if line_h <= 0 {
        return String::new();
    }
    let max_lines = (box_h / line_h) as usize;
    let (inner_x, inner_w) = if box_w > 2 * KERNING_MARGIN {
        (box_x + KERNING_MARGIN, box_w - 2 * KERNING_MARGIN)
    } else {
        (box_x, box_w)
    };
    let wrap_w = match valign {
        VAlign::Center => font.text_width(text).max(inner_w),
        VAlign::Top => inner_w,
    };
    let wrap = if valign == VAlign::Top && any_word_wider_than_font(font, text, inner_w) {
        wrap_text_per_char_font(font, text, inner_w, max_lines)
    } else {
        wrap_text_font(font, text, wrap_w, max_lines)
    };
    let baseline = font.baseline() as i32;
    let first_line_y = match valign {
        VAlign::Top => box_y,
        VAlign::Center if box_h > line_h => {
            let total_h = wrap.lines.len() as i32 * line_h;
            let baseline_in_box = (box_h - baseline) / 2;
            let first_baseline_y = box_y + baseline_in_box;
            let first_top = first_baseline_y - baseline;
            if total_h >= box_h {
                box_y
            } else {
                first_top.max(box_y)
            }
        }
        VAlign::Center => box_y,
    };

    let mut y = first_line_y;
    for (idx, line) in wrap.lines.iter().enumerate() {
        let tw = font.text_width(line);
        let is_para_end = wrap.paragraph_end.get(idx).copied().unwrap_or(true);
        match align {
            TextAlign::Left => render_text_virt_font(renderer, font, transform, line, inner_x, y),
            TextAlign::Center => render_text_virt_font(
                renderer,
                font,
                transform,
                line,
                inner_x + (inner_w - tw) / 2,
                y,
            ),
            TextAlign::Right => {
                render_text_virt_font(renderer, font, transform, line, inner_x + inner_w - tw, y)
            }
            TextAlign::Justified => render_justified_line_font(
                renderer,
                font,
                transform,
                line,
                inner_x,
                y,
                inner_w,
                is_para_end,
            ),
        }
        y += line_h;
    }
    wrap.remaining
}

pub fn wrap_text_font(font: &Font, text: &str, box_w: i32, max_lines: usize) -> WrapResult {
    let mut lines: Vec<String> = Vec::new();
    let mut paragraph_end: Vec<bool> = Vec::new();
    let mut consumed_chars = 0usize;
    if max_lines == 0 || box_w <= 0 {
        return WrapResult {
            remaining: text.to_string(),
            lines,
            paragraph_end,
        };
    }
    let mut cursor = 0usize;
    for (paragraph_index, paragraph) in text.split('\n').enumerate() {
        if paragraph_index > 0 {
            cursor += 1;
        }
        let words: Vec<&str> = paragraph.split_whitespace().collect();
        if words.is_empty() {
            if lines.len() < max_lines {
                lines.push(String::new());
                paragraph_end.push(true);
                consumed_chars = cursor + paragraph.len();
                continue;
            }
            break;
        }
        let space_w = font.text_width(" ").max(1);
        let mut i = 0;
        let mut last_word_end = 0usize;
        while i < words.len() {
            if lines.len() >= max_lines {
                break;
            }
            let mut line = String::from(words[i]);
            let mut cur_w = font.text_width(words[i]);
            let mut j = i + 1;
            while j < words.len() {
                let next_w = font.text_width(words[j]);
                let candidate_w = cur_w + space_w + next_w;
                if candidate_w > box_w {
                    break;
                }
                line.push(' ');
                line.push_str(words[j]);
                cur_w = candidate_w;
                j += 1;
            }
            if j < words.len() && j + 1 == words.len() && words[j].len() <= 5 && j > i + 1 {
                j -= 1;
                line = words[i..j].join(" ");
            }
            let is_para_end = j == words.len();
            lines.push(line);
            paragraph_end.push(is_para_end);
            let last_word = words[j - 1];
            let last_word_ptr = last_word.as_ptr() as usize - paragraph.as_ptr() as usize;
            last_word_end = last_word_ptr + last_word.len();
            i = j;
        }
        if i >= words.len() {
            consumed_chars = cursor + paragraph.len();
            cursor += paragraph.len();
        } else {
            consumed_chars = cursor + last_word_end;
            cursor += paragraph.len();
        }
        if lines.len() >= max_lines {
            break;
        }
    }
    let remaining = if consumed_chars >= text.len() {
        String::new()
    } else {
        text[consumed_chars..].trim_start().to_string()
    };
    WrapResult {
        remaining,
        lines,
        paragraph_end,
    }
}

/// Wrap polymorphic-font text for a bounded box, falling back to glyph-level
/// breaks when the locale uses an unspaced script or a single word cannot fit.
pub fn wrap_text_for_box_font(font: &Font, text: &str, box_w: i32, max_lines: usize) -> WrapResult {
    if any_word_wider_than_font(font, text, box_w) {
        wrap_text_per_char_font(font, text, box_w, max_lines)
    } else {
        wrap_text_font(font, text, box_w, max_lines)
    }
}

fn any_word_wider_than_font(font: &Font, text: &str, max_w: i32) -> bool {
    text.split_whitespace()
        .any(|word| font.text_width(word) > max_w)
}

/// Polymorphic counterpart to [`wrap_text_per_char`]. This is required
/// for locale-selected TrueType fonts because Japanese, Chinese, and
/// other unspaced scripts otherwise become one overflowing "word".
pub fn wrap_text_per_char_font(
    font: &Font,
    text: &str,
    box_w: i32,
    max_lines: usize,
) -> WrapResult {
    let mut lines = Vec::new();
    let mut paragraph_end = Vec::new();
    if max_lines == 0 || box_w <= 0 {
        return WrapResult {
            remaining: text.to_string(),
            lines,
            paragraph_end,
        };
    }

    let mut current = String::new();
    let mut current_w = 0i32;
    let mut consumed_bytes = 0usize;
    let push_line = |lines: &mut Vec<String>,
                     paragraph_end: &mut Vec<bool>,
                     line: &mut String,
                     width: &mut i32,
                     is_paragraph_end: bool| {
        lines.push(std::mem::take(line));
        paragraph_end.push(is_paragraph_end);
        *width = 0;
    };

    for (idx, ch) in text.char_indices() {
        if ch == '\n' {
            push_line(
                &mut lines,
                &mut paragraph_end,
                &mut current,
                &mut current_w,
                true,
            );
            consumed_bytes = idx + ch.len_utf8();
            if lines.len() >= max_lines {
                break;
            }
            continue;
        }

        let mut encoded = [0u8; 4];
        let char_w = font.text_width(ch.encode_utf8(&mut encoded));
        if current_w + char_w > box_w && !current.is_empty() {
            push_line(
                &mut lines,
                &mut paragraph_end,
                &mut current,
                &mut current_w,
                false,
            );
            consumed_bytes = idx;
            if lines.len() >= max_lines {
                break;
            }
        }
        current.push(ch);
        current_w += char_w;
    }

    if !current.is_empty() && lines.len() < max_lines {
        lines.push(std::mem::take(&mut current));
        paragraph_end.push(true);
        consumed_bytes = text.len();
    }

    let remaining = if consumed_bytes >= text.len() {
        String::new()
    } else {
        text[consumed_bytes..].to_string()
    };
    WrapResult {
        remaining,
        lines,
        paragraph_end,
    }
}

/// Render a single wrapped line with full justification: expand
/// inter-word spaces to fill the box width, leave the final line of
/// each paragraph un-justified, and snap the last word to the right
/// box edge so the line fills perfectly.
///
/// Justification is gated on word-count ≥ 2 and `!paragraph_end`.
/// Because our wrapper collapses whitespace at wrap time, "manual
/// break" collapses to "last line of paragraph" (`paragraph_end =
/// true`).
///
/// Gap math: slack is divided by `word_count` (not gap-count),
/// leftover 1-pixels go to the first `slack % word_count` gaps, and
/// the final word is snapped to `box_x + box_w - last_word_width` so
/// the line fills exactly.
#[allow(clippy::too_many_arguments)]
fn render_justified_line(
    renderer: &mut Renderer,
    font: &NativeFont,
    transform: MenuTransform,
    line: &str,
    box_x: i32,
    y: i32,
    box_w: i32,
    line_w: i32,
    is_paragraph_end: bool,
) {
    let _ = line_w;
    let words: Vec<&str> = line.split_whitespace().collect();
    let space_w = font.text_width(" ").max(1);
    let word_count = words.len() as i32;
    // Use the wrap-time line-width formula: sum of word widths + (N-1)
    // single spaces.  `font.text_width(line)` includes inter-character
    // kerning across space boundaries which can be negative, making the
    // measured line width smaller than what's actually rendered when we
    // emit each word with a plain `space_w` gap — causing the snapped
    // last word to overlap the previous one.
    let sum_word_w: i32 = words.iter().map(|w| font.text_width(w)).sum();
    let effective_line_w = if word_count > 0 {
        sum_word_w + (word_count - 1) * space_w
    } else {
        0
    };
    if words.len() < 2 || is_paragraph_end || effective_line_w >= box_w {
        render_text_virt(renderer, font, transform, line, box_x, y);
        return;
    }

    let slack = box_w - effective_line_w;
    let extra_per_gap = slack / word_count;
    let mut leftover = slack % word_count;

    let last_idx = words.len() - 1;
    let last_word_w = font.text_width(words[last_idx]);

    let mut x = box_x;
    for (i, word) in words.iter().enumerate() {
        if i == last_idx {
            // Snap last word to the right edge.
            render_text_virt(
                renderer,
                font,
                transform,
                word,
                box_x + box_w - last_word_w,
                y,
            );
        } else {
            render_text_virt(renderer, font, transform, word, x, y);
            let mut gap = space_w + extra_per_gap;
            if leftover > 0 {
                gap += 1;
                leftover -= 1;
            }
            x += font.text_width(word) + gap;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_justified_line_font(
    renderer: &mut Renderer,
    font: &Font,
    transform: MenuTransform,
    line: &str,
    box_x: i32,
    y: i32,
    box_w: i32,
    is_paragraph_end: bool,
) {
    let words: Vec<&str> = line.split_whitespace().collect();
    let space_w = font.text_width(" ").max(1);
    let word_count = words.len() as i32;
    let sum_word_w: i32 = words.iter().map(|w| font.text_width(w)).sum();
    let effective_line_w = if word_count > 0 {
        sum_word_w + (word_count - 1) * space_w
    } else {
        0
    };
    if words.len() < 2 || is_paragraph_end || effective_line_w >= box_w {
        render_text_virt_font(renderer, font, transform, line, box_x, y);
        return;
    }

    let slack = box_w - effective_line_w;
    let extra_per_gap = slack / word_count;
    let mut leftover = slack % word_count;
    let last_idx = words.len() - 1;
    let last_word_w = font.text_width(words[last_idx]);
    let mut x = box_x;
    for (i, word) in words.iter().enumerate() {
        if i == last_idx {
            render_text_virt_font(
                renderer,
                font,
                transform,
                word,
                box_x + box_w - last_word_w,
                y,
            );
        } else {
            render_text_virt_font(renderer, font, transform, word, x, y);
            let mut gap = space_w + extra_per_gap;
            if leftover > 0 {
                gap += 1;
                leftover -= 1;
            }
            x += font.text_width(word) + gap;
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// Hover-tooltip helper
// ═══════════════════════════════════════════════════════════════════

/// Hover-delay before a widget's tooltip text appears (~750 ms of
/// wall-clock idle hover, matching the original 75-tick / ~10 ms-per-
/// tick menu cadence).
pub const TOOLTIP_HOVER_DELAY: std::time::Duration = std::time::Duration::from_millis(750);

/// Wall-clock hover tracker for a single modal's `FrameWnd`.
///
/// Call [`Self::update`] once per frame after input processing to
/// retarget the tracked widget, then [`Self::draw`] during the render
/// pass to paint the tooltip near the cursor once the delay has
/// elapsed.  Both steps are no-ops when no widget is currently
/// hovered or the hover hasn't crossed the delay yet.
///
/// Exists because the Rust port doesn't have a single shared hover-
/// tooltip pipeline — every modal drives its own event loop and wires
/// its own instance of this tracker into that loop, so per-loop
/// tracking is the simplest analogue.
#[derive(Default)]
pub struct TooltipState {
    hover_widget: Option<crate::widget::WidgetId>,
    hover_since: Option<web_time::Instant>,
}

impl TooltipState {
    /// Fresh tracker with no widget hovered.
    pub fn new() -> Self {
        Self::default()
    }

    /// The widget currently tracked as hovered, if any.  Exposed so
    /// callers that layer additional hover-driven tooltips (e.g. the
    /// blazon-set grid) can suppress their own tooltip when a widget
    /// is already showing one.
    pub fn hover_widget(&self) -> Option<crate::widget::WidgetId> {
        self.hover_widget
    }

    /// Re-scan `frame` for the widget the cursor is over.  If the
    /// hover target changed, reset the idle timer.
    pub fn update(
        &mut self,
        frame: &crate::widget::FrameWnd,
        mouse_virt: engine_coordinates::ScreenPoint,
    ) {
        let hovered_now = frame
            .widgets()
            .iter()
            .find(|w| w.base().is_inside(mouse_virt) && w.base().has_tooltip())
            .map(|w| w.id());
        if hovered_now != self.hover_widget {
            self.hover_widget = hovered_now;
            self.hover_since = hovered_now.map(|_| web_time::Instant::now());
        }
    }

    /// Paint the tooltip for the currently-hovered widget, if any, and
    /// if the hover has been idle long enough.  No-op otherwise.
    pub fn draw(
        &self,
        renderer: &mut Renderer,
        font: &Font,
        transform: MenuTransform,
        frame: &crate::widget::FrameWnd,
        mouse_virt: engine_coordinates::ScreenPoint,
    ) {
        let Some(id) = self.hover_widget else { return };
        let Some(started) = self.hover_since else {
            return;
        };
        if started.elapsed() < TOOLTIP_HOVER_DELAY {
            return;
        }
        let Some(w) = frame.widget(id) else { return };
        draw_tooltip(
            renderer,
            font,
            transform,
            &w.base().tooltip_text,
            mouse_virt.x as i32,
            mouse_virt.y as i32,
        );
    }
}

/// Draw a tooltip string next to the cursor.
///
/// The tooltip flips to the left/above the cursor when it would
/// overflow the 640x480 virtual menu screen.
pub fn draw_tooltip(
    renderer: &mut Renderer,
    font: &Font,
    transform: MenuTransform,
    text: &str,
    mouse_virt_x: i32,
    mouse_virt_y: i32,
) {
    if text.is_empty() {
        return;
    }
    // Tooltip padding + anchor offset from the cursor hotspot
    // (virtual units).  Offset down-right so the box doesn't obscure
    // the target widget.
    const PAD_X: i32 = 4;
    const PAD_Y: i32 = 2;
    const CURSOR_OFFSET_X: i32 = 12;
    const CURSOR_OFFSET_Y: i32 = 16;

    let single_line_tw = font.text_width(text);
    let line_h = font.height() as i32;
    let max_box_w = MENU_W - 2 * PAD_X;

    // Multi-line tooltip strings (hard `\n` or wider than the screen)
    // get a wrapping fill rect: detect the wrap-needed case, run a
    // wrap pass, then size the box to the widest line and the line
    // count.
    let needs_wrap = text.contains('\n') || single_line_tw + 2 * PAD_X > MENU_W;
    let (lines, longest_line_w) = if needs_wrap {
        let wrap_w = (max_box_w).max(line_h);
        let wrap = wrap_text_for_box_font(font, text, wrap_w, usize::MAX);
        let widest = wrap
            .lines
            .iter()
            .map(|l| font.text_width(l))
            .max()
            .unwrap_or(0);
        (wrap.lines, widest)
    } else {
        (vec![text.to_string()], single_line_tw)
    };

    let line_count = lines.len() as i32;
    let box_w = longest_line_w + PAD_X * 2;
    let box_h = line_count * line_h + PAD_Y * 2;

    let mut box_x = mouse_virt_x + CURSOR_OFFSET_X;
    let mut box_y = mouse_virt_y + CURSOR_OFFSET_Y;
    if box_x + box_w > MENU_W {
        box_x = (mouse_virt_x - CURSOR_OFFSET_X - box_w).max(0);
    }
    if box_y + box_h > MENU_H {
        box_y = (mouse_virt_y - CURSOR_OFFSET_Y - box_h).max(0);
    }

    for (idx, line) in lines.iter().enumerate() {
        let line_x = box_x + PAD_X;
        let line_y = box_y + PAD_Y + idx as i32 * line_h;
        let (tx, ty) = transform.to_screen(line_x, line_y);
        render_text_screen_font(renderer, font, line, tx, ty);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_round_trip() {
        let t = MenuTransform::centered(1024, 768);
        let (sx, sy) = t.to_screen(123, 456);
        assert_eq!(t.from_screen(sx, sy), (123, 456));
    }

    #[test]
    fn align_bottom_right_stacks_buttons() {
        let labels: &[(&str, bool)] = &[("A", true), ("B", true), ("C", true)];
        let btns = align_bottom_right(labels, 100, 30);
        assert_eq!(btns.len(), 3);
        for b in &btns {
            assert_eq!(b.x + b.w, MENU_W);
        }
        let last = btns.last().unwrap();
        assert_eq!(last.y + last.h, MENU_H);
        assert_eq!(btns[1].y - (btns[0].y + btns[0].h), BUTTON_SPACING);
    }

    #[test]
    fn center_horizontally_centers_row() {
        let labels: &[(&str, bool)] = &[("A", true), ("B", true)];
        let btns = center_horizontally(labels, 100, 30, 10, 200);
        assert_eq!(btns.len(), 2);
        let row_left = btns[0].x;
        let row_right = btns[1].x + btns[1].w;
        assert_eq!(row_left + (row_right - row_left) / 2, MENU_W / 2);
        assert_eq!(btns[0].y, 200);
        assert_eq!(btns[1].y, 200);
    }

    #[test]
    fn align_on_first_widget_stacks_vertically() {
        let mut btns = vec![
            MenuButton {
                label: "A".into(),
                enabled: true,
                x: 10,
                y: 100,
                w: 80,
                h: 20,
            },
            MenuButton {
                label: "B".into(),
                enabled: true,
                x: 50,
                y: 0,
                w: 80,
                h: 20,
            },
        ];
        align_on_first_widget(&mut btns, 5);
        assert_eq!(btns[0].y, 100);
        assert_eq!(btns[1].y, 125);
        assert_eq!(btns[0].x, btns[1].x);
    }

    // The wrap helper is exercised by higher-level debriefing tests
    // (see debriefing.rs) because it needs a real NativeFont to compute
    // text width — the logic itself is straightforward enough to skip a
    // fake-font unit test here.
}
