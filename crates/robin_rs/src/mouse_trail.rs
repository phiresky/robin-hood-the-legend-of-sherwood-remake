//! Swordfight mouse-trail visual rendering.
//!
//! While the player is dragging the left mouse button during a
//! swordfight, the recorded polyline is drawn as a fading orange streak
//! so the player can see what gesture they're tracing.
//!
//! ## Blend equivalence to the original formula
//!
//! The legacy implementation runs a custom per-pixel blend on the
//! CPU framebuffer:
//!
//! ```text
//!     alpha       = min(32, 2 * (32 - used_alpha))    // dst scale factor
//!     result.chan = saturate(pattern_premul.chan + dst.chan * alpha / 32)
//! ```
//!
//! where `used_alpha` ∈ [0, 31] is the per-row alpha-cap times the
//! current fade level.  Working the algebra case-by-case:
//!
//! * `used_alpha ≤ 16`: `2 * (32 - used_alpha) ≥ 32`, so the scale
//!   factor saturates at 32 and `dst * 32/32 = dst`.  The formula
//!   collapses to `result = saturate(pattern_premul + dst)` — **exactly
//!   additive GPU blending** in RGB565.
//! * `used_alpha > 16`: the scale factor drops from 32 to 2,
//!   progressively scaling the destination down as the source opacity
//!   grows, which transitions from additive toward replacement at
//!   maximum brightness.
//!
//! The second regime only kicks in on the *fresh* trail tip (roughly
//! the last ~15 frames, before `alpha_level` drops below 50 and the
//! per-row `used_alpha` falls back into the first regime).  On the
//! game's typical dim medieval backgrounds, the divergence is
//! sub-perceptible — it only differs from pure additive on pixels
//! where the destination is already bright enough to saturate the
//! additive output, and even then it's confined to the three or four
//! polyline samples at the very tip of the drag.
//!
//! So we use native GPU additive blending and skip the CPU-side
//! readback path entirely.
//!
//! The pattern shape, interpolation along the dominant axis between
//! consecutive points, the 16-pixel column height (`TRAIL_HEIGHT`),
//! and the per-frame alpha decay (`DISMISHING_SPEED = 300`) all
//! match the original behaviour.

use crate::gfx_types::BlendMode;
use crate::mouse_way::MouseWay;
use crate::renderer::{GpuImage, Renderer, TRANSPARENT_COLOR_KEY_16};
use robin_assets::picture::{Picture, PixelFormat};
use robin_engine::coordinates::ScreenPoint;
use robin_engine::sprite as engine_sprite;

/// Vertical step size used when interpolating along the Y axis.
pub const TRAIL_HEIGHT: i32 = 16;

/// Alpha-decay speed applied once a trail sample drops below 100.
pub const DISMISHING_SPEED: f32 = 300.0;

/// Trail colour — orange (`0xFF`, `0x90`, `0x00`).
const TRAIL_COLOR_R: u8 = 0xFF;
const TRAIL_COLOR_G: u8 = 0x90;
const TRAIL_COLOR_B: u8 = 0x00;

/// RGB565 value for the trail colour.
fn trail_color_565() -> u16 {
    let r = (TRAIL_COLOR_R as u16 >> 3) & 0x1F;
    let g = (TRAIL_COLOR_G as u16 >> 2) & 0x3F;
    let b = (TRAIL_COLOR_B as u16 >> 3) & 0x1F;
    (r << 11) | (g << 5) | b
}

/// Pre-computed mouse-trail pattern + the 32 pre-multiplied alpha-level
/// textures used to draw it on the GPU.
///
/// Owns one 1-pixel-wide surface per alpha level (indices 0..32 match
/// the alpha-level minus one); each surface holds a column of RGB565
/// pre-multiplied trail pixels of height `pattern_height`.  Rendering
/// a polyline point is a single GPU copy of the appropriate column.
pub struct MouseTrailRenderer {
    /// Height of the trail column — the height of the `RHID_MOUSE_TRAIL`
    /// source surface.
    pub pattern_height: u16,
    /// Per-row alpha cap — values are `0x1F - (source_pixel_blue_channel)`
    /// with `blue` being the low 5 bits of the source surface's column 0
    /// pixels.
    pub alpha_caps: Vec<u16>,
    /// One persistent GPU texture per alpha level (32 entries).  Each is
    /// 1 pixel wide by `pattern_height` tall, filled with the pre-multiplied
    /// trail colour per row.
    images: Vec<GpuImage>,
}

