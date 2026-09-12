//! Persistent GPU resources and caches owned across frame submissions.

use std::collections::HashMap;

use robin_assets::frame_holder::{FrameHolder, SpriteVariant};

use crate::ui::AlphaMask;
use crate::window::GpuContext;

use super::atlas::SpriteAtlas;
use super::{
    BackgroundTexture, FontAtlas, ManagedSurface, MaskAlpha, MaskAtlasBounds, OUTLINE_PAD,
    SpriteCacheKey, SpriteResidency, SpriteTextureCache, TRANSPARENT_COLOR_KEY_16, make_tex_bg,
    outline_cache_key, rgb565_to_rgba_opaque, shadow_alpha_from_level, sprite_outline_rgba,
    sprite_rgba_for_upload, upload_rgba_texture,
};

pub(super) struct GpuResources {
    pub(super) uploads: super::diagnostics::UploadCounters,
    pub(super) managed_surfaces: HashMap<u32, ManagedSurface>,
    next_id: u32,
    pub(super) bit_depth: u16,
    /// Shared textures every decoded sprite frame is packed into.
    pub(super) sprite_atlas: SpriteAtlas,
    /// `(bank_id, variant, shadow) → sub-rect of a `sprite_atlas` layer.
    pub(super) sprite_cache: SpriteTextureCache,
    pub(super) mask_alpha_cache: HashMap<u32, MaskAlpha>,
    pub(super) background_texture: Option<BackgroundTexture>,
    pub(super) sampler: wgpu::Sampler,
    pub(super) linear_sampler: wgpu::Sampler,
    pub(super) bgl_tex: wgpu::BindGroupLayout,
    _white_view: wgpu::TextureView,
    pub(super) white_bg: wgpu::BindGroup,
    font_atlas_cache: HashMap<u64, FontAtlas>,
}

impl GpuResources {
    pub(super) fn sprite_residency_stats(&self) -> super::SpriteResidencyStats {
        let atlas = self.sprite_atlas.stats();
        super::SpriteResidencyStats {
            resident_bytes: atlas.bytes(),
            layers: atlas.layers,
            entries: self.sprite_cache.entries.len(),
            distinct_frames: self
                .sprite_cache
                .entries
                .keys()
                .map(|key| (key.bank_id, key.variant))
                .collect::<std::collections::HashSet<_>>()
                .len(),
            occupied_texels: atlas.used_texels,
            committed_texels: atlas.committed_texels,
        }
    }

    /// Decompress sprite frame `(bank_id, variant)` into the GPU cache,
    /// converting RGB565 → RGBA8 with the shadow key baked into alpha.
    /// Returns `Some((width, height))` of the cached frame on success.
    pub(super) fn ensure_sprite_cached(
        &mut self,
        gpu: &GpuContext,
        frame_holder: &FrameHolder,
        bank_id: u32,
        variant: SpriteVariant,
        shadow_color: u16,
        shadow_level: u16,
    ) -> Option<(u16, u16)> {
        let shadow_alpha = shadow_alpha_from_level(shadow_level);
        let key = SpriteCacheKey {
            bank_id,
            variant,
            shadow_color: shadow_color as u32,
            shadow_alpha,
        };
        if let Some(entry) = self.sprite_cache.entries.get(&key) {
            return Some(entry.dimensions());
        }
        // A still-streaming sprite must not enter the permanent cache as a
        // blank texture — skip the draw; it pops in once its grid lands.
        if frame_holder.sprite_pixels_pending(bank_id) {
            let skips = frame_holder.note_pending_sprite_draw();
            tracing::debug!(
                bank_id,
                skips,
                "skipped draw: sprite pixels still streaming"
            );
            return None;
        }
        let w = frame_holder.sprite_width(bank_id);
        let h = frame_holder.sprite_height(bank_id);
        if w == 0 || h == 0 {
            return None;
        }
        let rgba = sprite_rgba_for_upload(
            frame_holder,
            bank_id,
            variant,
            shadow_color,
            shadow_alpha,
            self.bit_depth,
        );
        let residency = self.place_sprite(gpu, w, h, &rgba);
        self.sprite_cache.entries.insert(key, residency);
        Some((w, h))
    }

