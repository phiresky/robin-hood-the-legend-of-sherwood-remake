//! Custom mouse cursor rendering.
//!
//! Each frame of an animated cursor has its own uploaded GPU image, and
//! the per-frame animation tick advances a wait counter.
//!
//! The game uses custom cursor sprites instead of the OS cursor.
//! Cursor pictures are loaded from `.res` resource files as `CUR ` entries.
//! When no resource is available a simple arrow fallback is drawn.

use crate::gfx_types::BlendMode;
use crate::renderer::{GpuImage, Renderer, TRANSPARENT_COLOR_KEY_16, rgb565_to_rgb8};
use robin_assets::frame_holder::SHADOW_KEY;
use robin_assets::picture::Picture;
use robin_assets::resource_manager::{ResourceId, ResourceManager};
use robin_engine::sprite::BBox;

/// One uploaded animation frame.
struct CursorFrame {
    color: Option<GpuImage>,
    shadow: Option<GpuImage>,
    width: u16,
    height: u16,
}

impl CursorFrame {
    fn invalid() -> Self {
        Self {
            color: None,
            shadow: None,
            width: 0,
            height: 0,
        }
    }

    fn is_valid(&self) -> bool {
        self.width > 0 && self.height > 0
    }

    #[cfg(test)]
    fn test_valid() -> Self {
        Self {
            color: None,
            shadow: None,
            width: 32,
            height: 32,
        }
    }
}

/// Manages the software mouse cursor: loads cursor sprites, tracks
/// the current cursor/frame, and blits to the screen each frame.
impl Default for CursorRenderer {
    fn default() -> Self {
        Self::new()
    }
}

pub struct CursorRenderer {
    /// All uploaded frames for the currently loaded cursor. Empty before
    /// a cursor is loaded / after `destroy`. The fallback arrow lives
    /// here as a single-frame animation.
    frames: Vec<CursorFrame>,

    /// Hotspot offset (where the "click point" is relative to top-left).
    hotspot_x: f32,
    hotspot_y: f32,

    /// Currently active cursor resource ID (-1 = fallback).
    current_cursor_id: ResourceId,
    /// Index into `frames` of the frame currently rendered.
    current_frame: u16,
    /// Wait ticks between frames.  The `CUR ` resource format carries a
    /// single `frame_length` that applies to every frame, so we keep one
    /// value here.
    frame_length: u16,
    /// Ticks accumulated toward the next frame advance.
    frame_timer: u16,

    /// Whether the OS cursor has been hidden.
    os_cursor_hidden: bool,
}

impl CursorRenderer {
    /// Create a new cursor renderer. Call [`init`] to set up surfaces.
    pub fn new() -> Self {
        Self {
            frames: Vec::new(),
            hotspot_x: 0.0,
            hotspot_y: 0.0,
            current_cursor_id: -1,
            current_frame: 0,
            frame_length: 1,
            frame_timer: 0,
            os_cursor_hidden: false,
        }
    }

    /// Initialize: hide the OS cursor and create a fallback arrow cursor.
    pub fn init(&mut self, renderer: &mut Renderer) {
        self.hide_os_cursor();
        self.create_fallback_cursor(renderer);
    }

    /// Hide the OS cursor. The wgpu/winit window handles cursor
    /// visibility via `set_cursor_visible(false)` at construction
    /// time, so this is now just a state-bookkeeping flag.
    fn hide_os_cursor(&mut self) {
        self.os_cursor_hidden = true;
    }

    /// Release every uploaded frame currently owned by the cursor renderer.
    fn clear_frames(&mut self) {
        self.frames.clear();
    }

