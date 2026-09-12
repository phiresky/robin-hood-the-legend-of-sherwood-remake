use super::*;

impl Renderer {
    /// Upload decoded pixels and return their unique retirement authority.
    pub fn upload_rgb565(
        &mut self,
        width: u16,
        height: u16,
        pixels: &[u16],
    ) -> Option<OwnedSurface> {
        let id = self.create_surface_from_rgb565(width, height, pixels)?;
        Some(self.adopt_surface(id))
    }

    /// Upload tightly packed little-endian RGB565 bytes without discarding a
    /// partial pixel. Invalid layouts are rejected before allocating pixels.
    pub(crate) fn upload_rgb565_bytes(
        &mut self,
        width: u16,
        height: u16,
        bytes: &[u8],
    ) -> Option<OwnedSurface> {
        let Some(pixels) = decode_rgb565_pixels(width, height, bytes) else {
            tracing::warn!(
                "invalid RGB565 byte layout: {width}x{height}, {} bytes",
                bytes.len()
            );
            return None;
        };
        self.upload_rgb565(width, height, &pixels)
    }

    pub(crate) fn upload_deferred_rgb565(
        &mut self,
        width: u16,
        height: u16,
        pixels: Box<[u16]>,
    ) -> OwnedSurface {
        let id = self.create_deferred_surface_from_rgb565(width, height, pixels);
        self.adopt_surface(id)
    }

    pub(super) fn create_surface_from_rgb565(
        &mut self,
        width: u16,
        height: u16,
        pixels: &[u16],
    ) -> Option<u32> {
        let expected = width as usize * height as usize;
        if expected == 0 || pixels.len() != expected {
            tracing::warn!(
                "create_surface_from_rgb565: invalid dimensions/data: {}x{}, {} pixels",
                width,
                height,
                pixels.len()
            );
            return None;
        }
        let surface = self.build_managed_surface(width, height, pixels, DEFAULT_SHADOW_ALPHA)?;
        Some(self.resources.insert_managed_surface(surface))
    }

    pub(super) fn build_managed_surface(
        &self,
        width: u16,
        height: u16,
        pixels: &[u16],
        shadow_alpha: u8,
    ) -> Option<ManagedSurface> {
        let w = width as usize;
        let h = height as usize;
        if w == 0 || h == 0 || pixels.len() != w * h {
            return None;
        }

        Some(ManagedSurface {
            width,
            height,
            pixels: ManagedSurfacePixels::Resident(
                self.upload_managed_surface(width, height, pixels),
            ),
            alpha_mask: AlphaMask::from_pixels(
                width,
                height,
                width as u32,
                pixels,
                TRANSPARENT_COLOR_KEY_16,
            ),
            shadow_alpha,
        })
    }

    /// Validate and register a surface immediately, retaining its RGB565 pixels
    /// until its first draw. Hit testing and dimensions are available before
    /// GPU residency, so unopened menus need no texture conversion or upload.
    pub(super) fn create_deferred_surface_from_rgb565(
        &mut self,
        width: u16,
        height: u16,
        pixels: Box<[u16]>,
    ) -> u32 {
        assert!(
            width > 0 && height > 0,
            "deferred surface requires nonzero dimensions"
        );
        assert_eq!(
            pixels.len(),
            width as usize * height as usize,
            "deferred surface RGB565 payload must match dimensions"
        );
        let max_dimension = self.gpu.device.limits().max_texture_dimension_2d;
        assert!(
            u32::from(width) <= max_dimension && u32::from(height) <= max_dimension,
            "deferred surface dimensions exceed GPU texture limit {max_dimension}"
        );
        let alpha_mask = AlphaMask::from_pixels(
            width,
            height,
            width as u32,
            &pixels,
            TRANSPARENT_COLOR_KEY_16,
        );
        self.resources.insert_managed_surface(ManagedSurface {
            width,
            height,
            pixels: ManagedSurfacePixels::Pending(pixels),
            alpha_mask,
            shadow_alpha: DEFAULT_SHADOW_ALPHA,
        })
    }

    pub(super) fn ensure_managed_surface_resident(&mut self, id: u32) {
        let Some(surface) = self.resources.managed_surfaces.get(&id) else {
            return;
        };
        let ManagedSurfacePixels::Pending(pixels) = &surface.pixels else {
            return;
        };
        let textures = self.upload_managed_surface(surface.width, surface.height, pixels);
        self.resources
            .managed_surfaces
            .get_mut(&id)
            .expect("surface remains registered during upload")
            .pixels = ManagedSurfacePixels::Resident(textures);
    }