    /// Pack one decoded RGBA frame into the shared atlas.
    ///
    /// `rgba` is uploaded verbatim — no colour conversion of any kind —
    /// which is what kept this pixel-identical to the one-texture-per-
    /// sprite path it replaced.
    fn place_sprite(
        &mut self,
        gpu: &GpuContext,
        width: u16,
        height: u16,
        rgba: &[u8],
    ) -> SpriteResidency {
        // Counted where the per-sprite `upload_rgba_texture` used to be,
        // so the `uploads/f` FPS line stays comparable across the
        // migration.
        self.uploads.inc("sprite atlas insert");
        SpriteResidency(self.sprite_atlas.insert(
            gpu,
            &self.bgl_tex,
            &self.sampler,
            width,
            height,
            rgba,
        ))
    }

    /// Build the GPU cache for the edge-map outline used by the
    /// selection / mouse-over highlights. The texture is transparent
    /// except for the two outside pixels written at each horizontal
    /// sprite-body edge; the draw-time tint supplies the actual colour.
    pub(super) fn ensure_outline_cached(
        &mut self,
        gpu: &GpuContext,
        frame_holder: &FrameHolder,
        bank_id: u32,
        variant: SpriteVariant,
        shadow_color: u16,
        _shadow_level: u16,
    ) -> Option<(u16, u16)> {
        let key = outline_cache_key(bank_id, variant, shadow_color);
        if let Some(entry) = self.sprite_cache.entries.get(&key) {
            return Some(entry.dimensions());
        }
        // See `ensure_sprite_cached`: never cache a still-streaming sprite.
        if frame_holder.sprite_pixels_pending(bank_id) {
            let skips = frame_holder.note_pending_sprite_draw();
            tracing::debug!(
                bank_id,
                skips,
                "skipped draw: sprite pixels still streaming"
            );
            return None;
        }

        let w = frame_holder.sprite_width(bank_id);
        let h = frame_holder.sprite_height(bank_id);
        if w == 0 || h == 0 {
            return None;
        }

        let mut rgb565 = vec![TRANSPARENT_COLOR_KEY_16; w as usize * h as usize];
        frame_holder.uncompress_frame(
            &mut rgb565,
            w as usize,
            bank_id,
            variant,
            shadow_color,
            self.bit_depth,
        );
        let outline_w = w as usize + OUTLINE_PAD * 2;
        let rgba = sprite_outline_rgba(
            &rgb565,
            w as usize,
            h as usize,
            outline_w,
            TRANSPARENT_COLOR_KEY_16,
            shadow_color,
        );
        let residency = self.place_sprite(gpu, outline_w as u16, h, &rgba);
        self.sprite_cache.entries.insert(key, residency);
        Some((outline_w as u16, h))
    }