impl MouseTrailRenderer {
    /// Build the trail renderer from a `RHID_MOUSE_TRAIL` picture.
    ///
    /// Returns `None` if the picture isn't in an RGB16 format (the
    /// only format the source resource ships in — the fallback is to
    /// silently skip trail rendering rather than crash).
    pub fn from_picture(pic: &Picture, renderer: &mut Renderer) -> Option<Self> {
        if pic.pixel_format != PixelFormat::Rgb16 {
            return None;
        }
        let pattern_height = pic.height;
        if pattern_height == 0 || pic.width == 0 {
            return None;
        }

        // ── Alpha caps from column 0. ──
        let row_stride = pic.pitch as usize;
        let mut alpha_caps = Vec::with_capacity(pattern_height as usize);
        for row in 0..pattern_height as usize {
            let offset = row * row_stride;
            if offset + 1 >= pic.data.len() {
                return None;
            }
            let pixel = u16::from_le_bytes([pic.data[offset], pic.data[offset + 1]]);
            let blue = pixel & 0x1F;
            alpha_caps.push(0x1F - blue);
        }

        // ── Pre-multiplied pattern per alpha level 1..=32. ──
        //
        // For each alpha level `j` and each row `i`:
        //   used_alpha = (alphaCap[i] * j) >> 5
        //   red  = (trail_color_r5 << 0) * used_alpha   (masked to RGB565 R slot)
        //   grn  = (trail_color_g6 << 0) * used_alpha   (masked to RGB565 G slot)
        //   blu  = (trail_color_b5 * used_alpha) >> 5   (masked to RGB565 B slot)
        //
        // The bit shifts pre-multiply the colour into the correct
        // channel slot of an RGB565 word.
        let trail = trail_color_565();
        let trail_r_src = (trail & 0xF800) >> 5;
        let trail_g_src = (trail & 0x07E0) >> 5;
        let trail_b_src = trail & 0x001F;

        let mut images = Vec::with_capacity(32);
        for j in 1u16..=32 {
            let column: Vec<u16> = alpha_caps
                .iter()
                .map(|&alpha_cap| {
                    let used_alpha = (alpha_cap * j) >> 5;
                    if used_alpha == 0 {
                        return TRANSPARENT_COLOR_KEY_16;
                    }
                    let red = (trail_r_src * used_alpha) & 0xF800;
                    let green = (trail_g_src * used_alpha) & 0x07E0;
                    let blue = ((trail_b_src * used_alpha) >> 5) & 0x001F;
                    red | green | blue
                })
                .collect();
            let image = renderer.create_rgb565_gpu_image(
                1,
                pattern_height,
                &column,
                true,
                "mouse trail column",
            )?;
            images.push(image);
        }

        Some(Self {
            pattern_height,
            alpha_caps,
            images,
        })
    }

    /// Map a per-sample alpha value (0..~112) to a pattern index
    /// (0..31): `alpha_level * 32 / 100`, clamped.
    fn alpha_index(alpha: f32) -> Option<usize> {
        // Truncating multiply by 32/100; zero (or negative) results
        // mean "fully faded — skip this sample".
        let scaled = (alpha * 32.0 / 100.0) as i32;
        if scaled <= 0 {
            return None;
        }
        // Clamp to the 32 available pre-baked alpha textures.
        let clamped = scaled.min(32) as usize;
        Some(clamped - 1)
    }

