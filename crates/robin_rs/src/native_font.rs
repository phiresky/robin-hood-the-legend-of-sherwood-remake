//! Native bitmap font loading and rendering.
//!
//! Loads `.sbf` font files containing bitmap glyphs with per-pixel
//! alpha, and renders text directly to 16-bit RGB565 pixel buffers.

use robin_engine::sbfile as engine_sbfile;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::font::TrueTypeFont;
use crate::sbfile::SbFile;
use robin_assets::picture::{Picture, read_i32, read_u16, read_u32};

const TAG_LEN: usize = 6;
const SBFONT_TAG: &[u8; TAG_LEN] = b"SBFONT";
const FONT_NAME_LEN: usize = 32;

/// Font file directory.
const FONT_PATH: &str = "Data/Interface/Fonts/";

/// Per-character glyph metrics.
#[derive(Debug, Clone)]
struct CharacterInfo {
    /// Horizontal pixel offset of the glyph in the atlas picture.
    start: u32,
    /// Width of the glyph in pixels.
    width: u32,
    /// Extra space before the glyph (left kerning).
    pre_spacing: i32,
    /// Extra space after the glyph (right kerning).
    post_spacing: i32,
}

/// A native bitmap font loaded from a `.sbf` file.
///
/// The glyph atlas is a single wide RGB565 picture with all character glyphs
/// laid out horizontally. A matching alpha picture controls transparency
/// (0 = transparent, non-zero = opaque).
/// One glyph's destination rect + UV rect into the font atlas, as
/// returned by [`NativeFont::layout_quads`]. The renderer turns each
/// quad into a `QueuedDraw` against the cached font-atlas texture.
#[derive(Clone, Copy, Debug)]
pub struct TextQuad {
    pub dst_x: i32,
    pub dst_y: i32,
    pub dst_w: u32,
    pub dst_h: u32,
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
}

pub struct NativeFont {
    /// Display name from the `.sbf` `FONT_HEADER`. Used in the
    /// missing-glyph diagnostic.
    name: String,
    height: u32,
    /// Baseline distance from the top of the glyph cell, in pixels.
    /// Read from the `.sbf` header. Used by text renderers to vertically
    /// centre text — `y = box_y + (box_h - baseline) / 2` biases the
    /// visual centre of letters (ignoring descenders) against the box
    /// midline.
    baseline: u32,
    extra_spacing: i32,
    characters: HashMap<u16, CharacterInfo>,
    /// Glyph atlas pixels (RGB565, row-major, width = `glyph_width`).
    glyph_pixels: Vec<u16>,
    glyph_width: u16,
    /// Alpha channel pixels (RGB565 values; 0 = transparent).
    alpha_pixels: Vec<u16>,
    alpha_width: u16,
    /// Per-instance set of already-warned-about missing glyphs, so the
    /// warning fires once per `(font, char)` pair instead of per call.
    missing_chars_warned: Mutex<HashSet<u16>>,
}

