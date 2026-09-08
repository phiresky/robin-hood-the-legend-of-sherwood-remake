// Test-only copies of the pre-projection serde, native and hash declarations.
// Keep independent field/default policies to detect accidental wire changes.
use super::*;

pub(super) trait LegacyWire: Sized {
    fn legacy_json(&self) -> String;
    fn legacy_from_json(json: &str) -> Self;
    fn legacy_hash(&self) -> u64;
    fn legacy_native_bytes(&self) -> Vec<u8>;
}

#[derive(Serialize, Deserialize, bitcode::Encode, robin_state_hash_derive::StateHash)]
struct LegacyAiController {
    me: NpcHandle,
    owner_entity_id: Option<EntityId>,
    path_id: Option<PathId>,
    alert_path_id: Option<PathId>,
    current_state: AiState,
    current_substate: Substate,
    old_state: i32,
    current_music_alert_status: AlertLevel,
    view_alert_status: AlertLevel,
    substate_at_last_timer_launch: Substate,
    attitude: Attitude,
    blood_alcohol: u8,
    initial_action: u32,
    number_of_looks: u8,
    has_patrol_path: bool,
    patrol_path: Option<PatrolPath>,
    detached_patrol_path_status: DetachedPatrolPathStatus,
    can_move: bool,
    stop_before_end_of_path: bool,
    use_max_norm_to_stop_before_end_of_path: bool,
    stop_before_end_of_path_distance: u16,
    think_recursion_depth: u8,
    #[serde(skip)]
    #[state_hash(skip)]
    #[bitcode(skip)]
    open_end_think_frames: u8,
    #[serde(skip)]
    #[state_hash(skip)]
    #[bitcode(skip)]
    engine_deferred_end_think_frames: u8,
    #[serde(skip)]
    #[state_hash(skip)]
    #[bitcode(skip)]
    engine_completion_verdict_resolved: bool,
    macro_command: Vec<u8>,
    macro_command_offset: usize,
    macro_command_waypoint: Option<(PathId, u8)>,
    number_of_remaining_macro_bytes: u16,
    macro_in_progress: bool,
    macro_started_in_this_frame: bool,
    #[serde(
        serialize_with = "serialize_optional_ai_handle",
        deserialize_with = "deserialize_optional_ai_handle"
    )]
    primary_target: Option<AiEntityHandle>,
    #[serde(
        serialize_with = "serialize_optional_ai_handle",
        deserialize_with = "deserialize_optional_ai_handle"
    )]
    friend_in_trouble: Option<AiEntityHandle>,
    #[serde(
        serialize_with = "serialize_optional_ai_handle",
        deserialize_with = "deserialize_optional_ai_handle"
    )]
    detected_body: Option<AiEntityHandle>,
    #[serde(
        serialize_with = "serialize_optional_ai_handle",
        deserialize_with = "deserialize_optional_ai_handle"
    )]
    interesting_object: Option<AiEntityHandle>,
    #[serde(
        serialize_with = "serialize_optional_ai_handle",
        deserialize_with = "deserialize_optional_ai_handle"
    )]
    antagonist: Option<AiEntityHandle>,
    last_stimulus_actor: Option<AiEntityHandle>,
    timer_is_running: bool,
    when_does_timer_ring: u32,
    macro_timer_is_running: bool,
    when_does_macro_timer_ring: u32,
    standing_around_timer: u16,
    sorrow_level: u16,
    last_stimulus: [StimulusType; 5],
    last_stimulus_multiplicity: [u16; 5],
    is_master: bool,
    #[serde(
        serialize_with = "serialize_optional_ai_handle",
        deserialize_with = "deserialize_optional_ai_handle"
    )]
    master: Option<AiEntityHandle>,
    seek_position: Position,
    alert_soldiers_point: Position,
    first_try: bool,
    panic_center_x: f32,
    panic_center_y: f32,
    lasting_panic_runs: u8,
    directed_panic: bool,
    list_us: Vec<HumanHandle>,
    list_alerted_us: Vec<NpcHandle>,
    list_staying_us: Vec<NpcHandle>,
    couldnt_reachpoint: bool,
    already_on_point: bool,
    already_turned: bool,
    completion_latch_inside_think: bool,
    likes_to_sit_around: bool,
    special_action: bool,
    remaining_tequila_gulps: u8,
    friends_are_alerted: bool,
    is_stay_at_home: bool,
    locks_flag_field: AiLockFlags,
    was_busy: bool,
    stimulus_queue: Vec<Stimulus>,
    script_locked: bool,
    remember_events: bool,
    leave_house_number: u16,
    last_hint_actuality: u32,
    last_hint_subject: Question,
    my_door_index: Option<crate::gate::DoorIndex>,
    looking_for_help_because_enemy_seen: bool,
    forgotten_objects: Vec<ObjectHandle>,
    #[serde(
        serialize_with = "serialize_optional_ai_handle",
        deserialize_with = "deserialize_optional_ai_handle"
    )]
    object_of_desire: Option<AiEntityHandle>,
    #[serde(
        serialize_with = "serialize_optional_ai_handle",
        deserialize_with = "deserialize_optional_ai_handle"
    )]
    checkpoint_charly: Option<AiEntityHandle>,
    #[serde(
        serialize_with = "serialize_optional_ai_handle",
        deserialize_with = "deserialize_optional_ai_handle"
    )]
    synchronize_charly: Option<AiEntityHandle>,
    synchronize_index: u16,
    delta_sorrow_level: u16,
    missed_in_action: Vec<NpcHandle>,
    frame_when_enemy_detected: u32,
    inside_halt_method: bool,
    synchronizing_actors: Vec<NpcHandle>,
    default_path_walking_flags: GotoFlags,
    forbidden_remark_ids: Vec<u32>,
    initial_view_cone: ViewCone,
    current_remark: Remark,
    current_remark_flags: u16,
    next_macro_rand: u8,
    next_macro_rand_forecasted: bool,
    current_emoticon_type: EmoticonType,
    emoticon_expiration_date: u32,
    emoticon_has_expiration_date: bool,
    my_reconnaissance_report: ReconnaissanceReport,
    knocked_out_in_money_fight: bool,
    looted_after_money_fight: bool,
    patrol_chief: Option<EntityId>,
    patrol: Vec<EntityId>,
    missed_patrol_members: Vec<EntityId>,
    theoretical_patrol: Vec<EntityId>,
    patrol_stopped: bool,
    patrol_direction: u16,
    needs_patrol_reinit: bool,
    got_the_beggar_trick: bool,
    ai_log: Vec<LogLine>,
    debug_view_cone_enabled: bool,
    last_goto_destination: Position,
    last_goto_flags: GotoFlags,
    stuck_counter: u16,
    outbox: AiOutbox,
    has_script_filter_override: bool,
    last_synced_focus_target: Option<AiEntityHandle>,
    initial_position: Position,
    initial_view_direction: u16,
    max_visibility: u32,
    cached_frame: u32,
    cached_in_building: bool,
}