    /// Upload the static binary alpha for a sprite-occlusion mask. It is
    /// rasterized into stencil for each affected sprite, matching the
    /// original engine's temporary-sprite transparency operation.
    pub(super) fn upload_mask_alphas<'a>(
        &mut self,
        gpu: &GpuContext,
        masks: impl IntoIterator<Item = (u32, &'a [u8], u16, u16)>,
    ) -> Result<(), String> {
        let mut masks: Vec<_> = masks.into_iter().collect();
        let limit = gpu.device.limits().max_texture_dimension_2d;
        let mut ids = std::collections::HashSet::new();
        for &(id, bytes, w, h) in &masks {
            validate_binary_mask(id, bytes.len(), w, h, limit)?;
            if !ids.insert(id) {
                return Err(format!("duplicate sprite mask {id}"));
            }
        }
        let enabled = mask_atlas_enabled();
        let edge = limit.min(2048);
        // Tall masks first keeps shelf waste small. Stable sorting keeps equal
        // dimensions deterministic and leaves mission mask IDs unchanged.
        masks.sort_by_key(|&(_, _, w, h)| std::cmp::Reverse((h, w)));
        let mut pending = Vec::new();
        for mask @ (id, bytes, w, h) in masks {
            if !enabled || u32::from(w) + 2 > edge || u32::from(h) + 2 > edge {
                self.upload_mask_alpha(gpu, id, bytes, w, h)?;
            } else {
                pending.push(mask);
            }
        }
        let mut pages = 0;
        let mut occupied_texels = 0u64;
        let mut uploaded_texels = 0u64;
        while !pending.is_empty() {
            let mut packer = super::atlas::ShelfPacker::new(edge);
            let mut slots = Vec::new();
            let mut extent = [0, 0];
            pending.retain(|&(id, bytes, w, h)| {
                if let Some((x, y)) = packer.reserve(u32::from(w), u32::from(h)) {
                    extent[0] = extent[0].max(x + u32::from(w) + 1);
                    extent[1] = extent[1].max(y + u32::from(h) + 1);
                    slots.push((
                        id,
                        bytes,
                        MaskAtlasBounds {
                            origin: [x, y],
                            size: [u32::from(w), u32::from(h)],
                        },
                    ));
                    false
                } else {
                    true
                }
            });
            assert!(
                !slots.is_empty(),
                "validated mask must fit an empty atlas page"
            );
            uploaded_texels += u64::from(extent[0]) * u64::from(extent[1]);
            let mut pixels = vec![0; (extent[0] * extent[1]) as usize];
            for &(_, bytes, bounds) in &slots {
                let [x, y] = bounds.origin;
                let [w, h] = bounds.size;
                occupied_texels += u64::from(w) * u64::from(h);
                // Replicate edge texels in the one-pixel gutter. Shader clamps
                // before textureLoad too, including far outside local 0..1 UV.
                for dy in 0..h + 2 {
                    let sy = dy.saturating_sub(1).min(h - 1);
                    let source = &bytes[(sy * w) as usize..((sy + 1) * w) as usize];
                    let start = ((y + dy - 1) * extent[0] + x - 1) as usize;
                    let row = &mut pixels[start..start + w as usize + 2];
                    row[0] = source[0];
                    row[1..w as usize + 1].copy_from_slice(source);
                    row[w as usize + 1] = source[w as usize - 1];
                }
            }
            // Reuse the standalone allocation/upload contract once per page,
            // then share its handles; no additional per-mask GPU allocations.
            let page_id = slots[0].0;
            self.upload_mask_alpha(gpu, page_id, &pixels, extent[0] as u16, extent[1] as u16)?;
            let page = self
                .mask_alpha_cache
                .remove(&page_id)
                .expect("just uploaded mask page");
            for (id, _, bounds) in slots {
                self.mask_alpha_cache.insert(
                    id,
                    MaskAlpha {
                        _texture: page._texture.clone(),
                        _view: page._view.clone(),
                        bind_group: page.bind_group.clone(),
                        width: bounds.size[0],
                        height: bounds.size[1],
                        atlas: Some(bounds),
                    },
                );
            }
            pages += 1;
        }
        tracing::debug!(
            pages,
            enabled,
            occupied_texels,
            uploaded_texels,
            "Uploaded binary mask atlas pages"
        );
        Ok(())
    }

    pub(super) fn upload_mask_alpha(
        &mut self,
        gpu: &GpuContext,
        mask_index: u32,
        bitmap: &[u8],
        mask_w: u16,
        mask_h: u16,
    ) -> Result<(), String> {
        let pixels = validate_binary_mask(
            mask_index,
            bitmap.len(),
            mask_w,
            mask_h,
            gpu.device.limits().max_texture_dimension_2d,
        )?;
        // Preserve original binary bytes. The nearest-sampled binary shader
        // tests nonzero, so expanding every byte to 255 would only add a full
        // bitmap allocation/pass (including unused atlas page space).
        self.uploads.inc("mask alpha");
        let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("mask alpha {mask_index}")),
            size: wgpu::Extent3d {
                width: mask_w as u32,
                height: mask_h as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bitmap[..pixels],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(mask_w as u32),
                rows_per_image: Some(mask_h as u32),
            },
            wgpu::Extent3d {
                width: mask_w as u32,
                height: mask_h as u32,
                depth_or_array_layers: 1,
            },
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = make_tex_bg(
            &gpu.device,
            &self.bgl_tex,
            &view,
            &self.sampler,
            "sprite mask alpha bg",
        );
        self.mask_alpha_cache.insert(
            mask_index,
            MaskAlpha {
                _texture: tex,
                _view: view,
                bind_group,
                width: mask_w as u32,
                height: mask_h as u32,
                atlas: None,
            },
        );
        Ok(())
    }

    pub(super) fn upload_occlusion_depth(
        &mut self,
        gpu: &GpuContext,
        texture_index: u32,
        depth: &[u16],
        width: u16,
        height: u16,
    ) -> Result<(), String> {
        let encoded_len = validate_depth_mask(
            depth.len(),
            width,
            height,
            gpu.device.limits().max_texture_dimension_2d,
        )?;
        let width = u32::from(width);
        let height = u32::from(height);
        self.uploads.inc("occlusion depth");
        // Store the high/low bytes separately. R16Unorm requires an optional
        // native wgpu feature, while Rg8Unorm is portable to WebGL/WebGPU too.
        let mut encoded = Vec::with_capacity(encoded_len);
        for value in depth {
            encoded.extend_from_slice(&value.to_be_bytes());
        }
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("continuous occlusion depth"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &encoded,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 2),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = make_tex_bg(
            &gpu.device,
            &self.bgl_tex,
            &view,
            &self.sampler,
            "continuous occlusion depth bg",
        );
        self.mask_alpha_cache.insert(
            texture_index,
            MaskAlpha {
                _texture: texture,
                _view: view,
                bind_group,
                width,
                height,
                atlas: None,
            },
        );
        Ok(())
    }
}

