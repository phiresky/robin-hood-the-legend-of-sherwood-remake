//! Optional visual and HTTP adapters, excluded from CPU-only parity.
use super::{
    BBox, BTreeMap, BlendMode, Engine, EntityId, GpuImage, Host, LevelAssets, MapPoint, Path,
    PathBuf, Renderer, RpcError, TextureScaleMode, draw_background, rgb565_to_rgb8,
};

#[cfg(feature = "client")]
pub(super) struct ActiveHttpStep {
    pub(super) request: robin_rs::http_server::PendingStep,
    pub(super) direction: &'static str,
    pub(super) from_frame: u32,
    pub(super) remaining: u32,
    pub(super) requested: u32,
}

#[cfg(feature = "client")]
pub(super) struct VisualReplay {
    pub(super) window: robin_rs::window::GameWindow,
    pub(super) renderer: Renderer,
    pub(super) host: Host,
    pub(super) sprite_images: BTreeMap<u32, (GpuImage, u16, u16)>,
}

#[cfg(feature = "client")]
impl VisualReplay {
    pub(super) fn new(
        mut window: robin_rs::window::GameWindow,
        mut host: Host,
        engine: &Engine,
        background: robin_engine::engine::level_loading::PreDecodedBackground,
    ) -> Self {
        window.set_logical_size(1024, 768);
        let mut renderer = Renderer::new(&window, 1024, 768, TextureScaleMode::Nearest);
        // Parity visualization uses a profile-free scratch host and follows the
        // recorded engine ambiance, independent of player graphics preferences.
        robin_rs::level_loading_host::initialize_sprite_variants_for_ambiance(
            &mut host,
            engine.weather().ambiance,
            engine.sim_config().bypass_fog_sprites_crash,
        );
        robin_rs::level_loading_host::apply_background_map(
            engine,
            &mut host,
            &mut renderer,
            background,
        );
        Self {
            window,
            renderer,
            host,
            sprite_images: BTreeMap::new(),
        }
    }

    /// Draw the parity engine's current state while deliberately ignoring all
    /// live keyboard/mouse commands. The trace remains the only input source.
    pub(super) fn queue_frame(&mut self, engine: &Engine) {
        let focus = engine
            .selected_hero_ids()
            .first()
            .and_then(|id| engine.get_entity(*id))
            .or_else(|| {
                engine
                    .entities_with_ids_iter()
                    .find_map(|(_, entity)| entity.is_human().then_some(entity))
            })
            .map(|entity| entity.element_data().position_map())
            .unwrap_or(MapPoint::ZERO);
        self.host.frontend.viewport.view_position =
            MapPoint::new((focus.x - 512.0).max(0.0), (focus.y - 319.0).max(0.0));
        self.host.frontend.viewport.zoom_factor = 1.0;
        draw_background(&self.host.frontend.viewport, &mut self.renderer);

        let mut entities: Vec<_> = engine.entities_with_ids_iter().collect();
        entities.sort_by(|(_, left), (_, right)| {
            left.sprite_visual_map_position()
                .y
                .total_cmp(&right.sprite_visual_map_position().y)
        });
        for (_, entity) in entities {
            if !entity.element_data().active
                || entity.element_data().hidden_in_building
                || !entity.is_to_be_displayed(true)
            {
                continue;
            }
            let sprite = entity.sprite();
            if sprite.current_width == 0 || sprite.current_height == 0 {
                continue;
            }
            let bank_id = sprite.bank_id_for(sprite.current_row, sprite.current_frame);
            if !self.sprite_images.contains_key(&bank_id) {
                let width = self
                    .host
                    .frontend
                    .resources
                    .frame_holder()
                    .sprite_width(bank_id);
                let height = self
                    .host
                    .frontend
                    .resources
                    .frame_holder()
                    .sprite_height(bank_id);
                if width == 0 || height == 0 {
                    continue;
                }
                let rgba = if let Some(rgba) = self
                    .host
                    .frontend
                    .resources
                    .frame_holder()
                    .rgba_data(bank_id)
                {
                    rgba.to_vec()
                } else {
                    let mut pixels = vec![0_u16; usize::from(width) * usize::from(height)];
                    self.host
                        .frontend
                        .resources
                        .frame_holder()
                        .uncompress_frame(
                            &mut pixels,
                            usize::from(width),
                            bank_id,
                            robin_assets::frame_holder::SpriteVariant::Day,
                            engine.weather().night_color,
                            16,
                        );
                    let mut rgba = Vec::with_capacity(pixels.len() * 4);
                    for pixel in pixels {
                        if pixel == robin_assets::frame_holder::TRANSPARENT_COLOR_16 {
                            rgba.extend_from_slice(&[0, 0, 0, 0]);
                        } else {
                            let (r, g, b) = rgb565_to_rgb8(pixel);
                            rgba.extend_from_slice(&[r, g, b, 255]);
                        }
                    }
                    rgba
                };
                let image = self
                    .renderer
                    .create_rgba_gpu_image(width, height, &rgba, "parity replay sprite")
                    .unwrap_or_else(|| panic!("create parity sprite image for bank {bank_id}"));
                self.sprite_images.insert(bank_id, (image, width, height));
            }
            let (image, width, height) = &self.sprite_images[&bank_id];
            let world = entity.sprite_visual_map_position();
            let offset = sprite.offset(sprite.current_row, sprite.current_frame);
            let sprite_x = (world.x - sprite.center.x).floor() + offset.x;
            let sprite_y = (world.y - sprite.center.y).floor() + offset.y;
            let dst = BBox::from_coords(
                sprite_x - self.host.frontend.viewport.view_position.x,
                sprite_y - self.host.frontend.viewport.view_position.y,
                sprite_x - self.host.frontend.viewport.view_position.x + f32::from(*width),
                sprite_y - self.host.frontend.viewport.view_position.y + f32::from(*height),
            );
            self.renderer
                .render_gpu_image(image, None, Some(&dst), BlendMode::Blend);
        }
    }

