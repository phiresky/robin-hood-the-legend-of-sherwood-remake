use super::*;

impl Renderer {
    /// Draw the persistent, linearly sampled fog mask. The CPU only uploads
    /// new texels when deterministic fog or an exact PC reveal circle changes;
    /// panning and zooming reuse the texture with a different UV rectangle.
    pub fn render_fog_mask(
        &mut self,
        width: u32,
        height: u32,
        generation: u64,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        uv: [f32; 4],
        build_rgba: impl FnOnce() -> Vec<u8>,
    ) {
        if w <= 0 || h <= 0 {
            return;
        }
        let recreate = self
            .fog_mask
            .as_ref()
            .is_none_or(|mask| mask.width != width || mask.height != height);
        let update = self
            .fog_mask
            .as_ref()
            .is_some_and(|mask| mask.generation != generation);
        if recreate || update {
            let rgba = build_rgba();
            let expected = width
                .checked_mul(height)
                .and_then(|pixels| pixels.checked_mul(4))
                .unwrap_or_else(|| panic!("fog mask dimensions overflow: {width}x{height}"));
            assert_eq!(
                rgba.len(),
                expected as usize,
                "fog mask pixel count does not match {width}x{height}"
            );
            if !recreate {
                let mask = self
                    .fog_mask
                    .as_mut()
                    .expect("fog mask disappeared while updating it");
                self.gpu.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &mask._texture,
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
                mask.generation = generation;
            } else {
                let (texture, view) = upload_rgba_texture(
                    &self.gpu,
                    &self.resources.uploads,
                    &rgba,
                    width,
                    height,
                    "smooth fog mask",
                );
                let bind_group = make_tex_bg(
                    &self.gpu.device,
                    &self.resources.bgl_tex,
                    &view,
                    &self.resources.linear_sampler,
                    "smooth fog mask bg",
                );
                self.fog_mask = Some(FogMaskTexture {
                    _texture: texture,
                    _view: view,
                    bind_group,
                    width,
                    height,
                    generation,
                });
            }
        }

        let bind_group = self
            .fog_mask
            .as_ref()
            .expect("fog mask was not created")
            .bind_group
            .clone();
        let tex_idx = self.queue_cached_bg(bind_group);
        self.frame.queued.push(QueuedDraw {
            dst: Rect { x, y, w, h },
            corners: None,
            uv,
            tint: [1.0; 4],
            operation: DrawOperation::Quad {
                texture: QuadTexture::Frame(tex_idx),
                blend: BlendMode::Blend,
            },
        });
    }

    /// Decompress sprite frame `(bank_id, variant)` into the GPU cache,
    /// converting RGB565 → RGBA8 with the shadow key baked into alpha.
    /// Returns `Some((width, height))` of the cached frame on success.
    pub fn ensure_sprite_cached(
        &mut self,
        frame_holder: &FrameHolder,
        bank_id: u32,
        variant: SpriteVariant,
        shadow_color: u16,
        shadow_level: u16,
    ) -> Option<(u16, u16)> {
        self.resources.ensure_sprite_cached(
            &self.gpu,
            frame_holder,
            bank_id,
            variant,
            shadow_color,
            shadow_level,
        )
    }

    /// Build the GPU cache for the edge-map outline used by the
    /// selection / mouse-over highlights. The texture is transparent
    /// except for the two outside pixels written at each horizontal
    /// sprite-body edge; the draw-time tint supplies the actual colour.
    pub fn ensure_outline_cached(
        &mut self,
        frame_holder: &FrameHolder,
        bank_id: u32,
        variant: SpriteVariant,
        shadow_color: u16,
        shadow_level: u16,
    ) -> Option<(u16, u16)> {
        self.resources.ensure_outline_cached(
            &self.gpu,
            frame_holder,
            bank_id,
            variant,
            shadow_color,
            shadow_level,
        )
    }

    /// Upload the static binary alpha for a sprite-occlusion mask. It is
    /// rasterized into stencil for each affected sprite, matching the
    /// original engine's temporary-sprite transparency operation.
    /// Upload mission masks in shared R8 pages. Standalone uploads remain
    /// available for oversized masks and the profiling override.
    pub fn upload_mask_alphas<'a>(
        &mut self,
        masks: impl IntoIterator<Item = (u32, &'a [u8], u16, u16)>,
    ) -> Result<(), String> {
        self.resources.upload_mask_alphas(&self.gpu, masks)
    }

