//! Measure Engine memory footprint and clone cost with real level data.
//!
//! Lives in the engine crate so the per-field clone measurements can reach
//! every sim-state field through `pub(crate)` visibility without widening
//! the public API.
//!
//! Usage:
//!   ROBINHOOD_DATA_DIR=datadirs/demo_leicester_ecoste \
//!     cargo run --release --example measure_engine -p robin_engine

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use robin_engine::campaign::Campaign;
use robin_engine::element::Entity;
use robin_engine::engine::{Engine, LevelAssets};
use robin_engine::profiles::ProfileManager;
use serde::Serialize;

struct TrackingAlloc;

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for TrackingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        ALLOCATED.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            if new_size > layout.size() {
                ALLOCATED.fetch_add(new_size - layout.size(), Ordering::Relaxed);
            } else {
                ALLOCATED.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        new_ptr
    }
}

#[global_allocator]
static GLOBAL: TrackingAlloc = TrackingAlloc;

fn allocated_bytes() -> usize {
    ALLOCATED.load(Ordering::Relaxed)
}

fn json_len<T: Serialize>(value: &T) -> usize {
    serde_json::to_vec(value).expect("json serialize").len()
}

fn add_component<T: Serialize>(
    totals: &mut BTreeMap<&'static str, usize>,
    name: &'static str,
    value: &T,
) {
    *totals.entry(name).or_default() += json_len(value);
}

fn measure_element_details(
    element: &robin_engine::element::ElementData,
    totals: &mut BTreeMap<&'static str, usize>,
) {
    add_component(totals, "element.sprite", &element.sprite);
    add_component(
        totals,
        "element.sprite.position_iface",
        &element.sprite.position_iface,
    );
    add_component(totals, "element.sprite.scripts", &element.sprite.scripts);
    add_component(
        totals,
        "element.sprite.alternate_scripts",
        &element.sprite.alternate_scripts,
    );
    add_component(
        totals,
        "element.sprite.conversion",
        &element.sprite.conversion,
    );
    add_component(
        totals,
        "element.sprite.alternate_conversion",
        &element.sprite.alternate_conversion,
    );
    add_component(
        totals,
        "element.sprite.profile_names",
        &(
            &element.sprite.frame_profile_name,
            &element.sprite.profile_cache_key,
            &element.sprite.alternate_profile_cache_key,
        ),
    );
    add_component(totals, "element.outline_colors", &element.outline_colors);
    add_component(totals, "element.grid_cell", &element.grid_cell);
}

fn measure_npc_details(
    npc: &robin_engine::element::NpcData,
    totals: &mut BTreeMap<&'static str, usize>,
) {
    add_component(totals, "npc.detectable_lists", &npc.detectable_lists);
    add_component(totals, "npc.ai_brain", &npc.ai_brain);
    add_component(totals, "npc.custom_values", &npc.custom_values);
    add_component(
        totals,
        "npc.view_radius_state",
        &(
            npc.view_radius,
            npc.view_radius_base,
            npc.view_radius_goal,
            npc.view_radius_step,
            npc.view_alpha_start,
            npc.view_longrange_radius_factor,
        ),
    );
    add_component(
        totals,
        "npc.view_angle_state",
        &(
            npc.eye_status,
            npc.half_aperture,
            npc.real_half_aperture,
            npc.view_angle,
            npc.view_angle_step,
            npc.view_transition,
            npc.view_half_angle_range,
            npc.view_direction,
            npc.view_lean_out,
        ),
    );
    add_component(
        totals,
        "npc.view_target_state",
        &(
            npc.drunken_cone_iterators,
            npc.stare_point,
            npc.follow_target,
        ),
    );
}