impl NativeFont {
    /// Load a native font from a `.sbf` file path.
    ///
    /// The path is resolved through `SbFile` (supports alternate data dirs
    /// and case-insensitive lookup).
    pub fn load(path: &str) -> Result<Self> {
        let mut file = SbFile::open(path, 0)
            .map_err(|e| anyhow::anyhow!("cannot open font '{}': error {}", path, e))?;

        // ── File header ─────────────────────────────────────────────
        let mut tag = [0u8; TAG_LEN];
        file.serialize_bytes(&mut tag)
            .map_err(|e| anyhow::anyhow!("read tag: {e}"))?;
        if &tag != SBFONT_TAG {
            bail!(
                "bad font tag: expected SBFONT, got {:?}",
                std::str::from_utf8(&tag).unwrap_or("???")
            );
        }
        let version = read_u32(&mut file)?;

        // ── FONT_HEADER ─────────────────────────────────────────────
        let name = {
            let mut buf = [0u8; FONT_NAME_LEN];
            file.serialize_bytes(&mut buf)
                .map_err(|e| anyhow::anyhow!("read name: {e}"))?;
            let len = buf.iter().position(|&b| b == 0).unwrap_or(FONT_NAME_LEN);
            String::from_utf8_lossy(&buf[..len]).to_string()
        };
        let _flags = read_u32(&mut file)?;
        let _styles = read_u32(&mut file)?;
        let height = read_u32(&mut file)?;

        // ── NATIVE_FONT_HEADER ──────────────────────────────────────
        let _char_cell_width = read_u32(&mut file)?;
        let baseline = read_u32(&mut file)?;
        let char_number = read_u32(&mut file)?;

        // ── SPACING_FONT_HEADER (version == 0x0200) ─────────────────
        // The gate is an exact-match on `0x0200`, not `>=`.
        let extra_spacing = if version == 0x0200 {
            read_i32(&mut file)?
        } else {
            0
        };

        // ── Character info map ──────────────────────────────────────
        let mut characters = HashMap::with_capacity(char_number as usize);
        for _ in 0..char_number {
            let char_code = read_u16(&mut file)?;
            let start = read_u32(&mut file)?;
            let width = read_u32(&mut file)?;
            let pre_spacing = read_i32(&mut file)?;
            let post_spacing = read_i32(&mut file)?;
            characters.insert(
                char_code,
                CharacterInfo {
                    start,
                    width,
                    pre_spacing,
                    post_spacing,
                },
            );
        }

        // ── Glyph picture (16-bit) ──────────────────────────────────
        let glyph_pic = Picture::load_sixteen_from_stream(&mut file).context("glyph picture")?;
        let glyph_width = glyph_pic.width;
        let glyph_pixels = bytes_to_u16(&glyph_pic.data);

        // ── Alpha picture (16-bit) ──────────────────────────────────
        let alpha_pic = Picture::load_sixteen_from_stream(&mut file).context("alpha picture")?;
        let alpha_width = alpha_pic.width;
        let alpha_pixels = bytes_to_u16(&alpha_pic.data);

        tracing::info!(
            "Loaded native font '{}': {}px, {} chars, glyph={}x{}, alpha={}x{}",
            name,
            height,
            characters.len(),
            glyph_width,
            glyph_pic.height,
            alpha_width,
            alpha_pic.height
        );

        Ok(Self {
            name,
            height,
            baseline,
            extra_spacing,
            characters,
            glyph_pixels,
            glyph_width,
            alpha_pixels,
            alpha_width,
            missing_chars_warned: Mutex::new(HashSet::new()),
        })
    }

    /// Font height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Baseline distance from the top of the glyph cell, in pixels.
    pub fn baseline(&self) -> u32 {
        self.baseline
    }