    pub fn upload_mask_alpha(
        &mut self,
        mask_index: u32,
        bitmap: &[u8],
        mask_w: u16,
        mask_h: u16,
    ) -> Result<(), String> {
        self.resources
            .upload_mask_alpha(&self.gpu, mask_index, bitmap, mask_w, mask_h)
    }

    /// Upload a map-sized R16 ground-depth field. Each texel stores the
    /// map-ground Y of the visible reconstructed surface at that pixel.
    pub fn upload_occlusion_depth(
        &mut self,
        depth: &[u16],
        width: u16,
        height: u16,
    ) -> Result<(), String> {
        self.resources.upload_occlusion_depth(
            &self.gpu,
            OCCLUSION_DEPTH_TEXTURE_INDEX,
            depth,
            width,
            height,
        )
    }

    /// Return a stable insertion point immediately before the next sprite or
    /// ground-mark draw that may need building occlusion.
    pub(crate) fn draw_queue_checkpoint(&self) -> usize {
        self.frame.queued.len()
    }

    /// Apply the union of `masks` to draws queued since `checkpoint`.
    ///
    /// Each mask is written into stencil, the affected draws are switched to
    /// stencil-tested pipelines, and the clipped sprite rectangle is cleared
    /// afterward. Masked fragments therefore never touch the framebuffer,
    /// exactly like the original temporary-sprite masking path.
    pub(crate) fn mask_queued_draws(
        &mut self,
        checkpoint: usize,
        masks: &[(u32, Rect)],
        clip_rect: Rect,
    ) {
        self.mask_queued_draws_impl(checkpoint, masks, clip_rect, None);
    }

    /// Apply legacy masks plus the continuous reconstructed-geometry depth
    /// field to one grounded sprite. `actor_ground_y` is world Y (not the
    /// projected map Y), so elevated and inclined paths compare correctly.
    pub(crate) fn mask_queued_draws_with_depth(
        &mut self,
        checkpoint: usize,
        masks: &[(u32, Rect)],
        clip_rect: Rect,
        view_x: f32,
        view_y: f32,
        zoom: f32,
        actor_ground_y: f32,
    ) {
        let depth = self
            .resources
            .mask_alpha_cache
            .get(&OCCLUSION_DEPTH_TEXTURE_INDEX)
            .map(|mask| {
                let rect = Rect::new(
                    (-view_x * zoom).round() as i32,
                    (-view_y * zoom).round() as i32,
                    (mask.width as f32 * zoom).round() as u32,
                    (mask.height as f32 * zoom).round() as u32,
                );
                // A small bias prevents the character's own ground surface
                // from flickering into stencil through quantization/filtering.
                let threshold = (actor_ground_y / mask.height as f32 + 2.0 / mask.height as f32)
                    .clamp(0.0, 1.0);
                (rect, threshold)
            });
        self.mask_queued_draws_impl(checkpoint, masks, clip_rect, depth);
    }

    pub(super) fn mask_queued_draws_impl(
        &mut self,
        checkpoint: usize,
        masks: &[(u32, Rect)],
        clip_rect: Rect,
        depth: Option<(Rect, f32)>,
    ) {
        assert!(
            checkpoint <= self.frame.queued.len(),
            "sprite mask checkpoint {checkpoint} exceeds draw queue length {}",
            self.frame.queued.len()
        );
        if masks.is_empty() && depth.is_none() {
            return;
        }
        assert!(
            checkpoint < self.frame.queued.len(),
            "sprite masks require at least one queued draw"
        );

        let mut stencil_draws = Vec::with_capacity(masks.len() + usize::from(depth.is_some()));
        for &(mask_index, mask_rect) in masks {
            assert!(
                self.resources.mask_alpha_cache.contains_key(&mask_index),
                "sprite mask {mask_index} was not uploaded"
            );
            let Some((dst, uv)) = clip_dst_to_uv(mask_rect, clip_rect) else {
                continue;
            };
            stencil_draws.push(QueuedDraw {
                dst,
                corners: None,
                uv,
                tint: self.resources.mask_alpha_cache[&mask_index]
                    .atlas
                    .map_or([0.5, 0.0, 1.0, 1.0], MaskAtlasBounds::tint),
                operation: DrawOperation::MaskAlpha(mask_index),
            });
        }
        if let Some((depth_rect, threshold)) = depth
            && let Some((dst, uv)) = clip_dst_to_uv(depth_rect, clip_rect)
        {
            stencil_draws.push(QueuedDraw {
                dst,
                corners: None,
                uv,
                // Green selects 16-bit high/low reconstruction in the
                // shared mask-stencil shader.
                tint: [threshold, 1.0, 1.0, 1.0],
                operation: DrawOperation::MaskAlpha(OCCLUSION_DEPTH_TEXTURE_INDEX),
            });
        }
        if stencil_draws.is_empty() {
            return;
        }

        mark_draws_stencil_tested(&mut self.frame.queued[checkpoint..]);
        self.frame
            .queued
            .splice(checkpoint..checkpoint, stencil_draws);
        self.frame.queued.push(QueuedDraw {
            dst: clip_rect,
            corners: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            tint: [0.5, 0.0, 1.0, 1.0],
            operation: DrawOperation::StencilClear,
        });
    }

