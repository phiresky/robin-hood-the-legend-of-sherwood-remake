
use super::*;
use crate::engine::SimCommand;

#[test]
fn engine_serde_facade_preserves_exact_wire_shape_and_roundtrip_hash() {
    let (engine, assets) = frame_api_fixture();
    let historical_bytes = serde_json::to_vec(&engine.inner).expect("historical inner codec");
    let bytes = serde_json::to_vec(&engine).expect("authoritative facade codec");
    assert_eq!(bytes, historical_bytes);
    let decoded: Engine = serde_json::from_slice(&bytes).expect("decode facade snapshot");
    assert!(!decoded.bootstrap_open);
    let restored = Engine::adopt_authoritative_snapshot(decoded, &assets)
        .expect("attach decoded snapshot resources");
    assert_eq!(serde_json::to_vec(&restored).unwrap(), bytes);
    assert_eq!(
        crate::replay::state_hash(&restored),
        crate::replay::state_hash(&engine)
    );
}

#[test]
fn bootstrap_authority_is_not_snapshot_state() {
    let (mut engine, assets) = frame_api_fixture();
    assert!(engine.bootstrap_open);
    assert!(!engine.clone().bootstrap_open);
    let decoded = Engine::decode_native_snapshot(&engine.encode_native_snapshot()).unwrap();
    assert!(!decoded.bootstrap_open);
    let restored = Engine::from_persisted_state(engine.capture_persisted_state().unwrap());
    assert!(!restored.bootstrap_open);
    // Adoption must also close authority when handed a freshly constructed
    // engine directly, rather than relying on the decoder to have done so.
    let (fresh, _) = frame_api_fixture();
    let adopted = Engine::adopt_authoritative_snapshot(fresh, &assets).unwrap();
    assert!(!adopted.bootstrap_open);
    let (fresh, _) = frame_api_fixture();
    let restored = Engine::restore_from_snapshot(
        &mut super::super::HostDisplayState::default(),
        fresh,
        &assets,
    )
    .unwrap();
    assert!(!restored.bootstrap_open);
    let bytes = serde_json::to_vec(&engine).unwrap();
    let decoded: Engine = serde_json::from_slice(&bytes).unwrap();
    assert!(!decoded.bootstrap_open);
    let hash = crate::replay::state_hash(&engine);
    assert_eq!(hash, crate::replay::state_hash(&engine.inner));
    engine.finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
    assert_eq!(crate::replay::state_hash(&engine), hash);
    assert_eq!(serde_json::to_vec(&engine).unwrap(), bytes);
}

#[test]
fn bootstrap_allows_pre_frame_zero_setup_admission() {
    let (mut engine, assets) = frame_api_fixture();
    engine
        .advance_frame(&assets, SimulationFrameInput::no_hourglass())
        .unwrap();
    engine.finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
    assert!(!engine.bootstrap_open);
}

#[test]
fn debrief_completion_preserves_clock_snapshot_and_hash() {
    let (mut engine, _) = frame_api_fixture();
    engine
        .inner
        .mission_domain
        .campaign
        .set_value(crate::campaign::CampaignValue::MissionLength, 23);
    let bytes = serde_json::to_vec(&engine).unwrap();
    let hash = crate::replay::state_hash(&engine);
    engine.finish_mission_bootstrap(MissionBootstrapCompletion::DebriefOnly);
    assert!(!engine.bootstrap_open);
    assert_eq!(
        engine.campaign().values[crate::campaign::CampaignValue::MissionLength],
        23
    );
    assert_eq!(serde_json::to_vec(&engine).unwrap(), bytes);
    assert_eq!(crate::replay::state_hash(&engine), hash);
    for completion in [
        MissionBootstrapCompletion::StartClock,
        MissionBootstrapCompletion::DebriefOnly,
    ] {
        let json = serde_json::to_string(&completion).unwrap();
        assert_eq!(
            serde_json::from_str::<MissionBootstrapCompletion>(&json).unwrap(),
            completion
        );
    }
}

#[test]
#[should_panic(expected = "mission bootstrap authority is closed")]
fn debrief_completion_cannot_later_start_clock() {
    let (mut engine, _) = frame_api_fixture();
    engine.finish_mission_bootstrap(MissionBootstrapCompletion::DebriefOnly);
    engine.finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
}

#[test]
#[should_panic(expected = "mission bootstrap authority is closed")]
fn bootstrap_cannot_be_finished_twice() {
    let (mut engine, _) = frame_api_fixture();
    engine.finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
    engine.finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
}

#[test]
#[should_panic(expected = "mission bootstrap authority is closed")]
fn bootstrap_cannot_be_reopened_by_clone() {
    let (engine, _) = frame_api_fixture();
    engine
        .clone()
        .finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
}

#[test]
#[should_panic(expected = "mission bootstrap authority is closed")]
fn bootstrap_cannot_be_reopened_by_deserialization() {
    let (engine, _) = frame_api_fixture();
    let mut decoded: Engine =
        serde_json::from_value(serde_json::to_value(engine).unwrap()).unwrap();
    decoded.finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
}

#[test]
#[should_panic(expected = "mission bootstrap authority is closed")]
fn bootstrap_cannot_be_reopened_by_snapshot_adoption() {
    let (engine, assets) = frame_api_fixture();
    Engine::adopt_authoritative_snapshot(engine, &assets)
        .unwrap()
        .finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
}

#[test]
#[should_panic(expected = "mission bootstrap authority is closed")]
fn first_hourglass_closes_bootstrap_even_without_explicit_finish() {
    let (mut engine, assets) = frame_api_fixture();
    engine
        .advance_frame(&assets, SimulationFrameInput::new(Vec::new()))
        .unwrap();
    engine.finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
}

#[test]
fn post_initialize_without_vm_records_the_stage_for_replay() {
    let (mut live, assets) = frame_api_fixture();
    assert!(live.inner.scripts.mission.is_none());
    live.inner.control.sim_config.script_enabled = true;
    let mut replay = live.clone();
    let output = live
        .advance_frame(
            &assets,
            SimulationFrameInput::no_hourglass().with_post_initialize(true),
        )
        .unwrap();
    let recorded_stage = output.post_initialize_events.is_some();
    assert!(
        recorded_stage,
        "the latch mutation must be recorded even without VM effects"
    );
    assert_eq!(live.parity_game_ui_state()["post_initialized"], true);
    let replay_output = replay
        .advance_frame(
            &assets,
            SimulationFrameInput::no_hourglass().with_post_initialize(recorded_stage),
        )
        .unwrap();
    assert_eq!(replay_output.state_hash, output.state_hash);
    assert_eq!(replay.parity_game_ui_state(), live.parity_game_ui_state());
    assert!(
        live.advance_frame(
            &assets,
            SimulationFrameInput::no_hourglass().with_post_initialize(true)
        )
        .unwrap()
        .post_initialize_events
        .is_none()
    );
}

#[test]
fn bootstrap_accepts_an_imported_nonzero_initial_frame() {
    let (mut engine, _) = frame_api_fixture();
    let mut imported = engine.inner.clone_authoritative_state();
    imported.control.frame_counter = 98_765;
    imported
        .mission_domain
        .campaign
        .set_value(crate::campaign::CampaignValue::MissionLength, 23);
    engine.install_legacy_adoption_inner(imported);
    engine.finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
    assert_eq!(engine.frame_counter(), 98_765);
    assert_eq!(
        engine.campaign().values[crate::campaign::CampaignValue::MissionLength],
        0
    );
    assert!(!engine.bootstrap_open);
}

#[test]
#[should_panic(expected = "mission bootstrap authority is closed")]
fn legacy_adoption_cannot_reopen_finished_bootstrap() {
    let (mut engine, _) = frame_api_fixture();
    engine.finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
    let (fresh, _) = frame_api_fixture();
    engine.install_legacy_adoption_inner(fresh.inner);
    engine.finish_mission_bootstrap(MissionBootstrapCompletion::StartClock);
}

fn frame_api_fixture() -> (Engine, LevelAssets) {
    let mut assets = LevelAssets::new();
    let sim_config = SimConfig {
        script_enabled: false,
        ignore_default_loose: true,
        ..Default::default()
    };
    let engine = Engine::new_for_test_with_simulation(
        1024.0,
        768.0,
        Campaign::default(),
        &mut assets,
        0xF4A6_E001,
        sim_config,
    )
    .expect("construct frame API fixture");
    (engine, assets)
}

#[test]
fn tick_admission_crosses_pending_arrow_refresh_before_hourglass() {
    use crate::coordinates::WorldPoint3D;
    use crate::element::{
        ElementData, ElementKind, ElementProjectile, Entity, ObjectData, ObjectType, ProjectileData,
    };

    let (mut engine, assets) = frame_api_fixture();
    let mut element = {
        let mut initial_element = ElementData::default();
        initial_element.kind = ElementKind::ObjectProjectile;
        initial_element.active = true;
        initial_element
    };
    element
        .sprite
        .position_iface
        .set_old_position(WorldPoint3D::new(-1.0, 0.0, 0.0));
    let arrow = engine
        .inner
        .add_entity(Entity::Projectile(ElementProjectile {
            element,
            object: ObjectData {
                object_type: ObjectType::Arrow,
                ..Default::default()
            },
            projectile: ProjectileData {
                flying: true,
                falling: true,
                falling_direction: 6,
                ..Default::default()
            },
        }));
    engine.inner.control.arrow_refresh_pending = true;

    engine
        .advance_frame(&assets, SimulationFrameInput::default())
        .expect("admit simulation tick");

    let Entity::Projectile(arrow) = engine.inner.get_entity(arrow).unwrap() else {
        unreachable!()
    };
    // The admitted refresh was crossed (the sprite assertions below see
    // it); this tick then schedules the following presentation refresh.
    assert!(engine.inner.control.arrow_refresh_pending);
    assert_eq!(arrow.element.sprite.current_row, 6);
    assert!((3..=5).contains(&arrow.element.sprite.current_frame));
    assert_eq!(arrow.projectile.falling_direction, 4);
}

#[test]
fn no_hourglass_admission_leaves_arrow_refresh_pending() {
    let (mut engine, assets) = frame_api_fixture();
    engine.inner.control.arrow_refresh_pending = true;

    engine
        .advance_frame(&assets, SimulationFrameInput::no_hourglass())
        .expect("admit no-hourglass boundary");

    assert!(engine.inner.control.arrow_refresh_pending);
}

fn typed_sentinel_snapshot_fixture() -> (Engine, EntityId) {
    let mut inner = EngineInner::new();
    let mut ai = crate::ai_enemy::EnemyAi::new(0);
    ai.base.primary_target = Some(crate::ai::AiEntityHandle::new(0));
    ai.base.seek_position.sector = Some(crate::position_interface::SectorHandle::from_number(
        crate::sector::SectorNumber::new(-1),
    ));
    ai.base.initial_position.sector = Some(
        crate::position_interface::SectorHandle::new(23)
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(0).unwrap()),
    );
    ai.base.detached_patrol_path_status = crate::ai::DetachedPatrolPathStatus {
        hiking_path_index: crate::ai::PathId::new(3),
        current_waypoint_index: 5,
        last_waypoint_index: 7,
        forward: false,
        history: vec![crate::ai::PathHistoryEntry {
            position: crate::ai::Position::default(),
            direction: 9,
            distance: 11,
        }],
    };
    let id = inner.add_entity(crate::element::Entity::Soldier(
        crate::element::ActorSoldier {
            element: {
                let mut initial_element = crate::element::ElementData::default();
                initial_element.kind = crate::element::ElementKind::ActorSoldier;
                initial_element
            },
            actor: Default::default(),
            human: Default::default(),
            npc: {
                crate::element::NpcData {
                    ai: crate::element::AiActorData {
                        ai_brain: crate::element::AiBrain::Enemy(Box::new(ai)),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            },
            soldier: Default::default(),
        },
    ));
    assert_eq!(id.index(), 0, "fixture must occupy live arena slot zero");
    (
        Engine {
            inner,
            bootstrap_open: false,
        },
        id,
    )
}

fn assert_typed_sentinel_snapshot(engine: &Engine, id: EntityId) {
    let ai = engine
        .inner
        .get_entity(id)
        .and_then(crate::element::Entity::enemy_ai)
        .expect("typed sentinel fixture retains EnemyAi");
    assert_eq!(
        ai.base.primary_target,
        Some(crate::ai::AiEntityHandle::new(0))
    );
    let signed_sector = ai.base.seek_position.sector.unwrap();
    assert_eq!(signed_sector.number().get(), -1);
    assert_eq!(signed_sector.arena_index(), None);
    let exact_sector = ai.base.initial_position.sector.unwrap();
    assert_eq!(exact_sector.number().get(), 23);
    assert_eq!(
        exact_sector.arena_index(),
        crate::fast_find_grid::SectorIndex::new(0)
    );
    let detached = &ai.base.detached_patrol_path_status;
    assert_eq!(detached.hiking_path_index, crate::ai::PathId::new(3));
    assert_eq!(detached.current_waypoint_index, 5);
    assert_eq!(detached.last_waypoint_index, 7);
    assert!(!detached.forward);
    assert_eq!(detached.history.len(), 1);
    assert_eq!(detached.history[0].direction, 9);
    assert_eq!(detached.history[0].distance, 11);
}

fn sherwood_trading_frame_fixture() -> (Engine, LevelAssets) {
    use crate::element::{ElementBonus, ElementData, ElementKind, Entity, ObjectData};
    use crate::mission::Mission;
    use crate::profiles::{Action, MissionLocation, MissionProfile, ProfileManager};

    let mut profiles = ProfileManager::default();
    profiles.missions.push(MissionProfile {
        location: MissionLocation::Sherwood,
        ..MissionProfile::default()
    });
    let mut assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..LevelAssets::default()
    };
    let mut campaign = Campaign::default();
    campaign.missions.push(Mission {
        profile_idx: Some(0),
        ..Mission::default()
    });
    campaign.current_mission_idx = Some(0);
    let mut engine = Engine::new_for_test_with_simulation(
        1024.0,
        768.0,
        campaign,
        &mut assets,
        0x7A4D_E001,
        SimConfig {
            sherwood_trading: true,
            script_enabled: false,
            ignore_default_loose: true,
            ..SimConfig::default()
        },
    )
    .expect("construct Sherwood trading frame fixture");
    engine
        .inner
        .world
        .entities
        .push(Some(Entity::Bonus(ElementBonus {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ObjectBonus;
                initial_element.active = true;
                initial_element
            },
            object: ObjectData {
                associated_action: Action::Bow,
                quantity: 1,
                ..ObjectData::default()
            },
        })));
    (engine, assets)
}

