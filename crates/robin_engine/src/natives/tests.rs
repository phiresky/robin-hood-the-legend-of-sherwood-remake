//! Unit tests for the script native dispatch.

use super::*;
use crate::interp::*;
use crate::vm::Instruction::*;

const TMP0: u16 = 0xC000;
const TMP4: u16 = 0xC004;
const TMP8: u16 = 0xC008;
const TMP12: u16 = 0xC00C;
const TMP16: u16 = 0xC010;

#[derive(Default)]
struct TestQueryViews<'a> {
    sequence_manager: Option<&'a mut crate::sequence::SequenceManager>,
    selected_pcs: Option<&'a mut Vec<crate::element::EntityId>>,
    sound_sources: Option<&'a mut crate::sound_source::SoundSourceManager>,
    weather: Option<&'a crate::engine::WeatherState>,
    frame_counter: Option<&'a u32>,
}

impl<'a> TestQueryViews<'a> {
    fn new(
        sequence_manager: &'a mut crate::sequence::SequenceManager,
        selected_pcs: &'a mut Vec<crate::element::EntityId>,
        sound_sources: &'a mut crate::sound_source::SoundSourceManager,
        weather: &'a crate::engine::WeatherState,
        frame_counter: &'a u32,
    ) -> Self {
        Self {
            sequence_manager: Some(sequence_manager),
            selected_pcs: Some(selected_pcs),
            sound_sources: Some(sound_sources),
            weather: Some(weather),
            frame_counter: Some(frame_counter),
        }
    }

    fn attach_to(
        self,
        capabilities: NativeSessionCapabilities<'a>,
    ) -> NativeSessionCapabilities<'a> {
        match (
            self.sequence_manager,
            self.selected_pcs,
            self.sound_sources,
            self.weather,
            self.frame_counter,
        ) {
            (Some(sequences), Some(selected), Some(sounds), Some(weather), Some(frame)) => {
                capabilities
                    .with_world_views(&[], &[], &[])
                    .with_queries(sequences, selected, sounds, weather, frame)
            }
            (None, None, None, None, None) => capabilities,
            _ => panic!("test query fixture must supply either every query owner or none"),
        }
    }
}

/// Helper: build a program that pushes constants, calls a native, and returns the result.
fn call_native_return(index: u32, args: &[i32]) -> Vec<crate::vm::Instruction> {
    let temps = [TMP0, TMP4, TMP8, TMP12, TMP16];
    let temp_count = (args.len() + 1) as u16; // +1 for the return slot
    let ret_slot = temps[args.len()]; // first unused temp

    let mut prog = vec![BeginFunction {
        volatile_count: 0,
        temp_count,
    }];
    for (i, &val) in args.iter().enumerate() {
        prog.push(Aff0IConstant {
            dst: temps[i],
            constant: val,
        });
    }
    for &temp in &temps[..args.len()] {
        prog.push(NativeParam { sym: temp });
    }
    prog.push(NativeCall { index });
    prog.push(Aff1NativeGetReturn { sym: ret_slot });
    prog.push(ReturnVal { sym: ret_slot });
    prog
}

fn run_native(index: u32, args: &[i32]) -> StopReason {
    let prog = call_native_return(index, args);
    let host = BoundScriptEffects::new();
    let mut vm = Vm::new().with_host(Box::new(host));
    vm.run(&prog)
}

fn seed_zone(host: &mut BoundScriptEffects, zone_idx: usize, handles: &[i32]) {
    host.script_domains
        .zones
        .scripts
        .resize_with(zone_idx + 1, crate::sector::ScriptSectorData::new);
    host.script_domains.zones.scripts[zone_idx].occupant_indices = handles
        .iter()
        .map(|handle| {
            crate::entity_id::EntityId::Civilian(crate::entity_id::CivilianId(
                ScriptHandleCodec::actor_handle_index(*handle).expect("actor handle") as u32,
            ))
        })
        .collect();
}

fn call_host_native(
    host: &mut BoundScriptEffects,
    native: NativeFn,
    stack: &mut NativeStack,
) -> i32 {
    HostFunctions::call(host, native as u32, stack).expect_return("non-nested native test")
}

fn call_host_native_with_queries(
    host: &mut BoundScriptEffects,
    native: NativeFn,
    stack: &mut NativeStack,
    queries: TestQueryViews<'_>,
) -> i32 {
    let sim = crate::sim_rng::test_context();
    let capabilities = queries.attach_to(NativeSessionCapabilities::new(
        &sim,
        &mut host.entities,
        &mut host.ai_global,
        &mut host.fast_grid,
    ));
    let mut context = NativeContext::with_bindings(
        &mut host.host,
        &mut host.state,
        &mut host.script_domains,
        AttachedScriptBindings::empty_ref(),
        &capabilities,
    );
    <NativeContext<'_, '_> as HostFunctions>::call(&mut context, native as u32, stack)
        .expect_return("non-nested native query test")
}

fn call_bound_host_native(
    host: &mut BoundScriptEffects,
    bindings: &AttachedScriptBindings,
    native: NativeFn,
    stack: &mut NativeStack,
) -> i32 {
    let sim = crate::sim_rng::test_context();
    let capabilities = NativeSessionCapabilities::new(
        &sim,
        &mut host.entities,
        &mut host.ai_global,
        &mut host.fast_grid,
    );
    let mut context = NativeContext::with_bindings(
        &mut host.host,
        &mut host.state,
        &mut host.script_domains,
        bindings,
        &capabilities,
    );
    <NativeContext<'_, '_> as HostFunctions>::call(&mut context, native as u32, stack)
        .expect_return("non-nested native test")
}

fn with_campaign_context<R>(
    host: &mut ScriptEffects,
    bindings: &AttachedScriptBindings,
    campaign: &mut crate::campaign::Campaign,
    mission_stat: &mut crate::mission_stat::MissionStat,
    f: impl FnOnce(&mut NativeContext<'_, '_>) -> R,
) -> R {
    let mut entities = crate::entities::Entities::new();
    let mut ai_global = crate::ai::AiGlobalState::default();
    let mut fast_grid = crate::fast_find_grid::FastFindGrid::default();
    let sim = crate::sim_rng::test_context();
    let capabilities =
        NativeSessionCapabilities::new(&sim, &mut entities, &mut ai_global, &mut fast_grid)
            .with_campaign(campaign, mission_stat);
    let mut state = ScriptState::default();
    let mut script_domains = crate::engine::ScriptDomains::default();
    let mut context = NativeContext::with_bindings(
        host,
        &mut state,
        &mut script_domains,
        bindings,
        &capabilities,
    );
    f(&mut context)
}

fn with_bound_campaign_context<R>(
    host: &mut BoundScriptEffects,
    bindings: &AttachedScriptBindings,
    campaign: &mut crate::campaign::Campaign,
    mission_stat: &mut crate::mission_stat::MissionStat,
    f: impl FnOnce(&mut NativeContext<'_, '_>) -> R,
) -> R {
    let sim = crate::sim_rng::test_context();
    let capabilities = NativeSessionCapabilities::new(
        &sim,
        &mut host.entities,
        &mut host.ai_global,
        &mut host.fast_grid,
    )
    .with_campaign(campaign, mission_stat);
    let mut context = NativeContext::with_bindings(
        &mut host.host,
        &mut host.state,
        &mut host.script_domains,
        bindings,
        &capabilities,
    );
    f(&mut context)
}

fn call_campaign_native(
    host: &mut ScriptEffects,
    campaign: &mut crate::campaign::Campaign,
    mission_stat: &mut crate::mission_stat::MissionStat,
    native: NativeFn,
    stack: &mut NativeStack,
) -> i32 {
    with_campaign_context(
        host,
        AttachedScriptBindings::empty_ref(),
        campaign,
        mission_stat,
        |context| {
            <NativeContext<'_, '_> as HostFunctions>::call(context, native as u32, stack)
                .expect_return("non-nested campaign native test")
        },
    )
}

struct CampaignScriptEffects {
    host: ScriptEffects,
    entities: crate::entities::Entities,
    ai_global: crate::ai::AiGlobalState,
    fast_grid: crate::fast_find_grid::FastFindGrid,
    state: ScriptState,
    script_domains: crate::engine::ScriptDomains,
    campaign: crate::campaign::Campaign,
    mission_stat: crate::mission_stat::MissionStat,
    short_briefings: crate::short_briefings::ShortBriefings,
}

impl HostFunctions for CampaignScriptEffects {
    fn call(&mut self, index: u32, stack: &mut NativeStack) -> NativeCallOutcome {
        let sim = crate::sim_rng::test_context();
        let capabilities = NativeSessionCapabilities::new(
            &sim,
            &mut self.entities,
            &mut self.ai_global,
            &mut self.fast_grid,
        )
        .with_campaign(&mut self.campaign, &mut self.mission_stat)
        .with_short_briefings(&mut self.short_briefings);
        NativeContext::with_bindings(
            &mut self.host,
            &mut self.state,
            &mut self.script_domains,
            AttachedScriptBindings::empty_ref(),
            &capabilities,
        )
        .call(index, stack)
    }
}

struct BoundScriptEffects {
    simulation: crate::sim_rng::SimulationContext,
    host: ScriptEffects,
    entities: crate::entities::Entities,
    ai_global: crate::ai::AiGlobalState,
    fast_grid: crate::fast_find_grid::FastFindGrid,
    state: ScriptState,
    script_domains: crate::engine::ScriptDomains,
    bindings: AttachedScriptBindings,
    selected_pcs: Vec<EntityId>,
    sequence_manager: crate::sequence::SequenceManager,
    sound_sources: crate::sound_source::SoundSourceManager,
    weather: crate::engine::WeatherState,
    frame: u32,
    short_briefings: crate::short_briefings::ShortBriefings,
    standard_view_radius: u16,
    campaign: crate::campaign::Campaign,
    mission_stat: crate::mission_stat::MissionStat,
}

impl BoundScriptEffects {
    fn new() -> Self {
        Self {
            simulation: crate::sim_rng::test_context(),
            host: ScriptEffects::new(),
            entities: crate::entities::Entities::new(),
            ai_global: crate::ai::AiGlobalState::default(),
            fast_grid: crate::fast_find_grid::FastFindGrid::default(),
            state: ScriptState::default(),
            script_domains: crate::engine::ScriptDomains::default(),
            bindings: AttachedScriptBindings::default(),
            selected_pcs: Vec::new(),
            sequence_manager: crate::sequence::SequenceManager::new(),
            sound_sources: crate::sound_source::SoundSourceManager::new(),
            weather: crate::engine::WeatherState::default(),
            frame: 0,
            short_briefings: crate::short_briefings::ShortBriefings::default(),
            standard_view_radius: crate::ai_vision::DEFAULT_VIEW_RADIUS,
            campaign: crate::campaign::Campaign::default(),
            mission_stat: crate::mission_stat::MissionStat::default(),
        }
    }

    fn door_index_for_goal_sector(
        &self,
        goal_sector: u16,
        goal: (f32, f32),
    ) -> Option<crate::gate::DoorIndex> {
        self.script_domains
            .interactables
            .doors
            .iter()
            .enumerate()
            .find_map(|(idx, door)| {
                let matches_endpoint =
                    door.sector_out == goal_sector || door.sector_in == goal_sector;
                let matches_click_sector = door.click_polygon_contains(goal.0, goal.1);
                (matches_endpoint || matches_click_sector)
                    .then_some(crate::gate::DoorIndex(idx as u32))
            })
    }

    fn entity_at_legacy_slot(&self, slot: u32) -> &crate::element::Entity {
        self.entities
            .get_legacy_slot(slot)
            .unwrap_or_else(|| panic!("missing test entity in legacy slot {slot}"))
            .1
    }

    fn entity_at_legacy_slot_mut(&mut self, slot: u32) -> &mut crate::element::Entity {
        self.entities
            .get_legacy_slot_mut(slot)
            .unwrap_or_else(|| panic!("missing test entity in legacy slot {slot}"))
            .1
    }
}

impl std::ops::Deref for BoundScriptEffects {
    type Target = ScriptEffects;

    fn deref(&self) -> &Self::Target {
        &self.host
    }
}

impl std::ops::DerefMut for BoundScriptEffects {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.host
    }
}

impl HostFunctions for BoundScriptEffects {
    fn call(&mut self, index: u32, stack: &mut NativeStack) -> NativeCallOutcome {
        let capabilities = NativeSessionCapabilities::new(
            &self.simulation,
            &mut self.entities,
            &mut self.ai_global,
            &mut self.fast_grid,
        )
        .with_world_views(&[], &[], &[])
        .with_queries(
            &mut self.sequence_manager,
            &mut self.selected_pcs,
            &mut self.sound_sources,
            &self.weather,
            &self.frame,
        )
        .with_short_briefings(&mut self.short_briefings)
        .with_standard_view_radius(&mut self.standard_view_radius)
        .with_campaign(&mut self.campaign, &mut self.mission_stat);
        NativeContext::with_bindings(
            &mut self.host,
            &mut self.state,
            &mut self.script_domains,
            &self.bindings,
            &capabilities,
        )
        .call(index, stack)
    }
}

