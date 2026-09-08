//! Swapchain, render-target, and transient frame-recording ownership.

use crate::gfx_types::Rect;
use crate::gpu_upscale::GpuUpscale;
use crate::window::{GpuContext, PresentationRect, SharedSurface};
use robin_engine::graphic_config::{TextureEffect, TextureScaleMode};

use super::pipelines::{PipelineStore, SPRITE_STENCIL_FORMAT, blend_index};
use super::resources::GpuResources;
use super::{
    DrawOperation, QuadTexture, QuadVertex, QueuedDraw, ScreenUniform, TextureSource, bind_counter,
    log_fps, make_alpha_source, make_tex_bg, present_time_record, upload_counter,
};

fn expand_queue_geometry(draws: &[QueuedDraw], verts: &mut Vec<QuadVertex>) {
    verts.clear();
    verts.reserve(draws.len() * 6);
    for draw in draws {
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
}

fn presentation_profile(
    ui_only_frame: bool,
    configured_mode: TextureScaleMode,
    configured_effect: TextureEffect,
) -> (TextureScaleMode, TextureEffect) {
    if ui_only_frame {
        // Match the sharp-bilinear UI composite. This is still valid at
        // fractional window scales, unlike forcing raw nearest-neighbour.
        (TextureScaleMode::PixelArt, TextureEffect::None)
    } else {
        (configured_mode, configured_effect)
    }
}

/// An ordered composition plan; the snapshot is taken once, immediately
/// before the first framebuffer-alpha draw, preserving legacy accumulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum CompositionPass {
    Draw {
        start: usize,
        end: usize,
        clear: bool,
    },
    SnapshotFramebuffer,
}

fn composition_plan(draws: &[QueuedDraw]) -> [Option<CompositionPass>; 3] {
    match draws
        .iter()
        .position(|draw| matches!(draw.operation, DrawOperation::FramebufferAlpha))
    {
        Some(index) => [
            Some(CompositionPass::Draw {
                start: 0,
                end: index,
                clear: true,
            }),
            Some(CompositionPass::SnapshotFramebuffer),
            Some(CompositionPass::Draw {
                start: index,
                end: draws.len(),
                clear: false,
            }),
        ],
        None => [
            Some(CompositionPass::Draw {
                start: 0,
                end: draws.len(),
                clear: true,
            }),
            None,
            None,
        ],
    }
}

pub(super) struct FrameState {
    pub(super) width: u16,
    pub(super) height: u16,
    gpu_phase_active: bool,
    /// Counts frames that actually reach the presentation queue. Temporal
    /// display effects deliberately advance here rather than on simulation
    /// ticks, so 90/120/144 Hz presentation remains smooth while paused.
    presentation_frame_count: usize,
    /// Top-level screens such as the main menu have no separate world
    /// layer, but still need to remain outside the configured gameplay
    /// upscaler/effect chain. They render into the ordinary logical target
    /// (so modal freezing keeps working) and request the same sharp
    /// presentation profile used by the split UI layer.
    ui_only_frame: bool,
    /// Absent only for explicit offscreen rendering; presentation requires it.
    surface: Option<SharedSurface>,
    surface_config: Option<wgpu::SurfaceConfiguration>,
    pub(super) render_target_texture: wgpu::Texture,
    render_target_view: wgpu::TextureView,
    ui_target_texture: wgpu::Texture,
    ui_target_view: wgpu::TextureView,
    _sprite_stencil_texture: wgpu::Texture,
    sprite_stencil_view: wgpu::TextureView,
    _ui_stencil_texture: wgpu::Texture,
    ui_stencil_view: wgpu::TextureView,
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
    /// CPU staging allocation retained alongside the GPU vertex allocation.
    vertex_scratch: Vec<QuadVertex>,
    pub(super) queued: Vec<QueuedDraw>,
    ui_layer_start: Option<usize>,
    pub(super) frame_texture_bgs: Vec<wgpu::BindGroup>,
    /// Atlas layer → its index in `frame_texture_bgs` for this frame.
    /// Small and dense (a mission uses a handful of layers), so a Vec
    /// of `(layer, idx)` beats a hash map.
    atlas_bg_slots: Vec<(u32, u32)>,
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
        let presentation_frame_count = self.presentation_frame_count;
        if compose_logical_frame {
            self.push_implicit_base_quad();
            self.upload_queue_geometry(gpu);
        }