impl LegacyAiController {
    fn capture(runtime: &AiController) -> Self {
        Self {
            me: runtime.me.clone(),
            owner_entity_id: runtime.owner_entity_id.clone(),
            path_id: runtime.path_id.clone(),
            alert_path_id: runtime.alert_path_id.clone(),
            current_state: runtime.current_state.clone(),
            current_substate: runtime.current_substate.clone(),
            old_state: runtime.old_state.clone(),
            current_music_alert_status: runtime.current_music_alert_status.clone(),
            view_alert_status: runtime.view_alert_status.clone(),
            substate_at_last_timer_launch: runtime.substate_at_last_timer_launch.clone(),
            attitude: runtime.attitude.clone(),
            blood_alcohol: runtime.blood_alcohol.clone(),
            initial_action: runtime.initial_action.clone(),
            number_of_looks: runtime.number_of_looks.clone(),
            has_patrol_path: runtime.has_patrol_path.clone(),
            patrol_path: runtime.patrol_path.clone(),
            detached_patrol_path_status: runtime.detached_patrol_path_status.clone(),
            can_move: runtime.can_move.clone(),
            stop_before_end_of_path: runtime.stop_before_end_of_path.clone(),
            use_max_norm_to_stop_before_end_of_path: runtime
                .use_max_norm_to_stop_before_end_of_path
                .clone(),
            stop_before_end_of_path_distance: runtime.stop_before_end_of_path_distance.clone(),
            think_recursion_depth: runtime.think_recursion_depth.clone(),
            open_end_think_frames: runtime.open_end_think_frames.clone(),
            engine_deferred_end_think_frames: runtime.engine_deferred_end_think_frames.clone(),
            engine_completion_verdict_resolved: runtime.engine_completion_verdict_resolved.clone(),
            macro_command: runtime.macro_command.clone(),
            macro_command_offset: runtime.macro_command_offset.clone(),
            macro_command_waypoint: runtime.macro_command_waypoint.clone(),
            number_of_remaining_macro_bytes: runtime.number_of_remaining_macro_bytes.clone(),
            macro_in_progress: runtime.macro_in_progress.clone(),
            macro_started_in_this_frame: runtime.macro_started_in_this_frame.clone(),
            primary_target: runtime.primary_target.clone(),
            friend_in_trouble: runtime.friend_in_trouble.clone(),
            detected_body: runtime.detected_body.clone(),
            interesting_object: runtime.interesting_object.clone(),
            antagonist: runtime.antagonist.clone(),
            last_stimulus_actor: runtime.last_stimulus_actor.clone(),
            timer_is_running: runtime.timer_is_running.clone(),
            when_does_timer_ring: runtime.when_does_timer_ring.clone(),
            macro_timer_is_running: runtime.macro_timer_is_running.clone(),
            when_does_macro_timer_ring: runtime.when_does_macro_timer_ring.clone(),
            standing_around_timer: runtime.standing_around_timer.clone(),
            sorrow_level: runtime.sorrow_level.clone(),
            last_stimulus: runtime.last_stimulus.clone(),
            last_stimulus_multiplicity: runtime.last_stimulus_multiplicity.clone(),
            is_master: runtime.is_master.clone(),
            master: runtime.master.clone(),
            seek_position: runtime.seek_position.clone(),
            alert_soldiers_point: runtime.alert_soldiers_point.clone(),
            first_try: runtime.first_try.clone(),
            panic_center_x: runtime.panic_center_x.clone(),
            panic_center_y: runtime.panic_center_y.clone(),
            lasting_panic_runs: runtime.lasting_panic_runs.clone(),
            directed_panic: runtime.directed_panic.clone(),
            list_us: runtime.list_us.clone(),
            list_alerted_us: runtime.list_alerted_us.clone(),
            list_staying_us: runtime.list_staying_us.clone(),
            couldnt_reachpoint: runtime.couldnt_reachpoint.clone(),
            already_on_point: runtime.already_on_point.clone(),
            already_turned: runtime.already_turned.clone(),
            completion_latch_inside_think: runtime.completion_latch_inside_think.clone(),
            likes_to_sit_around: runtime.likes_to_sit_around.clone(),
            special_action: runtime.special_action.clone(),
            remaining_tequila_gulps: runtime.remaining_tequila_gulps.clone(),
            friends_are_alerted: runtime.friends_are_alerted.clone(),
            is_stay_at_home: runtime.is_stay_at_home.clone(),
            locks_flag_field: runtime.locks_flag_field.clone(),
            was_busy: runtime.was_busy.clone(),
            stimulus_queue: runtime.stimulus_queue.clone(),
            script_locked: runtime.script_locked.clone(),
            remember_events: runtime.remember_events.clone(),
            leave_house_number: runtime.leave_house_number.clone(),
            last_hint_actuality: runtime.last_hint_actuality.clone(),
            last_hint_subject: runtime.last_hint_subject.clone(),
            my_door_index: runtime.my_door_index.clone(),
            looking_for_help_because_enemy_seen: runtime
                .looking_for_help_because_enemy_seen
                .clone(),
            forgotten_objects: runtime.forgotten_objects.clone(),
            object_of_desire: runtime.object_of_desire.clone(),
            checkpoint_charly: runtime.checkpoint_charly.clone(),
            synchronize_charly: runtime.synchronize_charly.clone(),
            synchronize_index: runtime.synchronize_index.clone(),
            delta_sorrow_level: runtime.delta_sorrow_level.clone(),
            missed_in_action: runtime.missed_in_action.clone(),
            frame_when_enemy_detected: runtime.frame_when_enemy_detected.clone(),
            inside_halt_method: runtime.inside_halt_method.clone(),
            synchronizing_actors: runtime.synchronizing_actors.clone(),
            default_path_walking_flags: runtime.default_path_walking_flags.clone(),
            forbidden_remark_ids: runtime.forbidden_remark_ids.clone(),
            initial_view_cone: runtime.initial_view_cone.clone(),
            current_remark: runtime.current_remark.clone(),
            current_remark_flags: runtime.current_remark_flags.clone(),
            next_macro_rand: runtime.next_macro_rand.clone(),
            next_macro_rand_forecasted: runtime.next_macro_rand_forecasted.clone(),
            current_emoticon_type: runtime.current_emoticon_type.clone(),
            emoticon_expiration_date: runtime.emoticon_expiration_date.clone(),
            emoticon_has_expiration_date: runtime.emoticon_has_expiration_date.clone(),
            my_reconnaissance_report: runtime.my_reconnaissance_report.clone(),
            knocked_out_in_money_fight: runtime.knocked_out_in_money_fight.clone(),
            looted_after_money_fight: runtime.looted_after_money_fight.clone(),
            patrol_chief: runtime.patrol_chief.clone(),
            patrol: runtime.patrol.clone(),
            missed_patrol_members: runtime.missed_patrol_members.clone(),
            theoretical_patrol: runtime.theoretical_patrol.clone(),
            patrol_stopped: runtime.patrol_stopped.clone(),
            patrol_direction: runtime.patrol_direction.clone(),
            needs_patrol_reinit: runtime.needs_patrol_reinit.clone(),
            got_the_beggar_trick: runtime.got_the_beggar_trick.clone(),
            ai_log: runtime.ai_log.clone(),
            debug_view_cone_enabled: runtime.debug_view_cone_enabled.clone(),
            last_goto_destination: runtime.last_goto_destination.clone(),
            last_goto_flags: runtime.last_goto_flags.clone(),
            stuck_counter: runtime.stuck_counter.clone(),
            outbox: runtime.outbox.clone(),
            has_script_filter_override: runtime.has_script_filter_override.clone(),
            last_synced_focus_target: runtime.last_synced_focus_target.clone(),
            initial_position: runtime.initial_position.clone(),
            initial_view_direction: runtime.initial_view_direction.clone(),
            max_visibility: runtime.max_visibility.clone(),
            cached_frame: runtime.cached_frame.clone(),
            cached_in_building: runtime.cached_in_building.clone(),
        }
    }
}