    /// Create a simple 16x16 arrow cursor as a fallback when no resource
    /// cursor is loaded.
    fn create_fallback_cursor(&mut self, renderer: &mut Renderer) {
        let w: u16 = 16;
        let h: u16 = 16;

        self.clear_frames();

        self.hotspot_x = 0.0;
        self.hotspot_y = 0.0;
        self.current_cursor_id = -1;
        self.current_frame = 0;

        // Draw a simple arrow cursor (white with black outline)
        // Arrow shape — each row is (start_col, end_col) for the filled region
        #[rustfmt::skip]
        const ARROW: [(u8, u8); 14] = [
            (0, 1),   // row 0
            (0, 2),   // row 1
            (0, 3),   // row 2
            (0, 4),   // row 3
            (0, 5),   // row 4
            (0, 6),   // row 5
            (0, 7),   // row 6
            (0, 8),   // row 7
            (0, 5),   // row 8
            (2, 5),   // row 9
            (3, 5),   // row 10
            (3, 5),   // row 11
            (4, 5),   // row 12
            (4, 5),   // row 13
        ];

        let white = Renderer::create_color_16(255, 255, 255);
        let black = Renderer::create_color_16(0, 0, 0);
        let mut pixels = vec![TRANSPARENT_COLOR_KEY_16; w as usize * h as usize];
        let pitch = w as usize;

        for (row, &(start, end)) in ARROW.iter().enumerate() {
            let y = row;
            // Black outline: 1px border around the filled region
            for x in start.saturating_sub(1)..=(end + 1).min(w as u8 - 1) {
                let idx = y * pitch + x as usize;
                if idx < pixels.len() {
                    pixels[idx] = black;
                }
            }
            // Also outline one row above and below
            if row > 0 {
                for x in start..=end {
                    let above = (y - 1) * pitch + x as usize;
                    if above < pixels.len() && pixels[above] == TRANSPARENT_COLOR_KEY_16 {
                        pixels[above] = black;
                    }
                }
            }
            if row + 1 < h as usize {
                for x in start..=end {
                    let below = (y + 1) * pitch + x as usize;
                    if below < pixels.len() && pixels[below] == TRANSPARENT_COLOR_KEY_16 {
                        pixels[below] = black;
                    }
                }
            }
        }

        // Fill interior white (after outline so it overwrites)
        for (row, &(start, end)) in ARROW.iter().enumerate() {
            for x in start..end {
                let idx = row * pitch + x as usize;
                if idx < pixels.len() {
                    pixels[idx] = white;
                }
            }
        }

        if let Some(frame) = upload_pixels_to_gpu_frame(w, h, &pixels, renderer) {
            self.frames.push(frame);
        }
    }

    /// Load a cursor from the resource manager.
    ///
    /// `cursor_id` is a resource ID from a `.res` file (type `CUR `).
    /// Every animation frame (picture) is uploaded to its own renderer
    /// surface so `advance_animation` can swap between them by index
    /// without re-uploading pixels.
    pub fn load_cursor(
        &mut self,
        cursor_id: ResourceId,
        resource_manager: &mut ResourceManager,
        renderer: &mut Renderer,
    ) -> bool {
        // Get cursor metadata
        let (hotspot, frame_length) = match resource_manager.get_mouse_entry(cursor_id) {
            Ok(entry) => (entry.hotspot, entry.frame_length),
            Err(e) => {
                tracing::warn!("Failed to load cursor {cursor_id}: {e}");
                return false;
            }
        };

        // Get cursor pictures, then clone the frames out so we don't
        // hold the resource-manager borrow across the renderer calls.
        let pictures: Vec<Option<Picture>> = match resource_manager.get_pictures(cursor_id) {
            Ok(pics) => pics.to_vec(),
            Err(e) => {
                tracing::warn!("Failed to get cursor pictures {cursor_id}: {e}");
                return false;
            }
        };

        if pictures.is_empty() {
            tracing::warn!("Cursor {cursor_id} has no frames");
            return false;
        }

        // Upload each frame to persistent GPU textures. Drop any previously
        // owned textures first so we don't leak on re-loads.
        self.clear_frames();

        let mut first_dims: Option<(u16, u16)> = None;
        for (idx, pic_opt) in pictures.iter().enumerate() {
            let pic = match pic_opt {
                Some(p) => p,
                None => {
                    tracing::warn!("Cursor {cursor_id} frame {idx} is empty");
                    self.frames.push(CursorFrame::invalid());
                    continue;
                }
            };
            let Some(frame) = upload_picture_to_gpu_frame(pic, renderer) else {
                tracing::warn!("Cursor {cursor_id} frame {idx} upload failed");
                self.frames.push(CursorFrame::invalid());
                continue;
            };
            first_dims.get_or_insert((frame.width, frame.height));
            self.frames.push(frame);
        }

        // Bail out if no frame uploaded successfully.
        if first_dims.is_none() {
            tracing::warn!("Cursor {cursor_id} had no valid frames");
            self.clear_frames();
            return false;
        }

        let (w, h) = first_dims.unwrap();
        self.hotspot_x = hotspot.x;
        self.hotspot_y = hotspot.y;
        self.current_cursor_id = cursor_id;
        self.current_frame = 0;
        self.frame_length = frame_length;
        self.frame_timer = 0;

        tracing::info!(
            "Loaded cursor {cursor_id}: {}x{}, hotspot ({}, {}), {} frames",
            w,
            h,
            hotspot.x,
            hotspot.y,
            self.frames.len(),
        );
        true
    }

