// Test-only copies of the pre-projection serde, native and hash declarations.
// Keep independent field/default policies to detect accidental wire changes.
use super::*;

pub(super) trait LegacyWire: Sized {
    fn legacy_json(&self) -> String;
    fn legacy_decoded_bytes_and_hash(json: &str) -> (Vec<u8>, u64);
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
    #[serde(with = "optional_ai_handle")]
    primary_target: Option<AiEntityHandle>,
    #[serde(with = "optional_ai_handle")]
    friend_in_trouble: Option<AiEntityHandle>,
    #[serde(with = "optional_ai_handle")]
    detected_body: Option<AiEntityHandle>,
    #[serde(with = "optional_ai_handle")]
    interesting_object: Option<AiEntityHandle>,
    #[serde(with = "optional_ai_handle")]
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
    #[serde(with = "optional_ai_handle")]
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
    #[serde(with = "optional_ai_handle")]
    object_of_desire: Option<AiEntityHandle>,
    #[serde(with = "optional_ai_handle")]
    checkpoint_charly: Option<AiEntityHandle>,
    #[serde(with = "optional_ai_handle")]
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
            me: runtime.me,
            owner_entity_id: runtime.owner_entity_id,
            path_id: runtime.path_id,
            alert_path_id: runtime.alert_path_id,
            current_state: runtime.current_state,
            current_substate: runtime.current_substate,
            old_state: runtime.old_state,
            current_music_alert_status: runtime.current_music_alert_status,
            view_alert_status: runtime.view_alert_status,
            substate_at_last_timer_launch: runtime.substate_at_last_timer_launch,
            attitude: runtime.attitude,
            blood_alcohol: runtime.blood_alcohol,
            initial_action: runtime.initial_action,
            number_of_looks: runtime.number_of_looks,
            has_patrol_path: runtime.has_patrol_path,
            patrol_path: runtime.patrol_path.clone(),
            detached_patrol_path_status: runtime.detached_patrol_path_status.clone(),
            can_move: runtime.can_move,
            stop_before_end_of_path: runtime.stop_before_end_of_path,
            use_max_norm_to_stop_before_end_of_path: runtime
                .use_max_norm_to_stop_before_end_of_path,
            stop_before_end_of_path_distance: runtime.stop_before_end_of_path_distance,
            think_recursion_depth: runtime.think_recursion_depth,
            open_end_think_frames: runtime.open_end_think_frames,
            engine_deferred_end_think_frames: runtime.engine_deferred_end_think_frames,
            engine_completion_verdict_resolved: runtime.engine_completion_verdict_resolved,
            macro_command: runtime.macro_command.clone(),
            macro_command_offset: runtime.macro_command_offset,
            macro_command_waypoint: runtime.macro_command_waypoint,
            number_of_remaining_macro_bytes: runtime.number_of_remaining_macro_bytes,
            macro_in_progress: runtime.macro_in_progress,
            macro_started_in_this_frame: runtime.macro_started_in_this_frame,
            primary_target: runtime.primary_target,
            friend_in_trouble: runtime.friend_in_trouble,
            detected_body: runtime.detected_body,
            interesting_object: runtime.interesting_object,
            antagonist: runtime.antagonist,
            last_stimulus_actor: runtime.last_stimulus_actor,
            timer_is_running: runtime.timer_is_running,
            when_does_timer_ring: runtime.when_does_timer_ring,
            macro_timer_is_running: runtime.macro_timer_is_running,
            when_does_macro_timer_ring: runtime.when_does_macro_timer_ring,
            standing_around_timer: runtime.standing_around_timer,
            sorrow_level: runtime.sorrow_level,
            last_stimulus: runtime.last_stimulus,
            last_stimulus_multiplicity: runtime.last_stimulus_multiplicity,
            is_master: runtime.is_master,
            master: runtime.master,
            seek_position: runtime.seek_position,
            alert_soldiers_point: runtime.alert_soldiers_point,
            first_try: runtime.first_try,
            panic_center_x: runtime.panic_center_x,
            panic_center_y: runtime.panic_center_y,
            lasting_panic_runs: runtime.lasting_panic_runs,
            directed_panic: runtime.directed_panic,
            list_us: runtime.list_us.clone(),
            list_alerted_us: runtime.list_alerted_us.clone(),
            list_staying_us: runtime.list_staying_us.clone(),
            couldnt_reachpoint: runtime.couldnt_reachpoint,
            already_on_point: runtime.already_on_point,
            already_turned: runtime.already_turned,
            completion_latch_inside_think: runtime.completion_latch_inside_think,
            likes_to_sit_around: runtime.likes_to_sit_around,
            special_action: runtime.special_action,
            remaining_tequila_gulps: runtime.remaining_tequila_gulps,
            friends_are_alerted: runtime.friends_are_alerted,
            is_stay_at_home: runtime.is_stay_at_home,
            locks_flag_field: runtime.locks_flag_field,
            was_busy: runtime.was_busy,
            stimulus_queue: runtime.stimulus_queue.clone(),
            script_locked: runtime.script_locked,
            remember_events: runtime.remember_events,
            leave_house_number: runtime.leave_house_number,
            last_hint_actuality: runtime.last_hint_actuality,
            last_hint_subject: runtime.last_hint_subject,
            my_door_index: runtime.my_door_index,
            looking_for_help_because_enemy_seen: runtime.looking_for_help_because_enemy_seen,
            forgotten_objects: runtime.forgotten_objects.clone(),
            object_of_desire: runtime.object_of_desire,
            checkpoint_charly: runtime.checkpoint_charly,
            synchronize_charly: runtime.synchronize_charly,
            synchronize_index: runtime.synchronize_index,
            delta_sorrow_level: runtime.delta_sorrow_level,
            missed_in_action: runtime.missed_in_action.clone(),
            frame_when_enemy_detected: runtime.frame_when_enemy_detected,
            inside_halt_method: runtime.inside_halt_method,
            synchronizing_actors: runtime.synchronizing_actors.clone(),
            default_path_walking_flags: runtime.default_path_walking_flags,
            forbidden_remark_ids: runtime.forbidden_remark_ids.clone(),
            initial_view_cone: runtime.initial_view_cone,
            current_remark: runtime.current_remark,
            current_remark_flags: runtime.current_remark_flags,
            next_macro_rand: runtime.next_macro_rand,
            next_macro_rand_forecasted: runtime.next_macro_rand_forecasted,
            current_emoticon_type: runtime.current_emoticon_type,
            emoticon_expiration_date: runtime.emoticon_expiration_date,
            emoticon_has_expiration_date: runtime.emoticon_has_expiration_date,
            my_reconnaissance_report: runtime.my_reconnaissance_report.clone(),
            knocked_out_in_money_fight: runtime.knocked_out_in_money_fight,
            looted_after_money_fight: runtime.looted_after_money_fight,
            patrol_chief: runtime.patrol_chief,
            patrol: runtime.patrol.clone(),
            missed_patrol_members: runtime.missed_patrol_members.clone(),
            theoretical_patrol: runtime.theoretical_patrol.clone(),
            patrol_stopped: runtime.patrol_stopped,
            patrol_direction: runtime.patrol_direction,
            needs_patrol_reinit: runtime.needs_patrol_reinit,
            got_the_beggar_trick: runtime.got_the_beggar_trick,
            ai_log: runtime.ai_log.clone(),
            debug_view_cone_enabled: runtime.debug_view_cone_enabled,
            last_goto_destination: runtime.last_goto_destination,
            last_goto_flags: runtime.last_goto_flags,
            stuck_counter: runtime.stuck_counter,
            outbox: runtime.outbox.clone(),
            has_script_filter_override: runtime.has_script_filter_override,
            last_synced_focus_target: runtime.last_synced_focus_target,
            initial_position: runtime.initial_position,
            initial_view_direction: runtime.initial_view_direction,
            max_visibility: runtime.max_visibility,
            cached_frame: runtime.cached_frame,
            cached_in_building: runtime.cached_in_building,
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
    fn legacy_decoded_bytes_and_hash(json: &str) -> (Vec<u8>, u64) {
        let legacy: LegacyAiController = serde_json::from_str(json).unwrap();
        (bitcode::encode(&legacy), compute(&legacy))
    }
}