        // Acquire swapchain frame. A suboptimal frame is still presented
        // this cycle; the reconfigure must wait until the acquired texture
        // has been handed back via `present` — configuring with an
        // outstanding surface texture is a wgpu validation error.
        let mut reconfigure_after_present = false;
        let acquire_start = web_time::Instant::now();
        let frame = match self
            .surface
            .as_ref()
            .expect("offscreen renderer cannot present to a surface")
            .get_current_texture()
        {
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
        let acquire_us = acquire_start.elapsed().as_micros();
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
            self.encode_scene_to_rt(&mut encoder, pipelines, resources);
            if let Some(ui_start) = self.ui_layer_start
                && ui_start < self.queued.len()
            {
                self.encode_pass_range_to_target(
                    &mut encoder,
                    pipelines,
                    resources,
                    ui_start,
                    self.queued.len(),
                    &self.ui_target_view,
                    &self.ui_stencil_view,
                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                );
            }
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
        let (scale_mode, texture_effect) = presentation_profile(
            self.ui_only_frame,
            pipelines.scale_mode,
            pipelines.texture_effect,
        );
        let multipass_upscale = GpuUpscale::is_multipass_mode(scale_mode, texture_effect);
        let multipass_rendered = if compose_logical_frame && multipass_upscale {
            pipelines
                .gpu_upscale
                .render_multipass(
                    scale_mode,
                    &mut encoder,
                    &self.render_target_texture,
                    presentation_target,
                    [swap_w, swap_h],
                    [dx, dy, dst_w, dst_h],
                    Some(presentation_frame_count),
                    Some(pipelines.shader_preset.as_str()),
                    texture_effect,
                    pipelines.upscale_parameters,
                    pipelines.texture_effect_parameters,
                )
                .unwrap_or_else(|error| {
                    panic!(
                        "selected upscaler/effect failed instead of silently falling back: {error}"
                    )
                })
        } else {
            false
        };
        if compose_logical_frame && !multipass_rendered {
            // Shader-based upscalers (sharp-bilinear, bicubic, lanczos,
            // CUT3, scale2x/3x, xBR) want their own pipeline + uniforms.
            // Build the per-frame source bind group + uniform write here
            // so the borrow on `gpu_upscale` doesn't outlive the pass.
            let upscale_state = if scale_mode.needs_shader() && !multipass_upscale {
                let selected_mode = scale_mode;
                let up = pipelines
                    .gpu_upscale
                    .pipeline_for(selected_mode)
                    .unwrap_or_else(|| {
                        panic!(
                            "selected shader mode {selected_mode:?} has no pipeline; refusing fallback"
                        )
                    });
                let uniforms = crate::gpu_upscale::FrameUniforms {
                    src: [
                        self.width as f32,
                        self.height as f32,
                        1.0 / self.width as f32,
                        1.0 / self.height as f32,
                    ],
                    dst: [dst_w, dst_h, 1.0 / dst_w, 1.0 / dst_h],
                };
                gpu.queue
                    .write_buffer(&up.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
                let tex_bg = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("upscale src bg"),
                    layout: &up.bind_group_layout_tex,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&self.render_target_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&up.sampler),
                        },
                    ],
                });
                Some((up, tex_bg))
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

        if compose_logical_frame
            && self
                .ui_layer_start
                .is_some_and(|start| start < self.queued.len())
        {
            pipelines.gpu_upscale.render_ui_overlay(
                &mut encoder,
                &self.ui_target_texture,
                presentation_target,
                [dx as f32, dy as f32, dst_w, dst_h],
            );
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

        let submit_start = web_time::Instant::now();
        gpu.queue.submit(Some(encoder.finish()));
        let submit_us = submit_start.elapsed().as_micros();
        let swap_start = web_time::Instant::now();
        gpu.queue.present(frame);
        tracing::trace!(target: "present_perf", acquire_us, submit_us,
            swap_us = swap_start.elapsed().as_micros(),
            total_us = present_start.elapsed().as_micros(), "present phases");
        self.presentation_frame_count = self.presentation_frame_count.wrapping_add(1);
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
            log_fps(
                draws_this_frame,
                uploads_this_frame,
                bind_counter::take_count(),
                bind_counter::take_draw_calls(),
                resources,
            );
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

    pub(super) fn begin_ui_layer(&mut self) {
        if self.ui_layer_start.is_none() {
            self.ui_layer_start = Some(self.queued.len());
        }
    }

    pub(super) fn begin_ui_only_frame(&mut self) {
        self.ui_only_frame = true;
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

    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        gpu: &GpuContext,
        resources: &GpuResources,
        screen_uniform: wgpu::Buffer,
        screen_bg: wgpu::BindGroup,
        swap_screen_uniform: wgpu::Buffer,
        swap_screen_bg: wgpu::BindGroup,
        surface: Option<SharedSurface>,
        surface_config: Option<wgpu::SurfaceConfiguration>,
        width: u16,
        height: u16,
    ) -> Self {
        let render_target_texture = create_render_target(&gpu.device, width, height);
        let render_target_view =
            render_target_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let ui_target_texture = create_render_target(&gpu.device, width, height);
        let ui_target_view = ui_target_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let (sprite_stencil_texture, sprite_stencil_view) =
            make_sprite_stencil_texture(&gpu.device, width, height);
        let (ui_stencil_texture, ui_stencil_view) =
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
            presentation_frame_count: 0,
            ui_only_frame: false,
            surface,
            surface_config,
            render_target_texture,
            render_target_view,
            ui_target_texture,
            ui_target_view,
            _sprite_stencil_texture: sprite_stencil_texture,
            sprite_stencil_view,
            _ui_stencil_texture: ui_stencil_texture,
            ui_stencil_view,
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
            vertex_scratch: Vec::new(),
            queued: Vec::new(),
            ui_layer_start: None,
            frame_texture_bgs: Vec::new(),
            atlas_bg_slots: Vec::new(),
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
                    operation: DrawOperation::Quad {
                        texture: QuadTexture::FrozenScene,
                        blend: crate::gfx_types::BlendMode::None,
                    },
                },
            );
            if let Some(start) = &mut self.ui_layer_start {
                *start += 1;
            }
        }
    }

    pub(super) fn upload_queue_geometry(&mut self, gpu: &GpuContext) {
        expand_queue_geometry(&self.queued, &mut self.vertex_scratch);
        let verts = &self.vertex_scratch;
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

    /// This frame's `frame_texture_bgs` index for an atlas layer, if it
    /// has already been queued.
    pub(super) fn atlas_bg_slot(&self, layer: u32) -> Option<u32> {
        self.atlas_bg_slots
            .iter()
            .find(|&&(l, _)| l == layer)
            .map(|&(_, idx)| idx)
    }

    pub(super) fn remember_atlas_bg_slot(&mut self, layer: u32, index: u32) {
        self.atlas_bg_slots.push((layer, index));
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

    pub(super) fn finish_loading_screen(&mut self, gpu: &GpuContext) {
        self.clear_recording();
        self.clear_frozen_scene();
        self.cached_present = None;
        self.presentation_frame_count = 0;
        self.vertex_scratch.clear();
        // Lost-Sherwood debriefing can freeze the target before any world
        // composition. Match freshly allocated targets instead of preserving
        // loading artwork underneath that first modal.
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("loading renderer handoff clear"),
            });
        for view in [&self.render_target_view, &self.ui_target_view] {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("loading renderer handoff clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        gpu.queue.submit(Some(encoder.finish()));
    }

    pub(super) fn clear_recording(&mut self) {
        self.queued.clear();
        self.frame_texture_bgs.clear();
        self.atlas_bg_slots.clear();
        self.gpu_phase_active = false;
        self.ui_layer_start = None;
        self.ui_only_frame = false;
    }

    fn reconfigure_surface(&self, gpu: &GpuContext) {
        if let Some(config) = &self.surface_config {
            self.surface
                .as_ref()
                .expect("configured renderer requires a surface")
                .configure(&gpu.device, config);
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
        // Logical resizes include temporary full-map captures. Do not retain
        // their potentially much larger CPU staging allocation on return to
        // the ordinary viewport; steady-size presentation still reuses it.
        self.vertex_scratch = Vec::new();
        self.render_target_texture = create_render_target(&gpu.device, width, height);
        self.render_target_view = self
            .render_target_texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.ui_target_texture = create_render_target(&gpu.device, width, height);
        self.ui_target_view = self
            .ui_target_texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let (stencil_texture, stencil_view) =
            make_sprite_stencil_texture(&gpu.device, width, height);
        self._sprite_stencil_texture = stencil_texture;
        self.sprite_stencil_view = stencil_view;
        let (ui_stencil_texture, ui_stencil_view) =
            make_sprite_stencil_texture(&gpu.device, width, height);
        self._ui_stencil_texture = ui_stencil_texture;
        self.ui_stencil_view = ui_stencil_view;
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

#[cfg(test)]
mod presentation_tests {
    use super::*;

    fn quad(operation: DrawOperation) -> QueuedDraw {
        QueuedDraw {
            dst: Rect::new(2, 3, 4, 5),
            corners: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            tint: [1.0; 4],
            operation,
        }
    }

    #[test]
    fn capture_and_world_preserve_snapshot_boundary() {
        let draws = [
            quad(DrawOperation::StencilClear),
            quad(DrawOperation::FramebufferAlpha),
            quad(DrawOperation::FramebufferAlpha),
        ];
        assert_eq!(
            composition_plan(&draws),
            [
                Some(CompositionPass::Draw {
                    start: 0,
                    end: 1,
                    clear: true
                }),
                Some(CompositionPass::SnapshotFramebuffer),
                Some(CompositionPass::Draw {
                    start: 1,
                    end: 3,
                    clear: false
                }),
            ]
        );
        // An alpha effect in UI must not cause a world snapshot.
        assert_eq!(
            composition_plan(&draws[..1]),
            [
                Some(CompositionPass::Draw {
                    start: 0,
                    end: 1,
                    clear: true
                }),
                None,
                None,
            ]
        );
        assert_eq!(
            composition_plan(&[]),
            [
                Some(CompositionPass::Draw {
                    start: 0,
                    end: 0,
                    clear: true
                }),
                None,
                None,
            ]
        );
    }

    #[test]
    fn geometry_reuses_allocation_and_preserves_triangle_order() {
        let draws = [quad(DrawOperation::StencilClear)];
        let mut vertices = Vec::new();
        expand_queue_geometry(&draws, &mut vertices);
        let allocation = vertices.as_ptr();
        let expected = [
            [2.0, 3.0],
            [6.0, 3.0],
            [2.0, 8.0],
            [2.0, 8.0],
            [6.0, 3.0],
            [6.0, 8.0],
        ];
        assert_eq!(vertices.iter().map(|v| v.pos).collect::<Vec<_>>(), expected);
        for _ in 0..1000 {
            expand_queue_geometry(&draws, &mut vertices);
        }
        assert_eq!(
            allocation,
            vertices.as_ptr(),
            "steady-size frames must retain their staging allocation"
        );
        assert_eq!(vertices.len(), 6);
        let mut triangle = draws[0].clone();
        triangle.corners = Some([(0.0, 0.0), (2.0, 0.0), (1.0, 2.0), (1.0, 2.0)]);
        expand_queue_geometry(&[triangle], &mut vertices);
        assert_eq!(vertices[2].pos, vertices[5].pos);
    }

    #[test]
    fn ui_only_frames_bypass_gameplay_scaling_and_effects() {
        assert_eq!(
            presentation_profile(true, TextureScaleMode::Anime4kC, TextureEffect::CrtRoyale,),
            (TextureScaleMode::PixelArt, TextureEffect::None)
        );
    }

    #[test]
    fn gameplay_frames_keep_the_configured_profile() {
        assert_eq!(
            presentation_profile(false, TextureScaleMode::Anime4kC, TextureEffect::CrtRoyale,),
            (TextureScaleMode::Anime4kC, TextureEffect::CrtRoyale)
        );
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
        self.encode_composition(encoder, pipelines, resources, self.queued.len());
    }

    /// World-only composition leaves UI outside presentation effects; capture
    /// uses the same pass planner with the complete queue.
    fn encode_scene_to_rt(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &PipelineStore,
        resources: &GpuResources,
    ) {
        self.encode_composition(
            encoder,
            pipelines,
            resources,
            self.ui_layer_start.unwrap_or(self.queued.len()),
        );
    }

    fn encode_composition(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &PipelineStore,
        resources: &GpuResources,
        end: usize,
    ) {
        for operation in composition_plan(&self.queued[..end]).into_iter().flatten() {
            match operation {
                CompositionPass::SnapshotFramebuffer => self.copy_rt_to_alpha_source(encoder),
                CompositionPass::Draw { start, end, clear } => self.encode_pass_range_to_target(
                    encoder,
                    pipelines,
                    resources,
                    start,
                    end,
                    &self.render_target_view,
                    &self.sprite_stencil_view,
                    if clear {
                        wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                    } else {
                        wgpu::LoadOp::Load
                    },
                ),
            }
        }
    }

    fn encode_pass_range_to_target(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &PipelineStore,
        resources: &GpuResources,
        start: usize,
        end: usize,
        target_view: &wgpu::TextureView,
        stencil_view: &wgpu::TextureView,
        load: wgpu::LoadOp<wgpu::Color>,
    ) {
        if start >= end && !matches!(load, wgpu::LoadOp::Clear(_)) {
            return;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("present quads → RT"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: stencil_view,
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

        // Consecutive draws that need no state change are recorded as
        // one `draw` over a contiguous vertex range instead of one call
        // per quad. Quads are laid out in queue order (6 vertices at
        // `i * 6`), so a run is always contiguous. This is what the
        // atlas buys: sprites sharing a layer no longer rebind, so they
        // coalesce.
        //
        // `pending` is `(first_vertex, vertex_count)`. It MUST be
        // flushed before any pipeline/bind-group/stencil change and
        // before any `continue`, or those draws would be recorded
        // against the wrong state.
        let mut pending: Option<(u32, u32)> = None;
        macro_rules! flush_run {
            () => {
                if let Some((first, count)) = pending.take() {
                    bind_counter::inc_draw_call();
                    pass.draw(first..first + count, 0..1);
                }
            };
        }

        for (i, d) in self.queued.iter().enumerate().take(end).skip(start) {
            match d.operation {
                DrawOperation::ColorizeFrozen => {
                    if last_pipeline != Some(BoundPipeline::Colorize) {
                        flush_run!();
                        pass.set_pipeline(&pipelines.colorize_pipeline);
                        last_pipeline = Some(BoundPipeline::Colorize);
                        last_blend = None;
                    }
                }
                DrawOperation::FramebufferAlpha => {
                    if last_pipeline != Some(BoundPipeline::BgAlpha) {
                        flush_run!();
                        pass.set_pipeline(&pipelines.bg_alpha_pipeline);
                        last_pipeline = Some(BoundPipeline::BgAlpha);
                        last_blend = None;
                    }
                }
                DrawOperation::ViewConeGradient => {
                    if last_pipeline != Some(BoundPipeline::ViewCone) {
                        flush_run!();
                        pass.set_pipeline(&pipelines.view_cone_pipeline);
                        last_pipeline = Some(BoundPipeline::ViewCone);
                        last_blend = None;
                    }
                }
                DrawOperation::MaskAlpha(_) | DrawOperation::StencilClear => {
                    if last_pipeline != Some(BoundPipeline::MaskStencil) {
                        flush_run!();
                        pass.set_pipeline(&pipelines.mask_stencil_pipeline);
                        last_pipeline = Some(BoundPipeline::MaskStencil);
                        last_blend = None;
                    }
                    // Unconditional, so stencil draws never coalesce —
                    // they are rare and each carries its own reference.
                    flush_run!();
                    pass.set_stencil_reference(
                        if matches!(d.operation, DrawOperation::MaskAlpha(_)) {
                            1
                        } else {
                            0
                        },
                    );
                }
                DrawOperation::Masked { blend, .. } => {
                    let bidx = blend_index(blend);
                    if last_pipeline != Some(BoundPipeline::MaskedQuad) || last_blend != Some(bidx)
                    {
                        flush_run!();
                        pass.set_pipeline(&pipelines.masked_pipelines[bidx]);
                        pass.set_stencil_reference(0);
                        last_pipeline = Some(BoundPipeline::MaskedQuad);
                        last_blend = Some(bidx);
                    }
                }
                DrawOperation::LoadingDissolve(_) => {
                    if last_pipeline != Some(BoundPipeline::LoadingDissolve) {
                        flush_run!();
                        pass.set_pipeline(&pipelines.loading_dissolve_pipeline);
                        last_pipeline = Some(BoundPipeline::LoadingDissolve);
                        last_blend = None;
                    }
                }
                DrawOperation::Quad { blend, .. } => {
                    let bidx = blend_index(blend);
                    if last_pipeline != Some(BoundPipeline::Quad) || last_blend != Some(bidx) {
                        flush_run!();
                        pass.set_pipeline(&pipelines.pipelines[bidx]);
                        last_pipeline = Some(BoundPipeline::Quad);
                        last_blend = Some(bidx);
                    }
                }
            }
            let texture = d.operation.texture();
            let need_rebind_tex = match texture {
                TextureSource::White => last_tex != Some(BoundTex::White),
                TextureSource::FrozenScene => last_tex != Some(BoundTex::Frozen),
                TextureSource::Framebuffer => last_tex != Some(BoundTex::AlphaSource),
                TextureSource::Frame(idx) => {
                    last_tex != Some(BoundTex::Frame) || last_frame_idx != Some(idx)
                }
                TextureSource::MaskAlpha(idx) => {
                    last_tex != Some(BoundTex::MaskAlpha) || last_frame_idx != Some(idx)
                }
                TextureSource::LoadingDissolve(idx) => {
                    last_tex != Some(BoundTex::LoadingDissolve) || last_frame_idx != Some(idx)
                }
            };
            if need_rebind_tex {
                flush_run!();
                bind_counter::inc();
                let (bg, kind, index) = match texture {
                    TextureSource::White => (&resources.white_bg, BoundTex::White, None),
                    TextureSource::FrozenScene => {
                        let (_, _, bg) = self
                            .frozen_scene
                            .as_ref()
                            .expect("frozen-scene draw requires a scene snapshot");
                        (bg, BoundTex::Frozen, None)
                    }
                    TextureSource::Framebuffer => {
                        (&self.alpha_source_bg, BoundTex::AlphaSource, None)
                    }
                    TextureSource::Frame(idx) | TextureSource::LoadingDissolve(idx) => {
                        let bg = self.frame_texture_bgs.get(idx as usize).unwrap_or_else(|| {
                            panic!("missing frame texture {idx} during rendering")
                        });
                        let kind = if matches!(texture, TextureSource::Frame(_)) {
                            BoundTex::Frame
                        } else {
                            BoundTex::LoadingDissolve
                        };
                        (bg, kind, Some(idx))
                    }
                    TextureSource::MaskAlpha(idx) => {
                        let mask = resources.mask_alpha_cache.get(&idx).unwrap_or_else(|| {
                            panic!("missing uploaded sprite mask {idx} during rendering")
                        });
                        (&mask.bind_group, BoundTex::MaskAlpha, Some(idx))
                    }
                };
                pass.set_bind_group(1, bg, &[]);
                last_tex = Some(kind);
                last_frame_idx = index;
            }
            let v0 = (i * 6) as u32;
            // `Option<(u32, u32)>` is `Copy`, so match by value — no
            // borrow of `pending` is held while it is reassigned.
            pending = match pending {
                // Contiguous with the run in progress: extend it.
                Some((first, count)) if first + count == v0 => Some((first, count + 6)),
                // A gap means a draw was skipped without flushing,
                // which the flush points above exist to prevent.
                Some((first, count)) => unreachable!(
                    "non-contiguous draw run: pending {first}+{count}, next quad at {v0}"
                ),
                None => Some((v0, 6)),
            };
        }
        flush_run!();
    }
}
