use super::*;

impl Renderer {
    /// Outline a rect on the GPU overlay layer. Color is RGB565 to match the
    /// rest of the legacy rendering API.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_rect_outline_screen(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, color: u16) {
        let (r, g, b) = rgb565_to_rgb8(color);
        self.render_gpu_line(x1, y1, x2, y1, r, g, b);
        self.render_gpu_line(x2, y1, x2, y2, r, g, b);
        self.render_gpu_line(x2, y2, x1, y2, r, g, b);
        self.render_gpu_line(x1, y2, x1, y1, r, g, b);
    }

    /// Draw a line on the GPU overlay layer. RGB565 color in.
    pub fn draw_line_screen(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, color: u16) {
        let (r, g, b) = rgb565_to_rgb8(color);
        self.render_gpu_line(x1, y1, x2, y2, r, g, b);
    }

    /// Fill a rect on the GPU overlay layer. `rect=None` fills the
    /// whole logical screen. RGB565 color in.
    pub fn fill_screen(&mut self, rect: Option<&BBox>, color: u16) -> bool {
        let (r, g, b) = rgb565_to_rgb8(color);
        let (x, y, w, h) = match rect {
            Some(r) => (
                r.min.x as i32,
                r.min.y as i32,
                (r.max.x - r.min.x) as i32,
                (r.max.y - r.min.y) as i32,
            ),
            None => (0, 0, self.frame.width as i32, self.frame.height as i32),
        };
        self.render_gpu_rect(x, y, w, h, r, g, b, 255);
        true
    }

    /// Start a GPU-only frame with the render target's normal black clear and
    /// no legacy framebuffer upload. Used by menus/loading states whose
    /// background is fully drawn by queued GPU quads.
    pub fn begin_gpu_frame_clear(&mut self) {
        self.frame.enter_gpu_phase();
    }

    /// Queue a borrowed upload only after checking its renderer and lifetime.
    pub fn draw_surface(
        &mut self,
        handle: SurfaceHandle,
        src_rect: Option<&BBox>,
        dst_rect: Option<&BBox>,
        flags: u32,
    ) -> Result<(), MissingSurface> {
        self.queue_managed_surface(handle, src_rect, dst_rect, flags, 1.0, None)
    }

    /// Rotate a borrowed UI surface counterclockwise inside its destination.
    pub(crate) fn draw_surface_rotated_ccw(
        &mut self,
        handle: SurfaceHandle,
        dst: &BBox,
    ) -> Result<(), MissingSurface> {
        let first = self.frame.queued.len();
        self.draw_surface(handle, None, Some(dst), BLIT_SOURCE_TRANSPARENT)?;
        for draw in &mut self.frame.queued[first..] {
            draw.corners = Some([
                (dst.min.x, dst.max.y),
                (dst.min.x, dst.min.y),
                (dst.max.x, dst.max.y),
                (dst.max.x, dst.min.y),
            ]);
        }
        Ok(())
    }

    /// Alpha draw with the same renderer/lifetime checks as ordinary typed draws.
    pub fn draw_surface_alpha(
        &mut self,
        handle: SurfaceHandle,
        src_rect: Option<&BBox>,
        dst_rect: Option<&BBox>,
        alpha_level: u16,
        flags: u32,
    ) -> Result<(), MissingSurface> {
        // alpha_level: 0 = fully opaque, 100 = fully transparent.
        let opacity = 100u16.saturating_sub(alpha_level) as f32 / 100.0;
        self.queue_managed_surface(handle, src_rect, dst_rect, flags, opacity, None)
    }

    /// Shadow draw for a borrowed upload, always targeting this renderer's screen.
    /// Shadow-key pixels darken the destination; their original color is ignored.
    pub fn draw_surface_with_shadow(
        &mut self,
        handle: SurfaceHandle,
        src_rect: Option<&BBox>,
        dst_rect: Option<&BBox>,
        shadow_level: u16,
        flags: u32,
    ) -> Result<(), MissingSurface> {
        let shadow_alpha = (u32::from(shadow_level.min(100)) * 255 / 100) as u8;
        self.queue_managed_surface(handle, src_rect, dst_rect, flags, 1.0, Some(shadow_alpha))
    }

    /// Validate the borrowed upload before resolving geometry or realizing its texture.
    /// Empty/inverted rectangles are no-ops, including destinations truncated below
    /// one pixel. Source UVs retain fractional coordinates for every draw variant.
    pub(super) fn queue_managed_surface(
        &mut self,
        handle: SurfaceHandle,
        src_rect: Option<&BBox>,
        dst_rect: Option<&BBox>,
        flags: u32,
        opacity: f32,
        shadow_alpha: Option<u8>,
    ) -> Result<(), MissingSurface> {
        let (width, height) = self.surface_dimensions(handle)?;
        let (dst, uv) = src_dst_uv(src_rect, dst_rect, width as f32, height as f32);
        if dst.w <= 0 || dst.h <= 0 || uv[0] >= uv[2] || uv[1] >= uv[3] {
            return Ok(());
        }
        if shadow_alpha.is_some() {
            self.frame.enter_gpu_phase();
        }
        self.ensure_managed_surface_resident(handle.id);
        let surface = self
            .resources
            .managed_surfaces
            .get(&handle.id)
            .expect("validated surface remains registered during upload");
        if flags & BLIT_SOURCE_TRANSPARENT != 0 {
            self.queue_transparent_managed_bgs(
                surface.textures().color_bg.clone(),
                surface.textures().shadow_bg.clone(),
                shadow_alpha.unwrap_or(surface.shadow_alpha),
                dst,
                uv,
                opacity,
            );
        } else {
            let tex_idx = self.queue_cached_bg(surface.textures().opaque_bg.clone());
            self.frame.queued.push(QueuedDraw {
                dst,
                corners: None,
                uv,
                tint: [1.0, 1.0, 1.0, opacity],
                operation: DrawOperation::Quad {
                    texture: QuadTexture::Frame(tex_idx),
                    blend: BlendMode::Blend,
                },
            });
        }
        Ok(())
    }

    pub(super) fn queue_transparent_managed_bgs(
        &mut self,
        color_bg: wgpu::BindGroup,
        shadow_bg: Option<wgpu::BindGroup>,
        shadow_alpha: u8,
        dst: Rect,
        uv: [f32; 4],
        opacity: f32,
    ) {
        if let Some(shadow_bg) = shadow_bg {
            let tex_idx = self.queue_cached_bg(shadow_bg);
            self.frame.queued.push(QueuedDraw {
                dst,
                corners: None,
                uv,
                tint: [
                    1.0,
                    1.0,
                    1.0,
                    (shadow_alpha as f32 / 255.0 * opacity).clamp(0.0, 1.0),
                ],
                operation: DrawOperation::Quad {
                    texture: QuadTexture::Frame(tex_idx),
                    blend: BlendMode::Blend,
                },
            });
        }
        let tex_idx = self.queue_cached_bg(color_bg);
        self.frame.queued.push(QueuedDraw {
            dst,
            corners: None,
            uv,
            tint: [1.0, 1.0, 1.0, opacity.clamp(0.0, 1.0)],
            operation: DrawOperation::Quad {
                texture: QuadTexture::Frame(tex_idx),
                blend: BlendMode::Blend,
            },
        });
    }

    /// Draw a 1-pixel-thick line from (x1,y1) to (x2,y2). Implemented
    /// as a rotated thin quad — 4 corners offset by ±0.5 perpendicular
    /// to the line direction — so diagonal lines render as a real line
    /// rather than the bounding-box outline placeholder. Used by the
    /// view-cone outlines and debug overlays.
    pub fn render_gpu_line(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, [r, g, b]: [u8; 3]) {
        let tint = Color::rgb(r, g, b).to_f32_srgb();
        // Axis-aligned single-pixel strips stay on the rect path to avoid
        // half-pixel rounding from the perpendicular offset.
        if y1 == y2 {
            let lx = x1.min(x2);
            let rx = x1.max(x2);
            self.frame.queued.push(QueuedDraw {
                dst: Rect {
                    x: lx,
                    y: y1,
                    w: (rx - lx).max(1),
                    h: 1,
                },
                corners: None,
                uv: [0.0, 0.0, 1.0, 1.0],
                tint,
                operation: DrawOperation::Quad {
                    texture: QuadTexture::White,
                    blend: BlendMode::None,
                },
            });
            return;
        }
        if x1 == x2 {
            let ty = y1.min(y2);
            let by = y1.max(y2);
            self.frame.queued.push(QueuedDraw {
                dst: Rect {
                    x: x1,
                    y: ty,
                    w: 1,
                    h: (by - ty).max(1),
                },
                corners: None,
                uv: [0.0, 0.0, 1.0, 1.0],
                tint,
                operation: DrawOperation::Quad {
                    texture: QuadTexture::White,
                    blend: BlendMode::None,
                },
            });
            return;
        }
        // Diagonal — build a thin rotated quad covering the line.
        // Endpoints sit on the half-pixel centre; the perpendicular
        // offset of ±0.5 gives a 1-pixel-thick strip oriented along
        // the line direction.
        let p1 = (x1 as f32 + 0.5, y1 as f32 + 0.5);
        let p2 = (x2 as f32 + 0.5, y2 as f32 + 0.5);
        let dx = p2.0 - p1.0;
        let dy = p2.1 - p1.1;
        let len = (dx * dx + dy * dy).sqrt().max(1e-6);
        let nx = -dy / len * 0.5;
        let ny = dx / len * 0.5;
        let corners = [
            (p1.0 + nx, p1.1 + ny), // TL
            (p2.0 + nx, p2.1 + ny), // TR
            (p1.0 - nx, p1.1 - ny), // BL
            (p2.0 - nx, p2.1 - ny), // BR
        ];
        self.frame.queued.push(QueuedDraw {
            dst: Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            corners: Some(corners),
            uv: [0.0, 0.0, 1.0, 1.0],
            tint,
            operation: DrawOperation::Quad {
                texture: QuadTexture::White,
                blend: BlendMode::None,
            },
        });
    }

    pub fn render_gpu_rect(&mut self, x: i32, y: i32, w: i32, h: i32, [r, g, b, a]: [u8; 4]) {
        self.frame.queued.push(QueuedDraw {
            dst: Rect { x, y, w, h },
            corners: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            tint: Color::rgba(r, g, b, a).to_f32_srgb(),
            operation: DrawOperation::Quad {
                texture: QuadTexture::White,
                blend: BlendMode::Blend,
            },
        });
    }

    /// Filled triangle on the GPU overlay. Submitted as a degenerate
    /// quad: corner layout is `[A, B, C, C]` so the 6-vertex
    /// expansion in `upload_queue_geometry` (`[TL, TR, BL, BL, TR,
    /// BR]`) emits one real triangle `(A, B, C)` followed by a
    /// zero-area triangle `(C, B, C)`. Used by the debug shadow /
    /// view-cone overlays.
    pub fn render_gpu_triangle(&mut self, pts: [(f32, f32); 3], [r, g, b, a]: [u8; 4]) {
        self.frame.queued.push(QueuedDraw {
            dst: Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            corners: Some([pts[0], pts[1], pts[2], pts[2]]),
            uv: [0.0, 0.0, 1.0, 1.0],
            tint: Color::rgba(r, g, b, a).to_f32_srgb(),
            operation: DrawOperation::Quad {
                texture: QuadTexture::White,
                blend: BlendMode::Blend,
            },
        });
    }

    /// Tint the current frame toward `desired_hue` with `scale`
    /// intensity (0..1). Used by the pause menu to dim + colour-shift
    /// the in-game image while the menu is up.
    ///
    /// Per-pixel HSV-replace using the desired hue with the source
    /// pixel's saturation and value (after a `(2*scale, scale, scale)`
    /// pre-scale that gives the dim a warm bias). Implemented as a
    /// fullscreen quad through the `fs_colorize` pipeline that samples
    /// the modal-snapshot texture (`freeze_scene_for_modal`) and writes
    /// the recoloured result into the offscreen RT, replacing the
    /// unmodified frozen-scene quad that would otherwise have been
    /// pushed by `present()`.
    pub fn colorize_framebuffer(&mut self, desired_hue: f32, scale: f32) {
        if self.frame.frozen_scene.is_none() {
            // No snapshot to recolour — colorize is a no-op outside
            // the modal flow. The recolour runs against the scene
            // snapshot captured by `freeze_scene_for_modal`.
            return;
        }
        let scale = scale.clamp(0.0, 1.0);
        let hue = desired_hue.rem_euclid(360.0) / 360.0;
        self.frame.queued.push(QueuedDraw {
            dst: Rect {
                x: 0,
                y: 0,
                w: self.frame.width as i32,
                h: self.frame.height as i32,
            },
            corners: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            tint: [hue, scale, 0.0, 0.0],
            operation: DrawOperation::ColorizeFrozen,
        });
    }

    pub fn render_framebuffer_alpha_rect(
        &mut self,
        dst_rect: Rect,
        uv: [f32; 4],
        color: u32,
        alpha_256: u32,
    ) -> bool {
        if dst_rect.w <= 0 || dst_rect.h <= 0 {
            return false;
        }
        let r = ((color >> 16) & 0xFF) as f32 / 255.0;
        let g = ((color >> 8) & 0xFF) as f32 / 255.0;
        let b = (color & 0xFF) as f32 / 255.0;
        let a = alpha_256.min(256) as f32 / 256.0;
        self.frame.queued.push(QueuedDraw {
            dst: dst_rect,
            corners: None,
            uv,
            tint: [r, g, b, a],
            operation: DrawOperation::FramebufferAlpha,
        });
        true
    }

    pub fn render_view_cone_span(
        &mut self,
        dst_rect: Rect,
        tint: (u8, u8, u8),
        alpha_left: u8,
        alpha_right: u8,
    ) {
        if dst_rect.w <= 0 || dst_rect.h <= 0 {
            return;
        }
        self.frame.queued.push(QueuedDraw {
            dst: dst_rect,
            corners: None,
            uv: [
                alpha_left as f32 / 255.0,
                0.0,
                alpha_right as f32 / 255.0,
                0.0,
            ],
            tint: [
                tint.0 as f32 / 255.0,
                tint.1 as f32 / 255.0,
                tint.2 as f32 / 255.0,
                1.0,
            ],
            operation: DrawOperation::ViewConeGradient,
        });
    }

    /// Render a string of native-font text by emitting one quad per
    /// glyph against the font's cached atlas texture.
    ///
    /// The atlas (built once per `NativeFont` via
    /// `NativeFont::build_rgba_atlas`) holds every glyph laid out in
    /// a single horizontal strip; alpha comes from the font's
    /// alpha-channel picture. Per-string layout uses
    /// `NativeFont::layout_quads` for the same spacing rules as the
    /// CPU-path glyph rasterization. Result: zero per-string upload —
    /// dynamic labels (counters, FPS overlay, dialogue) cost only
    /// `len(text)` quads in the GPU queue.
    pub fn render_text_argb(
        &mut self,
        font: &crate::native_font::NativeFont,
        text: &str,
        x: i32,
        y: i32,
    ) {
        if text.is_empty() || font.height() == 0 {
            return;
        }
        let atlas_bg = self.resources.ensure_font_atlas(&self.gpu, font);
        let tex_idx = self.queue_cached_bg(atlas_bg);
        for q in font.layout_quads(text, x, y) {
            self.frame.queued.push(QueuedDraw {
                dst: Rect {
                    x: q.dst_x,
                    y: q.dst_y,
                    w: q.dst_w as i32,
                    h: q.dst_h as i32,
                },
                corners: None,
                uv: [q.u0, q.v0, q.u1, q.v1],
                tint: [1.0, 1.0, 1.0, 1.0],
                operation: DrawOperation::Quad {
                    texture: QuadTexture::Frame(tex_idx),
                    blend: BlendMode::Blend,
                },
            });
        }
    }

    /// Render a string with a `.tfn`-backed TrueType font. Equivalent
    /// to a `TTF_RenderUNICODE_Solid` call that produces a per-string
    /// ARGB surface and blits it.
    ///
    /// `ab_glyph` doesn't ship a glyph atlas (and the .tfn font set is
    /// only used for list views, so per-string upload cost is trivial),
    /// so we rasterise into a temporary RGBA buffer sized by
    /// `font.total_pixel_height()` and upload as a one-shot wgpu
    /// texture, then queue the same blended quad the native-font path uses.
    pub fn render_text_truetype(&mut self, font: &TrueTypeFont, text: &str, x: i32, y: i32) {
        if text.is_empty() || !font.is_valid() {
            return;
        }
        let raw_w = font.get_string_width_total(text.chars().map(u32::from));
        if raw_w <= 0 {
            return;
        }
        // Pad horizontally — italic / wide glyphs occasionally extend
        // past the cumulative h_advance (especially the final glyph's
        // right side). The TTF_RenderUNICODE_Solid surface includes the
        // same overshoot.
        let overhang_pad = (text.chars().count() as u32).saturating_mul(2).min(128);
        let w = raw_w as u32 + 16 + overhang_pad;
        let h = font.total_pixel_height();
        if w == 0 || h == 0 {
            return;
        }
        let pitch = (w as usize) * 4;
        let mut rgba = vec![0u8; pitch * h as usize];
        font.render_to_rgba(&mut rgba, w as i32, h as i32, pitch, text, 0, 0);
        let (_tex, view) = upload_rgba_texture(
            &self.gpu,
            &self.resources.uploads,
            &rgba,
            w,
            h,
            "tt scratch",
        );
        let tex_idx = self.queue_frame_texture(&view);
        self.frame.queued.push(QueuedDraw {
            dst: Rect {
                x,
                y,
                w: w as i32,
                h: h as i32,
            },
            corners: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            tint: [1.0, 1.0, 1.0, 1.0],
            operation: DrawOperation::Quad {
                texture: QuadTexture::Frame(tex_idx),
                blend: BlendMode::Blend,
            },
        });
    }

    /// Release bitmap font atlases when leaving a loading/resource phase.
    /// Font identities prevent stale hits independently of this reclamation.
    pub fn clear_font_atlas_cache(&mut self) {
        self.resources.clear_font_atlas_cache();
    }

    /// Public helper for callers that own their own wgpu textures
    /// (titbit_renderer, campaign_map background) and want to enqueue
    /// them through the renderer's draw queue. The caller is
    /// responsible for keeping the texture alive until `present()`
    /// runs at end of frame.
    pub fn enqueue_external_texture(
        &mut self,
        view: &wgpu::TextureView,
        dst: Rect,
        uv: [f32; 4],
        tint: [f32; 4],
        blend: BlendMode,
    ) {
        let tex_idx = self.queue_frame_texture(view);
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

    /// Convenience wrapper around [`upload_rgba_texture`] for callers
    /// that build their own pixel buffers and want a texture+view they
    /// can hold across frames. The texture isn't tracked by the
    /// renderer — the caller manages its lifetime.
    pub fn create_static_rgba_texture(
        &self,
        rgba: &[u8],
        width: u32,
        height: u32,
        label: &str,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        upload_rgba_texture(
            &self.gpu,
            &self.resources.uploads,
            rgba,
            width,
            height,
            label,
        )
    }
}