impl LegacyWire for AiController {
    fn legacy_json(&self) -> String {
        serde_json::to_string(&LegacyAiController::capture(self)).unwrap()
    }
    fn legacy_hash(&self) -> u64 {
        compute(&LegacyAiController::capture(self))
    }
    fn legacy_native_bytes(&self) -> Vec<u8> {
        bitcode::encode(&LegacyAiController::capture(self))
    }
    fn legacy_from_json(json: &str) -> Self {
        let legacy: LegacyAiController = serde_json::from_str(json).unwrap();
        Self {
            me: legacy.me,
            owner_entity_id: legacy.owner_entity_id,
            path_id: legacy.path_id,
            alert_path_id: legacy.alert_path_id,
            current_state: legacy.current_state,
            current_substate: legacy.current_substate,
            old_state: legacy.old_state,
            current_music_alert_status: legacy.current_music_alert_status,
            view_alert_status: legacy.view_alert_status,
            substate_at_last_timer_launch: legacy.substate_at_last_timer_launch,
            attitude: legacy.attitude,
            blood_alcohol: legacy.blood_alcohol,
            initial_action: legacy.initial_action,
            number_of_looks: legacy.number_of_looks,
            has_patrol_path: legacy.has_patrol_path,
            patrol_path: legacy.patrol_path,
            detached_patrol_path_status: legacy.detached_patrol_path_status,
            can_move: legacy.can_move,
            stop_before_end_of_path: legacy.stop_before_end_of_path,
            use_max_norm_to_stop_before_end_of_path: legacy.use_max_norm_to_stop_before_end_of_path,
            stop_before_end_of_path_distance: legacy.stop_before_end_of_path_distance,
            think_recursion_depth: legacy.think_recursion_depth,
            open_end_think_frames: legacy.open_end_think_frames,
            engine_deferred_end_think_frames: legacy.engine_deferred_end_think_frames,
            engine_completion_verdict_resolved: legacy.engine_completion_verdict_resolved,
            macro_command: legacy.macro_command,
            macro_command_offset: legacy.macro_command_offset,
            macro_command_waypoint: legacy.macro_command_waypoint,
            number_of_remaining_macro_bytes: legacy.number_of_remaining_macro_bytes,
            macro_in_progress: legacy.macro_in_progress,
            macro_started_in_this_frame: legacy.macro_started_in_this_frame,
            primary_target: legacy.primary_target,
            friend_in_trouble: legacy.friend_in_trouble,
            detected_body: legacy.detected_body,
            interesting_object: legacy.interesting_object,
            antagonist: legacy.antagonist,
            last_stimulus_actor: legacy.last_stimulus_actor,
            timer_is_running: legacy.timer_is_running,
            when_does_timer_ring: legacy.when_does_timer_ring,
            macro_timer_is_running: legacy.macro_timer_is_running,
            when_does_macro_timer_ring: legacy.when_does_macro_timer_ring,
            standing_around_timer: legacy.standing_around_timer,
            sorrow_level: legacy.sorrow_level,
            last_stimulus: legacy.last_stimulus,
            last_stimulus_multiplicity: legacy.last_stimulus_multiplicity,
            is_master: legacy.is_master,
            master: legacy.master,
            seek_position: legacy.seek_position,
            alert_soldiers_point: legacy.alert_soldiers_point,
            first_try: legacy.first_try,
            panic_center_x: legacy.panic_center_x,
            panic_center_y: legacy.panic_center_y,
            lasting_panic_runs: legacy.lasting_panic_runs,
            directed_panic: legacy.directed_panic,
            list_us: legacy.list_us,
            list_alerted_us: legacy.list_alerted_us,
            list_staying_us: legacy.list_staying_us,
            couldnt_reachpoint: legacy.couldnt_reachpoint,
            already_on_point: legacy.already_on_point,
            already_turned: legacy.already_turned,
            completion_latch_inside_think: legacy.completion_latch_inside_think,
            likes_to_sit_around: legacy.likes_to_sit_around,
            special_action: legacy.special_action,
            remaining_tequila_gulps: legacy.remaining_tequila_gulps,
            friends_are_alerted: legacy.friends_are_alerted,
            is_stay_at_home: legacy.is_stay_at_home,
            locks_flag_field: legacy.locks_flag_field,
            was_busy: legacy.was_busy,
            stimulus_queue: legacy.stimulus_queue,
            script_locked: legacy.script_locked,
            remember_events: legacy.remember_events,
            leave_house_number: legacy.leave_house_number,
            last_hint_actuality: legacy.last_hint_actuality,
            last_hint_subject: legacy.last_hint_subject,
            my_door_index: legacy.my_door_index,
            looking_for_help_because_enemy_seen: legacy.looking_for_help_because_enemy_seen,
            forgotten_objects: legacy.forgotten_objects,
            object_of_desire: legacy.object_of_desire,
            checkpoint_charly: legacy.checkpoint_charly,
            synchronize_charly: legacy.synchronize_charly,
            synchronize_index: legacy.synchronize_index,
            delta_sorrow_level: legacy.delta_sorrow_level,
            missed_in_action: legacy.missed_in_action,
            frame_when_enemy_detected: legacy.frame_when_enemy_detected,
            inside_halt_method: legacy.inside_halt_method,
            synchronizing_actors: legacy.synchronizing_actors,
            default_path_walking_flags: legacy.default_path_walking_flags,
            forbidden_remark_ids: legacy.forbidden_remark_ids,
            initial_view_cone: legacy.initial_view_cone,
            current_remark: legacy.current_remark,
            current_remark_flags: legacy.current_remark_flags,
            next_macro_rand: legacy.next_macro_rand,
            next_macro_rand_forecasted: legacy.next_macro_rand_forecasted,
            current_emoticon_type: legacy.current_emoticon_type,
            emoticon_expiration_date: legacy.emoticon_expiration_date,
            emoticon_has_expiration_date: legacy.emoticon_has_expiration_date,
            my_reconnaissance_report: legacy.my_reconnaissance_report,
            knocked_out_in_money_fight: legacy.knocked_out_in_money_fight,
            looted_after_money_fight: legacy.looted_after_money_fight,
            patrol_chief: legacy.patrol_chief,
            patrol: legacy.patrol,
            missed_patrol_members: legacy.missed_patrol_members,
            theoretical_patrol: legacy.theoretical_patrol,
            patrol_stopped: legacy.patrol_stopped,
            patrol_direction: legacy.patrol_direction,
            needs_patrol_reinit: legacy.needs_patrol_reinit,
            got_the_beggar_trick: legacy.got_the_beggar_trick,
            ai_log: legacy.ai_log,
            debug_view_cone_enabled: legacy.debug_view_cone_enabled,
            last_goto_destination: legacy.last_goto_destination,
            last_goto_flags: legacy.last_goto_flags,
            stuck_counter: legacy.stuck_counter,
            outbox: legacy.outbox,
            has_script_filter_override: legacy.has_script_filter_override,
            last_synced_focus_target: legacy.last_synced_focus_target,
            initial_position: legacy.initial_position,
            initial_view_direction: legacy.initial_view_direction,
            max_visibility: legacy.max_visibility,
            cached_frame: legacy.cached_frame,
            cached_in_building: legacy.cached_in_building,
        }
    }
}

#[derive(Serialize, Deserialize, bitcode::Encode, robin_state_hash_derive::StateHash)]
struct LegacyAiGlobalState {
    green_alert_soldiers: u16,
    yellow_alert_soldiers: u16,
    red_alert_soldiers: u16,
    there_are_royalist_soldiers: bool,
    there_are_lacklandist_soldiers: bool,
    soldier_camps: std::collections::BTreeSet<crate::element_kinds::Camp>,
    stupid_soldiers_cheat: bool,
    freeze: bool,
    overall_alert_status: AlertLevel,
    overall_villain_alert_status: AlertLevel,
    ambush_points: Vec<AmbushPoint>,
    seek_points: Vec<SeekPoint>,
    archery_sectors: Vec<SectorArchery>,
    saved_random_seed: i64,
    remarks_forbidden_till_frame: Vec<u32>,
    forbidden_remarks: Vec<ForbiddenRemark>,
    screen_remarks: Vec<ScreenRemark>,
    attribute_display: bool,
    speech_display: bool,
    golden_eye_mode: bool,
    ezekiel_2517: bool,
    current_speech_variant: u16,
    repulsive_points: Vec<RepulsivePoint>,
    next_repulsive_point_id: i32,
    door_seek_infos: Vec<DoorSeekInfo>,
    reinforcement_doors: Vec<ReinforcementDoorInfo>,
    houses: Vec<House>,
    door_rally_points: Vec<DoorRallyPoint>,
    all_soldier_handles: std::sync::Arc<Vec<u32>>,
    same_frame_target_claims: Vec<(HumanHandle, HumanHandle)>,
    #[serde(skip)]
    #[bitcode(skip)]
    primary_target_multiplicity_scratch: std::collections::BTreeMap<HumanHandle, u32>,
    #[serde(skip)]
    #[bitcode(skip)]
    primary_target_multiplicity_initialized: bool,
}

