//! Cutscene video playback (Ogg Theora + Vorbis).
//!
//! Decodes Ogg Theora video + Vorbis audio via `ffmpeg-next`. Frames
//! are blitted directly through wgpu (a tiny self-contained pipeline
//! that samples a streaming RGBA texture and writes to the swapchain
//! at letterbox aspect). Audio is decoded fully into f32 stereo
//! samples upfront and handed to a temporary `kira::AudioManager` as
//! a `StaticSoundData` — kira drives the audio device while wgpu
//! pumps frames.
//!
//! The original game ships two cinematics:
//! `Data/Cinematics/Intro.ogg` and `Data/Cinematics/Outro.ogg`.
//!
//! ESC, mouse-click, or window-close all skip playback. Missing configured
//! cinematic content is returned as an explicit error to the calling menu.
//!
//! The whole module is gated on the `video` cargo feature. When the
//! feature is off (e.g. `wasm32-unknown-emscripten` builds, which
//! cannot link ffmpeg), [`play_video`] is exported but returns
//! `Ok(())` after logging.

#[cfg(not(feature = "video"))]
pub async fn play_video(
    _application_context: &crate::host::ApplicationContext,
    _window: &mut crate::window::GameWindow,
    path: &str,
) -> Result<(), String> {
    tracing::warn!("video feature disabled, skipping cinematic {path}");
    Ok(())
}

/// Whole-playback gate on the application sound flag. When sound is disabled
/// (`-NOSOUND` launcher flag), Intro/Outro buttons become no-ops rather
/// than playing the cinematic silently.
#[cfg(feature = "video")]
fn sound_enabled(application_context: &crate::host::ApplicationContext) -> bool {
    application_context.options().sound_enabled
}

#[cfg(feature = "video")]
use std::sync::{Arc, OnceLock};

#[cfg(feature = "video")]
use crate::window::GameWindow;
#[cfg(feature = "video")]
use robin_engine::sbfile::resolve_data_path;

/// One-shot ffmpeg initialization. Stores the result instead of using
/// `Once` + `expect` so a failed init surfaces as a playback error on
/// every attempt rather than poisoning the `Once` and panicking on all
/// later cinematics.
#[cfg(feature = "video")]
static FFMPEG_INIT: OnceLock<Result<(), String>> = OnceLock::new();

/// Drain every frame currently decodable from `dec`, resample each to
/// packed f32 stereo via `res`, and append the sample pairs to
/// `audio_frames`. Used both for per-packet decoding and for the final
/// decoder flush after EOF.
#[cfg(feature = "video")]
fn drain_resampled_audio(
    dec: &mut ffmpeg_next::decoder::Audio,
    res: &mut ffmpeg_next::software::resampling::Context,
    audio_frames: &mut Vec<kira::Frame>,
) {
    let mut frame = ffmpeg_next::frame::Audio::empty();
    while dec.receive_frame(&mut frame).is_ok() {
        let mut resampled = ffmpeg_next::frame::Audio::empty();
        if res.run(&frame, &mut resampled).is_err() {
            continue;
        }
        let n_samples = resampled.samples();
        if n_samples == 0 {
            continue;
        }
        let bytes = &resampled.data(0)[..n_samples * 2 * 4];
        let f32_pairs: &[f32] = bytemuck::cast_slice(bytes);
        audio_frames.reserve(n_samples);
        for pair in f32_pairs.as_chunks::<2>().0 {
            audio_frames.push(kira::Frame {
                left: pair[0],
                right: pair[1],
            });
        }
    }
}