#[test]
fn nested_sequence_immediates_finish_before_parent_continuation() {
    let mut host = BoundScriptEffects::new();
    let mut soldier = native_test_soldier();
    soldier.element_data_mut().blipped = true;
    host.entities.push(Some(soldier));
    let actor = ScriptHandleCodec::actor_handle_from_index(0);

    assert_eq!(
        HostFunctions::call(
            &mut host,
            NativeFn::Start as u32,
            &mut NativeStack::default(),
        )
        .expect_return("Start"),
        1
    );
    let mut message = NativeStack::default();
    message.push_i32(actor);
    message.push_i32(77);
    assert_eq!(
        HostFunctions::call(&mut host, NativeFn::RecordSendMessage as u32, &mut message)
            .expect_return("RecordSendMessage"),
        1
    );
    let mut unblip = NativeStack::default();
    unblip.push_i32(actor);
    assert_eq!(
        HostFunctions::call(&mut host, NativeFn::RecordUnBlip as u32, &mut unblip)
            .expect_return("RecordUnBlip"),
        1
    );
    let outer = HostFunctions::call(
        &mut host,
        NativeFn::Thanx as u32,
        &mut NativeStack::default(),
    );
    let NativeCallOutcome::Yield(crate::interp::NativeYield {
        operation:
            crate::interp::NativeOperation::SequenceAction(
                crate::interp::SynchronousSequenceOperation { continuation, .. },
            ),
        ..
    }) = outer
    else {
        panic!("outer Thanx must yield its recorded SendMessage action");
    };
    assert!(host.entity_at_legacy_slot(0).element_data().blipped);
    assert_eq!(continuation.len(), 1, "parent Unblip tail must be detached");
    assert!(
        !host.sequence_manager.has_pending_immediate_actions(),
        "detached parent siblings must be invisible to a nested callback"
    );
}

/// Run a native and return the queued deferred commands for inspection.
fn run_native_deferred(index: u32, args: &[i32]) -> (StopReason, Vec<DeferredCommand>) {
    let prog = call_native_return(index, args);
    let mut vm = Vm::new().with_host(BoundScriptEffects::new());
    let stop = vm.run(&prog);
    let host = vm.take_host();
    (stop, host.simulation_barriers())
}

#[test]
fn enter_and_leave_game_accept_original_movement_style_codes() {
    assert!(NativeContext::validate_style(0, "RecordEnterGame"));
    assert!(NativeContext::validate_style(1, "RecordLeaveGame"));
    assert!(!NativeContext::validate_style(2, "RecordEnterGame"));
    assert!(!NativeContext::validate_style(-1, "RecordLeaveGame"));
}

#[test]
fn enter_and_leave_game_styles_map_to_expected_orders() {
    assert_eq!(
        NativeContext::movement_style(0),
        crate::order::OrderType::WalkingUpright
    );
    assert_eq!(
        NativeContext::movement_style(1),
        crate::order::OrderType::RunningUpright
    );
}

#[test]
fn send_message_native_launches_and_yields_inline() {
    let (stop, commands) = run_native_deferred(NativeFn::SendMessage as u32, &[0, 1234]);
    assert!(matches!(
        stop,
        StopReason::Yield(crate::interp::NativeYield {
            operation: crate::interp::NativeOperation::SequenceAction(_),
            resume: crate::interp::ResumePolicy::Fixed(0),
        })
    ));
    assert!(commands.is_empty());

    let (stop, commands) = run_native_deferred(
        NativeFn::SendMessageWithArguments as u32,
        &[0, 2345, -11, 22],
    );
    assert!(matches!(
        stop,
        StopReason::Yield(crate::interp::NativeYield {
            operation: crate::interp::NativeOperation::SequenceAction(_),
            resume: crate::interp::ResumePolicy::Fixed(0),
        })
    ));
    assert!(commands.is_empty());
}

#[test]
fn thanx_returns_true_for_an_empty_active_recording() {
    let mut host = BoundScriptEffects::new();
    assert_eq!(
        HostFunctions::call(
            &mut host,
            NativeFn::Start as u32,
            &mut NativeStack::default(),
        )
        .expect_return("Start"),
        1
    );
    assert_eq!(
        HostFunctions::call(
            &mut host,
            NativeFn::Thanx as u32,
            &mut NativeStack::default(),
        )
        .expect_return("empty Thanx"),
        1
    );
    assert_eq!(host.sequence_manager.sequences_iter().count(), 0);
}

#[test]
fn globals_init_set_get() {
    let program = vec![
        BeginFunction {
            volatile_count: 0,
            temp_count: 3,
        },
        Aff0IConstant {
            dst: TMP0,
            constant: 42,
        },
        Aff0IConstant {
            dst: TMP4,
            constant: 100,
        },
        NativeParam { sym: TMP0 },
        NativeParam { sym: TMP4 },
        NativeCall { index: 0 }, // InitGlobal
        Aff0IConstant {
            dst: TMP4,
            constant: 200,
        },
        NativeParam { sym: TMP0 },
        NativeParam { sym: TMP4 },
        NativeCall { index: 1 }, // SetGlobal
        NativeParam { sym: TMP0 },
        NativeCall { index: 2 }, // GetGlobal
        Aff1NativeGetReturn { sym: TMP8 },
        ReturnVal { sym: TMP8 },
    ];
    let host = BoundScriptEffects::new();
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&program), StopReason::ReturnedValue(200));
}

#[test]
fn stub_returns_zero_and_logs() {
    let program = vec![
        BeginFunction {
            volatile_count: 0,
            temp_count: 2,
        },
        Aff0IConstant {
            dst: TMP0,
            constant: 5,
        },
        NativeParam { sym: TMP0 },
        NativeCall { index: 17 }, // StartDialog (stub)
        Aff1NativeGetReturn { sym: TMP4 },
        ReturnVal { sym: TMP4 },
    ];
    let host = BoundScriptEffects::new();
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&program), StopReason::ReturnedValue(0));
}

#[test]
fn name_lookup() {
    assert_eq!(native_name(0), "InitGlobal");
    assert_eq!(native_name(17), "StartDialog");
    assert_eq!(native_name(74), "ThisActor");
    assert_eq!(native_name(999), "unknown");
}

#[test]
fn script_effects_json_omits_runtime_entities() {
    let mut host = BoundScriptEffects::new();
    let mut npc = native_test_soldier();
    npc.npc_data_mut().unwrap().custom_values[7] = 456;
    host.entities.push(Some(npc));

    let value = serde_json::to_value(&*host).expect("save/rollback JSON value");
    assert!(value.get("entities").is_none());
    let json = serde_json::to_string(&*host).expect("serialize ScriptEffects");
    let _decoded: ScriptEffects = serde_json::from_str(&json).expect("deserialize ScriptEffects");

    assert_eq!(
        host.entities
            .get_legacy_slot(0)
            .unwrap()
            .1
            .npc_data()
            .unwrap()
            .custom_values[7],
        456
    );
}

#[test]
fn npc_custom_values_participate_in_state_hash() {
    let mut baseline = BoundScriptEffects::new();
    let mut same = BoundScriptEffects::new();
    let mut changed = BoundScriptEffects::new();
    for (host, value) in [(&mut baseline, 456), (&mut same, 456), (&mut changed, 457)] {
        let mut npc = native_test_soldier();
        npc.npc_data_mut().unwrap().custom_values[7] = value;
        host.entities.push(Some(npc));
    }

    assert_eq!(
        robin_util::state_hash::compute(&baseline.entities),
        robin_util::state_hash::compute(&same.entities)
    );
    assert_ne!(
        robin_util::state_hash::compute(&baseline.entities),
        robin_util::state_hash::compute(&changed.entities)
    );
}

#[test]
fn door_sector_goal_resolves_click_polygon_door_index() {
    let mut host = BoundScriptEffects::new();
    let mut door = Door {
        active: true,
        click_polygon: vec![(10.0, 10.0), (30.0, 10.0), (30.0, 30.0), (10.0, 30.0)],
        ..Default::default()
    };
    door.rebuild_click_bbox();
    host.script_domains.interactables.doors.push(door);

    assert_eq!(
        host.door_index_for_goal_sector(99, (20.0, 20.0)),
        Some(crate::gate::DoorIndex(0))
    );
}

#[test]
fn door_mutation_is_visible_to_later_native_in_same_callback() {
    let mut host = BoundScriptEffects::new();
    host.script_domains.interactables.doors.push(Door {
        active: false,
        locked_pc: true,
        ..Default::default()
    });
    let door = ScriptHandleCodec::door_handle_from_index(0);

    let mut unlock = NativeStack::default();
    unlock.push_i32(door);
    unlock.push_i32(0);
    assert_eq!(
        call_host_native(&mut host, NativeFn::SetDoorLockedPC, &mut unlock),
        0
    );

    let mut query = NativeStack::default();
    query.push_i32(door);
    assert_eq!(
        call_host_native(&mut host, NativeFn::IsDoorLockedPC, &mut query),
        0
    );
    assert!(
        host.script_domains.interactables.doors[0].active,
        "the Original activates a door when script unlocks it"
    );
}

#[test]
fn patch_mutation_is_visible_to_later_native_in_same_callback() {
    let mut host = BoundScriptEffects::new();
    host.script_domains.interactables.patches.push(Patch {
        active: true,
        initially_active: true,
        ..Default::default()
    });
    let patch = ScriptHandleCodec::patch_handle_from_index(0);

    let mut apply = NativeStack::default();
    apply.push_i32(patch);
    assert_eq!(
        call_host_native(&mut host, NativeFn::ApplyPatch, &mut apply),
        1
    );

    let mut query = NativeStack::default();
    query.push_i32(patch);
    assert_eq!(
        call_host_native(&mut host, NativeFn::IsPatchApplied, &mut query),
        1
    );
    assert!(matches!(
        host.simulation_barriers().as_slice(),
        [DeferredCommand::ProcessPatchEffects { patch_index, .. }]
            if usize::from(*patch_index) == 0
    ));
}

#[test]
fn mission_ui_mutations_are_visible_in_same_callback() {
    let mut host = BoundScriptEffects::new();

    let mut set_outline = NativeStack::default();
    set_outline.push_i32(1);
    assert_eq!(
        call_host_native(&mut host, NativeFn::SetOutlineDisplay, &mut set_outline),
        0
    );
    assert_eq!(
        call_host_native(
            &mut host,
            NativeFn::GetOutlineDisplay,
            &mut NativeStack::default(),
        ),
        1
    );
    assert!(host.script_domains.mission_ui.outline_display);
    assert!(matches!(
        host.engine_commands().as_slice(),
        [EngineCommand::SetOutlineDisplay { display: true }]
    ));

    assert_eq!(
        call_host_native(
            &mut host,
            NativeFn::ForceCheckVictory,
            &mut NativeStack::default(),
        ),
        0
    );
    assert!(host.script_domains.mission_ui.force_check);
}

// --- Sequence manager ---

#[test]
fn start_returns_one() {
    assert_eq!(run_native(30, &[]), StopReason::ReturnedValue(1));
}

#[test]
fn thanx_without_recording_returns_zero() {
    // Thanx with no active recording logs an error and returns false.
    assert_eq!(run_native(31, &[]), StopReason::ReturnedValue(0));
}

#[test]
fn then_outside_recording_returns_zero() {
    // Then with sequence_level < 1 logs an error and returns 0.  It
    // must not mutate any recording state — every call returns 0, not
    // an incrementing id.
    let program = vec![
        BeginFunction {
            volatile_count: 0,
            temp_count: 3,
        },
        NativeCall { index: 32 }, // Then → 0
        Aff1NativeGetReturn { sym: TMP0 },
        NativeCall { index: 32 }, // Then → 0
        Aff1NativeGetReturn { sym: TMP4 },
        NativeCall { index: 32 }, // Then → 0
        Aff1NativeGetReturn { sym: TMP8 },
        ReturnVal { sym: TMP8 },
    ];
    let host = BoundScriptEffects::new();
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&program), StopReason::ReturnedValue(0));
}

// --- Actor comparison & state queries ---

#[test]
fn script_actor_handle_maps_back_to_zero_based_entity_index() {
    assert_eq!(ScriptHandleCodec::actor_handle_index(0), None);
    assert_eq!(
        ScriptHandleCodec::actor_handle_index(ScriptHandleCodec::actor_handle_from_index(0)),
        Some(0)
    );
    assert_eq!(
        ScriptHandleCodec::actor_handle_index(ScriptHandleCodec::actor_handle_from_index(70)),
        Some(70)
    );
}

fn mobile_fx(mobile_index: u16) -> Entity {
    Entity::Fx(crate::element::ElementFx {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::Fx,
            active: true,
            ..Default::default()
        },
        fx: crate::element::FxData {
            mobile_index: Some(mobile_index),
            ..Default::default()
        },
    })
}

