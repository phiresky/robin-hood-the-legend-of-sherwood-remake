//! TrueType font loading and text metrics.
//!
//! Parses the Spellbound `.tfn` binary font descriptor format and loads the
//! referenced `.ttf` file via `ab_glyph` to provide character/string width
//! metrics.

use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use std::path::{Path, PathBuf};

const FONT_NAME_LEN: usize = 32;
const TAG_LEN: usize = 6;
const SBTTFT_TAG: &[u8; TAG_LEN] = b"SBTTFT";
const SBFONT_TAG: &[u8; TAG_LEN] = b"SBFONT";

// Expected .tfn file size: 6 (tag) + 4 (version) + 44 (FONT_HEADER) + 36 (TT_FONT_HEADER)
const MIN_SBF_SIZE: usize = 90;

/// Style flags.
pub const STYLE_BOLD: u32 = 1;
pub const STYLE_ITALIC: u32 = 2;

/// A loaded TrueType font with parsed metadata and `ab_glyph` metrics.
pub struct TrueTypeFont {
    // File header
    file_tag: [u8; TAG_LEN],
    file_version: u32,

    // Font header
    name: [u8; FONT_NAME_LEN],
    flags: u32,
    styles: u32,
    height: u32,

    // TrueType header
    tt_name: [u8; FONT_NAME_LEN],
    color: u32, // COLORREF (0x00BBGGRR)

    // Metrics provider
    font: Option<FontArc>,

    valid: bool,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// NUL-terminated length within a fixed-size byte buffer.
fn cstr_len(buf: &[u8]) -> usize {
    buf.iter().position(|&b| b == 0).unwrap_or(buf.len())
}

/// Read a little-endian u32 from a byte slice at `offset`.
fn read_u32_le(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

/// Case-insensitive file lookup in `dir`.
fn find_case_insensitive(dir: &Path, name: &str) -> Option<PathBuf> {
    let lower = name.to_ascii_lowercase();
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        if entry.file_name().to_string_lossy().to_ascii_lowercase() == lower {
            return Some(entry.path());
        }
    }
    None
}

fn ttf_search_dirs(sbf_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    push_unique_dir(&mut dirs, sbf_dir.map(Path::to_path_buf));
    push_unique_dir(&mut dirs, Some(PathBuf::from(".")));
    push_unique_dir(&mut dirs, Some(PathBuf::from("assets")));

    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        push_unique_dir(&mut dirs, Some(exe_dir.to_path_buf()));
        push_unique_dir(&mut dirs, Some(exe_dir.join("assets")));
        push_unique_dir(&mut dirs, exe_dir.parent().map(|p| p.join("assets")));
        push_unique_dir(
            &mut dirs,
            exe_dir
                .parent()
                .and_then(|p| p.parent())
                .map(|p| p.join("assets")),
        );
    }

    dirs
}

fn push_unique_dir(dirs: &mut Vec<PathBuf>, dir: Option<PathBuf>) {
    let Some(dir) = dir else {
        return;
    };
    if !dirs.iter().any(|existing| existing == &dir) {
        dirs.push(dir);
    }
}

// ---------------------------------------------------------------------------
// Core implementation
// ---------------------------------------------------------------------------

impl TrueTypeFont {
    fn new_invalid() -> Self {
        Self {
            file_tag: [0; TAG_LEN],
            file_version: 0,
            name: [0; FONT_NAME_LEN],
            flags: 0,
            styles: 0,
            height: 0,
            tt_name: [0; FONT_NAME_LEN],
            color: 0,
            font: None,
            valid: false,
        }
    }

