//! Unit tests for the script native dispatch.

use super::*;
use crate::engine::TickCtx;
use crate::interp::*;
use crate::vm::Instruction::*;

const TMP0: u16 = 0xC000;
const TMP4: u16 = 0xC004;
const TMP8: u16 = 0xC008;
const TMP12: u16 = 0xC00C;
const TMP16: u16 = 0xC010;

#[test]
fn movement_recording_without_start_is_rejected() {
    for native in [NativeFn::RecordMove, NativeFn::RecordMoveNear] {
        let mut host = NativeTestHost::new();
        let mut soldier = native_test_soldier();
        soldier
            .element_data_mut()
            .set_sector(crate::position_interface::SectorHandle::new(0));
        host.entities.push(Some(soldier));
        let actor = ScriptHandleCodec::actor_handle_from_index(0);
        let mut location_query = NativeStack::default();
        location_query.push_i32(actor);
        let location = call_host_native(&mut host, NativeFn::GetActorLocation, &mut location_query);
        let mut arguments = NativeStack::default();
        arguments.push_i32(actor);
        arguments.push_i32(location);
        arguments.push_i32(0);
        if native == NativeFn::RecordMoveNear {
            arguments.push_i32(10);
        }
        assert_eq!(call_host_native(&mut host, native, &mut arguments), 0);
        assert!(host.state.sequence_recorder.is_none());
    }
}

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
    let host = NativeTestHost::new();
    let mut vm = Vm::new().with_host(Box::new(host));
    vm.run(&prog)
}

