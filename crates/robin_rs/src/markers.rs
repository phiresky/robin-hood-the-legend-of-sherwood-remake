//! Visual markers — host-side renderer for `SelectionMark`.
//!
//! The sim state (`GroundMark`, `SelectionMark`) lives in
//! `robin_engine::markers`; this module only provides the GPU renderer for the
//! selection circle sprite.

use crate::gfx_types::{BlendMode, Rect};
use crate::renderer::{Renderer, TRANSPARENT_COLOR_KEY_16};
use robin_assets::resource_manager::ResourceManager;
use robin_engine::resource_ids::{RHID_GROUND_SELECT, RHID_GROUND_SELECT_SWORD};

/// The "pure blue" shadow sentinel used in raw sprite pixel data.
const SHADOW_KEY: u16 = 0x001F;

/// Fallback shadow color when the engine hasn't loaded a level yet.
const DEFAULT_SHADOW_COLOR: u16 = 0x2964;

struct SelectionMarkFrame {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

struct SelectionMarkRow {
    frames: Vec<SelectionMarkFrame>,
    width: u16,
    height: u16,
}

pub struct SelectionMarkRenderer {
    idle: Option<SelectionMarkRow>,
    sword: Option<SelectionMarkRow>,
}

impl Default for SelectionMarkRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl SelectionMarkRenderer {
    pub fn new() -> Self {
        Self {
            idle: None,
            sword: None,
        }
    }

    pub fn load(
        &mut self,
        resource_manager: &mut ResourceManager,
        renderer: &Renderer,
        shadow_color: u16,
    ) {
        let sc = if shadow_color == 0 {
            DEFAULT_SHADOW_COLOR
        } else {
            shadow_color
        };
        self.idle = load_row(
            renderer,
            resource_manager,
            RHID_GROUND_SELECT,
            "idle",
            sc,
            false,
        );
        self.sword = load_row(
            renderer,
            resource_manager,
            RHID_GROUND_SELECT_SWORD,
            "combat",
            sc,
            true,
        );
    }

    pub fn draw(
        &mut self,
        renderer: &mut Renderer,
        frame: u16,
        in_combat: bool,
        screen_x: i32,
        screen_y: i32,
    ) {
        let row = match if in_combat {
            self.sword.as_ref()
        } else {
            self.idle.as_ref()
        } {
            Some(r) => r,
            None => return,
        };

        if row.frames.is_empty() || row.width == 0 || row.height == 0 {
            return;
        }
        let frame_idx = (frame as usize).min(row.frames.len() - 1);
        let frame = &row.frames[frame_idx];
        let sw = row.width;
        let sh = row.height;

        let dst_x = screen_x - sw as i32 / 2;
        let dst_y = screen_y - sh as i32 / 2;

        let dst_rect = Rect::new(dst_x, dst_y, sw as u32, sh as u32);
        renderer.enqueue_external_texture(
            &frame.view,
            dst_rect,
            [0.0, 0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
            BlendMode::Blend,
        );
    }
}

fn load_row(
    renderer: &Renderer,
    resource_manager: &mut ResourceManager,
    resource_id: i32,
    label: &str,
    shadow_color: u16,
    in_combat: bool,
) -> Option<SelectionMarkRow> {
    let pictures = match resource_manager.get_pictures(resource_id) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("SelectionMark: failed to load {label} sprite ({resource_id}): {e}");
            return None;
        }
    };

    if pictures.is_empty() {
        tracing::warn!("SelectionMark: {label} sprite ({resource_id}) has no frames");
        return None;
    }

    let mut frames: Vec<SelectionMarkFrame> = Vec::with_capacity(pictures.len());
    let mut width = 0u16;
    let mut height = 0u16;

    for (i, slot) in pictures.iter().enumerate() {
        let Some(pic) = slot else {
            tracing::warn!("SelectionMark: {label} sprite ({resource_id}) frame {i} is empty");
            return None;
        };
        if i == 0 {
            width = pic.width;
            height = pic.height;
        } else if pic.width != width || pic.height != height {
            tracing::warn!(
                "SelectionMark: {label} sprite frame {i} size mismatch ({}x{} vs {}x{})",
                pic.width,
                pic.height,
                width,
                height
            );
            return None;
        }
        let rgba = selection_mark_rgba(&pic.data, pic.width, pic.height, shadow_color, in_combat);
        let (texture, view) = renderer.create_static_rgba_texture(
            &rgba,
            pic.width as u32,
            pic.height as u32,
            &format!("selection mark {label} frame {i}"),
        );
        frames.push(SelectionMarkFrame {
            _texture: texture,
            view,
        });
    }