    /// Bake the entire glyph atlas + alpha mask into a single
    /// RGBA8 buffer for upload to a GPU texture. Used by the
    /// renderer's `font_atlas_cache` so text rendering only needs
    /// one texture upload per font (per session) and per-string
    /// rendering becomes a fan of UV-mapped quads instead of a
    /// fresh upload per label.
    ///
    /// Each pixel:
    /// - RGB ← `glyph_pixels[i]` decoded from RGB565
    /// - A   ← `(alpha_pixels[i] & 0x1F) << 3` — blue-channel alpha
    ///   extraction.
    /// - Glyph pixels equal to the green colour key (`0x07C0`) get
    ///   alpha=0 regardless of the alpha mask.
    ///
    /// Returns `(rgba, width, height)` where width is the glyph
    /// atlas width and height is the font height (atlases are a
    /// single horizontal strip).
    pub fn build_rgba_atlas(&self) -> (Vec<u8>, u32, u32) {
        let w = self.glyph_width as usize;
        let h = self.height as usize;
        let aw = self.alpha_width as usize;
        let mut out = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let glyph_idx = y * w + x;
                let glyph_px = self.glyph_pixels[glyph_idx];
                if glyph_px == 0x07C0 {
                    continue;
                }
                let alpha_idx = y * aw + x;
                let alpha_px = if alpha_idx < self.alpha_pixels.len() {
                    self.alpha_pixels[alpha_idx]
                } else {
                    0
                };
                let alpha = ((alpha_px & 0x1F) << 3) as u8;
                let r = ((glyph_px >> 8) & 0xF8) as u8;
                let g = ((glyph_px >> 3) & 0xFC) as u8;
                let b = ((glyph_px << 3) & 0xF8) as u8;
                let dst = glyph_idx * 4;
                out[dst] = r;
                out[dst + 1] = g;
                out[dst + 2] = b;
                out[dst + 3] = alpha;
            }
        }
        (out, self.glyph_width as u32, self.height)
    }

    /// Lay out `text` at `(x, y)` and return one `TextQuad` per
    /// glyph — destination rect in pixel space + UV rect in 0..1
    /// atlas coordinates. Uses the same per-character spacing logic
    /// as `text_width` / `render_to_argb`.
    pub fn layout_quads(&self, text: &str, x: i32, y: i32) -> Vec<TextQuad> {
        let mut out = Vec::new();
        let aw = self.glyph_width as f32;
        let mut cx = x;
        for ch in text.encode_utf16() {
            if let Some(info) = self.get_char_info(ch) {
                cx += info.pre_spacing;
                let u0 = info.start as f32 / aw;
                let u1 = (info.start + info.width) as f32 / aw;
                if ch != b' ' as u16 {
                    out.push(TextQuad {
                        dst_x: cx,
                        dst_y: y,
                        dst_w: info.width,
                        dst_h: self.height,
                        u0,
                        v0: 0.0,
                        u1,
                        v1: 1.0,
                    });
                }
                cx += info.width as i32 + info.post_spacing + self.extra_spacing;
            }
        }
        out
    }

    /// Look up character info for a glyph. Returns `None` when the glyph
    /// is absent — callers must skip the glyph entirely. Logs a one-shot
    /// warning per `(font, char)` pair.
    fn get_char_info(&self, ch: u16) -> Option<&CharacterInfo> {
        if let Some(info) = self.characters.get(&ch) {
            return Some(info);
        }
        if let Ok(mut warned) = self.missing_chars_warned.lock()
            && warned.insert(ch)
        {
            tracing::warn!(
                "Character 0x{:04x} is missing in the font '{}'",
                ch,
                self.name
            );
        }
        None
    }

    /// Width of a single character including kerning: returns
    /// `pre_spacing + width + post_spacing`. Missing glyphs return 0.
    ///
    /// Does NOT include `extra_spacing()` — callers that need per-char
    /// pixel advances (e.g. `WidgetInputField::get_text_from_caret`)
    /// must add `extra_spacing()` themselves.
    pub fn character_width(&self, ch: char) -> u32 {
        let code = ch as u32;
        if code > u16::MAX as u32 {
            return 0;
        }
        let Some(info) = self.get_char_info(code as u16) else {
            return 0;
        };
        (info.pre_spacing + info.width as i32 + info.post_spacing).max(0) as u32
    }

    /// Inter-character spacing constant from the `SPACING_FONT_HEADER`.
    pub fn extra_spacing(&self) -> i32 {
        self.extra_spacing
    }

    /// Compute the rendered pixel width of a text string.
    ///
    /// `extra_spacing` is added once per character (not once per gap),
    /// so the cumulative extra-spacing contribution for an N-char
    /// string is `N * extra_spacing`. Missing glyphs contribute 0.
    pub fn text_width(&self, text: &str) -> i32 {
        let mut w = 0i32;
        for ch in text.encode_utf16() {
            if let Some(info) = self.get_char_info(ch) {
                w += info.pre_spacing + info.width as i32 + info.post_spacing + self.extra_spacing;
            }
        }
        w
    }
}

/// Convert a `&[u8]` of little-endian u16 pixel data to `Vec<u16>`.
fn bytes_to_u16(data: &[u8]) -> Vec<u16> {
    data.chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

// ─── Font config (manager.cfg) ────────────────────────────────────

/// One entry from `manager.cfg` — the two filename columns per key.
///
/// Stores both the native `.sbf` filename and the TrueType `.tfn`
/// descriptor filename. Either column may be empty, but not both.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FontEntry {
    pub native: Option<String>,
    pub truetype: Option<String>,
}

/// A font loaded through `load_font_by_name` — either the native
/// bitmap font or a TrueType descriptor.
pub enum Font {
    Native(NativeFont),
    TrueType(TrueTypeFont),
}

impl Font {
    /// True when the font can actually render glyphs.
    pub fn is_renderable(&self) -> bool {
        match self {
            Font::Native(_) => true,
            Font::TrueType(f) => f.is_valid() && f.has_loaded_face(),
        }
    }