#[test]
fn mobile_master_is_appended_to_script_actor_indices() {
    let mut host = BoundScriptEffects::new();
    host.entities
        .push(Some(Entity::Fx(crate::element::ElementFx {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::Fx,
                ..Default::default()
            },
            fx: crate::element::FxData::default(),
        })));
    host.entities.push(Some(mobile_fx(0)));

    let mut get = NativeStack::default();
    get.push_i32(1);
    let handle = call_host_native(&mut host, NativeFn::GetActorScript, &mut get);
    assert_eq!(handle, ScriptHandleCodec::actor_handle_from_index(1));

    let mut reverse = NativeStack::default();
    reverse.push_i32(handle);
    assert_eq!(
        call_host_native(&mut host, NativeFn::GetActorIndex, &mut reverse),
        0
    );

    let mut is_cart = NativeStack::default();
    is_cart.push_i32(handle);
    assert_eq!(
        call_host_native(&mut host, NativeFn::IsActorCart, &mut is_cart),
        1
    );
}

#[test]
fn generic_mobile_activation_propagates_to_all_children() {
    let mut host = BoundScriptEffects::new();
    host.entities.push(Some(mobile_fx(0)));
    host.entities.push(Some(mobile_fx(0)));
    let handle = ScriptHandleCodec::actor_handle_from_index(0);

    let mut deactivate = NativeStack::default();
    deactivate.push_i32(handle);
    assert_eq!(
        call_host_native(&mut host, NativeFn::Deactivate, &mut deactivate),
        1
    );
    assert!(
        host.entities
            .occupied()
            .all(|(_, entity)| !entity.is_active())
    );
    assert!(matches!(
        host.engine_commands().as_slice(),
        [EngineCommand::SetMobileActive {
            mobile_index: 0,
            active: false
        }]
    ));
}

#[test]
fn is_actor_equal_same() {
    assert_eq!(run_native(86, &[7, 7]), StopReason::ReturnedValue(1));
}

#[test]
fn is_actor_equal_different() {
    assert_eq!(run_native(86, &[7, 8]), StopReason::ReturnedValue(0));
}

#[test]
fn is_actor_dead_unknown_handle() {
    // No entity at handle 5 → default 0 (not dead).
    assert_eq!(run_native(87, &[5]), StopReason::ReturnedValue(0));
}

#[test]
fn is_actor_ko_unknown_handle() {
    assert_eq!(run_native(88, &[5]), StopReason::ReturnedValue(0));
}

#[test]
fn is_actor_tied_unknown_handle() {
    assert_eq!(run_native(89, &[5]), StopReason::ReturnedValue(0));
}

#[test]
fn is_actor_hs_unknown_handle() {
    assert_eq!(run_native(90, &[5]), StopReason::ReturnedValue(0));
}

// --- Actor action / activation ---

#[test]
fn god_returns_null_handle() {
    // God() returns NULL, which is handle 0.
    assert_eq!(run_native(111, &[]), StopReason::ReturnedValue(0));
}

#[test]
fn stop_actor_unknown_handle_noop() {
    // Invalid handle → warn, no deferred command.
    let (stop, cmds) = run_native_deferred(103, &[5]);
    assert_eq!(stop, StopReason::ReturnedValue(0));
    assert!(cmds.is_empty());
}

#[test]
fn select_select_all_queues_command() {
    // `Select` returns true unconditionally (including the error branch).
    let (stop, cmds) = run_native_deferred(112, &[31]);
    assert_eq!(stop, StopReason::ReturnedValue(1));
    assert!(matches!(
        cmds.first(),
        Some(DeferredCommand::SelectPC {
            actor: 0,
            select: true
        })
    ));
}

#[test]
fn select_unselect_all_queues_command() {
    let (stop, cmds) = run_native_deferred(112, &[0]);
    assert_eq!(stop, StopReason::ReturnedValue(1));
    assert!(matches!(
        cmds.first(),
        Some(DeferredCommand::SelectPC {
            actor: 0,
            select: false
        })
    ));
}

#[test]
fn select_unknown_code_warns_but_no_command() {
    let (stop, cmds) = run_native_deferred(112, &[5]);
    assert_eq!(stop, StopReason::ReturnedValue(1));
    assert!(cmds.is_empty());
}

#[test]
fn select_all_and_unselect_all_are_immediately_query_visible() {
    let mut host = BoundScriptEffects::new();
    host.entities
        .push(Some(native_test_pc(Vec::new(), Vec::new())));
    host.entities
        .push(Some(native_test_pc(Vec::new(), Vec::new())));

    let mut select = NativeStack::default();
    select.push_i32(31);
    assert_eq!(
        call_host_native(&mut host, NativeFn::Select, &mut select),
        1
    );
    assert_eq!(
        call_host_native(
            &mut host,
            NativeFn::GetNumberOfSelectedPCs,
            &mut NativeStack::default(),
        ),
        2
    );

    let mut unselect = NativeStack::default();
    unselect.push_i32(0);
    assert_eq!(
        call_host_native(&mut host, NativeFn::Select, &mut unselect),
        1
    );
    assert_eq!(
        call_host_native(
            &mut host,
            NativeFn::GetNumberOfSelectedPCs,
            &mut NativeStack::default(),
        ),
        0
    );
}

#[test]
fn deactivate_unknown_handle_noop() {
    assert_eq!(run_native(113, &[3]), StopReason::ReturnedValue(0));
}

#[test]
fn activate_unknown_handle_noop() {
    assert_eq!(run_native(114, &[3]), StopReason::ReturnedValue(0));
}

// --- AI control ---

#[test]
fn lock_ai_unknown_handle_noop() {
    assert_eq!(run_native(134, &[5, 1]), StopReason::ReturnedValue(0));
}

#[test]
fn unlock_ai_unknown_handle_noop() {
    assert_eq!(run_native(135, &[5]), StopReason::ReturnedValue(0));
}

#[test]
fn freeze_unknown_handle_noop() {
    assert_eq!(run_native(138, &[5, 1]), StopReason::ReturnedValue(0));
}

#[test]
fn freeze_all_queues_command() {
    let (stop, cmds) = run_native_deferred(139, &[1]);
    assert_eq!(stop, StopReason::ReturnedValue(0));
    assert!(matches!(
        cmds.first(),
        Some(DeferredCommand::FreezeAll { freeze: true })
    ));
}

#[test]
fn freeze_all_unfreeze_queues_command() {
    let (stop, cmds) = run_native_deferred(139, &[0]);
    assert_eq!(stop, StopReason::ReturnedValue(0));
    assert!(matches!(
        cmds.first(),
        Some(DeferredCommand::FreezeAll { freeze: false })
    ));
}

// --- Location / distance ---

#[test]
fn nowhere_returns_zero() {
    assert_eq!(run_native(159, &[]), StopReason::ReturnedValue(0));
}

#[test]
fn get_distance_with_positions() {
    let host = BoundScriptEffects::new();
    let bindings = AttachedScriptBindings {
        script_location_count: 2,
        script_point_count: 2,
        location_positions: std::sync::Arc::new(vec![(0.0, 0.0), (30.0, 40.0)]),
        ..Default::default()
    };
    let prog = call_native_return(
        160,
        &[
            ScriptHandleCodec::location_handle_from_index(0),
            ScriptHandleCodec::location_handle_from_index(1),
        ],
    );
    let mut vm = Vm::new().with_host(Box::new(BoundScriptEffects { bindings, ..host }));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(50)); // sqrt(30²+40²)=50
}

#[test]
fn get_distance_invalid_handle() {
    assert_eq!(run_native(160, &[99, 100]), StopReason::ReturnedValue(0));
}

#[test]
fn camera_commands_copy_static_and_vm_local_computed_points() {
    let mut host = BoundScriptEffects::new();
    host.state.computed_locations.push(ComputedScriptLocation {
        position: (90.0, 123.0),
        layer_sector: Some((2, 44)),
    });
    let bindings = AttachedScriptBindings {
        script_location_count: 1,
        script_point_count: 1,
        location_positions: std::sync::Arc::new(vec![(12.0, 34.0)]),
        ..Default::default()
    };

    let mut jump = NativeStack::default();
    jump.push_i32(ScriptHandleCodec::location_handle_from_index(1));
    assert_eq!(
        call_bound_host_native(&mut host, &bindings, NativeFn::JumpCameraTo, &mut jump),
        0
    );

    let mut scroll = NativeStack::default();
    scroll.push_i32(ScriptHandleCodec::location_handle_from_index(0));
    scroll.push_i32(0.75_f32.to_bits() as i32);
    assert_eq!(
        call_bound_host_native(
            &mut host,
            &bindings,
            NativeFn::ScrollCameraSlowlyTo,
            &mut scroll,
        ),
        0
    );

    assert!(matches!(
        host.engine_commands().as_slice(),
        [
            EngineCommand::JumpCameraTo { x: 90.0, y: 123.0 },
            EngineCommand::ScrollCameraTo {
                x: 12.0,
                y: 34.0,
                speed: 0.75,
            },
        ]
    ));
}

#[test]
fn is_inside_building_specific() {
    let mut host = BoundScriptEffects::new();
    let actor = ScriptHandleCodec::actor_handle_from_index(4);
    let building = ScriptHandleCodec::building_handle_from_index(2);
    host.script_domains
        .buildings
        .actor_building
        .insert(actor, building);
    let prog = call_native_return(98, &[actor, building]);
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(1));
}

#[test]
fn is_inside_building_wrong() {
    let mut host = BoundScriptEffects::new();
    let actor = ScriptHandleCodec::actor_handle_from_index(4);
    host.script_domains
        .buildings
        .actor_building
        .insert(actor, ScriptHandleCodec::building_handle_from_index(2));
    let prog = call_native_return(
        98,
        &[actor, ScriptHandleCodec::building_handle_from_index(6)],
    );
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(0));
}

#[test]
fn is_inside_building_null_checks_any() {
    let mut host = BoundScriptEffects::new();
    let actor = ScriptHandleCodec::actor_handle_from_index(4);
    host.script_domains
        .buildings
        .actor_building
        .insert(actor, ScriptHandleCodec::building_handle_from_index(2));
    // NULL building (0): checks if in ANY building
    let prog = call_native_return(98, &[actor, 0]);
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(1));
}

#[test]
fn is_inside_building_not_in_any() {
    let host = BoundScriptEffects::new();
    let prog = call_native_return(98, &[ScriptHandleCodec::actor_handle_from_index(4), 0]);
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(0));
}

#[test]
fn is_inside_zone() {
    let mut host = BoundScriptEffects::new();
    let actor = ScriptHandleCodec::actor_handle_from_index(4);
    let loc = ScriptHandleCodec::location_handle_from_index(1);
    seed_zone(
        &mut host,
        1,
        &[
            ScriptHandleCodec::actor_handle_from_index(2),
            actor,
            ScriptHandleCodec::actor_handle_from_index(6),
        ],
    );
    let prog = call_native_return(97, &[actor, loc]);
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(1));
}

#[test]
fn is_inside_zone_not_present() {
    let mut host = BoundScriptEffects::new();
    let actor = ScriptHandleCodec::actor_handle_from_index(4);
    let loc = ScriptHandleCodec::location_handle_from_index(1);
    seed_zone(
        &mut host,
        1,
        &[
            ScriptHandleCodec::actor_handle_from_index(2),
            ScriptHandleCodec::actor_handle_from_index(6),
        ],
    );
    let prog = call_native_return(97, &[actor, loc]);
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(0));
}

#[test]
fn actors_in_sector() {
    // GetNumberOfActorsInSector / GetActorInSector reject non-sector
    // handles via `is_script_sector_handle` (sector handles live in
    // `script_point_count < loc <= script_location_count`), so seed
    // counts so loc=2 is a valid sector handle.
    let mut host = BoundScriptEffects::new();
    let bindings = AttachedScriptBindings {
        script_point_count: 1,
        script_location_count: 2,
        ..Default::default()
    };
    let loc = ScriptHandleCodec::location_handle_from_index(1);
    seed_zone(
        &mut host,
        0,
        &[
            ScriptHandleCodec::actor_handle_from_index(2),
            ScriptHandleCodec::actor_handle_from_index(4),
            ScriptHandleCodec::actor_handle_from_index(6),
        ],
    );

    let prog = call_native_return(204, &[loc]);
    let mut vm = Vm::new().with_host(Box::new(BoundScriptEffects { bindings, ..host }));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(3));

    // Re-add occupants since vm takes ownership
    let mut host2 = BoundScriptEffects::new();
    let bindings2 = AttachedScriptBindings {
        script_point_count: 1,
        script_location_count: 2,
        ..Default::default()
    };
    seed_zone(
        &mut host2,
        0,
        &[
            ScriptHandleCodec::actor_handle_from_index(2),
            ScriptHandleCodec::actor_handle_from_index(4),
            ScriptHandleCodec::actor_handle_from_index(6),
        ],
    );
    let prog2 = call_native_return(205, &[loc, 1]);
    let mut vm2 = Vm::new().with_host(Box::new(BoundScriptEffects {
        bindings: bindings2,
        ..host2
    }));
    assert_eq!(
        vm2.run(&prog2),
        StopReason::ReturnedValue(ScriptHandleCodec::actor_handle_from_index(4))
    );
}