impl LegacyAiGlobalState {
    fn capture(runtime: &AiGlobalState) -> Self {
        Self {
            green_alert_soldiers: runtime.green_alert_soldiers.clone(),
            yellow_alert_soldiers: runtime.yellow_alert_soldiers.clone(),
            red_alert_soldiers: runtime.red_alert_soldiers.clone(),
            there_are_royalist_soldiers: runtime.there_are_royalist_soldiers.clone(),
            there_are_lacklandist_soldiers: runtime.there_are_lacklandist_soldiers.clone(),
            soldier_camps: runtime.soldier_camps.clone(),
            stupid_soldiers_cheat: runtime.stupid_soldiers_cheat.clone(),
            freeze: runtime.freeze.clone(),
            overall_alert_status: runtime.overall_alert_status.clone(),
            overall_villain_alert_status: runtime.overall_villain_alert_status.clone(),
            ambush_points: runtime.ambush_points.clone(),
            seek_points: runtime.seek_points.clone(),
            archery_sectors: runtime.archery_sectors.clone(),
            saved_random_seed: runtime.saved_random_seed.clone(),
            remarks_forbidden_till_frame: runtime.remarks_forbidden_till_frame.clone(),
            forbidden_remarks: runtime.forbidden_remarks.clone(),
            screen_remarks: runtime.screen_remarks.clone(),
            attribute_display: runtime.attribute_display.clone(),
            speech_display: runtime.speech_display.clone(),
            golden_eye_mode: runtime.golden_eye_mode.clone(),
            ezekiel_2517: runtime.ezekiel_2517.clone(),
            current_speech_variant: runtime.current_speech_variant.clone(),
            repulsive_points: runtime.repulsive_points.clone(),
            next_repulsive_point_id: runtime.next_repulsive_point_id.clone(),
            door_seek_infos: runtime.door_seek_infos.clone(),
            reinforcement_doors: runtime.reinforcement_doors.clone(),
            houses: runtime.houses.clone(),
            door_rally_points: runtime.door_rally_points.clone(),
            all_soldier_handles: runtime.all_soldier_handles.clone(),
            same_frame_target_claims: runtime.same_frame_target_claims.clone(),
            primary_target_multiplicity_scratch: runtime
                .primary_target_multiplicity_scratch
                .clone(),
            primary_target_multiplicity_initialized: runtime
                .primary_target_multiplicity_initialized
                .clone(),
        }
    }
}

impl LegacyWire for AiGlobalState {
    fn legacy_json(&self) -> String {
        serde_json::to_string(&LegacyAiGlobalState::capture(self)).unwrap()
    }
    fn legacy_hash(&self) -> u64 {
        compute(&LegacyAiGlobalState::capture(self))
    }
    fn legacy_native_bytes(&self) -> Vec<u8> {
        bitcode::encode(&LegacyAiGlobalState::capture(self))
    }
    fn legacy_from_json(json: &str) -> Self {
        let legacy: LegacyAiGlobalState = serde_json::from_str(json).unwrap();
        Self {
            green_alert_soldiers: legacy.green_alert_soldiers,
            yellow_alert_soldiers: legacy.yellow_alert_soldiers,
            red_alert_soldiers: legacy.red_alert_soldiers,
            there_are_royalist_soldiers: legacy.there_are_royalist_soldiers,
            there_are_lacklandist_soldiers: legacy.there_are_lacklandist_soldiers,
            soldier_camps: legacy.soldier_camps,
            stupid_soldiers_cheat: legacy.stupid_soldiers_cheat,
            freeze: legacy.freeze,
            overall_alert_status: legacy.overall_alert_status,
            overall_villain_alert_status: legacy.overall_villain_alert_status,
            ambush_points: legacy.ambush_points,
            seek_points: legacy.seek_points,
            archery_sectors: legacy.archery_sectors,
            saved_random_seed: legacy.saved_random_seed,
            remarks_forbidden_till_frame: legacy.remarks_forbidden_till_frame,
            forbidden_remarks: legacy.forbidden_remarks,
            screen_remarks: legacy.screen_remarks,
            attribute_display: legacy.attribute_display,
            speech_display: legacy.speech_display,
            golden_eye_mode: legacy.golden_eye_mode,
            ezekiel_2517: legacy.ezekiel_2517,
            current_speech_variant: legacy.current_speech_variant,
            repulsive_points: legacy.repulsive_points,
            next_repulsive_point_id: legacy.next_repulsive_point_id,
            door_seek_infos: legacy.door_seek_infos,
            reinforcement_doors: legacy.reinforcement_doors,
            houses: legacy.houses,
            door_rally_points: legacy.door_rally_points,
            all_soldier_handles: legacy.all_soldier_handles,
            same_frame_target_claims: legacy.same_frame_target_claims,
            primary_target_multiplicity_scratch: legacy.primary_target_multiplicity_scratch,
            primary_target_multiplicity_initialized: legacy.primary_target_multiplicity_initialized,
        }
    }
}

#[derive(Serialize, Deserialize, bitcode::Encode)]
#[serde(transparent)]
struct LegacyQueuedSelfStimulus {
    stimulus_type: StimulusType,
    #[serde(skip)]
    #[bitcode(skip)]
    origin: SelfStimulusOrigin,
}

impl robin_util::state_hash::StateHash for LegacyQueuedSelfStimulus {
    fn state_hash<H: std::hash::Hasher>(&self, state: &mut H) {
        robin_util::state_hash::StateHash::state_hash(&self.stimulus_type, state);
    }
}

impl LegacyQueuedSelfStimulus {
    fn capture(runtime: &QueuedSelfStimulus) -> Self {
        Self {
            stimulus_type: runtime.stimulus_type.clone(),
            origin: runtime.origin.clone(),
        }
    }
}

impl LegacyWire for QueuedSelfStimulus {
    fn legacy_json(&self) -> String {
        serde_json::to_string(&LegacyQueuedSelfStimulus::capture(self)).unwrap()
    }
    fn legacy_hash(&self) -> u64 {
        compute(&LegacyQueuedSelfStimulus::capture(self))
    }
    fn legacy_native_bytes(&self) -> Vec<u8> {
        bitcode::encode(&LegacyQueuedSelfStimulus::capture(self))
    }
    fn legacy_from_json(json: &str) -> Self {
        let legacy: LegacyQueuedSelfStimulus = serde_json::from_str(json).unwrap();
        Self {
            stimulus_type: legacy.stimulus_type,
            origin: legacy.origin,
        }
    }
}

#[derive(Serialize, Deserialize, bitcode::Encode)]
struct LegacyStimulus {
    stimulus_type: StimulusType,
    info: StimulusInfo,
    #[serde(
        serialize_with = "serialize_optional_ai_handle",
        deserialize_with = "deserialize_optional_ai_handle"
    )]
    owner: Option<AiEntityHandle>,
    to_whole_patrol: bool,
    #[serde(skip)]
    #[bitcode(skip)]
    self_origin: SelfStimulusOrigin,
}

impl robin_util::state_hash::StateHash for LegacyStimulus {
    fn state_hash<H: std::hash::Hasher>(&self, state: &mut H) {
        robin_util::state_hash::StateHash::state_hash(&self.stimulus_type, state);
        robin_util::state_hash::StateHash::state_hash(&self.info, state);
        robin_util::state_hash::StateHash::state_hash(&self.owner, state);
        robin_util::state_hash::StateHash::state_hash(&self.to_whole_patrol, state);
    }
}