    /// Queue a cached sprite as an alpha-blended GPU overlay quad.
    /// `pub(crate)` to match the original visibility so game_render
    /// can still reach it.
    pub(crate) fn render_cached_sprite(
        &mut self,
        bank_id: u32,
        variant: SpriteVariant,
        shadow_color: u16,
        shadow_level: u16,
        dst_rect: Rect,
    ) -> bool {
        let key = SpriteCacheKey {
            bank_id,
            variant,
            shadow_color: shadow_color as u32,
            shadow_alpha: shadow_alpha_from_level(shadow_level),
        };
        let Some((tex_idx, uv)) = self.queue_sprite_texture(&key) else {
            return false;
        };
        self.frame.queued.push(QueuedDraw {
            dst: dst_rect,
            corners: None,
            uv,
            tint: [1.0, 1.0, 1.0, 1.0],
            operation: DrawOperation::Quad {
                texture: QuadTexture::Frame(tex_idx),
                blend: BlendMode::Blend,
            },
        });
        true
    }

    /// Like [`render_cached_sprite`] but applies a per-frame alpha to
    /// the whole quad (used by the fade-out / damage-flash paths).
    pub(crate) fn render_cached_sprite_alpha(
        &mut self,
        bank_id: u32,
        variant: SpriteVariant,
        shadow_color: u16,
        shadow_level: u16,
        dst_rect: Rect,
        alpha: u8,
    ) -> bool {
        let key = SpriteCacheKey {
            bank_id,
            variant,
            shadow_color: shadow_color as u32,
            shadow_alpha: shadow_alpha_from_level(shadow_level),
        };
        let Some((tex_idx, uv)) = self.queue_sprite_texture(&key) else {
            return false;
        };
        self.frame.queued.push(QueuedDraw {
            dst: dst_rect,
            corners: None,
            uv,
            tint: [1.0, 1.0, 1.0, alpha as f32 / 255.0],
            operation: DrawOperation::Quad {
                texture: QuadTexture::Frame(tex_idx),
                blend: BlendMode::Blend,
            },
        });
        true
    }

    /// Queue the cached edge-map outline tinted by `rgb * alpha`.
    pub(crate) fn render_cached_outline(
        &mut self,
        bank_id: u32,
        variant: SpriteVariant,
        shadow_color: u16,
        _shadow_level: u16,
        dst_rect: Rect,
        rgb: (u8, u8, u8),
        alpha: u8,
    ) -> bool {
        let key = outline_cache_key(bank_id, variant, shadow_color);
        let Some((tex_idx, uv)) = self.queue_sprite_texture(&key) else {
            return false;
        };
        self.frame.queued.push(QueuedDraw {
            dst: dst_rect,
            corners: None,
            uv,
            tint: [
                rgb.0 as f32 / 255.0,
                rgb.1 as f32 / 255.0,
                rgb.2 as f32 / 255.0,
                alpha as f32 / 255.0,
            ],
            operation: DrawOperation::Quad {
                texture: QuadTexture::Frame(tex_idx),
                blend: BlendMode::Blend,
            },
        });
        true
    }