#[test]
fn compute_location_between() {
    let host = BoundScriptEffects::new();
    let bindings = AttachedScriptBindings {
        script_location_count: 2,
        script_point_count: 2,
        location_positions: std::sync::Arc::new(vec![(0.0, 0.0), (100.0, 200.0)]),
        location_layers: std::sync::Arc::new(vec![0, 0]),
        location_sectors: std::sync::Arc::new(vec![0, 0]),
        ..Default::default()
    };
    let lambda_bits = 0.5f32.to_bits() as i32;
    let prog = call_native_return(
        213,
        &[
            ScriptHandleCodec::location_handle_from_index(0),
            ScriptHandleCodec::location_handle_from_index(1),
            lambda_bits,
        ],
    );
    let mut vm = Vm::new().with_host(Box::new(BoundScriptEffects { bindings, ..host }));
    // Should return a handle >= 3 (first computed location)
    match vm.run(&prog) {
        StopReason::ReturnedValue(handle) => {
            assert_eq!(ScriptHandleCodec::location_index(handle), Some(2));
        }
        other => panic!("expected return, got {other:?}"),
    }
}

#[test]
fn are_all_pcs_inside() {
    let mut host = BoundScriptEffects::new();
    host.entities = crate::entities::Entities::from_legacy_slots(vec![
        Some(native_test_pc(Vec::new(), Vec::new())),
        Some(native_test_pc(Vec::new(), Vec::new())),
        Some(native_test_pc(Vec::new(), Vec::new())),
    ]);
    let loc = ScriptHandleCodec::location_handle_from_index(0);
    seed_zone(
        &mut host,
        0,
        &(0..3)
            .map(ScriptHandleCodec::actor_handle_from_index)
            .collect::<Vec<_>>(),
    );
    let prog = call_native_return(230, &[loc]);
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(1));
}

#[test]
fn are_all_pcs_inside_not_all() {
    let mut host = BoundScriptEffects::new();
    host.entities = crate::entities::Entities::from_legacy_slots(vec![
        Some(native_test_pc(Vec::new(), Vec::new())),
        Some(native_test_pc(Vec::new(), Vec::new())),
        Some(native_test_pc(Vec::new(), Vec::new())),
    ]);
    let handles: Vec<_> = (0..3)
        .map(ScriptHandleCodec::actor_handle_from_index)
        .collect();
    let loc = ScriptHandleCodec::location_handle_from_index(0);
    seed_zone(&mut host, 0, &[handles[0], handles[2]]); // PC 2 missing
    let prog = call_native_return(230, &[loc]);
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(0));
}

#[test]
fn register_production_sector() {
    let mut host = BoundScriptEffects::new();
    host.bindings.script_point_count = 1;
    host.bindings.script_location_count = 2;
    host.bindings.location_positions = std::sync::Arc::new(vec![(12.0, 34.0), (20.0, 40.0)]);
    host.bindings.location_layers = std::sync::Arc::new(vec![2, 2]);
    host.bindings.location_sectors = std::sync::Arc::new(vec![7, 7]);
    host.script_domains
        .zones
        .scripts
        .push(crate::sector::ScriptSectorData::default());

    let point_handle = ScriptHandleCodec::location_handle_from_index(0);
    let sector_handle = ScriptHandleCodec::location_handle_from_index(1);

    let mut registration = NativeStack::default();
    registration.push_i32(0);
    registration.push_i32(sector_handle);
    registration.push_i32(10);
    assert_eq!(
        call_host_native(
            &mut host,
            NativeFn::RegisterAsProductionSector,
            &mut registration,
        ),
        0
    );
    assert_eq!(
        host.script_domains.zones.scripts[0].production_sector_type,
        crate::sector_production::Type::MakeArrow
    );
    assert_eq!(host.campaign.production_sectors[0].speed, 10);
    assert_eq!(host.campaign.production_sectors[0].script_zone, Some(0));

    let mut point = NativeStack::default();
    point.push_i32(0);
    point.push_i32(point_handle);
    assert_eq!(
        call_host_native(&mut host, NativeFn::AddProductionPoint, &mut point),
        0
    );
    assert_eq!(
        host.campaign.production_sectors[0].production_points.len(),
        1
    );
    let saved = &host.campaign.production_sectors[0].production_points[0];
    assert_eq!(
        (saved.x, saved.y, saved.layer, saved.sector),
        (12.0, 34.0, 2, 7)
    );
    assert_eq!(saved.obstacle, 0xFFFF);
    assert!(host.engine_commands().is_empty());
    assert!(host.sound_commands().is_empty());
    assert!(host.simulation_barriers().is_empty());
}

#[test]
#[should_panic(expected = "is already attached")]
fn production_sector_rejects_duplicate_attachment() {
    let mut host = BoundScriptEffects::new();
    host.bindings.script_location_count = 1;
    host.script_domains
        .zones
        .scripts
        .push(crate::sector::ScriptSectorData::default());
    let sector = ScriptHandleCodec::location_handle_from_index(0);
    for _ in 0..1 {
        let mut registration = NativeStack::default();
        registration.push_i32(0);
        registration.push_i32(sector);
        registration.push_i32(10);
        call_host_native(
            &mut host,
            NativeFn::RegisterAsProductionSector,
            &mut registration,
        );
    }
    let mut registration = NativeStack::default();
    registration.push_i32(0);
    registration.push_i32(sector);
    registration.push_i32(11);
    call_host_native(
        &mut host,
        NativeFn::RegisterAsProductionSector,
        &mut registration,
    );
}

#[test]
#[should_panic(expected = "has no attached script sector")]
fn production_point_requires_registered_sector() {
    let mut host = BoundScriptEffects::new();
    host.bindings.script_point_count = 1;
    host.bindings.script_location_count = 1;
    host.bindings.location_positions = std::sync::Arc::new(vec![(12.0, 34.0)]);
    host.bindings.location_layers = std::sync::Arc::new(vec![2]);
    host.bindings.location_sectors = std::sync::Arc::new(vec![7]);
    let point_handle = ScriptHandleCodec::location_handle_from_index(0);
    let mut point = NativeStack::default();
    point.push_i32(0);
    point.push_i32(point_handle);
    call_host_native(&mut host, NativeFn::AddProductionPoint, &mut point);
}

// --- Custom campaign values ---

#[test]
fn campaign_values_set_get() {
    // SetCustomCampaignValue(7, 42); return GetCustomCampaignValue(7)
    let program = vec![
        BeginFunction {
            volatile_count: 0,
            temp_count: 3,
        },
        Aff0IConstant {
            dst: TMP0,
            constant: 7,
        },
        Aff0IConstant {
            dst: TMP4,
            constant: 42,
        },
        NativeParam { sym: TMP0 },
        NativeParam { sym: TMP4 },
        NativeCall { index: 196 }, // SetCustomCampaignValue
        NativeParam { sym: TMP0 },
        NativeCall { index: 195 }, // GetCustomCampaignValue
        Aff1NativeGetReturn { sym: TMP8 },
        ReturnVal { sym: TMP8 },
    ];
    let host = CampaignScriptEffects {
        host: ScriptEffects::new(),
        entities: crate::entities::Entities::new(),
        ai_global: crate::ai::AiGlobalState::default(),
        fast_grid: crate::fast_find_grid::FastFindGrid::default(),
        state: ScriptState::default(),
        script_domains: crate::engine::ScriptDomains::default(),
        campaign: crate::campaign::Campaign::default(),
        mission_stat: crate::mission_stat::MissionStat::default(),
        short_briefings: crate::short_briefings::ShortBriefings::default(),
    };
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&program), StopReason::ReturnedValue(42));
}

#[test]
fn campaign_value_default_zero() {
    assert_eq!(run_native(195, &[99]), StopReason::ReturnedValue(0));
}

// --- Custom NPC values ---

#[test]
fn npc_values_set_then_get_from_canonical_entity() {
    let actor = ScriptHandleCodec::actor_handle_from_index(0);
    // SetCustomNPCValue(actor, id=5, value=77); return GetCustomNPCValue(actor, id=5).
    let program = vec![
        BeginFunction {
            volatile_count: 0,
            temp_count: 3,
        },
        Aff0IConstant {
            dst: TMP0,
            constant: actor,
        }, // actor
        Aff0IConstant {
            dst: TMP4,
            constant: 5,
        }, // id
        Aff0IConstant {
            dst: TMP8,
            constant: 77,
        }, // value
        NativeParam { sym: TMP0 },
        NativeParam { sym: TMP4 },
        NativeParam { sym: TMP8 },
        NativeCall { index: 198 }, // SetCustomNPCValue
        NativeParam { sym: TMP0 },
        NativeParam { sym: TMP4 },
        NativeCall { index: 197 }, // GetCustomNPCValue
        Aff1NativeGetReturn { sym: TMP8 },
        ReturnVal { sym: TMP8 },
    ];
    let mut host = BoundScriptEffects::new();
    host.entities.push(Some(native_test_soldier()));
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&program), StopReason::ReturnedValue(77));
}

#[test]
fn custom_values_are_isolated_between_script_hosts() {
    fn set_campaign(
        host: &mut ScriptEffects,
        campaign: &mut crate::campaign::Campaign,
        stat: &mut crate::mission_stat::MissionStat,
        value: i32,
    ) {
        let mut stack = NativeStack::default();
        stack.push_i32(3);
        stack.push_i32(value);
        assert_eq!(
            call_campaign_native(
                host,
                campaign,
                stat,
                NativeFn::SetCustomCampaignValue,
                &mut stack,
            ),
            0
        );
    }

    fn get_campaign(
        host: &mut ScriptEffects,
        campaign: &mut crate::campaign::Campaign,
        stat: &mut crate::mission_stat::MissionStat,
    ) -> i32 {
        let mut stack = NativeStack::default();
        stack.push_i32(3);
        call_campaign_native(
            host,
            campaign,
            stat,
            NativeFn::GetCustomCampaignValue,
            &mut stack,
        )
    }

    let mut first = ScriptEffects::new();
    let mut first_campaign = crate::campaign::Campaign::default();
    let mut first_stat = crate::mission_stat::MissionStat::default();
    let mut second = ScriptEffects::new();
    let mut second_campaign = crate::campaign::Campaign::default();
    let mut second_stat = crate::mission_stat::MissionStat::default();

    set_campaign(&mut first, &mut first_campaign, &mut first_stat, 11);
    set_campaign(&mut second, &mut second_campaign, &mut second_stat, 22);

    assert_eq!(
        get_campaign(&mut first, &mut first_campaign, &mut first_stat),
        11
    );
    assert_eq!(
        get_campaign(&mut second, &mut second_campaign, &mut second_stat),
        22
    );
}

#[test]
fn ordered_script_effect_stream_round_trips_and_hashes_in_order() {
    let mut effects = ScriptEffects::new();
    effects.emit_engine(EngineCommand::DisplayMap { show: true });
    effects.emit_sound(crate::natives::SoundCommand::SuspendAll);
    effects.emit_engine(EngineCommand::ChooseVictoryDefeatText { id: 7 });
    effects.emit_barrier(DeferredCommand::SetPlayable {
        actor: 17,
        playable: false,
    });
    assert!(matches!(
        effects.ordered.as_slices().0,
        [
            ScriptEffect::Presentation(EngineCommand::DisplayMap { show: true }),
            ScriptEffect::ExternalSound(crate::natives::SoundCommand::SuspendAll),
            ScriptEffect::Simulation(SimulationEffect::Engine(
                EngineCommand::ChooseVictoryDefeatText { id: 7 },
            )),
            ScriptEffect::Simulation(SimulationEffect::Deferred(DeferredCommand::SetPlayable {
                actor: 17,
                playable: false,
            },)),
        ]
    ));

    let json = serde_json::to_string(&effects).expect("serialize ordered effects");
    let decoded: ScriptEffects = serde_json::from_str(&json).expect("deserialize ordered effects");
    assert_eq!(
        serde_json::to_value(&decoded).expect("ordered JSON value"),
        serde_json::to_value(&effects).expect("source JSON value")
    );
    assert_eq!(
        robin_util::state_hash::compute(&decoded),
        robin_util::state_hash::compute(&effects)
    );

    let mut reordered = ScriptEffects::new();
    reordered.emit_sound(crate::natives::SoundCommand::SuspendAll);
    reordered.emit_engine(EngineCommand::DisplayMap { show: true });
    reordered.emit_engine(EngineCommand::ChooseVictoryDefeatText { id: 7 });
    reordered.emit_barrier(DeferredCommand::SetPlayable {
        actor: 17,
        playable: false,
    });
    assert_ne!(
        robin_util::state_hash::compute(&reordered),
        robin_util::state_hash::compute(&effects),
        "cross-domain emission order participates in deterministic state"
    );
}