#[test]
fn paused_post_boundary_trade_delivers_its_receipt_in_the_same_transaction() {
    use crate::player_command::PlayerCommand;
    use crate::sector_production::Type;
    use crate::trading::{TradeOutcome, TradeQuantity};

    let (mut engine, assets) = sherwood_trading_frame_fixture();
    let output = engine
        .advance_frame(
            &assets,
            SimulationFrameInput::no_hourglass().with_post_commands(vec![SimCommand::host(
                PlayerCommand::CampaignSellProductionItem {
                    request_id: 77,
                    prod_type: Type::MakeArrow,
                    quantity: TradeQuantity::One,
                },
            )]),
        )
        .expect("admit paused modal trade");

    assert!(!output.hourglass_ran);
    assert!(output.events.side_effects().trade_receipts.is_empty());
    let receipts = &output.post_boundary_events.side_effects().trade_receipts;
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].request_id, 77);
    assert!(matches!(
        receipts[0].outcome,
        TradeOutcome::Sold {
            units: 1,
            remaining_stock: 0,
            ..
        }
    ));
}

#[test]
fn item_rules_apply_on_the_command_frame_and_survive_native_snapshot() {
    let (mut engine, assets) = frame_api_fixture();
    let rules = crate::gameplay_config::ItemGameplayConfig {
        apple_combat_interrupt: true,
        wasp_reliable_acquisition: false,
        stone_ground_distraction: true,
        stone_longer_range: false,
        net_selective_immunity: true,
        ale_reliable_distraction: false,
    };
    engine
        .advance_frame(
            &assets,
            SimulationFrameInput::new(vec![
                PlayerCommand::SetItemGameplayConfig { config: rules }.into(),
            ])
            .with_hourglass(false),
        )
        .expect("item rules command frame");
    assert_eq!(engine.sim_config().item_gameplay, rules);

    let restored = Engine::decode_native_snapshot(&engine.encode_native_snapshot())
        .expect("decode item rules snapshot");
    assert_eq!(restored.sim_config().item_gameplay, rules);
}

#[test]
fn ranked_policy_rejects_timed_and_ambience_noop_commands_in_both_phases() {
    use crate::engine::{RankedSimulationConfigField, SimulationCommandPhase};

    for (command, field) in [
        (
            PlayerCommand::SetTimedMissionsEnabled { enabled: true },
            RankedSimulationConfigField::EnableTimedMissions,
        ),
        (
            PlayerCommand::SetDynamicAmbienceEnabled { enabled: true },
            RankedSimulationConfigField::EnableDynamicAmbience,
        ),
    ] {
        for phase in [
            SimulationCommandPhase::PreHourglass,
            SimulationCommandPhase::PostHourglass,
        ] {
            let (mut engine, assets) = frame_api_fixture();
            let policy = crate::engine::RankedSimulationPolicy::standard_medium();
            let config = policy.expected_config();
            engine.inner.control.sim_config = config;
            engine.inner.control.mission_start_sim_config = config;
            engine
                .inner
                .control
                .install_ranked_simulation_policy(policy);
            let ranked_noop = SimCommand::host(command.clone());
            let input = match phase {
                SimulationCommandPhase::PreHourglass => {
                    SimulationFrameInput::new(vec![ranked_noop]).with_hourglass(false)
                }
                SimulationCommandPhase::PostHourglass => {
                    SimulationFrameInput::no_hourglass().with_post_commands(vec![ranked_noop])
                }
            };

            let error = engine
                .advance_frame(&assets, input)
                .expect_err("ranked no-op setting command must be rejected");
            assert_eq!(
                error,
                FrameAdvanceError::RankedSimulationSettingCommandRejected {
                    phase,
                    index: 0,
                    field,
                }
            );
            assert_eq!(engine.sim_config(), config);
        }
    }
}

#[test]
fn installing_original_parity_replay_forces_classic_item_rules() {
    let (mut engine, assets) = frame_api_fixture();
    engine.inner.control.sim_config.item_gameplay =
        crate::gameplay_config::ItemGameplayConfig::default();
    engine.inner.control.sim_config.noise_distraction_feedback = true;
    engine
        .parity_replay_setup()
        .replace_rng_draws(vec![0x1234_5678]);

    assert_eq!(
        engine.sim_config().item_gameplay,
        crate::gameplay_config::ItemGameplayConfig::classic()
    );
    assert!(!engine.sim_config().noise_distraction_feedback);

    engine
        .advance_frame(
            &assets,
            SimulationFrameInput::new(vec![
                PlayerCommand::SetItemGameplayConfig {
                    config: crate::gameplay_config::ItemGameplayConfig::default(),
                }
                .into(),
                PlayerCommand::SetNoiseDistractionFeedback { enabled: true }.into(),
            ])
            .with_hourglass(false),
        )
        .expect("Original-parity settings command frame");
    assert_eq!(
        engine.sim_config().item_gameplay,
        crate::gameplay_config::ItemGameplayConfig::classic()
    );
    assert!(!engine.sim_config().noise_distraction_feedback);
}

#[test]
fn native_snapshot_decodes_through_the_engine_facade() {
    let (mut engine, _) = frame_api_fixture();
    engine.inner.feedback.cutscene_camera.view_position =
        crate::coordinates::MapPoint::new(73.0, 91.0);
    engine.inner.feedback.cutscene_camera.zoom_factor = 2.0;
    let expected_hash = crate::replay::state_hash(&engine);
    let bytes = engine.encode_native_snapshot();

    let decoded = Engine::decode_native_snapshot(&bytes)
        .expect("decode the native Engine wire layout through the facade");

    assert_eq!(crate::replay::state_hash(&decoded), expected_hash);
    assert_eq!(
        decoded.inner.feedback.cutscene_camera.view_position,
        engine.inner.feedback.cutscene_camera.view_position
    );
    assert_eq!(decoded.inner.feedback.cutscene_camera.zoom_factor, 2.0);
}

