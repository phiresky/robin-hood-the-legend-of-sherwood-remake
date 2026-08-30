//! Swapchain, render-target, and transient frame-recording ownership.

use crate::gfx_types::Rect;
use crate::gpu_upscale::GpuUpscale;
use crate::window::{GpuContext, PresentationRect, SharedSurface};

use super::pipelines::{PipelineStore, SPRITE_STENCIL_FORMAT, blend_index};
use super::resources::GpuResources;
use super::{
    QuadVertex, QueuedDraw, ScreenUniform, TextureRef, log_fps, make_alpha_source, make_tex_bg,
    present_time_record, upload_counter,
};

pub(super) struct FrameState {
    pub(super) width: u16,
    pub(super) height: u16,
    gpu_phase_active: bool,
    shader_frame_count: Option<usize>,
    last_presented_shader_frame_count: usize,
    surface: SharedSurface,
    surface_config: Option<wgpu::SurfaceConfiguration>,
    pub(super) render_target_texture: wgpu::Texture,
    render_target_view: wgpu::TextureView,
    _sprite_stencil_texture: wgpu::Texture,
    sprite_stencil_view: wgpu::TextureView,
    render_target_bg: wgpu::BindGroup,
    alpha_source_texture: wgpu::Texture,
    alpha_source_view: wgpu::TextureView,
    alpha_source_bg: wgpu::BindGroup,
    screen_uniform: wgpu::Buffer,
    screen_bg: wgpu::BindGroup,
    swap_screen_uniform: wgpu::Buffer,
    swap_screen_bg: wgpu::BindGroup,
    vertex_buffer: Option<wgpu::Buffer>,
    vertex_capacity: u64,
    pub(super) queued: Vec<QueuedDraw>,
    pub(super) frame_texture_bgs: Vec<wgpu::BindGroup>,
    blit_vbo: Option<wgpu::Buffer>,
    cached_present_vbo: Option<wgpu::Buffer>,
    cached_present: Option<CachedPresentation>,
    native_refresh_presentation: bool,
    pub(super) frozen_scene: Option<(wgpu::Texture, wgpu::TextureView, wgpu::BindGroup)>,
}

/// Fully postprocessed frame retained between fixed simulation ticks. This is
/// deliberately downstream of RetroArch feedback/history passes: display-rate
/// repeats sample this immutable texture and cannot advance shader history.
struct CachedPresentation {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

impl FrameState {
    pub(super) fn present(
        &mut self,
        gpu: &GpuContext,
        pipelines: &mut PipelineStore,
        resources: &GpuResources,
    ) {
        let _ = self.present_impl(gpu, pipelines, resources, true);
    }

    /// Present the already-composited logical render target again without
    /// executing game rendering or consuming any queued draw/state. This is
    /// the side-effect-free presentation primitive used between 25 Hz ticks.
    pub(super) fn present_cached(
        &mut self,
        gpu: &GpuContext,
        pipelines: &mut PipelineStore,
        resources: &GpuResources,
    ) -> bool {
        self.present_impl(gpu, pipelines, resources, false)
    }