#[test]
fn selection_mutates_canonical_state_before_a_later_native_in_the_same_callback() {
    let mut host = BoundScriptEffects::new();
    host.entities
        .push(Some(native_test_pc(Vec::new(), Vec::new())));
    host.entities
        .push(Some(native_test_pc(Vec::new(), Vec::new())));
    let actor = ScriptHandleCodec::actor_handle_from_index(0);
    let previously_selected = ScriptHandleCodec::actor_handle_from_index(1);
    let mut sequences = crate::sequence::SequenceManager::new();
    let mut selected = vec![EntityId::Pc(crate::entity_id::PcId(1))];
    let mut sounds = crate::sound_source::SoundSourceManager::new();
    let weather = crate::engine::WeatherState::default();
    let frame = 17;
    let mut select = NativeStack::default();
    select.push_i32(actor);
    select.push_i32(1);
    assert_eq!(
        call_host_native_with_queries(
            &mut host,
            NativeFn::SelectActorPC,
            &mut select,
            TestQueryViews::new(&mut sequences, &mut selected, &mut sounds, &weather, &frame),
        ),
        0
    );

    let mut is_selected = NativeStack::default();
    is_selected.push_i32(actor);
    assert_eq!(
        call_host_native_with_queries(
            &mut host,
            NativeFn::IsPCSelected,
            &mut is_selected,
            TestQueryViews::new(&mut sequences, &mut selected, &mut sounds, &weather, &frame),
        ),
        1
    );
    assert_eq!(selected, [EntityId::Pc(crate::entity_id::PcId(0))]);

    let mut old_is_selected = NativeStack::default();
    old_is_selected.push_i32(previously_selected);
    assert_eq!(
        call_host_native_with_queries(
            &mut host,
            NativeFn::IsPCSelected,
            &mut old_is_selected,
            TestQueryViews::new(&mut sequences, &mut selected, &mut sounds, &weather, &frame),
        ),
        0,
        "Original MSG_SELECT_CHARACTER replaces the old selection"
    );
    assert!(matches!(
        host.simulation_barriers().as_slice(),
        [DeferredCommand::SelectPC {
            actor: queued_actor,
            select: true,
        }] if *queued_actor == actor
    ));
}

#[test]
fn ai_lock_yields_before_the_script_can_launch_replacement_work() {
    let mut host = BoundScriptEffects::new();
    host.entities.push(Some(native_test_soldier()));
    let actor = ScriptHandleCodec::actor_handle_from_index(0);
    let mut sequences = crate::sequence::SequenceManager::new();
    let mut selected = Vec::new();
    let mut sounds = crate::sound_source::SoundSourceManager::new();
    let weather = crate::engine::WeatherState::default();
    let frame = 17;
    let sim = crate::sim_rng::test_context();
    let capabilities = NativeSessionCapabilities::new(
        &sim,
        &mut host.entities,
        &mut host.ai_global,
        &mut host.fast_grid,
    )
    .with_world_views(&[], &[], &[])
    .with_queries(&mut sequences, &mut selected, &mut sounds, &weather, &frame);
    let mut context = NativeContext::with_bindings(
        &mut host.host,
        &mut host.state,
        &mut host.script_domains,
        AttachedScriptBindings::empty_ref(),
        &capabilities,
    );

    let mut lock = NativeStack::default();
    lock.push_i32(actor);
    lock.push_i32(1);
    assert!(matches!(
        <NativeContext<'_, '_> as HostFunctions>::call(
            &mut context,
            NativeFn::LockAI as u32,
            &mut lock,
        ),
        NativeCallOutcome::Yield(crate::interp::NativeYield {
            operation: crate::interp::NativeOperation::EngineAction(
                crate::interp::SynchronousScriptRequest::LockAi {
                    actor: yielded_actor,
                    remember_events: true,
                    native_return: 0,
                },
            ),
            resume: crate::interp::ResumePolicy::Fixed(0),
        }) if yielded_actor == actor
    ));
}

#[test]
fn thanx_launches_into_the_live_sequence_manager_before_returning() {
    let mut host = BoundScriptEffects::new();
    let simulation = crate::sim_rng::test_context();
    let capabilities = NativeSessionCapabilities::new(
        &simulation,
        &mut host.entities,
        &mut host.ai_global,
        &mut host.fast_grid,
    )
    .with_queries(
        &mut host.sequence_manager,
        &mut host.selected_pcs,
        &mut host.sound_sources,
        &host.weather,
        &host.frame,
    )
    .with_short_briefings(&mut host.short_briefings)
    .with_standard_view_radius(&mut host.standard_view_radius);
    let mut context = NativeContext::with_bindings(
        &mut host.host,
        &mut host.state,
        &mut host.script_domains,
        &host.bindings,
        &capabilities,
    );

    assert_eq!(
        context
            .call(NativeFn::Start as u32, &mut NativeStack::default())
            .expect_return("Start is synchronous"),
        1
    );
    let mut timer = NativeStack::default();
    timer.push_i32(12);
    assert_eq!(
        context
            .call(NativeFn::RecordTimer as u32, &mut timer)
            .expect_return("RecordTimer is synchronous"),
        1
    );
    assert_eq!(
        context
            .sequence_manager
            .as_ref()
            .expect("live sequence manager")
            .sequences_iter()
            .count(),
        0,
        "recording alone must not launch"
    );
    assert!(matches!(
        context.call(NativeFn::Thanx as u32, &mut NativeStack::default()),
        NativeCallOutcome::Yield(crate::interp::NativeYield {
            operation: crate::interp::NativeOperation::SequenceAction(_),
            resume: crate::interp::ResumePolicy::Fixed(1),
        })
    ));
    let sequence = context
        .sequence_manager
        .as_ref()
        .expect("live sequence manager")
        .sequences_iter()
        .next()
        .expect("Thanx launched the completed recording inline");
    assert_eq!(sequence.elements.len(), 1);
    assert_eq!(sequence.elements[0].command, Command::Timer);
    assert!(matches!(
        sequence.elements[0].get_property(Field::Timer),
        Some(FieldValue::Integer(12))
    ));
}

#[test]
fn set_view_radius_updates_live_ai_and_every_npc_before_returning() {
    let mut host = BoundScriptEffects::new();
    host.entities.push(Some(native_test_soldier()));
    host.standard_view_radius = 400;
    let simulation = crate::sim_rng::test_context();
    let capabilities = NativeSessionCapabilities::new(
        &simulation,
        &mut host.entities,
        &mut host.ai_global,
        &mut host.fast_grid,
    )
    .with_standard_view_radius(&mut host.standard_view_radius);
    let mut context = NativeContext::with_bindings(
        &mut host.host,
        &mut host.state,
        &mut host.script_domains,
        &host.bindings,
        &capabilities,
    );
    let mut set = NativeStack::default();
    set.push_i32(275);
    assert_eq!(
        context
            .call(NativeFn::SetViewRadius as u32, &mut set)
            .expect_return("SetViewRadius is synchronous"),
        0
    );
    assert_eq!(context.standard_view_radius.as_deref(), Some(&275));
    let npc = context
        .entities
        .get_legacy_slot(0)
        .expect("test NPC")
        .1
        .npc_data()
        .expect("test NPC data");
    assert_eq!(npc.view_radius, 275);
    assert_eq!(npc.view_radius_base, 275);
    assert_eq!(npc.view_radius_goal, 275);
    assert!(
        context.script_effects().engine_commands().is_empty(),
        "SetViewRadius has no host presentation effect"
    );
}

#[test]
fn briefing_and_objective_writes_share_the_live_canonical_model() {
    let mut host = BoundScriptEffects::new();
    let simulation = crate::sim_rng::test_context();
    let capabilities = NativeSessionCapabilities::new(
        &simulation,
        &mut host.entities,
        &mut host.ai_global,
        &mut host.fast_grid,
    )
    .with_short_briefings(&mut host.short_briefings);
    let mut context = NativeContext::with_bindings(
        &mut host.host,
        &mut host.state,
        &mut host.script_domains,
        &host.bindings,
        &capabilities,
    );

    let mut add_briefing = NativeStack::default();
    add_briefing.push_i32(7);
    add_briefing.push_i32(1);
    context
        .call(NativeFn::AddShortBriefing as u32, &mut add_briefing)
        .expect_return("AddShortBriefing is synchronous");
    let mut done_briefing = NativeStack::default();
    done_briefing.push_i32(7);
    context
        .call(NativeFn::DoneShortBriefing as u32, &mut done_briefing)
        .expect_return("DoneShortBriefing is synchronous");

    let mut add_objective = NativeStack::default();
    add_objective.push_i32(11);
    add_objective.push_i32(0);
    context
        .call(NativeFn::AddObjective as u32, &mut add_objective)
        .expect_return("AddObjective is synchronous");
    let mut complete_objective = NativeStack::default();
    complete_objective.push_i32(11);
    context
        .call(NativeFn::CompleteObjective as u32, &mut complete_objective)
        .expect_return("CompleteObjective is synchronous");

    let briefings = context
        .short_briefings
        .as_ref()
        .expect("live short-briefing model");
    assert_eq!(briefings.get_id(true, 0), Some(7));
    assert_eq!(briefings.is_entry_done(true, 0), Some(true));
    assert_eq!(briefings.get_id(false, 0), Some(11));
    assert_eq!(briefings.is_entry_done(false, 0), Some(true));
    assert!(context.script_effects().engine_commands().is_empty());
}

#[test]
fn honolulu_location_native_yields_canonical_engine_action() {
    let mut host = BoundScriptEffects::new();
    host.entities.push(Some(native_test_soldier()));
    let actor = ScriptHandleCodec::actor_handle_from_index(0);
    let mut sequences = crate::sequence::SequenceManager::new();
    let mut selected = Vec::new();
    let mut sounds = crate::sound_source::SoundSourceManager::new();
    let weather = crate::engine::WeatherState::default();
    let frame = 17;
    let sim = crate::sim_rng::test_context();
    let capabilities = NativeSessionCapabilities::new(
        &sim,
        &mut host.entities,
        &mut host.ai_global,
        &mut host.fast_grid,
    )
    .with_world_views(&[], &[], &[])
    .with_queries(&mut sequences, &mut selected, &mut sounds, &weather, &frame);
    let mut context = NativeContext::with_bindings(
        &mut host.host,
        &mut host.state,
        &mut host.script_domains,
        AttachedScriptBindings::empty_ref(),
        &capabilities,
    );

    let mut set_location = NativeStack::default();
    set_location.push_i32(actor);
    set_location.push_i32(0);
    assert!(matches!(
        <NativeContext<'_, '_> as HostFunctions>::call(
            &mut context,
            NativeFn::SetActorLocation as u32,
            &mut set_location,
        ),
        NativeCallOutcome::Yield(crate::interp::NativeYield {
            operation: crate::interp::NativeOperation::EngineAction(
                crate::interp::SynchronousScriptRequest::SetActorLocation {
                    actor: yielded_actor,
                    location: 0,
                    ..
                }
            ),
            resume: crate::interp::ResumePolicy::OperationResult,
        }) if yielded_actor == actor
    ));
}

#[test]
fn location_yield_precedes_later_presentation_effect() {
    let mut host = BoundScriptEffects::new();
    host.entities
        .push(Some(native_test_pc(Vec::new(), Vec::new())));
    let actor = ScriptHandleCodec::actor_handle_from_index(0);
    let building = ScriptHandleCodec::building_handle_from_index(0);

    let mut teleport = NativeStack::default();
    teleport.push_i32(actor);
    teleport.push_i32(0);
    assert!(matches!(
        HostFunctions::call(&mut host, NativeFn::SetActorLocation as u32, &mut teleport),
        NativeCallOutcome::Yield(crate::interp::NativeYield {
            operation: crate::interp::NativeOperation::EngineAction(
                crate::interp::SynchronousScriptRequest::SetActorLocation { .. }
            ),
            ..
        })
    ));
    let mut put = NativeStack::default();
    put.push_i32(actor);
    put.push_i32(building);
    assert_eq!(
        HostFunctions::call(&mut host, NativeFn::PutActorInBuilding as u32, &mut put)
            .expect_return("PutActorInBuilding"),
        0
    );

    assert!(matches!(
        host.ordered.as_slices().0,
        [ScriptEffect::Simulation(SimulationEffect::Deferred(
            DeferredCommand::PutActorInBuilding {
                actor: building_actor,
                building: queued_building,
            },
        ))] if *building_actor == actor
            && *queued_building == building
    ));
}

#[test]
fn set_actor_posture_ko_yields_one_canonical_engine_action() {
    let mut host = BoundScriptEffects::new();
    let mut soldier = native_test_soldier();
    let Entity::Soldier(soldier_data) = &mut soldier else {
        unreachable!()
    };
    soldier_data.npc.life_points = 100;
    host.entities.push(Some(soldier));
    let actor = ScriptHandleCodec::actor_handle_from_index(0);
    let owner = host.entities.get_legacy_slot(0).unwrap().0;

    let mut active = SequenceElement::new(1, Command::Move, Some(owner));
    active.priority = crate::sequence::SequencePriority::Normal;
    let active_id = host.sequence_manager.launch_element(active);
    host.sequence_manager.take_pending_synchronous_actions();
    host.sequence_manager.element_in_progress(active_id, 0);

    let mut posture = NativeStack::default();
    posture.push_i32(actor);
    posture.push_i32(17); // ID_KO
    assert!(matches!(
        HostFunctions::call(&mut host, NativeFn::SetActorPosture as u32, &mut posture),
        NativeCallOutcome::Yield(crate::interp::NativeYield {
            operation: crate::interp::NativeOperation::EngineAction(
                crate::interp::SynchronousScriptRequest::SetActorPosture {
                    actor: yielded_actor,
                    posture: 17,
                    ..
                }
            ),
            ..
        }) if yielded_actor == actor
    ));
    assert_eq!(
        host.sequence_manager
            .get_element(active_id, 0)
            .expect("old active element")
            .state,
        crate::sequence::SequenceState::InProgress,
        "the native adapter must not duplicate the engine posture pipeline"
    );
}

