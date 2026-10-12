//! Capture real entity sprites against an exported map for visual inspection.
use super::*;
use crate::window::GpuContext;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};

#[derive(Serialize, Deserialize)]
struct Capture {
    archive: PathBuf,
    sprite_sheet: PathBuf,
    sprite_rect: [u32; 4],
    sprite_offset: [i16; 2],
    output: PathBuf,
    probes: Vec<Probe>,
}

#[derive(Serialize, Deserialize)]
struct Probe {
    name: String,
    map: [f32; 2],
    height: f32,
    layer: u16,
    sector: u16,
    #[serde(default)]
    flying: bool,
}

pub(crate) fn capture(gpu: GpuContext, configuration: &std::path::Path) {
    let config: Capture = serde_json::from_slice(&std::fs::read(configuration).unwrap()).unwrap();
    assert!(!config.probes.is_empty());
    std::fs::create_dir_all(&config.output).unwrap();
    let files =
        robin_engine::sbfile::SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
    crate::mod_pack::mount_mod_overlay(&files, &config.archive).unwrap();
    let details: serde_json::Value =
        serde_json::from_slice(&files.read_shared("details.json").unwrap()).unwrap();
    let name = details["map"].as_str().unwrap();
    let loaded = robin_engine::level_data::LoadedLevel::hackable_from_json(
        &files
            .read_shared(&format!("Data/Levels/{name}.level.json"))
            .unwrap(),
    )
    .unwrap();
    assert!(
        loaded.proto.animations.is_empty(),
        "Fixture requires static scenery"
    );
    assert!(
        loaded.proto.patches.is_empty(),
        "Fixture requires an unchanging background"
    );
    let background = crate::level_loading_host::pre_decode_background_map_with_files(
        name,
        "Day",
        "Data/Levels",
        None,
        &mut |_| {},
        &files,
    )
    .unwrap()
    .unwrap();
    let mut assets = LevelAssets::new();
    let mut profiles = robin_engine::profiles::ProfileManager::new();
    let mut campaign = robin_engine::campaign::Campaign::new();
    let mission = campaign
        .force_next_mission_by_name(&mut profiles, name, name, true)
        .unwrap();
    campaign.current_mission_idx = Some(mission);
    assets.profile_manager = Arc::new(profiles);
    let mut engine = engine_api::Engine::new(engine_api::EngineArgs {
        campaign,
        level: engine_api::LevelLoadArgs {
            assets: &mut assets,
            level_directory: "",
            progress: &mut |_| {},
            loaded,
            bg_pixel_dims: (f32::from(background.width), f32::from(background.height)),
        },
        ground_mark_sprite: None,
        titbit_row_frame_counts: vec![],
        rng_seed: 0,
        original_rng_replay: None,
        sim_config: engine_api::SimConfig {
            script_enabled: false,
            ..Default::default()
        },
    })
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_str().unwrap();
    let mut profiles = robin_engine::player_profile::PlayerProfileManager::new(root.into());
    let index = profiles.create_profile(
        "Archive rendering".into(),
        robin_engine::player_profile::DifficultyLevel::default(),
    );
    profiles.set_active(index);
    let application = crate::application::ApplicationContext::complete(
        crate::player_profile_store::PlayerProfileStore::for_directory(root),
        engine_api::GlobalOptions::default(),
        profiles,
        crate::key_config_store::KeyConfigStore::new(root.into()),
        Some(Arc::new(
            robin_assets::shipping_datadir::ShippingDatadir::default(),
        )),
    )
    .unwrap();
    let mut host = crate::host::Host::new(application.try_into().unwrap(), 160., 160.).unwrap();
    let (sheet_width, sheet_height, sheet) = robin_assets::custom_sprites::decode_png_rgba_bytes(
        &std::fs::read(&config.sprite_sheet).unwrap(),
        config.sprite_sheet.to_str().unwrap(),
    )
    .unwrap();
    let [x, y, width, height] = config.sprite_rect;
    assert!(x + width <= u32::from(sheet_width) && y + height <= u32::from(sheet_height));
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for row in y..y + height {
        let start = ((row * u32::from(sheet_width) + x) * 4) as usize;
        pixels.extend_from_slice(&sheet[start..start + (width * 4) as usize]);
    }
    let mut holder = robin_assets::frame_holder::FrameHolder::new();
    let bank = holder.append_rgba_sprite(
        width.try_into().unwrap(),
        height.try_into().unwrap(),
        &pixels,
    );
    host.frontend
        .resources
        .install_frame_holder_before_publication(holder);
    let scripts = Arc::new(vec![robin_engine::sprite_script::SpriteScript {
        frame_ids: vec![bank],
        delays: vec![1],
        distances: vec![0],
        offsets: vec![SpriteFrameOffset::new(
            f32::from(config.sprite_offset[0]),
            f32::from(config.sprite_offset[1]),
        )],
        ..Default::default()
    }]);
    let mut renderer = Renderer::offscreen(gpu, 160, 160);
    renderer.upload_background_texture(
        u32::from(background.width),
        u32::from(background.height),
        &background.pixels,
    );
    renderer
        .upload_occlusion_depth(
            background.occlusion_depth.as_ref().unwrap(),
            background.width,
            background.height,
        )
        .unwrap();
    let mut report = Vec::new();
    for probe in &config.probes {
        assert!(
            probe
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        );
        let mut element = engine_element::ElementData::from_initial_posture(if probe.flying {
            Posture::Flying
        } else {
            Posture::Upright
        });
        element.kind = ElementKind::ActorPc;
        element.active = true;
        element.sprite = robin_engine::sprite::Sprite::new(scripts.clone(), Arc::new(Vec::new()));
        element.set_layer(probe.layer);
        element.set_sector(engine_position_interface::SectorHandle::new(probe.sector));
        element.set_position(engine_coordinates::WorldPoint3D {
            x: probe.map[0],
            y: probe.map[1] + probe.height,
            z: probe.height,
        });
        element.sprite.position_iface.settle_current_position();
        let actor = engine.test_add_entity(Entity::Pc(engine_element::ActorPc {
            element,
            actor: Default::default(),
            human: Default::default(),
            pc: engine_element::PcData {
                playable: true,
                life_points: 100,
                ..Default::default()
            },
        }));
        let presentation = host.presentation();
        let draw = presentation.draw();
        let view = engine.presentation_view();
        let mut frame = FramePresentationInputs::prepare(&draw, &view);
        frame.view = MapPoint::new(probe.map[0] - 80., probe.map[1] - 110.);
        frame.zoom = 1.;
        frame.draw_order_ids = vec![actor];
        renderer.begin_gpu_frame_clear();
        renderer.render_background_texture(
            Some(&BBox::from_coords(
                frame.view.x,
                frame.view.y,
                frame.view.x + 160.,
                frame.view.y + 160.,
            )),
            Some(&BBox::from_coords(0., 0., 160., 160.)),
        );
        let before = renderer.try_capture_frame_rgba().unwrap().2;
        renderer.begin_gpu_frame_clear();
        renderer.render_background_texture(
            Some(&BBox::from_coords(
                frame.view.x,
                frame.view.y,
                frame.view.x + 160.,
                frame.view.y + 160.,
            )),
            Some(&BBox::from_coords(0., 0., 160., 160.)),
        );
        render_entities_gpu(
            &draw,
            &frame,
            &view,
            &assets,
            &DevState::default(),
            &mut renderer,
            &mut TitbitRenderer::new(),
        );
        let (_, _, after) = renderer.try_capture_frame_rgba().unwrap();
        let changed = before
            .chunks_exact(4)
            .zip(after.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count();
        let file =
            std::fs::File::create(config.output.join(format!("{}.png", probe.name))).unwrap();
        let mut encoder = png::Encoder::new(file, 160, 160);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&after)
            .unwrap();
        report.push(serde_json::json!({"probe": probe, "changed_pixels": changed}));
    }
    std::fs::write(
        config.output.join("captures.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "scope": "native-entity-render-captures-requiring-visual-review-not-traversal-proof",
            "probes": report,
        }))
        .unwrap(),
    )
    .unwrap();
}