#[test]
fn rollback_native_snapshot_round_trips_typed_slot_zero_and_spatial_provenance() {
    std::thread::Builder::new()
        .name("typed-sentinel-rollback-snapshot".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(rollback_native_snapshot_round_trips_typed_slot_zero_and_spatial_provenance_inner)
        .expect("spawn large-stack rollback snapshot test")
        .join()
        .expect("rollback snapshot test panicked");
}

fn rollback_native_snapshot_round_trips_typed_slot_zero_and_spatial_provenance_inner() {
    let (engine, id) = typed_sentinel_snapshot_fixture();
    let bytes = engine.encode_native_snapshot();

    let decoded = Engine::decode_native_snapshot(&bytes).expect("decode rollback snapshot");

    assert_typed_sentinel_snapshot(&decoded, id);
    let present_hash = crate::replay::state_hash(&decoded);
    let mut absent = decoded;
    absent
        .inner
        .get_entity_mut(id)
        .and_then(crate::element::Entity::enemy_ai_mut)
        .unwrap()
        .base
        .primary_target = None;
    assert_ne!(
        present_hash,
        crate::replay::state_hash(&absent),
        "rollback hashing must distinguish live slot zero from absence"
    );
}

#[test]
fn network_initial_snapshot_round_trips_typed_slot_zero_and_spatial_provenance() {
    std::thread::Builder::new()
        .name("typed-sentinel-network-snapshot".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(network_initial_snapshot_round_trips_typed_slot_zero_and_spatial_provenance_inner)
        .expect("spawn large-stack network snapshot test")
        .join()
        .expect("network snapshot test panicked");
}

fn network_initial_snapshot_round_trips_typed_slot_zero_and_spatial_provenance_inner() {
    let (engine, id) = typed_sentinel_snapshot_fixture();
    let message = crate::multiplayer::NetMsg::InitialSnapshot {
        frame: 37,
        engine_bytes: engine.encode_native_snapshot(),
    };

    let decoded_message = crate::multiplayer::decode_msg(&crate::multiplayer::encode_msg(&message))
        .expect("decode network message");
    let crate::multiplayer::NetMsg::InitialSnapshot {
        frame,
        engine_bytes,
    } = decoded_message
    else {
        panic!("network message changed variant")
    };
    assert_eq!(frame, 37);
    let decoded = Engine::decode_native_snapshot(&engine_bytes)
        .expect("decode network-carried engine snapshot");
    assert_typed_sentinel_snapshot(&decoded, id);
}

fn pending_drop_ale_seek(
    owner: EntityId,
    destination: crate::coordinates::MapPoint,
    fallback_sector: crate::position_interface::SectorHandle,
) -> crate::sequence::SequenceElement {
    use crate::element::Command;
    use crate::sequence::{MoveFlags, Sequence, SequenceElement, SequenceElementData};

    let mut seek = SequenceElement::new_movement(
        1,
        Command::Seek,
        Some(owner),
        crate::order::OrderType::WalkingUpright,
    );
    seek.point_seek_route_provenance = crate::sequence::PointSeekRouteProvenance::OriginalReplay;
    let SequenceElementData::Movement {
        destination: seek_destination,
        layer,
        sector,
        flags,
        post_seek_sequence,
        ..
    } = &mut seek.data
    else {
        unreachable!()
    };
    *seek_destination = destination;
    *layer = 2;
    *sector = Some(fallback_sector);
    *flags |= MoveFlags::SEEK;
    let mut post_seek = Sequence::new();
    post_seek.append_element(SequenceElement::new(1, Command::DropAle, Some(owner)));
    *post_seek_sequence = Some(post_seek.into_post_seek());
    seek
}

fn recorded_drop_ale_fact(
    actor: EntityId,
    destination: crate::coordinates::MapPoint,
) -> RecordedDropAleRoute {
    RecordedDropAleRoute {
        actor,
        destination,
        goal_sector: crate::sector::SectorNumber::new(0),
        goal_sector_index: crate::fast_find_grid::SectorIndex::new(0)
            .expect("sector index zero is valid"),
        goal_layer: 0,
        recorded_gate_path: crate::gate::RecordedGatePath {
            source_sector: crate::sector::SectorNumber::new(133),
            source_sector_index: crate::fast_find_grid::SectorIndex::new(57),
            source_layer: 11,
            outcome: crate::gate::RecordedGateOutcome::Failure,
        },
    }
}

fn selection_boundary_fixture() -> (Engine, LevelAssets, EntityId, crate::sequence::SequenceId) {
    let (mut engine, mut assets) = frame_api_fixture();
    let mut actions = [crate::profiles::Action::NoAction; crate::profiles::NUMBER_OF_PC_ACTIONS];
    actions[0] = crate::profiles::Action::Net;
    let mut maximum_ammo = [0; crate::profiles::NUMBER_OF_PC_ACTIONS];
    maximum_ammo[0] = 1;
    let mut profiles = crate::profiles::ProfileManager::new();
    profiles
        .missions
        .push(crate::profiles::MissionProfile::default());
    profiles.characters.push(crate::profiles::CharacterProfile {
        actions,
        action_max_ammo: maximum_ammo,
        ..Default::default()
    });
    assets.profile_manager = std::sync::Arc::new(profiles);

    engine
        .inner
        .mission_domain
        .campaign
        .characters
        .push(crate::campaign::PcDescription {
            character_profile_idx: Some(crate::profiles::CharacterProfileIdx(0)),
            instanced: true,
            ..Default::default()
        });
    let pc_id = engine
        .inner
        .add_entity(crate::element::Entity::Pc(crate::element::ActorPc {
            element: {
                let mut initial_element = crate::element::ElementData::from_initial_posture(
                    crate::element::Posture::Upright,
                );
                initial_element.kind = crate::element::ElementKind::ActorPc;
                initial_element.active = true;
                initial_element
            },
            actor: crate::element::ActorData::default(),
            human: crate::element::HumanData::default(),
            pc: crate::element::PcData {
                profile_index: crate::profiles::CharacterProfileIdx(0),
                campaign_description_index: Some(0),
                life_points: 50,
                current_action: crate::profiles::Action::Net,
                ..Default::default()
            },
        }));

    let mut wait = crate::sequence::SequenceElement::new_generic(
        1,
        crate::element::Command::WaitTimer,
        Some(pc_id),
    );
    wait.priority = crate::sequence::SequencePriority::Wait;
    let wait_sequence = engine.inner.orders.sequence_manager.launch_element(wait);
    engine
        .inner
        .orders
        .sequence_manager
        .element_in_progress(wait_sequence, 0);
    (engine, assets, pc_id, wait_sequence)
}

#[test]
fn spatial_presentation_sampling_is_absolute_and_authoritative_hashes_are_unchanged() {
    let (mut previous, _, pc_id, _) = selection_boundary_fixture();
    previous
        .inner
        .get_entity_mut(pc_id)
        .expect("presentation test PC")
        .element_data_mut()
        .set_position(crate::coordinates::WorldPoint3D::ZERO);
    let mut current = previous.clone();
    {
        let entity = current
            .inner
            .get_entity_mut(pc_id)
            .expect("presentation test PC");
        entity
            .element_data_mut()
            .set_position(crate::coordinates::WorldPoint3D::new(40.0, 60.0, 10.0));
        entity
            .actor_data_mut()
            .expect("presentation test actor")
            .jump_z_offset = 8.0;
    }
    let previous_hash = crate::replay::state_hash(&previous);
    let current_hash = crate::replay::state_hash(&current);
    let previous_spatial = previous.spatial_presentation_snapshot();
    let current_spatial = current.spatial_presentation_snapshot();
    let mut presentation = PresentationEngine::new(&current);

    presentation.apply_spatial_presentation(&previous_spatial, &current_spatial, 0.25);
    let first_sample_hash = crate::replay::state_hash(&presentation.presentation);
    let sampled = presentation.view().get_entity(pc_id).expect("sampled PC");
    assert_eq!(
        sampled.element_data().position(),
        crate::coordinates::WorldPoint3D::new(10.0, 15.0, 2.5)
    );
    assert_eq!(
        sampled.element_data().position_map(),
        crate::coordinates::MapPoint::new(10.0, 12.5)
    );
    assert_eq!(
        sampled.actor_data().expect("sampled actor").jump_z_offset,
        2.0
    );

    presentation.apply_spatial_presentation(&previous_spatial, &current_spatial, 0.25);
    assert_eq!(
        crate::replay::state_hash(&presentation.presentation),
        first_sample_hash,
        "repeating one display sample must be idempotent"
    );
    assert_eq!(crate::replay::state_hash(&previous), previous_hash);
    assert_eq!(crate::replay::state_hash(&current), current_hash);
}

#[test]
fn spatial_presentation_snaps_layer_transitions_and_new_entities() {
    let (previous, _, pc_id, _) = selection_boundary_fixture();
    let mut current = previous.clone();
    {
        let element = current
            .inner
            .get_entity_mut(pc_id)
            .expect("presentation test PC")
            .element_data_mut();
        element.set_position_map(crate::coordinates::MapPoint::new(64.0, 96.0));
        element.set_layer(1);
    }
    let spawned_id =
        current
            .inner
            .add_entity(crate::element::Entity::Fx(crate::element::ElementFx {
                element: {
                    let mut initial_element = crate::element::ElementData::default();
                    initial_element.kind = crate::element::ElementKind::Fx;
                    initial_element
                },
                fx: Default::default(),
            }));
    current
        .inner
        .get_entity_mut(spawned_id)
        .expect("spawned presentation FX")
        .element_data_mut()
        .set_position_map(crate::coordinates::MapPoint::new(12.0, 34.0));
    let previous_spatial = previous.spatial_presentation_snapshot();
    let current_spatial = current.spatial_presentation_snapshot();
    let mut presentation = PresentationEngine::new(&current);

    presentation.apply_spatial_presentation(&previous_spatial, &current_spatial, 0.0);

    assert_eq!(
        presentation
            .view()
            .get_entity(pc_id)
            .expect("sampled PC")
            .element_data()
            .position_map(),
        crate::coordinates::MapPoint::new(64.0, 96.0),
        "layer transition must snap rather than sweep"
    );
    assert_eq!(
        presentation
            .view()
            .get_entity(spawned_id)
            .expect("sampled spawned FX")
            .element_data()
            .position_map(),
        crate::coordinates::MapPoint::new(12.0, 34.0),
        "spawned entity must use its current fixed-tick transform"
    );
}

#[test]
fn presentation_diagnostics_cannot_restore_live_presentation_or_simulation() {
    let (engine, _, _, _) = selection_boundary_fixture();
    let presentation = PresentationEngine::new(&engine);
    let diagnostic = serde_json::to_value(&presentation).expect("presentation diagnostic");
    assert_eq!(diagnostic["frame"], engine.frame_counter());
    assert_eq!(diagnostic.as_object().expect("diagnostic object").len(), 2);
    assert!(serde_json::from_value::<PresentationEngine>(diagnostic.clone()).is_err());
    assert!(serde_json::from_value::<Engine>(diagnostic).is_err());
    for view in [engine.presentation_view(), presentation.view()] {
        let diagnostic = serde_json::to_value(view).expect("read-view diagnostic");
        assert_eq!(diagnostic.as_object().expect("diagnostic object").len(), 2);
        assert!(
            serde_json::from_value::<super::super::PresentationView<'_>>(diagnostic.clone())
                .is_err()
        );
        assert!(serde_json::from_value::<Engine>(diagnostic).is_err());
    }
}

#[test]
fn presentation_queries_preserve_fixed_world_results_and_snapshot_bytes() {
    use crate::player_command::PlayerId;
    let (engine, assets, pc, _) = selection_boundary_fixture();
    let snapshot = engine.encode_native_snapshot();
    let hash = crate::replay::state_hash(&engine);
    let presentation = PresentationEngine::new(&engine);
    for view in [engine.presentation_view(), presentation.view()] {
        assert_eq!(view.frame_counter(), engine.frame_counter());
        assert_eq!(view.pc_ids(), engine.pc_ids());
        assert_eq!(view.npc_ids(), engine.npc_ids());
        assert_eq!(view.displayed_pc_ids(), engine.displayed_pc_ids());
        assert_eq!(view.sort_for_minimap(), engine.sort_for_minimap());
        assert_eq!(view.fog_entity_visible(pc), engine.fog_entity_visible(pc));
        assert_eq!(
            view.fog_entity_is_hostile(pc),
            engine.fog_entity_is_hostile(pc)
        );
        assert_eq!(
            view.has_mission_geometry(),
            engine.mission_script().is_some()
        );
        assert_eq!(view.mission_won(), engine.mission().mission_won);
        assert_eq!(
            view.more_combat_gestures(),
            engine.sim_config().more_combat_gestures
        );
        assert_eq!(
            view.timed_missions_enabled(),
            engine.sim_config().enable_timed_missions
        );
        assert_eq!(
            view.uses_original_rng_replay(),
            engine.original_rng_replay_cursor().is_some()
        );
        assert_eq!(
            view.get_entity(pc).unwrap().element_data().position(),
            engine.get_entity(pc).unwrap().element_data().position()
        );
        assert_eq!(
            view.active_entity_positions().collect::<Vec<_>>(),
            engine.active_entity_positions().collect::<Vec<_>>()
        );
        assert_eq!(
            view.hero_selection(PlayerId::HOST),
            engine.hero_selection(PlayerId::HOST)
        );
        assert_eq!(
            view.tactical_selection(PlayerId::HOST),
            engine.tactical_selection(PlayerId::HOST)
        );
        assert_eq!(
            view.selected_action_for_seat(PlayerId::HOST),
            engine.selected_action_for_seat(PlayerId::HOST)
        );
        assert_eq!(
            view.planned_action_for_seat(PlayerId::HOST),
            engine.planned_action_for_seat(PlayerId::HOST)
        );
        assert_eq!(
            view.compute_display_order().ids,
            engine.compute_display_order().ids
        );
        assert_eq!(
            format!("{:?}", view.minimap_dot_info(pc, &assets)),
            format!("{:?}", engine.minimap_dot_info(pc, &assets))
        );
        assert_eq!(
            view.compute_display_order().depths,
            engine.compute_display_order().depths
        );
        assert_eq!(
            serde_json::to_value(view.campaign()).unwrap(),
            serde_json::to_value(engine.campaign()).unwrap()
        );
    }
    assert_eq!(engine.encode_native_snapshot(), snapshot);
    assert_eq!(crate::replay::state_hash(&engine), hash);
}

fn adjacent_select_and_cancel(pc_id: EntityId) -> Vec<SimCommand> {
    vec![
        SimCommand::from(PlayerCommand::SelectPc {
            pc_id,
            append: false,
        }),
        SimCommand::from(PlayerCommand::CancelAction { pc_id }),
    ]
}

#[test]
fn original_parity_frame_preserves_pre_and_post_command_boundaries() {
    for post_hourglass in [false, true] {
        let (mut engine, assets, pc_id, wait_sequence) = selection_boundary_fixture();
        let commands = adjacent_select_and_cancel(pc_id);
        let frame = if post_hourglass {
            SimulationFrameInput::no_hourglass().with_post_commands(commands)
        } else {
            SimulationFrameInput::new(commands).with_hourglass(false)
        };
        engine
            .parity_replay_setup()
            .advance_frame(&assets, frame)
            .expect("advance Original parity frame");

        assert_eq!(
            engine
                .inner
                .orders
                .sequence_manager
                .get_element(wait_sequence, 0)
                .expect("interrupted wait remains inspectable")
                .state,
            crate::sequence::SequenceState::Interrupted,
            "{}-hourglass commands must remain independent recorded siblings",
            if post_hourglass { "post" } else { "pre" },
        );
    }
}

#[test]
fn ordinary_frame_keeps_live_nested_selection_inference() {
    let (mut engine, assets, pc_id, wait_sequence) = selection_boundary_fixture();

    engine
        .advance_frame(
            &assets,
            SimulationFrameInput::new(adjacent_select_and_cancel(pc_id)).with_hourglass(false),
        )
        .expect("advance ordinary frame");

    assert_eq!(
        engine
            .inner
            .orders
            .sequence_manager
            .get_element(wait_sequence, 0)
            .expect("live wait remains inspectable")
            .state,
        crate::sequence::SequenceState::InProgress,
        "ordinary admission must retain its existing nested-selection heuristic",
    );
}

#[test]
fn frame_api_matches_legacy_command_then_hourglass_boundary() {
    let (engine, assets) = frame_api_fixture();
    let mut legacy = engine.clone();
    let mut framed = engine;
    let commands = vec![PlayerInput::host(
        PlayerCommand::SetMenToBlazonConversionMode { on: true },
    )];

    let mut legacy_display = super::super::HostDisplayState::default();
    let mut legacy_input = InputState::default();
    let mut legacy_dev = DevState::default();
    legacy.apply_commands(&mut legacy_display, &mut legacy_input, &assets, &commands);
    let legacy_events = legacy.perform_hourglass(
        &mut legacy_display,
        &mut legacy_input,
        &assets,
        &mut legacy_dev,
    );
    let legacy_hash = crate::replay::state_hash(&legacy);

    let output = framed
        .advance_frame(&assets, SimulationFrameInput::from_player_inputs(commands))
        .expect("advance frame");

    assert_eq!(output.frame_before, 0);
    assert_eq!(output.frame_after, 1);
    assert_eq!(output.state_hash, legacy_hash);
    assert_eq!(crate::replay::state_hash(&framed), legacy_hash);
    assert_eq!(
        serde_json::to_value(output.events.side_effects()).expect("serialize frame events"),
        serde_json::to_value(&legacy_events).expect("serialize legacy side effects"),
    );
    assert_eq!(
        output.events.side_effects().pending_minimap_position,
        legacy_events.pending_minimap_position,
        "this host-local effect is serde-skipped and must be compared explicitly"
    );
    assert!(framed.is_men_to_blazon_conversion_mode());
}

#[test]
fn recorded_drop_ale_facts_round_trip_and_reject_atomically() {
    let (mut engine, assets) = frame_api_fixture();
    let owner = EntityId::Pc(crate::entity_id::PcId(36));
    let destination = crate::coordinates::MapPoint::new(778.0, 1714.0);
    let fallback_sector =
        crate::position_interface::SectorHandle::new(25).expect("fallback sector is valid");
    engine
        .inner
        .orders
        .sequence_manager
        .launch_element(pending_drop_ale_seek(owner, destination, fallback_sector));

    let fact = recorded_drop_ale_fact(owner, destination);
    let input = SimulationFrameInput::no_hourglass().with_external_facts(
        ExternalFacts::default().with_recorded_drop_ale_routes(vec![fact.clone()]),
    );
    let encoded = bitcode::encode(&input);
    let decoded: SimulationFrameInput = bitcode::decode(&encoded).expect("decode typed frame fact");
    let mut direct = engine.clone();
    let mut replayed = engine.clone();
    direct
        .advance_frame(&assets, input)
        .expect("admit recorded DropAle route");
    replayed
        .advance_frame(&assets, decoded)
        .expect("replay recorded DropAle route");
    assert_eq!(
        crate::replay::state_hash(&direct),
        crate::replay::state_hash(&replayed),
        "the full frame journal must retain delayed route resolution"
    );

    let before = crate::replay::state_hash(&engine);
    let rejected = engine
        .advance_frame(
            &assets,
            SimulationFrameInput::no_hourglass().with_external_facts(
                ExternalFacts::default().with_recorded_drop_ale_routes(vec![
                    fact,
                    recorded_drop_ale_fact(EntityId::Pc(crate::entity_id::PcId(37)), destination),
                ]),
            ),
        )
        .expect_err("a route without a pending DropAle seek must be rejected");
    assert!(matches!(
        rejected,
        FrameAdvanceError::RecordedDropAleRouteRejected { index: 1, .. }
    ));
    assert_eq!(
        crate::replay::state_hash(&engine),
        before,
        "a rejected later fact must not publish the accepted prefix"
    );
}

#[test]
fn paused_campaign_actions_are_typed_no_hourglass_transactions() {
    let (mut engine, assets) = frame_api_fixture();
    engine
        .inner
        .mission_domain
        .campaign
        .last_pseudo_mission_status = crate::mission::MissionStatus::Won;
    let blazons_before = engine
        .campaign()
        .get_value(crate::campaign::CampaignValue::Blazon);

    let output = engine
        .advance_frame(
            &assets,
            SimulationFrameInput::no_hourglass().with_external_actions(vec![
                ExternalAction::CampaignBuyBlazon { mission_index: 0 },
                ExternalAction::AcknowledgePseudoMissionDebrief,
            ]),
        )
        .expect("admit paused campaign actions");

    assert!(!output.hourglass_ran);
    assert_eq!(output.frame_before, output.frame_after);
    assert!(matches!(
        output.external_action_results.as_slice(),
        [
            ExternalActionResult::CampaignBuyBlazon {
                closed_by_cascade: false
            },
            ExternalActionResult::AcknowledgePseudoMissionDebrief
        ]
    ));
    assert_eq!(
        engine
            .campaign()
            .get_value(crate::campaign::CampaignValue::Blazon),
        blazons_before + 1
    );
    assert_eq!(
        engine.campaign().get_last_pseudo_mission_status(),
        crate::mission::MissionStatus::Available
    );
}

#[test]
fn frame_api_applies_sound_external_fact_at_pre_hourglass_boundary() {
    use crate::sound::{ExclamationGroup, PendingExclamation, ResolvedExclamation};

    let (mut framed, assets) = frame_api_fixture();
    let profile_id = 0x4651_0000;
    framed
        .inner
        .feedback
        .sound_sim
        .pending_exclamations
        .push(PendingExclamation {
            actor_id: 191,
            group: ExclamationGroup::Civilian,
            profile_id,
            exclamation_id: 62,
            variant: -1,
        });
    let resolution = ResolvedExclamation {
        actor_id: 191,
        identifier: profile_id | 62,
        exclamation_id: 62,
        duration_frames: 24,
    };

    framed
        .advance_frame(
            &assets,
            SimulationFrameInput::new(vec![SimCommand::from(PlayerCommand::Noop)])
                .with_external_facts(
                    ExternalFacts::default()
                        .with_sound_boundary(SoundBoundary::live(vec![resolution])),
                ),
        )
        .expect("advance frame with sound fact");

    assert!(framed.sound_sim().resolved_exclamations.is_empty());
    assert_eq!(framed.sound_sim().playing_exclamations.len(), 1);
    assert_eq!(framed.sound_sim().playing_exclamations[0].actor_id, 191);
    assert_eq!(
        framed.sound_sim().playing_exclamations[0].finish_frame,
        24,
        "the fact is resolved at the frame-0 boundary before the hourglass increments the clock"
    );
}

fn ranked_sound_boundary_fixture(variant: i32) -> (Engine, LevelAssets) {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use crate::sound::{ExclamationGroup, PendingExclamation};

    let (mut engine, mut assets) = frame_api_fixture();
    let policy = crate::engine::RankedSimulationPolicy::standard_medium();
    let config = policy.expected_config();
    engine.inner.control.sim_config = config;
    engine.inner.control.mission_start_sim_config = config;
    engine
        .inner
        .control
        .install_ranked_simulation_policy(policy);

    let profile_id = 0x4651_0000;
    let exclamation_id = 62;
    engine
        .inner
        .feedback
        .sound_sim
        .pending_exclamations
        .push(PendingExclamation {
            actor_id: 191,
            group: ExclamationGroup::Civilian,
            profile_id,
            exclamation_id,
            variant,
        });
    assets.audio.speech_timing_catalog = Arc::new(crate::engine::SpeechTimingCatalog {
        groups: BTreeMap::from([(
            profile_id | u32::from(exclamation_id),
            crate::engine::SpeechTimingGroup {
                gaps: 1,
                variants: vec![
                    crate::engine::SpeechTimingVariant {
                        sample_identity: "speech-a.wav".into(),
                        duration_frames: Some(17),
                    },
                    crate::engine::SpeechTimingVariant {
                        sample_identity: "speech-b.wav".into(),
                        duration_frames: Some(29),
                    },
                ],
            },
        )]),
    });
    (engine, assets)
}

fn ranked_sound_resolution(duration_frames: u32) -> crate::sound::ResolvedExclamation {
    crate::sound::ResolvedExclamation {
        actor_id: 191,
        identifier: 0x4651_003e,
        exclamation_id: 62,
        duration_frames,
    }
}

#[test]
fn ranked_sound_boundary_accepts_only_sealed_authored_timing() {
    let (mut random_engine, random_assets) = ranked_sound_boundary_fixture(-1);
    random_engine
        .advance_frame(
            &random_assets,
            SimulationFrameInput::no_hourglass().with_external_facts(
                ExternalFacts::default()
                    .with_sound_boundary(SoundBoundary::live(vec![ranked_sound_resolution(29)])),
            ),
        )
        .expect("random playback uses the canonical maximum English duration");

    let (mut explicit_engine, explicit_assets) = ranked_sound_boundary_fixture(0);
    explicit_engine
        .advance_frame(
            &explicit_assets,
            SimulationFrameInput::no_hourglass().with_external_facts(
                ExternalFacts::default()
                    .with_sound_boundary(SoundBoundary::live(vec![ranked_sound_resolution(17)])),
            ),
        )
        .expect("an explicit speech choice must use its ordered sealed variant");
}

#[test]
fn ranked_sound_boundary_rejects_forged_duration_and_variant() {
    for (variant, duration, expected_reason) in [
        (-1, 23, "unauthoritative duration"),
        (-1, 17, "unauthoritative duration"),
        (0, 29, "unauthoritative duration"),
        (2, 17, "outside the 2 authored variants"),
        (-2, 17, "invalid variant -2"),
    ] {
        let (mut engine, assets) = ranked_sound_boundary_fixture(variant);
        let before = crate::replay::state_hash(&engine);
        let error = engine
            .advance_frame(
                &assets,
                SimulationFrameInput::no_hourglass().with_external_facts(
                    ExternalFacts::default().with_sound_boundary(SoundBoundary::live(vec![
                        ranked_sound_resolution(duration),
                    ])),
                ),
            )
            .expect_err("forged ranked speech timing must fail closed");
        let FrameAdvanceError::SoundBoundaryRejected { reason, .. } = error else {
            panic!("unexpected ranked sound error: {error:?}");
        };
        assert!(
            reason.contains(expected_reason),
            "unexpected rejection for variant {variant}: {reason}"
        );
        assert_eq!(crate::replay::state_hash(&engine), before);
    }
}

#[test]
fn ranked_sound_boundary_rejects_replay_policy() {
    let (mut engine, assets) = ranked_sound_boundary_fixture(-1);
    let before = crate::replay::state_hash(&engine);
    let error = engine
        .advance_frame(
            &assets,
            SimulationFrameInput::no_hourglass().with_external_facts(
                ExternalFacts::default()
                    .with_sound_boundary(SoundBoundary::replay(vec![ranked_sound_resolution(17)])),
            ),
        )
        .expect_err("ranked runs must reject Original-trace sound authority");
    assert!(matches!(
        error,
        FrameAdvanceError::SoundBoundaryRejected {
            policy: SoundBoundaryPolicy::Replay,
            ..
        }
    ));
    assert_eq!(crate::replay::state_hash(&engine), before);
}

#[test]
fn rejected_live_sound_boundary_is_atomic() {
    use crate::sound::{ExclamationGroup, PendingExclamation, ResolvedExclamation};

    let (mut engine, assets) = frame_api_fixture();
    engine
        .inner
        .feedback
        .sound_sim
        .pending_exclamations
        .push(PendingExclamation {
            actor_id: 191,
            group: ExclamationGroup::Civilian,
            profile_id: 0x4651_0000,
            exclamation_id: 62,
            variant: -1,
        });
    let invalid_resolution = ResolvedExclamation {
        actor_id: 192,
        identifier: 0x4651_003f,
        exclamation_id: 63,
        duration_frames: 24,
    };
    let engine_hash_before = crate::replay::state_hash(&engine);
    let error = engine
        .advance_frame(
            &assets,
            SimulationFrameInput::new(vec![SimCommand::from(
                PlayerCommand::SetMenToBlazonConversionMode { on: true },
            )])
            .with_external_facts(
                ExternalFacts::default()
                    .with_sound_boundary(SoundBoundary::live(vec![invalid_resolution])),
            ),
        )
        .expect_err("a live sound resolution must match the pending FIFO");

    assert!(matches!(
        error,
        FrameAdvanceError::SoundBoundaryRejected {
            policy: SoundBoundaryPolicy::Live,
            ..
        }
    ));
    assert_eq!(crate::replay::state_hash(&engine), engine_hash_before);
    assert_eq!(engine.frame_counter(), 0);
    assert!(!engine.is_men_to_blazon_conversion_mode());
}

#[test]
fn rejected_external_fact_prevents_command_and_hourglass() {
    use crate::element::Command;
    use crate::sequence::{Field, FieldValue, Sequence, SequenceElement};

    let (mut engine, assets) = frame_api_fixture();
    engine.set_external_director_completion_replay(true);

    // Launch a real camera command. The first completion therefore mutates
    // the Engine before the second, invalid completion is rejected.
    let mut camera = SequenceElement::new_generic(1, Command::CameraGoto, None);
    camera.set_property(
        Field::CameraPoint,
        FieldValue::GeoPoint2D { x: 100.0, y: 100.0 },
    );
    camera.set_property(Field::CameraSpeed, FieldValue::Integer(0));
    let mut sequence = Sequence::new();
    sequence.append_element(camera);
    let sequence_id = engine
        .inner
        .orders
        .sequence_manager
        .launch_sequence(sequence);
    engine
        .inner
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);
    engine.inner.feedback.cutscene_camera.sequence_element =
        Some(crate::sequence::SequenceElementRef::new(sequence_id, 0));
    assert!(
        engine
            .inner
            .feedback
            .cutscene_camera
            .sequence_element
            .is_some(),
        "fixture must have an active CameraGoto"
    );

    let engine_hash_before = crate::replay::state_hash(&engine);
    let mut accepted_engine = engine.clone();
    accepted_engine
        .advance_frame(
            &assets,
            SimulationFrameInput::no_hourglass().with_external_facts(
                ExternalFacts::default()
                    .with_director_completions(vec![DirectorCompletion::CameraGoto]),
            ),
        )
        .expect("the first director fact must be independently valid");
    assert_ne!(
        crate::replay::state_hash(&accepted_engine),
        engine_hash_before,
        "the accepted prefix must mutate the staged engine"
    );
    let error = engine
        .advance_frame(
            &assets,
            SimulationFrameInput::new(vec![
                SimCommand::from(PlayerCommand::SetMenToBlazonConversionMode { on: true }),
                SimCommand::from(PlayerCommand::MouseRightUp),
            ])
            .with_external_facts(ExternalFacts::default().with_director_completions(
                vec![
                    DirectorCompletion::CameraGoto,
                    DirectorCompletion::CameraGoto,
                ],
            )),
        )
        .expect_err("the second completion has no active camera command");

    assert!(matches!(
        error,
        FrameAdvanceError::DirectorCompletionRejected {
            index: 1,
            completion: DirectorCompletion::CameraGoto,
            ..
        }
    ));
    assert_eq!(crate::replay::state_hash(&engine), engine_hash_before);
    assert_eq!(engine.frame_counter(), 0);
    assert!(!engine.is_men_to_blazon_conversion_mode());
}

#[test]
fn no_hourglass_director_prefix_exposes_new_delayed_drop_ale_seek() {
    use crate::element::Command;
    use crate::sequence::{Field, FieldValue, Sequence, SequenceElement};

    let (mut engine, assets) = frame_api_fixture();
    engine.set_external_director_completion_replay(true);
    let owner = EntityId::Pc(crate::entity_id::PcId(36));
    let destination = crate::coordinates::MapPoint::new(778.0, 1714.0);
    let fallback_sector =
        crate::position_interface::SectorHandle::new(25).expect("valid fallback sector");

    let mut camera = SequenceElement::new_generic(1, Command::CameraGoto, None);
    camera.set_property(
        Field::CameraPoint,
        FieldValue::GeoPoint2D { x: 100.0, y: 100.0 },
    );
    camera.set_property(Field::CameraSpeed, FieldValue::Integer(0));
    let mut seek = pending_drop_ale_seek(owner, destination, fallback_sector);
    seek.command_level = 2;
    let mut sequence = Sequence::new();
    sequence.append_element(camera);
    sequence.append_element(seek);
    let sequence_id = engine
        .inner
        .orders
        .sequence_manager
        .launch_sequence(sequence);
    engine
        .inner
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);
    engine.inner.feedback.cutscene_camera.sequence_element =
        Some(crate::sequence::SequenceElementRef::new(sequence_id, 0));

    assert!(
        !engine.has_pending_recorded_drop_ale_route(owner, destination),
        "the later command level must not be claimable before the prefix releases it"
    );
    engine
        .advance_frame(
            &assets,
            SimulationFrameInput::no_hourglass().with_external_facts(
                ExternalFacts::default()
                    .with_director_completions(vec![DirectorCompletion::CameraGoto]),
            ),
        )
        .expect("stage director prefix");
    assert!(
        engine.has_pending_recorded_drop_ale_route(owner, destination),
        "the delayed route collector must inspect state after the director/sound prefix"
    );
}