#[test]
fn scroll_status_mutates_canonical_state_before_a_later_native_in_the_same_callback() {
    let mut host = BoundScriptEffects::new();
    host.entities
        .push(Some(Entity::Scroll(crate::element::ElementScroll {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ObjectScroll,
                ..Default::default()
            },
            ..Default::default()
        })));
    let scroll = ScriptHandleCodec::actor_handle_from_index(0);

    let mut set = NativeStack::default();
    set.push_i32(scroll);
    set.push_i32(3);
    assert_eq!(
        call_host_native(&mut host, NativeFn::SetScrollStatus, &mut set),
        0
    );

    let mut get = NativeStack::default();
    get.push_i32(scroll);
    assert_eq!(
        call_host_native(&mut host, NativeFn::GetScrollStatus, &mut get),
        3
    );
    assert_eq!(host.script_domains.scrolls.status.get(&scroll), Some(&3));
    assert!(matches!(
        host.engine_commands().as_slice(),
        [EngineCommand::SetScrollStatus {
            scroll_handle,
            status: 3,
        }] if *scroll_handle == scroll
    ));
}

#[test]
fn sound_destruction_mutates_the_live_source_manager_before_returning() {
    let mut host = BoundScriptEffects::new();
    let mut sequences = crate::sequence::SequenceManager::new();
    let mut sounds = crate::sound_source::SoundSourceManager::new();
    sounds.sources_push_some(crate::sound_source::SoundSource::default());
    let weather = crate::engine::WeatherState::default();
    let frame = 23;
    let mut selected = Vec::new();
    let handle = ScriptHandleCodec::sound_source_handle_from_index(0);

    let mut destroy = NativeStack::default();
    destroy.push_i32(handle);
    assert_eq!(
        call_host_native_with_queries(
            &mut host,
            NativeFn::DestroySoundSource,
            &mut destroy,
            TestQueryViews::new(&mut sequences, &mut selected, &mut sounds, &weather, &frame),
        ),
        1
    );

    let mut lookup = NativeStack::default();
    lookup.push_i32(0);
    assert_eq!(
        call_host_native_with_queries(
            &mut host,
            NativeFn::GetSoundSourceScript,
            &mut lookup,
            TestQueryViews::new(&mut sequences, &mut selected, &mut sounds, &weather, &frame),
        ),
        0
    );
    assert!(
        sounds.get(0).is_none(),
        "same-callback lookup must read the canonical destroyed slot"
    );
    assert!(matches!(
        host.sound_commands().as_slice(),
        [SoundCommand::Destroy(queued)] if *queued == handle
    ));
}

#[test]
fn current_action_and_frame_queries_read_canonical_runtime_state() {
    let pc_id = EntityId::Pc(crate::entity_id::PcId(0));
    let pc_handle = ScriptHandleCodec::actor_handle(pc_id);
    let mut pc_host = BoundScriptEffects::new();
    pc_host
        .entities
        .push(Some(native_test_pc(Vec::new(), Vec::new())));
    let mut sequences = crate::sequence::SequenceManager::new();
    let mut element =
        crate::sequence::SequenceElement::new(1, crate::element::Command::Move, Some(pc_id));
    element.push_order(crate::order::Order::new(
        crate::order::OrderType::RunningUpright,
        0.0,
        0.0,
        std::num::NonZeroU32::new(1).unwrap(),
    ));
    let sequence_id = sequences.launch_element(element);
    sequences.element_in_progress(sequence_id, 0);
    let mut sounds = crate::sound_source::SoundSourceManager::new();
    let weather = crate::engine::WeatherState::default();
    let frame = 123;
    let mut selected = Vec::new();

    let mut action = NativeStack::default();
    action.push_i32(pc_handle);
    assert_eq!(
        call_host_native_with_queries(
            &mut pc_host,
            NativeFn::GetCurrentAction,
            &mut action,
            TestQueryViews::new(&mut sequences, &mut selected, &mut sounds, &weather, &frame),
        ),
        crate::order::OrderType::RunningUpright as i32
    );

    let mut npc_host = BoundScriptEffects::new();
    npc_host.entities.push(Some(native_test_soldier()));
    let mut emoticon = NativeStack::default();
    emoticon.push_i32(ScriptHandleCodec::actor_handle_from_index(0));
    emoticon.push_i32(crate::ai::EmoticonType::QuestionMark as i32);
    emoticon.push_i32(7);
    assert_eq!(
        call_host_native_with_queries(
            &mut npc_host,
            NativeFn::SetNPCEmoticon,
            &mut emoticon,
            TestQueryViews::new(&mut sequences, &mut selected, &mut sounds, &weather, &frame),
        ),
        0
    );
    assert_eq!(
        npc_host
            .entity_at_legacy_slot(0)
            .ai_controller()
            .unwrap()
            .emoticon_expiration_date,
        130
    );
}

#[test]
fn canonical_query_views_are_isolated_between_engine_instances() {
    let mut first_sequences = crate::sequence::SequenceManager::new();
    let mut first_selection = vec![EntityId::Pc(crate::entity_id::PcId(0))];
    let mut first_sounds = crate::sound_source::SoundSourceManager::new();
    let first_weather = crate::engine::WeatherState::default();
    let first_frame = 10;
    let mut second_sequences = crate::sequence::SequenceManager::new();
    let mut second_selection = vec![
        EntityId::Pc(crate::entity_id::PcId(0)),
        EntityId::Pc(crate::entity_id::PcId(1)),
    ];
    let mut second_sounds = crate::sound_source::SoundSourceManager::new();
    let second_weather = crate::engine::WeatherState::default();
    let second_frame = 900;
    let first_queries = TestQueryViews::new(
        &mut first_sequences,
        &mut first_selection,
        &mut first_sounds,
        &first_weather,
        &first_frame,
    );
    let second_queries = TestQueryViews::new(
        &mut second_sequences,
        &mut second_selection,
        &mut second_sounds,
        &second_weather,
        &second_frame,
    );
    let mut first_host = BoundScriptEffects::new();
    let mut second_host = BoundScriptEffects::new();

    assert_eq!(
        call_host_native_with_queries(
            &mut first_host,
            NativeFn::GetNumberOfSelectedPCs,
            &mut NativeStack::default(),
            first_queries,
        ),
        1
    );
    assert_eq!(
        call_host_native_with_queries(
            &mut second_host,
            NativeFn::GetNumberOfSelectedPCs,
            &mut NativeStack::default(),
            second_queries,
        ),
        2
    );
}

#[test]
fn sight_query_view_borrows_canonical_world_arrays() {
    let mut host = ScriptEffects::new();
    let mut state = ScriptState::default();
    let mut script_domains = crate::engine::ScriptDomains::default();
    let mut entities = crate::entities::Entities::new();
    let mut ai_global = crate::ai::AiGlobalState::default();
    let mut fast_grid = crate::fast_find_grid::FastFindGrid::default();
    let static_obstacles = vec![crate::sight_obstacle::SightObstacle::new_default(0)];
    let dynamic_obstacles = vec![crate::sight_obstacle::SightObstacle::new_default(1)];
    let static_active = vec![false];
    let sim = crate::sim_rng::test_context();
    let capabilities =
        NativeSessionCapabilities::new(&sim, &mut entities, &mut ai_global, &mut fast_grid)
            .with_world_views(&static_obstacles, &dynamic_obstacles, &static_active);
    let context = NativeContext::with_bindings(
        &mut host,
        &mut state,
        &mut script_domains,
        AttachedScriptBindings::empty_ref(),
        &capabilities,
    );
    let sight = context.sight_obstacles.expect("canonical sight view");

    assert!(std::ptr::eq(
        sight.static_obstacles.as_ptr(),
        static_obstacles.as_ptr()
    ));
    assert!(std::ptr::eq(
        sight.dynamic_obstacles.as_ptr(),
        dynamic_obstacles.as_ptr()
    ));
    assert!(std::ptr::eq(
        sight.static_active.as_ptr(),
        static_active.as_ptr()
    ));
    assert!(!sight.is_active(0));
}

#[test]
fn query_only_state_is_not_persisted_in_script_effects_json() {
    let mut value = serde_json::to_value(ScriptEffects::new()).expect("serialize ScriptEffects");
    let object = value
        .as_object_mut()
        .expect("ScriptEffects serializes as an object");
    for (field, old_value) in [
        ("current_animations", serde_json::json!({"123": 7})),
        ("selected_pc_handles", serde_json::json!([123, 456])),
        ("sound_source_alive", serde_json::json!([true, false])),
        ("sound_source_count", serde_json::json!(2)),
        ("ambiance", serde_json::json!("Night")),
        ("is_forest_level", serde_json::json!(true)),
        ("frame_counter", serde_json::json!(9876)),
        ("verbose", serde_json::json!(true)),
    ] {
        object.insert(field.into(), old_value);
    }

    let restored: ScriptEffects =
        serde_json::from_value(value).expect("unknown query-only fields are ignored");
    let saved_again = serde_json::to_value(&restored).expect("re-serialize ScriptEffects");
    for field in [
        "current_animations",
        "selected_pc_handles",
        "sound_source_alive",
        "sound_source_count",
        "ambiance",
        "is_forest_level",
        "frame_counter",
        "verbose",
    ] {
        assert!(
            saved_again.get(field).is_none(),
            "query-only field {field} returned"
        );
    }

    let mut sequences = crate::sequence::SequenceManager::new();
    let mut selection = vec![EntityId::Pc(crate::entity_id::PcId(4))];
    let mut sounds = crate::sound_source::SoundSourceManager::new();
    let weather = crate::engine::WeatherState::default();
    let frame = 4;
    assert_eq!(
        call_host_native_with_queries(
            &mut BoundScriptEffects {
                host: restored,
                ..BoundScriptEffects::new()
            },
            NativeFn::GetNumberOfSelectedPCs,
            &mut NativeStack::default(),
            TestQueryViews::new(
                &mut sequences,
                &mut selection,
                &mut sounds,
                &weather,
                &frame
            ),
        ),
        1,
        "loaded hosts query canonical runtime state, not stale save mirrors"
    );
}

#[test]
fn animation_state_write_is_immediately_visible_from_canonical_entity() {
    let actor = ScriptHandleCodec::actor_handle_from_index(0);
    let mut host = BoundScriptEffects::new();
    host.entities
        .push(Some(Entity::Fx(crate::element::ElementFx {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::Fx,
                ..Default::default()
            },
            fx: crate::element::FxData::default(),
        })));

    let mut set = NativeStack::default();
    set.push_i32(actor);
    set.push_i32(1);
    assert_eq!(
        call_host_native(&mut host, NativeFn::SetAnimationState, &mut set),
        1
    );

    let mut get = NativeStack::default();
    get.push_i32(actor);
    assert_eq!(
        call_host_native(&mut host, NativeFn::IsAnimationActive, &mut get),
        1
    );
    assert!(host.entity_at_legacy_slot(0).element_data().active);
}

#[test]
fn npc_value_nonexistent_actor_returns_minus_one() {
    // `GetCustomNPCValue` emits an error and returns -1 when
    // ActorExists fails.  Without entity setup the actor handle
    // resolves to no entity, so we exercise that error path.
    assert_eq!(run_native(197, &[1, 1]), StopReason::ReturnedValue(-1));
}

/// Verify `compute_border_point`: given an inside point and a facing
/// direction, the border is on the edge opposite the direction of
/// travel, and the outside point sits comfortably past that edge
/// (actor silhouette no longer overlaps the map box).
#[test]
fn compute_border_point_cardinal_directions() {
    use crate::coordinates::MapBBox;

    let map_bbox = MapBBox::from_coords(0.0, 0.0, 1000.0, 800.0);
    let inside = (400.0, 300.0);

    // Direction 0 = facing north (-y). Actor enters from the south
    // edge walking north, so border is on y=800 and outside is below.
    let (border, outside) = compute_border_point_bbox(map_bbox, inside, 0);
    assert!((border.0 - 400.0).abs() < 0.1);
    assert!((border.1 - 800.0).abs() < 0.1);
    assert!(outside.1 > 800.0);

    // Direction 8 = facing south (+y). Border on y=0 (top edge),
    // outside above the map.
    let (border, outside) = compute_border_point_bbox(map_bbox, inside, 8);
    assert!((border.0 - 400.0).abs() < 0.1);
    assert!((border.1 - 0.0).abs() < 0.1);
    assert!(outside.1 < 0.0);

    // Direction 4 = facing east (+x). Border on x=0 (left edge),
    // outside to the left.
    let (border, outside) = compute_border_point_bbox(map_bbox, inside, 4);
    assert!((border.0 - 0.0).abs() < 0.1);
    assert!((border.1 - 300.0).abs() < 0.1);
    assert!(outside.0 < 0.0);

    // Direction 12 = facing west (-x). Border on x=1000, outside to
    // the right.
    let (border, outside) = compute_border_point_bbox(map_bbox, inside, 12);
    assert!((border.0 - 1000.0).abs() < 0.1);
    assert!((border.1 - 300.0).abs() < 0.1);
    assert!(outside.0 > 1000.0);
}

// ── Direct campaign-owner side effects ────────────────────────────

