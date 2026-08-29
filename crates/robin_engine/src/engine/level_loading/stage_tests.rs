use super::*;

#[test]
fn initializer_reports_each_ordered_stage_boundary_once() {
    let config = SimConfig {
        script_enabled: false,
        ..Default::default()
    };
    let sim = crate::sim_rng::SimulationContext::with_seed_and_config(7, config);
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let mut staging = LevelLoadStaging::default();
    let mut progress_updates = Vec::new();

    engine
        .initialize_from_mission(
            &sim,
            &mut assets,
            &mut staging,
            "stage-order-test",
            "stage-order-proto",
            crate::level_data::LoadedLevel::empty_for_test(),
            "Data/Levels",
            (0.0, 0.0),
            &mut |progress| progress_updates.push(progress),
        )
        .expect("empty no-script mission should traverse every load stage");

    assert_eq!(progress_updates, vec![1.0; 8]);
}