    pub(crate) fn render_hidden_mask_outline(
        &mut self,
        frame_holder: &FrameHolder,
        bank_id: u32,
        variant: SpriteVariant,
        shadow_color: u16,
        mask_bitmap: &[u8],
        mask_w: u16,
        mask_h: u16,
        mask_rect: Rect,
        sprite_rect: Rect,
        rgb: (u8, u8, u8),
    ) -> bool {
        if mask_w == 0 || mask_h == 0 || mask_rect.w <= 0 || mask_rect.h <= 0 {
            return true;
        }
        if sprite_rect.w <= 0 || sprite_rect.h <= 0 {
            return true;
        }
        let expected_mask = mask_w as usize * mask_h as usize;
        if mask_bitmap.len() < expected_mask {
            return false;
        }

        let viewport = Rect::new(0, 0, self.frame.width.into(), self.frame.height.into());
        let Some(overlap) = mask_rect
            .intersection(sprite_rect)
            .and_then(|overlap| overlap.intersection(viewport))
        else {
            return true;
        };
        let left = overlap.left();
        let top = overlap.top();
        let right = overlap.right();
        let bottom = overlap.bottom();

        let sw = frame_holder.sprite_width(bank_id) as usize;
        let sh = frame_holder.sprite_height(bank_id) as usize;
        if sw == 0 || sh == 0 {
            return false;
        }

        let mut sprite = vec![TRANSPARENT_COLOR_KEY_16; sw * sh];
        frame_holder.uncompress_frame(
            &mut sprite,
            sw,
            bank_id,
            variant,
            shadow_color,
            self.resources.bit_depth,
        );

        let out_w = (right - left) as usize;
        let out_h = (bottom - top) as usize;
        let mut rgba = vec![0u8; out_w * out_h * 4];
        let mut any = false;
        let mask_scale_x = mask_w as f32 / mask_rect.w as f32;
        let mask_scale_y = mask_h as f32 / mask_rect.h as f32;
        let sprite_scale_x = sw as f32 / sprite_rect.w as f32;
        let sprite_scale_y = sh as f32 / sprite_rect.h as f32;

        for y in top..bottom {
            let mask_y = ((y - mask_rect.y) as f32 * mask_scale_y).floor() as usize;
            let sy = ((y - sprite_rect.y) as f32 * sprite_scale_y).floor() as usize;
            if mask_y >= mask_h as usize || sy >= sh {
                continue;
            }
            for x in left..right {
                let mask_x = ((x - mask_rect.x) as f32 * mask_scale_x).floor() as usize;
                if mask_x >= mask_w as usize {
                    continue;
                }
                if mask_bitmap[mask_y * mask_w as usize + mask_x] == 0 {
                    continue;
                }

                let sx = ((x - sprite_rect.x) as f32 * sprite_scale_x).floor() as usize;
                if sx + 1 >= sw {
                    continue;
                }
                let row = sy * sw;
                let mut p1 = sprite[row + sx];
                let mut p2 = sprite[row + sx + 1];
                if p1 == shadow_color {
                    p1 = TRANSPARENT_COLOR_KEY_16;
                }
                if p2 == shadow_color {
                    p2 = TRANSPARENT_COLOR_KEY_16;
                }
                if p1 != p2 && (p1 == TRANSPARENT_COLOR_KEY_16 || p2 == TRANSPARENT_COLOR_KEY_16) {
                    let out_idx = (((y - top) as usize * out_w) + (x - left) as usize) * 4;
                    rgba[out_idx] = rgb.0;
                    rgba[out_idx + 1] = rgb.1;
                    rgba[out_idx + 2] = rgb.2;
                    rgba[out_idx + 3] = 255;
                    any = true;
                }
            }
        }

        if !any {
            return true;
        }

        let (_texture, view) = upload_rgba_texture(
            &self.gpu,
            &self.resources.uploads,
            &rgba,
            out_w as u32,
            out_h as u32,
            "hidden mask outline",
        );
        let tex_idx = self.queue_frame_texture(&view);
        self.frame.queued.push(QueuedDraw {
            dst: Rect::new(left, top, out_w as u32, out_h as u32),
            corners: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            tint: [1.0, 1.0, 1.0, 1.0],
            operation: DrawOperation::Quad {
                texture: QuadTexture::Frame(tex_idx),
                blend: BlendMode::Blend,
            },
        });
        true
    }

    pub fn clear_mask_alpha_cache(&mut self) {
        self.resources.clear_mask_alpha_cache();
    }
}