    fn present_impl(
        &mut self,
        gpu: &GpuContext,
        pipelines: &mut PipelineStore,
        resources: &GpuResources,
        compose_logical_frame: bool,
    ) -> bool {
        let present_start = web_time::Instant::now();
        if !compose_logical_frame
            && (!self.native_refresh_presentation || self.cached_present.is_none())
        {
            return false;
        }
        let shader_frame_count = Some(shader_frame_for_present(
            &mut self.shader_frame_count,
            &mut self.last_presented_shader_frame_count,
            compose_logical_frame,
        ));
        if compose_logical_frame {
            self.push_implicit_base_quad();
            self.upload_queue_geometry(gpu);
        }

        // Acquire swapchain frame. A suboptimal frame is still presented
        // this cycle; the reconfigure must wait until the acquired texture
        // has been handed back via `present` — configuring with an
        // outstanding surface texture is a wgpu validation error.
        let mut reconfigure_after_present = false;
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            wgpu::CurrentSurfaceTexture::Suboptimal(f) => {
                reconfigure_after_present = true;
                f
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.reconfigure_surface(gpu);
                if compose_logical_frame {
                    self.clear_recording();
                }
                return false;
            }
            status => {
                tracing::warn!("get_current_texture: {status:?}");
                if compose_logical_frame {
                    self.clear_recording();
                }
                return false;
            }
        };
        let swap_view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let swap_w = frame.texture.width();
        let swap_h = frame.texture.height();
        if compose_logical_frame && self.native_refresh_presentation {
            self.ensure_cached_presentation(gpu, resources, swap_w, swap_h);
        }
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("present"),
            });

        if compose_logical_frame {
            self.encode_pass1_to_rt(&mut encoder, pipelines, resources);
        }

        // ── Pass 2: blit RT into swapchain with letterbox ──
        // Compute the largest aspect-correct dst rect that fits in the
        // swapchain. Bars outside the dst are the clear-to-black.
        let presentation = PresentationRect::aspect_fit(
            u32::from(self.width),
            u32::from(self.height),
            swap_w,
            swap_h,
        );
        let dx = presentation.x;
        let dy = presentation.y;
        let dst_w = presentation.width;
        let dst_h = presentation.height;

        // One-quad vertex buffer for the blit. Build it on a separate
        // small per-frame buffer so it can't collide with the queue's
        // shared vbo offset usage.
        let blit_verts = [
            QuadVertex {
                pos: [dx, dy],
                uv: [0.0, 0.0],
                tint: [1.0; 4],
            },
            QuadVertex {
                pos: [dx + dst_w, dy],
                uv: [1.0, 0.0],
                tint: [1.0; 4],
            },
            QuadVertex {
                pos: [dx, dy + dst_h],
                uv: [0.0, 1.0],
                tint: [1.0; 4],
            },
            QuadVertex {
                pos: [dx, dy + dst_h],
                uv: [0.0, 1.0],
                tint: [1.0; 4],
            },
            QuadVertex {
                pos: [dx + dst_w, dy],
                uv: [1.0, 0.0],
                tint: [1.0; 4],
            },
            QuadVertex {
                pos: [dx + dst_w, dy + dst_h],
                uv: [1.0, 1.0],
                tint: [1.0; 4],
            },
        ];
        // Reuse the blit vbo across frames — it's always 6 vertices,
        // only the contents change as the letterbox dst rect adapts
        // to the swapchain size.
        if self.blit_vbo.is_none() {
            self.blit_vbo = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("blit vbo"),
                size: std::mem::size_of_val(&blit_verts) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let blit_vbo = self.blit_vbo.as_ref().unwrap();
        gpu.queue
            .write_buffer(blit_vbo, 0, bytemuck::cast_slice(&blit_verts));

        // Pass-2 screen uniform lives in its own buffer so the
        // pass-1 uniform write isn't overwritten before the GPU
        // executes pass 1 (queue.write_buffer + a single submit
        // means both passes see the latest value of any buffer they
        // share).
        let screen_swap = ScreenUniform {
            screen_size: [swap_w as f32, swap_h as f32],
            _pad: [0.0; 2],
        };
        gpu.queue.write_buffer(
            &self.swap_screen_uniform,
            0,
            bytemuck::bytes_of(&screen_swap),
        );

        let presentation_target = if self.native_refresh_presentation {
            &self
                .cached_present
                .as_ref()
                .expect("native-refresh presentation cache was not created")
                .view
        } else {
            &swap_view
        };
        let multipass_upscale = GpuUpscale::is_multipass_mode(pipelines.scale_mode);
        let multipass_rendered = compose_logical_frame
            && multipass_upscale
            && pipelines
                .gpu_upscale
                .render_multipass(
                    pipelines.scale_mode,
                    &mut encoder,
                    &self.render_target_texture,
                    presentation_target,
                    [swap_w, swap_h],
                    [dx, dy, dst_w, dst_h],
                    shader_frame_count,
                    Some(pipelines.shader_preset.as_str()),
                )
                .is_some();
        if compose_logical_frame && !multipass_rendered {
            // Shader-based upscalers (sharp-bilinear, bicubic, lanczos,
            // CUT3, scale2x/3x, xBR) want their own pipeline + uniforms.
            // Build the per-frame source bind group + uniform write here
            // so the borrow on `gpu_upscale` doesn't outlive the pass.
            let upscale_state = if pipelines.scale_mode.needs_shader() && !multipass_upscale {
                pipelines
                    .gpu_upscale
                    .pipeline_for(pipelines.scale_mode)
                    .map(|up| {
                        let uniforms = crate::gpu_upscale::FrameUniforms {
                            src: [
                                self.width as f32,
                                self.height as f32,
                                1.0 / self.width as f32,
                                1.0 / self.height as f32,
                            ],
                            dst: [dst_w, dst_h, 1.0 / dst_w, 1.0 / dst_h],
                        };
                        gpu.queue.write_buffer(
                            &up.uniform_buffer,
                            0,
                            bytemuck::bytes_of(&uniforms),
                        );
                        let tex_bg = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("upscale src bg"),
                            layout: &up.bind_group_layout_tex,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: wgpu::BindingResource::TextureView(
                                        &self.render_target_view,
                                    ),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: wgpu::BindingResource::Sampler(&up.sampler),
                                },
                            ],
                        });
                        (up, tex_bg)
                    })
            } else {
                None
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit RT → swapchain"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: presentation_target,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            if let Some((up, tex_bg)) = upscale_state.as_ref() {
                // Upscale shader path: attribute-less fullscreen
                // triangle constrained to the letterbox dst rect via
                // viewport. Bind groups 0/1 are empty placeholders;
                // 2 = source texture+sampler, 3 = src/dst uniforms.
                pass.set_viewport(dx, dy, dst_w, dst_h, 0.0, 1.0);
                pass.set_pipeline(&up.pipeline);
                pass.set_bind_group(0, &up.empty_bind_group, &[]);
                pass.set_bind_group(1, &up.empty_bind_group, &[]);
                pass.set_bind_group(2, tex_bg, &[]);
                pass.set_bind_group(3, &up.uniform_bind_group, &[]);
                pass.draw(0..3, 0..1);
            } else {
                // Plain Nearest / Linear / PixelArt path — straight
                // textured-quad blit through `blit_pipeline`.
                pass.set_pipeline(&pipelines.blit_pipeline);
                pass.set_bind_group(0, &self.swap_screen_bg, &[]);
                pass.set_bind_group(1, &self.render_target_bg, &[]);
                pass.set_vertex_buffer(0, blit_vbo.slice(..));
                pass.draw(0..6, 0..1);
            }
        }

        if self.native_refresh_presentation {
            self.encode_cached_present_to_swapchain(
                gpu,
                pipelines,
                &mut encoder,
                &swap_view,
                swap_w,
                swap_h,
            );
        }

        gpu.queue.submit(Some(encoder.finish()));
        gpu.queue.present(frame);
        if reconfigure_after_present {
            self.reconfigure_surface(gpu);
        }

        // Frame done — clear queues and reset GPU phase for next frame.
        if compose_logical_frame {
            let draws_this_frame = self.queued.len();
            let uploads_this_frame = upload_counter::take_count();
            let present_us = present_start.elapsed().as_micros() as u64;
            present_time_record(present_us);
            self.clear_recording();
            log_fps(draws_this_frame, uploads_this_frame);
        }
        true
    }

    fn ensure_cached_presentation(
        &mut self,
        gpu: &GpuContext,
        resources: &GpuResources,
        width: u32,
        height: u32,
    ) {
        if self
            .cached_present
            .as_ref()
            .is_some_and(|cached| cached.width == width && cached.height == height)
        {
            return;
        }
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fully postprocessed presentation cache"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: gpu.surface_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = make_tex_bg(
            &gpu.device,
            &resources.bgl_tex,
            &view,
            &resources.sampler,
            "fully postprocessed presentation cache bg",
        );
        self.cached_present = Some(CachedPresentation {
            _texture: texture,
            view,
            bind_group,
            width,
            height,
        });
    }

    fn encode_cached_present_to_swapchain(
        &mut self,
        gpu: &GpuContext,
        pipelines: &PipelineStore,
        encoder: &mut wgpu::CommandEncoder,
        swap_view: &wgpu::TextureView,
        swap_w: u32,
        swap_h: u32,
    ) {
        let verts = [
            QuadVertex {
                pos: [0.0, 0.0],
                uv: [0.0, 0.0],
                tint: [1.0; 4],
            },
            QuadVertex {
                pos: [swap_w as f32, 0.0],
                uv: [1.0, 0.0],
                tint: [1.0; 4],
            },
            QuadVertex {
                pos: [0.0, swap_h as f32],
                uv: [0.0, 1.0],
                tint: [1.0; 4],
            },
            QuadVertex {
                pos: [0.0, swap_h as f32],
                uv: [0.0, 1.0],
                tint: [1.0; 4],
            },
            QuadVertex {
                pos: [swap_w as f32, 0.0],
                uv: [1.0, 0.0],
                tint: [1.0; 4],
            },
            QuadVertex {
                pos: [swap_w as f32, swap_h as f32],
                uv: [1.0, 1.0],
                tint: [1.0; 4],
            },
        ];
        if self.cached_present_vbo.is_none() {
            self.cached_present_vbo = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cached presentation vbo"),
                size: std::mem::size_of_val(&verts) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let vbo = self
            .cached_present_vbo
            .as_ref()
            .expect("cached presentation vbo was not created");
        gpu.queue.write_buffer(vbo, 0, bytemuck::cast_slice(&verts));
        let cached = self
            .cached_present
            .as_ref()
            .expect("cached presentation requested before a completed frame");
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("cached presentation → swapchain"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: swap_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipelines.blit_pipeline);
        pass.set_bind_group(0, &self.swap_screen_bg, &[]);
        pass.set_bind_group(1, &cached.bind_group, &[]);
        pass.set_vertex_buffer(0, vbo.slice(..));
        pass.draw(0..6, 0..1);
    }

    pub(super) fn dimensions(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    pub(super) fn is_gpu_phase(&self) -> bool {
        self.gpu_phase_active
    }

    pub(super) fn enter_gpu_phase(&mut self) {
        self.gpu_phase_active = true;
    }

    pub(super) fn freeze_scene(&mut self, gpu: &GpuContext, resources: &GpuResources) {
        if self.frozen_scene.is_some() {
            return;
        }
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("frozen scene"),
            size: wgpu::Extent3d {
                width: self.width as u32,
                height: self.height as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("freeze scene"),
            });
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.render_target_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: self.width as u32,
                height: self.height as u32,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit(Some(encoder.finish()));
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = make_tex_bg(
            &gpu.device,
            &resources.bgl_tex,
            &view,
            &resources.sampler,
            "frozen scene bg",
        );
        self.frozen_scene = Some((texture, view, bind_group));
        self.enter_gpu_phase();
    }

    pub(super) fn clear_frozen_scene(&mut self) {
        self.frozen_scene = None;
    }

    pub(super) fn set_shader_frame_count(&mut self, frame_count: Option<usize>) {
        self.shader_frame_count = frame_count;
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        gpu: &GpuContext,
        resources: &GpuResources,
        screen_uniform: wgpu::Buffer,
        screen_bg: wgpu::BindGroup,
        swap_screen_uniform: wgpu::Buffer,
        swap_screen_bg: wgpu::BindGroup,
        surface: SharedSurface,
        surface_config: Option<wgpu::SurfaceConfiguration>,
        width: u16,
        height: u16,
    ) -> Self {
        let render_target_texture = create_render_target(&gpu.device, width, height);
        let render_target_view =
            render_target_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let (sprite_stencil_texture, sprite_stencil_view) =
            make_sprite_stencil_texture(&gpu.device, width, height);
        let render_target_bg = make_tex_bg(
            &gpu.device,
            &resources.bgl_tex,
            &render_target_view,
            &resources.sampler,
            "rt bg",
        );
        let (alpha_source_texture, alpha_source_view, alpha_source_bg) = make_alpha_source(
            &gpu.device,
            &resources.bgl_tex,
            &resources.sampler,
            width,
            height,
        );
        let native_refresh_presentation = surface_config
            .as_ref()
            .is_some_and(|config| config.present_mode == wgpu::PresentMode::Fifo);

        Self {
            width,
            height,
            gpu_phase_active: false,
            shader_frame_count: None,
            last_presented_shader_frame_count: 0,
            surface,
            surface_config,
            render_target_texture,
            render_target_view,
            _sprite_stencil_texture: sprite_stencil_texture,
            sprite_stencil_view,
            render_target_bg,
            alpha_source_texture,
            alpha_source_view,
            alpha_source_bg,
            screen_uniform,
            screen_bg,
            swap_screen_uniform,
            swap_screen_bg,
            vertex_buffer: None,
            vertex_capacity: 0,
            queued: Vec::new(),
            frame_texture_bgs: Vec::new(),
            blit_vbo: None,
            cached_present_vbo: None,
            cached_present: None,
            native_refresh_presentation,
            frozen_scene: None,
        }
    }

    pub(super) fn push_implicit_base_quad(&mut self) {
        if self.frozen_scene.is_some() {
            self.queued.insert(
                0,
                QueuedDraw {
                    dst: Rect {
                        x: 0,
                        y: 0,
                        w: self.width as i32,
                        h: self.height as i32,
                    },
                    corners: None,
                    uv: [0.0, 0.0, 1.0, 1.0],
                    tint: [1.0, 1.0, 1.0, 1.0],
                    tex: TextureRef::FrozenScene,
                    blend: crate::gfx_types::BlendMode::None,
                },
            );
        }
    }

    pub(super) fn upload_queue_geometry(&mut self, gpu: &GpuContext) {
        let mut verts: Vec<QuadVertex> = Vec::with_capacity(self.queued.len() * 6);
        for draw in &self.queued {
            let corners = draw.corners.unwrap_or_else(|| {
                let x0 = draw.dst.x as f32;
                let y0 = draw.dst.y as f32;
                let x1 = (draw.dst.x + draw.dst.w) as f32;
                let y1 = (draw.dst.y + draw.dst.h) as f32;
                [(x0, y0), (x1, y0), (x0, y1), (x1, y1)]
            });
            let [u0, v0, u1, v1] = draw.uv;
            let tl = QuadVertex {
                pos: [corners[0].0, corners[0].1],
                uv: [u0, v0],
                tint: draw.tint,
            };
            let tr = QuadVertex {
                pos: [corners[1].0, corners[1].1],
                uv: [u1, v0],
                tint: draw.tint,
            };
            let bl = QuadVertex {
                pos: [corners[2].0, corners[2].1],
                uv: [u0, v1],
                tint: draw.tint,
            };
            let br = QuadVertex {
                pos: [corners[3].0, corners[3].1],
                uv: [u1, v1],
                tint: draw.tint,
            };
            verts.extend_from_slice(&[tl, tr, bl, bl, tr, br]);
        }
        let needed = (verts.len() * std::mem::size_of::<QuadVertex>()) as u64;
        if needed > 0 {
            if self.vertex_capacity < needed {
                let capacity = needed.next_power_of_two().max(4096);
                self.vertex_buffer = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("quad vbo"),
                    size: capacity,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
                self.vertex_capacity = capacity;
            }
            if let Some(buffer) = &self.vertex_buffer {
                gpu.queue
                    .write_buffer(buffer, 0, bytemuck::cast_slice(&verts));
            }
        }
        let uniform = ScreenUniform {
            screen_size: [self.width as f32, self.height as f32],
            _pad: [0.0; 2],
        };
        gpu.queue
            .write_buffer(&self.screen_uniform, 0, bytemuck::bytes_of(&uniform));
    }

    pub(super) fn queue_cached_bg(&mut self, bind_group: wgpu::BindGroup) -> u32 {
        let index = self.frame_texture_bgs.len() as u32;
        self.frame_texture_bgs.push(bind_group);
        index
    }

    pub(super) fn queue_frame_texture(
        &mut self,
        gpu: &GpuContext,
        resources: &GpuResources,
        view: &wgpu::TextureView,
    ) -> u32 {
        let bind_group = make_tex_bg(
            &gpu.device,
            &resources.bgl_tex,
            view,
            &resources.sampler,
            "frame tex bg",
        );
        self.queue_cached_bg(bind_group)
    }

    pub(super) fn clear_recording(&mut self) {
        self.queued.clear();
        self.frame_texture_bgs.clear();
        self.gpu_phase_active = false;
    }

    fn reconfigure_surface(&self, gpu: &GpuContext) {
        if let Some(config) = &self.surface_config {
            self.surface.configure(&gpu.device, config);
        }
    }

    pub(super) fn configure_surface_size(&mut self, gpu: &GpuContext, width: u32, height: u32) {
        if let Some(config) = &mut self.surface_config {
            let width = width.max(1);
            let height = height.max(1);
            if config.width == width && config.height == height {
                return;
            }
            config.width = width;
            config.height = height;
            self.reconfigure_surface(gpu);
        }
    }

    pub(super) fn configure_present_mode(
        &mut self,
        gpu: &GpuContext,
        enabled: bool,
        surface_width: u32,
        surface_height: u32,
    ) {
        self.native_refresh_presentation = enabled;
        if !enabled {
            self.cached_present = None;
        }
        if let Some(config) = &mut self.surface_config {
            // GameWindow owns resize events. Synchronize its authoritative
            // physical dimensions before reconfiguring so changing present
            // mode cannot restore this renderer's older config clone.
            config.width = surface_width.max(1);
            config.height = surface_height.max(1);
            config.present_mode = if enabled {
                wgpu::PresentMode::Fifo
            } else {
                wgpu::PresentMode::AutoNoVsync
            };
            self.reconfigure_surface(gpu);
        }
    }

    pub(super) fn resize(
        &mut self,
        gpu: &GpuContext,
        resources: &GpuResources,
        width: u16,
        height: u16,
    ) {
        if width == 0 || height == 0 || (width == self.width && height == self.height) {
            return;
        }
        self.width = width;
        self.height = height;
        self.render_target_texture = create_render_target(&gpu.device, width, height);
        self.render_target_view = self
            .render_target_texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let (stencil_texture, stencil_view) =
            make_sprite_stencil_texture(&gpu.device, width, height);
        self._sprite_stencil_texture = stencil_texture;
        self.sprite_stencil_view = stencil_view;
        self.render_target_bg = make_tex_bg(
            &gpu.device,
            &resources.bgl_tex,
            &self.render_target_view,
            &resources.sampler,
            "rt bg",
        );
        let (texture, view, bind_group) = make_alpha_source(
            &gpu.device,
            &resources.bgl_tex,
            &resources.sampler,
            width,
            height,
        );
        self.alpha_source_texture = texture;
        self.alpha_source_view = view;
        self.alpha_source_bg = bind_group;
        // Keep an existing modal snapshot alive across the resize. It is
        // sampled with normalized UVs and intentionally scales to the new
        // logical target, avoiding a black backdrop until gameplay resumes
        // and redraws at the new dimensions. The gameplay path explicitly
        // clears the snapshot on its next frame.
    }
}

