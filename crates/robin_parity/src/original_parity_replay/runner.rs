//! Extracted runner boundary; wire layouts remain in the parent.
use super::*;

pub fn main() {
    let options = parse_options();
    if options.inspect_capabilities {
        // Inspection reads an existing native artifact; never convert or
        // quarantine a source recording as a side effect of inspection.
        let native = requested_native_trace_path(&options.trace_path);
        validate_standalone_native_trace(&native);
        let header = read_binary_trace_header(&native).trace;
        validate_trace_header(&header);
        let footer = read_binary_trace_footer(&native).expect("read native trace extent");
        let capabilities = crate::result::TraceCapabilities::new(
            header.schema,
            footer.version,
            header.initial_npc_transients.is_none(),
        );
        println!(
            "{}",
            serde_json::to_string_pretty(&capabilities).expect("serialize trace capabilities")
        );
        return;
    }
    if options.reblock {
        reblock_native_trace(&options.trace_path, options.reblock_policy);
        return;
    }
    if options.validate_native {
        validate_native_trace(&options.trace_path);
        return;
    }
    if options.convert {
        convert_recording_to_native(&options.trace_path);
        return;
    }
    if options.bench_encodings {
        bench_trace_encodings(&options.trace_path);
        return;
    }

    #[cfg(not(feature = "client"))]
    {
        assert!(
            !options.visual,
            "--visual requires rebuilding original_parity_replay with --features client"
        );
        assert!(
            options.frame_zero_screenshot_dir.is_none(),
            "--frame-zero-screenshot-dir requires rebuilding original_parity_replay with --features client"
        );
        assert!(
            options.http_server.is_none(),
            "--http-server requires rebuilding original_parity_replay with --features client"
        );
        tracing_subscriber::fmt::init();
        std::process::exit(run_replay(options, None));
    }

    #[cfg(feature = "client")]
    {
        if options.frame_zero_screenshot_dir.is_some() {
            let exit = robin_rs::window::run_with_game_visibility(
                "Robin Hood — Original parity frame-zero capture",
                1024,
                768,
                options.visual,
                move |mut window| async move {
                    capture_full_frame_zero_screenshot(options, &mut window).await
                },
            )
            .unwrap_or_else(|error| panic!("start frame-zero parity capture: {error}"));
            std::process::exit(exit);
        }
        tracing_subscriber::fmt::init();
        if options.visual {
            let visible = options.visual;
            let exit = robin_rs::window::run_with_game_visibility(
                "Robin Hood — Original parity replay",
                1024,
                768,
                visible,
                move |window| async move { run_replay(options, Some(window)) },
            )
            .unwrap_or_else(|error| panic!("start visual parity replay: {error}"));
            std::process::exit(exit);
        }
        std::process::exit(run_replay(options, None));
    }
}

#[cfg(feature = "client")]
async fn capture_full_frame_zero_screenshot(
    options: Options,
    window: &mut robin_rs::window::GameWindow,
) -> i32 {
    let invocation_dir =
        std::env::current_dir().expect("resolve invocation directory for frame-zero screenshot");
    let trace_path = canonicalize_trace_identity(&options.trace_path);
    let output_dir = options
        .frame_zero_screenshot_dir
        .expect("frame-zero capture lost its output directory");
    let output_dir = if output_dir.is_absolute() {
        output_dir
    } else {
        invocation_dir.join(output_dir)
    };
    let output_path = frame_zero_screenshot_path(&output_dir, &trace_path);

    let native_path = ensure_native_binary_trace(&trace_path);
    let header = read_binary_trace_header(&native_path).trace;
    validate_trace_header(&header);
    let initial_save = decode_and_validate_initial_save(&header);

    let (_launcher_campaign, profiles, application_context) = robin_rs::main_entry::rust_init()
        .unwrap_or_else(|error| panic!("initialize game: {error}"));
    let campaign = restore_campaign(&header.campaign, &profiles);
    let mut game_args = robin_rs::main_entry::try_parse_cli_from([
        "original_parity_replay",
        "--mission",
        header.mission.as_str(),
        "--proto",
        header.proto_level.as_str(),
        "--no-sound",
        "--http-server=0",
        "--rollback-check=false",
    ])
    .unwrap_or_else(|error| panic!("construct frame-zero game arguments: {error}"));
    game_args.mission_start_map_output = Some(output_path.clone());
    game_args.mission_start_map_frame = 0;
    game_args.mission_start_viewport_capture = true;
    game_args.mission_start_legacy_save = initial_save;
    game_args.preserve_forced_mission_campaign = true;
    game_args.fast_forward = true;

    match robin_rs::main_entry::run_rust_game(
        window,
        campaign,
        profiles,
        application_context,
        &game_args,
    )
    .await
    {
        Ok(_) => {
            eprintln!(
                "full frame-zero parity screenshot written to {}",
                output_path.display()
            );
            0
        }
        Err(error) => {
            eprintln!("frame-zero parity screenshot failed: {error}");
            1
        }
    }
}

#[cfg(feature = "client")]
pub(super) type ClientWindow = robin_rs::window::GameWindow;

#[cfg(not(feature = "client"))]
pub(super) type ClientWindow = ();

