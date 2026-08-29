//! Persistent GPU resources and caches owned across frame submissions.

use std::collections::HashMap;

use robin_assets::frame_holder::{FrameHolder, SpriteVariant};

use crate::ui::AlphaMask;
use crate::window::GpuContext;

use super::atlas::SpriteAtlas;
use super::{
    BackgroundTexture, FontAtlas, LegacySpriteTexture, ManagedSurface, MaskAlpha, OUTLINE_PAD,
    SpriteCacheKey, SpriteResidency, SpriteTextureCache, TRANSPARENT_COLOR_KEY_16, make_tex_bg,
    outline_cache_key, rgb565_to_rgba_opaque, shadow_alpha_from_level, sprite_atlas_enabled,
    sprite_outline_rgba, sprite_rgba_for_upload, upload_counter, upload_rgba_texture,
};

pub(super) struct GpuResources {
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
    pub(super) bgl_tex: wgpu::BindGroupLayout,
    _white_view: wgpu::TextureView,
    pub(super) white_bg: wgpu::BindGroup,
    font_atlas_cache: HashMap<u64, FontAtlas>,
}

impl GpuResources {
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
            let skips = robin_assets::late_sprites::note_skipped_draw();
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
        let label = format!("sprite {bank_id:?}/{variant:?}");
        let residency = self.place_sprite(gpu, w, h, &rgba, &label, "sprite bg");
        self.sprite_cache.entries.insert(key, residency);
        Some((w, h))
    }

    /// Put one decoded RGBA frame on the GPU, either packed into the
    /// shared atlas or (A/B flag only) as its own texture.
    ///
    /// `rgba` is uploaded verbatim on both paths — that is what makes
    /// the two residencies pixel-identical.
    fn place_sprite(
        &mut self,
        gpu: &GpuContext,
        width: u16,
        height: u16,
        rgba: &[u8],
        texture_label: &str,
        bind_group_label: &str,
    ) -> SpriteResidency {
        if sprite_atlas_enabled() {
            // Counted where the per-sprite `upload_rgba_texture` used
            // to be, so the `uploads/f` FPS line stays comparable
            // across the migration.
            upload_counter::inc("sprite atlas insert");
            return SpriteResidency::Atlas(self.sprite_atlas.insert(
                gpu,
                &self.bgl_tex,
                &self.sampler,
                width,
                height,
                rgba,
            ));
        }
        let (texture, view) = upload_rgba_texture(
            gpu,
            rgba,
            u32::from(width),
            u32::from(height),
            texture_label,
        );
        let bind_group = make_tex_bg(
            &gpu.device,
            &self.bgl_tex,
            &view,
            &self.sampler,
            bind_group_label,
        );
        SpriteResidency::Legacy(LegacySpriteTexture {
            _texture: texture,
            _view: view,
            bind_group,
            width,
            height,
        })
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
            let skips = robin_assets::late_sprites::note_skipped_draw();
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
        let label = format!("sprite outline {bank_id:?}/{variant:?}");
        let residency =
            self.place_sprite(gpu, outline_w as u16, h, &rgba, &label, "sprite outline bg");
        self.sprite_cache.entries.insert(key, residency);
        Some((outline_w as u16, h))
    }

    /// Upload the static binary alpha for a sprite-occlusion mask. It is
    /// rasterized into stencil for each affected sprite, matching the
    /// original engine's temporary-sprite transparency operation.
    pub(super) fn upload_mask_alpha(
        &mut self,
        gpu: &GpuContext,
        mask_index: u32,
        bitmap: &[u8],
        mask_w: u16,
        mask_h: u16,
    ) -> bool {
        if mask_w == 0 || mask_h == 0 {
            return false;
        }
        let pixels = mask_w as usize * mask_h as usize;
        if bitmap.len() < pixels {
            return false;
        }
        // Spread the binary bitmap (`0` / `1`) into R8 (`0` / `255`) so
        // the sampler returns full 0..1, matching the alpha falloff of
        // the original RGBA compose at building edges.
        let mut r8 = Vec::with_capacity(pixels);
        for &b in &bitmap[..pixels] {
            r8.push(if b != 0 { 0xFFu8 } else { 0x00 });
        }
        upload_counter::inc("mask alpha");
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
            &r8,
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
            },
        );
        true
    }

    pub(super) fn upload_occlusion_depth(
        &mut self,
        gpu: &GpuContext,
        texture_index: u32,
        depth: &[u16],
        width: u16,
        height: u16,
    ) -> bool {
        let width = u32::from(width);
        let height = u32::from(height);
        if width == 0 || height == 0 || depth.len() != width as usize * height as usize {
            return false;
        }
        upload_counter::inc("occlusion depth");
        // Store the high/low bytes separately. R16Unorm requires an optional
        // native wgpu feature, while Rg8Unorm is portable to WebGL/WebGPU too.
        let mut encoded = Vec::with_capacity(depth.len() * 2);
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
            },
        );
        true
    }
}

impl GpuResources {
    pub(super) fn new(
        sampler: wgpu::Sampler,
        bgl_tex: wgpu::BindGroupLayout,
        white_view: wgpu::TextureView,
        white_bg: wgpu::BindGroup,
    ) -> Self {
        Self {
            managed_surfaces: HashMap::new(),
            next_id: 2,
            bit_depth: 16,
            sprite_atlas: SpriteAtlas::default(),
            sprite_cache: SpriteTextureCache::default(),
            mask_alpha_cache: HashMap::new(),
            background_texture: None,
            sampler,
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
        self.next_id += 1;
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
        if let Some(surface) = self.managed_surfaces.get_mut(&id) {
            surface.shadow_alpha = shadow_alpha;
        }
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
        upload_counter::inc("background texture");
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("level background"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
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
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
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
        font_id: u64,
        font: &crate::native_font::NativeFont,
    ) -> wgpu::BindGroup {
        if let Some(atlas) = self.font_atlas_cache.get(&font_id) {
            return atlas.bind_group.clone();
        }
        let (rgba, width, height) = font.build_rgba_atlas();
        let (texture, view) = upload_rgba_texture(gpu, &rgba, width, height, "font atlas");
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
                _texture: texture,
                _view: view,
                bind_group: bind_group.clone(),
            },
        );
        bind_group
    }
}