fn shader_frame_for_present(
    queued: &mut Option<usize>,
    last_presented: &mut usize,
    compose_logical_frame: bool,
) -> usize {
    if compose_logical_frame {
        let frame_count = queued
            .take()
            .unwrap_or_else(|| last_presented.wrapping_add(1));
        *last_presented = frame_count;
    }
    // Cached blits deliberately retain both values. In particular, returning
    // `None` to a temporal preset would advance its implicit frame counter.
    *last_presented
}

#[cfg(test)]
mod tests {
    use super::shader_frame_for_present;

    #[test]
    fn cached_presents_do_not_consume_or_advance_shader_frame_state() {
        let mut queued = Some(42);
        let mut last_presented = 7;

        assert_eq!(
            shader_frame_for_present(&mut queued, &mut last_presented, false),
            7
        );
        assert_eq!(queued, Some(42));
        assert_eq!(last_presented, 7);

        assert_eq!(
            shader_frame_for_present(&mut queued, &mut last_presented, true),
            42
        );
        assert_eq!(queued, None);
        assert_eq!(last_presented, 42);

        assert_eq!(
            shader_frame_for_present(&mut queued, &mut last_presented, false),
            42
        );
        assert_eq!(queued, None);
        assert_eq!(last_presented, 42);
    }
}