/// Play a cutscene video file.
///
/// Decodes Ogg Theora video + Vorbis audio via `ffmpeg-next`, blits
/// frames through wgpu, and plays audio through a temporary kira
/// `AudioManager`. ESC or mouse-click skips playback.
#[cfg(feature = "video")]
pub async fn play_video(
    application_context: &crate::host::ApplicationContext,
    window: &mut GameWindow,
    path: &str,
) -> Result<(), String> {
    if !sound_enabled(application_context) {
        tracing::info!("sound disabled, skipping cinematic {path}");
        return Ok(());
    }
    // ffmpeg requires a native pathname. Shipping browser/Android bundles
    // expose cinematics as VFS bytes, so materialize those bytes for the
    // decoder while retaining the temporary file for the whole playback.
    let mut bundled_video = None;
    let resolved = match resolve_data_path(path) {
        Some(path) => path,
        None => {
            use std::io::Write as _;

            let bytes = robin_engine::sbfile::SbFile::read_all(path)
                .map_err(|status| format!("Video file {path} is unavailable (SBFile {status})"))?;
            let mut temporary = tempfile::Builder::new()
                .prefix("robin-cinematic-")
                .suffix(".ogg")
                .tempfile()
                .map_err(|error| format!("create temporary cinematic for {path}: {error}"))?;
            temporary
                .write_all(&bytes)
                .map_err(|error| format!("write temporary cinematic for {path}: {error}"))?;
            let resolved = temporary.path().to_owned();
            bundled_video = Some(temporary);
            resolved
        }
    };
    tracing::info!("Playing video: {}", resolved.display());

    FFMPEG_INIT
        .get_or_init(|| {
            ffmpeg_next::init().map_err(|e| format!("failed to initialize ffmpeg: {e}"))
        })
        .clone()?;

    // ── Open container ──────────────────────────────────────────────
    let mut ictx = ffmpeg_next::format::input(&resolved)
        .map_err(|e| format!("Failed to open {}: {e}", resolved.display()))?;
    // Keep the bundle-backed temporary alive until every decoder/container
    // handle has finished using its path.
    let _bundled_video = bundled_video;

    // ── Video stream ────────────────────────────────────────────────
    let video_idx = ictx
        .streams()
        .best(ffmpeg_next::media::Type::Video)
        .ok_or("No video stream")?
        .index();
    let video_time_base = ictx.stream(video_idx).unwrap().time_base();
    let video_params = ictx.stream(video_idx).unwrap().parameters();
    let video_ctx = ffmpeg_next::codec::context::Context::from_parameters(video_params)
        .map_err(|e| e.to_string())?;
    let video_dec = video_ctx.decoder().video().map_err(|e| e.to_string())?;
    let vid_w = video_dec.width();
    let vid_h = video_dec.height();
    tracing::info!(
        "Video: {vid_w}x{vid_h}, time_base={}/{}",
        video_time_base.numerator(),
        video_time_base.denominator()
    );

    // ── Audio stream (optional) ─────────────────────────────────────
    let audio_idx = ictx
        .streams()
        .best(ffmpeg_next::media::Type::Audio)
        .map(|s| s.index());
    let mut audio_dec = audio_idx
        .map(|idx| {
            let params = ictx.stream(idx).unwrap().parameters();
            let ctx = ffmpeg_next::codec::context::Context::from_parameters(params)?;
            ctx.decoder().audio()
        })
        .transpose()
        .map_err(|e: ffmpeg_next::Error| e.to_string())?;

    // ── Pixel-format converter (any → RGBA8) ────────────────────────
    let mut scaler = ffmpeg_next::software::scaling::Context::get(
        video_dec.format(),
        vid_w,
        vid_h,
        ffmpeg_next::format::Pixel::RGBA,
        vid_w,
        vid_h,
        ffmpeg_next::software::scaling::Flags::BILINEAR,
    )
    .map_err(|e| format!("Scaler init failed: {e}"))?;

    // ── Audio resampler → f32 stereo @ 44100 Hz (kira's frame layout) ──
    let output_sample_rate: u32 = 44100;
    let mut audio_resampler = audio_dec
        .as_mut()
        .map(|dec| {
            ffmpeg_next::software::resampling::Context::get(
                dec.format(),
                dec.channel_layout(),
                dec.rate(),
                ffmpeg_next::format::Sample::F32(ffmpeg_next::format::sample::Type::Packed),
                ffmpeg_next::ChannelLayout::STEREO,
                output_sample_rate,
            )
        })
        .transpose()
        .map_err(|e: ffmpeg_next::Error| format!("Resampler init failed: {e}"))?;

    // ── First pass: decode audio fully into kira frames ─────────────
    // Cinematics are ≤ 2 minutes; ~2 min × 44.1 kHz × 2 ch × 4 B = 21 MB.
    // Cheap enough to buffer up-front and let kira drive the audio device
    // while we focus on rendering.
    let mut audio_frames: Vec<kira::Frame> = Vec::new();
    if let (Some(dec), Some(res)) = (audio_dec.as_mut(), audio_resampler.as_mut()) {
        // We rewind ictx after this scan so the video-pump pass can walk
        // packets again from the start.
        let aidx = audio_idx.unwrap();
        for (stream, packet) in ictx.packets() {
            if stream.index() == aidx && dec.send_packet(&packet).is_ok() {
                drain_resampled_audio(dec, res, &mut audio_frames);
            }
        }
        // Flush decoder.
        let _ = dec.send_eof();
        drain_resampled_audio(dec, res, &mut audio_frames);
        tracing::info!(
            "Audio decoded: {} frames ({} sec @ {} Hz)",
            audio_frames.len(),
            audio_frames.len() as f32 / output_sample_rate as f32,
            output_sample_rate,
        );
        // Re-open the container so the video pass starts from frame 0
        // (ffmpeg's `seek` API is brittle for variable-rate Ogg; a fresh
        // open is the safest reset).
    }

    // Re-open if we consumed the container scanning audio.
    let mut ictx = ffmpeg_next::format::input(&resolved)
        .map_err(|e| format!("Re-open {} failed: {e}", resolved.display()))?;
    let video_ctx = ffmpeg_next::codec::context::Context::from_parameters(
        ictx.stream(video_idx).unwrap().parameters(),
    )
    .map_err(|e| e.to_string())?;
    let mut video_dec = video_ctx.decoder().video().map_err(|e| e.to_string())?;

    // ── kira: kick off audio playback ───────────────────────────────
    let audio_handle = if !audio_frames.is_empty() {
        let mut manager =
            kira::AudioManager::<kira::DefaultBackend>::new(kira::AudioManagerSettings::default())
                .map_err(|e| format!("kira init: {e}"))?;
        let sound = kira::sound::static_sound::StaticSoundData {
            sample_rate: output_sample_rate,
            frames: Arc::from(audio_frames.into_boxed_slice()),
            settings: kira::sound::static_sound::StaticSoundSettings::default(),
            slice: None,
        };
        let handle = manager.play(sound).map_err(|e| format!("kira play: {e}"))?;
        Some((manager, handle))
    } else {
        None
    };

    // ── wgpu video blit pipeline (self-contained) ──────────────────
    let blit = VideoBlit::new(&window.gpu, window.gpu.surface_format, vid_w, vid_h);

    // ── Main decode / display loop ──────────────────────────────────
    let wall_start = web_time::Instant::now();
    let mut skipped = false;
    'pump: for (stream, packet) in ictx.packets() {
        if poll_skip(window) {
            skipped = true;
            break;
        }
        if stream.index() != video_idx {
            continue;
        }
        if video_dec.send_packet(&packet).is_err() {
            continue;
        }
        let mut frame = ffmpeg_next::frame::Video::empty();
        while video_dec.receive_frame(&mut frame).is_ok() {
            let mut rgba = ffmpeg_next::frame::Video::empty();
            if scaler.run(&frame, &mut rgba).is_err() {
                continue;
            }
            let pts_ms = frame
                .pts()
                .map(|pts| {
                    (pts as f64 * f64::from(video_time_base.numerator())
                        / f64::from(video_time_base.denominator())
                        * 1000.0) as u64
                })
                .unwrap_or(0);
            // Wall-clock sync — kira's audio runs independently on its
            // own thread, so we just pace video against the wall clock.
            let wall_ms = wall_start.elapsed().as_millis() as u64;
            if pts_ms > wall_ms + 2 {
                crate::window::sleep_ms((pts_ms - wall_ms).min(50)).await;
                if poll_skip(window) {
                    skipped = true;
                    break 'pump;
                }
            }
            blit.upload_frame(&window.gpu, rgba.data(0), rgba.stride(0), vid_w, vid_h);
            blit.present(window);
        }
    }

    // ── Cleanup ─────────────────────────────────────────────────────
    drop(audio_handle); // stop audio + drop AudioManager
    tracing::info!("Video playback finished (skipped={skipped})");
    Ok(())
}