pub(super) fn run_replay(options: Options, visual_window: Option<ClientWindow>) -> i32 {
    let replay_started = Instant::now();
    let scan_all = options.scan_all;
    let no_auto_dump = options.no_auto_dump;
    let trace_path = options.trace_path;
    #[cfg(feature = "client")]
    let http_server = options.http_server;
    #[cfg(feature = "client")]
    let mut manual_pause = options.start_paused;
    let trace_path = canonicalize_trace_identity(&trace_path);
    let native_path = ensure_native_binary_trace(&trace_path);
    let mut dump = options.dump.map(|options| {
        let file = File::create(&options.path)
            .unwrap_or_else(|e| panic!("create diagnostic dump {}: {e}", options.path.display()));
        (options, BufWriter::new(file))
    });
    let cached_header = read_binary_trace_header(&native_path);
    // Normally grow the replay RNG one frame at a time so loading the trace
    // does not decode every large frame twice. Zero-prefix loaded saves need
    // future draws during deterministic reconstruction. The environment
    // override keeps an exact full-preload A/B path for diagnosing either
    // strategy without another build.
    let preload_complete_rng_stream = should_preload_complete_rng_stream(
        cached_header.trace.start_state,
        cached_header.rng_prefix.draws.gameplay_draw_count(),
        std::env::var_os("PARITY_PRELOAD_RNG").is_some(),
    );
    let initial_rng_draws = if preload_complete_rng_stream {
        read_all_rng_draws(&native_path)
    } else {
        simulation_rng_draws(&cached_header.rng_prefix.draws)
    };
    let header = cached_header.trace;
    validate_trace_header(&header);
    let footer = read_binary_trace_footer(&native_path)
        .unwrap_or_else(|error| panic!("read result trace extent: {error}"));
    let executable = crate::result::executable_path();
    let mut result = crate::result::ReplayResult {
        result_version: 1,
        trace_path: trace_path.to_string_lossy().into_owned(),
        native_trace_sha256: trace_content_sha256(&native_path),
        executable_path: executable.to_string_lossy().into_owned(),
        executable_sha256: trace_content_sha256(&executable),
        expected_frames: footer.frame_count,
        processed_frames: 0,
        expected_final_frame: footer.final_frame,
        final_frame: header.initial_frame,
        terminator_validated: false,
        divergent_frames: 0,
        first_divergences: Vec::new(),
        outcome: "incomplete".into(),
        capabilities: crate::result::TraceCapabilities::new(
            header.schema,
            footer.version,
            header.initial_npc_transients.is_none(),
        ),
    };
    validate_trace_start(
        header.start_state,
        header.session_index,
        header.initial_frame,
    );
    let initial_save = decode_and_validate_initial_save(&header);

    if let Ok(dir) = std::env::var("ROBINHOOD_DATA_DIR") {
        std::env::set_current_dir(&dir).expect("chdir to ROBINHOOD_DATA_DIR");
    }
    register_language_data_paths_for_tool();

    let mut records = BinaryTraceReader::open(&native_path);
    let stream_header = records.read_header();
    assert_eq!(stream_header.trace.schema, header.schema);
    assert_eq!(stream_header.trace.session_index, header.session_index);
    assert_eq!(stream_header.trace.initial_frame, header.initial_frame);
    assert_eq!(stream_header.trace.mission, header.mission);
    assert_eq!(stream_header.trace.rng_seed, header.rng_seed);
    assert_eq!(
        stream_header.trace.synchronous_pathfinding,
        header.synchronous_pathfinding
    );
    assert!(
        header.synchronous_pathfinding,
        "trace was recorded with asynchronous pathfinding"
    );

    let prefix = stream_header.rng_prefix;
    assert_eq!(prefix.r#type, "rng_prefix");
    assert_eq!(prefix.draws.first_index, 0);
    let prefix_end = prefix.draws.gameplay_draw_count();
    if let Some((options, writer)) = &mut dump {
        write_jsonl_record(
            writer,
            &serde_json::json!({
                "schema": "robin-parity-engine-dump.v1",
                "type": "header",
                "source_trace": trace_path,
                "mission": header.mission,
                "rng_seed": header.rng_seed,
                "frame_range": {
                    "from": options.from_frame,
                    "through": (options.through_frame != u64::MAX)
                        .then_some(options.through_frame),
                },
                "entity_filter": options.entities,
            }),
        );
    }

    assert!(
        initial_rng_draws.len() >= prefix_end,
        "loaded simulation RNG stream is shorter than prefix"
    );
    let rewind_loaded_save_rng =
        header.start_state == TraceStartState::LoadedSave && prefix_end == 0;
    #[cfg(feature = "client")]
    let (mut engine, assets, mut host, background, mission_scb, _menu_text) =
        initialize_engine(&header, initial_rng_draws.clone());
    #[cfg(not(feature = "client"))]
    let (mut engine, assets, mission_scb) =
        initialize_headless_engine(&header, initial_rng_draws.clone());
    let mut loaded_save_host = None;
    let mut legacy_blocked_box_shadows = BTreeMap::new();
    if let Some(initial_save) = initial_save {
        let save = robin_engine::legacy_save::initialized::decode_initialized_v48_save(
            initial_save,
            format!("{}#initial_save", trace_path.display()),
            &engine,
            &assets,
            &mission_scb,
            &robin_engine::legacy_save::body::LegacySaveBodyLimits::default(),
        )
        .unwrap_or_else(|error| panic!("decode current-schema initial_save body: {error}"));
        eprintln!(
            "decoded current-schema Original save through byte {} ({} elements, {} dynamic, {} pending paths, {} failed paths)",
            save.end_offset,
            save.element_envelope.records.len(),
            save.element_envelope
                .records
                .iter()
                .filter(|record| matches!(
                    record.resolution,
                    robin_engine::legacy_save::elements::LegacyElementResolution::ConstructDynamic {
                        ..
                    }
                ))
                .count(),
            save.tail.pathfinder.requests.len(),
            save.failed_path_requests.requests.len(),
        );
        if std::env::var_os("PARITY_DEBUG_STAGE_TIMING").is_some() {
            for (index, request) in save.tail.pathfinder.requests.iter().enumerate() {
                eprintln!("parity stage: saved pending path {index}: {request:?}");
            }
            for (index, request) in save.failed_path_requests.requests.iter().enumerate() {
                eprintln!("parity stage: saved failed path {index}: {request:?}");
            }
        }
        legacy_blocked_box_shadows = initial_legacy_blocked_box_shadows(&save);
        loaded_save_host = Some(
            robin_engine::legacy_save::adopt_engine::adopt_known_linux_v48_replay(
                &mut engine,
                &assets,
                &save,
            )
            .unwrap_or_else(|error| panic!("adopt current-schema initial_save body: {error}")),
        );
        eprintln!("atomically adopted current-schema Original Linux-v48 save");
    }
    let restored_dormant_macros = apply_legacy_interactive_chain_macro_fallback(
        &trace_path,
        &header,
        prefix_end,
        &mut engine,
        &assets,
    );
    if restored_dormant_macros != 0 {
        eprintln!(
            "restored {restored_dormant_macros} dormant waypoint-macro cursors from the authoritative preceding interactive-session terminal snapshot"
        );
    }
    if let Some(transients) = header.initial_npc_transients.as_deref() {
        apply_initial_npc_transients(&mut engine, transients);
        eprintln!(
            "restored {} explicit schema-{TRACE_SCHEMA_VERSION} NPC session-boundary transients",
            transients.len()
        );
    } else if header.start_state == TraceStartState::LoadedSave
        && header.session_index > 1
        && legacy_loaded_save_retains_process_transients(prefix_end)
    {
        let restored = apply_legacy_segment_visibility_fallback(&mut engine);
        eprintln!(
            "warning: legacy schema-{TRACE_SCHEMA_VERSION} segment lacks initial_npc_transients; reconstructed maximal_visibility for {restored} dead/unconscious NPCs"
        );
    } else {
        eprintln!(
            "warning: legacy schema-{TRACE_SCHEMA_VERSION} trace lacks initial_npc_transients; retained authoritative constructor/restored NPC transient state"
        );
    }
    if rewind_loaded_save_rng {
        let setup_draws = engine
            .original_rng_replay_cursor()
            .expect("loaded-save reconstruction lost Original RNG replay");
        engine
            .parity_replay_setup()
            .replace_rng_draws(initial_rng_draws);
        eprintln!("rewound loaded-save RNG after {setup_draws} deterministic construction draws");
    }
    let mut motion_line_parity = MotionLineParity::build(&engine, &header.motion_grid);
    engine
        .parity_replay_setup()
        .use_external_director_completions(true);
    #[cfg(feature = "client")]
    let mut display = HostDisplayState::default();
    #[cfg(feature = "client")]
    let mut selected_view_element = None;
    if let Some(loaded) = &loaded_save_host {
        #[cfg(feature = "client")]
        {
            loaded.apply_display_to(&mut display);
            loaded.apply_display_to(&mut host.frontend.presentation.engine_display);
            selected_view_element = loaded.selected_view_element();
            host.frontend
                .set_selected_view_element(selected_view_element);
        }
        assert!(
            loaded.trajectory_output().clear_preview,
            "Original loaded-save adoption must invalidate trajectory preview"
        );
        let _post_load = loaded.post_load_output();
        // This replay host was constructed afresh, so all
        // explicit Original post-load transient clears already hold.
    }
    #[cfg(feature = "client")]
    let mut visual =
        visual_window.map(|window| VisualReplay::new(window, host, &engine, background));
    #[cfg(not(feature = "client"))]
    assert!(
        visual_window.is_none(),
        "headless parity replay received an impossible client window"
    );
    assert_eq!(
        engine.original_rng_replay_cursor(),
        Some(prefix_end),
        "Rust mission setup consumed a different number of global RNG draws than the original"
    );
    if std::env::var_os("PARITY_DEBUG_RNG_PREFIX").is_some() {
        for (index, site) in engine
            .original_rng_replay_sites(0..prefix_end)
            .expect("original RNG site history unexpectedly disabled")
            .into_iter()
            .enumerate()
        {
            eprintln!("Rust startup RNG {index}: {site:?}");
        }
    }
    let mut entity_map: Option<EntityMap> = None;
    let mut divergent_frames = 0_u64;
    let mut first_by_field = BTreeMap::<String, (u64, String)>::new();
    let mut gameplay_rng_index = prefix_end;
    #[cfg(feature = "client")]
    let mut active_http_step = None;
    let debug_stage_timing = std::env::var_os("PARITY_DEBUG_STAGE_TIMING").is_some();
    let automatic_dump_enabled = dump.is_none() && !scan_all && !no_auto_dump;
    let mut rolling_dump = VecDeque::<RollingDumpFrame>::new();
    let mut legacy_terminal_success_repair_applied = false;
    let mut previous_legacy_presentation_entities = None;
    // TODO: A future native trace version should record explicit ordinary vs
    // nested-refresh provenance (or the messenger's no-mouse timer). Old
    // random-input traces omit both, so retain only the exact consecutive
    // command proof needed to disambiguate their popup singleton.
    let infer_legacy_random_refresh_phase = header.random_input_seed.is_some();
    let mut legacy_refresh_orientation_provenance = LegacyRefreshOrientationProvenance::default();
    let campaign_run_id = replay_campaign_run_id(&trace_path, header.session_index);

    #[cfg(feature = "client")]
    let mut http_transport = robin_rs::http_server::HttpTransport::default();
    #[cfg(feature = "client")]
    if let Some(port) = http_server {
        let replay_service =
            std::sync::Arc::new(robin_rs::replay_service::ReplayService::default());
        http_transport
            .start(port, replay_service.exports(), replay_service.launches())
            .unwrap_or_else(|e| panic!("start parity replay HTTP server: {e}"));
        eprintln!(
            "parity replay HTTP server ready on http://127.0.0.1:{port} (frame {})",
            engine.frame_counter()
        );
    }

    #[cfg(feature = "client")]
    let mut http_ingress = http_transport.attach();

    // Original retains path events across the full boundary between frame
    // writes. PostInitialize, sound callbacks, and resolved input can enqueue
    // movement before the next simulation tick, so this capture deliberately
    // remains active outside the simulation body.
    // Keep both captures open across the complete recorded-frame boundary.
    // In particular, queued sound completions run before the simulation body
    // and may synchronously invoke authoritative AI visibility checks.
    robin_engine::sight_obstacle::begin_parity_visibility_capture();
    robin_engine::pathfinder::begin_parity_path_capture();

    // Correlate original-game RNG event offsets with the Rust
    // site labels observed at the same draw indices while the streams still
    // agree. On a cursor mismatch the accumulated map names the Original
    // callsites of the frame, which identifies the draw Rust skipped or
    // invented even when Rust never reached that site.
    let debug_rng_site_map = std::env::var_os("PARITY_DEBUG_RNG_SITE_MAP").is_some();
    let mut rng_site_map: BTreeMap<u32, BTreeSet<String>> = BTreeMap::new();
    let mut known_arrow_falling_callsites = BTreeSet::new();
    let profile_timing = std::env::var_os("PARITY_PROFILE_TIMING").is_some();
    let mut simulation_time = Duration::ZERO;
    let mut comparison_time = Duration::ZERO;
    let mut rng_diagnostic_time = Duration::ZERO;

    let mut line_index = 0_usize;
    let mut trace_timeline = TraceTimeline::new(header.initial_frame);
    let terminator = loop {
        let mut frame = match records.read_record() {
            BinaryTraceRecord::Frame(frame) => frame,
            end @ BinaryTraceRecord::End { .. } => break end,
        };
        assert_eq!(
            frame.record_type, "frame",
            "invalid parity frame record type"
        );
        let legacy_additive_omissions = header.initial_npc_transients.is_none();
        if legacy_additive_omissions {
            restore_legacy_route_construction_diagnostics(&mut frame.route_construction_events);
        }
        validate_trace_frame_with_legacy_additive_omissions(
            header.schema,
            &frame,
            legacy_additive_omissions,
        );
        if debug_stage_timing {
            eprintln!(
                "parity stage: loaded original frame {} -> {}",
                frame.frame_before, frame.frame_after
            );
        }
        trace_timeline
            .observe(frame.frame_before, frame.frame_after)
            .unwrap_or_else(|error| panic!("invalid parity frame timeline: {error}"));
        line_index += 1;
        #[cfg(feature = "client")]
        let mut http_frame_commands = robin_engine::player_command::FrameCommands::new();
        #[cfg(not(feature = "client"))]
        let http_frame_commands = robin_engine::player_command::FrameCommands::new();
        #[cfg(feature = "client")]
        if http_server.is_some() {
            loop {
                let drained = drain_headless_http(
                    &mut http_ingress,
                    &mut engine,
                    &assets,
                    &mut selected_view_element,
                    &mut manual_pause,
                    &mut active_http_step,
                );
                http_frame_commands.commands.extend(drained.commands);
                if !manual_pause || active_http_step.is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        let rng_start = gameplay_rng_index;
        let rng_end = rng_start + frame.rng_draws.gameplay_draw_count();
        gameplay_rng_index = rng_end;
        if !preload_complete_rng_stream {
            engine
                .parity_replay_setup()
                .append_rng_draws(simulation_rng_draws(&frame.rng_draws));
        }
        let capture_commands = automatic_dump_enabled
            || dump
                .as_ref()
                .is_some_and(|(options, _)| options.includes(frame.frame_after));
        let resolved_commands = capture_commands.then(|| {
            serde_json::to_value(&frame.commands).expect("serialize resolved trace commands")
        });
        assert_eq!(
            engine.original_rng_replay_cursor(),
            Some(rng_start),
            "RNG cursor already diverged before original frame {}",
            frame.frame_before
        );
        if scan_all
            && (frame.frame_before.is_multiple_of(10) || (150..=180).contains(&frame.frame_before))
        {
            eprintln!("scanning original frame {}", frame.frame_before);
        }

        // Establish identity from the untouched mission-start state.  Besides
        // being the strongest isomorphism anchor, this lets startup debugging
        // distinguish load-time differences from first-hourglass mutations.
        let map = entity_map.get_or_insert_with(|| EntityMap::build(&engine, &assets, &frame));
        map.refresh_trace_indices(&frame);
        engine
            .parity_replay_setup()
            .set_impossible_action_done_deadlines(
            frame
                .strike_proposal_events
                .iter()
                .filter(|event| event.phase == "opponent_inputs")
                .map(|event| {
                    (
                        event.actor_creation_order,
                        event.principal_opponent_creation_order.unwrap_or_else(|| {
                            panic!(
                                "schema-{TRACE_SCHEMA_VERSION} frame {} opponent_inputs invocation {} lacks principal_opponent_creation_order",
                                frame.frame_before, event.invocation
                            )
                        }),
                        i16::try_from(event.time_limit.unwrap_or_else(|| {
                            panic!(
                                "schema-{TRACE_SCHEMA_VERSION} frame {} opponent_inputs invocation {} lacks time_limit",
                                frame.frame_before, event.invocation
                            )
                        }))
                        .unwrap_or_else(|_| {
                            panic!(
                                "Original strike deadline does not fit SWORD: {:?}",
                                event.time_limit
                            )
                        }),
                    )
                }),
        );
        if debug_stage_timing {
            eprintln!("parity stage: mapped original frame {}", frame.frame_before);
        }
        if frame.frame_before == 0 && std::env::var_os("PARITY_DEBUG_NPC_ORDER").is_some() {
            let reverse: BTreeMap<_, _> = map
                .entities
                .iter()
                .map(|(original, rust)| (*rust, *original))
                .collect();
            eprintln!(
                "Rust NPC iteration mapped to original IDs: {:?}",
                engine
                    .npc_ids()
                    .into_iter()
                    .map(|id| reverse[&id])
                    .collect::<Vec<_>>()
            );
        }
        let debug_startup =
            frame.frame_before == 0 && std::env::var_os("PARITY_DEBUG_STARTUP").is_some();
        if debug_startup {
            print_startup_actors("before Rust frame 1", &engine, &frame, map);
        }
        let director_completions = frame
            .director_completions
            .drain(..)
            .map(robin_engine::engine::DirectorCompletion::from)
            .collect::<Vec<_>>();
        // The original game records the engine frame before running the host sound
        // manager. Consequently these
        // resolutions belong chronologically before the input commands stored
        // on this following frame record. Consume that sound-only boundary
        // first so a current-frame selection bark cannot jump ahead of an NPC
        // request created by the preceding boundary's SoundIsFinished callback.
        let resolutions = frame
            .resolved_exclamations
            .drain(..)
            .map(|resolved| {
                let _selection_diagnostics = (resolved.selected_variant, resolved.selected_entry);
                robin_engine::sound::ResolvedExclamation {
                    actor_id: map.translate(resolved.actor).index(),
                    identifier: resolved.identifier,
                    exclamation_id: resolved.exclamation_id,
                    duration_frames: resolved.duration_frames,
                }
            })
            .collect();
        let external_facts = robin_engine::engine::ExternalFacts::new(
            director_completions,
            Some(robin_engine::engine::SoundBoundary::replay(resolutions)),
        );
        // Delayed DropAle routes are recorded by Original only when the
        // postponed Seek reaches its Go()/RefreshSeek boundary. Director and
        // sound callbacks precede that boundary and may release the matching
        // sequence in this same frame. Preview only that typed fact prefix on
        // a clone; the authoritative engine still receives one atomic frame
        // admission containing the prefix and every matched route outcome.
        let frame_has_cross_sector_route_outcome = frame
            .route_construction_events
            .iter()
            .any(|event| event.kind == "move" && event.source_sector != event.goal_sector);
        let preview_delayed_drop_ale_fact_prefix = frame_has_cross_sector_route_outcome
            && (!external_facts.director_completions.is_empty()
                || external_facts.sound_boundary.is_some());
        let mut delayed_drop_ale_fact_preview =
            preview_delayed_drop_ale_fact_prefix.then(|| {
                robin_engine::sight_obstacle::with_discarded_parity_visibility_capture(|| {
                    let mut preview = engine.clone();
                    preview
                        .advance_frame(
                            &assets,
                            robin_engine::engine::SimulationFrameInput::no_hourglass()
                                .with_external_facts(external_facts.clone()),
                        )
                        .unwrap_or_else(|error| {
                            panic!(
                                "schema-16 frame {} rejected its external-fact prefix while resolving delayed DropAle routes: {error}",
                                frame.frame_before,
                            )
                        });
                    preview
                })
            });
        let popup_nested_refresh = frame
            .popup_events
            .iter()
            .any(|event| event.stage == "nested_refresh_entry" && event.remove_mouse == Some(true));
        let trace_commands_were_empty = frame.commands.is_empty();
        let ordinary_refresh_eligible = frame
            .commands
            .iter()
            .filter_map(|command| match command {
                TraceCommand::OrientActionAt { actor, action, .. }
                    if engine
                        .parity_replay_setup()
                        .orientation_would_emit_before_hourglass(
                            map.translate(*actor),
                            (*action).into(),
                        ) =>
                {
                    Some((*actor, *action))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let force_single_popup_orientation_late = infer_legacy_random_refresh_phase
            && legacy_refresh_orientation_provenance
                .proves_single_popup_orientation_is_late(&frame.commands, popup_nested_refresh);
        let next_legacy_refresh_orientation_provenance = if infer_legacy_random_refresh_phase {
            legacy_refresh_orientation_provenance.advance(&frame.commands, popup_nested_refresh)
        } else {
            LegacyRefreshOrientationProvenance::None
        };
        let (commands_before_hourglass, commands_after_hourglass) =
            split_refresh_owned_orientations(
                std::mem::take(&mut frame.commands),
                popup_nested_refresh,
                &ordinary_refresh_eligible,
                force_single_popup_orientation_late,
            );
        legacy_refresh_orientation_provenance = next_legacy_refresh_orientation_provenance;
        let mut consumed_drop_ale_route_ordinals = BTreeSet::new();
        let mut consumed_group_move_route_ordinals = BTreeSet::new();
        let mut trace_qa_recording = engine.is_recording_macro();
        let mut commands_before_hourglass_resolved = Vec::new();
        for command in commands_before_hourglass {
            if debug_stage_timing {
                eprintln!(
                    "parity stage: converting command before frame {}: {command:?}",
                    frame.frame_after
                );
            }
            let drop_ale_resolution = resolve_current_drop_ale(
                &command,
                &frame.route_construction_events,
                &mut consumed_drop_ale_route_ordinals,
                map,
                current_drop_ale_same_sector_goal(&command, map, &engine),
                trace_qa_recording,
            );
            let group_move_resolution = resolve_current_group_move_route(
                &command,
                &frame.route_construction_events,
                &mut consumed_group_move_route_ordinals,
                map,
                &assets
                    .navigation
                    .legacy_grid_topology
                    .as_ref()
                    .expect("parity replay requires retained Original fast-grid topology")
                    .sectors,
            );
            advance_trace_qa_recording_state(&mut trace_qa_recording, &command);
            let converted = command.into_player_command(
                map,
                &engine,
                drop_ale_resolution,
                group_move_resolution,
            );
            if let Some(command) = converted {
                commands_before_hourglass_resolved.push(command);
            }
        }
        let mut commands_after_hourglass = commands_after_hourglass
            .into_iter()
            .filter_map(|command| {
                let drop_ale_resolution = resolve_current_drop_ale(
                    &command,
                    &frame.route_construction_events,
                    &mut consumed_drop_ale_route_ordinals,
                    map,
                    current_drop_ale_same_sector_goal(&command, map, &engine),
                    trace_qa_recording,
                );
                let group_move_resolution = resolve_current_group_move_route(
                    &command,
                    &frame.route_construction_events,
                    &mut consumed_group_move_route_ordinals,
                    map,
                    &assets
                        .navigation
                        .legacy_grid_topology
                        .as_ref()
                        .expect("parity replay requires retained Original fast-grid topology")
                        .sectors,
                );
                advance_trace_qa_recording_state(&mut trace_qa_recording, &command);

                command.into_player_command(
                    map,
                    &engine,
                    drop_ale_resolution,
                    group_move_resolution,
                )
            })
            .collect::<Vec<_>>();
        append_legacy_retained_terminal_success_repair(
            &mut commands_before_hourglass_resolved,
            &mut commands_after_hourglass,
            header.sim_config.difficulty.into(),
            campaign_run_id,
            header.schema,
            frame.frame_before,
            frame.frame_after,
            frame.simulation_body_ran,
            frame.game_code,
            &mut legacy_terminal_success_repair_applied,
        );
        let current_legacy_presentation_entities =
            legacy_presentation_entity_states(&frame.elements);
        let has_teleport_star_lifecycle = has_legacy_teleport_star_lifecycle(
            previous_legacy_presentation_entities.as_deref(),
            &current_legacy_presentation_entities,
        );
        previous_legacy_presentation_entities = Some(current_legacy_presentation_entities);
        let legacy_presentation_sprite_rng_draws = legacy_presentation_sprite_rng_burst(
            header.schema,
            trace_commands_were_empty,
            frame.game_code,
            frame.simulation_body_ran,
            engine.parity_replay_setup().retained_scroll_count(),
            has_teleport_star_lifecycle,
            &frame.rng_draws.gameplay_callsite_offsets(),
            &frame.rng_draws.gameplay_values(),
        );
        let delayed_drop_ale_route_engine = match delayed_drop_ale_fact_preview.as_mut() {
            Some(preview) => preview,
            None => &mut engine,
        };
        let recorded_drop_ale_routes = collect_current_delayed_drop_ale_routes(
            &frame.route_construction_events,
            &mut consumed_drop_ale_route_ordinals,
            &consumed_group_move_route_ordinals,
            map,
            delayed_drop_ale_route_engine,
        );
        if debug_stage_timing {
            eprintln!(
                "parity stage: entering Rust frame {} -> {}",
                frame.frame_before, frame.frame_after
            );
        }
        let pending_arrow_draws = engine
            .parity_replay_setup()
            .pending_falling_arrow_refresh_draw_count();
        if let Some(additional_draws) = legacy_additional_arrow_refresh_draws(
            header.schema,
            &frame.rng_draws.gameplay_callsite_offsets(),
            &known_arrow_falling_callsites,
            pending_arrow_draws,
        ) {
            engine
                .parity_replay_setup()
                .replay_legacy_additional_arrow_refreshes(additional_draws);
        }
        print_debug_element("before", &engine, &frame);
        robin_engine::movement_diagnostics::begin_parity_movement_capture();
        let simulation_started = Instant::now();
        let tick_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut pre_commands = http_frame_commands
                .commands
                .into_iter()
                .map(robin_engine::engine::SimCommand::from)
                .collect::<Vec<_>>();
            pre_commands.extend(
                commands_before_hourglass_resolved
                    .into_iter()
                    .map(robin_engine::engine::SimCommand::from),
            );
            let frame_input = robin_engine::engine::SimulationFrameInput::new(pre_commands)
                .with_external_facts(
                    external_facts.with_recorded_drop_ale_routes(recorded_drop_ale_routes),
                )
                .with_post_commands(
                    commands_after_hourglass
                        .into_iter()
                        .map(robin_engine::engine::SimCommand::from)
                        .collect(),
                )
                .with_simulation_body_allowed(frame.simulation_body_ran);
            engine
                // The original game's messenger records raw-mouse depth-2 messages but omits
                // SelectPc's depth-3 restitution. Preserve those independently
                // recorded command boundaries in both admission phases.
                .parity_replay_setup()
                .advance_frame(&assets, frame_input)
                .unwrap_or_else(|error| {
                    panic!("admit original frame {}: {error}", frame.frame_before)
                })
                .events
                .into_side_effects()
        }));
        if profile_timing {
            simulation_time += simulation_started.elapsed();
        }
        let actual_visibility_queries =
            robin_engine::sight_obstacle::take_parity_visibility_capture();
        // Restart immediately so post-frame work and the next frame's
        // director/input/sound prefix are attributed to that next frame,
        // matching the Original recorder's frame envelope.
        robin_engine::sight_obstacle::begin_parity_visibility_capture();
        let actual_movement_steps =
            robin_engine::movement_diagnostics::take_parity_movement_capture();
        let actual_flight_steps = robin_engine::movement_diagnostics::take_parity_flight_capture();
        let actual_move_box_extractions =
            robin_engine::movement_diagnostics::take_parity_move_box_extractions();
        let late_movement_retranslations =
            robin_engine::movement_diagnostics::take_parity_late_movement_retranslations();
        let actual_path_events = robin_engine::pathfinder::take_parity_path_capture();
        // Restart immediately: the post-frame comparison and one-shot
        // PostInitialize below precede the next recorded frame boundary.
        robin_engine::pathfinder::begin_parity_path_capture();
        let tick_effects = tick_result.unwrap_or_else(|payload| {
            eprintln!(
                "Rust simulation panicked while replaying original frame {} -> {}",
                frame.frame_before, frame.frame_after
            );
            std::panic::resume_unwind(payload);
        });
        let rust_rng_after_tick = engine
            .original_rng_replay_cursor()
            .expect("original RNG replay unexpectedly disabled after Rust frame");
        let legacy_presentation_sprite_rng_draws = missing_legacy_presentation_sprite_rng_draws(
            legacy_presentation_sprite_rng_draws,
            rust_rng_after_tick,
            rng_end,
        );
        if let Some(draw_count) = legacy_presentation_sprite_rng_draws {
            // TODO(parity-schema-next): record an explicit lifecycle or
            // presentation-RNG event. New traces must not infer this boundary
            // from build-specific RNG callsites.
            engine
                .parity_replay_setup()
                .consume_legacy_presentation_sprite_rng(draw_count);
        }
        if debug_stage_timing {
            eprintln!("parity stage: completed Rust frame {}", frame.frame_after);
        }
        print_debug_element("after", &engine, &frame);
        map.extend_runtime_entities(&engine, &frame);
        record_arrow_publication_before_compare(&engine, &frame, map);
        if debug_stage_timing {
            eprintln!(
                "parity stage: extended runtime identity through frame {}",
                frame.frame_after
            );
        }
        #[cfg(feature = "client")]
        if let Some(visual) = &mut visual
            && !visual.render(&engine)
        {
            eprintln!(
                "visual parity replay closed by user at frame {}",
                engine.frame_counter()
            );
            return 0;
        }
        let actual_rng_end = engine
            .original_rng_replay_cursor()
            .expect("original RNG replay unexpectedly disabled");
        if let Ok(original_index) = std::env::var("PARITY_DEBUG_SOLDIER").map(|value| {
            value
                .parse::<u32>()
                .expect("PARITY_DEBUG_SOLDIER must be a u32")
        }) && frame.frame_after
            <= std::env::var("PARITY_DEBUG_UNTIL")
                .map(|value| {
                    value
                        .parse::<u64>()
                        .expect("PARITY_DEBUG_UNTIL must be a u64")
                })
                .unwrap_or(10)
        {
            let id = map.translate(TraceEntityId {
                kind: TraceEntityKind::Soldier,
                index: original_index,
            });
            let entity = engine.get_entity(id).expect("debug soldier exists");
            let sprite = &entity.element_data().sprite;
            let actor = entity.actor_data().expect("debug soldier 83 is an actor");
            let ai = entity.ai_controller().expect("debug soldier has AI");
            eprintln!(
                "rust frame {} soldier{} pos={:?} goal={:?} increment={:?} dir={:?}/{:?} command={:?} order={:?} last_action={:?} sprite_row={} sprite_frame={}/{} frame_distance={} sprite_motion={:?} execute_init={} last_order={:?} ai={:?}/{:?} chief={:?} patrol={:?} path={:?} history={:?}",
                frame.frame_after,
                original_index,
                entity.element_data().position_map(),
                sprite.position_iface.map_goal(),
                sprite
                    .position_iface
                    .is_increment_map_computed()
                    .then(|| sprite.position_iface.get_increment_map()),
                sprite.position_iface.get_direction(),
                sprite.position_iface.get_direction_goal(),
                engine.actor_command(id),
                engine.actor_order_type(id),
                sprite.last_action,
                sprite.current_row,
                sprite.current_frame,
                sprite.frame_count,
                sprite.current_frame_distance(),
                sprite.last_motion_state,
                actor.execute_order_initialising,
                actor.last_execute_order_id,
                ai.current_state,
                ai.current_substate,
                ai.patrol_chief,
                ai.patrol,
                ai.patrol_path.as_ref().map(|path| (
                    path.hiking_path_index,
                    path.current_waypoint_index,
                    path.last_waypoint_index,
                    path.forward,
                    path.current_waypoint(&assets.navigation.hiking_paths)
                )),
                ai.patrol_path.as_ref().map(|path| &path.history),
            );
            if frame.frame_after == 1
                && let Some(path) = ai.patrol_path.as_ref()
            {
                eprintln!(
                    "rust soldier{} route {:?}",
                    original_index,
                    assets.navigation.hiking_paths[usize::from(path.hiking_path_index)].waypoints
                );
            }
        }
        if debug_startup {
            print_startup_actors("after Rust frame 1", &engine, &frame, map);
        }

        let comparison_started = Instant::now();
        map.validate_building_sector_mapping(&engine, &frame);
        let mut differences =
            motion_line_parity.apply_changes_and_compare(&engine, &frame.motion_line_changes);
        differences.extend(compare_visibility_queries(
            &frame.visibility_queries,
            &actual_visibility_queries,
        ));
        differences.extend(compare_path_events(
            &frame.path_events,
            &actual_path_events,
            map,
        ));
        differences.extend(compare_frame(
            &engine,
            &assets,
            &frame,
            tick_effects.code as i32,
            map,
            &late_movement_retranslations,
            header.initial_npc_transients.is_none(),
            header.schema <= LAST_TRACE_SCHEMA_WITHOUT_DRAW_VIEW,
            &mut legacy_blocked_box_shadows,
        ));
        if profile_timing {
            comparison_time += comparison_started.elapsed();
        }
        if debug_stage_timing {
            eprintln!(
                "parity stage: compared Rust frame {} ({} differences)",
                frame.frame_after,
                differences.len()
            );
        }
        let rng_diagnostic_started = Instant::now();
        let rust_rng_sites = engine
            .original_rng_replay_sites(rng_start..actual_rng_end)
            .expect("original RNG site history unexpectedly disabled");
        let rust_rng_diagnostics = engine
            .original_rng_replay_diagnostics(rng_start..actual_rng_end)
            .expect("original RNG diagnostics unexpectedly disabled");
        if actual_rng_end == rng_end {
            for (offset, site) in frame
                .rng_draws
                .gameplay_callsite_offsets()
                .into_iter()
                .zip(&rust_rng_sites)
            {
                if *site == robin_engine::sim_rng::RngSite::ArrowFallingFrame {
                    known_arrow_falling_callsites.insert(offset);
                }
            }
        }
        if profile_timing {
            rng_diagnostic_time += rng_diagnostic_started.elapsed();
        }
        if let Some((options, writer)) = &mut dump
            && options.includes(frame.frame_after)
        {
            write_engine_dump_frame(
                writer,
                options,
                &engine,
                map,
                &frame,
                resolved_commands
                    .clone()
                    .expect("included diagnostic frame captured its resolved commands"),
                rng_start,
                rng_end,
                actual_rng_end,
                &rust_rng_sites,
                &rust_rng_diagnostics,
                &actual_path_events,
                &actual_visibility_queries,
                &frame.movement_steps,
                &actual_movement_steps,
                &frame.flight_steps,
                &actual_flight_steps,
                &actual_move_box_extractions,
                &differences,
            );
        }
        if automatic_dump_enabled {
            push_rolling_window(
                &mut rolling_dump,
                RollingDumpFrame {
                    engine: engine.diagnostic_snapshot_without_original_rng_replay(),
                    frame_before: frame.frame_before,
                    frame_after: frame.frame_after,
                    selected_pcs: frame.selected_pcs.clone(),
                    rng_draws: frame.rng_draws.clone(),
                    resolved_commands: resolved_commands
                        .expect("automatic diagnostic frame captured its resolved commands"),
                    original_path_events: frame.path_events.clone(),
                    rust_path_events: actual_path_events.clone(),
                    original_visibility_queries: frame.visibility_queries.clone(),
                    rust_visibility_queries: actual_visibility_queries.clone(),
                    original_movement_steps: frame.movement_steps.clone(),
                    rust_movement_steps: actual_movement_steps.clone(),
                    original_flight_steps: frame.flight_steps.clone(),
                    rust_flight_steps: actual_flight_steps.clone(),
                    rust_move_box_extractions: actual_move_box_extractions.clone(),
                    rng_start,
                    expected_rng_end: rng_end,
                    actual_rng_end,
                    rust_rng_sites: rust_rng_sites.clone(),
                    rust_rng_diagnostics: rust_rng_diagnostics.clone(),
                    differences: differences.clone(),
                },
            );
        }
        // Diagnostic: correlate the original game's per-draw event
        // offsets with the Rust `RngSite` names. Only meaningful while the
        // two streams still agree on the draw count for the frame, which is
        // exactly when the pairing is positional and unambiguous.
        if std::env::var_os("PARITY_DEBUG_RNG_SITE_MAP").is_some() {
            let offsets = frame.rng_draws.gameplay_callsite_offsets();
            eprintln!(
                "RNG_FRAME frame={} start={rng_start} original_offsets={offsets:?} rust_sites={rust_rng_sites:?}",
                frame.frame_before,
            );
            if actual_rng_end == rng_end && offsets.len() == rust_rng_sites.len() {
                for (offset, site) in offsets.iter().zip(rust_rng_sites.iter()) {
                    eprintln!("RNG_SITE_MAP offset={offset} site={site:?}");
                }
            }
        }
        // Preserve the complete divergent frame in --dump-jsonl before
        // stopping on an RNG cursor mismatch. RNG ordering failures are often
        // precisely where the broad engine snapshot is most useful.
        if debug_rng_site_map {
            let offsets = frame.rng_draws.gameplay_callsite_offsets();
            if actual_rng_end == rng_end && offsets.len() == rust_rng_sites.len() {
                for (offset, site) in offsets.iter().copied().zip(rust_rng_sites.iter()) {
                    rng_site_map
                        .entry(offset)
                        .or_default()
                        .insert(format!("{site:?}"));
                }
            } else {
                eprintln!(
                    "original-game event offsets for frame {}: {:?}",
                    frame.frame_before, offsets
                );
                for (index, offset) in offsets.iter().copied().enumerate() {
                    let known = rng_site_map
                        .get(&offset)
                        .map(|sites| sites.iter().cloned().collect::<Vec<_>>().join("|"))
                        .unwrap_or_else(|| "<unseen>".to_string());
                    eprintln!("  original draw {index}: offset {offset} -> {known}");
                }
                for (index, site) in rust_rng_sites.iter().enumerate() {
                    eprintln!("  rust draw {index}: {site:?}");
                }
                for difference in &differences {
                    eprintln!("  state difference: {difference}");
                }
                for (offset, sites) in &rng_site_map {
                    eprintln!(
                        "rng site map: {offset} {}",
                        sites.iter().cloned().collect::<Vec<_>>().join("|")
                    );
                }
            }
        }
        if actual_rng_end != rng_end {
            if automatic_dump_enabled {
                write_automatic_rolling_dump(
                    &rolling_dump,
                    &trace_path,
                    &header,
                    map,
                    frame.frame_after,
                );
            }
            panic!(
                "Rust consumed RNG draws {:?} at sites {rust_rng_sites:?} with script diagnostics {rust_rng_diagnostics:#?} during original frame {}; original ended at draw {rng_end}; Original simulation callsite offsets for the frame: {:?}",
                rng_start..actual_rng_end,
                frame.frame_before,
                frame.rng_draws.gameplay_callsite_offsets(),
            );
        }
        if !differences.is_empty() {
            #[cfg(feature = "client")]
            if let Some(step) = active_http_step.take() {
                step.request.respond_err(RpcError::internal(format!(
                    "parity divergence after frame {}: {} differences",
                    frame.frame_after,
                    differences.len()
                )));
            }
            divergent_frames += 1;
            for difference in &differences {
                first_by_field
                    .entry(difference_field(difference).to_string())
                    .or_insert_with(|| (frame.frame_after, difference.clone()));
            }
            #[cfg(feature = "client")]
            let continue_scanning = scan_all && visual.is_none();
            #[cfg(not(feature = "client"))]
            let continue_scanning = scan_all;
            if continue_scanning {
                // The original game records the authoritative frame immediately after
                // simulation tick, then runs one-shot post-initialization
                // hook after refresh/sound. Apply that boundary only after
                // comparing this frame, before advancing to the next one.
                cross_post_initialize_frame(&mut engine, &assets);
                continue;
            }
            let mut fields = BTreeMap::<&str, usize>::new();
            for difference in &differences {
                let field = difference_field(difference);
                *fields.entry(field).or_default() += 1;
            }
            eprintln!(
                "first parity divergence after frame {} ({} differences; showing up to 40):",
                frame.frame_after,
                differences.len()
            );
            eprintln!("  mismatch counts by logical field: {fields:?}");
            for (field, (_, example)) in &first_by_field {
                eprintln!("  first {field}: {example}");
            }
            for difference in differences.iter().take(40) {
                eprintln!("  {difference}");
            }
            if !frame.path_events.is_empty() || !actual_path_events.is_empty() {
                eprintln!(
                    "  Original path events this frame: {}",
                    serde_json::to_string(&frame.path_events)
                        .expect("serialize Original path-event diagnostics")
                );
                eprintln!(
                    "  Rust path events this frame: {}",
                    serde_json::to_string(&actual_path_events)
                        .expect("serialize Rust path-event diagnostics")
                );
            }
            if !frame.route_construction_events.is_empty() {
                eprintln!(
                    "  Original route-construction events this frame: {}",
                    serde_json::to_string(&frame.route_construction_events)
                        .expect("serialize Original route-construction diagnostics")
                );
            }
            print_current_trace_events("popup events", &frame.popup_events);
            print_current_trace_events("AI forecast events", &frame.ai_forecast_events);
            print_current_trace_events("alert-formation events", &frame.alert_formation_events);
            print_current_trace_events(
                "direct-movement authorization events",
                &frame.goto_authorization_events,
            );
            print_current_trace_events("strike-proposal events", &frame.strike_proposal_events);
            print_current_trace_events(
                "sequence-lifecycle events",
                &frame.sequence_lifecycle_events,
            );
            print_current_trace_events("target lifecycle events", &frame.target_lifecycle_events);
            print_current_trace_actor_diagnostics(&frame.elements);
            if automatic_dump_enabled {
                write_automatic_rolling_dump(
                    &rolling_dump,
                    &trace_path,
                    &header,
                    map,
                    frame.frame_after,
                );
            }
            #[cfg(feature = "client")]
            if http_server.is_some() {
                eprintln!(
                    "parity replay halted at frame {}; HTTP inspection remains available",
                    engine.frame_counter()
                );
                serve_halted_http(
                    &mut http_ingress,
                    &mut engine,
                    &assets,
                    &mut selected_view_element,
                );
            }
            #[cfg(feature = "client")]
            if let Some(visual) = &mut visual {
                eprintln!(
                    "visual parity replay frozen at first divergence; close the window to exit"
                );
                visual.wait_until_closed();
            }
            result.processed_frames = u64::try_from(line_index).expect("frame count exceeds u64");
            result.final_frame = u64::from(engine.frame_counter());
            result.divergent_frames = divergent_frames;
            result.outcome = "divergence".into();
            result.first_divergences = structured_divergences(&first_by_field);
            result.publish();
            return 1;
        }
        // Original captures the frame above before its post-refresh
        // PostInitialize hook. The hook's effects belong to the starting
        // state of the next recorded frame, not the frame just compared.
        cross_post_initialize_frame(&mut engine, &assets);
        #[cfg(feature = "client")]
        if let Some(step) = &mut active_http_step {
            step.remaining -= 1;
            if step.remaining == 0 {
                let step = active_http_step.take().expect("active HTTP step exists");
                step.request.respond_ok(serde_json::json!({
                    "direction": step.direction,
                    "from_frame": step.from_frame,
                    "frame": engine.frame_counter(),
                    "advanced": step.requested,
                    "parity": "matched",
                }));
            }
        }
    };

    if profile_timing {
        eprintln!(
            "parity timing: total={:.3}s simulation={:.3}s comparison={:.3}s rng_diagnostics={:.3}s other={:.3}s",
            replay_started.elapsed().as_secs_f64(),
            simulation_time.as_secs_f64(),
            comparison_time.as_secs_f64(),
            rng_diagnostic_time.as_secs_f64(),
            replay_started
                .elapsed()
                .saturating_sub(simulation_time + comparison_time + rng_diagnostic_time)
                .as_secs_f64(),
        );
    }

    match terminator {
        BinaryTraceRecord::End {
            rng_suffix: Some(_),
            final_frame: Some(final_frame),
            frame_count: Some(frame_count),
        } => {
            records
                .validate_terminator(frame_count, final_frame)
                .unwrap_or_else(|error| {
                    panic!(
                        "native parity trace {} has an invalid terminal record: {error}",
                        native_path.display()
                    )
                });
            assert_eq!(
                frame_count,
                u64::try_from(line_index).expect("parity frame count exceeds u64"),
                "parity terminator frame_count disagrees with the frame stream"
            );
            trace_timeline
                .validate_terminator(frame_count, final_frame)
                .unwrap_or_else(|error| panic!("invalid parity terminator timeline: {error}"));
            assert_eq!(
                u64::from(engine.frame_counter()),
                final_frame,
                "Rust final frame disagrees with the clean parity terminator"
            );
        }
        BinaryTraceRecord::End {
            rng_suffix: None,
            final_frame: None,
            frame_count: None,
        } => panic!("parity trace ended without a clean rng_suffix terminator"),
        BinaryTraceRecord::End { .. } => {
            panic!("native parity trace contains a partially populated terminator")
        }
        BinaryTraceRecord::Frame(_) => unreachable!("replay loop exits only on a terminator"),
    }

    #[cfg(feature = "client")]
    if let Some(step) = active_http_step.take() {
        step.request
            .respond_err(RpcError::unavailable_capability(format!(
                "trace ended at frame {} with {} requested frames still pending",
                engine.frame_counter(),
                step.remaining
            )));
    }
    result.processed_frames = u64::try_from(line_index).expect("frame count exceeds u64");
    result.final_frame = u64::from(engine.frame_counter());
    result.divergent_frames = divergent_frames;
    result.terminator_validated = true;
    result.first_divergences = structured_divergences(&first_by_field);
    result.outcome = if divergent_frames == 0 {
        "exact_eof"
    } else {
        "divergence"
    }
    .into();
    result.publish();
    if divergent_frames == 0 {
        println!("parity trace matched every recorded frame");
        #[cfg(feature = "client")]
        if let Some(visual) = &mut visual {
            eprintln!("visual parity replay finished; close the window to exit");
            visual.wait_until_closed();
        }
        0
    } else {
        println!("logical parity scan: {divergent_frames} divergent frames");
        for (field, (frame, example)) in first_by_field {
            println!("  first {field} divergence after frame {frame}: {example}");
        }
        1
    }
}