impl GpuResources {
    pub(super) fn new(
        sampler: wgpu::Sampler,
        linear_sampler: wgpu::Sampler,
        bgl_tex: wgpu::BindGroupLayout,
        white_view: wgpu::TextureView,
        white_bg: wgpu::BindGroup,
    ) -> Self {
        Self {
            uploads: Default::default(),
            managed_surfaces: HashMap::new(),
            next_id: 2,
            bit_depth: 16,
            sprite_atlas: SpriteAtlas::default(),
            sprite_cache: SpriteTextureCache::default(),
            mask_alpha_cache: HashMap::new(),
            background_texture: None,
            sampler,
            linear_sampler,
            bgl_tex,
            _white_view: white_view,
            white_bg,
            font_atlas_cache: HashMap::new(),
        }
    }

    #[inline]
    pub(super) fn resolve_surface_id(id: u32) -> u32 {
        if id == 1 { 0 } else { id }
    }

    pub(super) fn insert_managed_surface(&mut self, surface: ManagedSurface) -> u32 {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("renderer surface IDs exhausted");
        self.managed_surfaces.insert(id, surface);
        id
    }

    pub(super) fn delete_managed_surface(&mut self, id: u32) -> bool {
        let id = Self::resolve_surface_id(id);
        id != 0 && self.managed_surfaces.remove(&id).is_some()
    }

    pub(super) fn surface_dimensions(&self, id: u32) -> Option<(u16, u16)> {
        let id = Self::resolve_surface_id(id);
        self.managed_surfaces
            .get(&id)
            .map(|surface| (surface.width, surface.height))
    }

    pub(super) fn alpha_mask(&self, id: u32) -> Option<AlphaMask> {
        let id = Self::resolve_surface_id(id);
        self.managed_surfaces
            .get(&id)
            .map(|surface| surface.alpha_mask.clone())
    }

    pub(super) fn set_shadow_alpha(&mut self, id: u32, shadow_alpha: u8) {
        let id = Self::resolve_surface_id(id);
        let surface = self.managed_surfaces.get_mut(&id).unwrap_or_else(|| {
            panic!("cannot update shadow alpha of missing renderer surface {id}")
        });
        surface.shadow_alpha = shadow_alpha;
    }

    pub(super) fn clear_mask_alpha_cache(&mut self) {
        self.mask_alpha_cache.clear();
    }

    pub(super) fn upload_background_texture(
        &mut self,
        gpu: &GpuContext,
        width: u32,
        height: u32,
        pixels: &[u16],
    ) -> bool {
        if width == 0 || height == 0 || pixels.len() != width as usize * height as usize {
            return false;
        }
        let rgba = rgb565_to_rgba_opaque(pixels, width as usize, height as usize);
        let (_texture, view) = upload_rgba_texture(
            gpu,
            &self.uploads,
            &rgba,
            width,
            height,
            "background texture",
        );
        let bind_group = make_tex_bg(
            &gpu.device,
            &self.bgl_tex,
            &view,
            &self.sampler,
            "background bg",
        );
        self.background_texture = Some(BackgroundTexture {
            _view: view,
            bind_group,
            width,
            height,
        });
        true
    }

