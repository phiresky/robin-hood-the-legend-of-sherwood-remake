//! GPU sprite atlas.
//!
//! Historically every decoded sprite frame became its own
//! `wgpu::Texture` + `TextureView` + `BindGroup`, so a frame that drew
//! N sprites issued N `set_bind_group` calls and N single-quad draws,
//! and mission load paid one texture allocation per sprite.
//!
//! This module packs those decoded frames into a small number of large
//! textures ("layers"). A sprite becomes a *sub-rect* of a layer, so
//! consecutive sprites resolved from the same layer share one bind
//! group and collapse into a single draw range.
//!
//! ## Pixel identity
//!
//! Packing is an exact transformation and is deliberately kept that
//! way. The bytes written into a layer are byte-for-byte the RGBA the
//! per-sprite texture path uploaded; the layer uses the same
//! `Rgba8UnormSrgb` format and the same NEAREST sampler; and the UV
//! sub-rect maps each destination fragment to the same texel it used to
//! land on. Concretely, for a `w`-wide sprite at layer origin `x`, the
//! old path sampled texel `floor(u * w)` and the new one samples
//! `floor(x + u * w) - x` — identical for integral `x`, and the
//! fragment sampling points sit at texel centres (half a texel from any
//! boundary) so float rounding at layer scale (~1e-4) cannot move them.
//!
//! Every sub-rect is surrounded by a [`GUTTER`]-texel transparent
//! border. NEAREST sampling of an exact sub-rect never reads it — the
//! gutter exists so that *no* sampling mistake can bleed a neighbouring
//! sprite into this one; it fails to transparent instead.

use crate::window::GpuContext;

use super::make_tex_bg;

/// Transparent border kept around every packed sprite. See the module
/// docs: correctness does not depend on it, containment does.
const GUTTER: u32 = 1;

/// Edge length of an atlas layer. 1024² × RGBA8 = 4 MiB.
///
/// Layers are reserved whole, so this is a memory-vs-binds dial: every
/// layer a frame touches costs one texture bind, but every layer that
/// is only partly filled wastes its remainder. Measured on real
/// missions, a fixed 2048 (16 MiB) sat at 17–26% occupancy, and a
/// doubling 512→1024→2048 schedule reached 21 MiB at 20% because
/// spilling into the next size step over-reserves so badly. A uniform
/// 1024 grows in fine-grained 4 MiB steps that track actual sprite
/// area, while keeping the layer count — and so the bind count — in
/// the low single digits for a mission.
const LAYER_SIZE: u32 = 1024;

/// Where a sprite ended up inside the atlas.
#[derive(Clone, Copy, Debug)]
pub(super) struct AtlasSlot {
    /// Index into [`SpriteAtlas::layers`] — also the bind-group identity
    /// the draw path uses to elide redundant binds.
    pub layer: u32,
    /// `(u0, v0, u1, v1)` sub-rect in 0..1 layer coords.
    pub uv: [f32; 4],
    pub width: u16,
    pub height: u16,
}

/// One shelf (row) of the packer.
#[derive(Clone, Copy)]
struct Shelf {
    /// Top edge of the shelf in layer texels.
    y: u32,
    /// Height reserved by the first sprite placed on this shelf,
    /// gutters included.
    height: u32,
    /// Next free x in this shelf.
    cursor: u32,
}

/// Shelf-based rectangle packer for one layer.
///
/// Kept free of GPU types so the placement arithmetic — the part that
/// has to be right — is directly unit-testable.
struct ShelfPacker {
    size: u32,
    shelves: Vec<Shelf>,
    /// Next unused y; where a new shelf would start.
    next_y: u32,
    /// Texels actually handed out, for the occupancy report.
    used_texels: u64,
}

impl ShelfPacker {
    fn new(size: u32) -> Self {
        Self {
            size,
            shelves: Vec::new(),
            next_y: 0,
            used_texels: 0,
        }
    }