#[test]
fn external_facts_are_part_of_the_authoritative_frame_journal() {
    use crate::sound::{ExclamationGroup, PendingExclamation, ResolvedExclamation};

    let (mut initial, assets) = frame_api_fixture();
    let profile_id = 0x4651_0000;
    initial
        .inner
        .feedback
        .sound_sim
        .pending_exclamations
        .push(PendingExclamation {
            actor_id: 191,
            group: ExclamationGroup::Civilian,
            profile_id,
            exclamation_id: 62,
            variant: -1,
        });
    let mut complete_journal = initial.clone();
    let mut command_only_journal = initial;
    let command = SimCommand::from(PlayerCommand::Noop);
    let resolution = ResolvedExclamation {
        actor_id: 191,
        identifier: profile_id | 62,
        exclamation_id: 62,
        duration_frames: 24,
    };

    let complete_output = complete_journal
        .advance_frame(
            &assets,
            SimulationFrameInput::new(vec![command.clone()]).with_external_facts(
                ExternalFacts::default().with_sound_boundary(SoundBoundary::live(vec![resolution])),
            ),
        )
        .expect("advance complete frame journal");

    let command_only_output = command_only_journal
        .advance_frame(&assets, SimulationFrameInput::new(vec![command]))
        .expect("advance command-only frame journal");

    assert_ne!(
        complete_output.state_hash, command_only_output.state_hash,
        "replaying commands without the recorded host sound fact must not be treated as equivalent"
    );
    assert_eq!(complete_journal.sound_sim().playing_exclamations.len(), 1);
    assert!(complete_journal.sound_sim().pending_exclamations.is_empty());
    assert!(
        command_only_journal
            .sound_sim()
            .playing_exclamations
            .is_empty()
    );
    assert_eq!(
        command_only_journal.sound_sim().pending_exclamations.len(),
        1
    );
}

#[test]
fn closed_body_gate_is_not_a_paused_presentation_boundary() {
    let (mut engine, assets) = frame_api_fixture();
    let output = engine
        .advance_frame(
            &assets,
            SimulationFrameInput::default().with_simulation_body_allowed(false),
        )
        .expect("advance with only the actor/world body gated");

    assert_eq!(output.frame_before, 0);
    assert_eq!(output.frame_after, 1);
    assert_eq!(engine.frame_counter(), 1);
}