fn measure_entity_components(
    entity: &Entity,
    totals: &mut BTreeMap<&'static str, usize>,
    detail_totals: &mut BTreeMap<&'static str, usize>,
) {
    match entity {
        Entity::Pc(e) => {
            add_component(totals, "element", &e.element);
            measure_element_details(&e.element, detail_totals);
            add_component(totals, "actor", &e.actor);
            add_component(totals, "human", &e.human);
            add_component(totals, "pc", &e.pc);
        }
        Entity::Soldier(e) => {
            add_component(totals, "element", &e.element);
            measure_element_details(&e.element, detail_totals);
            add_component(totals, "actor", &e.actor);
            add_component(totals, "human", &e.human);
            add_component(totals, "npc", &e.npc);
            measure_npc_details(&e.npc, detail_totals);
            add_component(totals, "soldier", &e.soldier);
        }
        Entity::Civilian(e) => {
            add_component(totals, "element", &e.element);
            measure_element_details(&e.element, detail_totals);
            add_component(totals, "actor", &e.actor);
            add_component(totals, "human", &e.human);
            add_component(totals, "npc", &e.npc);
            measure_npc_details(&e.npc, detail_totals);
            add_component(totals, "civilian", &e.civilian);
        }
        Entity::Fx(e) => {
            add_component(totals, "element", &e.element);
            measure_element_details(&e.element, detail_totals);
            add_component(totals, "fx", &e.fx);
        }
        Entity::Target(e) => {
            add_component(totals, "element", &e.element);
            measure_element_details(&e.element, detail_totals);
            add_component(totals, "fx", &e.fx);
            add_component(totals, "target", &e.target);
        }
        Entity::Bonus(e) => {
            add_component(totals, "element", &e.element);
            measure_element_details(&e.element, detail_totals);
            add_component(totals, "object", &e.object);
        }
        Entity::Scroll(e) => {
            add_component(totals, "element", &e.element);
            measure_element_details(&e.element, detail_totals);
            add_component(totals, "object", &e.object);
            add_component(totals, "scroll_presence", &e.presence);
            add_component(totals, "scroll_script_class", &e.script_class);
        }
        Entity::Projectile(e) => {
            add_component(totals, "element", &e.element);
            measure_element_details(&e.element, detail_totals);
            add_component(totals, "object", &e.object);
            add_component(totals, "projectile", &e.projectile);
        }
        Entity::Net(e) => {
            add_component(totals, "element", &e.element);
            measure_element_details(&e.element, detail_totals);
            add_component(totals, "object", &e.object);
            add_component(totals, "projectile", &e.projectile);
            add_component(totals, "net", &e.net);
        }
    }
}