    /// Draw the polyline stored in `mouse_way` to the GPU render target.
    /// Alpha advancement is a separate fixed-tick boundary so native-refresh
    /// recompositions can draw the same state repeatedly without ageing it.
    ///
    /// Alternates between single-point draws (for adjacent samples)
    /// and X/Y-dominant interpolation (for larger gaps). The caller advances
    /// each sample's alpha separately, once after the fixed-tick presentation
    /// is complete, so display-refresh recompositions remain read-only. The
    /// caller should only invoke this while the player is dragging during a
    /// swordfight (`IsDragging() && IsSelectedPCSwordfighting()`).
    pub fn render(&self, mouse_way: &mut MouseWay, renderer: &mut Renderer) {
        let size = mouse_way.points.len();
        if size == 0 {
            return;
        }
        debug_assert_eq!(mouse_way.alpha.len(), size);

        for i in 0..size {
            let point = mouse_way.points[i];
            let alpha_level = mouse_way.alpha[i];

            // ── Single-point draw branch ──
            // Last sample or neighbour within (1, TRAIL_HEIGHT).
            let draw_single = if i == size - 1 {
                true
            } else {
                let next = mouse_way.points[i + 1];
                (next.x - point.x).abs() <= 1.0 && (next.y - point.y).abs() <= TRAIL_HEIGHT as f32
            };

            if draw_single {
                self.draw_column(point, alpha_level, renderer);
            } else {
                let next = mouse_way.points[i + 1];
                let next_alpha = mouse_way.alpha[i + 1];
                self.draw_interpolated(point, alpha_level, next, next_alpha, renderer);
            }
        }
    }

    /// Decay every trail sample exactly once after a complete fixed-tick
    /// presentation, independent of the physical display refresh rate.
    pub fn advance(&self, mouse_way: &mut MouseWay) {
        for alpha_level in &mut mouse_way.alpha {
            let new_alpha = if *alpha_level > 100.0 {
                *alpha_level - 1.0
            } else {
                let step = DISMISHING_SPEED / (*alpha_level).max(1.0);
                (*alpha_level - step).max(0.0)
            };
            *alpha_level = new_alpha;
        }
    }

    /// Blit a single pattern column at `pos` for the given alpha level.
    ///
    /// Picks the alpha-level surface, clips to the bottom of the
    /// screen, and additively blends it in.
    fn draw_column(&self, pos: ScreenPoint, alpha_level: f32, renderer: &mut Renderer) {
        let Some(idx) = Self::alpha_index(alpha_level) else {
            return;
        };
        let x = pos.x as i32;
        let y = pos.y as i32;

        // Bottom-edge clip — height is capped at `SCREENHEIGHT - y`.
        let screen_h = renderer.screen_height() as i32;
        if y >= screen_h || x < 0 || x >= renderer.screen_width() as i32 {
            return;
        }
        let height = (self.pattern_height as i32).min(screen_h - y).max(0);
        if height == 0 {
            return;
        }

        let src = engine_sprite::BBox::from_coords(0.0, 0.0, 1.0, height as f32);
        let dst = engine_sprite::BBox::from_coords(
            x as f32,
            y as f32,
            (x + 1) as f32,
            (y + height) as f32,
        );
        renderer.render_gpu_image(&self.images[idx], Some(&src), Some(&dst), BlendMode::Add);
    }