#[test]
fn parity_engine_state_preserves_next_original_creation_order() {
    let mut inner = EngineInner::new();
    inner.world.next_original_creation_order = 417;
    inner.control.chorus_timer = 23;
    inner.script_domains.mission_ui.force_check = true;
    inner
        .script_domains
        .mission_ui
        .men_to_blazon_conversion_mode = true;
    let state = Engine {
        inner,
        bootstrap_open: false,
    }
    .parity_engine_state();

    assert_eq!(state.next_creation_order, 417);
    assert_eq!(state.chorus_timer, 23);
    assert!(state.force_check);
    assert!(state.men_to_blazon_conversion);
}

fn parity_position_sprite(state: &serde_json::Value) -> (u32, u32) {
    (
        state["position"]["sprite"]["x"]["bits"]
            .as_u64()
            .expect("sprite x bits") as u32,
        state["position"]["sprite"]["y"]["bits"]
            .as_u64()
            .expect("sprite y bits") as u32,
    )
}

#[test]
fn parity_runtime_projects_current_sprite_top_left_for_ordinary_entities() {
    let mut inner = EngineInner::new();
    let mut element = {
        let mut initial_element = crate::element::ElementData::default();
        initial_element.kind = crate::element::ElementKind::Fx;
        initial_element
    };
    element.sprite.center = crate::coordinates::SpriteAnchor::new(150.0, 150.0);
    element
        .sprite
        .position_iface
        .set_cached_sprite_position(crate::coordinates::MapPoint::new(1688.0, 150.0));
    element.set_position_map(crate::coordinates::MapPoint::new(1836.2246, 301.3214));
    let id = inner.add_entity(crate::element::Entity::Fx(crate::element::ElementFx {
        element,
        fx: Default::default(),
    }));

    let state = Engine {
        inner,
        bootstrap_open: false,
    }
    .parity_entity_runtime_state(id, &LevelAssets::new());

    assert_eq!(
        parity_position_sprite(&state),
        (1686.0_f32.to_bits(), 151.0_f32.to_bits())
    );
}

#[test]
fn parity_runtime_preserves_target_cached_sprite_anchor() {
    let mut inner = EngineInner::new();
    let mut element = {
        let mut initial_element = crate::element::ElementData::default();
        initial_element.kind = crate::element::ElementKind::Target;
        initial_element
    };
    element.sprite.center = crate::coordinates::SpriteAnchor::new(30.0, 140.0);
    element
        .sprite
        .position_iface
        .set_cached_sprite_position(crate::coordinates::MapPoint::new(2791.0, 171.0));
    element.set_position_map_preserving_3d(crate::coordinates::MapPoint::new(2823.0, 312.0));
    let id = inner.add_entity(crate::element::Entity::Target(
        crate::element::ElementTarget {
            element,
            fx: Default::default(),
            target: Default::default(),
        },
    ));

    let state = Engine {
        inner,
        bootstrap_open: false,
    }
    .parity_entity_runtime_state(id, &LevelAssets::new());

    assert_eq!(
        parity_position_sprite(&state),
        (2791.0_f32.to_bits(), 171.0_f32.to_bits())
    );
}

#[test]
fn parity_runtime_refreshes_bank_dimensions_only_after_recorded_boundary() {
    struct Frames;
    impl crate::engine::PixelOpacityLookup for Frames {
        fn sprite_dimensions(&self, bank_id: u32) -> Option<(u16, u16)> {
            (bank_id == 73).then_some((48, 19))
        }

        fn is_pixel_opaque(
            &self,
            _bank_id: u32,
            _x: u16,
            _y: u16,
            _blue_pixels_are_in: bool,
        ) -> bool {
            false
        }
    }

    let mut inner = EngineInner::new();
    let mut sprite = crate::sprite::Sprite {
        current_width: 44,
        current_height: 20,
        current_row: 0,
        current_frame: 1,
        masked: true,
        ..Default::default()
    };
    // The stale 44-pixel cache ends before the viewport, while the
    // current 48-pixel bank surface intersects it. The original game's visibility test
    // uses the latter before target-sprite creation publishes the cache.
    sprite
        .position_iface
        .set_map_position(crate::coordinates::MapPoint::new(-47.0, 0.0));
    sprite.scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
        frame_ids: vec![72, 73],
        offsets: vec![
            crate::coordinates::SpriteFrameOffset::ZERO,
            crate::coordinates::SpriteFrameOffset::ZERO,
        ],
        ..Default::default()
    }]);
    let id = inner.add_entity(crate::element::Entity::Fx(crate::element::ElementFx {
        element: {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::Fx;
            initial_element.active = true;
            initial_element.sprite = sprite;
            initial_element
        },
        fx: Default::default(),
    }));
    let assets = LevelAssets {
        attachments: crate::engine::LevelRuntimeAttachments {
            pixel_opacity: Some(std::sync::Arc::new(Frames)),
            ..Default::default()
        },
        ..LevelAssets::new()
    };

    let mut engine = Engine {
        inner,
        bootstrap_open: false,
    };
    let before_refresh = engine.parity_entity_runtime_state(id, &assets);
    engine
        .parity_replay_setup()
        .refresh_sprite_dimension_cache(&assets, false);
    let after_refresh = engine.parity_entity_runtime_state(id, &assets);

    assert_eq!(before_refresh["sprite"]["width"], 44);
    assert_eq!(before_refresh["sprite"]["height"], 20);
    assert_eq!(before_refresh["sprite"]["masked"], true);
    assert_eq!(after_refresh["sprite"]["width"], 48);
    assert_eq!(after_refresh["sprite"]["height"], 19);
    assert_eq!(after_refresh["sprite"]["masked"], false);
}

#[test]
fn missing_draw_view_skips_presentation_cache_refresh() {
    struct Frames;
    impl crate::engine::PixelOpacityLookup for Frames {
        fn sprite_dimensions(&self, bank_id: u32) -> Option<(u16, u16)> {
            (bank_id == 73).then_some((24, 55))
        }

        fn is_pixel_opaque(
            &self,
            _bank_id: u32,
            _x: u16,
            _y: u16,
            _blue_pixels_are_in: bool,
        ) -> bool {
            false
        }
    }

    let mut inner = EngineInner::new();
    let mut sprite = crate::sprite::Sprite {
        current_width: 20,
        current_height: 53,
        current_row: 0,
        current_frame: 0,
        ..Default::default()
    };
    // The reconstructed view ends at x=1024. Original passes that exact
    // box to sprite visibility; DrawManager's separate +/-1 draw range
    // must not leak into the element refresh visibility test.
    sprite.center = crate::coordinates::SpriteAnchor::new(-1025.0, -20.0);
    sprite.scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
        frame_ids: vec![73],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
        ..Default::default()
    }]);
    let id = inner.add_entity(crate::element::Entity::Fx(crate::element::ElementFx {
        element: {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::Fx;
            initial_element.active = true;
            initial_element.sprite = sprite;
            initial_element
        },
        fx: Default::default(),
    }));
    let assets = LevelAssets {
        attachments: crate::engine::LevelRuntimeAttachments {
            pixel_opacity: Some(std::sync::Arc::new(Frames)),
            ..Default::default()
        },
        ..LevelAssets::new()
    };
    let mut engine = Engine {
        inner,
        bootstrap_open: false,
    };
    let gameplay_position = engine.inner.world.entities[id]
        .as_ref()
        .expect("test entity must remain occupied")
        .element_data()
        .position_map();

    engine
        .parity_replay_setup()
        .refresh_sprite_dimension_cache(&assets, false);
    let without_legacy_edge = engine.parity_entity_runtime_state(id, &assets);
    assert_eq!(without_legacy_edge["sprite"]["width"], 20);
    assert_eq!(without_legacy_edge["sprite"]["height"], 53);

    engine
        .parity_replay_setup()
        .refresh_sprite_dimension_cache(&assets, true);
    let with_legacy_edge = engine.parity_entity_runtime_state(id, &assets);
    assert_eq!(with_legacy_edge["sprite"]["width"], 20);
    assert_eq!(with_legacy_edge["sprite"]["height"], 53);
    assert_eq!(
        engine.inner.world.entities[id]
            .as_ref()
            .expect("test entity must remain occupied")
            .element_data()
            .position_map(),
        gameplay_position,
        "presentation compatibility must not mutate gameplay position"
    );

    let sprite = engine.inner.world.entities[id]
        .as_mut()
        .expect("test entity must remain occupied")
        .sprite_mut();
    sprite.current_width = 20;
    sprite.current_height = 53;
    sprite.center = crate::coordinates::SpriteAnchor::new(-1024.0, -20.0);
    engine
        .parity_replay_setup()
        .refresh_sprite_dimension_cache(&assets, true);
    let touching_view_edge = engine.parity_entity_runtime_state(id, &assets);
    assert_eq!(touching_view_edge["sprite"]["width"], 20);
    assert_eq!(touching_view_edge["sprite"]["height"], 53);

    engine
        .parity_replay_setup()
        .refresh_sprite_dimension_cache(&assets, false);
    let with_exact_view = engine.parity_entity_runtime_state(id, &assets);
    assert_eq!(with_exact_view["sprite"]["width"], 24);
    assert_eq!(with_exact_view["sprite"]["height"], 55);

    // Original also leaves an off-screen sprite's cached dimensions
    // untouched when its current frame ends one unit above the viewport
    // (interactive session 003, session 0003, frame 780).
    let sprite = engine.inner.world.entities[id]
        .as_mut()
        .expect("test entity must remain occupied")
        .sprite_mut();
    sprite.current_width = 20;
    sprite.current_height = 53;
    sprite.center = crate::coordinates::SpriteAnchor::new(0.0, 56.0);
    engine
        .parity_replay_setup()
        .refresh_sprite_dimension_cache(&assets, true);
    let above_legacy_near_edge = engine.parity_entity_runtime_state(id, &assets);
    assert_eq!(above_legacy_near_edge["sprite"]["width"], 20);
    assert_eq!(above_legacy_near_edge["sprite"]["height"], 53);
}

#[test]
fn parity_runtime_projection_ordinal_includes_original_default_ground_slot() {
    use crate::sight_obstacle::{SIGHTOBSTACLE_PROJECTION_AREA, SightObstacle};

    let mut assets = LevelAssets::new();
    assets.environment.static_sight_obstacles = std::sync::Arc::new(vec![
        SightObstacle::new_default(10),
        SightObstacle::new(11, SIGHTOBSTACLE_PROJECTION_AREA),
        SightObstacle::new_default(12),
        SightObstacle::new(13, SIGHTOBSTACLE_PROJECTION_AREA),
    ]);

    for (handle, expected_ordinal) in [(1_u32, 1_u64), (3, 2)] {
        let mut inner = EngineInner::new();
        let mut element = {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::Fx;
            initial_element
        };
        element.set_layer(0);
        element.set_obstacle_index(
            crate::position_interface::ObstacleHandle::new(handle),
            Some(crate::position_interface::PlaneZCoeffs {
                az: 0.0,
                bz: 0.0,
                dz: 0.0,
            }),
        );
        let id = inner.add_entity(crate::element::Entity::Fx(crate::element::ElementFx {
            element,
            fx: Default::default(),
        }));

        let state = Engine {
            inner,
            bootstrap_open: false,
        }
        .parity_entity_runtime_state(id, &assets);
        assert_eq!(
            state["position"]["obstacle"],
            serde_json::json!({ "kind": "projection", "index": expected_ordinal })
        );
    }
}

#[test]
fn parity_game_ui_state_preserves_serialized_latches() {
    let mut inner = EngineInner::new();
    let ui = &mut inner.script_domains.mission_ui;
    ui.campaign_map = true;
    ui.campaign_map_displayed = true;
    ui.game_post_initialized = true;
    ui.start_mission_disabled_temp = true;
    ui.quit_mission_disabled_temp = false;
    ui.start_mission_enabled = true;
    ui.quit_mission_enabled = false;

    assert_eq!(
        Engine {
            inner,
            bootstrap_open: false
        }
        .parity_game_ui_state(),
        serde_json::json!({
            "campaign_map": true,
            "campaign_map_displayed": true,
            "post_initialized": true,
            "start_mission_disabled_temp": true,
            "quit_mission_disabled_temp": false,
            "start_mission_enabled": true,
            "quit_mission_enabled": false,
        })
    );
}

#[test]
fn parity_messenger_controller_is_independent_of_camera_locker() {
    let mut inner = EngineInner::new();
    inner.players.view_locked = true;
    inner.players.seats[0].locker_active = false;
    inner.players.seats[0].selected_action = crate::profiles::Action::Bow;

    let engine = Engine {
        inner,
        bootstrap_open: false,
    };
    assert_eq!(
        engine.parity_messenger_controller_state(),
        serde_json::json!({ "view_locked": true, "selected_action": 1 })
    );
    assert!(!engine.locker_active());
    assert!(engine.view_locked());
}

#[test]
fn parity_shield_controller_preserves_global_protocol_state() {
    let mut inner = EngineInner::new();
    inner.world.shield.is_protected = false;
    inner.world.shield.protected_pc = Some(EntityId::new(7, crate::element::EntityIdKind::Pc));
    inner.world.shield.danger_point = crate::coordinates::WorldPoint3D {
        x: 1.25,
        y: -2.5,
        z: 3.75,
    };

    assert_eq!(
        Engine {
            inner,
            bootstrap_open: false
        }
        .parity_shield_controller_state(),
        serde_json::json!({
            "is_protected": false,
            "protected_pc": { "kind": "pc", "index": 7 },
            "danger_point": {
                "x": { "bits": 1.25_f32.to_bits() },
                "y": { "bits": (-2.5_f32).to_bits() },
                "z": { "bits": 3.75_f32.to_bits() },
            },
        })
    );
}