    /// Reserve a `w × h` region. Returns the top-left texel of the
    /// *sprite*, with its gutter already skipped, or `None` when this
    /// layer cannot fit it.
    fn reserve(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let need_w = w + GUTTER * 2;
        let need_h = h + GUTTER * 2;
        if need_w > self.size || need_h > self.size {
            return None;
        }

        // Best fit: the open shelf that fits and wastes the least
        // vertical space, so tall sprites don't poison a shelf of short
        // ones.
        let mut best: Option<usize> = None;
        for (i, shelf) in self.shelves.iter().enumerate() {
            if shelf.height < need_h || shelf.cursor + need_w > self.size {
                continue;
            }
            if best.is_none_or(|b| shelf.height < self.shelves[b].height) {
                best = Some(i);
            }
        }

        // Tried and rejected: opening a fresh right-height shelf whenever
        // the best fit would waste more than a quarter of the sprite's
        // height. It sounds better and measures worse — eagerly starting
        // shelves burns the layer's vertical budget, so the demo scene
        // went from 2 layers/8 MiB/64% packed to 3 layers/12 MiB/51%.
        // Filling an imperfect shelf beats reserving a perfect one.
        let (x, y) = if let Some(i) = best {
            let shelf = &mut self.shelves[i];
            let x = shelf.cursor;
            shelf.cursor += need_w;
            (x, shelf.y)
        } else {
            if self.next_y + need_h > self.size {
                return None;
            }
            let y = self.next_y;
            self.next_y += need_h;
            self.shelves.push(Shelf {
                y,
                height: need_h,
                cursor: need_w,
            });
            (0, y)
        };

        self.used_texels += u64::from(w) * u64::from(h);
        Some((x + GUTTER, y + GUTTER))
    }
}

struct AtlasLayer {
    /// Held alive for the bind group's lifetime.
    texture: wgpu::Texture,
    _view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    packer: ShelfPacker,
}

impl AtlasLayer {
    fn new(
        gpu: &GpuContext,
        bgl: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        size: u32,
    ) -> Self {
        // WebGPU zero-initializes textures, so gutters and any
        // never-allocated region read as transparent without an
        // explicit clear.
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sprite atlas layer"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Must match the per-sprite path exactly — sRGB-encoded
            // RGBA8. Anything else changes the blend result.
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = make_tex_bg(&gpu.device, bgl, &view, sampler, "sprite atlas bg");
        Self {
            texture,
            _view: view,
            bind_group,
            packer: ShelfPacker::new(size),
        }
    }
}

/// Occupancy snapshot for the memory report.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct AtlasStats {
    pub layers: usize,
    /// Total texels reserved by all layers (their full area).
    pub layer_texels: u64,
    /// Texels actually covered by sprite pixels.
    pub used_texels: u64,
    /// Texels the packer has committed to shelves, i.e. everything below
    /// each layer's high-water mark. The difference from `used_texels` is
    /// true packing overhead (gutters and shelf-height slack); the
    /// difference from `layer_texels` is unused tail capacity that later
    /// sprites will occupy.
    pub committed_texels: u64,
    pub sprites: usize,
}

impl AtlasStats {
    /// Resident GPU bytes held by the atlas layers.
    pub fn bytes(&self) -> u64 {
        self.layer_texels * 4
    }

    /// Fraction of reserved texels carrying sprite pixels.
    pub fn occupancy(&self) -> f32 {
        if self.layer_texels == 0 {
            0.0
        } else {
            self.used_texels as f32 / self.layer_texels as f32
        }
    }

    /// Sprite pixels as a fraction of what the packer actually committed.
    /// This is the packer's own efficiency, with the not-yet-filled tail
    /// of the newest layer excluded — the number to judge the packer by.
    pub fn packing_efficiency(&self) -> f32 {
        if self.committed_texels == 0 {
            0.0
        } else {
            self.used_texels as f32 / self.committed_texels as f32
        }
    }
}

/// Packs decoded sprite frames into shared textures.
#[derive(Default)]
pub(super) struct SpriteAtlas {
    layers: Vec<AtlasLayer>,
    sprites: usize,
}

impl SpriteAtlas {
    /// Bind group for a previously returned [`AtlasSlot::layer`].
    pub(super) fn bind_group(&self, layer: u32) -> &wgpu::BindGroup {
        &self.layers[layer as usize].bind_group
    }

    pub(super) fn stats(&self) -> AtlasStats {
        AtlasStats {
            layers: self.layers.len(),
            layer_texels: self
                .layers
                .iter()
                .map(|l| u64::from(l.packer.size) * u64::from(l.packer.size))
                .sum(),
            used_texels: self.layers.iter().map(|l| l.packer.used_texels).sum(),
            committed_texels: self
                .layers
                .iter()
                .map(|l| u64::from(l.packer.next_y) * u64::from(l.packer.size))
                .sum(),
            sprites: self.sprites,
        }
    }