    tracing::info!(
        "SelectionMark: loaded {label} sprite ({resource_id}): {} frames {}x{}",
        frames.len(),
        width,
        height
    );
    Some(SelectionMarkRow {
        frames,
        width,
        height,
    })
}

fn selection_mark_rgba(
    pixels: &[u8],
    width: u16,
    height: u16,
    shadow_color: u16,
    in_combat: bool,
) -> Vec<u8> {
    let pixel_count = width as usize * height as usize;
    // Slicing validates the complete frame before conversion; trailing resource
    // bytes are not pixels. Missing frame data must not become transparent padding.
    let pixels = pixels[..pixel_count * 2].as_chunks::<2>().0;
    let mut rgba = vec![0u8; pixel_count * 4];
    for (src, dst) in pixels.iter().zip(rgba.as_chunks_mut::<4>().0) {
        let src = remap_shadow_pixel(u16::from_le_bytes(*src), shadow_color);
        if src == TRANSPARENT_COLOR_KEY_16 {
            continue;
        }
        let (r, g, b) = robin_util::color::rgb565_to_rgb8(src);
        let alpha = if in_combat {
            (u16::from(r) * 2).min(255) as u8
        } else {
            g
        };
        if alpha != 0 {
            *dst = [r, g, b, alpha];
        }
    }
    rgba
}

pub(crate) fn apply_arno_law(pixels: &mut [u16], shadow_color: u16) {
    for pixel in pixels {
        *pixel = remap_shadow_pixel(*pixel, shadow_color);
    }
}

fn remap_shadow_pixel(mut pixel: u16, shadow_color: u16) -> u16 {
    // These replacements are ordered: advancing an authored shadow-color
    // pixel can itself produce the blue sentinel.
    if pixel == shadow_color {
        pixel = pixel.wrapping_add(1);
    }
    if pixel == SHADOW_KEY {
        pixel = shadow_color;
    }
    pixel
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_conversion_preserves_all_colors_and_ordered_shadow_replacements() {
        let pixels: Vec<u8> = (0..=u16::MAX).flat_map(u16::to_le_bytes).collect();
        for shadow_color in [
            0,
            SHADOW_KEY - 1,
            SHADOW_KEY,
            TRANSPARENT_COLOR_KEY_16,
            DEFAULT_SHADOW_COLOR,
            u16::MAX,
        ] {
            let mut remapped: Vec<u16> = (0..=u16::MAX).collect();
            apply_arno_law(&mut remapped, shadow_color);
            for in_combat in [false, true] {
                let rgba = selection_mark_rgba(&pixels, 256, 256, shadow_color, in_combat);
                assert_eq!(rgba.len(), 65536 * 4);
                for (source, actual) in (0..=u16::MAX).zip(rgba.as_chunks::<4>().0) {
                    let mut pixel = source;
                    if pixel == shadow_color {
                        pixel = pixel.wrapping_add(1);
                    }
                    if pixel == 0x001F {
                        pixel = shadow_color;
                    }
                    assert_eq!(remapped[source as usize], pixel);
                    let red = ((pixel >> 11) * 8) as u8;
                    let green = (((pixel >> 5) & 63) * 4) as u8;
                    let blue = ((pixel & 31) * 8) as u8;
                    let alpha = if in_combat {
                        (u16::from(red) * 2).min(255) as u8
                    } else {
                        green
                    };
                    let expected = if pixel == TRANSPARENT_COLOR_KEY_16 || alpha == 0 {
                        [0; 4]
                    } else {
                        [red, green, blue, alpha]
                    };
                    assert_eq!(
                        *actual, expected,
                        "source {source:#06x}, shadow {shadow_color:#06x}, combat {in_combat}"
                    );
                }
            }
        }
    }

    #[test]
    fn marker_conversion_uses_only_declared_frame_pixels() {
        let pixels = [0x00, 0xF8, 0xFF, 0xFF, 0xAA];
        assert_eq!(
            selection_mark_rgba(&pixels, 1, 1, DEFAULT_SHADOW_COLOR, true),
            [248, 0, 0, 255]
        );
        assert!(selection_mark_rgba(&pixels, 0, 1, DEFAULT_SHADOW_COLOR, true).is_empty());
    }

    #[test]
    #[should_panic]
    fn marker_conversion_rejects_incomplete_frame_data() {
        selection_mark_rgba(&[0xFF; 3], 2, 1, DEFAULT_SHADOW_COLOR, false);
    }
}
