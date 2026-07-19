//! Visual markers — host-side renderer for `SelectionMark`.
//!
//! The sim state (`GroundMark`, `SelectionMark`) lives in
//! `robin_engine::markers`; this module only provides the GPU renderer for the
//! selection circle sprite.

use robin_assets::picture::Picture;

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
        let mut pixels = picture_pixels_u16(pic);
        apply_arno_law(&mut pixels, shadow_color);

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
        let rgba = selection_mark_rgba(&pixels, pic.width, pic.height, in_combat);
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

fn selection_mark_rgba(pixels: &[u16], width: u16, height: u16, in_combat: bool) -> Vec<u8> {
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    for sy in 0..height as usize {
        for sx in 0..width as usize {
            let src = pixels[sy * width as usize + sx];
            if src == TRANSPARENT_COLOR_KEY_16 {
                continue;
            }
            let alpha = if in_combat {
                let a = ((src >> 8) & 0xF8) << 1;
                a.min(255) as u8
            } else {
                ((src >> 3) & 0xFC) as u8
            };
            if alpha == 0 {
                continue;
            }
            let di = (sy * width as usize + sx) * 4;
            rgba[di] = ((src >> 8) & 0xF8) as u8;
            rgba[di + 1] = ((src >> 3) & 0xFC) as u8;
            rgba[di + 2] = ((src << 3) & 0xF8) as u8;
            rgba[di + 3] = alpha;
        }
    }
    rgba
}

pub(crate) fn apply_arno_law(pixels: &mut [u16], shadow_color: u16) {
    for p in pixels.iter_mut() {
        if *p == shadow_color {
            *p = p.wrapping_add(1);
        }
        if *p == SHADOW_KEY {
            *p = shadow_color;
        }
    }
}

fn picture_pixels_u16(pic: &Picture) -> Vec<u16> {
    pic.data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}