    pub(super) fn upload_managed_surface(
        &self,
        width: u16,
        height: u16,
        pixels: &[u16],
    ) -> ManagedSurfaceTextures {
        let opaque_rgba = rgb565_to_rgba_opaque(pixels, width as usize, height as usize);
        let (color_rgba, shadow_rgba) =
            rgb565_to_color_shadow_rgba(pixels, TRANSPARENT_COLOR_KEY_16);
        let (opaque_texture, opaque_view) = upload_rgba_texture(
            &self.gpu,
            &self.resources.uploads,
            &opaque_rgba,
            width as u32,
            height as u32,
            "managed surface opaque",
        );
        let opaque_bg = make_tex_bg(
            &self.gpu.device,
            &self.resources.bgl_tex,
            &opaque_view,
            &self.resources.sampler,
            "managed surface opaque bg",
        );
        let (color_texture, color_view) = upload_rgba_texture(
            &self.gpu,
            &self.resources.uploads,
            &color_rgba,
            width as u32,
            height as u32,
            "managed surface color",
        );
        let color_bg = make_tex_bg(
            &self.gpu.device,
            &self.resources.bgl_tex,
            &color_view,
            &self.resources.sampler,
            "managed surface color bg",
        );
        let (shadow_texture, shadow_view, shadow_bg) = if let Some(shadow_rgba) = shadow_rgba {
            let (texture, view) = upload_rgba_texture(
                &self.gpu,
                &self.resources.uploads,
                &shadow_rgba,
                width as u32,
                height as u32,
                "managed surface shadow",
            );
            let bg = make_tex_bg(
                &self.gpu.device,
                &self.resources.bgl_tex,
                &view,
                &self.resources.sampler,
                "managed surface shadow bg",
            );
            (Some(texture), Some(view), Some(bg))
        } else {
            (None, None, None)
        };

        ManagedSurfaceTextures {
            _opaque_texture: opaque_texture,
            _opaque_view: opaque_view,
            opaque_bg,
            _color_texture: color_texture,
            _color_view: color_view,
            color_bg,
            _shadow_texture: shadow_texture,
            _shadow_view: shadow_view,
            shadow_bg,
        }
    }

    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(super) fn delete_surface(&mut self, id: u32) -> bool {
        self.try_delete_legacy_surface(id)
            .expect("owned upload must be retired with its ownership token")
    }

    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn try_delete_legacy_surface(
        &mut self,
        id: u32,
    ) -> Result<bool, SurfaceOwnershipError> {
        if self.owned_surfaces.contains(&id) {
            return Err(SurfaceOwnershipError::AlreadyOwned(id));
        }
        Ok(self.resources.delete_managed_surface(id))
    }

