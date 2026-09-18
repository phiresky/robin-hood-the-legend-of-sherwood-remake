//! Engine fixtures shared with other crates' tests.
//!
//! Compiled only for this crate's own tests or when a dependent enables the
//! `test-support` feature from its `[dev-dependencies]`; production builds
//! never see it. `EngineInner`-level fixtures live in
//! `engine::test_support` and stay crate-private.

use crate::campaign::Campaign;
use crate::engine::{Engine, LevelAssets, SimConfig};

/// Scriptless empty-level engine on an 800x600 screen, with its assets.
pub fn fresh_engine() -> (Engine, LevelAssets) {
    fresh_engine_sized(800.0, 600.0)
}

/// [`fresh_engine`] with an explicit screen size.
pub fn fresh_engine_sized(screen_width: f32, screen_height: f32) -> (Engine, LevelAssets) {
    let mut assets = LevelAssets::new();
    let engine = Engine::new_for_test(
        screen_width,
        screen_height,
        Campaign::default(),
        &mut assets,
    )
    .expect("construct scriptless test engine");
    (engine, assets)
}

/// Scriptless empty-level engine with an explicit mission seed, for replay,
/// rollback and frame-contract tests that compare deterministic runs.
pub fn seeded_engine(seed: u64) -> (Engine, LevelAssets) {
    let mut assets = LevelAssets::new();
    let engine = Engine::new_for_test_with_simulation(
        800.0,
        600.0,
        Campaign::default(),
        &mut assets,
        seed,
        SimConfig {
            // Empty LevelAssets has no mission program to run.
            script_enabled: false,
            ..SimConfig::default()
        },
    )
    .expect("construct seeded test engine");
    (engine, assets)
}