#[cfg(feature = "video")]
fn poll_skip(window: &mut GameWindow) -> bool {
    use crate::gfx_types::{GameEvent, Keycode};
    for event in window.poll_events() {
        match event {
            GameEvent::Quit
            | GameEvent::KeyDown {
                keycode: Keycode::Escape,
                ..
            }
            | GameEvent::MouseDown(..) => return true,
            _ => {}
        }
    }
    false
}

/// Self-contained wgpu pipeline that streams a video frame texture to
/// the swapchain. Mirrors the renderer's pass-2 letterbox blit but is
/// independent so cinematics can run before / after the main game
/// renderer is alive.
#[cfg(feature = "video")]
struct VideoBlit {
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    bgl: wgpu::BindGroupLayout,
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    letterbox_buffer: wgpu::Buffer,
    vid_w: u32,
    vid_h: u32,
}

#[cfg(feature = "video")]
const VIDEO_BLIT_WGSL: &str = r#"
struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

struct Letterbox {
    // xy = src offset in NDC, zw = src size in NDC
    rect: vec4<f32>,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_samp: sampler;
@group(0) @binding(2) var<uniform> letterbox: Letterbox;

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    // Fullscreen-triangle quad layout: 6 vertices, 2 triangles.
    var pos = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    var uv = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let p = pos[vid];
    let ndc = letterbox.rect.xy + p * letterbox.rect.zw;
    var out: VsOut;
    out.clip_pos = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = uv[vid];
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(src_tex, src_samp, in.uv);
}
"#;