#[test]
fn parity_sound_sources_preserves_sparse_slots_and_authoritative_fields() {
    let mut inner = EngineInner::new();
    inner.feedback.sound_sim.sources.sources_push_none();
    let mut source = crate::sound_source::SoundSource::new();
    source.source_kind = crate::sound_source::SoundSourceKind::Delayed;
    source.id = 73;
    source.inner_distance = 12;
    source.outer_distance = 34;
    source.noise_covering_distance = 56;
    source.inner_volume = 78;
    source.outer_volume = 9;
    source
        .shape
        .push(crate::coordinates::MapPoint::new(1.5, -2.0));
    source.altitude = crate::sound_geometry::SoundSourceAltitude::Top;
    source.min_delay = 4;
    source.max_delay = 18;
    source.delay_stepping = 5;
    source.timer = 11;
    source.active = true;
    inner.feedback.sound_sim.sources.sources_push_some(source);
    let engine = Engine {
        inner,
        bootstrap_open: false,
    };

    let state = engine.parity_sound_sources_state();
    assert!(state[0].is_null());
    assert_eq!(state[1]["kind"], 2);
    assert_eq!(state[1]["id"], 73);
    assert_eq!(state[1]["noise_covering_distance"], 56);
    assert_eq!(state[1]["shape"][0]["x"]["bits"], 1.5f32.to_bits());
    assert_eq!(state[1]["altitude"], 2);
    assert_eq!(state[1]["timer"], 11);
    assert_eq!(state[1]["active"], true);
    assert_eq!(state[1]["ambience_enabled"], true);
}

#[test]
fn parity_sound_completion_frontier_preserves_pending_order() {
    let mut inner = EngineInner::new();
    inner
        .feedback
        .sound_sim
        .sources
        .sources_push_some(crate::sound_source::SoundSource::new());
    inner
        .feedback
        .sound_sim
        .sources
        .sources_push_some(crate::sound_source::SoundSource::new());
    inner
        .feedback
        .sound_sim
        .playing_sources
        .push(crate::sound::PlayingSource {
            source_index: 1,
            finish_frame: 73,
        });
    inner
        .feedback
        .sound_sim
        .playing_sources
        .push(crate::sound::PlayingSource {
            source_index: 0,
            finish_frame: 91,
        });

    let state = Engine {
        inner,
        bootstrap_open: false,
    }
    .parity_sound_completion_frontier_state();
    assert_eq!(state[0]["source_index"], 1);
    assert_eq!(state[0]["finish_frame"], 73);
    assert_eq!(state[1]["source_index"], 0);
    assert_eq!(state[1]["finish_frame"], 91);
}

#[test]
fn parity_ai_global_preserves_ordered_statuses_reservations_and_alerts() {
    let mut inner = EngineInner::new();
    inner.ai.global.stupid_soldiers_cheat = true;
    inner.ai.global.green_alert_soldiers = 3;
    inner.ai.global.yellow_alert_soldiers = 4;
    inner.ai.global.red_alert_soldiers = 5;
    inner.ai.global.overall_alert_status = crate::ai::AlertLevel::Yellow;
    inner.ai.global.overall_villain_alert_status = crate::ai::AlertLevel::Red;
    inner.ai.global.saved_random_seed = -73;
    inner.ai.global.current_speech_variant = 2;
    inner
        .ai
        .global
        .forbidden_remarks
        .push(crate::ai::ForbiddenRemark {
            remark: crate::ai::Remark::Warcry,
            flags: crate::ai::RemarkTargetFlags::THIS_GUY.bits(),
            speech_id: 91,
            guy_index: 47,
            bad_guy: true,
            forbidden_till_frame: 1234,
        });
    let mut seek = crate::ai::SeekPoint::from_position(
        &crate::sim_rng::SimulationContext::with_seed(1),
        crate::ai::Position::default(),
    );
    seek.frame_when_full_interest = 99;
    seek.last_calculated_interest = 41;
    seek.locked = true;
    inner.ai.global.seek_points.push(seek);
    inner
        .ai
        .global
        .archery_sectors
        .push(crate::ai::SectorArchery {
            points: vec![crate::ai::PointArchery {
                position: crate::ai::Position::default(),
                direction: 7,
                is_shooting_point: true,
                sector_index: crate::sector::SectorNumber(2),
                owner: None,
            }],
            polygon: Vec::new(),
            layer: 0,
            index_first_shooting_point: Some(crate::sector::ArcheryPointIdx(0)),
            index_last_shooting_point: Some(crate::sector::ArcheryPointIdx(0)),
            num_shooting_points: 1,
            num_owners: 0,
        });
    let engine = Engine {
        inner,
        bootstrap_open: false,
    };

    let state = engine.parity_ai_global_state();
    assert_eq!(state["stupid_soldiers_cheat"], true);
    assert_eq!(state["seek_points"][0]["frame_when_full_interest"], 99);
    assert_eq!(state["seek_points"][0]["last_calculated_interest"], 41);
    assert_eq!(state["seek_points"][0]["locked"], true);
    assert_eq!(state["archery_sectors"][0]["num_owners"], 0);
    assert!(state["archery_sectors"][0]["point_owners"][0].is_null());
    assert_eq!(state["overall_alert_status"], 1);
    assert_eq!(state["overall_villain_alert_status"], 2);
    assert_eq!(state["saved_random_seed"], -73);
    assert_eq!(state["forbidden_remarks"][0]["remark"], 9);
    assert_eq!(state["forbidden_remarks"][0]["flags"], 8);
    assert_eq!(state["forbidden_remarks"][0]["speech_id"], 91);
    assert_eq!(state["forbidden_remarks"][0]["guy_index"], 47);
    assert_eq!(state["forbidden_remarks"][0]["bad_guy"], true);
    assert_eq!(state["forbidden_remarks"][0]["forbidden_till_frame"], 1234);
    assert_eq!(state["current_speech_variant"], 2);
}

#[test]
fn parity_pc_registry_preserves_original_order_not_portrait_order() {
    let mut inner = EngineInner::new();
    let new_pc = || {
        crate::element::Entity::Pc(crate::element::ActorPc {
            element: crate::element::ElementData::default(),
            actor: crate::element::ActorData::default(),
            human: crate::element::HumanData::default(),
            pc: crate::element::PcData::default(),
        })
    };
    let first = inner.add_entity(new_pc());
    let second = inner.add_entity(new_pc());
    inner.world.pc_ids = vec![first, second];
    inner.world.original_pc_registry_ids = vec![second, first];

    let state = Engine {
        inner,
        bootstrap_open: false,
    }
    .parity_pc_registry_state();
    assert_eq!(state[0]["kind"], "pc");
    assert_eq!(state[0]["index"], second.index());
    assert_eq!(state[1]["index"], first.index());
}