    pub(super) fn render(&mut self, engine: &Engine) -> bool {
        let _events = self.window.poll_events();
        if self.window.close_requested {
            return false;
        }

        self.queue_frame(engine);
        self.renderer.present();
        std::thread::sleep(std::time::Duration::from_millis(16));
        true
    }

    pub(super) fn wait_until_closed(&mut self) {
        while !self.window.close_requested {
            let _events = self.window.poll_events();
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
    }
}

#[cfg(feature = "client")]
pub(super) fn frame_zero_screenshot_path(output_dir: &Path, trace_path: &Path) -> PathBuf {
    let relative = trace_path
        .ancestors()
        .find(|ancestor| ancestor.file_name().is_some_and(|name| name == "traces"))
        .and_then(|trace_root| trace_path.strip_prefix(trace_root).ok());
    let source_name = relative.unwrap_or_else(|| {
        trace_path
            .file_name()
            .map(Path::new)
            .expect("parity trace path has no filename")
    });
    let mut flat_name = source_name
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("__");
    for suffix in [".rhrec.jsonl.zst", ".jsonl.zst", ".rhrec.jsonl", ".jsonl"] {
        if let Some(stem) = flat_name.strip_suffix(suffix) {
            flat_name = stem.to_owned();
            break;
        }
    }
    output_dir.join(format!("{flat_name}.png"))
}

#[cfg(feature = "client")]
pub(super) fn drain_headless_http(
    http: &mut robin_rs::http_server::SessionIngress,
    engine: &mut Engine,
    assets: &LevelAssets,
    selected_view_element: &mut Option<EntityId>,
    manual_pause: &mut bool,
    active_step: &mut Option<ActiveHttpStep>,
) -> robin_engine::player_command::FrameCommands {
    let commands = http.drain_headless(engine, assets, selected_view_element);
    for request in http.take_pending_steps() {
        match request.kind {
            robin_rs::http_server::StepKind::Forward { n, .. } => {
                if n == 0 {
                    request.respond_ok(serde_json::json!({
                        "direction": "forward",
                        "from_frame": engine.frame_counter(),
                        "frame": engine.frame_counter(),
                        "advanced": 0,
                        "parity": "matched",
                    }));
                } else if active_step.is_some() {
                    request.respond_err(RpcError::capacity(
                        "another parity replay step is already active",
                    ));
                } else {
                    *active_step = Some(ActiveHttpStep {
                        request,
                        direction: "forward",
                        from_frame: engine.frame_counter(),
                        remaining: n,
                        requested: n,
                    });
                }
            }
            robin_rs::http_server::StepKind::Back { .. } => {
                request.respond_err(RpcError::unavailable_capability(
                    "step-back is unavailable for Original parity traces; restart and go-to-frame",
                ));
            }
            robin_rs::http_server::StepKind::GoToFrame { target, .. } => {
                let current = engine.frame_counter();
                if target < current {
                    request.respond_err(RpcError::unavailable_capability(
                        "backward go-to-frame is unavailable for Original parity traces; restart the runner",
                    ));
                } else if target == current {
                    request.respond_ok(serde_json::json!({
                        "direction": "go-to-frame",
                        "from_frame": current,
                        "frame": current,
                        "advanced": 0,
                        "parity": "matched",
                    }));
                } else if active_step.is_some() {
                    request.respond_err(RpcError::capacity(
                        "another parity replay step is already active",
                    ));
                } else {
                    *active_step = Some(ActiveHttpStep {
                        request,
                        direction: "go-to-frame",
                        from_frame: current,
                        remaining: target - current,
                        requested: target - current,
                    });
                }
            }
            robin_rs::http_server::StepKind::SetPaused { paused } => {
                *manual_pause = paused;
                request.respond_ok(serde_json::json!({
                    "paused": paused,
                    "frame": engine.frame_counter(),
                }));
            }
        }
    }
    commands
}

#[cfg(feature = "client")]
pub(super) fn serve_halted_http(
    http: &mut robin_rs::http_server::SessionIngress,
    engine: &mut Engine,
    assets: &LevelAssets,
    selected_view_element: &mut Option<EntityId>,
) -> ! {
    loop {
        let _ = http.drain_headless(engine, assets, selected_view_element);
        for request in http.take_pending_steps() {
            request.respond_err(RpcError::internal(format!(
                "parity replay is halted at divergent frame {}",
                engine.frame_counter()
            )));
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