#[cfg(feature = "video")]
impl VideoBlit {
    fn new(
        gpu: &crate::window::GpuContext,
        target_format: wgpu::TextureFormat,
        vid_w: u32,
        vid_h: u32,
    ) -> Self {
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("video blit"),
                source: wgpu::ShaderSource::Wgsl(VIDEO_BLIT_WGSL.into()),
            });
        let bgl = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("video blit bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("video blit layout"),
                bind_group_layouts: &[Some(&bgl)],
                immediate_size: 0,
            });
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("video blit pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("video blit sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("video frame"),
            size: wgpu::Extent3d {
                width: vid_w,
                height: vid_h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        // Letterbox uniform — written each frame in `present()` from
        // the live swapchain size; placeholder values here.
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("video letterbox"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("video blit bg"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffer.as_entire_binding(),
                },
            ],
        });
        // Stash the buffer alongside the bind group via Self; we'll
        // rewrite it on each frame.

        Self {
            pipeline,
            sampler,
            bgl,
            texture,
            bind_group,
            letterbox_buffer: buffer,
            vid_w,
            vid_h,
        }
    }

    fn upload_frame(
        &self,
        gpu: &crate::window::GpuContext,
        rgba: &[u8],
        stride: usize,
        vid_w: u32,
        vid_h: u32,
    ) {
        // ffmpeg's RGBA stride may include row padding; `write_texture`
        // wants `bytes_per_row` honoured.
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride as u32),
                rows_per_image: Some(vid_h),
            },
            wgpu::Extent3d {
                width: vid_w,
                height: vid_h,
                depth_or_array_layers: 1,
            },
        );
    }

    fn present(&self, window: &mut GameWindow) {
        let frame = match window.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            status => {
                tracing::warn!("video present: get_current_texture: {status:?}");
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let swap_w = frame.texture.width() as f32;
        let swap_h = frame.texture.height() as f32;
        // Letterbox: largest aspect-correct rect fitting in the
        // swapchain. Rect is in NDC: x/y in [-1, 1], w/h in [0, 2].
        let vid_aspect = self.vid_w as f32 / self.vid_h as f32;
        let swap_aspect = swap_w / swap_h;
        let (w_ndc, h_ndc) = if swap_aspect >= vid_aspect {
            (2.0 * vid_aspect / swap_aspect, 2.0)
        } else {
            (2.0, 2.0 * swap_aspect / vid_aspect)
        };
        let x_ndc = -w_ndc * 0.5;
        let y_ndc = -h_ndc * 0.5;
        let letterbox = [x_ndc, y_ndc, w_ndc, h_ndc];
        window
            .gpu
            .queue
            .write_buffer(&self.letterbox_buffer, 0, bytemuck::cast_slice(&letterbox));

        let mut encoder =
            window
                .gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("video present"),
                });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("video blit pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..6, 0..1);
        }
        window.gpu.queue.submit(Some(encoder.finish()));
        window.gpu.queue.present(frame);
        // Silence "unused" warnings — these fields exist to keep the
        // bind-group layout / sampler alive for the pipeline's life.
        let _ = (&self.bgl, &self.sampler);
    }
}