    /// Font height in pixels.
    pub fn height(&self) -> u32 {
        match self {
            Font::Native(f) => f.height(),
            Font::TrueType(f) => f.get_height(),
        }
    }

    /// Baseline distance from the top of the glyph cell, in pixels.
    pub fn baseline(&self) -> u32 {
        match self {
            Font::Native(f) => f.baseline(),
            Font::TrueType(f) => f.get_baseline(),
        }
    }

    /// Rendered pixel width of `text`.
    pub fn text_width(&self, text: &str) -> i32 {
        match self {
            Font::Native(f) => f.text_width(text),
            Font::TrueType(f) => {
                let chars: Vec<u32> = text.chars().map(|c| c as u32).collect();
                f.get_string_width_total(&chars)
            }
        }
    }

    /// Width of a single character (with kerning, without `extra_spacing`).
    /// Missing glyphs return 0.
    pub fn character_width(&self, ch: char) -> u32 {
        match self {
            Font::Native(f) => f.character_width(ch),
            Font::TrueType(f) => f.get_char_width_total(ch as u32),
        }
    }

    /// Inter-character spacing constant.
    /// TrueType fonts always return 0 (matching `font.rs::get_extra_spacing`).
    pub fn extra_spacing(&self) -> i32 {
        match self {
            Font::Native(f) => f.extra_spacing(),
            Font::TrueType(f) => f.get_extra_spacing(),
        }
    }
}

/// Parse `Data/Interface/Fonts/manager.cfg` and return a map of
/// font key name → `FontEntry`.
///
/// Config format (one entry per line):
/// ```text
/// KeyName: NativeFont.sbf, TrueTypeFont.tfn
/// ```
/// Either column may be empty — e.g. the `ListDefault` entry ships only
/// a `.tfn` filename and relies on the TrueType fallback in
/// [`load_font_by_name`].
pub fn load_font_config() -> Result<HashMap<String, FontEntry>> {
    let config_path = format!("{FONT_PATH}manager.cfg");
    let mut file = SbFile::open(&config_path, 0)
        .map_err(|e| anyhow::anyhow!("cannot open '{}': error {}", config_path, e))?;

    let size = file.get_size() as usize;
    let mut buf = vec![0u8; size];
    file.serialize_bytes(&mut buf)
        .map_err(|e| anyhow::anyhow!("read config: {e}"))?;

    let text = String::from_utf8_lossy(&buf);
    let result = parse_font_config(&text);
    tracing::info!("Font config: {} entries", result.len());
    Ok(result)
}

/// Parse `manager.cfg` text into a key → `FontEntry` map.
///
/// Separated from [`load_font_config`] so config parsing can be tested
/// without a datadir.
fn parse_font_config(text: &str) -> HashMap<String, FontEntry> {
    let mut result = HashMap::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();

        let (native, truetype) = match rest.split_once(',') {
            Some((n, t)) => (n.trim(), t.trim()),
            None => (rest.trim(), ""),
        };

        let entry = FontEntry {
            native: (!native.is_empty()).then(|| native.to_string()),
            truetype: (!truetype.is_empty()).then(|| truetype.to_string()),
        };

        if entry.native.is_none() && entry.truetype.is_none() {
            continue;
        }
        result.insert(key.to_string(), entry);
    }

    result
}