fn create_render_target(device: &wgpu::Device, width: u16, height: u16) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("logical render target"),
        size: wgpu::Extent3d {
            width: width as u32,
            height: height as u32,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn make_sprite_stencil_texture(
    device: &wgpu::Device,
    width: u16,
    height: u16,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sprite occlusion stencil"),
        size: wgpu::Extent3d {
            width: width as u32,
            height: height as u32,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SPRITE_STENCIL_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

impl FrameState {
    fn copy_rt_to_alpha_source(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.render_target_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &self.alpha_source_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: self.width as u32,
                height: self.height as u32,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Encode pass 1: queued draws → offscreen render target. Caller
    /// owns the encoder so it can either follow up with pass 2
    /// (`present`) or with a `copy_texture_to_buffer` (screenshot
    /// readback).
    pub(super) fn encode_pass1_to_rt(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &PipelineStore,
        resources: &GpuResources,
    ) {
        let first_framebuffer_alpha = self
            .queued
            .iter()
            .position(|d| matches!(d.tex, TextureRef::FramebufferAlpha));
        let Some(first_framebuffer_alpha) = first_framebuffer_alpha else {
            self.encode_pass1_range_to_rt(
                encoder,
                pipelines,
                resources,
                0,
                self.queued.len(),
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            );
            return;
        };

        self.encode_pass1_range_to_rt(
            encoder,
            pipelines,
            resources,
            0,
            first_framebuffer_alpha,
            wgpu::LoadOp::Clear(wgpu::Color::BLACK),
        );
        self.copy_rt_to_alpha_source(encoder);
        self.encode_pass1_range_to_rt(
            encoder,
            pipelines,
            resources,
            first_framebuffer_alpha,
            self.queued.len(),
            wgpu::LoadOp::Load,
        );
    }

    fn encode_pass1_range_to_rt(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &PipelineStore,
        resources: &GpuResources,
        start: usize,
        end: usize,
        load: wgpu::LoadOp<wgpu::Color>,
    ) {
        if start >= end && !matches!(load, wgpu::LoadOp::Clear(_)) {
            return;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("present quads → RT"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.render_target_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.sprite_stencil_view,
                depth_ops: None,
                stencil_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0),
                    store: wgpu::StoreOp::Discard,
                }),
            }),
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        let Some(vbo) = self.vertex_buffer.as_ref() else {
            return;
        };
        pass.set_bind_group(0, &self.screen_bg, &[]);
        pass.set_vertex_buffer(0, vbo.slice(..));

        /// Which fragment pipeline family is currently bound. Tracked
        /// separately from the blend index, since e.g. `Colorize` uses
        /// its own pipeline regardless of the queued draw's blend slot.
        #[derive(PartialEq, Clone, Copy)]
        enum BoundPipeline {
            Quad,
            MaskedQuad,
            Colorize,
            BgAlpha,
            ViewCone,
            MaskStencil,
            LoadingDissolve,
        }
        /// Which texture bind group is currently bound at slot 1.
        #[derive(PartialEq, Clone, Copy)]
        enum BoundTex {
            White,
            Frozen,
            AlphaSource,
            Frame,
            MaskAlpha,
            LoadingDissolve,
        }
        let mut last_pipeline: Option<BoundPipeline> = None;
        let mut last_blend: Option<usize> = None;
        let mut last_tex: Option<BoundTex> = None;
        let mut last_frame_idx: Option<u32> = None;
        for (i, d) in self.queued.iter().enumerate().take(end).skip(start) {
            match d.tex {
                TextureRef::ColorizeFromFrozen => {
                    if last_pipeline != Some(BoundPipeline::Colorize) {
                        pass.set_pipeline(&pipelines.colorize_pipeline);
                        last_pipeline = Some(BoundPipeline::Colorize);
                        last_blend = None;
                    }
                }
                TextureRef::FramebufferAlpha => {
                    if last_pipeline != Some(BoundPipeline::BgAlpha) {
                        pass.set_pipeline(&pipelines.bg_alpha_pipeline);
                        last_pipeline = Some(BoundPipeline::BgAlpha);
                        last_blend = None;
                    }
                }
                TextureRef::ViewConeGradient => {
                    if last_pipeline != Some(BoundPipeline::ViewCone) {
                        pass.set_pipeline(&pipelines.view_cone_pipeline);
                        last_pipeline = Some(BoundPipeline::ViewCone);
                        last_blend = None;
                    }
                }
                TextureRef::MaskAlpha(_) | TextureRef::StencilClear => {
                    if last_pipeline != Some(BoundPipeline::MaskStencil) {
                        pass.set_pipeline(&pipelines.mask_stencil_pipeline);
                        last_pipeline = Some(BoundPipeline::MaskStencil);
                        last_blend = None;
                    }
                    pass.set_stencil_reference(if matches!(d.tex, TextureRef::MaskAlpha(_)) {
                        1
                    } else {
                        0
                    });
                }
                TextureRef::MaskedFrame(_) => {
                    let bidx = blend_index(d.blend);
                    if last_pipeline != Some(BoundPipeline::MaskedQuad) || last_blend != Some(bidx)
                    {
                        pass.set_pipeline(&pipelines.masked_pipelines[bidx]);
                        pass.set_stencil_reference(0);
                        last_pipeline = Some(BoundPipeline::MaskedQuad);
                        last_blend = Some(bidx);
                    }
                }
                TextureRef::LoadingDissolveFrame(_) => {
                    if last_pipeline != Some(BoundPipeline::LoadingDissolve) {
                        pass.set_pipeline(&pipelines.loading_dissolve_pipeline);
                        last_pipeline = Some(BoundPipeline::LoadingDissolve);
                        last_blend = None;
                    }
                }
                _ => {
                    let bidx = blend_index(d.blend);
                    if last_pipeline != Some(BoundPipeline::Quad) || last_blend != Some(bidx) {
                        pass.set_pipeline(&pipelines.pipelines[bidx]);
                        last_pipeline = Some(BoundPipeline::Quad);
                        last_blend = Some(bidx);
                    }
                }
            }
            let need_rebind_tex = match d.tex {
                TextureRef::White => last_tex != Some(BoundTex::White),
                TextureRef::FrozenScene => last_tex != Some(BoundTex::Frozen),
                TextureRef::ColorizeFromFrozen => last_tex != Some(BoundTex::Frozen),
                TextureRef::FramebufferAlpha => last_tex != Some(BoundTex::AlphaSource),
                TextureRef::ViewConeGradient => last_tex != Some(BoundTex::White),
                TextureRef::Frame(idx) | TextureRef::MaskedFrame(idx) => {
                    last_tex != Some(BoundTex::Frame) || last_frame_idx != Some(idx)
                }
                TextureRef::MaskAlpha(idx) => {
                    last_tex != Some(BoundTex::MaskAlpha) || last_frame_idx != Some(idx)
                }
                TextureRef::StencilClear => last_tex != Some(BoundTex::White),
                TextureRef::LoadingDissolveFrame(idx) => {
                    last_tex != Some(BoundTex::LoadingDissolve) || last_frame_idx != Some(idx)
                }
            };
            if need_rebind_tex {
                match d.tex {
                    TextureRef::White => {
                        pass.set_bind_group(1, &resources.white_bg, &[]);
                        last_tex = Some(BoundTex::White);
                        last_frame_idx = None;
                    }
                    TextureRef::FrozenScene | TextureRef::ColorizeFromFrozen => {
                        if let Some((_, _, bg)) = self.frozen_scene.as_ref() {
                            pass.set_bind_group(1, bg, &[]);
                            last_tex = Some(BoundTex::Frozen);
                            last_frame_idx = None;
                        } else {
                            continue;
                        }
                    }
                    TextureRef::FramebufferAlpha => {
                        pass.set_bind_group(1, &self.alpha_source_bg, &[]);
                        last_tex = Some(BoundTex::AlphaSource);
                        last_frame_idx = None;
                    }
                    TextureRef::ViewConeGradient => {
                        pass.set_bind_group(1, &resources.white_bg, &[]);
                        last_tex = Some(BoundTex::White);
                        last_frame_idx = None;
                    }
                    TextureRef::Frame(idx) | TextureRef::MaskedFrame(idx) => {
                        if let Some(bg) = self.frame_texture_bgs.get(idx as usize) {
                            pass.set_bind_group(1, bg, &[]);
                            last_tex = Some(BoundTex::Frame);
                            last_frame_idx = Some(idx);
                        } else {
                            continue;
                        }
                    }
                    TextureRef::MaskAlpha(idx) => {
                        let mask = resources.mask_alpha_cache.get(&idx).unwrap_or_else(|| {
                            panic!("missing uploaded sprite mask {idx} during rendering")
                        });
                        pass.set_bind_group(1, &mask.bind_group, &[]);
                        last_tex = Some(BoundTex::MaskAlpha);
                        last_frame_idx = Some(idx);
                    }
                    TextureRef::StencilClear => {
                        pass.set_bind_group(1, &resources.white_bg, &[]);
                        last_tex = Some(BoundTex::White);
                        last_frame_idx = None;
                    }
                    TextureRef::LoadingDissolveFrame(idx) => {
                        if let Some(bg) = self.frame_texture_bgs.get(idx as usize) {
                            pass.set_bind_group(1, bg, &[]);
                            last_tex = Some(BoundTex::LoadingDissolve);
                            last_frame_idx = Some(idx);
                        } else {
                            continue;
                        }
                    }
                }
            }
            let v0 = (i * 6) as u32;
            pass.draw(v0..v0 + 6, 0..1);
        }
    }
}