#[derive(Serialize, Deserialize, bitcode::Encode, robin_state_hash_derive::StateHash)]
struct LegacyAiGlobalState {
    green_alert_soldiers: u16,
    yellow_alert_soldiers: u16,
    red_alert_soldiers: u16,
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
            green_alert_soldiers: runtime.green_alert_soldiers,
            yellow_alert_soldiers: runtime.yellow_alert_soldiers,
            red_alert_soldiers: runtime.red_alert_soldiers,
            soldier_camps: runtime.soldier_camps.clone(),
            stupid_soldiers_cheat: runtime.stupid_soldiers_cheat,
            freeze: runtime.freeze,
            overall_alert_status: runtime.overall_alert_status,
            overall_villain_alert_status: runtime.overall_villain_alert_status,
            ambush_points: runtime.ambush_points.clone(),
            seek_points: runtime.seek_points.clone(),
            archery_sectors: runtime.archery_sectors.clone(),
            saved_random_seed: runtime.saved_random_seed,
            remarks_forbidden_till_frame: runtime.remarks_forbidden_till_frame.clone(),
            forbidden_remarks: runtime.forbidden_remarks.clone(),
            screen_remarks: runtime.screen_remarks.clone(),
            attribute_display: runtime.attribute_display,
            speech_display: runtime.speech_display,
            golden_eye_mode: runtime.golden_eye_mode,
            ezekiel_2517: runtime.ezekiel_2517,
            current_speech_variant: runtime.current_speech_variant,
            repulsive_points: runtime.repulsive_points.clone(),
            next_repulsive_point_id: runtime.next_repulsive_point_id,
            door_seek_infos: runtime.door_seek_infos.clone(),
            reinforcement_doors: runtime.reinforcement_doors.clone(),
            houses: runtime.houses.clone(),
            door_rally_points: runtime.door_rally_points.clone(),
            all_soldier_handles: runtime.all_soldier_handles.clone(),
            primary_target_multiplicity_scratch: runtime
                .primary_target_multiplicity_scratch
                .clone(),
            primary_target_multiplicity_initialized: runtime
                .primary_target_multiplicity_initialized,
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
    fn legacy_decoded_bytes_and_hash(json: &str) -> (Vec<u8>, u64) {
        let legacy: LegacyAiGlobalState = serde_json::from_str(json).unwrap();
        (bitcode::encode(&legacy), compute(&legacy))
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
            stimulus_type: runtime.stimulus_type,
            origin: runtime.origin,
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
    fn legacy_decoded_bytes_and_hash(json: &str) -> (Vec<u8>, u64) {
        let legacy: LegacyQueuedSelfStimulus = serde_json::from_str(json).unwrap();
        (bitcode::encode(&legacy), compute(&legacy))
    }
}

#[derive(Serialize, Deserialize, bitcode::Encode)]
struct LegacyStimulus {
    stimulus_type: StimulusType,
    info: StimulusInfo,
    #[serde(with = "optional_ai_handle")]
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
            stimulus_type: runtime.stimulus_type,
            info: runtime.info,
            owner: runtime.owner,
            to_whole_patrol: runtime.to_whole_patrol,
            self_origin: runtime.self_origin,
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
    fn legacy_decoded_bytes_and_hash(json: &str) -> (Vec<u8>, u64) {
        let legacy: LegacyStimulus = serde_json::from_str(json).unwrap();
        (bitcode::encode(&legacy), compute(&legacy))
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
    fn legacy_decoded_bytes_and_hash(json: &str) -> (Vec<u8>, u64) {
        let legacy: LegacyAiOutbox = serde_json::from_str(json).unwrap();
        (bitcode::encode(&legacy), compute(&legacy))
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
            mark_alerted: runtime.mark_alerted,
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
    fn legacy_decoded_bytes_and_hash(json: &str) -> (Vec<u8>, u64) {
        let legacy: LegacyAiDetectionOutbox = serde_json::from_str(json).unwrap();
        (bitcode::encode(&legacy), compute(&legacy))
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
            engine_drains_after_script_go_on: runtime.engine_drains_after_script_go_on,
            cross_npc_actions: runtime.cross_npc_actions.clone(),
            self_stimuli: runtime.self_stimuli.clone(),
            finish_macro_after_self_stimuli: runtime.finish_macro_after_self_stimuli,
            owner_work: runtime.owner_work.clone(),
            reconsider_approach_completion_pending: runtime.reconsider_approach_completion_pending,
            reconsider_approach_replaced_path_waiter: runtime
                .reconsider_approach_replaced_path_waiter,
            battle_observe_completion_pending: runtime.battle_observe_completion_pending,
            look_for_help_completion_pending: runtime.look_for_help_completion_pending,
            waypoint_script_reach_point: runtime.waypoint_script_reach_point,
            alert_soldier_completion_pending: runtime.alert_soldier_completion_pending,
            dead_body_alert_completion_pending: runtime.dead_body_alert_completion_pending,
            tower_guard_alert_officer_completion_pending: runtime
                .tower_guard_alert_officer_completion_pending,
            civilian_report_alert_officer_completion_pending: runtime
                .civilian_report_alert_officer_completion_pending,
            brawl_hitting_completion_pending: runtime.brawl_hitting_completion_pending,
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
    fn legacy_decoded_bytes_and_hash(json: &str) -> (Vec<u8>, u64) {
        let legacy: LegacyAiReentrantOutbox = serde_json::from_str(json).unwrap();
        (bitcode::encode(&legacy), compute(&legacy))
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
    #[serde(default, with = "optional_ai_handle")]
    missed_pc: Option<AiEntityHandle>,
    pc_missed: bool,
    pc_gone_away_in_this_direction: u16,
    frame_when_missed_charly: u32,
    heard_nets: Vec<ObjectHandle>,
    detected_something_there: Position,
    #[serde(default)]
    investigating_distraction: bool,
    last_seek_direction_index: u8,
    #[serde(default, with = "optional_ai_handle")]
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
    #[serde(default, with = "optional_ai_handle")]
    archer_behind_me: Option<AiEntityHandle>,
    #[serde(default, with = "optional_ai_handle")]
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
    #[serde(default, with = "optional_ai_handle")]
    left_combat_neighbour: Option<AiEntityHandle>,
    #[serde(default, with = "optional_ai_handle")]
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
            pending_special_strike: runtime.pending_special_strike,
            pending_sword_strike_consideration: runtime.pending_sword_strike_consideration,
            pending_combat_insult_after_strike_consideration: runtime
                .pending_combat_insult_after_strike_consideration,
            missed_pc: runtime.missed_pc,
            pc_missed: runtime.pc_missed,
            pc_gone_away_in_this_direction: runtime.pc_gone_away_in_this_direction,
            frame_when_missed_charly: runtime.frame_when_missed_charly,
            heard_nets: runtime.heard_nets.clone(),
            detected_something_there: runtime.detected_something_there,
            investigating_distraction: runtime.investigating_distraction,
            last_seek_direction_index: runtime.last_seek_direction_index,
            beggar_to_examine: runtime.beggar_to_examine,
            beggar_is_npc: runtime.beggar_is_npc,
            current_task_priority: runtime.current_task_priority,
            minimal_task_priority: runtime.minimal_task_priority,
            new_task_priority: runtime.new_task_priority,
            number_of_different_checkpoints: runtime.number_of_different_checkpoints,
            thirsty: runtime.thirsty,
            position_change_locked_for_test: runtime.position_change_locked_for_test,
            other_bodies_to_examine: runtime.other_bodies_to_examine.clone(),
            beggars_to_control: runtime.beggars_to_control.clone(),
            positions_of_beggars_to_control: runtime.positions_of_beggars_to_control.clone(),
            seen_dead_body: runtime.seen_dead_body,
            seeking_charly: runtime.seeking_charly,
            my_seek_points: runtime.my_seek_points.clone(),
            personal_seek_point_1: runtime.personal_seek_point_1.clone(),
            personal_seek_point_2: runtime.personal_seek_point_2.clone(),
            seek_center: runtime.seek_center,
            actual_seek_point: runtime.actual_seek_point,
            seek_point_view_directions: runtime.seek_point_view_directions.clone(),
            seek_flags: runtime.seek_flags,
            old_odds: runtime.old_odds,
            gather_position: runtime.gather_position,
            gather_direction: runtime.gather_direction,
            gather_position_instructed: runtime.gather_position_instructed,
            search_charly_way: runtime.search_charly_way.clone(),
            officers_position: runtime.officers_position,
            previous_state: runtime.previous_state,
            previous_substate: runtime.previous_substate,
            reported_to_officer: runtime.reported_to_officer,
            missed_soldier_timer: runtime.missed_soldier_timer,
            old_money: runtime.old_money,
            other_seen_money: runtime.other_seen_money.clone(),
            other_seen_ale: runtime.other_seen_ale.clone(),
            money_fight_enemies: runtime.money_fight_enemies.clone(),
            money_fight_victims: runtime.money_fight_victims.clone(),
            archer_behind_me: runtime.archer_behind_me,
            shield_bearer_before_me: runtime.shield_bearer_before_me,
            shield_bearer_direction: runtime.shield_bearer_direction,
            phalanx_aborted: runtime.phalanx_aborted,
            changed_to_alert_path: runtime.changed_to_alert_path,
            already_seen_bodies: runtime.already_seen_bodies.clone(),
            alerted_us: runtime.alerted_us.clone(),
            pending_alert_soldier_candidates: runtime.pending_alert_soldier_candidates.clone(),
            pending_group_instruction_candidates: runtime
                .pending_group_instruction_candidates
                .clone(),
            pending_group_instruction_seek_flags: runtime.pending_group_instruction_seek_flags,
            pending_group_instruction_clear_location_after_accept: runtime
                .pending_group_instruction_clear_location_after_accept,
            my_shooting_point: runtime.my_shooting_point,
            my_archery_sector: runtime.my_archery_sector,
            my_archery_sector_index: runtime.my_archery_sector_index,
            my_archery_point_index: runtime.my_archery_point_index,
            my_archery_point_increment: runtime.my_archery_point_increment,
            enemy_seen_below: runtime.enemy_seen_below,
            enemy_had_this_elevation: runtime.enemy_had_this_elevation,
            known_enemy_strike_1: runtime.known_enemy_strike_1,
            known_enemy_strike_2: runtime.known_enemy_strike_2,
            known_enemy_strike_3: runtime.known_enemy_strike_3,
            return_to_patrol_point: runtime.return_to_patrol_point,
            fleeing_seen_enemy_counter: runtime.fleeing_seen_enemy_counter,
            last_stimulus_dispatched_to_patrol: runtime.last_stimulus_dispatched_to_patrol,
            character_id: runtime.character_id,
            old_life_points: runtime.old_life_points,
            initial_life_points: runtime.initial_life_points,
            list_them: runtime.list_them.clone(),
            ambush_point_array_reset: runtime.ambush_point_array_reset,
            ambush_point_status: runtime.ambush_point_status.clone(),
            forced_next_battle_decision: runtime.forced_next_battle_decision,
            reset_battle_decision: runtime.reset_battle_decision,
            soldier_profile_iq: runtime.soldier_profile_iq,
            soldier_profile_courage: runtime.soldier_profile_courage,
            soldier_profile_shooting: runtime.soldier_profile_shooting,
            soldier_profile_vip: runtime.soldier_profile_vip,
            soldier_profile_bee_time: runtime.soldier_profile_bee_time,
            soldier_profile_pride: runtime.soldier_profile_pride,
            soldier_profile_hearing_factor: runtime.soldier_profile_hearing_factor,
            soldier_profile_rank: runtime.soldier_profile_rank,
            soldier_profile_initiative: runtime.soldier_profile_initiative,
            soldier_profile_beer: runtime.soldier_profile_beer,
            ale_reliable_distraction: runtime.ale_reliable_distraction,
            soldier_profile_money: runtime.soldier_profile_money,
            soldier_profile_apple: runtime.soldier_profile_apple,
            soldier_profile_whistle: runtime.soldier_profile_whistle,
            soldier_profile_duty: runtime.soldier_profile_duty,
            soldier_profile_endurance: runtime.soldier_profile_endurance,
            is_vip: runtime.is_vip,
            sword_range: runtime.sword_range,
            hth_weapon_id: runtime.hth_weapon_id,
            sword_is_charge_weapon: runtime.sword_is_charge_weapon,
            next_sword_strike_frame: runtime.next_sword_strike_frame,
            company_number: runtime.company_number,
            left_combat_neighbour: runtime.left_combat_neighbour,
            right_combat_neighbour: runtime.right_combat_neighbour,
            attentive: runtime.attentive,
            will_be_attentive: runtime.will_be_attentive,
            forced_attentive: runtime.forced_attentive,
            guarded_pc: runtime.guarded_pc,
            my_line_jump: runtime.my_line_jump,
            tower_guard: runtime.tower_guard,
            combat_trainer: runtime.combat_trainer,
            is_archer_unit: runtime.is_archer_unit,
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
    fn legacy_decoded_bytes_and_hash(json: &str) -> (Vec<u8>, u64) {
        let legacy: LegacyEnemyAi = serde_json::from_str(json).unwrap();
        (bitcode::encode(&legacy), compute(&legacy))
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
            beggar_dont_talk_counter: runtime.beggar_dont_talk_counter,
            fleeing_seen_enemy_counter: runtime.fleeing_seen_enemy_counter,
            wants_to_talk: runtime.wants_to_talk,
            last_talk_partner: runtime.last_talk_partner,
            can_go_away: runtime.can_go_away,
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
    fn legacy_decoded_bytes_and_hash(json: &str) -> (Vec<u8>, u64) {
        let legacy: LegacyFriendlyAi = serde_json::from_str(json).unwrap();
        (bitcode::encode(&legacy), compute(&legacy))
    }
}