/// Load a font by its config key (e.g. "MenuButtonEnabled").
///
/// Tries the native `.sbf` first and falls back to the TrueType `.tfn`
/// descriptor on failure.
///
/// Panics if the key is present in the config but both filename
/// columns are absent (project rule: never silently substitute data).
/// Returns `Err` if the key is missing, or if both load attempts fail
/// — callers can chain `.or_else` on a secondary key, as the HUD font
/// loader does.
pub fn load_font_by_name(config: &HashMap<String, FontEntry>, name: &str) -> Result<Font> {
    let Some(entry) = config.get(name) else {
        bail!("font '{}' not in config", name);
    };

    if entry.native.is_none() && entry.truetype.is_none() {
        panic!("no font for key '{name}' (both native and TrueType columns empty)");
    }

    // Native-first with TrueType fallback.
    let mut native_err: Option<anyhow::Error> = None;
    if let Some(filename) = entry.native.as_deref() {
        match NativeFont::load(&format!("{FONT_PATH}{filename}")) {
            Ok(f) => return Ok(Font::Native(f)),
            Err(e) => {
                tracing::debug!("native font '{name}' ({filename}) failed: {e}; trying TrueType");
                native_err = Some(e);
            }
        }
    }

    if let Some(filename) = entry.truetype.as_deref() {
        let rel = format!("{FONT_PATH}{filename}");
        // `TrueTypeFont::load` uses `std::fs::read` directly, so the
        // game-relative path has to be resolved through the SbFile
        // datadir-search machinery (unlike `NativeFont::load`, which
        // opens via `SbFile::open`).
        let resolved =
            engine_sbfile::resolve_data_path(&rel).unwrap_or_else(|| PathBuf::from(&rel));
        let tt = TrueTypeFont::load(&resolved);
        if tt.is_valid() {
            return Ok(Font::TrueType(tt));
        }
        bail!(
            "font '{}': TrueType load failed for '{}'{}",
            name,
            resolved.display(),
            native_err
                .map(|e| format!(" (native also failed: {e})"))
                .unwrap_or_default()
        );
    }

    Err(native_err.unwrap_or_else(|| anyhow::anyhow!("font '{}' has no loadable filename", name)))
}