    /// Pack `rgba` (`width × height`, tightly packed RGBA8) into a layer
    /// and upload it.
    ///
    /// `rgba` is uploaded verbatim — this performs no colour conversion
    /// whatsoever, which is what keeps the atlas path pixel-identical to
    /// the per-sprite texture path.
    pub(super) fn insert(
        &mut self,
        gpu: &GpuContext,
        bgl: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        width: u16,
        height: u16,
        rgba: &[u8],
    ) -> AtlasSlot {
        let w = u32::from(width);
        let h = u32::from(height);
        let expected = (w as usize) * (h as usize) * 4;
        // `>=`, not `==`: `sprite_rgba_for_upload` hands back a
        // runtime PNG overlay's buffer verbatim, and the per-sprite
        // `write_texture` this replaces likewise consumed only the
        // first `w*h*4` bytes. A short buffer is a real bug, so that
        // still trips.
        assert!(
            rgba.len() >= expected,
            "atlas insert: {width}x{height} sprite needs {expected} RGBA bytes, got {}",
            rgba.len()
        );
        let rgba = &rgba[..expected];

        // Try existing layers, then a fresh one. A sprite larger than
        // the standard layer gets a private layer sized to fit — rare,
        // but it must not silently fail.
        let mut placed = None;
        for (i, layer) in self.layers.iter_mut().enumerate() {
            if let Some((x, y)) = layer.packer.reserve(w, h) {
                placed = Some((i, x, y));
                break;
            }
        }
        let (layer_idx, x, y) = match placed {
            Some(found) => found,
            None => {
                let limit = gpu.device.limits().max_texture_dimension_2d;
                // A sprite too big for the scheduled layer size gets a
                // layer sized to fit it instead — rare, but it must not
                // silently fail.
                let needed = (w.max(h) + GUTTER * 2).next_power_of_two();
                let size = LAYER_SIZE.min(limit).max(needed);
                assert!(
                    size <= limit,
                    "sprite {width}x{height} exceeds the device's \
                     max_texture_dimension_2d of {limit}"
                );
                tracing::debug!(
                    "sprite atlas: new layer {} at {size}x{size} ({} sprites so far)",
                    self.layers.len(),
                    self.sprites,
                );
                self.layers.push(AtlasLayer::new(gpu, bgl, sampler, size));
                let i = self.layers.len() - 1;
                let (x, y) = self.layers[i]
                    .packer
                    .reserve(w, h)
                    .expect("freshly sized atlas layer must fit the sprite it was sized for");
                (i, x, y)
            }
        };

        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.layers[layer_idx].texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.sprites += 1;

        let size = self.layers[layer_idx].packer.size as f32;
        AtlasSlot {
            layer: layer_idx as u32,
            uv: [
                x as f32 / size,
                y as f32 / size,
                (x + w) as f32 / size,
                (y + h) as f32 / size,
            ],
            width,
            height,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The packer must never hand out overlapping regions, and every
    /// returned rect must sit inside the layer with its gutter intact.
    #[test]
    fn shelf_packer_produces_disjoint_gutter_separated_rects() {
        let size = 256;
        let boxes: Vec<(u32, u32)> = (0..80u32)
            .map(|i| (7 + (i * 13) % 40, 5 + (i * 7) % 30))
            .collect();
        let mut packer = ShelfPacker::new(size);
        let mut placed: Vec<(u32, u32, u32, u32)> = Vec::new();
        for &(w, h) in &boxes {
            let Some((x, y)) = packer.reserve(w, h) else {
                continue;
            };
            assert!(x >= GUTTER && y >= GUTTER, "gutter missing at {x},{y}");
            assert!(x + w + GUTTER <= size, "overflows right edge");
            assert!(y + h + GUTTER <= size, "overflows bottom edge");
            for &(px, py, pw, ph) in &placed {
                let disjoint = x + w <= px || px + pw <= x || y + h <= py || py + ph <= y;
                assert!(
                    disjoint,
                    "rect {x},{y} {w}x{h} overlaps {px},{py} {pw}x{ph}"
                );
            }
            placed.push((x, y, w, h));
        }
        assert!(
            placed.len() > 30,
            "packer rejected too much: {} of {}",
            placed.len(),
            boxes.len()
        );
    }

    /// The gutter has to be counted against the layer bound, or a
    /// sprite flush against the right edge would have no border.
    #[test]
    fn oversize_reservation_is_refused_rather_than_clipped() {
        let mut packer = ShelfPacker::new(64);
        assert!(packer.reserve(64, 8).is_none(), "gutter must be accounted");
        assert!(packer.reserve(62, 8).is_some());
    }

    /// Occupancy counts sprite texels, not the gutters around them.
    #[test]
    fn used_texels_tracks_sprite_area_only() {
        let mut packer = ShelfPacker::new(256);
        packer.reserve(10, 20).expect("fits");
        packer.reserve(4, 4).expect("fits");
        assert_eq!(packer.used_texels, 10 * 20 + 4 * 4);
    }
}