    pub(super) fn ensure_font_atlas(
        &mut self,
        gpu: &GpuContext,
        font: &crate::native_font::NativeFont,
    ) -> wgpu::BindGroup {
        let font_id = font.atlas_cache_identity();
        if let Some(atlas) = self.font_atlas_cache.get(&font_id) {
            return atlas.bind_group.clone();
        }
        // A cache miss is an opportunity to reclaim fonts replaced since the
        // last upload. Queued bind groups retain any in-flight GPU resources.
        self.font_atlas_cache
            .retain(|_, atlas| atlas.font_lifetime.strong_count() > 0);
        let (rgba, width, height) = font.build_rgba_atlas();
        let (texture, view) =
            upload_rgba_texture(gpu, &self.uploads, &rgba, width, height, "font atlas");
        let bind_group = make_tex_bg(
            &gpu.device,
            &self.bgl_tex,
            &view,
            &self.sampler,
            "font atlas bg",
        );
        self.font_atlas_cache.insert(
            font_id,
            FontAtlas {
                font_lifetime: font.atlas_lifetime(),
                _texture: texture,
                _view: view,
                bind_group: bind_group.clone(),
            },
        );
        bind_group
    }

    pub(super) fn clear_font_atlas_cache(&mut self) {
        self.font_atlas_cache.clear();
    }
}

fn mask_pixel_count(width: u16, height: u16, limit: u32) -> Result<usize, String> {
    if width == 0 || height == 0 || u32::from(width).max(u32::from(height)) > limit {
        return Err(format!(
            "invalid mask extent {width}x{height} (device limit {limit})"
        ));
    }
    Ok(usize::from(width) * usize::from(height))
}

fn validate_binary_mask(
    id: u32,
    bytes: usize,
    width: u16,
    height: u16,
    limit: u32,
) -> Result<usize, String> {
    let pixels = mask_pixel_count(width, height, limit)
        .map_err(|error| format!("sprite mask {id}: {error}"))?;
    if bytes < pixels {
        return Err(format!(
            "sprite mask {id}: {width}x{height} requires {pixels} bytes, got {bytes}"
        ));
    }
    Ok(pixels)
}

fn validate_depth_mask(
    values: usize,
    width: u16,
    height: u16,
    limit: u32,
) -> Result<usize, String> {
    let pixels = mask_pixel_count(width, height, limit)?;
    if values != pixels {
        return Err(format!(
            "occlusion depth {width}x{height} requires {pixels} values, got {values}"
        ));
    }
    pixels
        .checked_mul(2)
        .ok_or_else(|| "occlusion depth byte count overflow".to_string())
}

#[cfg(test)]
mod mask_upload_tests {
    use super::*;

    #[test]
    fn binary_masks_require_a_complete_prefix_and_valid_device_extent() {
        assert_eq!(validate_binary_mask(7, 6, 3, 2, 8), Ok(6));
        assert_eq!(validate_binary_mask(7, 9, 3, 2, 8), Ok(6));
        assert!(
            validate_binary_mask(7, 5, 3, 2, 8)
                .unwrap_err()
                .contains("requires 6 bytes, got 5")
        );
        for (width, height, limit) in [(0, 2, 8), (3, 0, 8), (9, 2, 8), (2, 9, 8)] {
            let error = validate_binary_mask(7, 100, width, height, limit).unwrap_err();
            assert!(error.contains("sprite mask 7"));
            assert!(error.contains("invalid mask extent"));
        }
    }

    #[test]
    fn depth_masks_require_exact_shape_and_compute_encoded_size() {
        assert_eq!(validate_depth_mask(6, 3, 2, 8), Ok(12));
        assert!(validate_depth_mask(5, 3, 2, 8).is_err());
        assert!(validate_depth_mask(7, 3, 2, 8).is_err());
        assert!(validate_depth_mask(6, 3, 2, 2).is_err());
        assert!(validate_depth_mask(0, 0, 2, 8).is_err());
    }
}

/// Developer-only A/B control, deliberately absent from the product UI.
fn mask_atlas_enabled() -> bool {
    #[cfg(target_arch = "wasm32")]
    let value = web_sys::window()
        .and_then(|window| window.location().search().ok())
        .and_then(|search| web_sys::UrlSearchParams::new_with_str(&search).ok())
        .and_then(|params| params.get("mask-atlas"));
    #[cfg(not(target_arch = "wasm32"))]
    let value = std::env::var("ROBIN_MASK_ATLAS").ok();
    match value.as_deref() {
        None | Some("1") => true,
        Some("0") => false,
        Some(other) => {
            tracing::warn!(
                value = other,
                "Invalid mask atlas override; expected 0 or 1"
            );
            true
        }
    }
}