// ─── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_font() -> NativeFont {
        let mut chars = HashMap::new();
        chars.insert(
            b'A' as u16,
            CharacterInfo {
                start: 0,
                width: 3,
                pre_spacing: 0,
                post_spacing: 1,
            },
        );
        chars.insert(
            b'B' as u16,
            CharacterInfo {
                start: 3,
                width: 3,
                pre_spacing: 1,
                post_spacing: 0,
            },
        );
        chars.insert(
            b' ' as u16,
            CharacterInfo {
                start: 0,
                width: 2,
                pre_spacing: 0,
                post_spacing: 0,
            },
        );

        // 6-pixel-wide atlas, 2 rows tall. All pixels opaque (alpha
        // blue channel = 0x1F, extracted via `& 0x1F`). Glyph pixels
        // avoid the 0x07C0 color-key sentinel the render path skips.
        NativeFont {
            name: "test".to_string(),
            height: 2,
            baseline: 1,
            extra_spacing: 0,
            characters: chars,
            glyph_pixels: vec![
                0x1111, 0x2222, 0x3333, 0x4444, 0x5555, 0x6666, // row 0
                0xAAAA, 0xBBBB, 0xCCCC, 0xDDDD, 0xEEEE, 0xFFFF, // row 1
            ],
            glyph_width: 6,
            alpha_pixels: vec![0x001F; 12],
            alpha_width: 6,
            missing_chars_warned: Mutex::new(HashSet::new()),
        }
    }

    #[test]
    fn text_width_basic() {
        let f = make_test_font();
        assert_eq!(f.text_width(""), 0);
        // A: pre=0, w=3, post=1 → 4 (extra=0)
        assert_eq!(f.text_width("A"), 4);
        // A(4) + B(pre=1+w=3+post=0=4) → 8 (extra=0)
        assert_eq!(f.text_width("AB"), 8);
    }

    #[test]
    fn layout_quads_skip_spaces_but_preserve_advance() {
        let f = make_test_font();
        let quads = f.layout_quads("A B", 10, 20);

        assert_eq!(quads.len(), 2, "space glyphs must not be sampled");
        assert_eq!(quads[0].dst_x, 10);
        // A advances 4 px, the space advances 2 px, then B has 1 px pre-spacing.
        assert_eq!(quads[1].dst_x, 17);
    }

    #[test]
    fn test_extra_spacing() {
        let mut f = make_test_font();
        f.extra_spacing = 2;
        // Extra spacing is added once per character, not once per gap.
        // A: 0+3+1+2=6, B: 1+3+0+2=6 → total = 12
        assert_eq!(f.text_width("AB"), 12);
    }

    #[test]
    fn missing_char_returns_no_metrics() {
        // Missing chars do NOT fall back to the space glyph —
        // `text_width` contributes 0.
        let f = make_test_font();
        // 'Z' is not in the test font; only A/B/space are.
        assert_eq!(f.text_width("Z"), 0);
        // "AZ" should equal "A" (the Z contributes nothing).
        assert_eq!(f.text_width("AZ"), f.text_width("A"));
    }

    // ── Font config parsing ────────────────────────────────────────────

    #[test]
    fn config_parses_native_only() {
        let cfg = parse_font_config("Tooltips: tooltips.bfn,");
        let e = cfg.get("Tooltips").expect("entry present");
        assert_eq!(e.native.as_deref(), Some("tooltips.bfn"));
        assert!(e.truetype.is_none());
    }

    #[test]
    fn config_parses_truetype_only() {
        // Matches the real `manager.cfg` shape: leading comma, then
        // the TrueType filename in the second column.
        let cfg = parse_font_config("ListDefault: ,\tListDefault.tfn");
        let e = cfg.get("ListDefault").expect("entry present");
        assert!(
            e.native.is_none(),
            "native column should be empty, got {:?}",
            e.native
        );
        assert_eq!(e.truetype.as_deref(), Some("ListDefault.tfn"));
    }

    #[test]
    fn config_parses_both_columns() {
        let cfg = parse_font_config("Mixed: foo.sbf, foo.tfn");
        let e = cfg.get("Mixed").unwrap();
        assert_eq!(e.native.as_deref(), Some("foo.sbf"));
        assert_eq!(e.truetype.as_deref(), Some("foo.tfn"));
    }

    #[test]
    fn config_drops_fully_empty_lines() {
        let cfg = parse_font_config("Empty: ,\n\nReal: only_native.bfn,");
        assert!(!cfg.contains_key("Empty"));
        assert!(cfg.contains_key("Real"));
    }

    #[test]
    fn config_parses_real_manager_cfg_shape() {
        // Reduced copy of datadirs/.../manager.cfg — exercises the
        // tab-separated, trailing-comma, TT-only, and native-only
        // line variants in one go.
        let cfg = parse_font_config(
            "Tooltips\t\t:\ttooltips.bfn,\n\
             ListDefault\t:\t,\t\tListDefault.tfn\n\
             ListFocused\t:\t,\t\tListFocused.tfn\n",
        );
        assert_eq!(
            cfg.get("Tooltips").unwrap().native.as_deref(),
            Some("tooltips.bfn")
        );
        assert!(cfg.get("Tooltips").unwrap().truetype.is_none());

        let list = cfg.get("ListDefault").unwrap();
        assert!(list.native.is_none());
        assert_eq!(list.truetype.as_deref(), Some("ListDefault.tfn"));
    }

    /// TrueType-fallback smoke test: when the native column is empty,
    /// `load_font_by_name` must attempt the TrueType path and surface
    /// the result through the [`Font::TrueType`] variant rather than
    /// failing outright as it did pre-fix.
    ///
    /// We can't build a real `.tfn` on disk in a hermetic unit test
    /// (SBFile resolves against the game datadir), so this test relies
    /// on [`TrueTypeFont::load`] returning an *invalid* font for a
    /// nonexistent path — we then verify the error message names the
    /// TrueType filename, proving the TrueType branch was taken.
    #[test]
    fn load_font_by_name_attempts_truetype_when_native_absent() {
        let mut cfg = HashMap::new();
        cfg.insert(
            "ListDefault".to_string(),
            FontEntry {
                native: None,
                truetype: Some("NonexistentListDefault.tfn".to_string()),
            },
        );

        let err = match load_font_by_name(&cfg, "ListDefault") {
            Ok(_) => panic!("expected load to fail for a nonexistent .tfn"),
            Err(e) => e,
        };
        let msg = format!("{err}");
        assert!(
            msg.contains("NonexistentListDefault.tfn"),
            "error should reference the TrueType filename, got: {msg}",
        );
    }

    #[test]
    #[should_panic(expected = "no font for key")]
    fn load_font_by_name_panics_on_empty_entry() {
        let mut cfg = HashMap::new();
        cfg.insert(
            "Empty".to_string(),
            FontEntry {
                native: None,
                truetype: None,
            },
        );
        let _ = load_font_by_name(&cfg, "Empty");
    }
}