fn seed_zone(host: &mut NativeTestHost, zone_idx: usize, handles: &[i32]) {
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

fn call_host_native(host: &mut NativeTestHost, native: NativeFn, stack: &mut NativeStack) -> i32 {
    HostFunctions::call(host, native as u32, stack).expect_return("non-nested native test")
}

fn call_host_command(
    host: &mut NativeTestHost,
    native: NativeFn,
    stack: &mut NativeStack,
    expected_return: i32,
) -> NativeCommand {
    match HostFunctions::call(host, native as u32, stack) {
        NativeCallOutcome::Yield(NativeYield {
            operation: NativeOperation::Command(command),
            resume: ResumePolicy::Fixed(value),
        }) => {
            assert_eq!(value, expected_return);
            command
        }
        other => panic!("expected an engine native operation, got {other:?}"),
    }
}

/// Call `native(actor, argument)` for a lone soldier in slot 0 with empty
/// world views and live query owners; returns the actor handle and outcome.
fn lone_soldier_native_outcome(native: NativeFn, argument: i32) -> (i32, NativeCallOutcome) {
    let mut host = NativeTestHost::new();
    host.entities.push(Some(native_test_soldier()));
    let actor = ScriptHandleCodec::actor_handle_from_index(0);
    let mut sequences = crate::sequence::SequenceManager::new();
    let mut selected = Vec::new();
    let mut sounds = crate::sound_source::SoundSourceManager::new();
    let weather = crate::engine::WeatherState::default();
    let frame = 17;
    let sim = crate::sim_rng::test_context();
    let mut capabilities = NativeSessionCapabilities::new(
        &sim,
        &mut host.entities,
        &mut host.ai_global,
        &mut host.fast_grid,
        &mut host.globals,
    )
    .with_world_views(&[], &[], &[])
    .with_queries(&mut sequences, &mut selected, &mut sounds, &weather, &frame);
    let mut context = NativeContext::with_bindings(
        &mut host.state,
        &mut host.script_domains,
        AttachedScriptBindings::empty_ref(),
        &mut capabilities,
    );
    let mut stack = NativeStack::default();
    stack.push_i32(actor);
    stack.push_i32(argument);
    let outcome =
        <NativeContext<'_, '_> as HostFunctions>::call(&mut context, native as u32, &mut stack);
    (actor, outcome)
}

fn call_host_native_with_queries(
    host: &mut NativeTestHost,
    native: NativeFn,
    stack: &mut NativeStack,
    queries: TestQueryViews<'_>,
) -> i32 {
    let sim = crate::sim_rng::test_context();
    let pc_registry = host
        .entities
        .pcs()
        .map(|(id, _)| EntityId::Pc(id))
        .collect::<Vec<_>>();
    // Production sessions always attach the mission diplomacy; allegiance
    // natives (`Sees` on forest levels, `GetAIAttitude`, ...) require it.
    let mut diplomacy = crate::diplomacy::DiplomacyState::default();
    let mut capabilities = queries.attach_to(
        NativeSessionCapabilities::new(
            &sim,
            &mut host.entities,
            &mut host.ai_global,
            &mut host.fast_grid,
            &mut host.globals,
        )
        .with_pc_registry(&pc_registry)
        .with_diplomacy(&mut diplomacy),
    );
    let mut context = NativeContext::with_bindings(
        &mut host.state,
        &mut host.script_domains,
        AttachedScriptBindings::empty_ref(),
        &mut capabilities,
    );
    <NativeContext<'_, '_> as HostFunctions>::call(&mut context, native as u32, stack)
        .expect_return("non-nested native query test")
}

fn call_bound_host_native(
    host: &mut NativeTestHost,
    bindings: &AttachedScriptBindings,
    native: NativeFn,
    stack: &mut NativeStack,
) -> i32 {
    let sim = crate::sim_rng::test_context();
    let pc_registry = host
        .entities
        .pcs()
        .map(|(id, _)| EntityId::Pc(id))
        .collect::<Vec<_>>();
    let mut capabilities = NativeSessionCapabilities::new(
        &sim,
        &mut host.entities,
        &mut host.ai_global,
        &mut host.fast_grid,
        &mut host.globals,
    )
    .with_pc_registry(&pc_registry);
    let mut context = NativeContext::with_bindings(
        &mut host.state,
        &mut host.script_domains,
        bindings,
        &mut capabilities,
    );
    <NativeContext<'_, '_> as HostFunctions>::call(&mut context, native as u32, stack)
        .expect_return("non-nested native test")
}

fn with_campaign_context<R>(
    bindings: &AttachedScriptBindings,
    campaign: &mut crate::campaign::Campaign,
    mission_stat: &mut crate::mission_stat::MissionStat,
    f: impl FnOnce(&mut NativeContext<'_, '_>) -> R,
) -> R {
    let mut entities = crate::entities::Entities::new();
    let mut ai_global = crate::ai::AiGlobalState::default();
    let mut fast_grid = crate::fast_find_grid::FastFindGrid::default();
    let mut globals = Vec::new();
    let sim = crate::sim_rng::test_context();
    let mut capabilities = NativeSessionCapabilities::new(
        &sim,
        &mut entities,
        &mut ai_global,
        &mut fast_grid,
        &mut globals,
    )
    .with_campaign(campaign, mission_stat);
    let mut state = ScriptState::default();
    let mut script_domains = crate::engine::ScriptDomains::default();
    let mut context =
        NativeContext::with_bindings(&mut state, &mut script_domains, bindings, &mut capabilities);
    f(&mut context)
}

fn with_bound_campaign_context<R>(
    host: &mut NativeTestHost,
    bindings: &AttachedScriptBindings,
    campaign: &mut crate::campaign::Campaign,
    mission_stat: &mut crate::mission_stat::MissionStat,
    f: impl FnOnce(&mut NativeContext<'_, '_>) -> R,
) -> R {
    let sim = crate::sim_rng::test_context();
    let mut capabilities = NativeSessionCapabilities::new(
        &sim,
        &mut host.entities,
        &mut host.ai_global,
        &mut host.fast_grid,
        &mut host.globals,
    )
    .with_campaign(campaign, mission_stat);
    let mut context = NativeContext::with_bindings(
        &mut host.state,
        &mut host.script_domains,
        bindings,
        &mut capabilities,
    );
    f(&mut context)
}

fn call_campaign_native(
    campaign: &mut crate::campaign::Campaign,
    mission_stat: &mut crate::mission_stat::MissionStat,
    native: NativeFn,
    stack: &mut NativeStack,
) -> i32 {
    with_campaign_context(
        AttachedScriptBindings::empty_ref(),
        campaign,
        mission_stat,
        |context| {
            <NativeContext<'_, '_> as HostFunctions>::call(context, native as u32, stack)
                .expect_return("non-nested campaign native test")
        },
    )
}

struct CampaignNativeHost {
    entities: crate::entities::Entities,
    ai_global: crate::ai::AiGlobalState,
    fast_grid: crate::fast_find_grid::FastFindGrid,
    globals: Vec<i32>,
    state: ScriptState,
    script_domains: crate::engine::ScriptDomains,
    campaign: crate::campaign::Campaign,
    mission_stat: crate::mission_stat::MissionStat,
    short_briefings: crate::short_briefings::ShortBriefings,
}

impl HostFunctions for CampaignNativeHost {
    fn call(&mut self, index: u32, stack: &mut NativeStack) -> NativeCallOutcome {
        let sim = crate::sim_rng::test_context();
        let mut capabilities = NativeSessionCapabilities::new(
            &sim,
            &mut self.entities,
            &mut self.ai_global,
            &mut self.fast_grid,
            &mut self.globals,
        )
        .with_campaign(&mut self.campaign, &mut self.mission_stat)
        .with_short_briefings(&mut self.short_briefings);
        NativeContext::with_bindings(
            &mut self.state,
            &mut self.script_domains,
            AttachedScriptBindings::empty_ref(),
            &mut capabilities,
        )
        .call(index, stack)
    }
}

struct NativeTestHost {
    simulation: crate::sim_rng::SimulationContext,

    entities: crate::entities::Entities,
    ai_global: crate::ai::AiGlobalState,
    fast_grid: crate::fast_find_grid::FastFindGrid,
    globals: Vec<i32>,
    state: ScriptState,
    script_domains: crate::engine::ScriptDomains,
    bindings: AttachedScriptBindings,
    selected_pcs: Vec<EntityId>,
    pc_registry_override: Option<Vec<EntityId>>,
    selected_action: crate::profiles::Action,
    sequence_manager: crate::sequence::SequenceManager,
    sound_sources: crate::sound_source::SoundSourceManager,
    weather: crate::engine::WeatherState,
    frame: u32,
    short_briefings: crate::short_briefings::ShortBriefings,
    standard_view_radius: u16,
    campaign: crate::campaign::Campaign,
    mission_stat: crate::mission_stat::MissionStat,
}

impl NativeTestHost {
    fn new() -> Self {
        Self {
            simulation: crate::sim_rng::test_context(),

            entities: crate::entities::Entities::new(),
            ai_global: crate::ai::AiGlobalState::default(),
            fast_grid: crate::fast_find_grid::FastFindGrid::default(),
            globals: Vec::new(),
            state: ScriptState::default(),
            script_domains: crate::engine::ScriptDomains::default(),
            bindings: AttachedScriptBindings::default(),
            selected_pcs: Vec::new(),
            pc_registry_override: None,
            selected_action: crate::profiles::Action::NoAction,
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
                    .then_some(crate::gate::DoorIndex::new(idx as u32).expect("valid door index"))
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

impl HostFunctions for NativeTestHost {
    fn call(&mut self, index: u32, stack: &mut NativeStack) -> NativeCallOutcome {
        let pc_registry = self.pc_registry_override.clone().unwrap_or_else(|| {
            self.entities
                .pcs()
                .map(|(id, _)| EntityId::Pc(id))
                .collect()
        });
        let mut capabilities = NativeSessionCapabilities::new(
            &self.simulation,
            &mut self.entities,
            &mut self.ai_global,
            &mut self.fast_grid,
            &mut self.globals,
        )
        .with_pc_registry(&pc_registry)
        .with_world_views(&[], &[], &[])
        .with_queries(
            &mut self.sequence_manager,
            &mut self.selected_pcs,
            &mut self.sound_sources,
            &self.weather,
            &self.frame,
        )
        .with_selected_action(&mut self.selected_action)
        .with_short_briefings(&mut self.short_briefings)
        .with_standard_view_radius(&mut self.standard_view_radius)
        .with_campaign(&mut self.campaign, &mut self.mission_stat);
        NativeContext::with_bindings(
            &mut self.state,
            &mut self.script_domains,
            &self.bindings,
            &mut capabilities,
        )
        .call(index, stack)
    }
}

#[test]
fn finishing_recording_yields_the_complete_sequence() {
    let mut host = NativeTestHost::new();
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
        operation: crate::interp::NativeOperation::LaunchSequence(sequence),
        ..
    }) = outer
    else {
        panic!("Thanx must yield the recorded sequence for inline execution");
    };
    assert!(host.entity_at_legacy_slot(0).element_data().blipped);
    let sequence = host
        .sequence_manager
        .get_sequence(sequence)
        .expect("recorded sequence inserted");
    assert_eq!(sequence.elements.len(), 2);
    assert_eq!(sequence.elements[0].command, Command::SendMessage);
    assert_eq!(sequence.elements[1].command, Command::Unblip);
}

#[test]
fn recording_leaves_future_wait_priority_for_live_instruction() {
    use crate::sequence::{RecordingSession, SequenceElement, SequencePriority};

    let mut host = NativeTestHost::new();
    host.entities.push(Some(native_test_soldier()));
    let owner = host.entities.get_legacy_slot(0).unwrap().0;
    let mut recording = RecordingSession::new();
    recording.add_element(SequenceElement::new(1, Command::LockUser, None));
    recording.advance_level();
    recording.add_element(SequenceElement::new(2, Command::Wait, Some(owner)));
    let mut authored = SequenceElement::new(2, Command::Wait, Some(owner));
    authored.priority = SequencePriority::Script;
    recording.add_element(authored);
    host.state.sequence_recorder = Some(recording);
    let outcome = HostFunctions::call(
        &mut host,
        NativeFn::Thanx as u32,
        &mut NativeStack::default(),
    );
    let NativeCallOutcome::Yield(NativeYield {
        operation: NativeOperation::LaunchSequence(id),
        ..
    }) = outcome
    else {
        panic!("recording must enter engine sequence execution");
    };

    host.entity_at_legacy_slot_mut(0)
        .human_data_mut()
        .unwrap()
        .unconscious = true;
    let context = crate::element_priority::ActorPriorityContext::new(
        host.entity_at_legacy_slot(0).kind(),
        false,
        true,
    );
    let sequence = host.sequence_manager.get_sequence(id).unwrap();
    assert_eq!(sequence.elements[1].priority, SequencePriority::NotYetSet);
    assert_eq!(
        crate::element_priority::determine_priority(context, &sequence.elements[1]),
        SequencePriority::Ko
    );
    assert_eq!(
        crate::element_priority::determine_priority(context, &sequence.elements[2]),
        SequencePriority::Script
    );
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
    let stop = run_native(NativeFn::SendMessage as u32, &[0, 1234]);
    assert!(matches!(
        stop,
        StopReason::Yield(crate::interp::NativeYield {
            operation: crate::interp::NativeOperation::LaunchSequence(_),
            resume: crate::interp::ResumePolicy::Fixed(0),
        })
    ));

    let stop = run_native(
        NativeFn::SendMessageWithArguments as u32,
        &[0, 2345, -11, 22],
    );
    assert!(matches!(
        stop,
        StopReason::Yield(crate::interp::NativeYield {
            operation: crate::interp::NativeOperation::LaunchSequence(_),
            resume: crate::interp::ResumePolicy::Fixed(0),
        })
    ));
}

#[test]
fn sequence_recording_errors_and_empty_completion_leave_no_persisted_history() {
    assert!(
        serde_json::from_value::<ScriptState>(serde_json::json!({
            "computed_locations": []
        }))
        .is_err()
    );
    let mut host = NativeTestHost::new();
    let call = |host: &mut NativeTestHost, native: NativeFn| {
        HostFunctions::call(host, native as u32, &mut NativeStack::default())
            .expect_return("recording control is synchronous")
    };
    let baseline = robin_util::state_hash::compute(&host.state);
    assert_eq!(call(&mut host, NativeFn::Then), 0);
    assert_eq!(call(&mut host, NativeFn::Thanx), 0);
    assert_eq!(robin_util::state_hash::compute(&host.state), baseline);
    for _ in 0..2 {
        assert_eq!(call(&mut host, NativeFn::Start), 1);
        let recording_hash = robin_util::state_hash::compute(&host.state);
        assert_eq!(call(&mut host, NativeFn::Start), 0);
        assert_eq!(robin_util::state_hash::compute(&host.state), recording_hash);
        assert_eq!(call(&mut host, NativeFn::Then), 1);
        assert_eq!(call(&mut host, NativeFn::Thanx), 1);
        assert_eq!(robin_util::state_hash::compute(&host.state), baseline);
        assert!(serde_json::to_value(&host.state).unwrap()["sequence_recorder"].is_null());
        assert_eq!(host.sequence_manager.sequences_iter().count(), 0);
    }
}

#[test]
fn sequence_recording_continues_after_json_and_native_state_snapshots() {
    let call = |host: &mut NativeTestHost, native: NativeFn| {
        HostFunctions::call(host, native as u32, &mut NativeStack::default())
            .expect_return("recording control is synchronous")
    };
    let record_timer = |host: &mut NativeTestHost| {
        let mut args = NativeStack::default();
        args.push_i32(12);
        assert_eq!(
            HostFunctions::call(host, NativeFn::RecordTimer as u32, &mut args)
                .expect_return("record timer"),
            1
        );
    };
    let mut original = NativeTestHost::new();
    assert_eq!(call(&mut original, NativeFn::Start), 1);
    for level in [2, 3] {
        record_timer(&mut original);
        assert_eq!(call(&mut original, NativeFn::Then), level);
    }
    record_timer(&mut original);
    let hash = robin_util::state_hash::compute(&original.state);
    let json = serde_json::to_value(&original.state).unwrap();
    assert_eq!(json["sequence_recorder"]["command_level"], 3);
    assert!(json["sequence_recorder"].get("sequence_id").is_none());
    let json_state: ScriptState = serde_json::from_value(json).unwrap();
    let native_state: ScriptState = bitcode::decode(&bitcode::encode(&original.state)).unwrap();
    for state in [json_state, native_state] {
        let mut restored = NativeTestHost::new();
        restored.state = state;
        assert_eq!(robin_util::state_hash::compute(&restored.state), hash);
        assert_eq!(call(&mut restored, NativeFn::Start), 0);
        assert_eq!(call(&mut restored, NativeFn::Then), 4);
        assert_eq!(call(&mut restored, NativeFn::Then), 4);
        record_timer(&mut restored);
        let NativeCallOutcome::Yield(crate::interp::NativeYield {
            operation: crate::interp::NativeOperation::LaunchSequence(sequence),
            resume: crate::interp::ResumePolicy::Fixed(1),
        }) = HostFunctions::call(
            &mut restored,
            NativeFn::Thanx as u32,
            &mut NativeStack::default(),
        )
        else {
            panic!("Thanx must hand the complete recording to the engine");
        };
        assert_eq!(
            robin_util::state_hash::compute(&restored.state),
            robin_util::state_hash::compute(&ScriptState::default())
        );
        let sequence = restored
            .sequence_manager
            .get_sequence(sequence)
            .expect("recorded sequence inserted");
        assert_eq!(
            sequence
                .elements
                .iter()
                .map(|element| element.command_level)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
    }
}

#[test]
fn thanx_returns_true_for_an_empty_active_recording() {
    let mut host = NativeTestHost::new();
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
fn global_natives_share_allocated_slots_across_sessions() {
    let mut host = NativeTestHost::new();
    let mut stack = NativeStack::default();
    stack.push_i32(0);
    stack.push_i32(7);
    call_host_native(&mut host, NativeFn::InitGlobal, &mut stack);
    assert_eq!(host.globals.len(), 16);
    assert_eq!(host.globals[0], 7);

    let mut stack = NativeStack::default();
    stack.push_i32(1);
    assert_eq!(
        call_host_native(&mut host, NativeFn::GetGlobal, &mut stack),
        0
    );

    let mut stack = NativeStack::default();
    stack.push_i32(15);
    stack.push_i32(9);
    call_host_native(&mut host, NativeFn::SetGlobal, &mut stack);
    assert_eq!(host.globals[15], 9);

    let mut stack = NativeStack::default();
    stack.push_i32(16);
    stack.push_i32(11);
    call_host_native(&mut host, NativeFn::InitGlobal, &mut stack);
    assert_eq!(host.globals.len(), 32);
    assert_eq!(host.globals[15], 9);
    assert_eq!(host.globals[16], 11);
    assert_eq!(&host.globals[17..], &[0; 15]);

    let mut stack = NativeStack::default();
    stack.push_i32(0);
    stack.push_i32(13);
    call_host_native(&mut host, NativeFn::InitGlobal, &mut stack);
    assert_eq!(host.globals.len(), 32);
    assert_eq!(host.globals[0], 13);

    for id in [-1, 32] {
        let before = host.globals.clone();
        let mut stack = NativeStack::default();
        stack.push_i32(id);
        stack.push_i32(99);
        call_host_native(&mut host, NativeFn::SetGlobal, &mut stack);
        assert_eq!(
            host.globals, before,
            "invalid SetGlobal({id}) must not grow storage"
        );
    }
}

#[test]
fn scb_globals_access_initialized_padding_slots() {
    for (id, expected) in [(1, 0), (15, 9)] {
        let program = vec![
            BeginFunction {
                volatile_count: 0,
                temp_count: 3,
            },
            Aff0IConstant {
                dst: TMP0,
                constant: 0,
            },
            Aff0IConstant {
                dst: TMP4,
                constant: 7,
            },
            NativeParam { sym: TMP0 },
            NativeParam { sym: TMP4 },
            NativeCall {
                index: NativeFn::InitGlobal as u32,
            },
            Aff0IConstant {
                dst: TMP0,
                constant: 15,
            },
            Aff0IConstant {
                dst: TMP4,
                constant: 9,
            },
            NativeParam { sym: TMP0 },
            NativeParam { sym: TMP4 },
            NativeCall {
                index: NativeFn::SetGlobal as u32,
            },
            Aff0IConstant {
                dst: TMP0,
                constant: id,
            },
            NativeParam { sym: TMP0 },
            NativeCall {
                index: NativeFn::GetGlobal as u32,
            },
            Aff1NativeGetReturn { sym: TMP8 },
            ReturnVal { sym: TMP8 },
        ];
        let mut vm = Vm::new().with_host(Box::new(NativeTestHost::new()));
        assert_eq!(
            vm.run(&program),
            StopReason::ReturnedValue(expected),
            "allocated global slot {id}"
        );
    }
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
    let host = NativeTestHost::new();
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&program), StopReason::ReturnedValue(200));
}

#[test]
fn dialog_native_suspends_before_returning_to_the_vm() {
    assert!(matches!(
        run_native(NativeFn::StartDialog as u32, &[5]),
        StopReason::Yield(NativeYield {
            operation: NativeOperation::Command(NativeCommand::Engine(
                EngineCommand::StartDialog { dialog_id: 5 }
            )),
            resume: ResumePolicy::Fixed(0),
        })
    ));
}

#[test]
fn name_lookup() {
    assert_eq!(native_name(0), "InitGlobal");
    assert_eq!(native_name(17), "StartDialog");
    assert_eq!(native_name(74), "ThisActor");
    assert_eq!(native_name(999), "unknown");
}

#[test]
fn npc_custom_values_participate_in_state_hash() {
    let mut baseline = NativeTestHost::new();
    let mut same = NativeTestHost::new();
    let mut changed = NativeTestHost::new();
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
    let mut host = NativeTestHost::new();
    let mut door = Door {
        active: true,
        click_polygon: vec![(10.0, 10.0), (30.0, 10.0), (30.0, 30.0), (10.0, 30.0)],
        ..Default::default()
    };
    door.rebuild_click_bbox();
    host.script_domains.interactables.doors.push(door);

    assert_eq!(
        host.door_index_for_goal_sector(99, (20.0, 20.0)),
        Some(crate::gate::DoorIndex::new(0).expect("valid door index"))
    );
}

#[test]
fn door_mutation_is_visible_to_later_native_in_same_callback() {
    let mut host = NativeTestHost::new();
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
fn patch_native_completes_before_the_next_native_query() {
    let mut engine = crate::engine::EngineInner::new();
    let assets = crate::engine::LevelAssets::new();
    engine.script_domains.interactables.patches.push(Patch {
        active: true,
        initially_active: true,
        ..Default::default()
    });
    engine
        .scripts
        .install_mission(crate::engine::test_support::asm::empty_mission_script(
            "patch_native.scs",
        ));
    engine.scripts.attach_native_capabilities(&assets);
    let sim = crate::sim_rng::test_context();
    let patch = ScriptHandleCodec::patch_handle_from_index(0);
    assert_eq!(
        engine
            .call_external_native(TickCtx::new(&sim, &assets), "ApplyPatch", &[patch])
            .unwrap(),
        1
    );
    assert_eq!(
        engine
            .call_external_native(TickCtx::new(&sim, &assets), "IsPatchApplied", &[patch])
            .unwrap(),
        1
    );
}

#[test]
fn mission_ui_mutations_are_visible_in_same_callback() {
    let mut host = NativeTestHost::new();

    let mut set_outline = NativeStack::default();
    set_outline.push_i32(1);
    let command = call_host_command(&mut host, NativeFn::SetOutlineDisplay, &mut set_outline, 0);
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
        command,
        NativeCommand::Engine(EngineCommand::SetOutlineDisplay { display: true })
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
fn recorded_direct_gate_route_retains_pass_door_direction() {
    use crate::coordinates::MapPoint;
    use crate::gate::Door;
    use crate::sequence::{RecordingSession, SequenceElementData};

    // Seed-2M linux2/Profile_002/ExQuickSave/replay-001 reaches this path:
    // script RecordMove builds a direct route from sector_out to sector_in.
    // Its approach/exit geometry was correct, but the separately recorded
    // PassDoor used SequenceElementData's default direction (indirect).
    let mut host = NativeTestHost::new();
    host.entities = crate::entities::Entities::from_legacy_slots(vec![Some(native_test_soldier())]);
    host.script_domains.interactables.doors.push(Door {
        point_out: MapPoint::new(876.0, 879.0),
        point_in: MapPoint::new(859.0, 897.0),
        layer_out: 4,
        layer_in: 3,
        sector_out: crate::sector::SectorNumber::new(103),
        sector_in: crate::sector::SectorNumber::new(98),
        ..Door::default()
    });
    host.state.sequence_recorder = Some(RecordingSession::new());
    let actor = ScriptHandleCodec::actor_handle_from_index(0);

    {
        let mut capabilities = NativeSessionCapabilities::new(
            &host.simulation,
            &mut host.entities,
            &mut host.ai_global,
            &mut host.fast_grid,
            &mut host.globals,
        );
        let mut context = NativeContext::with_bindings(
            &mut host.state,
            &mut host.script_domains,
            &host.bindings,
            &mut capabilities,
        );
        assert!(context.append_move_to_sequence(SequenceMoveRequest {
            actor_handle: actor,
            action: crate::order::OrderType::WalkingUpright,
            source: SequenceMovePoint {
                position: (876.0, 879.0),
                sector: crate::position_interface::SectorHandle::new(103).unwrap(),
                layer: 4
            },
            goal: SequenceMovePoint {
                position: (859.0, 897.0),
                sector: crate::position_interface::SectorHandle::new(98).unwrap(),
                layer: 3
            },
            victim: None,
            tolerance: 0.0,
            initial_flags: MoveFlags::CALLED_BY_SCRIPT,
            speed_factor: 1.0
        }));
    }

    let pass = host
        .state
        .sequence_recorder
        .as_ref()
        .expect("recording remains open")
        .sequence
        .elements
        .iter()
        .find(|element| element.command == crate::element::Command::PassDoor)
        .expect("direct route records PassDoor");
    let SequenceElementData::Movement {
        destination,
        gate_id,
        direction,
        ..
    } = &pass.data
    else {
        panic!("recorded PassDoor must be movement data")
    };
    assert_eq!(*destination, MapPoint::new(859.0, 897.0));
    assert_eq!(
        *gate_id,
        Some(crate::gate::DoorIndex::new(0).expect("valid door index"))
    );
    assert_eq!(
        *direction, 1,
        "gate assignment copies the direct flag into the selected movement element"
    );
}

#[test]
fn recorded_move_recovers_exact_source_before_same_sector_comparison() {
    use crate::coordinates::{MapBBox, MapPoint};
    use crate::fast_find_grid::GridSector;
    use crate::position_interface::SectorHandle;
    use crate::sector::{SectorNumber, SectorType};
    use crate::sequence::{RecordingSession, SequenceElementData};

    let mut host = NativeTestHost::new();
    host.entities = crate::entities::Entities::from_legacy_slots(vec![Some(native_test_soldier())]);
    host.fast_grid.size_map(8, 8);
    host.fast_grid.allocate_layers(1);
    let arena = host.fast_grid.add_sector(
        GridSector {
            points: vec![
                MapPoint::new(0.0, 0.0),
                MapPoint::new(100.0, 0.0),
                MapPoint::new(100.0, 100.0),
                MapPoint::new(0.0, 100.0),
            ],
            bounding_box: MapBBox::from_coords(0.0, 0.0, 100.0, 100.0),
            sector_type: SectorType::MOTION | SectorType::AREA | SectorType::BUILDING,
            layer: 0,
            sector_number: SectorNumber::new(0),
            ..Default::default()
        },
        0,
    );
    host.state.sequence_recorder = Some(RecordingSession::new());
    let actor = ScriptHandleCodec::actor_handle_from_index(0);
    let source = SectorHandle::new(0).unwrap();
    let goal = source.with_arena_index(
        crate::fast_find_grid::SectorIndex::new(arena).expect("test arena index is valid"),
    );

    {
        let mut capabilities = NativeSessionCapabilities::new(
            &host.simulation,
            &mut host.entities,
            &mut host.ai_global,
            &mut host.fast_grid,
            &mut host.globals,
        );
        let mut context = NativeContext::with_bindings(
            &mut host.state,
            &mut host.script_domains,
            &host.bindings,
            &mut capabilities,
        );
        assert!(context.append_move_to_sequence(SequenceMoveRequest {
            actor_handle: actor,
            action: crate::order::OrderType::RunningUpright,
            source: SequenceMovePoint {
                position: (10.0, 10.0),
                sector: source,
                layer: 0
            },
            goal: SequenceMovePoint {
                position: (20.0, 20.0),
                sector: goal,
                layer: 0
            },
            victim: None,
            tolerance: 0.0,
            initial_flags: MoveFlags::CALLED_BY_SCRIPT,
            speed_factor: 1.0
        }));
        assert!(context.append_move_to_sequence(SequenceMoveRequest {
            actor_handle: actor,
            action: crate::order::OrderType::RunningUpright,
            source: SequenceMovePoint {
                position: (20.0, 20.0),
                sector: goal,
                layer: 0
            },
            goal: SequenceMovePoint {
                position: (30.0, 30.0),
                sector: source,
                layer: 0
            },
            victim: None,
            tolerance: 0.0,
            initial_flags: MoveFlags::CALLED_BY_SCRIPT,
            speed_factor: 1.0
        }));
    }

    let elements = &host
        .state
        .sequence_recorder
        .as_ref()
        .expect("recording remains open")
        .sequence
        .elements;
    assert_eq!(elements.len(), 2);
    assert_eq!(elements[0].command, crate::element::Command::Move);
    let SequenceElementData::Movement { destination, .. } = &elements[0].data else {
        panic!("recorded Move must retain movement data")
    };
    assert_eq!(*destination, MapPoint::new(20.0, 20.0));
    assert_eq!(elements[1].command, crate::element::Command::Move);
    let SequenceElementData::Movement { destination, .. } = &elements[1].data else {
        panic!("second recorded Move must retain movement data")
    };
    assert_eq!(*destination, MapPoint::new(30.0, 30.0));
}

#[test]
fn recorded_move_retains_exact_four_gate_pointer_route_with_numeric_legacy_control() {
    use crate::coordinates::MapPoint;
    use crate::fast_find_grid::SectorIndex;
    use crate::gate::{Door, build_gate_links};
    use crate::position_interface::SectorHandle;
    use crate::sequence::{RecordingSession, SequenceElementData};

    fn handle(public: u16, arena: u32, exact: bool) -> SectorHandle {
        let handle = SectorHandle::new(public).unwrap();
        if exact {
            handle.with_arena_index(SectorIndex::new(arena).unwrap())
        } else {
            handle
        }
    }

    for exact in [true, false] {
        let mut host = NativeTestHost::new();
        host.entities =
            crate::entities::Entities::from_legacy_slots(vec![Some(native_test_soldier())]);
        host.entities
            .get_legacy_slot_mut(0)
            .unwrap()
            .1
            .element_data_mut()
            .set_position_map(MapPoint::new(0.0, 0.0));
        host.entities
            .get_legacy_slot_mut(0)
            .unwrap()
            .1
            .element_data_mut()
            .sprite
            .position_iface
            .set_sector_topology(
                Some(handle(103, 10_010, exact)),
                exact.then(|| SectorIndex::new(10_010).unwrap()),
            );
        let mut doors = (0..123)
            .map(|index| Door {
                active: false,
                sector_out: crate::sector::SectorNumber::new(1000 + index),
                sector_in: crate::sector::SectorNumber::new(2000 + index),
                sector_out_index: exact.then(|| SectorIndex::new(1000 + index as u32).unwrap()),
                sector_in_index: exact.then(|| SectorIndex::new(2000 + index as u32).unwrap()),
                ..Door::default()
            })
            .collect::<Vec<_>>();
        let sectors = [
            (103, 10_010),
            (66, 10_011),
            (66, 10_012),
            (72, 10_013),
            (89, 10_014),
        ];
        let public = |index: usize| {
            if !exact && index == 2 {
                67
            } else {
                sectors[index].0
            }
        };
        let make_door = |out: usize, inside: usize, direct_points: bool| Door {
            active: true,
            point_out: MapPoint::new(out as f32 * 20.0, 0.0),
            point_in: MapPoint::new(inside as f32 * 20.0, 0.0),
            point_mid: MapPoint::new((out + inside) as f32 * 10.0, 0.0),
            sector_out: crate::sector::SectorNumber::new(public(out)),
            sector_in: crate::sector::SectorNumber::new(public(inside)),
            sector_out_index: exact.then(|| SectorIndex::new(sectors[out].1).unwrap()),
            sector_in_index: exact.then(|| SectorIndex::new(sectors[inside].1).unwrap()),
            layer_out: if direct_points {
                out as u16
            } else {
                inside as u16
            },
            layer_in: if direct_points {
                inside as u16
            } else {
                out as u16
            },
            ..Door::default()
        };
        doors[118] = make_door(0, 1, true); // direct
        doors[117] = make_door(2, 1, false); // indirect: 1 -> 2
        doors[122] = make_door(2, 3, true); // direct
        doors[120] = make_door(4, 3, false); // indirect: 3 -> 4
        build_gate_links(&mut doors);
        let auth = host
            .entities
            .get_legacy_slot(0)
            .unwrap()
            .1
            .actor_auth_info();
        let expected_path = crate::gate::find_path_gates_with_sector_indices(
            &doors,
            (0.0, 0.0),
            103,
            exact.then(|| SectorIndex::new(10_010).unwrap()),
            (80.0, 0.0),
            89,
            exact.then(|| SectorIndex::new(10_014).unwrap()),
            Some(&auth),
            false,
            &|_| true,
            &|_| None,
        )
        .unwrap();
        assert_eq!(
            expected_path
                .iter()
                .map(|step| u32::from(step.door_index))
                .collect::<Vec<_>>(),
            vec![118, 117, 122, 120]
        );
        host.script_domains.interactables.doors = doors;
        host.state.sequence_recorder = Some(RecordingSession::new());
        let actor = ScriptHandleCodec::actor_handle_from_index(0);
        host.bindings.script_location_count = 1;
        host.bindings.script_point_count = 1;
        host.bindings.location_positions = std::sync::Arc::new(vec![(80.0, 0.0)]);
        host.bindings.location_layers = std::sync::Arc::new(vec![2]);
        host.bindings.location_sectors = std::sync::Arc::new(vec![89]);
        host.bindings.location_sector_handles =
            std::sync::Arc::new(vec![exact.then(|| handle(89, 10_014, true))]);
        assert!(
            crate::engine::current_door_for_route_source(
                host.entities.get_legacy_slot(0).unwrap().1
            )
            .is_none()
        );
        assert_eq!(
            host.entities
                .get_legacy_slot(0)
                .unwrap()
                .1
                .element_data()
                .sector()
                .unwrap()
                .arena_index(),
            exact.then(|| SectorIndex::new(10_010).unwrap())
        );
        assert_eq!(
            host.entities
                .get_legacy_slot(0)
                .unwrap()
                .1
                .element_data()
                .position_map(),
            MapPoint::new(0.0, 0.0)
        );
        let location = ScriptHandleCodec::location_handle_from_index(0);
        {
            let mut capabilities = NativeSessionCapabilities::new(
                &host.simulation,
                &mut host.entities,
                &mut host.ai_global,
                &mut host.fast_grid,
                &mut host.globals,
            );
            let context = NativeContext::with_bindings(
                &mut host.state,
                &mut host.script_domains,
                &host.bindings,
                &mut capabilities,
            );
            assert_eq!(
                context
                    .resolve_location_layer_sector_handle(location)
                    .unwrap()
                    .1
                    .arena_index(),
                exact.then(|| SectorIndex::new(10_014).unwrap())
            );
        }
        let mut stack = NativeStack::default();
        stack.push_i32(actor);
        stack.push_i32(location);
        stack.push_i32(0);
        assert_eq!(
            call_host_native(&mut host, NativeFn::RecordMove, &mut stack),
            1
        );
        let recording = host.state.sequence_recorder.as_ref().unwrap();
        assert_eq!(
            recording.moving_actors[&actor].sector.arena_index(),
            exact.then(|| SectorIndex::new(10_014).unwrap())
        );
        let gate_ids = recording
            .sequence
            .elements
            .iter()
            .filter_map(|element| match &element.data {
                SequenceElementData::Movement { gate_id, .. } => *gate_id,
                _ => None,
            })
            .map(u32::from)
            .collect::<Vec<_>>();
        assert_eq!(
            gate_ids,
            vec![118, 117, 122, 120],
            "recorded commands: {:?}",
            recording
                .sequence
                .elements
                .iter()
                .map(|element| element.command)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
#[should_panic(expected = "script-recorded gate path references missing door 7")]
fn recorded_gate_path_missing_door_is_an_invariant_failure() {
    super::script_gate_path_door(
        &[],
        crate::gate::GatePathStep {
            door_index: crate::gate::DoorIndex::new(7).expect("valid door index"),
            direct: true,
        },
    );
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
    let host = NativeTestHost::new();
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
        element: {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::Fx;
            initial_element.active = true;
            initial_element
        },
        fx: crate::element::FxData {
            mobile_index: Some(mobile_index),
            ..Default::default()
        },
    })
}

#[test]
fn mobile_master_is_appended_to_script_actor_indices() {
    let mut host = NativeTestHost::new();
    host.entities
        .push(Some(Entity::Fx(crate::element::ElementFx {
            element: {
                let mut initial_element = crate::element::ElementData::default();
                initial_element.kind = crate::element::ElementKind::Fx;
                initial_element
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
fn mobile_activation_yields_the_canonical_master_operation() {
    let mut host = NativeTestHost::new();
    host.entities.push(Some(mobile_fx(0)));
    host.entities.push(Some(mobile_fx(0)));
    let handle = ScriptHandleCodec::actor_handle_from_index(0);

    let mut deactivate = NativeStack::default();
    deactivate.push_i32(handle);
    let command = call_host_command(&mut host, NativeFn::Deactivate, &mut deactivate, 1);
    assert!(
        host.entities
            .occupied()
            .all(|(_, entity)| entity.is_active())
    );
    assert!(matches!(
        command,
        NativeCommand::Engine(EngineCommand::SetMobileActive {
            mobile_index: 0,
            active: false
        })
    ));
}

#[test]
fn activating_a_rescue_pc_makes_it_player_controllable() {
    use crate::human_control::{CombatStance, CommandInterface, MissionRole};

    let mut prisoner = native_test_pc(Vec::new(), Vec::new());
    let pc = prisoner.pc_data_mut().expect("rescue fixture must be a PC");
    pc.playable = false;
    pc.command_interface = CommandInterface::None;
    pc.mission_role = MissionRole::RescueTarget;
    pc.combat_stance = CombatStance::Defensive;

    let mut engine = crate::engine::EngineInner::new();
    let owner = engine.add_test_entity(prisoner);
    let mut assets = crate::engine::LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    engine
        .scripts
        .install_mission(crate::engine::test_support::asm::empty_mission_script(
            "rescue_native.scs",
        ));
    engine.scripts.attach_native_capabilities(&assets);
    engine
        .call_external_native(
            TickCtx::new(&crate::sim_rng::test_context(), &assets),
            "Activate",
            &[ScriptHandleCodec::actor_handle(owner)],
        )
        .unwrap();
    let pc = engine.pc(owner);
    assert!(pc.playable);
    assert_eq!(pc.command_interface, CommandInterface::HeroActions);
    assert_eq!(pc.mission_role, MissionRole::PlayerParty);
    assert_eq!(pc.combat_stance, CombatStance::Aggressive);
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
    // The global actor lookup returns no value, represented by handle zero.
    assert_eq!(run_native(111, &[]), StopReason::ReturnedValue(0));
}

#[test]
fn stop_actor_unknown_handle_noop() {
    // Invalid handle → warn, no deferred command.
    let stop = run_native(103, &[5]);
    assert_eq!(stop, StopReason::ReturnedValue(0));
}

#[test]
fn select_select_all_yields_before_vm_continues() {
    assert!(matches!(
        run_native(112, &[31]),
        StopReason::Yield(NativeYield {
            operation: NativeOperation::Command(NativeCommand::World(
                WorldNativeCommand::SelectPC {
                    actor: 0,
                    select: true
                }
            )),
            resume: ResumePolicy::Fixed(1),
        })
    ));
}

#[test]
fn select_unselect_all_yields_before_vm_continues() {
    assert!(matches!(
        run_native(112, &[0]),
        StopReason::Yield(NativeYield {
            operation: NativeOperation::Command(NativeCommand::World(
                WorldNativeCommand::SelectPC {
                    actor: 0,
                    select: false
                }
            )),
            resume: ResumePolicy::Fixed(1),
        })
    ));
}

#[test]
fn select_unknown_code_warns_but_no_command() {
    let stop = run_native(112, &[5]);
    assert_eq!(stop, StopReason::ReturnedValue(1));
}

#[test]
fn select_all_and_unselect_all_are_immediately_query_visible() {
    let mut engine = crate::engine::EngineInner::new();
    for _ in 0..2 {
        engine.add_test_entity(native_test_pc(Vec::new(), Vec::new()));
    }
    let mut assets = crate::engine::LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    engine
        .scripts
        .install_mission(crate::engine::test_support::asm::empty_mission_script(
            "select_all.scs",
        ));
    engine.scripts.attach_native_capabilities(&assets);
    let sim = crate::sim_rng::test_context();
    assert_eq!(
        engine
            .call_external_native(TickCtx::new(&sim, &assets), "Select", &[31])
            .unwrap(),
        1
    );
    assert_eq!(
        engine
            .call_external_native(TickCtx::new(&sim, &assets), "GetNumberOfSelectedPCs", &[])
            .unwrap(),
        2
    );
    assert_eq!(
        engine
            .call_external_native(TickCtx::new(&sim, &assets), "Select", &[0])
            .unwrap(),
        1
    );
    assert_eq!(
        engine
            .call_external_native(TickCtx::new(&sim, &assets), "GetNumberOfSelectedPCs", &[])
            .unwrap(),
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
fn freeze_all_yields_before_vm_continues() {
    assert!(matches!(
        run_native(139, &[1]),
        StopReason::Yield(NativeYield {
            operation: NativeOperation::Command(NativeCommand::World(
                WorldNativeCommand::FreezeAll { freeze: true }
            )),
            resume: ResumePolicy::Fixed(0),
        })
    ));
}

#[test]
fn freeze_all_unfreeze_yields_before_vm_continues() {
    assert!(matches!(
        run_native(139, &[0]),
        StopReason::Yield(NativeYield {
            operation: NativeOperation::Command(NativeCommand::World(
                WorldNativeCommand::FreezeAll { freeze: false }
            )),
            resume: ResumePolicy::Fixed(0),
        })
    ));
}

// --- Location / distance ---

#[test]
fn nowhere_returns_zero() {
    assert_eq!(run_native(159, &[]), StopReason::ReturnedValue(0));
}

#[test]
fn get_distance_with_positions() {
    let host = NativeTestHost::new();
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
    let mut vm = Vm::new().with_host(Box::new(NativeTestHost { bindings, ..host }));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(50)); // sqrt(30²+40²)=50
}

#[test]
fn get_distance_invalid_handle() {
    assert_eq!(run_native(160, &[99, 100]), StopReason::ReturnedValue(0));
}

#[test]
fn camera_commands_copy_static_and_vm_local_computed_points() {
    let mut host = NativeTestHost::new();
    host.state
        .computed_locations
        .push(Some(ComputedScriptLocation {
            position: (90.0, 123.0),
            layer: Some(2),
            sector: Some(44),
            sector_handle: None,
            active: true,
        }));
    let bindings = AttachedScriptBindings {
        script_location_count: 1,
        script_point_count: 1,
        location_positions: std::sync::Arc::new(vec![(12.0, 34.0)]),
        ..Default::default()
    };

    host.bindings = bindings;
    let mut jump = NativeStack::default();
    jump.push_i32(ScriptHandleCodec::location_handle_from_index(1));
    let jump_command = call_host_command(&mut host, NativeFn::JumpCameraTo, &mut jump, 0);

    let mut scroll = NativeStack::default();
    scroll.push_i32(ScriptHandleCodec::location_handle_from_index(0));
    scroll.push_i32(0.75_f32.to_bits() as i32);
    let scroll_command =
        call_host_command(&mut host, NativeFn::ScrollCameraSlowlyTo, &mut scroll, 0);

    assert!(matches!(
        jump_command,
        NativeCommand::Engine(EngineCommand::JumpCameraTo { x: 90.0, y: 123.0 })
    ));
    assert!(matches!(
        scroll_command,
        NativeCommand::Engine(EngineCommand::ScrollCameraTo {
            x: 12.0,
            y: 34.0,
            speed: 0.75
        })
    ));
}

#[test]
fn is_inside_building_specific() {
    let mut host = NativeTestHost::new();
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
    let mut host = NativeTestHost::new();
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
    let mut host = NativeTestHost::new();
    let actor = ScriptHandleCodec::actor_handle_from_index(4);
    host.script_domains
        .buildings
        .actor_building
        .insert(actor, ScriptHandleCodec::building_handle_from_index(2));
    // No building specified (0): checks if in any building
    let prog = call_native_return(98, &[actor, 0]);
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(1));
}

#[test]
fn is_inside_building_not_in_any() {
    let host = NativeTestHost::new();
    let prog = call_native_return(98, &[ScriptHandleCodec::actor_handle_from_index(4), 0]);
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(0));
}

#[test]
fn is_inside_zone() {
    let mut host = NativeTestHost::new();
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
    let mut host = NativeTestHost::new();
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

fn geometric_is_inside_host(actor_sector: u16, actor_layer: Option<u16>) -> NativeTestHost {
    let mut actor = native_test_soldier();
    let element = actor.element_data_mut();
    element.set_position_map(crate::coordinates::MapPoint::new(5.0, 5.0));
    if let Some(layer) = actor_layer {
        element.set_layer(layer);
    } else {
        element.clear_layer();
    }
    element.set_sector(crate::position_interface::SectorHandle::new(actor_sector));

    let points = vec![
        crate::coordinates::MapPoint::new(0.0, 0.0),
        crate::coordinates::MapPoint::new(10.0, 0.0),
        crate::coordinates::MapPoint::new(10.0, 10.0),
        crate::coordinates::MapPoint::new(0.0, 10.0),
    ];
    let mut bounding_box = crate::coordinates::MapBBox::new();
    for &point in &points {
        bounding_box.expand_point(point);
    }

    let mut host = NativeTestHost::new();
    host.entities = crate::entities::Entities::from_legacy_slots(vec![Some(actor)]);
    std::sync::Arc::make_mut(&mut host.fast_grid.level)
        .sectors
        .push(crate::fast_find_grid::GridSector {
            points,
            bounding_box,
            sector_type: crate::sector::SectorType::SCRIPT,
            layer: 0,
            sector_number: crate::sector::SectorNumber::new(-1),
            ..Default::default()
        });
    host.script_domains
        .zones
        .scripts
        .push(crate::sector::ScriptSectorData {
            owning_motion_sector: crate::sector::SectorNumber::new(7),
            ..Default::default()
        });
    host.bindings = AttachedScriptBindings {
        script_point_count: 1,
        script_location_count: 2,
        script_zone_grid_indices: std::sync::Arc::new(vec![0]),
        ..Default::default()
    };
    host
}

#[test]
fn is_inside_zone_geometry_requires_owning_motion_sector() {
    let actor = ScriptHandleCodec::actor_handle_from_index(0);
    let zone = ScriptHandleCodec::location_handle_from_index(1);
    let program = call_native_return(NativeFn::IsInside as u32, &[actor, zone]);

    let matching_host = geometric_is_inside_host(7, Some(0));
    assert_eq!(
        matching_host.fast_grid.level.sectors[0].sector_number,
        crate::sector::SectorNumber::new(-1),
        "script geometry must not claim the owning motion-sector identity"
    );
    assert_eq!(
        matching_host.script_domains.zones.scripts[0].owning_motion_sector,
        crate::sector::SectorNumber::new(7)
    );
    let mut matching = Vm::new().with_host(Box::new(matching_host));
    assert_eq!(matching.run(&program), StopReason::ReturnedValue(1));

    // The point is geometrically inside, but the original game rejects
    // actors whose current motion sector differs from the script zone's.
    let mut adjacent = Vm::new().with_host(Box::new(geometric_is_inside_host(8, Some(0))));
    assert_eq!(adjacent.run(&program), StopReason::ReturnedValue(0));
}

#[test]
fn is_inside_zone_rejects_original_no_layer_sentinel_without_panicking() {
    let actor = ScriptHandleCodec::actor_handle_from_index(0);
    let zone = ScriptHandleCodec::location_handle_from_index(1);
    let program = call_native_return(NativeFn::IsInside as u32, &[actor, zone]);

    // Original-game sector containment compares layers first. Its
    // 0xffff element sentinel differs from this layer-0 zone, so geometry and
    // sector identity are never consulted.
    let mut vm = Vm::new().with_host(Box::new(geometric_is_inside_host(7, None)));
    assert_eq!(vm.run(&program), StopReason::ReturnedValue(0));
}

#[test]
fn actors_in_sector() {
    // GetNumberOfActorsInSector / GetActorInSector reject non-sector
    // handles via `is_script_sector_handle` (sector handles live in
    // `script_point_count < loc <= script_location_count`), so seed
    // counts so loc=2 is a valid sector handle.
    let mut host = NativeTestHost::new();
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
    let mut vm = Vm::new().with_host(Box::new(NativeTestHost { bindings, ..host }));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(3));

    // Re-add occupants since vm takes ownership
    let mut host2 = NativeTestHost::new();
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
    let mut vm2 = Vm::new().with_host(Box::new(NativeTestHost {
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
    let host = NativeTestHost::new();
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
    let mut vm = Vm::new().with_host(Box::new(NativeTestHost { bindings, ..host }));
    // Should return a handle >= 3 (first computed location)
    match vm.run(&prog) {
        StopReason::ReturnedValue(handle) => {
            assert_eq!(ScriptHandleCodec::location_index(handle), Some(2));
        }
        other => panic!("expected return, got {other:?}"),
    }
}

#[test]
fn get_actor_location_retains_exact_sector_for_later_record_move() {
    let mut host = NativeTestHost::new();
    let mut soldier = native_test_soldier();
    let arena = crate::fast_find_grid::SectorIndex::new(91).unwrap();
    soldier
        .element_data_mut()
        .sprite
        .position_iface
        .set_sector_topology(
            crate::position_interface::SectorHandle::new(44),
            Some(arena),
        );
    host.entities.push(Some(soldier));
    let actor = ScriptHandleCodec::actor_handle_from_index(0);
    let mut stack = NativeStack::default();
    stack.push_i32(actor);
    let location = call_host_native(&mut host, NativeFn::GetActorLocation, &mut stack);
    let index = ScriptHandleCodec::location_index(location).unwrap();
    let computed = host.state.computed_locations[index - host.bindings.script_location_count]
        .as_ref()
        .unwrap();
    assert_eq!(computed.sector_handle.unwrap().arena_index(), Some(arena));
}

#[test]
fn get_actor_location_accepts_live_no_layer_element_without_fabricating_metadata() {
    let mut host = NativeTestHost::new();
    let mut soldier = native_test_soldier();
    soldier
        .element_data_mut()
        .sprite
        .position_iface
        .clear_layer();
    soldier
        .element_data_mut()
        .sprite
        .position_iface
        .set_sector(None);
    host.entities.push(Some(soldier));
    let actor = ScriptHandleCodec::actor_handle_from_index(0);
    let mut stack = NativeStack::default();
    stack.push_i32(actor);

    let location = call_host_native(&mut host, NativeFn::GetActorLocation, &mut stack);
    let index = ScriptHandleCodec::location_index(location).unwrap();
    let computed = host.state.computed_locations[index - host.bindings.script_location_count]
        .as_ref()
        .unwrap();
    assert_eq!(computed.layer, None);
    assert_eq!(computed.sector, None);
    assert_eq!(computed.sector_handle, None);
}

#[test]
#[should_panic(expected = "exact position provenance is required")]
fn record_move_rejects_mixed_exact_source_and_legacy_number_only_computed_goal() {
    let mut host = NativeTestHost::new();
    let mut soldier = native_test_soldier();
    let source_arena = crate::fast_find_grid::SectorIndex::new(90).unwrap();
    soldier
        .element_data_mut()
        .sprite
        .position_iface
        .set_sector_topology(
            crate::position_interface::SectorHandle::new(44),
            Some(source_arena),
        );
    host.entities.push(Some(soldier));
    host.state.sequence_recorder = Some(crate::sequence::RecordingSession::new());
    host.state
        .computed_locations
        .push(Some(ComputedScriptLocation {
            position: (10.0, 10.0),
            layer: Some(0),
            sector: Some(44),
            sector_handle: None,
            active: true,
        }));
    let actor = ScriptHandleCodec::actor_handle_from_index(0);
    let location = ScriptHandleCodec::location_handle_from_index(0);
    let mut stack = NativeStack::default();
    stack.push_i32(actor);
    stack.push_i32(location);
    stack.push_i32(0);
    let _ = call_host_native(&mut host, NativeFn::RecordMove, &mut stack);
}

#[test]
fn are_all_pcs_inside() {
    let mut host = NativeTestHost::new();
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
    let mut host = NativeTestHost::new();
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
fn are_all_pcs_inside_ignores_pc_retired_from_original_registry() {
    let mut host = NativeTestHost::new();
    host.entities = crate::entities::Entities::from_legacy_slots(vec![
        Some(native_test_pc(Vec::new(), Vec::new())),
        Some(native_test_pc(Vec::new(), Vec::new())),
    ]);
    let active = host
        .entities
        .get_legacy_slot(1)
        .expect("active replacement PC")
        .0;
    host.pc_registry_override = Some(vec![active]);

    let active_handle = ScriptHandleCodec::actor_handle(active);
    let loc = ScriptHandleCodec::location_handle_from_index(0);
    seed_zone(&mut host, 0, &[active_handle]);
    let prog = call_native_return(230, &[loc]);
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&prog), StopReason::ReturnedValue(1));
}

#[test]
fn register_production_sector() {
    let mut host = NativeTestHost::new();
    host.bindings.script_point_count = 1;
    host.bindings.script_location_count = 2;
    host.bindings.location_positions = std::sync::Arc::new(vec![(12.0, 34.0), (20.0, 40.0)]);
    host.bindings.location_layers = std::sync::Arc::new(vec![2, 2]);
    host.bindings.location_sectors = std::sync::Arc::new(vec![7, 7]);
    let exact_sector = crate::position_interface::SectorHandle::new(7)
        .unwrap()
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(0).unwrap());
    host.bindings.location_sector_handles =
        std::sync::Arc::new(vec![Some(exact_sector), Some(exact_sector)]);
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
    assert_eq!(saved.obstacle, None);
}

#[test]
fn make_noise_preserves_the_script_points_exact_sector_identity() {
    let mut host = NativeTestHost::new();
    host.bindings.script_location_count = 1;
    host.bindings.script_point_count = 1;
    // H01_Lin_VL Node_15: DrawBridgeRoom passes this point to MakeNoise.
    host.bindings.location_positions = std::sync::Arc::new(vec![(1135.0, 1843.0)]);
    host.bindings.location_layers = std::sync::Arc::new(vec![0]);
    host.bindings.location_sectors = std::sync::Arc::new(vec![24]);
    // Deliberately use a sparse arena slot which differs from the public
    // sector number: reducing this to `u16` would lose the identity that
    // The original game carries this in the position's sector reference.
    let exact_sector = crate::position_interface::SectorHandle::new(24)
        .unwrap()
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(10_024).unwrap());
    host.bindings.location_sector_handles = std::sync::Arc::new(vec![Some(exact_sector)]);

    let mut stack = NativeStack::default();
    stack.push_i32(ScriptHandleCodec::location_handle_from_index(0));
    stack.push_i32(1);
    let command = call_host_command(&mut host, NativeFn::MakeNoise, &mut stack, 0);
    assert!(matches!(
        command,
        NativeCommand::Engine(EngineCommand::MakeNoise {
            noise_type: crate::ai::NoiseType::Drawbridge,
            x: 1135.0,
            y: 1843.0,
            layer: 0,
            sector,
        }) if sector == exact_sector
    ));
}

#[test]
#[should_panic(expected = "is already attached")]
fn production_sector_rejects_duplicate_attachment() {
    let mut host = NativeTestHost::new();
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
    let mut host = NativeTestHost::new();
    host.bindings.script_point_count = 1;
    host.bindings.script_location_count = 1;
    host.bindings.location_positions = std::sync::Arc::new(vec![(12.0, 34.0)]);
    host.bindings.location_layers = std::sync::Arc::new(vec![2]);
    host.bindings.location_sectors = std::sync::Arc::new(vec![7]);
    host.bindings.location_sector_handles = std::sync::Arc::new(vec![Some(
        crate::position_interface::SectorHandle::new(7)
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(0).unwrap()),
    )]);
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
    let host = CampaignNativeHost {
        entities: crate::entities::Entities::new(),
        ai_global: crate::ai::AiGlobalState::default(),
        fast_grid: crate::fast_find_grid::FastFindGrid::default(),
        globals: Vec::new(),
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
    let mut host = NativeTestHost::new();
    host.entities.push(Some(native_test_soldier()));
    let mut vm = Vm::new().with_host(Box::new(host));
    assert_eq!(vm.run(&program), StopReason::ReturnedValue(77));
}

#[test]
fn custom_values_are_isolated_between_script_hosts() {
    fn set_campaign(
        campaign: &mut crate::campaign::Campaign,
        stat: &mut crate::mission_stat::MissionStat,
        value: i32,
    ) {
        let mut stack = NativeStack::default();
        stack.push_i32(3);
        stack.push_i32(value);
        assert_eq!(
            call_campaign_native(campaign, stat, NativeFn::SetCustomCampaignValue, &mut stack,),
            0
        );
    }

    fn get_campaign(
        campaign: &mut crate::campaign::Campaign,
        stat: &mut crate::mission_stat::MissionStat,
    ) -> i32 {
        let mut stack = NativeStack::default();
        stack.push_i32(3);
        call_campaign_native(campaign, stat, NativeFn::GetCustomCampaignValue, &mut stack)
    }

    let mut first_campaign = crate::campaign::Campaign::default();
    let mut first_stat = crate::mission_stat::MissionStat::default();

    let mut second_campaign = crate::campaign::Campaign::default();
    let mut second_stat = crate::mission_stat::MissionStat::default();

    set_campaign(&mut first_campaign, &mut first_stat, 11);
    set_campaign(&mut second_campaign, &mut second_stat, 22);

    assert_eq!(get_campaign(&mut first_campaign, &mut first_stat), 11);
    assert_eq!(get_campaign(&mut second_campaign, &mut second_stat), 22);
}

#[test]
fn selection_native_completes_portrait_cleanup_before_later_queries() {
    let mut engine = crate::engine::EngineInner::new();
    let target = engine.add_test_entity(native_test_pc(Vec::new(), Vec::new()));
    let previous = engine.add_test_entity(native_test_pc(Vec::new(), Vec::new()));
    engine.players.seats[0].selection = vec![previous];
    engine.pc_mut(previous).portrait.open = true;
    let mut assets = crate::engine::LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    engine
        .scripts
        .install_mission(crate::engine::test_support::asm::empty_mission_script(
            "selection.scs",
        ));
    engine.scripts.attach_native_capabilities(&assets);
    let sim = crate::sim_rng::test_context();
    let target_handle = ScriptHandleCodec::actor_handle(target);
    engine
        .call_external_native(
            TickCtx::new(&sim, &assets),
            "SelectActorPC",
            &[target_handle, 1],
        )
        .unwrap();
    assert_eq!(engine.players.seats[0].selection, [target]);
    assert!(!engine.pc(previous).portrait.open);
    assert_eq!(
        engine
            .call_external_native(
                TickCtx::new(&sim, &assets),
                "IsPCSelected",
                &[target_handle]
            )
            .unwrap(),
        1
    );
    assert_eq!(
        engine
            .call_external_native(
                TickCtx::new(&sim, &assets),
                "IsPCSelected",
                &[ScriptHandleCodec::actor_handle(previous)]
            )
            .unwrap(),
        0
    );
}

#[test]
fn ai_lock_yields_before_the_script_can_launch_replacement_work() {
    let (actor, outcome) = lone_soldier_native_outcome(NativeFn::LockAI, 1);
    assert!(matches!(
        outcome,
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
fn assign_path_yields_until_return_to_duty_finishes() {
    let (actor, outcome) = lone_soldier_native_outcome(NativeFn::AssignPath, 7);
    assert!(matches!(
        outcome,
        NativeCallOutcome::Yield(crate::interp::NativeYield {
            operation: crate::interp::NativeOperation::EngineAction(
                crate::interp::SynchronousScriptRequest::AssignPath {
                    actor: yielded_actor,
                    way: 7,
                    native_return: 0,
                },
            ),
            resume: crate::interp::ResumePolicy::Fixed(0),
        }) if yielded_actor == actor
    ));
}

#[test]
fn stare_actor_preserves_entity_slot_zero_and_turn_flag() {
    let mut host = NativeTestHost::new();
    host.entities
        .push(Some(native_test_pc(Vec::new(), Vec::new())));
    host.entities.push(Some(native_test_soldier()));
    let target = ScriptHandleCodec::actor_handle_from_index(0);
    let actor = ScriptHandleCodec::actor_handle_from_index(1);

    let mut stack = NativeStack::default();
    stack.push_i32(actor);
    stack.push_i32(target);
    stack.push_i32(1);
    assert!(matches!(
        HostFunctions::call(&mut host, NativeFn::StareActor as u32, &mut stack),
        NativeCallOutcome::Yield(crate::interp::NativeYield {
            operation: crate::interp::NativeOperation::EngineAction(
                crate::interp::SynchronousScriptRequest::StareActor {
                    actor: yielded_actor,
                    target: crate::entity_id::EntityId::Pc(crate::entity_id::PcId(0)),
                    turn_sprite: true,
                    native_return: 0,
                },
            ),
            resume: crate::interp::ResumePolicy::Fixed(0),
        }) if yielded_actor == actor
    ));
}

#[test]
fn assign_post_yield_preserves_the_script_points_position_topology() {
    let mut host = NativeTestHost::new();
    host.entities.push(Some(native_test_soldier()));
    host.bindings.script_location_count = 1;
    host.bindings.script_point_count = 1;
    host.bindings.location_positions = std::sync::Arc::new(vec![(780.0, 995.0)]);
    host.bindings.location_layers = std::sync::Arc::new(vec![3]);
    host.bindings.location_sectors = std::sync::Arc::new(vec![97]);
    let source_arena = crate::fast_find_grid::SectorIndex::new(89).unwrap();
    let middle_arena = crate::fast_find_grid::SectorIndex::new(72).unwrap();
    let goal_arena = crate::fast_find_grid::SectorIndex::new(97).unwrap();
    let exact_sector = crate::position_interface::SectorHandle::new(97)
        .unwrap()
        .with_arena_index(goal_arena);
    host.bindings.location_sector_handles = std::sync::Arc::new(vec![Some(exact_sector)]);

    let mut doors = vec![crate::gate::Door::default(); 123];
    doors[120] = crate::gate::Door {
        active: true,
        point_out: crate::coordinates::MapPoint::new(608.0, 1193.0),
        point_in: crate::coordinates::MapPoint::new(700.0, 1100.0),
        sector_out: crate::sector::SectorNumber::new(89),
        sector_in: crate::sector::SectorNumber::new(72),
        sector_out_index: Some(source_arena),
        sector_in_index: Some(middle_arena),
        ..crate::gate::Door::default()
    };
    doors[122] = crate::gate::Door {
        active: true,
        point_out: crate::coordinates::MapPoint::new(700.0, 1100.0),
        point_in: crate::coordinates::MapPoint::new(780.0, 995.0),
        sector_out: crate::sector::SectorNumber::new(72),
        sector_in: crate::sector::SectorNumber::new(97),
        sector_out_index: Some(middle_arena),
        sector_in_index: Some(goal_arena),
        ..crate::gate::Door::default()
    };
    crate::gate::build_gate_links(&mut doors);
    let auth = host
        .entities
        .get_legacy_slot(0)
        .unwrap()
        .1
        .actor_auth_info();
    let route = crate::gate::find_path_gates_with_sector_indices(
        &doors,
        (608.0, 1193.0),
        89,
        Some(source_arena),
        (780.0, 995.0),
        97,
        Some(goal_arena),
        Some(&auth),
        false,
        &|_| true,
        &|_| None,
    )
    .expect("exact ReturnToDuty post must route through the two retained gates");
    assert_eq!(
        route
            .iter()
            .map(|step| u32::from(step.door_index))
            .collect::<Vec<_>>(),
        vec![120, 122]
    );
    assert!(
        crate::gate::find_path_gates_with_sector_indices(
            &doors,
            (608.0, 1193.0),
            89,
            Some(source_arena),
            (780.0, 995.0),
            97,
            crate::fast_find_grid::SectorIndex::new(98),
            Some(&auth),
            false,
            &|_| true,
            &|_| None,
        )
        .is_none(),
        "a foreign sector with the same public number is not the authored RTD post"
    );
    let actor = ScriptHandleCodec::actor_handle_from_index(0);
    let point = ScriptHandleCodec::location_handle_from_index(0);

    let mut assign = NativeStack::default();
    assign.push_i32(actor);
    assign.push_i32(point);
    assign.push_i32(3);
    assert!(matches!(
        HostFunctions::call(&mut host, NativeFn::AssignPost as u32, &mut assign),
        NativeCallOutcome::Yield(crate::interp::NativeYield {
            operation: crate::interp::NativeOperation::EngineAction(
                crate::interp::SynchronousScriptRequest::AssignPost {
                    actor: yielded_actor,
                    post_x: 780.0,
                    post_y: 995.0,
                    post_sector,
                    post_level: 3,
                    direction: 3,
                    native_return: 0,
                },
            ),
            resume: crate::interp::ResumePolicy::Fixed(0),
        }) if yielded_actor == actor && post_sector == exact_sector
    ));
}

#[test]
fn assign_post_static_legacy_binding_preserves_number_only_compatibility() {
    let mut host = NativeTestHost::new();
    host.entities.push(Some(native_test_soldier()));
    host.bindings.script_location_count = 1;
    host.bindings.script_point_count = 1;
    host.bindings.location_positions = std::sync::Arc::new(vec![(780.0, 995.0)]);
    host.bindings.location_layers = std::sync::Arc::new(vec![3]);
    host.bindings.location_sectors = std::sync::Arc::new(vec![97]);
    host.bindings.location_sector_handles = std::sync::Arc::new(vec![None]);
    let actor = ScriptHandleCodec::actor_handle_from_index(0);
    let point = ScriptHandleCodec::location_handle_from_index(0);

    let mut assign = NativeStack::default();
    assign.push_i32(actor);
    assign.push_i32(point);
    assign.push_i32(3);
    assert!(matches!(
        HostFunctions::call(&mut host, NativeFn::AssignPost as u32, &mut assign),
        NativeCallOutcome::Yield(crate::interp::NativeYield {
            operation: crate::interp::NativeOperation::EngineAction(
                crate::interp::SynchronousScriptRequest::AssignPost {
                    post_sector,
                    post_level: 3,
                    ..
                },
            ),
            ..
        }) if post_sector == crate::position_interface::SectorHandle::new(97).unwrap()
            && post_sector.arena_index().is_none()
    ));
}

#[test]
#[should_panic(expected = "exact position provenance is required")]
fn assign_post_rejects_number_only_computed_location() {
    let mut host = NativeTestHost::new();
    host.entities.push(Some(native_test_soldier()));
    host.state
        .computed_locations
        .push(Some(ComputedScriptLocation {
            position: (780.0, 995.0),
            layer: Some(3),
            sector: Some(97),
            sector_handle: None,
            active: true,
        }));
    let actor = ScriptHandleCodec::actor_handle_from_index(0);
    let point = ScriptHandleCodec::location_handle_from_index(0);
    let mut assign = NativeStack::default();
    assign.push_i32(actor);
    assign.push_i32(point);
    assign.push_i32(3);
    let _ = HostFunctions::call(&mut host, NativeFn::AssignPost as u32, &mut assign);
}

#[test]
fn thanx_launches_into_the_live_sequence_manager_before_returning() {
    let mut host = NativeTestHost::new();
    let simulation = crate::sim_rng::test_context();
    let mut capabilities = NativeSessionCapabilities::new(
        &simulation,
        &mut host.entities,
        &mut host.ai_global,
        &mut host.fast_grid,
        &mut host.globals,
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
        &mut host.state,
        &mut host.script_domains,
        &host.bindings,
        &mut capabilities,
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
    let NativeCallOutcome::Yield(crate::interp::NativeYield {
        operation: crate::interp::NativeOperation::LaunchSequence(sequence),
        resume: crate::interp::ResumePolicy::Fixed(1),
    }) = context.call(NativeFn::Thanx as u32, &mut NativeStack::default())
    else {
        panic!("Thanx must hand the complete recording to the engine");
    };
    let sequence = context
        .sequence_manager
        .as_ref()
        .expect("sequence manager")
        .get_sequence(sequence)
        .expect("recorded sequence inserted");
    assert_eq!(sequence.elements.len(), 1);
    assert_eq!(sequence.elements[0].command, Command::Timer);
    assert!(matches!(
        sequence.elements[0].get_property(Field::Timer),
        Some(FieldValue::Integer(12))
    ));
}

#[test]
fn set_view_radius_updates_live_ai_and_every_npc_before_returning() {
    let mut host = NativeTestHost::new();
    host.entities.push(Some(native_test_soldier()));
    host.standard_view_radius = 400;
    let simulation = crate::sim_rng::test_context();
    let mut capabilities = NativeSessionCapabilities::new(
        &simulation,
        &mut host.entities,
        &mut host.ai_global,
        &mut host.fast_grid,
        &mut host.globals,
    )
    .with_standard_view_radius(&mut host.standard_view_radius);
    let mut context = NativeContext::with_bindings(
        &mut host.state,
        &mut host.script_domains,
        &host.bindings,
        &mut capabilities,
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
}

#[test]
fn briefing_and_objective_writes_share_the_live_canonical_model() {
    let mut host = NativeTestHost::new();
    let simulation = crate::sim_rng::test_context();
    let mut capabilities = NativeSessionCapabilities::new(
        &simulation,
        &mut host.entities,
        &mut host.ai_global,
        &mut host.fast_grid,
        &mut host.globals,
    )
    .with_short_briefings(&mut host.short_briefings);
    let mut context = NativeContext::with_bindings(
        &mut host.state,
        &mut host.script_domains,
        &host.bindings,
        &mut capabilities,
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
    assert_eq!(briefings.entries(true)[0].id, 7);
    assert!(briefings.entries(true)[0].done);
    assert_eq!(briefings.entries(false)[0].id, 11);
    assert!(briefings.entries(false)[0].done);
}

#[test]
fn honolulu_location_native_yields_canonical_engine_action() {
    let (actor, outcome) = lone_soldier_native_outcome(NativeFn::SetActorLocation, 0);
    assert!(matches!(
        outcome,
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
fn building_placement_yields_a_complete_native_operation() {
    let mut host = NativeTestHost::new();
    host.entities
        .push(Some(native_test_pc(Vec::new(), Vec::new())));
    let actor = ScriptHandleCodec::actor_handle_from_index(0);
    let building = ScriptHandleCodec::building_handle_from_index(0);
    let mut put = NativeStack::default();
    put.push_i32(actor);
    put.push_i32(building);
    assert!(
        matches!(call_host_command(&mut host, NativeFn::PutActorInBuilding, &mut put, 0),
        NativeCommand::World(WorldNativeCommand::PutActorInBuilding { actor: owner, building: destination })
            if owner == actor && destination == building)
    );
}

#[test]
fn set_actor_posture_ko_yields_one_canonical_engine_action() {
    let mut host = NativeTestHost::new();
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
    let active_id = host.sequence_manager.insert_element(active);

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
        crate::sequence::SequenceState::Todo,
        "the native adapter must not duplicate the engine posture pipeline"
    );
}

#[test]
fn scroll_status_native_finishes_the_open_animation_before_querying() {
    let mut engine = crate::engine::EngineInner::new();
    let owner = engine.add_test_entity(Entity::Scroll(crate::element::ElementScroll {
        element: {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::ObjectScroll;
            initial_element
        },
        ..Default::default()
    }));
    let assets = crate::engine::LevelAssets::new();
    engine
        .scripts
        .install_mission(crate::engine::test_support::asm::empty_mission_script(
            "scroll_native.scs",
        ));
    engine.scripts.attach_native_capabilities(&assets);
    let sim = crate::sim_rng::test_context();
    let scroll = ScriptHandleCodec::actor_handle(owner);
    engine
        .call_external_native(TickCtx::new(&sim, &assets), "SetScrollStatus", &[scroll, 3])
        .unwrap();
    assert_eq!(
        engine
            .call_external_native(TickCtx::new(&sim, &assets), "GetScrollStatus", &[scroll])
            .unwrap(),
        3
    );
    assert_eq!(
        engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .object_data()
            .unwrap()
            .animation,
        crate::order::OrderType::BonusThree
    );
}

#[test]
fn sound_destruction_completes_before_the_next_native_query() {
    let mut engine = crate::engine::EngineInner::new();
    let assets = crate::engine::LevelAssets::new();
    engine
        .scripts
        .install_mission(crate::engine::test_support::asm::empty_mission_script(
            "sound_native.scs",
        ));
    engine.scripts.attach_native_capabilities(&assets);
    engine
        .feedback
        .sound_sim
        .sources
        .sources_push_some(crate::sound_source::SoundSource::default());
    let handle = ScriptHandleCodec::sound_source_handle_from_index(0);
    let sim = crate::sim_rng::test_context();
    assert_eq!(
        engine
            .call_external_native(TickCtx::new(&sim, &assets), "DestroySoundSource", &[handle])
            .unwrap(),
        1
    );
    assert!(engine.feedback.sound_sim.sources.get(0).is_none());
    assert_eq!(
        engine
            .call_external_native(TickCtx::new(&sim, &assets), "GetSoundSourceScript", &[0])
            .unwrap(),
        0
    );
}

#[test]
fn current_action_and_frame_queries_read_canonical_runtime_state() {
    let pc_id = EntityId::Pc(crate::entity_id::PcId(0));
    let pc_handle = ScriptHandleCodec::actor_handle(pc_id);
    let mut pc_host = NativeTestHost::new();
    let mut pc = native_test_pc(Vec::new(), Vec::new());
    // The current-action query reads the actor's installed order (Original
    // actor order), which instruction handling publishes from the selected sequence order.
    let mut sequences = crate::sequence::SequenceManager::new();
    let mut element =
        crate::sequence::SequenceElement::new(1, crate::element::Command::Move, Some(pc_id));
    element.push_order(crate::order::Order::new(
        crate::order::OrderType::RunningUpright,
        0.0,
        0.0,
        std::num::NonZeroU32::new(1).unwrap(),
    ));
    let sequence = sequences.insert_element(element);
    let installed = crate::element::InstalledActorOrder::new(
        crate::sequence::SequenceElementRef::new(sequence, 0),
        sequences
            .get_element(sequence, 0)
            .unwrap()
            .current_order()
            .unwrap(),
    );
    sequences
        .get_element_mut(sequence, 0)
        .unwrap()
        .orders
        .lease_slot(installed.slot);
    pc.actor_data_mut().unwrap().installed_order = Some(installed);
    pc_host.entities.push(Some(pc));
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

    let mut npc_host = NativeTestHost::new();
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
fn current_action_returns_nonanimation_end_without_an_installed_order() {
    let pc_id = EntityId::Pc(crate::entity_id::PcId(0));
    let pc_handle = ScriptHandleCodec::actor_handle(pc_id);
    let mut host = NativeTestHost::new();
    host.entities
        .push(Some(native_test_pc(Vec::new(), Vec::new())));
    let mut sequences = crate::sequence::SequenceManager::new();
    let mut selected = Vec::new();
    let mut sounds = crate::sound_source::SoundSourceManager::new();
    let weather = crate::engine::WeatherState::default();
    let frame = 123;
    let mut action = NativeStack::default();
    action.push_i32(pc_handle);

    assert_eq!(
        call_host_native_with_queries(
            &mut host,
            NativeFn::GetCurrentAction,
            &mut action,
            TestQueryViews::new(&mut sequences, &mut selected, &mut sounds, &weather, &frame),
        ),
        crate::order::OrderType::NonanimationEnd as i32
    );
}

#[test]
fn any_action_selected_reads_the_messenger_action_not_the_pcs_remembered_action() {
    let pc_id = EntityId::Pc(crate::entity_id::PcId(0));
    let pc_handle = ScriptHandleCodec::actor_handle(pc_id);
    let mut host = NativeTestHost::new();
    host.entities
        .push(Some(native_test_pc(Vec::new(), Vec::new())));
    host.selected_pcs.push(pc_id);

    // The messenger action is authoritative for script dispatch.
    // The PC field can legitimately lag it in either direction.
    host.selected_action = crate::profiles::Action::Net;
    host.entities
        .get_mut(pc_id)
        .unwrap()
        .pc_data_mut()
        .unwrap()
        .current_action = crate::profiles::Action::NoAction;
    let mut action = NativeStack::default();
    action.push_i32(pc_handle);
    assert_eq!(
        host.call(NativeFn::HasAnyActionSelected as u32, &mut action)
            .expect_return("HasAnyActionSelected is synchronous"),
        1
    );

    host.selected_action = crate::profiles::Action::NoAction;
    host.entities
        .get_mut(pc_id)
        .unwrap()
        .pc_data_mut()
        .unwrap()
        .current_action = crate::profiles::Action::Bow;
    let mut action = NativeStack::default();
    action.push_i32(pc_handle);
    assert_eq!(
        host.call(NativeFn::HasAnyActionSelected as u32, &mut action)
            .expect_return("HasAnyActionSelected is synchronous"),
        0
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
    let mut first_host = NativeTestHost::new();
    let mut second_host = NativeTestHost::new();

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
    let mut state = ScriptState::default();
    let mut script_domains = crate::engine::ScriptDomains::default();
    let mut entities = crate::entities::Entities::new();
    let mut ai_global = crate::ai::AiGlobalState::default();
    let mut fast_grid = crate::fast_find_grid::FastFindGrid::default();
    let mut globals = Vec::new();
    let static_obstacles = vec![crate::sight_obstacle::SightObstacle::new_default(0)];
    let dynamic_obstacles = vec![crate::sight_obstacle::SightObstacle::new_default(1)];
    let static_active = vec![false];
    let sim = crate::sim_rng::test_context();
    let mut capabilities = NativeSessionCapabilities::new(
        &sim,
        &mut entities,
        &mut ai_global,
        &mut fast_grid,
        &mut globals,
    )
    .with_world_views(&static_obstacles, &dynamic_obstacles, &static_active);
    let context = NativeContext::with_bindings(
        &mut state,
        &mut script_domains,
        AttachedScriptBindings::empty_ref(),
        &mut capabilities,
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
fn animation_state_write_is_immediately_visible_from_canonical_entity() {
    let actor = ScriptHandleCodec::actor_handle_from_index(0);
    let mut host = NativeTestHost::new();
    host.entities
        .push(Some(Entity::Fx(crate::element::ElementFx {
            element: {
                let mut initial_element = crate::element::ElementData::default();
                initial_element.kind = crate::element::ElementKind::Fx;
                initial_element
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
fn compute_border_point_matches_original_rounded_half_line_arithmetic() {
    use crate::coordinates::MapBBox;

    // H12's Knight03 RecordEnterGame exposed the difference between the
    // Original-game geometry uses a rounded-float half-line plus a double-precision line
    // equation and a direct f32 ray parameterization. Keep the exact result
    // as a general regression for large-coordinate diagonal entries.
    let map_bbox = MapBBox::from_coords(0.0, 0.0, 3000.0, 4000.0);
    let (border, outside) = compute_border_point_bbox(map_bbox, (8.0, 2272.0), 5);

    assert_eq!(border.0.to_bits(), 0.0f32.to_bits());
    assert_eq!(border.1.to_bits(), 2_268.687_3_f32.to_bits());
    assert_eq!(outside.0.to_bits(), (-55.432_774f32).to_bits());
    assert_eq!(outside.1.to_bits(), 2_245.725_8_f32.to_bits());
}

#[test]
fn direct_owner_add_campaign_value_ransom_credits_stat_and_queues_jingle() {
    let mut campaign = crate::campaign::Campaign::default();
    let mut mission_stat = crate::mission_stat::MissionStat::default();
    with_campaign_context(
        AttachedScriptBindings::empty_ref(),
        &mut campaign,
        &mut mission_stat,
        |context| {
            context.add_campaign_value(crate::campaign::CampaignValue::Ransom, 250, 100);
            assert!(matches!(
                context.pending_yield,
                Some(NativeYield {
                    operation: NativeOperation::Command(NativeCommand::Sound(
                        SoundCommand::PlayJingle(crate::sound::Jingle::CashWon)
                    )),
                    ..
                })
            ));
        },
    );

    assert_eq!(
        campaign.get_value(crate::campaign::CampaignValue::Ransom),
        crate::campaign::INITIAL_RANSOM + 250
    );
    assert_eq!(mission_stat.collected_money, 250);
}

#[test]
fn direct_owner_set_campaign_value_ransom_jingle_only_when_growing() {
    let mut campaign = crate::campaign::Campaign::default();
    let mut mission_stat = crate::mission_stat::MissionStat::default();
    campaign.values[crate::campaign::CampaignValue::Ransom] = 200;

    // Lowering: no jingle.
    with_campaign_context(
        AttachedScriptBindings::empty_ref(),
        &mut campaign,
        &mut mission_stat,
        |context| {
            context.set_campaign_value(crate::campaign::CampaignValue::Ransom, 100, 50);
            assert!(context.pending_yield.is_none());
        },
    );

    // Raising: jingle queued.
    with_campaign_context(
        AttachedScriptBindings::empty_ref(),
        &mut campaign,
        &mut mission_stat,
        |context| {
            context.set_campaign_value(crate::campaign::CampaignValue::Ransom, 500, 50);
            assert!(matches!(
                context.pending_yield,
                Some(NativeYield {
                    operation: NativeOperation::Command(NativeCommand::Sound(
                        SoundCommand::PlayJingle(crate::sound::Jingle::CashWon)
                    )),
                    ..
                })
            ));
        },
    );
    // Value assignment does NOT credit collected_money.
    assert_eq!(mission_stat.collected_money, 0);
}

#[test]
fn ransom_natives_round_trip_through_borrowed_campaign_owner() {
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
    let mut globals = Vec::new();
    let mut selected = Vec::new();
    let sim = crate::sim_rng::test_context();
    let mut capabilities = NativeSessionCapabilities::new(
        &sim,
        &mut entities,
        &mut ai_global,
        &mut fast_grid,
        &mut globals,
    )
    .with_queries(&mut sequences, &mut selected, &mut sounds, &weather, &frame)
    .with_campaign(&mut campaign, &mut mission_stat);
    let mut context = NativeContext::with_bindings(
        &mut state,
        &mut script_domains,
        AttachedScriptBindings::empty_ref(),
        &mut capabilities,
    );

    let mut set = NativeStack::default();
    set.push_i32(1_234);
    assert!(matches!(
        context.call(NativeFn::SetRansomMoney as u32, &mut set),
        NativeCallOutcome::Yield(NativeYield {
            operation: NativeOperation::Command(NativeCommand::Sound(SoundCommand::PlayJingle(_))),
            resume: ResumePolicy::Fixed(0)
        })
    ));
    let mut get = NativeStack::default();
    assert_eq!(
        context
            .call(NativeFn::GetRansomMoney as u32, &mut get)
            .expect_return("GetRansomMoney is synchronous"),
        1_234
    );
    drop(context);

    assert_eq!(
        campaign.get_value(crate::campaign::CampaignValue::Ransom),
        1_234
    );
    assert_eq!(mission_stat.collected_money, 0);
}

#[test]
fn direct_owner_add_campaign_value_score_credits_added_score_silently() {
    let mut campaign = crate::campaign::Campaign::default();
    let mut mission_stat = crate::mission_stat::MissionStat::default();
    with_campaign_context(
        AttachedScriptBindings::empty_ref(),
        &mut campaign,
        &mut mission_stat,
        |context| {
            context.add_campaign_value(crate::campaign::CampaignValue::Score, 750, 100);
        },
    );

    assert_eq!(mission_stat.added_score, 750);
}

fn native_test_soldier() -> Entity {
    Entity::Soldier(crate::element::ActorSoldier {
        element: {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::ActorSoldier;
            initial_element
        },
        actor: crate::element::ActorData::default(),
        human: crate::element::HumanData::default(),
        npc: crate::element::NpcData {
            ai: crate::element::AiActorData {
                ai_brain: crate::element::AiBrain::Enemy(Box::new(crate::ai_enemy::EnemyAi::new(
                    0,
                ))),
                ..Default::default()
            },
            ..Default::default()
        },
        soldier: crate::element::SoldierData::default(),
    })
}

#[test]
fn set_always_attentive_promotes_green_view_when_music_is_already_yellow() {
    let mut soldier = crate::engine::test_support::actors::make_test_ai_soldier(
        crate::element::Camp::Lacklandists,
    );
    let enemy = soldier
        .enemy_ai_mut()
        .expect("native test soldier requires an enemy AI");
    enemy.base.current_music_alert_status = crate::ai::AlertLevel::Yellow;
    enemy.base.view_alert_status = crate::ai::AlertLevel::Green;

    let mut engine = crate::engine::EngineInner::new();
    let owner = engine.add_test_entity(soldier);
    engine.control.frame_counter = 656;
    let mut assets = crate::engine::LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    engine
        .scripts
        .install_mission(crate::engine::test_support::asm::empty_mission_script(
            "attentive_native.scs",
        ));
    engine.scripts.attach_native_capabilities(&assets);

    assert_eq!(
        engine
            .call_external_native(
                TickCtx::new(&crate::sim_rng::test_context(), &assets),
                "SetAlwaysAttentive",
                &[ScriptHandleCodec::actor_handle(owner), 1],
            )
            .expect("attentive native must complete through its engine callback"),
        0
    );

    let enemy = engine.enemy(owner);
    assert!(enemy.forced_attentive);
    assert!(enemy.will_be_attentive);
    assert_eq!(
        enemy.base.current_music_alert_status,
        crate::ai::AlertLevel::Yellow
    );
    assert_eq!(enemy.base.view_alert_status, crate::ai::AlertLevel::Yellow);
}

#[test]
fn set_always_attentive_preserves_ordinary_alert_branches() {
    use crate::ai::AlertLevel;

    struct Case {
        name: &'static str,
        frame: u32,
        target: bool,
        initial_forced: bool,
        music: AlertLevel,
        view: AlertLevel,
        expected_music: AlertLevel,
        expected_view: AlertLevel,
    }

    let cases = [
        Case {
            name: "ordinary green",
            frame: 50,
            target: true,
            initial_forced: false,
            music: AlertLevel::Green,
            view: AlertLevel::Green,
            expected_music: AlertLevel::Yellow,
            expected_view: AlertLevel::Yellow,
        },
        Case {
            name: "already yellow",
            frame: 50,
            target: true,
            initial_forced: false,
            music: AlertLevel::Yellow,
            view: AlertLevel::Yellow,
            expected_music: AlertLevel::Yellow,
            expected_view: AlertLevel::Yellow,
        },
        Case {
            name: "red remains red",
            frame: 50,
            target: true,
            initial_forced: false,
            music: AlertLevel::Red,
            view: AlertLevel::Red,
            expected_music: AlertLevel::Red,
            expected_view: AlertLevel::Red,
        },
        Case {
            name: "disable retains split alert",
            frame: 50,
            target: false,
            initial_forced: true,
            music: AlertLevel::Yellow,
            view: AlertLevel::Green,
            expected_music: AlertLevel::Yellow,
            expected_view: AlertLevel::Green,
        },
        Case {
            name: "initial frame retains split alert",
            frame: 1,
            target: true,
            initial_forced: false,
            music: AlertLevel::Yellow,
            view: AlertLevel::Green,
            expected_music: AlertLevel::Yellow,
            expected_view: AlertLevel::Green,
        },
    ];

    for case in cases {
        let mut soldier = crate::engine::test_support::actors::make_test_ai_soldier(
            crate::element::Camp::Lacklandists,
        );
        let enemy = soldier
            .enemy_ai_mut()
            .expect("native test soldier requires an enemy AI");
        enemy.forced_attentive = case.initial_forced;
        enemy.will_be_attentive = true;
        enemy.base.current_music_alert_status = case.music;
        enemy.base.view_alert_status = case.view;

        let mut engine = crate::engine::EngineInner::new();
        let owner = engine.add_test_entity(soldier);
        engine.control.frame_counter = case.frame;
        let mut assets = crate::engine::LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine
            .scripts
            .install_mission(crate::engine::test_support::asm::empty_mission_script(
                "attentive_native.scs",
            ));
        engine.scripts.attach_native_capabilities(&assets);
        assert_eq!(
            engine
                .call_external_native(
                    TickCtx::new(&crate::sim_rng::test_context(), &assets),
                    "SetAlwaysAttentive",
                    &[
                        ScriptHandleCodec::actor_handle(owner),
                        i32::from(case.target)
                    ],
                )
                .expect("attentive native must complete through its engine callback"),
            0,
            "{}",
            case.name
        );

        let enemy = engine.enemy(owner);
        assert_eq!(enemy.forced_attentive, case.target, "{}", case.name);
        assert_eq!(
            enemy.base.current_music_alert_status, case.expected_music,
            "{} music",
            case.name
        );
        assert_eq!(
            enemy.base.view_alert_status, case.expected_view,
            "{} view",
            case.name
        );
    }
}

fn native_test_pc(disabled_actions: Vec<bool>, disabled_actions_temp: Vec<bool>) -> Entity {
    Entity::Pc(crate::element::ActorPc {
        element: {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::ActorPc;
            initial_element
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

fn mission_team_identity_pc(
    profile_index: crate::profiles::CharacterProfileIdx,
    campaign_description_index: Option<u32>,
) -> Entity {
    let mut pc = native_test_pc(Vec::new(), Vec::new());
    let data = pc.pc_data_mut().expect("mission-team fixture must be a PC");
    data.profile_index = profile_index;
    data.campaign_description_index = campaign_description_index;
    pc
}

fn call_mission_team_native(host: &mut NativeTestHost, native: NativeFn, actor: i32) {
    let mut stack = NativeStack::default();
    stack.push_i32(actor);
    assert!(matches!(
        HostFunctions::call(host, native as u32, &mut stack),
        NativeCallOutcome::Return(0)
            | NativeCallOutcome::Yield(NativeYield {
                operation: NativeOperation::Command(NativeCommand::Engine(
                    EngineCommand::MarkPc { .. }
                )),
                resume: ResumePolicy::Fixed(0)
            })
    ));
}

#[test]
fn mission_team_natives_use_exact_description_identity_for_shared_profiles() {
    let shared_profile = crate::profiles::CharacterProfileIdx(8);
    let mut host = NativeTestHost::new();
    host.campaign.characters = vec![crate::campaign::PcDescription::default(); 33];
    for index in [3, 11, 32] {
        host.campaign.characters[index].character_profile_idx = Some(shared_profile);
    }
    host.entities = crate::entities::Entities::from_legacy_slots(vec![
        Some(mission_team_identity_pc(shared_profile, Some(11))),
        Some(mission_team_identity_pc(shared_profile, Some(32))),
    ]);
    let pc_11 = ScriptHandleCodec::actor_handle_from_index(0);
    let pc_32 = ScriptHandleCodec::actor_handle_from_index(1);

    call_mission_team_native(&mut host, NativeFn::AddPCToMissionTeam, pc_32);
    assert_eq!(host.campaign.mission_team_indices, [32]);
    call_mission_team_native(&mut host, NativeFn::AddPCToMissionTeam, pc_11);
    assert_eq!(host.campaign.mission_team_indices, [32, 11]);
    call_mission_team_native(&mut host, NativeFn::RemovePCFromMissionTeam, pc_11);
    assert_eq!(host.campaign.mission_team_indices, [32]);
}

#[test]
fn mission_team_natives_reject_corrupt_live_pc_description_identity() {
    let shared_profile = crate::profiles::CharacterProfileIdx(8);
    let other_profile = crate::profiles::CharacterProfileIdx(7);

    for (name, description_index) in [
        ("missing", None),
        ("out of bounds", Some(99)),
        ("profile mismatch", Some(11)),
    ] {
        let mut host = NativeTestHost::new();
        host.campaign.characters = vec![crate::campaign::PcDescription::default(); 33];
        host.campaign.characters[3].character_profile_idx = Some(shared_profile);
        host.campaign.characters[11].character_profile_idx = Some(other_profile);
        host.campaign.characters[32].character_profile_idx = Some(shared_profile);
        host.campaign.mission_team_indices = vec![32];
        host.entities = crate::entities::Entities::from_legacy_slots(vec![Some(
            mission_team_identity_pc(shared_profile, description_index),
        )]);
        let actor = ScriptHandleCodec::actor_handle_from_index(0);

        call_mission_team_native(&mut host, NativeFn::AddPCToMissionTeam, actor);
        assert_eq!(host.campaign.mission_team_indices, [32], "{name}: add");

        call_mission_team_native(&mut host, NativeFn::RemovePCFromMissionTeam, actor);
        assert_eq!(host.campaign.mission_team_indices, [32], "{name}: remove");
    }
}

#[test]
fn mission_team_natives_keep_raw_profile_fallback_without_live_actor() {
    let profile_index = crate::profiles::CharacterProfileIdx(8);
    let mut profiles = crate::profiles::ProfileManager::new();
    profiles
        .characters
        .resize_with(9, crate::profiles::CharacterProfile::default);

    let mut host = NativeTestHost::new();
    host.bindings.profile_manager = std::sync::Arc::new(profiles);
    host.campaign.characters = vec![crate::campaign::PcDescription::default(); 4];
    host.campaign.characters[3].character_profile_idx = Some(profile_index);

    call_mission_team_native(
        &mut host,
        NativeFn::AddPCToMissionTeam,
        i32::try_from(u32::from(profile_index)).expect("profile index fits native integer"),
    );
    assert_eq!(host.campaign.mission_team_indices, [3]);
}

fn persistent_property_test_host(
    with_campaign: bool,
) -> (
    NativeTestHost,
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
    let mut host = NativeTestHost::new();
    let bindings = AttachedScriptBindings {
        profile_manager: std::sync::Arc::new(profiles),
        ..Default::default()
    };

    let mut pc = native_test_pc(vec![true; 3], vec![false; 3]);
    let pc_data = pc.pc_data_mut().expect("test entity must be a PC");
    pc_data.profile_index = CharacterProfileIdx(0);
    pc_data.campaign_description_index = with_campaign.then_some(0);
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
    host: &mut NativeTestHost,
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
    host: &mut NativeTestHost,
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
    host: &mut NativeTestHost,
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
    host: &mut NativeTestHost,
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

    let mut life_host = NativeTestHost::new();
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

    let mut concussion_host = NativeTestHost::new();
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
    host: &mut NativeTestHost,
    weather: &crate::engine::WeatherState,
    npc_index: usize,
    target_index: usize,
) -> i32 {
    native_sees_at_frame(host, weather, npc_index, target_index, 0)
}

/// `Sees` memoizes the computed view radius per viewer and universal frame
/// like the Original surface cache, so a query under changed ambiance must
/// run on a fresh frame to observe the recomputed radius.
fn native_sees_at_frame(
    host: &mut NativeTestHost,
    weather: &crate::engine::WeatherState,
    npc_index: usize,
    target_index: usize,
    frame: u32,
) -> i32 {
    let mut sequences = crate::sequence::SequenceManager::new();
    let mut sounds = crate::sound_source::SoundSourceManager::new();
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

fn native_sees_host(target: crate::coordinates::MapPoint, camp: Camp) -> NativeTestHost {
    let mut npc = native_test_soldier();
    npc.element_data_mut()
        .set_position_map(crate::coordinates::MapPoint::ZERO);
    npc.element_data_mut().set_direction_instantly(4);
    npc.element_data_mut()
        .publish_order_posture(Posture::Upright);
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
    pc.element_data_mut()
        .publish_order_posture(Posture::Upright);

    let mut host = NativeTestHost::new();
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
    // view-radius calculation blend to the 400-unit day shadow-polygon radius,
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

    // The night light lookup reproduces sector lookup's grid-block walk, so the
    // shadow sector must be registered through the sized grid rather than
    // pushed straight into the sector table.
    host.fast_grid.size_map(20, 20);
    host.fast_grid.allocate_layers(1);
    let points = vec![
        crate::coordinates::MapPoint::new(240.0, -10.0),
        crate::coordinates::MapPoint::new(260.0, -10.0),
        crate::coordinates::MapPoint::new(260.0, 10.0),
        crate::coordinates::MapPoint::new(240.0, 10.0),
    ];
    let mut bounding_box = crate::coordinates::MapBBox::new();
    for &point in &points {
        bounding_box.expand_point(point);
    }
    host.fast_grid.add_sector(
        crate::fast_find_grid::GridSector {
            points,
            bounding_box,
            sector_type: crate::sector::SectorType::SHADOW,
            layer: 0,
            sector_number: crate::sector::SectorNumber::new(1),
            ..Default::default()
        },
        0,
    );
    let level = std::sync::Arc::make_mut(&mut host.fast_grid.level);
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
    assert_eq!(native_sees_at_frame(&mut host, &weather, 0, 1, 0), 1);

    // The night query runs on the next universal frame so the per-frame
    // view-radius memo from the day query cannot satisfy it.
    weather.ambiance = crate::engine::Ambiance::Night;
    assert_eq!(native_sees_at_frame(&mut host, &weather, 0, 1, 1), 0);
}

fn set_experiences_test_host() -> (NativeTestHost, crate::campaign::Campaign, i32) {
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

    let mut host = NativeTestHost::new();
    host.entities = crate::entities::Entities::from_legacy_slots(vec![Some(native_test_pc(
        Vec::new(),
        Vec::new(),
    ))]);
    (host, campaign, actor)
}

fn call_set_experiences(
    host: &mut NativeTestHost,
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
    let mut host = NativeTestHost::new();
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
    let mut host = NativeTestHost::new();
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
    let mut host = NativeTestHost::new();
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
    let mut host = NativeTestHost::new();
    host.entities = crate::entities::Entities::from_legacy_slots(vec![
        Some(native_test_soldier()),
        Some(native_test_soldier()),
    ]);

    let mut stack = NativeStack::default();
    stack.push_i32(ScriptHandleCodec::actor_handle_from_index(0));
    stack.push_i32(ScriptHandleCodec::actor_handle_from_index(1));
    let command = call_host_command(&mut host, NativeFn::AddAsSubordinate, &mut stack, 0);
    assert!(matches!(
        command,
        NativeCommand::World(WorldNativeCommand::AddAsSubordinateInitialize { .. })
    ));

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

#[test]
fn add_as_subordinate_existing_membership_preserves_patrol_and_rejects_transfer() {
    let mut host = NativeTestHost::new();
    host.entities = crate::entities::Entities::from_legacy_slots(vec![
        Some(native_test_soldier()),
        Some(native_test_soldier()),
        Some(native_test_soldier()),
    ]);
    let chief = EntityId::Soldier(crate::entity_id::SoldierId(0));
    let member = EntityId::Soldier(crate::entity_id::SoldierId(1));
    {
        let ai = host
            .entities
            .get_mut(chief)
            .unwrap()
            .ai_controller_mut()
            .unwrap();
        ai.theoretical_patrol.push(member);
        ai.patrol.push(member);
        ai.needs_patrol_reinit = false;
    }
    host.entities
        .get_mut(member)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .patrol_chief = Some(chief);
    for chief_slot in [0, 2] {
        let mut stack = NativeStack::default();
        stack.push_i32(ScriptHandleCodec::actor_handle_from_index(chief_slot));
        stack.push_i32(ScriptHandleCodec::actor_handle_from_index(1));
        assert_eq!(
            call_host_native(&mut host, NativeFn::AddAsSubordinate, &mut stack),
            0
        );
        let ai = host.entities.get(chief).unwrap().ai_controller().unwrap();
        assert_eq!(ai.theoretical_patrol, [member]);
        assert_eq!(ai.patrol, [member]);
        assert!(!ai.needs_patrol_reinit);
        assert_eq!(
            host.entities
                .get(member)
                .unwrap()
                .ai_controller()
                .unwrap()
                .patrol_chief,
            Some(chief)
        );
        assert!(
            host.entity_at_legacy_slot(2)
                .ai_controller()
                .unwrap()
                .theoretical_patrol
                .is_empty()
        );
    }
}

#[test]
fn remove_all_subordinates_yields_engine_clear_before_vm_continues() {
    let mut host = NativeTestHost::new();
    host.entities = crate::entities::Entities::from_legacy_slots(vec![Some(native_test_soldier())]);
    let actor = ScriptHandleCodec::actor_handle_from_index(0);

    let mut stack = NativeStack::default();
    stack.push_i32(actor);
    assert!(matches!(
        HostFunctions::call(
            &mut host,
            NativeFn::RemoveAllSubordinates as u32,
            &mut stack,
        ),
        NativeCallOutcome::Yield(crate::interp::NativeYield {
            operation: crate::interp::NativeOperation::EngineAction(
                crate::interp::SynchronousScriptRequest::RemoveAllSubordinates {
                    actor: yielded_actor,
                    native_return: 0,
                },
            ),
            resume: crate::interp::ResumePolicy::Fixed(0),
        }) if yielded_actor == actor
    ));
}

#[test]
fn remove_all_subordinates_rejects_invalid_or_non_npc_chief_without_yielding() {
    let invalid_actor = ScriptHandleCodec::actor_handle_from_index(1);
    let mut host = NativeTestHost::new();
    host.entities = crate::entities::Entities::from_legacy_slots(vec![Some(native_test_pc(
        Vec::new(),
        Vec::new(),
    ))]);

    for actor in [invalid_actor, ScriptHandleCodec::actor_handle_from_index(0)] {
        let mut stack = NativeStack::default();
        stack.push_i32(actor);
        assert_eq!(
            HostFunctions::call(
                &mut host,
                NativeFn::RemoveAllSubordinates as u32,
                &mut stack,
            ),
            NativeCallOutcome::Return(0)
        );
    }
}