    /// Load from a `.tfn` (Spellbound TrueType font descriptor) file.
    pub fn load(sbf_path: &Path) -> Self {
        let mut font = Self::new_invalid();

        let data = match std::fs::read(sbf_path) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("robin_rs font: cannot read '{}': {}", sbf_path.display(), e);
                return font;
            }
        };

        if !font.parse_sbf(&data) {
            return font;
        }

        // Resolve and load the .ttf file for metrics
        let resolved = font.find_and_load_ttf(sbf_path.parent());
        font.warn_unsupported_styles(font.styles & !resolved);
        font
    }

    /// Create from explicit parameters + raw TTF bytes (for callers that
    /// already have the headers + TTF data in memory).
    pub fn from_parts(
        name: &[u8; FONT_NAME_LEN],
        height: u32,
        styles: u32,
        flags: u32,
        tt_name: &[u8; FONT_NAME_LEN],
        color: u32,
        ttf_data: &[u8],
    ) -> Self {
        let mut font = Self::new_invalid();
        font.file_tag = *SBTTFT_TAG;
        font.file_version = 0x0100;
        font.name = *name;
        font.flags = flags;
        font.styles = styles;
        font.height = height;
        font.tt_name = *tt_name;
        font.color = color;
        font.valid = true;

        if let Ok(f) = FontArc::try_from_vec(ttf_data.to_vec()) {
            font.font = Some(f);
        }
        // No path available, so no chance to resolve a sibling variant file.
        font.warn_unsupported_styles(font.styles);
        font
    }

    /// `ab_glyph` cannot synthesise bold/italic from a regular face, so
    /// `find_and_load_ttf` first tries sibling variant files
    /// (`<name>bd.ttf`, `<name>-Italic.ttf`, etc.). This warns for the
    /// *unresolved* residue — styles the `.tfn` requested that no sibling
    /// file satisfied, so metrics will fall back to the upright face.
    fn warn_unsupported_styles(&self, unresolved_mask: u32) {
        if unresolved_mask & (STYLE_BOLD | STYLE_ITALIC) != 0 {
            tracing::warn!(
                "TrueType font '{}' declares styles {:#x} (bold/italic) but no \
                 sibling variant .ttf was found (unresolved {:#x}); ab_glyph \
                 cannot synthesise these — using upright metrics",
                self.truetype_name_str(),
                self.styles,
                unresolved_mask,
            );
        }
    }

    // -- Binary format parsing -----------------------------------------------

    /// Parse the Spellbound `.tfn` binary format.
    ///
    /// Layout (all integers little-endian u32):
    /// ```text
    /// [0..6]   TAG           "SBTTFT"
    /// [6..10]  version       0x0100
    /// [10..42] name          font display name (NUL-padded)
    /// [42..46] flags         (unused)
    /// [46..50] styles        Bold=1, Italic=2
    /// [50..54] height        pixel height
    /// [54..86] ttf_name      .ttf filename (NUL-padded)
    /// [86..90] color         COLORREF (0x00BBGGRR)
    /// ```
    fn parse_sbf(&mut self, data: &[u8]) -> bool {
        if data.len() < MIN_SBF_SIZE {
            return false;
        }

        // TAG
        self.file_tag.copy_from_slice(&data[0..TAG_LEN]);
        if &self.file_tag != SBTTFT_TAG {
            tracing::warn!("robin_rs font: bad tag {:?}", &self.file_tag);
            return false;
        }

        // Version
        self.file_version = read_u32_le(data, 6);

        // FONT_HEADER
        self.name.copy_from_slice(&data[10..10 + FONT_NAME_LEN]);
        self.flags = read_u32_le(data, 42);
        self.styles = read_u32_le(data, 46);
        self.height = read_u32_le(data, 50);

        // TT_FONT_HEADER
        self.tt_name.copy_from_slice(&data[54..54 + FONT_NAME_LEN]);
        self.color = read_u32_le(data, 86);

        self.valid = true;
        true
    }

    // -- TTF loading ---------------------------------------------------------

    /// Locate and load the .ttf file referenced by `tt_name`. When the `.tfn`
    /// declares Bold/Italic styles, look for sibling variant files first
    /// (e.g. `<base>bd.ttf`, `<base>-Italic.ttf`) since `ab_glyph` cannot
    /// synthesise bold/italic from an upright face. Falls back to the
    /// upright face if no variant matches.
    ///
    /// Returns the bitmask of style flags satisfied by the loaded file
    /// (0 if it's the upright fallback). Used by callers to suppress the
    /// "unsupported style" warning for resolved styles.
    ///
    /// The suffix list covers Spellbound-style compact names (`bd`, `bi`,
    /// `i`) and common Windows family names (`-Bold`, `-Italic`,
    /// `-BoldItalic`). Shipped data does not request styled TrueType faces,
    /// so missing variants are logged and fall back to the upright face.
    fn find_and_load_ttf(&mut self, sbf_dir: Option<&Path>) -> u32 {
        let raw = self.truetype_name_str().to_owned();
        let base = if raw.to_ascii_lowercase().ends_with(".ttf") {
            raw[..raw.len() - 4].to_string()
        } else {
            raw
        };

        let dirs = ttf_search_dirs(sbf_dir);

        // Build candidate list (suffix, styles_resolved). Most-specific first
        // so that, e.g., a Bold|Italic .tfn prefers a `<base>bi.ttf` over
        // `<base>bd.ttf` (which would only resolve Bold).
        let want_bold = self.styles & STYLE_BOLD != 0;
        let want_italic = self.styles & STYLE_ITALIC != 0;
        let mut candidates: Vec<(&'static str, u32)> = Vec::new();
        if want_bold && want_italic {
            for s in ["bi", "-BoldItalic", "z"] {
                candidates.push((s, STYLE_BOLD | STYLE_ITALIC));
            }
        }
        if want_bold {
            for s in ["bd", "-Bold", "b"] {
                candidates.push((s, STYLE_BOLD));
            }
        }
        if want_italic {
            for s in ["i", "-Italic"] {
                candidates.push((s, STYLE_ITALIC));
            }
        }
        candidates.push(("", 0));

        for (suffix, resolved) in candidates {
            let name = format!("{}{}.ttf", base, suffix);
            for dir in &dirs {
                if let Some(path) = find_case_insensitive(dir, &name)
                    && let Ok(data) = std::fs::read(&path)
                {
                    match FontArc::try_from_vec(data) {
                        Ok(f) => {
                            self.font = Some(f);
                            if resolved != 0 {
                                tracing::debug!(
                                    "robin_rs font: '{}' resolved style \
                                     variant {:#x} via '{}'",
                                    self.name_str(),
                                    resolved,
                                    path.display(),
                                );
                            }
                            return resolved;
                        }
                        Err(e) => {
                            tracing::warn!(
                                "robin_rs font: failed to parse '{}': {}",
                                path.display(),
                                e
                            );
                        }
                    }
                }
            }
        }
        tracing::warn!(
            "robin_rs font: could not find TTF '{}.ttf' for metrics",
            base
        );
        0
    }

    // -- Accessors -----------------------------------------------------------

    pub fn is_valid(&self) -> bool {
        self.valid
    }

    pub fn has_loaded_face(&self) -> bool {
        self.font.is_some()
    }

    pub fn name_str(&self) -> &str {
        std::str::from_utf8(&self.name[..cstr_len(&self.name)]).unwrap_or("")
    }

    pub fn truetype_name_str(&self) -> &str {
        std::str::from_utf8(&self.tt_name[..cstr_len(&self.tt_name)]).unwrap_or("")
    }

    pub fn get_height(&self) -> u32 {
        self.height
    }

    pub fn get_styles(&self) -> u32 {
        self.styles
    }

    pub fn get_color(&self) -> u32 {
        self.color
    }

    pub fn is_native(&self) -> bool {
        self.file_tag == *SBFONT_TAG
    }

    // -- Metrics (from ab_glyph) ---------------------------------------------

    fn px_scale(&self) -> PxScale {
        PxScale::from(self.height as f32)
    }

    /// Baseline position (ascent) in pixels.
    ///
    /// Uses `.round()` instead of truncation to avoid off-by-one
    /// differences at small px sizes (plain `as u32` truncation produced
    /// noticeable misalignment).
    pub fn get_baseline(&self) -> u32 {
        match self.font {
            Some(ref f) => f.as_scaled(self.px_scale()).ascent().round() as u32,
            None => 0,
        }
    }

    /// Extra inter-character spacing. TrueType fonts always return 0.
    pub fn get_extra_spacing(&self) -> i32 {
        0
    }

    /// Total rasterised pixel height (ascent + descent), used to size a
    /// scratch ARGB buffer that won't clip descenders. Falls back to the
    /// .tfn `height` field when the TTF face isn't loaded.
    pub fn total_pixel_height(&self) -> u32 {
        match self.font {
            Some(ref f) => {
                let s = f.as_scaled(self.px_scale());
                (s.ascent() - s.descent()).ceil().max(1.0) as u32
            }
            None => self.height,
        }
    }

    /// Width of a single character. Kerning out-params are always 0 for
    /// TrueType.
    pub fn get_char_width(&self, ch: u32, left_kerning: &mut i32, right_kerning: &mut i32) -> u32 {
        *left_kerning = 0;
        *right_kerning = 0;

        let Some(ref f) = self.font else { return 0 };
        let Some(c) = char::from_u32(ch) else {
            return 0;
        };

        let scaled = f.as_scaled(self.px_scale());
        let glyph_id = f.glyph_id(c);
        scaled.h_advance(glyph_id) as u32
    }

    /// Width of a character (simple form, including kerning).
    pub fn get_char_width_total(&self, ch: u32) -> u32 {
        let mut lk = 0i32;
        let mut rk = 0i32;
        let w = self.get_char_width(ch, &mut lk, &mut rk);
        (lk + w as i32 + rk) as u32
    }

    /// Compute width of a string of Unicode codepoints.
    pub fn get_string_width(
        &self,
        chars: &[u32],
        left_kerning: &mut i32,
        right_kerning: &mut i32,
    ) -> i32 {
        let mut result: i32 = 0;
        *left_kerning = 0;
        *right_kerning = 0;

        let extra = self.get_extra_spacing();
        let last = chars.len().wrapping_sub(1);

        for (i, &ch) in chars.iter().enumerate() {
            let mut cur_lk: i32 = 0;
            let mut cur_rk: i32 = 0;
            let w = self.get_char_width(ch, &mut cur_lk, &mut cur_rk) as i32;
            result += w + extra;

            if i == 0 {
                *left_kerning = cur_lk;
            } else {
                result += cur_lk;
            }

            if i == last {
                *right_kerning = cur_rk;
            } else {
                result += cur_rk;
            }
        }

        result
    }

    /// Simple string width (with kerning folded in).
    pub fn get_string_width_total(&self, chars: &[u32]) -> i32 {
        let mut lk = 0i32;
        let mut rk = 0i32;
        let w = self.get_string_width(chars, &mut lk, &mut rk);
        lk + w + rk
    }

    // -- Glyph rasterisation -------------------------------------------------

    /// Rasterise `text` into an ARGB8888 buffer using
    /// `ab_glyph::Font::outline_glyph`. The font's `color` field is
    /// applied as the foreground.
    ///
    /// Buffer layout: ARGB8888 little-endian memory order, each pixel is
    /// 4 bytes `[B, G, R, A]`. `pitch` is in **bytes**.
    ///
    /// Wired into the renderer via `Renderer::render_text_truetype`,
    /// which sizes a scratch ARGB buffer with [`Self::total_pixel_height`],
    /// calls this method, then uploads the buffer as a one-shot GPU
    /// texture for the standard blend-quad path.
    #[allow(clippy::too_many_arguments)]
    pub fn render_to_argb(
        &self,
        data: &mut [u8],
        surface_w: i32,
        surface_h: i32,
        pitch: usize,
        text: &str,
        x: i32,
        y: i32,
    ) {
        let Some(ref ttf) = self.font else { return };
        let scaled = ttf.as_scaled(self.px_scale());
        let baseline_y = scaled.ascent();

        // color is COLORREF (0x00BBGGRR).
        let r = (self.color & 0xFF) as u8;
        let g = ((self.color >> 8) & 0xFF) as u8;
        let b = ((self.color >> 16) & 0xFF) as u8;

        let mut cx = x as f32;
        let pen_y = y as f32 + baseline_y;

        for ch in text.chars() {
            let glyph_id = ttf.glyph_id(ch);
            let advance = scaled.h_advance(glyph_id);
            let glyph =
                glyph_id.with_scale_and_position(self.px_scale(), ab_glyph::point(cx, pen_y));
            if let Some(outlined) = ttf.outline_glyph(glyph) {
                let bb = outlined.px_bounds();
                let min_x = bb.min.x as i32;
                let min_y = bb.min.y as i32;
                outlined.draw(|gx, gy, c| {
                    let alpha = (c * 255.0).clamp(0.0, 255.0) as u8;
                    if alpha == 0 {
                        return;
                    }
                    let dx = min_x + gx as i32;
                    let dy = min_y + gy as i32;
                    if dx < 0 || dy < 0 || dx >= surface_w || dy >= surface_h {
                        return;
                    }
                    let off = dy as usize * pitch + dx as usize * 4;
                    if off + 3 < data.len() {
                        // Store straight-alpha color. The GPU blend pass
                        // multiplies RGB by alpha; premultiplying here would
                        // apply coverage twice and make TrueType list text
                        // thin and low contrast.
                        data[off] = b;
                        data[off + 1] = g;
                        data[off + 2] = r;
                        data[off + 3] = data[off + 3].max(alpha);
                    }
                });
            }
            cx += advance;
        }
    }

    // -- Serialization (write back to .tfn) ----------------------------------

    /// Serialize to the Spellbound `.tfn` binary format.
    pub fn to_sbf_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(MIN_SBF_SIZE);
        out.extend_from_slice(&self.file_tag);
        out.extend_from_slice(&self.file_version.to_le_bytes());
        out.extend_from_slice(&self.name);
        out.extend_from_slice(&self.flags.to_le_bytes());
        out.extend_from_slice(&self.styles.to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());
        out.extend_from_slice(&self.tt_name);
        out.extend_from_slice(&self.color.to_le_bytes());
        out
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: locate arial.ttf relative to the workspace root.
    fn find_arial() -> PathBuf {
        for candidate in ["assets/arial.ttf", "../../assets/arial.ttf"] {
            let p = PathBuf::from(candidate);
            if p.exists() {
                return p;
            }
        }
        panic!("arial.ttf not found — run tests from the repo root");
    }

    fn make_test_font() -> TrueTypeFont {
        let ttf_data = std::fs::read(find_arial()).expect("read arial.ttf");
        let mut name = [0u8; FONT_NAME_LEN];
        name[..12].copy_from_slice(b"List Default");
        let mut tt_name = [0u8; FONT_NAME_LEN];
        tt_name[..5].copy_from_slice(b"Arial");

        TrueTypeFont::from_parts(&name, 15, 0, 0, &tt_name, 0x004080FF, &ttf_data)
    }

    #[test]
    fn test_from_parts_valid() {
        let f = make_test_font();
        assert!(f.is_valid());
        assert_eq!(f.name_str(), "List Default");
        assert_eq!(f.truetype_name_str(), "Arial");
        assert_eq!(f.get_height(), 15);
        assert_eq!(f.get_color(), 0x004080FF);
        assert!(!f.is_native());
        assert!(f.font.is_some());
    }

    #[test]
    fn test_baseline_nonzero() {
        let f = make_test_font();
        let bl = f.get_baseline();
        assert!(
            bl > 0 && bl < f.get_height(),
            "baseline {} should be in (0, {})",
            bl,
            f.get_height()
        );
    }

    #[test]
    fn test_char_width() {
        let f = make_test_font();
        let mut lk = 0i32;
        let mut rk = 0i32;

        // 'A' should have a positive width
        let w = f.get_char_width('A' as u32, &mut lk, &mut rk);
        assert!(w > 0, "width of 'A' should be positive, got {}", w);
        assert_eq!(lk, 0);
        assert_eq!(rk, 0);

        // Space should have a positive width
        let ws = f.get_char_width(' ' as u32, &mut lk, &mut rk);
        assert!(ws > 0, "width of space should be positive, got {}", ws);

        // 'i' should be narrower than 'W'
        let wi = f.get_char_width('i' as u32, &mut lk, &mut rk);
        let ww = f.get_char_width('W' as u32, &mut lk, &mut rk);
        assert!(wi < ww, "'i' ({}) should be narrower than 'W' ({})", wi, ww);
    }

    #[test]
    fn test_string_width() {
        let f = make_test_font();

        // "Hello" width should be positive
        let hello: Vec<u32> = "Hello".chars().map(|c| c as u32).collect();
        let w = f.get_string_width_total(&hello);
        assert!(
            w > 0,
            "string width of 'Hello' should be positive, got {}",
            w
        );

        // Empty string width should be 0
        let w_empty = f.get_string_width_total(&[]);
        assert_eq!(w_empty, 0);

        // Width should scale roughly with length
        let hh: Vec<u32> = "HelloHello".chars().map(|c| c as u32).collect();
        let w2 = f.get_string_width_total(&hh);
        assert_eq!(w2, w * 2, "double string should be double width");
    }

    #[test]
    fn test_extra_spacing_zero() {
        let f = make_test_font();
        assert_eq!(f.get_extra_spacing(), 0);
    }

    #[test]
    fn test_parse_sbf_binary() {
        // Build a .tfn file in memory matching the format from the hex dump
        let mut data = Vec::with_capacity(MIN_SBF_SIZE);

        // TAG
        data.extend_from_slice(b"SBTTFT");
        // version
        data.extend_from_slice(&0x0100u32.to_le_bytes());
        // strName (32 bytes)
        let mut name = [0u8; 32];
        name[..12].copy_from_slice(b"List Default");
        data.extend_from_slice(&name);
        // flags
        data.extend_from_slice(&0u32.to_le_bytes());
        // styles
        data.extend_from_slice(&0u32.to_le_bytes());
        // height
        data.extend_from_slice(&15u32.to_le_bytes());
        // strTTName (32 bytes)
        let mut tt = [0u8; 32];
        tt[..5].copy_from_slice(b"Arial");
        data.extend_from_slice(&tt);
        // color
        data.extend_from_slice(&0x004080FFu32.to_le_bytes());

        assert_eq!(data.len(), MIN_SBF_SIZE);

        let mut font = TrueTypeFont::new_invalid();
        assert!(font.parse_sbf(&data));
        assert!(font.is_valid());
        assert_eq!(font.name_str(), "List Default");
        assert_eq!(font.truetype_name_str(), "Arial");
        assert_eq!(font.get_height(), 15);
        assert_eq!(font.get_styles(), 0);
        assert_eq!(font.get_color(), 0x004080FFu32);
        assert_eq!(font.file_version, 0x0100);
    }

    #[test]
    fn test_roundtrip_sbf() {
        let mut font = TrueTypeFont::new_invalid();
        let mut data = Vec::with_capacity(MIN_SBF_SIZE);
        data.extend_from_slice(b"SBTTFT");
        data.extend_from_slice(&0x0100u32.to_le_bytes());
        let mut name = [0u8; 32];
        name[..4].copy_from_slice(b"Test");
        data.extend_from_slice(&name);
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes()); // Bold
        data.extend_from_slice(&20u32.to_le_bytes());
        let mut tt = [0u8; 32];
        tt[..5].copy_from_slice(b"Arial");
        data.extend_from_slice(&tt);
        data.extend_from_slice(&0xFFFFFFu32.to_le_bytes());

        font.parse_sbf(&data);
        let out = font.to_sbf_bytes();
        assert_eq!(data, out);
    }

    #[test]
    fn test_parse_real_tfn() {
        // Try to parse an actual .tfn file from the game data
        for candidate in [
            "../../datadirs/demo/Data/Interface/Fonts/ListDefault.tfn",
            "datadirs/demo/Data/Interface/Fonts/ListDefault.tfn",
        ] {
            let p = PathBuf::from(candidate);
            if p.exists() {
                let data = std::fs::read(&p).unwrap();
                let mut font = TrueTypeFont::new_invalid();
                assert!(font.parse_sbf(&data), "failed to parse {}", p.display());
                assert!(font.is_valid());
                assert_eq!(font.truetype_name_str(), "Arial");
                assert_eq!(font.get_height(), 15);
                return;
            }
        }
        tracing::warn!("skipping test_parse_real_tfn: no game data found");
    }
}