impl LegacyStimulus {
    fn capture(runtime: &Stimulus) -> Self {
        Self {
            stimulus_type: runtime.stimulus_type.clone(),
            info: runtime.info.clone(),
            owner: runtime.owner.clone(),
            to_whole_patrol: runtime.to_whole_patrol.clone(),
            self_origin: runtime.self_origin.clone(),
        }
    }
}

impl LegacyWire for Stimulus {
    fn legacy_json(&self) -> String {
        serde_json::to_string(&LegacyStimulus::capture(self)).unwrap()
    }
    fn legacy_hash(&self) -> u64 {
        compute(&LegacyStimulus::capture(self))
    }
    fn legacy_native_bytes(&self) -> Vec<u8> {
        bitcode::encode(&LegacyStimulus::capture(self))
    }
    fn legacy_from_json(json: &str) -> Self {
        let legacy: LegacyStimulus = serde_json::from_str(json).unwrap();
        Self {
            stimulus_type: legacy.stimulus_type,
            info: legacy.info,
            owner: legacy.owner,
            to_whole_patrol: legacy.to_whole_patrol,
            self_origin: legacy.self_origin,
        }
    }
}

#[derive(Serialize, Deserialize, bitcode::Encode, robin_state_hash_derive::StateHash)]
struct LegacyAiOutbox {
    patrol: AiPatrolOutbox,
    detection: AiDetectionOutbox,
    reentrant: AiReentrantOutbox,
    actor: AiActorOutbox,
    recovery: AiRecoveryOutbox,
    music: AiMusicOutbox,
}

impl LegacyAiOutbox {
    fn capture(runtime: &AiOutbox) -> Self {
        Self {
            patrol: runtime.patrol.clone(),
            detection: runtime.detection.clone(),
            reentrant: runtime.reentrant.clone(),
            actor: runtime.actor.clone(),
            recovery: runtime.recovery.clone(),
            music: runtime.music.clone(),
        }
    }
}

impl LegacyWire for AiOutbox {
    fn legacy_json(&self) -> String {
        serde_json::to_string(&LegacyAiOutbox::capture(self)).unwrap()
    }
    fn legacy_hash(&self) -> u64 {
        compute(&LegacyAiOutbox::capture(self))
    }
    fn legacy_native_bytes(&self) -> Vec<u8> {
        bitcode::encode(&LegacyAiOutbox::capture(self))
    }
    fn legacy_from_json(json: &str) -> Self {
        let legacy: LegacyAiOutbox = serde_json::from_str(json).unwrap();
        Self {
            patrol: legacy.patrol,
            detection: legacy.detection,
            reentrant: legacy.reentrant,
            actor: legacy.actor,
            recovery: legacy.recovery,
            music: legacy.music,
        }
    }
}

#[derive(Serialize, Deserialize, bitcode::Encode, robin_state_hash_derive::StateHash)]
struct LegacyAiDetectionOutbox {
    stimuli: Vec<Stimulus>,
    mark_alerted: bool,
}

impl LegacyAiDetectionOutbox {
    fn capture(runtime: &AiDetectionOutbox) -> Self {
        Self {
            stimuli: runtime.stimuli.clone(),
            mark_alerted: runtime.mark_alerted.clone(),
        }
    }
}

impl LegacyWire for AiDetectionOutbox {
    fn legacy_json(&self) -> String {
        serde_json::to_string(&LegacyAiDetectionOutbox::capture(self)).unwrap()
    }
    fn legacy_hash(&self) -> u64 {
        compute(&LegacyAiDetectionOutbox::capture(self))
    }
    fn legacy_native_bytes(&self) -> Vec<u8> {
        bitcode::encode(&LegacyAiDetectionOutbox::capture(self))
    }
    fn legacy_from_json(json: &str) -> Self {
        let legacy: LegacyAiDetectionOutbox = serde_json::from_str(json).unwrap();
        Self {
            stimuli: legacy.stimuli,
            mark_alerted: legacy.mark_alerted,
        }
    }
}

#[derive(Serialize, Deserialize, bitcode::Encode, robin_state_hash_derive::StateHash)]
struct LegacyAiReentrantOutbox {
    #[serde(skip)]
    #[state_hash(skip)]
    #[bitcode(skip)]
    engine_drains_after_script_go_on: bool,
    cross_npc_actions: Vec<CrossNpcAction>,
    self_stimuli: Vec<QueuedSelfStimulus>,
    finish_macro_after_self_stimuli: bool,
    owner_work: Vec<AiOwnerWork>,
    reconsider_approach_completion_pending: bool,
    #[serde(default)]
    reconsider_approach_replaced_path_waiter: bool,
    #[serde(default)]
    battle_observe_completion_pending: bool,
    look_for_help_completion_pending: bool,
    waypoint_script_reach_point: Option<(PathId, u8)>,
    #[serde(default)]
    alert_soldier_completion_pending: bool,
    #[serde(default)]
    dead_body_alert_completion_pending: bool,
    #[serde(default)]
    tower_guard_alert_officer_completion_pending: bool,
    #[serde(default)]
    civilian_report_alert_officer_completion_pending: bool,
    #[serde(default)]
    brawl_hitting_completion_pending: bool,
}

impl LegacyAiReentrantOutbox {
    fn capture(runtime: &AiReentrantOutbox) -> Self {
        Self {
            engine_drains_after_script_go_on: runtime.engine_drains_after_script_go_on.clone(),
            cross_npc_actions: runtime.cross_npc_actions.clone(),
            self_stimuli: runtime.self_stimuli.clone(),
            finish_macro_after_self_stimuli: runtime.finish_macro_after_self_stimuli.clone(),
            owner_work: runtime.owner_work.clone(),
            reconsider_approach_completion_pending: runtime
                .reconsider_approach_completion_pending
                .clone(),
            reconsider_approach_replaced_path_waiter: runtime
                .reconsider_approach_replaced_path_waiter
                .clone(),
            battle_observe_completion_pending: runtime.battle_observe_completion_pending.clone(),
            look_for_help_completion_pending: runtime.look_for_help_completion_pending.clone(),
            waypoint_script_reach_point: runtime.waypoint_script_reach_point.clone(),
            alert_soldier_completion_pending: runtime.alert_soldier_completion_pending.clone(),
            dead_body_alert_completion_pending: runtime.dead_body_alert_completion_pending.clone(),
            tower_guard_alert_officer_completion_pending: runtime
                .tower_guard_alert_officer_completion_pending
                .clone(),
            civilian_report_alert_officer_completion_pending: runtime
                .civilian_report_alert_officer_completion_pending
                .clone(),
            brawl_hitting_completion_pending: runtime.brawl_hitting_completion_pending.clone(),
        }
    }
}

impl LegacyWire for AiReentrantOutbox {
    fn legacy_json(&self) -> String {
        serde_json::to_string(&LegacyAiReentrantOutbox::capture(self)).unwrap()
    }
    fn legacy_hash(&self) -> u64 {
        compute(&LegacyAiReentrantOutbox::capture(self))
    }
    fn legacy_native_bytes(&self) -> Vec<u8> {
        bitcode::encode(&LegacyAiReentrantOutbox::capture(self))
    }
    fn legacy_from_json(json: &str) -> Self {
        let legacy: LegacyAiReentrantOutbox = serde_json::from_str(json).unwrap();
        Self {
            engine_drains_after_script_go_on: legacy.engine_drains_after_script_go_on,
            cross_npc_actions: legacy.cross_npc_actions,
            self_stimuli: legacy.self_stimuli,
            finish_macro_after_self_stimuli: legacy.finish_macro_after_self_stimuli,
            owner_work: legacy.owner_work,
            reconsider_approach_completion_pending: legacy.reconsider_approach_completion_pending,
            reconsider_approach_replaced_path_waiter: legacy
                .reconsider_approach_replaced_path_waiter,
            battle_observe_completion_pending: legacy.battle_observe_completion_pending,
            look_for_help_completion_pending: legacy.look_for_help_completion_pending,
            waypoint_script_reach_point: legacy.waypoint_script_reach_point,
            alert_soldier_completion_pending: legacy.alert_soldier_completion_pending,
            dead_body_alert_completion_pending: legacy.dead_body_alert_completion_pending,
            tower_guard_alert_officer_completion_pending: legacy
                .tower_guard_alert_officer_completion_pending,
            civilian_report_alert_officer_completion_pending: legacy
                .civilian_report_alert_officer_completion_pending,
            brawl_hitting_completion_pending: legacy.brawl_hitting_completion_pending,
        }
    }
}