    /// Tick the animation one game frame.
    ///
    /// Call once per tick.  The frame timer increments by one each tick;
    /// if past the threshold it increments a second time and advances the
    /// frame index.  The timer is **never reset**, so once it crosses the
    /// threshold it stays past it and the cursor advances one frame per
    /// tick from then on.
    pub fn advance_animation(&mut self) {
        if self.frames.len() <= 1 {
            return;
        }

        self.frame_timer = self.frame_timer.saturating_add(1);

        if self.frame_timer >= self.frame_length {
            // Second redundant increment to keep the timer past the
            // threshold so subsequent ticks keep advancing.
            self.frame_timer = self.frame_timer.saturating_add(1);

            // Wrap to the next *valid* frame (skip invalid uploads rather
            // than blitting from a bogus surface).
            let n = self.frames.len();
            for _ in 0..n {
                let next = self.current_frame.wrapping_add(1) as usize % n;
                self.current_frame = next as u16;
                if self.frames[next].is_valid() {
                    break;
                }
            }
        }
    }

    /// Render the cursor at the given screen position.
    ///
    /// `opacity` controls the shadow intensity (0 = fully transparent,
    /// 40 = default, 50 = bow-no/civilian/VIP, 75 = max during shooting).
    /// `shadow_color` is the shadow tint as 16-bit RGB565 (0 = no tint).
    ///
    /// Call this just before `Renderer::flip()` so the cursor appears
    /// on top of everything else.
    pub fn render(
        &self,
        renderer: &mut Renderer,
        mouse_x: f32,
        mouse_y: f32,
        opacity: u16,
        _shadow_color: u16,
    ) {
        let Some(frame) = self.frames.get(self.current_frame as usize) else {
            return;
        };
        if !frame.is_valid() {
            return;
        }

        let dst_x = mouse_x - self.hotspot_x;
        let dst_y = mouse_y - self.hotspot_y;

        let src_box = BBox::from_coords(0.0, 0.0, frame.width as f32, frame.height as f32);
        let dst_box = BBox::from_coords(
            dst_x,
            dst_y,
            dst_x + frame.width as f32,
            dst_y + frame.height as f32,
        );

        if opacity > 0
            && let Some(shadow) = frame.shadow.as_ref()
        {
            renderer.render_gpu_image_tinted(
                shadow,
                Some(&src_box),
                Some(&dst_box),
                BlendMode::Blend,
                [
                    1.0,
                    1.0,
                    1.0,
                    (opacity.min(100) as f32 / 100.0).clamp(0.0, 1.0),
                ],
            );
        }
        if let Some(color) = frame.color.as_ref() {
            renderer.render_gpu_image(color, Some(&src_box), Some(&dst_box), BlendMode::Blend);
        }
    }

    /// Dimensions of the frame currently being rendered.  Returns
    /// `(0, 0)` when no cursor is loaded.
    pub fn current_frame_size(&self) -> (u16, u16) {
        match self.frames.get(self.current_frame as usize) {
            Some(f) if f.is_valid() => (f.width, f.height),
            _ => (0, 0),
        }
    }

    pub fn current_cursor_id(&self) -> ResourceId {
        self.current_cursor_id
    }
}

/// Upload a `Picture` to persistent GPU images and return
/// the resulting [`CursorFrame`]. Returns `None` if the upload failed.
fn upload_picture_to_gpu_frame(pic: &Picture, renderer: &mut Renderer) -> Option<CursorFrame> {
    let w = pic.width;
    let h = pic.height;
    let pixel_u16: Vec<u16> = pic
        .data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    upload_pixels_to_gpu_frame(w, h, &pixel_u16, renderer)
}

