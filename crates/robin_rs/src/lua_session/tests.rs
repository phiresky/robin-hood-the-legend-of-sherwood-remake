use super::*;
use crate::main_entry::PendingLuaMission;

fn write_test_zip(path: &Path, entries: &[(&str, &[u8])]) {
    use std::io::Write as _;
    let file = fs::File::create(path).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in entries {
        writer.start_file(*name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
}

fn session_with_script(source: &str) -> LuaSession {
    let tempdir = tempfile::tempdir().expect("tempdir");
    fs::write(tempdir.path().join("test_mission.lua"), source).expect("write script");
    let mut state = MissionLuaState::new(tempdir.path()).expect("new Lua state");
    register_natives(&mut state).expect("register natives");
    state.load_script("test_mission").expect("load script");
    let mut package = SpellforgePackage {
        contract_version: SPELLFORGE_CONTRACT_VERSION,
        vm_abi: spellforge_vm_abi().to_owned(),
        script_mode: SpellforgeScriptMode::Replace,
        entrypoint: "test_mission.lua".to_owned(),
        files: BTreeMap::from([("test_mission.lua".to_owned(), source.as_bytes().to_vec())]),
        sha256: [0; 32],
    };
    package.sha256 = compute_package_sha256(&package);
    LuaSession {
        _tempdir: tempdir,
        state,
        mission_basename: "test_mission".to_owned(),
        runtime: Arc::new(SpellforgeRuntime51::new(package).expect("runtime")),
    }
}

fn spellforge_args() -> crate::main_entry::MissionLaunch {
    let mut args = crate::main_entry::MissionLaunch::from(CliArgs {
        rollback_check: false,
        ..CliArgs::default()
    });
    args.pending_lua_mission = Some(PendingLuaMission {
        rhm_basename: "test_mission".to_owned(),
        requires_spellforge: true,
        spellforge_package: None,
    });
    args
}

#[test]
fn deterministic_and_network_modes_allow_spellforge() {
    let mut replay = spellforge_args();
    replay.replay = Some("unused.rhrec.jsonl".to_owned());
    validate_launch_mode(&replay, false).unwrap();

    let mut rollback = spellforge_args();
    rollback.rollback_check = true;
    validate_launch_mode(&rollback, false).unwrap();

    let mut host = spellforge_args();
    host.server = true;
    validate_launch_mode(&host, false).unwrap();

    let mut client = spellforge_args();
    client.connect = Some("an-endpoint-id".to_owned());
    validate_launch_mode(&client, false).unwrap();

    validate_launch_mode(&spellforge_args(), true).unwrap();
}

#[test]
fn nested_library_paths_have_one_native_browser_package_identity() {
    let temporary = tempfile::tempdir().unwrap();
    let archive = temporary.path().join("mission.zip");
    write_test_zip(
        &archive,
        &[
            ("English/Data/Levels/test_mission.rhm", b"rhm"),
            (
                "English/Data/Levels/test_mission.lua",
                b"require('lib.helpers.values')",
            ),
            (
                "English/Data/Levels/lib/helpers/values.lua",
                b"nested_value=17",
            ),
        ],
    );
    let bytes = fs::read(&archive).unwrap();
    let package = build_package_from_archives(
        &bytes,
        "English/Data/Levels/test_mission.rhm",
        "test_mission",
        None,
    )
    .unwrap();
    assert_eq!(
        package.files.get("lib/helpers/values.lua").unwrap(),
        b"nested_value=17"
    );
    assert_eq!(
        package.sha256,
        robin_spellforge::compute_package_sha256(&package)
    );
}

#[test]
fn unsafe_nested_library_archive_path_is_rejected() {
    let temporary = tempfile::tempdir().unwrap();
    let archive = temporary.path().join("mission.zip");
    write_test_zip(
        &archive,
        &[
            ("test_mission.rhm", b"rhm"),
            ("test_mission.lua", b"return 1"),
            ("lib/../escape.lua", b"return 1"),
        ],
    );
    let bytes = fs::read(&archive).unwrap();
    let error = build_package_from_archives(&bytes, "test_mission.rhm", "test_mission", None)
        .expect_err("parent traversal must be rejected");
    assert_eq!(
        error.kind,
        robin_spellforge::SpellforgePackageErrorKind::UnsafePath
    );
}

#[test]
fn normal_single_player_and_vanilla_launches_remain_allowed() {
    let spellforge = spellforge_args();
    validate_launch_mode(&spellforge, false).unwrap();

    let mut vanilla = spellforge;
    vanilla
        .pending_lua_mission
        .as_mut()
        .unwrap()
        .requires_spellforge = false;
    vanilla.rollback_check = true;
    vanilla.replay = Some("unused.rhrec.jsonl".to_owned());
    validate_launch_mode(&vanilla, false).unwrap();
}

#[test]
fn required_spellforge_construction_error_keeps_mission_context() {
    let launch = CustomMissionLaunch {
        slug: "test-mod".to_owned(),
        mod_title: "Test Mod".to_owned(),
        claimed_author: "Test Author".to_owned(),
        version: "1".to_owned(),
        source_url: "https://example.invalid".to_owned(),
        license: "CC0-1.0".to_owned(),
        version_zip: PathBuf::from("definitely-missing-spellforge.zip"),
        installed_source: None,
        version_zip_bytes: None,
        rhm_zip_entry: "test_mission.rhm".to_owned(),
        rhm_basename: "test_mission".to_owned(),
        map_filename: String::new(),
        requires_spellforge: true,
    };
    assert!(matches!(
        LuaSession::start_for_launch(&launch, Path::new("unused-mods")),
        Err(SpellforgeSessionError::Startup { mission, source: LuaSessionError::OpenZip(_, _) })
            if mission == "test_mission"
    ));
}

#[test]
fn event_returns_are_checked_table_driven() {
    let session = session_with_script(
        r#"
        function NoReturn() end
        function IntegerReturn() return 17 end
        function IntegralNumberReturn() return 18 / 1 end
        function BooleanReturn() return true end
        function WideIntegerReturn() return 2147483648 end
        function BadReturn() return {} end
        "#,
    );
    let mut host = ScriptEffects::new();
    let mut entities = robin_engine::entities::Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let simulation = robin_engine::sim_rng::SimulationContext::with_seed(1);
    let mut native_globals = Vec::new();
    let capabilities = robin_engine::natives::NativeSessionCapabilities::new(
        &simulation,
        &mut entities,
        &mut ai_global,
        &mut fast_grid,
        &mut native_globals,
    );
    let mut script_state = ScriptState::default();
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    let valid_cases = [
        ("Missing", 0),
        ("NoReturn", 0),
        ("IntegerReturn", 17),
        ("IntegralNumberReturn", 18),
        ("BooleanReturn", 1),
    ];
    for (event, expected) in valid_cases {
        assert_eq!(
            session
                .run_event(
                    &mut host,
                    &mut script_state,
                    &mut script_domains,
                    &capabilities,
                    event,
                    &[],
                )
                .unwrap(),
            expected
        );
    }

    assert!(matches!(
        session.run_event(
            &mut host,
                    &mut script_state,
                    &mut script_domains,
                    &capabilities,
            "BadReturn",
            &[],
        ),
        Err(LuaSessionError::UnexpectedEventReturn { actual, .. }) if actual == "table"
    ));
    #[cfg(target_pointer_width = "64")]
    assert!(matches!(
        session.run_event(
            &mut host,
            &mut script_state,
            &mut script_domains,
            &capabilities,
            "WideIntegerReturn",
            &[],
        ),
        Err(LuaSessionError::EventIntegerOutOfRange {
            value: 2_147_483_648,
            ..
        })
    ));
}

#[test]
fn event_lua_errors_are_not_replaced_with_zero() {
    let session = session_with_script(
        r#"
        function Fails()
            error("deliberate failure")
        end
        "#,
    );
    let mut host = ScriptEffects::new();
    let mut entities = robin_engine::entities::Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let simulation = robin_engine::sim_rng::SimulationContext::with_seed(1);
    let mut native_globals = Vec::new();
    let capabilities = robin_engine::natives::NativeSessionCapabilities::new(
        &simulation,
        &mut entities,
        &mut ai_global,
        &mut fast_grid,
        &mut native_globals,
    );
    let mut script_state = ScriptState::default();
    let mut script_domains = robin_engine::engine::ScriptDomains::default();

    let err = session
        .run_event(
            &mut host,
            &mut script_state,
            &mut script_domains,
            &capabilities,
            "Fails",
            &[],
        )
        .unwrap_err();
    assert!(matches!(err, LuaSessionError::Event { .. }));
    assert!(err.to_string().contains("deliberate failure"));
}

#[test]
fn required_startup_event_error_aborts_the_startup_pair() {
    let session = session_with_script(
        r#"
        post_initialized = false
        function Initialize()
            error("deliberate startup failure")
        end
        function PostInitialize()
            post_initialized = true
        end
        "#,
    );
    let mut host = ScriptEffects::new();
    let mut entities = robin_engine::entities::Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let mut script_state = ScriptState::default();
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    let bindings = robin_engine::natives::AttachedScriptBindings::default();

    let err = robin_engine::sim_rng::with_seed(7, |sim| {
        let mut native_globals = Vec::new();
        let capabilities = robin_engine::natives::NativeSessionCapabilities::new(
            sim,
            &mut entities,
            &mut ai_global,
            &mut fast_grid,
            &mut native_globals,
        );
        session
            .run_required_startup_events(
                Some((
                    &mut host,
                    &mut script_state,
                    &mut script_domains,
                    &bindings,
                    &capabilities,
                )),
                123,
            )
            .unwrap_err()
    });
    assert!(matches!(
        err,
        SpellforgeSessionError::RequiredEvent {
            event: "Initialize",
            source: LuaSessionError::Event { .. },
            ..
        }
    ));
    let post_initialized: bool = session
        .state
        .lua()
        .globals()
        .get("post_initialized")
        .unwrap();
    assert!(
        !post_initialized,
        "PostInitialize must not run after Initialize fails"
    );
}

#[test]
fn required_startup_rejects_a_missing_script_effects() {
    let session = session_with_script("function Initialize() end");
    assert!(matches!(
        session.run_required_startup_events(None, 0),
        Err(SpellforgeSessionError::MissingScriptEffects {
            event: "Initialize",
            ..
        })
    ));
}

#[test]
fn engine_lua_startup_mutates_the_canonical_campaign_owner() {
    use robin_engine::campaign::{Campaign, CampaignValue};
    use robin_engine::engine::{Engine, LevelAssets};
    use robin_engine::profiles::MissionProfile;
    use robin_engine::scb::{ClassEntry, SCB_VERSION, ScbFile};
    use robin_engine::script_manager::ScriptProgram;

    let session = session_with_script(
        r#"
        function Initialize()
            SetCustomCampaignValue(7, 4242)
        end
        "#,
    );

    let startup = ClassEntry {
        source_file: "lua_campaign_owner_test.scs".into(),
        class_name: "StartUp".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: Vec::new(),
        quads: Vec::new(),
    };
    let program = ScriptProgram::from_scb(ScbFile {
        version: SCB_VERSION,
        classes: vec![startup],
    })
    .expect("prepare empty test bytecode");

    let mut assets = LevelAssets::new();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .missions
        .push(MissionProfile {
            mission_filename: "lua_campaign_owner_test".into(),
            ..MissionProfile::default()
        });
    assets.scripts.mission_programs = std::sync::Arc::new(std::collections::BTreeMap::from([(
        "lua_campaign_owner_test".to_owned(),
        std::sync::Arc::new(program),
    )]));

    let mut engine = Engine::new_for_test_with_simulation(
        800.0,
        600.0,
        Campaign::default(),
        &mut assets,
        0,
        robin_engine::engine::SimConfig::default(),
    )
    .expect("construct engine with the minimal mission script");
    engine.test_with_mission_script_effects_and_rng(&assets, |_simulation, native_parts| {
        session
            .run_required_startup_events(native_parts, 0)
            .expect("Lua startup campaign native succeeds")
    });

    let slot = CampaignValue::custom(7).expect("custom campaign slot 7");
    assert_eq!(
        engine.campaign().values[slot],
        4242,
        "Lua must mutate Engine's canonical campaign through the opaque query capability",
    );
}

#[test]
fn engine_lua_startup_borrows_the_scoped_canonical_ai_global() {
    use robin_engine::campaign::Campaign;
    use robin_engine::engine::{Engine, LevelAssets};
    use robin_engine::profiles::MissionProfile;
    use robin_engine::scb::{ClassEntry, SCB_VERSION, ScbFile};
    use robin_engine::script_manager::ScriptProgram;

    let session = session_with_script(
        r#"
        function Initialize()
            local id = AddRepulsivePoint(GetLocationScript(0), 10.0, 20.0, 0)
            DeleteRepulsivePoint(id)
        end
        "#,
    );

    let startup = ClassEntry {
        source_file: "lua_ai_owner_test.scs".into(),
        class_name: "StartUp".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: Vec::new(),
        quads: Vec::new(),
    };
    let program = ScriptProgram::from_scb(ScbFile {
        version: SCB_VERSION,
        classes: vec![startup],
    })
    .expect("prepare empty test bytecode");

    let mut assets = LevelAssets::new();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .missions
        .push(MissionProfile {
            mission_filename: "lua_ai_owner_test".into(),
            ..MissionProfile::default()
        });
    assets.scripts.mission_programs = std::sync::Arc::new(std::collections::BTreeMap::from([(
        "lua_ai_owner_test".to_owned(),
        std::sync::Arc::new(program),
    )]));
    assets.scripts.location_count = 1;
    assets.scripts.point_count = 1;
    assets.scripts.location_positions = std::sync::Arc::new(vec![(12.0, 34.0)]);
    assets.scripts.location_layers = std::sync::Arc::new(vec![2]);
    assets.scripts.location_sectors = std::sync::Arc::new(vec![44]);

    let mut engine = Engine::new_for_test_with_simulation(
        800.0,
        600.0,
        Campaign::default(),
        &mut assets,
        0,
        robin_engine::engine::SimConfig::default(),
    )
    .expect("construct engine with the minimal mission script");
    engine.test_with_mission_script_effects_and_rng(&assets, |_simulation, native_parts| {
        session
            .run_required_startup_events(native_parts, 0)
            .expect("Lua startup AI natives succeed")
    });

    assert_eq!(engine.ai_global().next_repulsive_point_id, 2);
    assert!(engine.ai_global().repulsive_points.is_empty());
}

#[test]
fn startup_random_draw_uses_the_attached_authoritative_context() {
    let session = session_with_script(
        r#"
        function Initialize()
            startup_roll = math.random(1, 1000000)
        end
        "#,
    );
    let mut host = ScriptEffects::new();
    let mut entities = robin_engine::entities::Entities::new();
    let mut ai_global = robin_engine::ai::AiGlobalState::default();
    let mut fast_grid = robin_engine::fast_find_grid::FastFindGrid::default();
    let mut script_state = ScriptState::default();
    let mut script_domains = robin_engine::engine::ScriptDomains::default();
    let bindings = robin_engine::natives::AttachedScriptBindings::default();
    robin_engine::sim_rng::with_seed(0x5eed, |sim| {
        let mut native_globals = Vec::new();
        let capabilities = robin_engine::natives::NativeSessionCapabilities::new(
            sim,
            &mut entities,
            &mut ai_global,
            &mut fast_grid,
            &mut native_globals,
        );
        session
            .run_required_startup_events(
                Some((
                    &mut host,
                    &mut script_state,
                    &mut script_domains,
                    &bindings,
                    &capabilities,
                )),
                0,
            )
            .unwrap();
    });
    let startup_roll: i64 = session.state.lua().globals().get("startup_roll").unwrap();
    assert!((1..=1_000_000).contains(&startup_roll));
}