#[derive(Serialize, Deserialize, bitcode::Encode, robin_state_hash_derive::StateHash)]
struct LegacyEnemyAi {
    base: AiController,
    pending_special_strike: bool,
    #[serde(default)]
    pending_sword_strike_consideration: bool,
    #[serde(default)]
    pending_combat_insult_after_strike_consideration: bool,
    #[serde(
        default,
        serialize_with = "serialize_optional_ai_handle",
        deserialize_with = "deserialize_optional_ai_handle"
    )]
    missed_pc: Option<AiEntityHandle>,
    pc_missed: bool,
    pc_gone_away_in_this_direction: u16,
    frame_when_missed_charly: u32,
    heard_nets: Vec<ObjectHandle>,
    detected_something_there: Position,
    #[serde(default)]
    investigating_distraction: bool,
    last_seek_direction_index: u8,
    #[serde(
        default,
        serialize_with = "serialize_optional_ai_handle",
        deserialize_with = "deserialize_optional_ai_handle"
    )]
    beggar_to_examine: Option<AiEntityHandle>,
    beggar_is_npc: bool,
    current_task_priority: u16,
    minimal_task_priority: u16,
    new_task_priority: u16,
    number_of_different_checkpoints: u8,
    thirsty: bool,
    position_change_locked_for_test: bool,
    other_bodies_to_examine: Vec<HumanHandle>,
    beggars_to_control: Vec<HumanHandle>,
    positions_of_beggars_to_control: Vec<Position>,
    seen_dead_body: bool,
    seeking_charly: bool,
    my_seek_points: Vec<u16>,
    personal_seek_point_1: Option<SeekPoint>,
    personal_seek_point_2: Option<SeekPoint>,
    seek_center: Position,
    actual_seek_point: Option<u16>,
    seek_point_view_directions: Vec<u16>,
    seek_flags: SeekFlags,
    old_odds: i16,
    gather_position: Position,
    gather_direction: u16,
    gather_position_instructed: bool,
    search_charly_way: Vec<Position>,
    officers_position: Position,
    previous_state: i32,
    previous_substate: i32,
    reported_to_officer: bool,
    missed_soldier_timer: u16,
    old_money: u16,
    other_seen_money: Vec<ObjectHandle>,
    other_seen_ale: Vec<ObjectHandle>,
    money_fight_enemies: Vec<NpcHandle>,
    money_fight_victims: Vec<NpcHandle>,
    #[serde(
        default,
        serialize_with = "serialize_optional_ai_handle",
        deserialize_with = "deserialize_optional_ai_handle"
    )]
    archer_behind_me: Option<AiEntityHandle>,
    #[serde(
        default,
        serialize_with = "serialize_optional_ai_handle",
        deserialize_with = "deserialize_optional_ai_handle"
    )]
    shield_bearer_before_me: Option<AiEntityHandle>,
    shield_bearer_direction: u16,
    phalanx_aborted: bool,
    changed_to_alert_path: bool,
    already_seen_bodies: Vec<HumanHandle>,
    alerted_us: Vec<HumanHandle>,
    pending_alert_soldier_candidates: Vec<HumanHandle>,
    #[serde(default)]
    pending_group_instruction_candidates: Vec<(HumanHandle, Position)>,
    #[serde(default)]
    pending_group_instruction_seek_flags: u16,
    #[serde(default)]
    pending_group_instruction_clear_location_after_accept: bool,
    my_shooting_point: Option<(u16, u16)>,
    my_archery_sector: Option<u16>,
    my_archery_sector_index: u16,
    my_archery_point_index: crate::sector::ArcheryPointIdx,
    my_archery_point_increment: i8,
    enemy_seen_below: bool,
    enemy_had_this_elevation: u16,
    known_enemy_strike_1: Option<crate::weapons::SwordStrike>,
    known_enemy_strike_2: Option<crate::weapons::SwordStrike>,
    known_enemy_strike_3: Option<crate::weapons::SwordStrike>,
    return_to_patrol_point: Position,
    fleeing_seen_enemy_counter: u16,
    last_stimulus_dispatched_to_patrol: Option<Stimulus>,
    character_id: u32,
    old_life_points: u8,
    initial_life_points: u8,
    list_them: Vec<HumanHandle>,
    ambush_point_array_reset: bool,
    ambush_point_status: Vec<AmbushPointStatus>,
    forced_next_battle_decision: Decision,
    reset_battle_decision: bool,
    soldier_profile_iq: u16,
    soldier_profile_courage: u16,
    soldier_profile_shooting: u16,
    soldier_profile_vip: bool,
    soldier_profile_bee_time: u16,
    soldier_profile_pride: u16,
    soldier_profile_hearing_factor: f32,
    soldier_profile_rank: ProfileRank,
    soldier_profile_initiative: u16,
    soldier_profile_beer: u16,
    #[serde(default)]
    ale_reliable_distraction: bool,
    soldier_profile_money: u16,
    soldier_profile_apple: u16,
    soldier_profile_whistle: u16,
    soldier_profile_duty: bool,
    soldier_profile_endurance: u16,
    is_vip: bool,
    sword_range: u16,
    hth_weapon_id: u32,
    sword_is_charge_weapon: bool,
    next_sword_strike_frame: u32,
    company_number: u16,
    #[serde(
        default,
        serialize_with = "serialize_optional_ai_handle",
        deserialize_with = "deserialize_optional_ai_handle"
    )]
    left_combat_neighbour: Option<AiEntityHandle>,
    #[serde(
        default,
        serialize_with = "serialize_optional_ai_handle",
        deserialize_with = "deserialize_optional_ai_handle"
    )]
    right_combat_neighbour: Option<AiEntityHandle>,
    attentive: bool,
    will_be_attentive: bool,
    forced_attentive: bool,
    guarded_pc: Option<PcId>,
    my_line_jump: Option<u32>,
    tower_guard: bool,
    combat_trainer: bool,
    is_archer_unit: bool,
}