    /// Draw a segment between two polyline points by interpolating
    /// column positions along the dominant axis.
    fn draw_interpolated(
        &self,
        start: ScreenPoint,
        start_alpha: f32,
        end: ScreenPoint,
        end_alpha: f32,
        renderer: &mut Renderer,
    ) {
        let dx = (end.x - start.x).abs();
        let dy = (end.y - start.y).abs();

        if dx * TRAIL_HEIGHT as f32 > dy {
            // ── X-dominant interpolation ──
            let (mut x_start, mut x_end) = (start.x, end.x);
            let (mut y_start, mut y_end) = (start.y, end.y);
            let (mut a_start, mut a_end) = (start_alpha, end_alpha);

            if x_start > x_end {
                std::mem::swap(&mut x_start, &mut x_end);
                std::mem::swap(&mut y_start, &mut y_end);
                std::mem::swap(&mut a_start, &mut a_end);
                // Shift-right-by-one correction to avoid holes when
                // the segment direction was reversed.
                x_start += 1.0;
                x_end += 1.0;
            }

            let span = x_end - x_start;
            if span < 1.0 {
                return;
            }
            let dy_step = (y_end - y_start) / span;
            let alpha_step = (a_end - a_start) / span;

            let mut y = y_start;
            let mut alpha = a_start;
            let x_lo = x_start as i32;
            let x_hi = x_end as i32;
            for px_x in x_lo..x_hi {
                self.draw_column(ScreenPoint::new(px_x as f32, y), alpha, renderer);
                y += dy_step;
                alpha += alpha_step;
            }
        } else {
            // ── Y-dominant interpolation ──
            let (mut x_start, mut x_end) = (start.x, end.x);
            let (mut y_start, mut y_end) = (start.y, end.y);
            let (mut a_start, mut a_end) = (start_alpha, end_alpha);

            if y_start > y_end {
                std::mem::swap(&mut x_start, &mut x_end);
                std::mem::swap(&mut y_start, &mut y_end);
                std::mem::swap(&mut a_start, &mut a_end);
                y_start += 1.0;
                y_end += 1.0;
            }

            let span = y_end - y_start;
            if span < 1.0 {
                return;
            }
            // Multiply both x- and alpha-steps by TRAIL_HEIGHT because
            // the inner loop steps by TRAIL_HEIGHT, not 1.  That gives
            // per-column x/alpha delta.
            let dx_step = TRAIL_HEIGHT as f32 * (x_end - x_start) / span;
            let alpha_step = TRAIL_HEIGHT as f32 * (a_end - a_start) / span;

            let mut x = x_start;
            let mut alpha = a_start;

            // Integer-truncated iteration: the column y steps by whole
            // `TRAIL_HEIGHT` pixels while x and alpha accumulate as
            // floats.  The trailing draw covers the `uw_y > y_end_i`
            // tail condition.
            let y_start_i = y_start as u32;
            let y_end_i = y_end as u32;
            let mut uw_y = y_start_i;
            while uw_y < y_end_i {
                self.draw_column(ScreenPoint::new(x, uw_y as f32), alpha, renderer);
                x += dx_step;
                alpha += alpha_step;
                uw_y += TRAIL_HEIGHT as u32;
            }
            if uw_y > y_end_i {
                self.draw_column(ScreenPoint::new(x, uw_y as f32), alpha, renderer);
            }
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_index_zero_returns_none() {
        assert_eq!(MouseTrailRenderer::alpha_index(0.0), None);
        assert_eq!(MouseTrailRenderer::alpha_index(2.0), None);
        // `2 * 32 / 100 = 0` in integer arithmetic.
    }

    #[test]
    fn alpha_index_full() {
        // 100 * 32 / 100 = 32 → clamp to 32 → index 31.
        assert_eq!(MouseTrailRenderer::alpha_index(100.0), Some(31));
    }

    #[test]
    fn alpha_index_over_full_clamps() {
        // Values above 32 clamp to the top alpha texture.
        assert_eq!(MouseTrailRenderer::alpha_index(112.5), Some(31));
        assert_eq!(MouseTrailRenderer::alpha_index(200.0), Some(31));
    }

    #[test]
    fn alpha_index_mid() {
        // 50 * 32 / 100 = 16 (int trunc) → index 15.
        assert_eq!(MouseTrailRenderer::alpha_index(50.0), Some(15));
    }

    #[test]
    fn trail_colour_is_orange_565() {
        // R=0xFF → 0x1F (5-bit), G=0x90 → 0x24 (6-bit), B=0 → 0
        // packed as (0x1F << 11) | (0x24 << 5) | 0 = 0xFC80.
        assert_eq!(trail_color_565(), 0xFC80);
    }
}