    /// Compatibility boundary: resolve legacy screen aliases or mint a local upload reference.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(super) fn legacy_surface_target(&self, id: u32) -> Result<SurfaceTarget, MissingSurface> {
        if id <= 1 {
            Ok(SurfaceTarget::Screen)
        } else {
            self.surface_handle(id).map(SurfaceTarget::Upload)
        }
    }

    /// Validate an existing compatibility upload ID. Screen IDs are rejected.
    pub(super) fn surface_handle(&self, id: u32) -> Result<SurfaceHandle, MissingSurface> {
        let handle = SurfaceHandle {
            id,
            renderer: self.identity,
        };
        self.surface_dimensions(handle)?;
        Ok(handle)
    }

    pub(super) fn adopt_surface(&mut self, id: u32) -> OwnedSurface {
        self.try_adopt_surface(id)
            .expect("ownership requires a live unowned upload")
    }

    pub(super) fn try_adopt_surface(
        &mut self,
        id: u32,
    ) -> Result<OwnedSurface, SurfaceOwnershipError> {
        self.validate_surface_adoption(id)?;
        let handle = self.surface_handle(id)?;
        self.owned_surfaces.insert(id);
        Ok(OwnedSurface { handle })
    }

    pub(super) fn validate_surface_adoption(&self, id: u32) -> Result<(), SurfaceOwnershipError> {
        self.surface_handle(id)?;
        if self.owned_surfaces.contains(&id) {
            return Err(SurfaceOwnershipError::AlreadyOwned(id));
        }
        Ok(())
    }

    pub fn retire_surface(&mut self, surface: OwnedSurface) {
        self.try_retire_surface(surface)
            .expect("retirement requires the originating renderer and a live owned upload");
    }

    pub(crate) fn identity(&self) -> u64 {
        self.identity
    }

    /// Preflight a whole owner bank before retiring any member.
    pub(crate) fn validate_surface_retirement(
        &self,
        surface: &OwnedSurface,
    ) -> Result<(), SurfaceOwnershipError> {
        self.surface_dimensions(surface.handle)?;
        if !self.owned_surfaces.contains(&surface.handle.id) {
            return Err(SurfaceOwnershipError::NotOwned(surface.handle.id));
        }
        Ok(())
    }

    /// On rejection the caller retains its token and can retire it with the correct renderer.
    pub fn try_retire_surface(
        &mut self,
        surface: OwnedSurface,
    ) -> Result<(), (SurfaceOwnershipError, OwnedSurface)> {
        if let Err(error) = self.surface_dimensions(surface.handle) {
            return Err((error.into(), surface));
        }
        if !self.owned_surfaces.remove(&surface.handle.id) {
            return Err((SurfaceOwnershipError::NotOwned(surface.handle.id), surface));
        }
        assert!(
            self.resources.delete_managed_surface(surface.handle.id),
            "owned surface disappeared during retirement"
        );
        Ok(())
    }

    pub fn target_dimensions(&self, target: SurfaceTarget) -> Result<(u16, u16), MissingSurface> {
        match target {
            SurfaceTarget::Screen => Ok(self.frame.dimensions()),
            SurfaceTarget::Upload(handle) => self.surface_dimensions(handle),
        }
    }

    pub fn surface_dimensions(&self, handle: SurfaceHandle) -> Result<(u16, u16), MissingSurface> {
        if handle.renderer != self.identity || handle.id <= 1 {
            return Err(MissingSurface(handle));
        }
        self.resources
            .surface_dimensions(handle.id)
            .ok_or(MissingSurface(handle))
    }

    /// Build an `AlphaMask` from a managed surface — one bit per pixel,
    /// flagging non-transparent (`pixel != color_key`) pixels. Used by
    /// the UI hit-test path (`RendererBase::is_real_point`) so widget
    /// clicks on visually-transparent corners of round/non-rectangular
    /// sprites get rejected via a viewport pixel sample.
    pub fn build_alpha_mask(&self, id: u32) -> Option<AlphaMask> {
        self.resources.alpha_mask(id)
    }

    /// A foreign or retired handle is an error, not an absent optional mask.
    pub fn surface_alpha_mask(&self, handle: SurfaceHandle) -> Result<AlphaMask, MissingSurface> {
        self.surface_dimensions(handle)?;
        self.resources
            .alpha_mask(handle.id)
            .ok_or(MissingSurface(handle))
    }

    /// Override the shadow alpha baked into `SHADOW_KEY` pixels at
    /// upload time. Set to `MENU_BUTTON_SHADOW_ALPHA` (50%) for
    /// menu-button packs, leave at the default `DEFAULT_SHADOW_ALPHA`
    /// (40%) for everything else.
    pub fn set_surface_shadow_alpha(
        &mut self,
        handle: SurfaceHandle,
        shadow_alpha: u8,
    ) -> Result<(), MissingSurface> {
        self.surface_dimensions(handle)?;
        self.set_shadow_alpha(handle.id, shadow_alpha);
        Ok(())
    }

    pub(super) fn set_shadow_alpha(&mut self, id: u32, shadow_alpha: u8) {
        self.resources.set_shadow_alpha(id, shadow_alpha);
    }

    pub fn create_loading_dissolve_textures(
        &self,
        width: u32,
        height: u32,
        initial_pixels: impl ExactSizeIterator<Item = u16>,
        final_pixels: impl ExactSizeIterator<Item = u16>,
        height_field: &crate::loading_screen::HeightField,
    ) -> Option<crate::loading_dissolve_gpu::LoadingDissolveTextures> {
        crate::loading_dissolve_gpu::upload_textures(
            self,
            width,
            height,
            initial_pixels,
            final_pixels,
            height_field,
        )
    }

    /// Queue the sand-dissolve composite of the loading artwork into `dst`
    /// (logical canvas pixels). The caller picks the rectangle so the
    /// artwork can be aspect-fitted and centred on a canvas whose shape
    /// differs from the pictures' native 4:3.
    pub fn render_loading_dissolve(
        &mut self,
        textures: &crate::loading_dissolve_gpu::LoadingDissolveTextures,
        threshold: u32,
        dst: Rect,
    ) {
        if textures.width == 0 || textures.height == 0 || dst.w <= 0 || dst.h <= 0 {
            return;
        }
        let bind_group = crate::loading_dissolve_gpu::create_frame_bind_group(
            &self.gpu.device,
            &self.pipelines.bgl_loading_dissolve,
            textures,
            &self.resources.sampler,
        );
        let frame_idx = self.frame.frame_texture_bgs.len() as u32;
        self.frame.frame_texture_bgs.push(bind_group);
        self.frame.queued.push(QueuedDraw {
            dst,
            corners: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            // The shader compares `height > threshold`; threshold can be 256
            // at progress 0, so keep the normalized value slightly above 1.0.
            tint: [threshold as f32 / 255.0, 0.0, 0.0, 1.0],
            operation: DrawOperation::LoadingDissolve(frame_idx),
        });
    }

    pub fn create_rgb565_gpu_image(
        &self,
        width: u16,
        height: u16,
        pixels: &[u16],
        transparent: bool,
        label: &str,
    ) -> Option<GpuImage> {
        let expected = width as usize * height as usize;
        if expected == 0 || pixels.len() != expected {
            tracing::warn!(
                "create_rgb565_gpu_image: invalid dimensions/data for {label}: {}x{}, {} pixels",
                width,
                height,
                pixels.len()
            );
            return None;
        }
        let rgba = if transparent {
            rgb565_to_rgba_with_key(
                pixels,
                width as usize,
                height as usize,
                self.transparent_color(),
                0,
                None,
            )
        } else {
            rgb565_to_rgba_opaque(pixels, width as usize, height as usize)
        };
        let (texture, view) = upload_rgba_texture(
            &self.gpu,
            &self.resources.uploads,
            &rgba,
            width as u32,
            height as u32,
            label,
        );
        let bind_group = make_tex_bg(
            &self.gpu.device,
            &self.resources.bgl_tex,
            &view,
            &self.resources.sampler,
            "gpu image bg",
        );
        Some(GpuImage {
            _texture: texture,
            _view: view,
            bind_group,
            width,
            height,
        })
    }

    pub fn create_rgba_gpu_image(
        &self,
        width: u16,
        height: u16,
        rgba: &[u8],
        label: &str,
    ) -> Option<GpuImage> {
        let expected = width as usize * height as usize * 4;
        if expected == 0 || rgba.len() != expected {
            tracing::warn!(
                "create_rgba_gpu_image: invalid dimensions/data for {label}: {}x{}, {} bytes",
                width,
                height,
                rgba.len()
            );
            return None;
        }
        let (texture, view) = upload_rgba_texture(
            &self.gpu,
            &self.resources.uploads,
            rgba,
            width as u32,
            height as u32,
            label,
        );
        let bind_group = make_tex_bg(
            &self.gpu.device,
            &self.resources.bgl_tex,
            &view,
            &self.resources.sampler,
            "gpu image bg",
        );
        Some(GpuImage {
            _texture: texture,
            _view: view,
            bind_group,
            width,
            height,
        })
    }

    pub fn render_gpu_image(
        &mut self,
        image: &GpuImage,
        src_rect: Option<&BBox>,
        dst_rect: Option<&BBox>,
        blend: BlendMode,
    ) {
        self.render_gpu_image_tinted(image, src_rect, dst_rect, blend, [1.0, 1.0, 1.0, 1.0]);
    }

    pub fn render_gpu_image_tinted(
        &mut self,
        image: &GpuImage,
        src_rect: Option<&BBox>,
        dst_rect: Option<&BBox>,
        blend: BlendMode,
        tint: [f32; 4],
    ) {
        if image.width == 0 || image.height == 0 {
            return;
        }
        let (dst, uv) = src_dst_uv(src_rect, dst_rect, image.width as f32, image.height as f32);
        let tex_idx = self.queue_cached_bg(image.bind_group.clone());
        self.frame.queued.push(QueuedDraw {
            dst,
            corners: None,
            uv,
            tint,
            operation: DrawOperation::Quad {
                texture: QuadTexture::Frame(tex_idx),
                blend,
            },
        });
    }
}