impl LegacyEnemyAi {
    fn capture(runtime: &EnemyAi) -> Self {
        Self {
            base: runtime.base.clone(),
            pending_special_strike: runtime.pending_special_strike.clone(),
            pending_sword_strike_consideration: runtime.pending_sword_strike_consideration.clone(),
            pending_combat_insult_after_strike_consideration: runtime
                .pending_combat_insult_after_strike_consideration
                .clone(),
            missed_pc: runtime.missed_pc.clone(),
            pc_missed: runtime.pc_missed.clone(),
            pc_gone_away_in_this_direction: runtime.pc_gone_away_in_this_direction.clone(),
            frame_when_missed_charly: runtime.frame_when_missed_charly.clone(),
            heard_nets: runtime.heard_nets.clone(),
            detected_something_there: runtime.detected_something_there.clone(),
            investigating_distraction: runtime.investigating_distraction.clone(),
            last_seek_direction_index: runtime.last_seek_direction_index.clone(),
            beggar_to_examine: runtime.beggar_to_examine.clone(),
            beggar_is_npc: runtime.beggar_is_npc.clone(),
            current_task_priority: runtime.current_task_priority.clone(),
            minimal_task_priority: runtime.minimal_task_priority.clone(),
            new_task_priority: runtime.new_task_priority.clone(),
            number_of_different_checkpoints: runtime.number_of_different_checkpoints.clone(),
            thirsty: runtime.thirsty.clone(),
            position_change_locked_for_test: runtime.position_change_locked_for_test.clone(),
            other_bodies_to_examine: runtime.other_bodies_to_examine.clone(),
            beggars_to_control: runtime.beggars_to_control.clone(),
            positions_of_beggars_to_control: runtime.positions_of_beggars_to_control.clone(),
            seen_dead_body: runtime.seen_dead_body.clone(),
            seeking_charly: runtime.seeking_charly.clone(),
            my_seek_points: runtime.my_seek_points.clone(),
            personal_seek_point_1: runtime.personal_seek_point_1.clone(),
            personal_seek_point_2: runtime.personal_seek_point_2.clone(),
            seek_center: runtime.seek_center.clone(),
            actual_seek_point: runtime.actual_seek_point.clone(),
            seek_point_view_directions: runtime.seek_point_view_directions.clone(),
            seek_flags: runtime.seek_flags.clone(),
            old_odds: runtime.old_odds.clone(),
            gather_position: runtime.gather_position.clone(),
            gather_direction: runtime.gather_direction.clone(),
            gather_position_instructed: runtime.gather_position_instructed.clone(),
            search_charly_way: runtime.search_charly_way.clone(),
            officers_position: runtime.officers_position.clone(),
            previous_state: runtime.previous_state.clone(),
            previous_substate: runtime.previous_substate.clone(),
            reported_to_officer: runtime.reported_to_officer.clone(),
            missed_soldier_timer: runtime.missed_soldier_timer.clone(),
            old_money: runtime.old_money.clone(),
            other_seen_money: runtime.other_seen_money.clone(),
            other_seen_ale: runtime.other_seen_ale.clone(),
            money_fight_enemies: runtime.money_fight_enemies.clone(),
            money_fight_victims: runtime.money_fight_victims.clone(),
            archer_behind_me: runtime.archer_behind_me.clone(),
            shield_bearer_before_me: runtime.shield_bearer_before_me.clone(),
            shield_bearer_direction: runtime.shield_bearer_direction.clone(),
            phalanx_aborted: runtime.phalanx_aborted.clone(),
            changed_to_alert_path: runtime.changed_to_alert_path.clone(),
            already_seen_bodies: runtime.already_seen_bodies.clone(),
            alerted_us: runtime.alerted_us.clone(),
            pending_alert_soldier_candidates: runtime.pending_alert_soldier_candidates.clone(),
            pending_group_instruction_candidates: runtime
                .pending_group_instruction_candidates
                .clone(),
            pending_group_instruction_seek_flags: runtime
                .pending_group_instruction_seek_flags
                .clone(),
            pending_group_instruction_clear_location_after_accept: runtime
                .pending_group_instruction_clear_location_after_accept
                .clone(),
            my_shooting_point: runtime.my_shooting_point.clone(),
            my_archery_sector: runtime.my_archery_sector.clone(),
            my_archery_sector_index: runtime.my_archery_sector_index.clone(),
            my_archery_point_index: runtime.my_archery_point_index.clone(),
            my_archery_point_increment: runtime.my_archery_point_increment.clone(),
            enemy_seen_below: runtime.enemy_seen_below.clone(),
            enemy_had_this_elevation: runtime.enemy_had_this_elevation.clone(),
            known_enemy_strike_1: runtime.known_enemy_strike_1.clone(),
            known_enemy_strike_2: runtime.known_enemy_strike_2.clone(),
            known_enemy_strike_3: runtime.known_enemy_strike_3.clone(),
            return_to_patrol_point: runtime.return_to_patrol_point.clone(),
            fleeing_seen_enemy_counter: runtime.fleeing_seen_enemy_counter.clone(),
            last_stimulus_dispatched_to_patrol: runtime.last_stimulus_dispatched_to_patrol.clone(),
            character_id: runtime.character_id.clone(),
            old_life_points: runtime.old_life_points.clone(),
            initial_life_points: runtime.initial_life_points.clone(),
            list_them: runtime.list_them.clone(),
            ambush_point_array_reset: runtime.ambush_point_array_reset.clone(),
            ambush_point_status: runtime.ambush_point_status.clone(),
            forced_next_battle_decision: runtime.forced_next_battle_decision.clone(),
            reset_battle_decision: runtime.reset_battle_decision.clone(),
            soldier_profile_iq: runtime.soldier_profile_iq.clone(),
            soldier_profile_courage: runtime.soldier_profile_courage.clone(),
            soldier_profile_shooting: runtime.soldier_profile_shooting.clone(),
            soldier_profile_vip: runtime.soldier_profile_vip.clone(),
            soldier_profile_bee_time: runtime.soldier_profile_bee_time.clone(),
            soldier_profile_pride: runtime.soldier_profile_pride.clone(),
            soldier_profile_hearing_factor: runtime.soldier_profile_hearing_factor.clone(),
            soldier_profile_rank: runtime.soldier_profile_rank.clone(),
            soldier_profile_initiative: runtime.soldier_profile_initiative.clone(),
            soldier_profile_beer: runtime.soldier_profile_beer.clone(),
            ale_reliable_distraction: runtime.ale_reliable_distraction.clone(),
            soldier_profile_money: runtime.soldier_profile_money.clone(),
            soldier_profile_apple: runtime.soldier_profile_apple.clone(),
            soldier_profile_whistle: runtime.soldier_profile_whistle.clone(),
            soldier_profile_duty: runtime.soldier_profile_duty.clone(),
            soldier_profile_endurance: runtime.soldier_profile_endurance.clone(),
            is_vip: runtime.is_vip.clone(),
            sword_range: runtime.sword_range.clone(),
            hth_weapon_id: runtime.hth_weapon_id.clone(),
            sword_is_charge_weapon: runtime.sword_is_charge_weapon.clone(),
            next_sword_strike_frame: runtime.next_sword_strike_frame.clone(),
            company_number: runtime.company_number.clone(),
            left_combat_neighbour: runtime.left_combat_neighbour.clone(),
            right_combat_neighbour: runtime.right_combat_neighbour.clone(),
            attentive: runtime.attentive.clone(),
            will_be_attentive: runtime.will_be_attentive.clone(),
            forced_attentive: runtime.forced_attentive.clone(),
            guarded_pc: runtime.guarded_pc.clone(),
            my_line_jump: runtime.my_line_jump.clone(),
            tower_guard: runtime.tower_guard.clone(),
            combat_trainer: runtime.combat_trainer.clone(),
            is_archer_unit: runtime.is_archer_unit.clone(),
        }
    }
}