fn upload_pixels_to_gpu_frame(
    w: u16,
    h: u16,
    pixels: &[u16],
    renderer: &mut Renderer,
) -> Option<CursorFrame> {
    if pixels.len() != w as usize * h as usize {
        return None;
    }

    let mut color_rgba = Vec::with_capacity(pixels.len() * 4);
    let mut shadow_rgba = Vec::with_capacity(pixels.len() * 4);
    let mut has_shadow = false;
    for &px in pixels {
        if px == TRANSPARENT_COLOR_KEY_16 {
            color_rgba.extend_from_slice(&[0, 0, 0, 0]);
            shadow_rgba.extend_from_slice(&[0, 0, 0, 0]);
        } else if px == SHADOW_KEY {
            color_rgba.extend_from_slice(&[0, 0, 0, 0]);
            shadow_rgba.extend_from_slice(&[0, 0, 0, 255]);
            has_shadow = true;
        } else {
            let (r, g, b) = rgb565_to_rgb8(px);
            color_rgba.extend_from_slice(&[r, g, b, 255]);
            shadow_rgba.extend_from_slice(&[0, 0, 0, 0]);
        }
    }

    let color = renderer.create_rgba_gpu_image(w, h, &color_rgba, "cursor color")?;
    let shadow = if has_shadow {
        renderer.create_rgba_gpu_image(w, h, &shadow_rgba, "cursor shadow")
    } else {
        None
    };
    Some(CursorFrame {
        color: Some(color),
        shadow,
        width: w,
        height: h,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_renderer_new_defaults() {
        let cr = CursorRenderer::new();
        assert!(cr.frames.is_empty());
        assert_eq!(cr.hotspot_x, 0.0);
        assert_eq!(cr.hotspot_y, 0.0);
        assert_eq!(cr.current_cursor_id, -1);
        assert_eq!(cr.current_frame, 0);
        assert!(!cr.os_cursor_hidden);
    }

    #[test]
    fn advance_animation_single_frame_stays_put() {
        let mut cr = CursorRenderer::new();
        cr.frames.push(CursorFrame::test_valid());
        cr.frame_length = 2;
        cr.advance_animation();
        cr.advance_animation();
        cr.advance_animation();
        assert_eq!(cr.current_frame, 0);
    }

    #[test]
    fn advance_animation_wraps_around() {
        let mut cr = CursorRenderer::new();
        for _ in 0..3 {
            cr.frames.push(CursorFrame::test_valid());
        }
        cr.frame_length = 1;
        cr.advance_animation();
        assert_eq!(cr.current_frame, 1);
        cr.advance_animation();
        assert_eq!(cr.current_frame, 2);
        cr.advance_animation();
        assert_eq!(cr.current_frame, 0);
    }

    #[test]
    fn advance_animation_wraps_invalid_u16_sentinel_frame() {
        let mut cr = CursorRenderer::new();
        for _ in 0..3 {
            cr.frames.push(CursorFrame::test_valid());
        }
        cr.current_frame = u16::MAX;
        cr.frame_length = 1;

        cr.advance_animation();

        assert_eq!(cr.current_frame, 0);
    }

    #[test]
    fn advance_animation_waits_for_frame_length() {
        let mut cr = CursorRenderer::new();
        for _ in 0..2 {
            cr.frames.push(CursorFrame::test_valid());
        }
        cr.frame_length = 3;
        cr.advance_animation();
        assert_eq!(cr.current_frame, 0);
        cr.advance_animation();
        assert_eq!(cr.current_frame, 0);
        cr.advance_animation();
        assert_eq!(cr.current_frame, 1);
    }

    #[test]
    fn advance_animation_skips_invalid_frames() {
        let mut cr = CursorRenderer::new();
        cr.frames.push(CursorFrame::test_valid());
        cr.frames.push(CursorFrame::invalid());
        cr.frames.push(CursorFrame::test_valid());
        cr.frame_length = 1;
        cr.advance_animation();
        // Should skip the invalid frame and land on index 2.
        assert_eq!(cr.current_frame, 2);
    }
}