#[test]
fn parity_runtime_roots_preserves_mission_stat_and_empty_reference_roots() {
    struct MenuText;
    impl crate::sherwood_stat::MenuTextLookup for MenuText {
        fn get(&self, id: usize) -> String {
            format!("menu-{id}")
        }
    }

    let mut inner = EngineInner::new();
    inner.players.user_locked = true;
    inner.mission_domain.mission_stat.collected_money = 73;
    inner.mission_domain.mission_stat.added_score = 91;
    inner
        .mission_domain
        .mission_stat
        .pc_names
        .push(crate::mission_stat::PcStatName::new(
            "fallback".into(),
            Some(crate::pc_status::SpecialPeasantName::B),
        ));
    let engine = Engine {
        inner,
        bootstrap_open: false,
    };

    let state = engine.parity_engine_runtime_roots_state(&MenuText);
    assert_eq!(state["timer_elements"].as_array().unwrap().len(), 0);
    assert!(state["camera_sequence"].is_null());
    assert!(state["dead_pc"].is_null());
    assert_eq!(state["mission_stat"]["collected_money"], 73);
    assert_eq!(state["mission_stat"]["added_score"], 91);
    assert_eq!(state["mission_stat"]["pc_names"][0], "menu-251");
    assert_eq!(state["user_locked"], true);
    assert_eq!(
        state["selection_before_user_lock"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert!(state["follow_element"].is_null());
}

#[test]
fn parity_world_interactables_preserves_dynamic_patch_and_door_fields() {
    let mut inner = EngineInner::new();
    let patch = crate::patch::Patch {
        active: true,
        locked: true,
        applied: true,
        in_transition: true,
        ..Default::default()
    };
    inner.script_domains.interactables.patches.push(patch);
    let door = crate::gate::Door {
        active: false,
        locked_pc: true,
        locked_npc_villain: true,
        unlockable: true,
        locked_pc_after_patch: true,
        locked_npc_civilian_after_patch: true,
        unlockable_after_patch: true,
        special_authorisation_pc: true,
        authorised_pc_direct: 0x12,
        authorised_pc_indirect: 0x34,
        ..Default::default()
    };
    inner.script_domains.interactables.doors.push(door);
    let engine = Engine {
        inner,
        bootstrap_open: false,
    };

    let state = engine.parity_world_interactables_state(&LevelAssets::new());
    assert_eq!(state["patches"][0]["active"], true);
    assert_eq!(state["patches"][0]["locked"], true);
    assert_eq!(state["patches"][0]["applied"], true);
    assert_eq!(state["patches"][0]["in_transition"], true);
    assert_eq!(
        state["patches"][0]["occupants"].as_array().unwrap().len(),
        0
    );
    assert_eq!(state["doors"][0]["kind"], "door");
    assert_eq!(state["doors"][0]["active"], false);
    assert_eq!(state["doors"][0]["locked_pc"], true);
    assert_eq!(state["doors"][0]["locked_npc_villain"], true);
    assert_eq!(state["doors"][0]["unlockable"], true);
    assert_eq!(state["doors"][0]["locked_pc_after_patch"], true);
    assert_eq!(state["doors"][0]["locked_npc_civilian_after_patch"], true);
    assert_eq!(state["doors"][0]["unlockable_after_patch"], true);
    assert_eq!(state["doors"][0]["special_authorisation_pc"], true);
    assert_eq!(state["doors"][0]["authorised_pc_direct"], 0x12);
    assert_eq!(state["doors"][0]["authorised_pc_indirect"], 0x34);
    assert_eq!(state["sector_doors"].as_array().unwrap().len(), 0);
}

#[test]
fn parity_world_interactables_preserves_lift_runtime_state() {
    let mut inner = EngineInner::new();
    let sector_number = crate::sector::SectorNumber::new(47);
    let level = std::sync::Arc::make_mut(&mut inner.world.fast_grid_mut().level);
    level.sector_number_map.insert(sector_number, 0);
    level.sectors.push(crate::fast_find_grid::GridSector {
        points: Vec::new(),
        bounding_box: crate::coordinates::MapBBox::new(),
        sector_type: crate::sector::SectorType::LIFT,
        layer: 0,
        sector_number,
        door_index: None,
        lift_type: Some(crate::sector::LiftType::Ladder),
        lift_direction: 0,
        force_crouched: false,
        building_index: None,
        low_exit_point: None,
        high_exit_point: None,
        lowest_door_index: None,
        jump_line_indices: Vec::new(),
        gate_indices: Vec::new(),
        underlying_sector: None,
    });
    inner.world.fast_grid_mut().lift_state.insert(
        0,
        crate::fast_find_grid::LiftRuntimeState {
            occupants_pc: 2,
            occupants: 3,
            occupied_upwards: true,
            occupied_downwards: false,
            wait_time: 71,
        },
    );

    let state = Engine {
        inner,
        bootstrap_open: false,
    }
    .parity_world_interactables_state(&LevelAssets::new());
    assert_eq!(state["lifts"][0]["sector"], 47);
    assert_eq!(state["lifts"][0]["occupants_pc"], 2);
    assert_eq!(state["lifts"][0]["occupants"], 3);
    assert_eq!(state["lifts"][0]["occupied_upwards"], true);
    assert_eq!(state["lifts"][0]["occupied_downwards"], false);
    assert_eq!(state["lifts"][0]["wait_time"], 71);
}

#[test]
fn parity_world_interactables_preserves_ordered_building_and_zone_state() {
    let mut inner = EngineInner::new();
    let new_pc = || {
        crate::element::Entity::Pc(crate::element::ActorPc {
            element: crate::element::ElementData::default(),
            actor: crate::element::ActorData::default(),
            human: crate::element::HumanData::default(),
            pc: crate::element::PcData::default(),
        })
    };
    let first = inner.add_entity(new_pc());
    let second = inner.add_entity(new_pc());
    inner.script_domains.buildings.occupants.push(vec![
        crate::natives::ScriptHandleCodec::actor_handle(second),
        crate::natives::ScriptHandleCodec::actor_handle(first),
    ]);
    inner.script_domains.buildings.arrow_reserves.push(true);

    inner
        .script_domains
        .zones
        .scripts
        .push(crate::sector::ScriptSectorData {
            sector_index: crate::fast_find_grid::SectorIndex::new(0),
            transformed_to_apex: true,
            max_throwing_apex_height: 12.5,
            occupant_indices: vec![first, second],
            ..Default::default()
        });
    let level = std::sync::Arc::make_mut(&mut inner.world.fast_grid_mut().level);
    level.sectors.push(crate::fast_find_grid::GridSector {
        points: Vec::new(),
        bounding_box: crate::coordinates::MapBBox::new(),
        sector_type: crate::sector::SectorType::SCRIPT,
        layer: 0,
        sector_number: crate::sector::SectorNumber::new(47),
        door_index: None,
        lift_type: None,
        lift_direction: 0,
        force_crouched: false,
        building_index: None,
        low_exit_point: None,
        high_exit_point: None,
        lowest_door_index: None,
        jump_line_indices: Vec::new(),
        gate_indices: Vec::new(),
        underlying_sector: None,
    });
    inner
        .world
        .fast_grid_mut()
        .or_sector_type_overlay(0, crate::sector::SectorType::APEX);
    let mut assets = LevelAssets::new();
    std::sync::Arc::make_mut(&mut assets.scripts.zone_grid_indices).push(0);

    let state = Engine {
        inner,
        bootstrap_open: false,
    }
    .parity_world_interactables_state(&assets);
    assert_eq!(
        state["buildings"][0]["occupants"][0]["index"],
        second.index()
    );
    assert_eq!(
        state["buildings"][0]["occupants"][1]["index"],
        first.index()
    );
    assert_eq!(state["buildings"][0]["arrow_reserve"], true);
    assert_eq!(
        state["script_zones"][0]["occupants"][0]["index"],
        first.index()
    );
    assert_eq!(
        state["script_zones"][0]["occupants"][1]["index"],
        second.index()
    );
    assert_eq!(state["script_zones"][0]["transformed_to_apex"], true);
    assert_eq!(
        state["script_zones"][0]["max_apex_height"]["bits"],
        12.5f32.to_bits()
    );
}

#[test]
fn parity_repulsive_points_preserves_serialized_fields_order_and_next_id() {
    let mut inner = EngineInner::new();
    inner.world.original_repulsive_point_counter = 42;
    let mut first = crate::ai::RepulsivePoint::new(
        17,
        crate::ai::Position {
            x: 1.25,
            y: -2.5,
            sector: None,
            level: 3,
        },
        4.0,
        5.0,
        1 | 4 | 8,
    );
    first.concave = true;
    first.limit_left = crate::coordinates::MapVec::new(6.0, 7.0);
    first.limit_right = crate::coordinates::MapVec::new(8.0, 9.0);
    inner.ai.global.repulsive_points.push(first);
    inner
        .ai
        .global
        .repulsive_points
        .push(crate::ai::RepulsivePoint::new(
            18,
            crate::ai::Position {
                level: 5,
                ..Default::default()
            },
            10.0,
            11.0,
            2,
        ));
    let engine = Engine {
        inner,
        bootstrap_open: false,
    };

    let state = engine.parity_repulsive_points_state();
    assert_eq!(state["next_id"], 42);
    assert_eq!(state["points"][0]["id"], 17);
    assert_eq!(state["points"][1]["id"], 18);
    assert_eq!(
        state["points"][0]["position"]["x"]["bits"],
        1.25f32.to_bits()
    );
    assert_eq!(
        state["points"][0]["position"]["y"]["bits"],
        (-2.5f32).to_bits()
    );
    assert_eq!(state["points"][0]["concave"], true);
    assert_eq!(
        state["points"][0]["limit_left"]["x"]["bits"],
        6.0f32.to_bits()
    );
    assert_eq!(
        state["points"][0]["limit_right"]["y"]["bits"],
        9.0f32.to_bits()
    );
    assert_eq!(state["points"][0]["radius"]["bits"], 4.0f32.to_bits());
    assert_eq!(
        state["points"][0]["action_radius"]["bits"],
        9.0f32.to_bits()
    );
    assert_eq!(state["points"][0]["affects_pcs"], true);
    assert_eq!(state["points"][0]["affects_soldiers"], false);
    assert_eq!(state["points"][0]["affects_civilians"], true);
    assert_eq!(state["points"][0]["affects_animals"], true);
    assert_eq!(state["points"][0]["layer"], 3);
}

#[test]
fn parity_titbits_preserves_serialized_manager_and_live_entry_fields() {
    let mut inner = EngineInner::new();
    let id = inner.feedback.titbit_manager.add_titbit(
        crate::coordinates::WorldPoint3D::new(1.5, -2.0, 3.25),
        4,
        crate::titbit::TitbitKind::DangerPoint,
        crate::titbit::ElementHandle::INVALID,
        7,
        crate::titbit::ElementHandle::INVALID,
        false,
        crate::titbit::INVALID_ID,
        true,
        None,
        None,
    );
    let titbit = &mut inner.feedback.titbit_manager.titbits_mut()[0];
    titbit.sprite_row = 8;
    titbit.sprite_frame = 9;
    titbit.frame_count = 10;
    titbit.display_order = 11.5;
    titbit.blinking = true;
    let engine = Engine {
        inner,
        bootstrap_open: false,
    };

    let state = engine.parity_titbit_manager_state();
    assert_eq!(state["current_id"], 1);
    assert_eq!(state["titbits"][0]["kind"], 10);
    assert_eq!(state["titbits"][0]["phase"], 7);
    assert_eq!(state["titbits"][0]["sprite_row"], 8);
    assert_eq!(state["titbits"][0]["sprite_frame"], 9);
    assert_eq!(state["titbits"][0]["frame_count"], 10);
    assert_eq!(
        state["titbits"][0]["display_order"]["bits"],
        11.5f32.to_bits()
    );
    assert_eq!(state["titbits"][0]["layer"], 4);
    assert_eq!(state["titbits"][0]["blinking"], true);
    assert_eq!(
        state["titbits"][0]["id"],
        id.expect("titbit allocation succeeds").get()
    );
    assert!(state["titbits"][0]["element_supplier"].is_null());
    assert!(state["titbits"][0]["element_manager"].is_null());
    assert_eq!(
        state["titbits"][0]["position"]["x"]["bits"],
        1.5f32.to_bits()
    );
}

#[test]
fn diagnostic_snapshot_omits_only_nonserializable_original_rng_replay() {
    let mut inner = EngineInner::new();
    inner.control.rng = SimulationRng::with_original_replay(vec![11, 22]);
    let engine = Engine {
        inner,
        bootstrap_open: false,
    };

    assert!(serde_json::to_value(&engine).is_err());
    let diagnostic = engine.diagnostic_snapshot_without_original_rng_replay();

    assert_eq!(engine.original_rng_replay_cursor(), Some(0));
    assert_eq!(diagnostic.original_rng_replay_cursor(), None);
    serde_json::to_value(&diagnostic).expect("diagnostic engine must serialize");
}

#[test]
fn legacy_additional_arrow_refreshes_advance_real_sprite_state() {
    let mut inner = EngineInner::new();
    inner.control.rng = SimulationRng::with_original_replay(vec![11, 22, 33, 44]);
    inner.control.arrow_refresh_pending = true;
    let projectile = crate::element::ElementProjectile {
        element: {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::ObjectProjectile;
            initial_element.active = true;
            initial_element
        },
        object: crate::element::ObjectData {
            object_type: crate::element::ObjectType::Arrow,
            ..Default::default()
        },
        projectile: crate::element::ProjectileData {
            falling: true,
            falling_direction: 8,
            trajectory: vec![crate::element::TrajectoryPoint {
                position: crate::coordinates::WorldPoint3D::new(1.0, 0.0, 0.0),
                time: 1,
            }],
            ..Default::default()
        },
    };
    let id = inner.add_entity(crate::element::Entity::Projectile(projectile));
    let mut engine = Engine {
        inner,
        bootstrap_open: false,
    };

    assert_eq!(
        engine
            .parity_replay_setup()
            .pending_falling_arrow_refresh_draw_count(),
        1
    );
    engine
        .parity_replay_setup()
        .replay_legacy_additional_arrow_refreshes(3);

    assert_eq!(engine.original_rng_replay_cursor(), Some(3));
    assert!(engine.inner.control.arrow_refresh_pending);
    let crate::element::Entity::Projectile(arrow) = engine.get_entity(id).unwrap() else {
        panic!("test arrow changed entity kind");
    };
    assert_eq!(arrow.projectile.falling_direction, 2);
    assert_eq!(arrow.element.sprite.current_row, 4);
    assert!((3..=5).contains(&arrow.element.sprite.current_frame));
}

#[test]
fn campaign_selection_transfers_one_rng_sequence_to_mission_construction() {
    let mut profiles = crate::profiles::ProfileManager::default();
    profiles.missions.push(crate::profiles::MissionProfile {
        id: 0,
        location: crate::profiles::MissionLocation::Sherwood,
        life_time: 100,
        max_ransom: 200_000,
        max_gang_size: u16::MAX,
        ..Default::default()
    });
    for id in 1..=2 {
        profiles.missions.push(crate::profiles::MissionProfile {
            id,
            mission_type: crate::profiles::MissionType::Rescue,
            location: crate::profiles::MissionLocation::York,
            life_time: 100,
            access_probability: 50,
            max_ransom: 200_000,
            max_gang_size: u16::MAX,
            ..Default::default()
        });
    }

    let mut campaign = Campaign::default();
    for profile_idx in 0..3 {
        campaign.missions.push(crate::mission::Mission {
            profile_idx: Some(profile_idx),
            ..Default::default()
        });
    }
    campaign.accessible_mission_indices = vec![1, 2];

    let seed = 0xCA11_AB1E;
    let config = SimConfig::default();
    let reference_context = crate::sim_rng::SimulationContext::with_seed_and_config(seed, config);
    let mut reference_campaign = campaign.clone();
    let expected_mission = reference_campaign.determine_next_mission(&reference_context, &profiles);
    let expected_next_seed = reference_context.seed();
    let expected_next_draw = crate::sim_rng::u32(
        &reference_context,
        crate::sim_rng::RngSite::TitbitUpdate,
        ..,
    );

    let (_campaign, mission, next_seed, next_config) =
        Engine::select_next_mission(campaign, &profiles, seed, config);
    let mission_context =
        crate::sim_rng::SimulationContext::with_seed_and_config(next_seed, config);
    let actual_next_draw =
        crate::sim_rng::u32(&mission_context, crate::sim_rng::RngSite::TitbitUpdate, ..);

    assert_eq!(mission, expected_mission);
    assert_eq!(next_seed, expected_next_seed);
    assert_eq!(next_config, config);
    assert_eq!(actual_next_draw, expected_next_draw);
}

fn scripted_snapshot_fixture() -> (
    Engine,
    LevelAssets,
    std::sync::Arc<crate::script_manager::ScriptProgram>,
    crate::sequence::SequenceId,
) {
    let scb = crate::scb::ScbFile {
        version: crate::scb::SCB_VERSION,
        classes: vec![crate::scb::ClassEntry {
            source_file: "snapshot_attachment_test.scs".to_owned(),
            class_name: "StartUp".to_owned(),
            size_of_member_variables: 0,
            member_variables: Vec::new(),
            functions: Vec::new(),
            quads: Vec::new(),
        }],
    };
    let program = std::sync::Arc::new(
        crate::script_manager::ScriptProgram::from_scb(scb).expect("prepare test bytecode"),
    );
    let script_name = "snapshot_attachment_test".to_owned();
    let script = crate::engine::MissionScript::from_program(script_name.clone(), program.clone())
        .expect("minimal StartUp script");

    let mut assets = LevelAssets::new();
    assets.scripts.mission_name = Some(script_name.clone());
    std::sync::Arc::make_mut(&mut assets.scripts.mission_programs)
        .insert(script_name, program.clone());

    let mut inner = EngineInner::new();
    inner.scripts.install_mission(script);
    inner.scripts.attach_native_capabilities(&assets);
    inner
        .scripts
        .mission
        .as_mut()
        .expect("fixture mission script")
        .script_effects
        .emit_engine(crate::natives::EngineCommand::UpdateInformationBars);
    {
        let effects = &mut inner
            .scripts
            .mission
            .as_mut()
            .expect("fixture mission script")
            .script_effects;
        effects.emit_sound(crate::natives::SoundCommand::SuspendAll);
        effects.emit_engine(crate::natives::EngineCommand::ChooseVictoryDefeatText { id: 17 });
        effects.emit_barrier(crate::natives::DeferredCommand::FreezeAll { freeze: true });
    }

    let mut sequence = crate::sequence::Sequence::new();
    sequence.append_element(crate::sequence::SequenceElement::new(
        1,
        crate::element::Command::Generic,
        None,
    ));
    let sequence_id = inner.orders.sequence_manager.launch_sequence(sequence);

    (
        Engine {
            inner,
            bootstrap_open: false,
        },
        assets,
        program,
        sequence_id,
    )
}

fn decoded_engine(engine: &Engine) -> Engine {
    serde_json::from_str(&serde_json::to_string(engine).expect("serialize engine snapshot"))
        .expect("decode engine snapshot")
}

#[test]
fn persisted_capture_rejects_nonfinite_state_and_original_parity_rng() {
    let (mut source, _, _, _) = scripted_snapshot_fixture();
    source.inner.feedback.cutscene_camera.zoom_factor = f32::NAN;
    assert!(source.capture_persisted_state().is_err());
    let serialized = serde_json::to_vec(&source).unwrap();
    assert!(serde_json::from_slice::<Engine>(&serialized).is_err());

    source.inner.feedback.cutscene_camera.zoom_factor = 1.0;
    source.inner.control.rng = super::super::SimulationRng::with_original_replay(vec![1]);
    assert!(source.capture_persisted_state().is_err());
    assert!(serde_json::to_vec(&source).is_err());
}

#[test]
fn persisted_projection_matches_disk_reconstruction_and_reattaches_script_resources() {
    let (mut source, assets, program, _) = scripted_snapshot_fixture();
    source
        .inner
        .ai
        .global
        .primary_target_multiplicity_initialized = true;
    let raw = source.clone();
    let projection = source.capture_persisted_state().unwrap();
    assert_eq!(
        serde_json::to_vec(&projection).unwrap(),
        serde_json::to_vec(&source).unwrap()
    );
    let projected = Engine::from_persisted_state(projection);
    let disk = decoded_engine(&source);
    assert_eq!(
        projected.encode_native_snapshot(),
        disk.encode_native_snapshot()
    );
    assert_eq!(
        crate::replay::state_hash(&projected),
        crate::replay::state_hash(&disk)
    );
    assert!(
        !projected
            .inner
            .ai
            .global
            .primary_target_multiplicity_initialized
    );
    assert!(raw.inner.ai.global.primary_target_multiplicity_initialized);
    assert!(std::sync::Arc::ptr_eq(
        &raw.inner.scripts.mission.as_ref().unwrap().manager.program,
        &program
    ));
    assert!(!std::sync::Arc::ptr_eq(
        &projected
            .inner
            .scripts
            .mission
            .as_ref()
            .unwrap()
            .manager
            .program,
        &program
    ));
    let mut projected_display = super::super::HostDisplayState::default();
    let mut disk_display = super::super::HostDisplayState::default();
    let projected =
        Engine::restore_from_snapshot(&mut projected_display, projected, &assets).unwrap();
    let disk = Engine::restore_from_snapshot(&mut disk_display, disk, &assets).unwrap();
    projected.inner.scripts.assert_native_attachments_ready();
    assert!(std::sync::Arc::ptr_eq(
        &projected
            .inner
            .scripts
            .mission
            .as_ref()
            .unwrap()
            .manager
            .program,
        &program
    ));
    assert_eq!(
        projected.encode_native_snapshot(),
        disk.encode_native_snapshot()
    );
    assert_eq!(
        crate::replay::state_hash(&projected),
        crate::replay::state_hash(&disk)
    );
}

#[test]
fn failed_construction_returns_the_same_campaign_allocation() {
    let mut profiles = crate::profiles::ProfileManager::default();
    profiles
        .missions
        .push(crate::profiles::MissionProfile::default());
    profiles.soldiers.push(crate::profiles::SoldierProfile {
        filename: "missing-construction-test-sprite".to_owned(),
        profile_name: "missing-construction-test-profile".to_owned(),
        ..crate::profiles::SoldierProfile::default()
    });

    let mut campaign = crate::campaign::Campaign::default();
    campaign.missions.push(crate::mission::Mission {
        profile_idx: Some(0),
        ..crate::mission::Mission::default()
    });
    campaign.current_mission_idx = Some(0);
    campaign.missions.reserve_exact(257);
    assert!(!campaign.missions.is_empty());
    let missions = campaign.missions.as_ptr();
    let mission_capacity = campaign.missions.capacity();

    let mut assets = LevelAssets::new();
    assets.profile_manager = std::sync::Arc::new(profiles);
    let mut loaded = crate::level_data::LoadedLevel::empty_for_test();
    loaded.mission.soldiers.push(crate::level_data::RawSoldier {
        position_x: 0,
        position_y: 0,
        direction: 0,
        action: 0,
        obstacle_index: 0,
        sector: 0,
        layer: 0,
        material: 0,
        profile_number: 0,
        profile_id: None,
        allegiance: None,
        command_interface: crate::human_control::CommandInterface::None,
        mission_role: crate::human_control::MissionRole::Combatant,
        combat_stance: crate::human_control::CombatStance::Aggressive,
        revealed: false,
        tower_guard: false,
        company_number: 0,
        drunk_level: 0,
        money: 0,
        subordinate_ids: Vec::new(),
        path_id: 0,
        alert_path_id: 0,
        script_class: None,
    });

    let result = Engine::new_preserving_campaign(EngineArgs {
        campaign,
        level: LevelLoadArgs {
            assets: &mut assets,
            level_directory: "",
            progress: &mut |_| {},
            loaded,
            bg_pixel_dims: (0.0, 0.0),
        },
        ground_mark_sprite: None,
        titbit_row_frame_counts: Vec::new(),
        rng_seed: 0,
        original_rng_replay: None,
        sim_config: SimConfig::default(),
    });

    let (error, returned) = match result {
        Ok(_) => panic!("missing sprite must fail construction"),
        Err(failure) => failure,
    };
    assert!(matches!(error, EngineError::ProfileSpriteLoadFailed { .. }));
    assert_eq!(returned.missions.as_ptr(), missions);
    assert_eq!(returned.missions.capacity(), mission_capacity);
    assert_eq!(returned.current_mission_idx, Some(0));
}

/// Serialized camera state belongs to the snapshot; the previous live
/// engine is not an attachment source. Only immutable `LevelAssets` are
/// admitted by the preparation path.
#[test]
fn restore_uses_snapshot_camera_state_not_previous_engine() {
    let mut source_inner = EngineInner::new();

    source_inner.feedback.cutscene_camera.level_size =
        crate::coordinates::MapSize::new(1234.0, 5678.0);

    let source = Engine {
        inner: source_inner,
        bootstrap_open: false,
    };

    let json = serde_json::to_string(&source).expect("serialize");
    let decoded: Engine = serde_json::from_str(&json).expect("deserialize");

    let mut display = crate::engine::HostDisplayState::default();
    let restored = Engine::restore_from_snapshot(&mut display, decoded, &LevelAssets::new())
        .expect("restore compatible snapshot");

    assert_eq!(
        restored.inner.feedback.cutscene_camera.level_size,
        crate::coordinates::MapSize::new(1234.0, 5678.0)
    );
}

#[test]
fn try_restore_rejects_mismatched_runtime_lengths_without_mutating_live_engine() {
    let mut live_inner = EngineInner::new();
    live_inner.feedback.cutscene_camera.level_size =
        crate::coordinates::MapSize::new(1234.0, 5678.0);
    let live = Engine {
        inner: live_inner,
        bootstrap_open: false,
    };

    let mut malformed_inner = EngineInner::new();
    malformed_inner.world.fast_grid_mut().line_active.push(true);
    let malformed = Engine {
        inner: malformed_inner,
        bootstrap_open: false,
    };

    let mut display = crate::engine::HostDisplayState::default();
    let error = Engine::restore_from_snapshot(&mut display, malformed, &LevelAssets::new())
        .err()
        .expect("malformed snapshot must be rejected");
    assert_eq!(
        error,
        SnapshotRestoreError::FastGridLengthMismatch {
            component: SnapshotGridComponent::Lines,
            snapshot_len: 1,
            level_len: 0,
        }
    );
    assert_eq!(
        live.inner.feedback.cutscene_camera.level_size,
        crate::coordinates::MapSize::new(1234.0, 5678.0),
        "validation must happen before replacing the live engine"
    );
}

#[test]
fn try_restore_rejects_world_parallel_mismatch_before_mutating_live_engine() {
    let live = Engine {
        inner: EngineInner::new(),
        bootstrap_open: false,
    };
    let mut malformed_inner = EngineInner::new();
    malformed_inner
        .script_domains
        .zones
        .scripts
        .push(crate::sector::ScriptSectorData::new());
    let malformed = Engine {
        inner: malformed_inner,
        bootstrap_open: false,
    };

    let mut display = crate::engine::HostDisplayState::default();
    let error = Engine::restore_from_snapshot(&mut display, malformed, &LevelAssets::new())
        .err()
        .expect("malformed snapshot must be rejected");
    assert_eq!(
        error,
        SnapshotRestoreError::WorldInvariantViolation {
            detail: "script-zone runtime length 1 does not match level zone-index length 0"
                .to_owned(),
        }
    );
    assert!(live.inner.script_domains.zones.scripts.is_empty());
}

#[test]
fn network_adoption_is_fully_attached_and_preserves_hash_and_script_queue() {
    let (source, assets, program, sequence_id) = scripted_snapshot_fixture();
    let source_hash = crate::replay::state_hash(&source);
    let snapshot = decoded_engine(&source);
    let live =
        Engine::adopt_authoritative_snapshot(snapshot, &assets).expect("adopt compatible snapshot");

    assert_eq!(crate::replay::state_hash(&live), source_hash);
    live.inner.scripts.assert_native_attachments_ready();
    let script = live.inner.scripts.mission.as_ref().expect("adopted script");
    assert!(std::sync::Arc::ptr_eq(&script.manager.program, &program));
    assert!(std::sync::Arc::ptr_eq(
        &script.bindings.profile_manager,
        &assets.profile_manager
    ));
    assert!(matches!(
        script.script_effects.ordered.as_slices(),
        (
            [
                crate::natives::ScriptEffect::Presentation(
                    crate::natives::EngineCommand::UpdateInformationBars
                ),
                crate::natives::ScriptEffect::ExternalSound(
                    crate::natives::SoundCommand::SuspendAll
                ),
                crate::natives::ScriptEffect::Simulation(crate::natives::SimulationEffect::Engine(
                    crate::natives::EngineCommand::ChooseVictoryDefeatText { id: 17 }
                )),
                crate::natives::ScriptEffect::Simulation(
                    crate::natives::SimulationEffect::Deferred(
                        crate::natives::DeferredCommand::FreezeAll { freeze: true }
                    )
                )
            ],
            []
        )
    ));
    assert_eq!(
        live.inner
            .orders
            .sequence_manager
            .get_sequence(sequence_id)
            .map(|sequence| sequence.id),
        Some(sequence_id),
        "serialized sequences must be addressable after lookup indices rebuild"
    );
}

#[test]
fn save_restore_attaches_before_fixups_and_appends_save_only_hud_repair() {
    let (mut source, assets, program, _) = scripted_snapshot_fixture();
    source.inner.feedback.cutscene_camera.level_size =
        crate::coordinates::MapSize::new(4096.0, 4096.0);
    source
        .inner
        .feedback
        .cutscene_camera
        .display
        .background_transform
        .zoom_to_up = true;
    source.inner.feedback.cutscene_camera.zoom_init_done = true;
    let queued_engine_commands = source
        .inner
        .scripts
        .mission
        .as_ref()
        .expect("fixture script")
        .script_effects
        .engine_commands()
        .len();
    let snapshot = decoded_engine(&source);
    let mut display = super::super::HostDisplayState::default();

    let observed_fixups_before_hud_repair = std::cell::Cell::new(false);
    let mut live =
        Engine::restore_from_snapshot_with_observer(&mut display, snapshot, &assets, |inner| {
            observed_fixups_before_hud_repair.set(true);
            assert_eq!(
                inner.orders.messenger.count(),
                3,
                "zoom-end, stature, and select-action must already be queued"
            );
            assert_eq!(
                inner
                    .scripts
                    .mission
                    .as_ref()
                    .expect("restored script during fixup observation")
                    .script_effects
                    .engine_commands()
                    .len(),
                queued_engine_commands,
                "save-only HUD repair must not be queued until engine fixups finish"
            );
        })
        .expect("restore compatible save snapshot");
    assert!(observed_fixups_before_hud_repair.get());

    live.inner.scripts.assert_native_attachments_ready();
    let script = live
        .inner
        .scripts
        .mission
        .as_ref()
        .expect("restored script");
    assert!(std::sync::Arc::ptr_eq(&script.manager.program, &program));
    assert_eq!(
        script.script_effects.engine_commands().len(),
        queued_engine_commands + 1,
        "saved queue must survive and save-load must append one HUD repair"
    );
    let messages = live.inner.orders.messenger.drain();
    assert_eq!(messages.len(), 3);
    assert_eq!(
        messages[0].msg_type,
        crate::messenger::MessageType::Simple(crate::messenger::SimpleMessage::ZoomUpEnd)
    );
    assert_eq!(
        messages[1].msg_type,
        crate::messenger::MessageType::Simple(crate::messenger::SimpleMessage::Stature)
    );
    assert!(matches!(
        messages[2].msg_type,
        crate::messenger::MessageType::Pc(crate::messenger::PcMessage::SelectAction, _)
    ));
    assert_eq!(display.display_op, crate::engine::DisplayOpCode::Redraw);
}

#[test]
fn failed_attachment_preflight_does_not_mutate_live_engine() {
    let (source, mut assets, _, _) = scripted_snapshot_fixture();
    let snapshot = decoded_engine(&source);
    assets.scripts.mission_programs = std::sync::Arc::new(std::collections::BTreeMap::new());

    let mut live_inner = EngineInner::new();
    live_inner.control.frame_counter = 77;
    let live = Engine {
        inner: live_inner,
        bootstrap_open: false,
    };
    let before_hash = crate::replay::state_hash(&live);

    let error = Engine::adopt_authoritative_snapshot(snapshot, &assets)
        .err()
        .expect("snapshot with missing attachment must be rejected");
    assert!(matches!(
        error,
        SnapshotRestoreError::AttachmentFailure { ref detail }
            if detail.contains("missing mission script program 'snapshot_attachment_test'")
    ));
    assert_eq!(crate::replay::state_hash(&live), before_hash);
    assert_eq!(live.frame_counter(), 77);
}

#[test]
fn adoption_rejects_wrong_loaded_mission_identity() {
    let (source, mut assets, _, _) = scripted_snapshot_fixture();
    let snapshot = decoded_engine(&source);
    assets.scripts.mission_name = Some("different_mission".to_owned());
    let live = Engine {
        inner: EngineInner::new(),
        bootstrap_open: false,
    };

    let error = Engine::adopt_authoritative_snapshot(snapshot, &assets)
        .err()
        .expect("snapshot for wrong mission must be rejected");
    assert!(matches!(
        error,
        SnapshotRestoreError::AttachmentFailure { ref detail }
            if detail.contains("does not match loaded mission script 'different_mission'")
    ));
    assert!(live.inner.scripts.mission.is_none());
}

#[test]
fn adoption_rejects_mobile_count_from_level_assets_atomically() {
    let mut assets = LevelAssets::new();
    assets.entities.mobile_element_count = 1;
    let snapshot = Engine {
        inner: EngineInner::new(),
        bootstrap_open: false,
    };
    let mut live_inner = EngineInner::new();
    live_inner.control.frame_counter = 91;
    let live = Engine {
        inner: live_inner,
        bootstrap_open: false,
    };
    let before_hash = crate::replay::state_hash(&live);

    let error = Engine::adopt_authoritative_snapshot(snapshot, &assets)
        .err()
        .expect("snapshot with wrong mobile count must be rejected");
    assert_eq!(
        error,
        SnapshotRestoreError::WorldInvariantViolation {
            detail: "snapshot mobile-element count 0 does not match loaded level count 1"
                .to_owned()
        }
    );
    assert_eq!(crate::replay::state_hash(&live), before_hash);
    assert_eq!(live.frame_counter(), 91);
}

#[test]
fn adoption_rejects_malformed_fog_grid_atomically() {
    let assets = LevelAssets::new();
    let mut malformed_inner = EngineInner::new();
    malformed_inner.set_level_size(192.0, 144.0);
    malformed_inner
        .players
        .fog_of_war
        .corrupt_visible_region_for_test();
    let snapshot = Engine {
        inner: malformed_inner,
        bootstrap_open: false,
    };

    let mut live_inner = EngineInner::new();
    live_inner.control.frame_counter = 92;
    let live = Engine {
        inner: live_inner,
        bootstrap_open: false,
    };
    let before_hash = crate::replay::state_hash(&live);

    let error = Engine::adopt_authoritative_snapshot(snapshot, &assets)
        .err()
        .expect("malformed fog grid must be rejected");
    assert_eq!(
        error,
        SnapshotRestoreError::FogOfWarInvariantViolation {
            detail: "uninitialized fog state must be the exact empty 0x0 state".to_owned(),
        }
    );
    assert_eq!(crate::replay::state_hash(&live), before_hash);
    assert_eq!(live.frame_counter(), 92);
}