fn main() {
    tracing_subscriber::fmt::init();

    if let Ok(dir) = std::env::var("ROBINHOOD_DATA_DIR") {
        std::env::set_current_dir(&dir).expect("chdir to ROBINHOOD_DATA_DIR");
        eprintln!("Working directory: {dir}");
    }

    let mut pm = ProfileManager::new();
    let mut cpf = robin_engine::sbfile::SbFile::open(
        "Data/Configuration/profile.cpf",
        robin_engine::sbfile::SB_FILE_READ,
    )
    .expect("open profile.cpf");
    pm.load_all_legacy_cpf(&mut cpf).expect("parse profile.cpf");
    let profiles = std::sync::Arc::new(pm);
    eprintln!(
        "Profiles loaded: {} chars, {} soldiers",
        profiles.characters.len(),
        profiles.soldiers.len()
    );

    let mut campaign = Campaign::new();
    campaign.reset(
        &profiles,
        robin_engine::player_profile::DifficultyLevel::Medium,
    );
    campaign.create_gang_from_pcs(
        "RJMT",
        &profiles,
        robin_engine::player_profile::DifficultyLevel::Medium,
    );
    campaign.add_all_to_mission_team();
    campaign.current_mission_idx = Some(1);

    let mut assets = LevelAssets::new();
    assets.profile_manager = profiles.clone();
    // This engine-only example does not depend on robin_assets and
    // therefore cannot ask FrameHolder for the loaded bank signature.
    // The Leicester ECoste demo RHS files all carry this signature.
    assets.bank_signature = 0x019b_b696;

    let before_load = allocated_bytes();
    eprintln!(
        "\nBefore level load: {:.1} MB allocated",
        before_load as f64 / 1_048_576.0
    );

    // RAII: pre-load the mission before constructing the engine, so
    // `bg_pixel_dims` / mask pipeline have real data.  This example
    // doesn't decode the bitmap (lives in robin_assets), so dims are
    // zero — AI init will log "rejected" patrols but memory measurement
    // (the purpose of this example) is unaffected.
    let loaded = robin_engine::engine::level_loading::load_mission_for_campaign(
        &campaign,
        &profiles,
        "Data/Levels",
        &mut |_| {},
    )
    .expect("load mission");

    let engine = Engine::new(robin_engine::engine::EngineArgs {
        campaign,
        level: robin_engine::engine::LevelLoadArgs {
            assets: &mut assets,
            level_directory: "Data/Levels",
            progress: &mut |_| {},
            loaded,
            bg_pixel_dims: (4096.0, 4096.0),
        },
        ground_mark_sprite: None,
        titbit_row_frame_counts: Vec::new(),
        rng_seed: 0,
        original_rng_replay: None,
        sim_config: robin_engine::engine::SimConfig::default(),
    })
    .expect("load level");

    let after_load = allocated_bytes();
    eprintln!(
        "After level load: {:.1} MB allocated",
        after_load as f64 / 1_048_576.0
    );
    eprintln!(
        "Level data cost: {:.1} MB",
        (after_load - before_load) as f64 / 1_048_576.0
    );

    // Skip the warmup ticks — perform_hourglass needs sprite scripts
    // loaded via robin_assets, which this engine-only example can't
    // see. Measurements below run on the post-load (pre-tick) state.
    let after_warmup = allocated_bytes();
    eprintln!(
        "After level load: {:.1} MB allocated",
        after_warmup as f64 / 1_048_576.0
    );

    let entity_count = engine.entities_iter().count();
    eprintln!("\nEntities: {}", entity_count);
    eprintln!(
        "Entity enum size: {} bytes",
        std::mem::size_of::<robin_engine::element::Entity>()
    );

    eprintln!("\n=== Per-field clone cost (via public accessors) ===");
    macro_rules! measure_clone {
        ($name:expr, $val:expr) => {{
            let before = allocated_bytes();
            let c = $val.clone();
            let after = allocated_bytes();
            std::hint::black_box(&c);
            let cost = after.saturating_sub(before);
            if cost > 1024 {
                eprintln!("  {:30} {:>10.1} KB", $name, cost as f64 / 1024.0);
            }
            let _ = std::hint::black_box(c);
        }};
    }
    measure_clone!("fast_grid", *engine.fast_grid());
    measure_clone!("pathfinder", *engine.pathfinder());
    measure_clone!("ai_global", *engine.ai_global());
    measure_clone!("titbit_manager", *engine.titbit_manager());

    let before_clone = allocated_bytes();
    let clone = engine.clone();
    let after_clone = allocated_bytes();
    let clone_heap = after_clone - before_clone;
    eprintln!("\n=== Clone cost ===");
    eprintln!(
        "Heap per clone: {:.1} KB ({:.2} MB)",
        clone_heap as f64 / 1024.0,
        clone_heap as f64 / 1_048_576.0
    );

    let n = 1000;
    let start = Instant::now();
    for _ in 0..n {
        let c = engine.clone();
        std::hint::black_box(&c);
    }
    let clone_elapsed = start.elapsed();
    let clone_us = clone_elapsed.as_micros() as f64 / n as f64;
    eprintln!(
        "Clone time: {:.1} µs ({} clones in {:.0} ms)",
        clone_us,
        n,
        clone_elapsed.as_millis()
    );

    let json = serde_json::to_string(&engine).expect("serialize");
    let n_ser = 1;
    let start = Instant::now();
    for _ in 0..n_ser {
        let j = serde_json::to_string(&engine).unwrap();
        std::hint::black_box(&j);
    }
    let ser_elapsed = start.elapsed();
    let ser_us = ser_elapsed.as_micros() as f64 / n_ser as f64;

    eprintln!("\n=== Serialization ===");
    eprintln!("JSON size: {:.1} KB", json.len() as f64 / 1024.0);
    eprintln!("Serialize time: {:.1} µs", ser_us);
    if let Ok(serde_json::Value::Object(fields)) = serde_json::to_value(&engine) {
        let mut sizes: Vec<(String, usize)> = fields
            .into_iter()
            .map(|(name, value)| {
                let bytes = serde_json::to_vec(&value).expect("serialize field value");
                (name, bytes.len())
            })
            .collect();
        sizes.sort_by_key(|(_, bytes)| std::cmp::Reverse(*bytes));
        eprintln!("Largest JSON fields:");
        for (name, bytes) in sizes.into_iter().take(16) {
            eprintln!("  {:30} {:>10.1} KB", name, bytes as f64 / 1024.0);
        }
    }

    let mut by_kind: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut largest_entities: Vec<(u32, String, usize)> = Vec::new();
    let mut component_totals: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut detail_totals: BTreeMap<&'static str, usize> = BTreeMap::new();
    for (id, entity) in engine.entities_iter().enumerate() {
        let kind = format!("{:?}", entity.kind());
        let entity_json = json_len(entity);
        let entry = by_kind.entry(kind.clone()).or_default();
        entry.0 += 1;
        entry.1 += entity_json;
        largest_entities.push((id as u32, kind, entity_json));
        measure_entity_components(entity, &mut component_totals, &mut detail_totals);
    }
    largest_entities.sort_by_key(|(_, _, bytes)| std::cmp::Reverse(*bytes));

    eprintln!("Entity JSON by kind:");
    let mut by_kind_vec: Vec<_> = by_kind.into_iter().collect();
    by_kind_vec.sort_by_key(|(_, (_, bytes))| std::cmp::Reverse(*bytes));
    for (kind, (count, json_bytes)) in by_kind_vec {
        eprintln!(
            "  {:20} {:>4} {:>10.1} KB json",
            kind,
            count,
            json_bytes as f64 / 1024.0
        );
    }

    eprintln!("Largest entities by JSON:");
    for (id, kind, json_bytes) in largest_entities.into_iter().take(16) {
        eprintln!(
            "  #{:<4} {:20} {:>10.1} KB json",
            id,
            kind,
            json_bytes as f64 / 1024.0
        );
    }

    eprintln!("Entity component totals:");
    let mut component_vec: Vec<_> = component_totals.into_iter().collect();
    component_vec.sort_by_key(|(_, json_bytes)| std::cmp::Reverse(*json_bytes));
    for (name, json_bytes) in component_vec {
        eprintln!("  {:20} {:>10.1} KB json", name, json_bytes as f64 / 1024.0);
    }

    eprintln!("Entity detail totals:");
    let mut detail_vec: Vec<_> = detail_totals.into_iter().collect();
    detail_vec.sort_by_key(|(_, json_bytes)| std::cmp::Reverse(*json_bytes));
    for (name, json_bytes) in detail_vec {
        eprintln!("  {:32} {:>10.1} KB json", name, json_bytes as f64 / 1024.0);
    }

    eprintln!("\n══════════════════════════════════");
    eprintln!("  Engine snapshot ({} entities)", entity_count);
    eprintln!("  Heap per clone:  {:.1} KB", clone_heap as f64 / 1024.0);
    eprintln!("  Clone time:      {:.1} µs", clone_us);
    eprintln!("  JSON size:       {:.1} KB", json.len() as f64 / 1024.0);
    eprintln!("  Serialize time:  {:.1} µs", ser_us);
    eprintln!("──────────────────────────────────");
    eprintln!("  250 snapshots (10s @ 25fps):");
    eprintln!(
        "    Memory: {:.1} MB",
        250.0 * clone_heap as f64 / 1_048_576.0
    );
    eprintln!(
        "    Budget: {:.1}% of frame ({:.1} µs / 40000 µs)",
        clone_us / 400.0,
        clone_us
    );
    eprintln!("══════════════════════════════════");

    drop(clone);
}