#[test]
fn direct_owner_add_campaign_value_ransom_credits_stat_and_queues_jingle() {
    let mut host = ScriptEffects::new();
    let mut campaign = crate::campaign::Campaign::default();
    let mut mission_stat = crate::mission_stat::MissionStat::default();
    with_campaign_context(
        &mut host,
        AttachedScriptBindings::empty_ref(),
        &mut campaign,
        &mut mission_stat,
        |context| {
            context.add_campaign_value(crate::campaign::CampaignValue::Ransom, 250, 100);
        },
    );

    assert_eq!(
        campaign.get_value(crate::campaign::CampaignValue::Ransom),
        crate::campaign::INITIAL_RANSOM + 250
    );
    assert_eq!(mission_stat.collected_money, 250);
    let jingle_count = host
        .sound_commands()
        .iter()
        .filter(|c| matches!(c, SoundCommand::PlayJingle(crate::sound::Jingle::CashWon)))
        .count();
    assert_eq!(jingle_count, 1);
}

#[test]
fn direct_owner_set_campaign_value_ransom_jingle_only_when_growing() {
    let mut host = ScriptEffects::new();
    let mut campaign = crate::campaign::Campaign::default();
    let mut mission_stat = crate::mission_stat::MissionStat::default();
    campaign.values[crate::campaign::CampaignValue::Ransom] = 200;

    // Lowering: no jingle.
    with_campaign_context(
        &mut host,
        AttachedScriptBindings::empty_ref(),
        &mut campaign,
        &mut mission_stat,
        |context| {
            context.set_campaign_value(crate::campaign::CampaignValue::Ransom, 100, 50);
        },
    );
    assert!(host.engine_commands().is_empty());
    assert!(host.sound_commands().is_empty());

    // Raising: jingle queued.
    with_campaign_context(
        &mut host,
        AttachedScriptBindings::empty_ref(),
        &mut campaign,
        &mut mission_stat,
        |context| {
            context.set_campaign_value(crate::campaign::CampaignValue::Ransom, 500, 50);
        },
    );
    let jingle_count = host
        .sound_commands()
        .iter()
        .filter(|c| matches!(c, SoundCommand::PlayJingle(crate::sound::Jingle::CashWon)))
        .count();
    assert_eq!(jingle_count, 1);
    // SetValue does NOT credit collected_money.
    assert_eq!(mission_stat.collected_money, 0);
}

#[test]
fn ransom_natives_round_trip_through_borrowed_campaign_owner() {
    let mut host = ScriptEffects::new();
    let mut campaign = crate::campaign::Campaign::default();
    let mut mission_stat = crate::mission_stat::MissionStat::default();
    let mut sequences = crate::sequence::SequenceManager::new();
    let mut sounds = crate::sound_source::SoundSourceManager::new();
    let weather = crate::engine::WeatherState::default();
    let frame = 50;
    let mut state = ScriptState::default();
    let mut script_domains = crate::engine::ScriptDomains::default();
    let mut entities = crate::entities::Entities::new();
    let mut ai_global = crate::ai::AiGlobalState::default();
    let mut fast_grid = crate::fast_find_grid::FastFindGrid::default();
    let mut selected = Vec::new();
    let sim = crate::sim_rng::test_context();
    let capabilities =
        NativeSessionCapabilities::new(&sim, &mut entities, &mut ai_global, &mut fast_grid)
            .with_queries(&mut sequences, &mut selected, &mut sounds, &weather, &frame)
            .with_campaign(&mut campaign, &mut mission_stat);
    let mut context = NativeContext::with_bindings(
        &mut host,
        &mut state,
        &mut script_domains,
        AttachedScriptBindings::empty_ref(),
        &capabilities,
    );

    let mut set = NativeStack::default();
    set.push_i32(1_234);
    assert_eq!(
        context
            .call(NativeFn::SetRansomMoney as u32, &mut set)
            .expect_return("SetRansomMoney is synchronous"),
        0
    );
    let mut get = NativeStack::default();
    assert_eq!(
        context
            .call(NativeFn::GetRansomMoney as u32, &mut get)
            .expect_return("GetRansomMoney is synchronous"),
        1_234
    );
    drop(context);
    drop(capabilities);

    assert_eq!(
        campaign.get_value(crate::campaign::CampaignValue::Ransom),
        1_234
    );
    assert_eq!(mission_stat.collected_money, 0);
}

#[test]
fn direct_owner_add_campaign_value_score_credits_added_score_silently() {
    let mut host = ScriptEffects::new();
    let mut campaign = crate::campaign::Campaign::default();
    let mut mission_stat = crate::mission_stat::MissionStat::default();
    with_campaign_context(
        &mut host,
        AttachedScriptBindings::empty_ref(),
        &mut campaign,
        &mut mission_stat,
        |context| {
            context.add_campaign_value(crate::campaign::CampaignValue::Score, 750, 100);
        },
    );

    assert_eq!(mission_stat.added_score, 750);
    assert!(host.engine_commands().is_empty());
}

fn native_test_soldier() -> Entity {
    Entity::Soldier(crate::element::ActorSoldier {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorSoldier,
            ..Default::default()
        },
        actor: crate::element::ActorData::default(),
        human: crate::element::HumanData::default(),
        npc: crate::element::NpcData {
            ai_brain: crate::element::AiBrain::Enemy(Box::new(crate::ai_enemy::EnemyAi::new(0))),
            ..Default::default()
        },
        soldier: crate::element::SoldierData::default(),
    })
}

fn native_test_pc(disabled_actions: Vec<bool>, disabled_actions_temp: Vec<bool>) -> Entity {
    Entity::Pc(crate::element::ActorPc {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorPc,
            ..Default::default()
        },
        actor: crate::element::ActorData::default(),
        human: crate::element::HumanData::default(),
        pc: crate::element::PcData {
            disabled_actions,
            disabled_actions_temp,
            ..Default::default()
        },
    })
}

fn persistent_property_test_host(
    with_campaign: bool,
) -> (
    BoundScriptEffects,
    AttachedScriptBindings,
    Option<crate::campaign::Campaign>,
    i32,
) {
    use crate::profiles::{Action, CharacterProfile, CharacterProfileIdx};

    let mut profiles = crate::profiles::ProfileManager::new();
    profiles.characters.push(CharacterProfile {
        actions: [Action::Bow, Action::Stone, Action::Apple],
        action_max_ammo: [12, 6, 6],
        ..Default::default()
    });
    let mut host = BoundScriptEffects::new();
    let bindings = AttachedScriptBindings {
        profile_manager: std::sync::Arc::new(profiles),
        ..Default::default()
    };

    let mut pc = native_test_pc(vec![true; 3], vec![false; 3]);
    let pc_data = pc.pc_data_mut().expect("test entity must be a PC");
    pc_data.profile_index = CharacterProfileIdx(0);
    pc_data.current_action = Action::Bow;
    pc_data.saved_action = Action::Bow;
    host.entities = crate::entities::Entities::from_legacy_slots(vec![Some(pc)]);

    let campaign = if with_campaign {
        let mut status = crate::pc_status::PcStatus::default();
        status.set_ammo(Action::Bow, 2);
        status.set_ammo(Action::Stone, 5);
        Some(crate::campaign::Campaign {
            characters: vec![crate::campaign::PcDescription {
                character_profile_idx: Some(CharacterProfileIdx(0)),
                status,
                ..Default::default()
            }],
            ..Default::default()
        })
    } else {
        None
    };

    (
        host,
        bindings,
        campaign,
        ScriptHandleCodec::actor_handle_from_index(0),
    )
}

fn call_set_persistent_property(
    host: &mut BoundScriptEffects,
    bindings: &AttachedScriptBindings,
    actor: i32,
    prop: i32,
    amount: i32,
) -> i32 {
    let mut stack = NativeStack::default();
    stack.push_i32(actor);
    stack.push_i32(prop);
    stack.push_i32(amount);
    call_bound_host_native(host, bindings, NativeFn::SetPersistentProperty, &mut stack)
}

fn call_get_persistent_property(
    host: &mut BoundScriptEffects,
    bindings: &AttachedScriptBindings,
    actor: i32,
    prop: i32,
) -> i32 {
    let mut stack = NativeStack::default();
    stack.push_i32(actor);
    stack.push_i32(prop);
    call_bound_host_native(host, bindings, NativeFn::GetPersistentProperty, &mut stack)
}

fn call_set_persistent_property_with_campaign(
    host: &mut BoundScriptEffects,
    bindings: &AttachedScriptBindings,
    campaign: &mut crate::campaign::Campaign,
    mission_stat: &mut crate::mission_stat::MissionStat,
    actor: i32,
    prop: i32,
    amount: i32,
) -> i32 {
    let mut stack = NativeStack::default();
    stack.push_i32(actor);
    stack.push_i32(prop);
    stack.push_i32(amount);
    with_bound_campaign_context(host, bindings, campaign, mission_stat, |context| {
        context
            .call(NativeFn::SetPersistentProperty as u32, &mut stack)
            .expect_return("non-nested persistent-property test")
    })
}

fn call_get_persistent_property_with_campaign(
    host: &mut BoundScriptEffects,
    bindings: &AttachedScriptBindings,
    campaign: &mut crate::campaign::Campaign,
    mission_stat: &mut crate::mission_stat::MissionStat,
    actor: i32,
    prop: i32,
) -> i32 {
    let mut stack = NativeStack::default();
    stack.push_i32(actor);
    stack.push_i32(prop);
    with_bound_campaign_context(host, bindings, campaign, mission_stat, |context| {
        context
            .call(NativeFn::GetPersistentProperty as u32, &mut stack)
            .expect_return("non-nested persistent-property test")
    })
}

fn set_then_get_persistent_program(
    actor: i32,
    property: i32,
    amount: i32,
) -> Vec<crate::vm::Instruction> {
    vec![
        BeginFunction {
            volatile_count: 0,
            temp_count: 4,
        },
        Aff0IConstant {
            dst: TMP0,
            constant: actor,
        },
        Aff0IConstant {
            dst: TMP4,
            constant: property,
        },
        Aff0IConstant {
            dst: TMP8,
            constant: amount,
        },
        NativeParam { sym: TMP0 },
        NativeParam { sym: TMP4 },
        NativeParam { sym: TMP8 },
        NativeCall {
            index: NativeFn::SetPersistentProperty as u32,
        },
        NativeParam { sym: TMP0 },
        NativeParam { sym: TMP4 },
        NativeCall {
            index: NativeFn::GetPersistentProperty as u32,
        },
        Aff1NativeGetReturn { sym: TMP12 },
        ReturnVal { sym: TMP12 },
    ]
}

#[test]
fn persistent_life_and_concussion_use_typed_engine_yields() {
    let actor = ScriptHandleCodec::actor_handle_from_index(0);

    let mut life_host = BoundScriptEffects::new();
    let mut pc = native_test_pc(Vec::new(), Vec::new());
    let Entity::Pc(pc_data) = &mut pc else {
        unreachable!()
    };
    pc_data.pc.life_points = 100;
    life_host.entities.push(Some(pc));
    let mut life_vm = Vm::new().with_host(Box::new(life_host));
    assert!(matches!(
        life_vm.run(&set_then_get_persistent_program(actor, 2, 37)),
        StopReason::Yield(crate::interp::NativeYield {
            operation: crate::interp::NativeOperation::EngineAction(
                crate::interp::SynchronousScriptRequest::SetPersistentLifePoints { .. }
            ),
            ..
        })
    ));

    let mut concussion_host = BoundScriptEffects::new();
    concussion_host
        .entities
        .push(Some(native_test_pc(Vec::new(), Vec::new())));
    let mut concussion_vm = Vm::new().with_host(Box::new(concussion_host));
    assert!(matches!(
        concussion_vm.run(&set_then_get_persistent_program(actor, 3, 123)),
        StopReason::Yield(crate::interp::NativeYield {
            operation: crate::interp::NativeOperation::EngineAction(
                crate::interp::SynchronousScriptRequest::SetPersistentConcussion { .. }
            ),
            ..
        })
    ));
}

#[test]
fn set_persistent_property_updates_live_pc_ammo_without_campaign() {
    use crate::element::PcAmmoData;
    use crate::profiles::Action;

    let (mut host, bindings, campaign, actor) = persistent_property_test_host(false);
    assert!(campaign.is_none());

    assert_eq!(
        call_set_persistent_property(&mut host, &bindings, actor, 0, 7),
        1
    );
    assert_eq!(
        call_set_persistent_property(&mut host, &bindings, actor, 5, 4),
        1
    );

    let pc = host.entity_at_legacy_slot(0).pc_data().unwrap();
    assert_eq!(
        pc.ammo,
        PcAmmoData {
            arrows: 7,
            stones: 4,
            ..Default::default()
        }
    );
    assert_eq!(pc.disabled_actions, [false, false, true]);
    assert_eq!(pc.current_action, Action::Bow);
    assert_eq!(pc.saved_action, Action::Bow);
    assert_eq!(
        call_get_persistent_property(&mut host, &bindings, actor, 0),
        7
    );
    assert_eq!(
        call_get_persistent_property(&mut host, &bindings, actor, 5),
        4
    );
}