impl LegacyWire for EnemyAi {
    fn legacy_json(&self) -> String {
        serde_json::to_string(&LegacyEnemyAi::capture(self)).unwrap()
    }
    fn legacy_hash(&self) -> u64 {
        compute(&LegacyEnemyAi::capture(self))
    }
    fn legacy_native_bytes(&self) -> Vec<u8> {
        bitcode::encode(&LegacyEnemyAi::capture(self))
    }
    fn legacy_from_json(json: &str) -> Self {
        let legacy: LegacyEnemyAi = serde_json::from_str(json).unwrap();
        Self {
            base: legacy.base,
            pending_special_strike: legacy.pending_special_strike,
            pending_sword_strike_consideration: legacy.pending_sword_strike_consideration,
            pending_combat_insult_after_strike_consideration: legacy
                .pending_combat_insult_after_strike_consideration,
            missed_pc: legacy.missed_pc,
            pc_missed: legacy.pc_missed,
            pc_gone_away_in_this_direction: legacy.pc_gone_away_in_this_direction,
            frame_when_missed_charly: legacy.frame_when_missed_charly,
            heard_nets: legacy.heard_nets,
            detected_something_there: legacy.detected_something_there,
            investigating_distraction: legacy.investigating_distraction,
            last_seek_direction_index: legacy.last_seek_direction_index,
            beggar_to_examine: legacy.beggar_to_examine,
            beggar_is_npc: legacy.beggar_is_npc,
            current_task_priority: legacy.current_task_priority,
            minimal_task_priority: legacy.minimal_task_priority,
            new_task_priority: legacy.new_task_priority,
            number_of_different_checkpoints: legacy.number_of_different_checkpoints,
            thirsty: legacy.thirsty,
            position_change_locked_for_test: legacy.position_change_locked_for_test,
            other_bodies_to_examine: legacy.other_bodies_to_examine,
            beggars_to_control: legacy.beggars_to_control,
            positions_of_beggars_to_control: legacy.positions_of_beggars_to_control,
            seen_dead_body: legacy.seen_dead_body,
            seeking_charly: legacy.seeking_charly,
            my_seek_points: legacy.my_seek_points,
            personal_seek_point_1: legacy.personal_seek_point_1,
            personal_seek_point_2: legacy.personal_seek_point_2,
            seek_center: legacy.seek_center,
            actual_seek_point: legacy.actual_seek_point,
            seek_point_view_directions: legacy.seek_point_view_directions,
            seek_flags: legacy.seek_flags,
            old_odds: legacy.old_odds,
            gather_position: legacy.gather_position,
            gather_direction: legacy.gather_direction,
            gather_position_instructed: legacy.gather_position_instructed,
            search_charly_way: legacy.search_charly_way,
            officers_position: legacy.officers_position,
            previous_state: legacy.previous_state,
            previous_substate: legacy.previous_substate,
            reported_to_officer: legacy.reported_to_officer,
            missed_soldier_timer: legacy.missed_soldier_timer,
            old_money: legacy.old_money,
            other_seen_money: legacy.other_seen_money,
            other_seen_ale: legacy.other_seen_ale,
            money_fight_enemies: legacy.money_fight_enemies,
            money_fight_victims: legacy.money_fight_victims,
            archer_behind_me: legacy.archer_behind_me,
            shield_bearer_before_me: legacy.shield_bearer_before_me,
            shield_bearer_direction: legacy.shield_bearer_direction,
            phalanx_aborted: legacy.phalanx_aborted,
            changed_to_alert_path: legacy.changed_to_alert_path,
            already_seen_bodies: legacy.already_seen_bodies,
            alerted_us: legacy.alerted_us,
            pending_alert_soldier_candidates: legacy.pending_alert_soldier_candidates,
            pending_group_instruction_candidates: legacy.pending_group_instruction_candidates,
            pending_group_instruction_seek_flags: legacy.pending_group_instruction_seek_flags,
            pending_group_instruction_clear_location_after_accept: legacy
                .pending_group_instruction_clear_location_after_accept,
            my_shooting_point: legacy.my_shooting_point,
            my_archery_sector: legacy.my_archery_sector,
            my_archery_sector_index: legacy.my_archery_sector_index,
            my_archery_point_index: legacy.my_archery_point_index,
            my_archery_point_increment: legacy.my_archery_point_increment,
            enemy_seen_below: legacy.enemy_seen_below,
            enemy_had_this_elevation: legacy.enemy_had_this_elevation,
            known_enemy_strike_1: legacy.known_enemy_strike_1,
            known_enemy_strike_2: legacy.known_enemy_strike_2,
            known_enemy_strike_3: legacy.known_enemy_strike_3,
            return_to_patrol_point: legacy.return_to_patrol_point,
            fleeing_seen_enemy_counter: legacy.fleeing_seen_enemy_counter,
            last_stimulus_dispatched_to_patrol: legacy.last_stimulus_dispatched_to_patrol,
            character_id: legacy.character_id,
            old_life_points: legacy.old_life_points,
            initial_life_points: legacy.initial_life_points,
            list_them: legacy.list_them,
            ambush_point_array_reset: legacy.ambush_point_array_reset,
            ambush_point_status: legacy.ambush_point_status,
            forced_next_battle_decision: legacy.forced_next_battle_decision,
            reset_battle_decision: legacy.reset_battle_decision,
            soldier_profile_iq: legacy.soldier_profile_iq,
            soldier_profile_courage: legacy.soldier_profile_courage,
            soldier_profile_shooting: legacy.soldier_profile_shooting,
            soldier_profile_vip: legacy.soldier_profile_vip,
            soldier_profile_bee_time: legacy.soldier_profile_bee_time,
            soldier_profile_pride: legacy.soldier_profile_pride,
            soldier_profile_hearing_factor: legacy.soldier_profile_hearing_factor,
            soldier_profile_rank: legacy.soldier_profile_rank,
            soldier_profile_initiative: legacy.soldier_profile_initiative,
            soldier_profile_beer: legacy.soldier_profile_beer,
            ale_reliable_distraction: legacy.ale_reliable_distraction,
            soldier_profile_money: legacy.soldier_profile_money,
            soldier_profile_apple: legacy.soldier_profile_apple,
            soldier_profile_whistle: legacy.soldier_profile_whistle,
            soldier_profile_duty: legacy.soldier_profile_duty,
            soldier_profile_endurance: legacy.soldier_profile_endurance,
            is_vip: legacy.is_vip,
            sword_range: legacy.sword_range,
            hth_weapon_id: legacy.hth_weapon_id,
            sword_is_charge_weapon: legacy.sword_is_charge_weapon,
            next_sword_strike_frame: legacy.next_sword_strike_frame,
            company_number: legacy.company_number,
            left_combat_neighbour: legacy.left_combat_neighbour,
            right_combat_neighbour: legacy.right_combat_neighbour,
            attentive: legacy.attentive,
            will_be_attentive: legacy.will_be_attentive,
            forced_attentive: legacy.forced_attentive,
            guarded_pc: legacy.guarded_pc,
            my_line_jump: legacy.my_line_jump,
            tower_guard: legacy.tower_guard,
            combat_trainer: legacy.combat_trainer,
            is_archer_unit: legacy.is_archer_unit,
        }
    }
}

#[derive(Serialize, Deserialize, bitcode::Encode, robin_state_hash_derive::StateHash)]
struct LegacyFriendlyAi {
    base: AiController,
    beggar_dont_talk_counter: u16,
    fleeing_seen_enemy_counter: u16,
    wants_to_talk: bool,
    last_talk_partner: Option<AiEntityHandle>,
    can_go_away: bool,
}

impl LegacyFriendlyAi {
    fn capture(runtime: &FriendlyAi) -> Self {
        Self {
            base: runtime.base.clone(),
            beggar_dont_talk_counter: runtime.beggar_dont_talk_counter.clone(),
            fleeing_seen_enemy_counter: runtime.fleeing_seen_enemy_counter.clone(),
            wants_to_talk: runtime.wants_to_talk.clone(),
            last_talk_partner: runtime.last_talk_partner.clone(),
            can_go_away: runtime.can_go_away.clone(),
        }
    }
}

impl LegacyWire for FriendlyAi {
    fn legacy_json(&self) -> String {
        serde_json::to_string(&LegacyFriendlyAi::capture(self)).unwrap()
    }
    fn legacy_hash(&self) -> u64 {
        compute(&LegacyFriendlyAi::capture(self))
    }
    fn legacy_native_bytes(&self) -> Vec<u8> {
        bitcode::encode(&LegacyFriendlyAi::capture(self))
    }
    fn legacy_from_json(json: &str) -> Self {
        let legacy: LegacyFriendlyAi = serde_json::from_str(json).unwrap();
        Self {
            base: legacy.base,
            beggar_dont_talk_counter: legacy.beggar_dont_talk_counter,
            fleeing_seen_enemy_counter: legacy.fleeing_seen_enemy_counter,
            wants_to_talk: legacy.wants_to_talk,
            last_talk_partner: legacy.last_talk_partner,
            can_go_away: legacy.can_go_away,
        }
    }
}