#[test]
fn set_persistent_property_updates_live_and_campaign_pc_ammo() {
    use crate::element::PcAmmoData;
    use crate::profiles::Action;

    let (mut host, bindings, campaign, actor) = persistent_property_test_host(true);
    let mut campaign = campaign.expect("campaign fixture");
    let mut mission_stat = crate::mission_stat::MissionStat::default();
    {
        let pc = host.entity_at_legacy_slot_mut(0).pc_data_mut().unwrap();
        pc.ammo.arrows = 2;
        pc.ammo.stones = 5;
        pc.current_action = Action::Stone;
        pc.saved_action = Action::Stone;
    }

    assert_eq!(
        call_set_persistent_property_with_campaign(
            &mut host,
            &bindings,
            &mut campaign,
            &mut mission_stat,
            actor,
            0,
            6,
        ),
        1
    );
    assert_eq!(
        call_set_persistent_property_with_campaign(
            &mut host,
            &bindings,
            &mut campaign,
            &mut mission_stat,
            actor,
            5,
            0,
        ),
        1
    );

    let pc = host.entity_at_legacy_slot(0).pc_data().unwrap();
    assert_eq!(
        pc.ammo,
        PcAmmoData {
            arrows: 6,
            ..Default::default()
        }
    );
    assert_eq!(pc.disabled_actions, [false, true, true]);
    assert_eq!(pc.current_action, Action::NoAction);
    assert_eq!(pc.saved_action, Action::NoAction);

    let status = &campaign.characters[0].status;
    assert_eq!(status.get_ammo(Action::Bow), 6);
    assert_eq!(status.get_ammo(Action::Stone), 0);
    assert_eq!(
        call_get_persistent_property_with_campaign(
            &mut host,
            &bindings,
            &mut campaign,
            &mut mission_stat,
            actor,
            0,
        ),
        6
    );
    assert_eq!(
        call_get_persistent_property_with_campaign(
            &mut host,
            &bindings,
            &mut campaign,
            &mut mission_stat,
            actor,
            5,
        ),
        0
    );
}

fn native_sees(
    host: &mut BoundScriptEffects,
    weather: &crate::engine::WeatherState,
    npc_index: usize,
    target_index: usize,
) -> i32 {
    let mut sequences = crate::sequence::SequenceManager::new();
    let mut sounds = crate::sound_source::SoundSourceManager::new();
    let frame = 0;
    let mut selected = Vec::new();
    let mut stack = NativeStack::default();
    stack.push_i32(ScriptHandleCodec::actor_handle_from_index(npc_index));
    stack.push_i32(ScriptHandleCodec::actor_handle_from_index(target_index));
    call_host_native_with_queries(
        host,
        NativeFn::Sees,
        &mut stack,
        TestQueryViews::new(&mut sequences, &mut selected, &mut sounds, weather, &frame),
    )
}

fn native_sees_host(target: crate::coordinates::MapPoint, camp: Camp) -> BoundScriptEffects {
    let mut npc = native_test_soldier();
    npc.element_data_mut()
        .set_position_map(crate::coordinates::MapPoint::ZERO);
    npc.element_data_mut().set_direction_instantly(4);
    npc.element_data_mut().posture = Posture::Upright;
    let npc_data = npc.npc_data_mut().expect("test soldier has NPC data");
    npc_data.view_radius = 400;
    npc_data.eye_status = crate::element::EyeStatus::LookForward;
    npc_data.view_direction = [1.0, 0.0];
    npc_data.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    let Entity::Soldier(soldier) = &mut npc else {
        unreachable!("native_test_soldier must return a soldier")
    };
    soldier.soldier.cached_camp = camp;

    let mut pc = native_test_pc(Vec::new(), Vec::new());
    pc.element_data_mut().set_position_map(target);
    pc.element_data_mut().posture = Posture::Upright;

    let mut host = BoundScriptEffects::new();
    host.entities = crate::entities::Entities::from_legacy_slots(vec![Some(npc), Some(pc)]);
    host
}

#[test]
fn sees_uses_forest_royalist_180_degree_rule() {
    // A target due south is outside an east-facing 0.5-radian cone but
    // inside the flat forward 180-degree half-plane (dot product == 0).
    let mut host = native_sees_host(
        crate::coordinates::MapPoint::new(0.0, 100.0),
        Camp::Royalists,
    );
    let mut weather = crate::engine::WeatherState::default();

    assert_eq!(native_sees(&mut host, &weather, 0, 1), 0);

    weather.is_forest_level = true;
    assert_eq!(native_sees(&mut host, &weather, 0, 1), 1);

    let Entity::Soldier(soldier) = host.entity_at_legacy_slot_mut(0) else {
        unreachable!("observer must remain a soldier")
    };
    soldier.soldier.cached_camp = Camp::Lacklandists;
    assert_eq!(native_sees(&mut host, &weather, 0, 1), 0);
}

#[test]
fn sees_uses_ambiance_adjusted_view_radius() {
    // With a 500-unit raw radius, a target 450 units ahead is visible in
    // day ambiance. At night the nearby light sector drives the original
    // ComputeViewRadius blend to the 400-unit day shadow-polygon radius,
    // making that same target invisible. This exercises native Sees all the
    // way through the shared compute_view_radius + compute_visibility path.
    let mut host = native_sees_host(
        crate::coordinates::MapPoint::new(450.0, 0.0),
        Camp::Lacklandists,
    );
    let mut weather = crate::engine::WeatherState::default();
    host.entity_at_legacy_slot_mut(0)
        .npc_data_mut()
        .unwrap()
        .view_radius = 500;

    let level = std::sync::Arc::make_mut(&mut host.fast_grid.level);
    level.sectors.push(crate::fast_find_grid::GridSector {
        points: vec![
            crate::coordinates::MapPoint::new(240.0, -10.0),
            crate::coordinates::MapPoint::new(260.0, -10.0),
            crate::coordinates::MapPoint::new(260.0, 10.0),
            crate::coordinates::MapPoint::new(240.0, 10.0),
        ],
        bounding_box: crate::coordinates::MapBBox::new(),
        sector_type: crate::sector::SectorType::SHADOW,
        layer: 0,
        sector_number: crate::sector::SectorNumber::new(1),
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
    level.shadow_data.insert(
        0,
        crate::sector::ShadowData {
            barycentre_2d: crate::coordinates::MapPoint::new(250.0, 0.0),
            barycentre_3d_x: 250.0,
            barycentre_3d_y: 0.0,
            barycentre_3d_z: 45.0,
            radius: 10.0,
        },
    );

    assert_eq!(weather.ambiance, crate::engine::Ambiance::Day);
    assert_eq!(native_sees(&mut host, &weather, 0, 1), 1);

    weather.ambiance = crate::engine::Ambiance::Night;
    assert_eq!(native_sees(&mut host, &weather, 0, 1), 0);
}

fn set_experiences_test_host() -> (BoundScriptEffects, crate::campaign::Campaign, i32) {
    let actor = ScriptHandleCodec::actor_handle_from_index(0);
    let profile_idx = crate::profiles::CharacterProfileIdx(0);
    let mut status = crate::pc_status::PcStatus::default();
    status.human_status.hand_to_hand = crate::pc_status::Skill {
        experience: 37,
        capacity: 11,
    };
    status.human_status.bow = crate::pc_status::Skill {
        experience: 83,
        capacity: 22,
    };

    let mut campaign = crate::campaign::Campaign::default();
    campaign.characters.push(crate::campaign::PcDescription {
        character_profile_idx: Some(profile_idx),
        instanced: true,
        status,
    });

    let mut host = BoundScriptEffects::new();
    host.entities = crate::entities::Entities::from_legacy_slots(vec![Some(native_test_pc(
        Vec::new(),
        Vec::new(),
    ))]);
    (host, campaign, actor)
}

fn call_set_experiences(
    host: &mut BoundScriptEffects,
    campaign: &mut crate::campaign::Campaign,
    mission_stat: &mut crate::mission_stat::MissionStat,
    actor: i32,
    sword: i32,
    bow: i32,
) {
    let mut stack = NativeStack::default();
    stack.push_i32(actor);
    stack.push_i32(sword);
    stack.push_i32(bow);
    assert_eq!(
        with_bound_campaign_context(
            host,
            AttachedScriptBindings::empty_ref(),
            campaign,
            mission_stat,
            |context| {
                context
                    .call(NativeFn::SetExperiences as u32, &mut stack)
                    .expect_return("non-nested SetExperiences test")
            },
        ),
        0
    );
}

#[test]
fn set_experiences_updates_exact_backing_status_for_live_pc() {
    let (mut host, mut campaign, actor) = set_experiences_test_host();
    let mut mission_stat = crate::mission_stat::MissionStat::default();

    call_set_experiences(&mut host, &mut campaign, &mut mission_stat, actor, 64, 29);

    let status = &campaign.characters[0].status;
    assert_eq!(status.human_status.hand_to_hand.capacity, 64);
    assert_eq!(status.human_status.hand_to_hand.experience, 37);
    assert_eq!(status.human_status.bow.capacity, 29);
    assert_eq!(status.human_status.bow.experience, 83);
}

#[test]
fn set_experiences_capacities_persist_with_campaign_description() {
    let (mut host, mut campaign, actor) = set_experiences_test_host();
    let mut mission_stat = crate::mission_stat::MissionStat::default();
    call_set_experiences(&mut host, &mut campaign, &mut mission_stat, actor, 73, 41);

    let encoded =
        serde_json::to_string(&campaign).expect("serialize campaign after SetExperiences");
    let restored: crate::campaign::Campaign =
        serde_json::from_str(&encoded).expect("restore serialized campaign");

    let status = &restored.characters[0].status;
    assert_eq!(status.human_status.hand_to_hand.capacity, 73);
    assert_eq!(status.human_status.hand_to_hand.experience, 37);
    assert_eq!(status.human_status.bow.capacity, 41);
    assert_eq!(status.human_status.bow.experience, 83);
}

#[test]
fn set_action_available_validates_but_does_not_mutate_disabled_actions() {
    let mut host = BoundScriptEffects::new();
    host.entities = crate::entities::Entities::from_legacy_slots(vec![Some(native_test_pc(
        vec![false, false, false],
        vec![false, false, false],
    ))]);

    let mut stack = NativeStack::default();
    stack.push_i32(ScriptHandleCodec::actor_handle_from_index(0));
    stack.push_i32(0);
    stack.push_i32(0);
    let ret = call_host_native(&mut host, NativeFn::SetActionAvailable, &mut stack);
    assert_eq!(ret, 1);
    let pc = host.entity_at_legacy_slot(0).pc_data().unwrap();
    assert_eq!(pc.disabled_actions, [false, false, false]);
}

#[test]
fn is_action_available_rejects_out_of_range_slot() {
    let mut host = BoundScriptEffects::new();
    host.entities = crate::entities::Entities::from_legacy_slots(vec![Some(native_test_pc(
        vec![false, false, false],
        vec![false, false, false],
    ))]);

    let mut stack = NativeStack::default();
    stack.push_i32(ScriptHandleCodec::actor_handle_from_index(0));
    stack.push_i32(-1);
    let ret = call_host_native(&mut host, NativeFn::IsActionAvailable, &mut stack);
    assert_eq!(ret, 0);
}

#[test]
fn is_action_available_reads_persistent_and_temp_slot_masks() {
    let mut host = BoundScriptEffects::new();
    host.entities = crate::entities::Entities::from_legacy_slots(vec![Some(native_test_pc(
        vec![false, true, false],
        vec![false, false, true],
    ))]);
    let actor = ScriptHandleCodec::actor_handle_from_index(0);

    let mut stack = NativeStack::default();
    stack.push_i32(actor);
    stack.push_i32(0);
    assert_eq!(
        call_host_native(&mut host, NativeFn::IsActionAvailable, &mut stack),
        1
    );

    let mut stack = NativeStack::default();
    stack.push_i32(actor);
    stack.push_i32(1);
    assert_eq!(
        call_host_native(&mut host, NativeFn::IsActionAvailable, &mut stack),
        0
    );

    let mut stack = NativeStack::default();
    stack.push_i32(actor);
    stack.push_i32(2);
    assert_eq!(
        call_host_native(&mut host, NativeFn::IsActionAvailable, &mut stack),
        0
    );
}

#[test]
fn add_as_subordinate_requests_patrol_reinit() {
    let mut host = BoundScriptEffects::new();
    host.entities = crate::entities::Entities::from_legacy_slots(vec![
        Some(native_test_soldier()),
        Some(native_test_soldier()),
    ]);

    let mut stack = NativeStack::default();
    stack.push_i32(ScriptHandleCodec::actor_handle_from_index(0));
    stack.push_i32(ScriptHandleCodec::actor_handle_from_index(1));
    let ret = call_host_native(&mut host, NativeFn::AddAsSubordinate, &mut stack);
    assert_eq!(ret, 0);

    let chief_ai = host
        .entity_at_legacy_slot(0)
        .ai_controller()
        .expect("chief has AI");
    assert_eq!(
        chief_ai.theoretical_patrol,
        vec![EntityId::Soldier(crate::entity_id::SoldierId(1))]
    );
    assert!(chief_ai.patrol.is_empty());
    assert!(chief_ai.missed_patrol_members.is_empty());
    assert!(chief_ai.needs_patrol_reinit);
}
