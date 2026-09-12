//! Typed save projections for AI owners.
//!
//! Capture copies only persisted fields; reconstruction creates fresh callback
//! provenance and scratch state. Raw runtime Clone remains rollback-exact.
//! Field-exhaustive patterns make additions to live owners require a decision
//! here, instead of silently inheriting a serializer's runtime-field policy.

use super::*;
use crate::ai_enemy::{AmbushPointStatus, EnemyAi, ProfileRank, SeekFlags};
use crate::ai_friendly::FriendlyAi;
use crate::entity_id::PcId;

impl Serialize for AiController {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        PersistedAiController::capture(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AiController {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        PersistedAiController::deserialize(deserializer).map(PersistedAiController::into_runtime)
    }
}

impl Serialize for AiGlobalState {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        PersistedAiGlobalState::capture(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AiGlobalState {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        PersistedAiGlobalState::deserialize(deserializer).map(PersistedAiGlobalState::into_runtime)
    }
}

impl Serialize for QueuedSelfStimulus {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        PersistedQueuedSelfStimulus::capture(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for QueuedSelfStimulus {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        PersistedQueuedSelfStimulus::deserialize(deserializer)
            .map(PersistedQueuedSelfStimulus::into_runtime)
    }
}

impl Serialize for Stimulus {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        PersistedStimulus::capture(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Stimulus {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        PersistedStimulus::deserialize(deserializer).map(PersistedStimulus::into_runtime)
    }
}

impl Serialize for AiOutbox {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        PersistedAiOutbox::capture(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AiOutbox {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        PersistedAiOutbox::deserialize(deserializer).map(PersistedAiOutbox::into_runtime)
    }
}

impl Serialize for AiDetectionOutbox {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        PersistedAiDetectionOutbox::capture(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AiDetectionOutbox {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        PersistedAiDetectionOutbox::deserialize(deserializer)
            .map(PersistedAiDetectionOutbox::into_runtime)
    }
}

impl Serialize for AiReentrantOutbox {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        PersistedAiReentrantOutbox::capture(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AiReentrantOutbox {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        PersistedAiReentrantOutbox::deserialize(deserializer)
            .map(PersistedAiReentrantOutbox::into_runtime)
    }
}

impl Serialize for EnemyAi {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        PersistedEnemyAi::capture(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for EnemyAi {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        PersistedEnemyAi::deserialize(deserializer).map(PersistedEnemyAi::into_runtime)
    }
}

impl Serialize for FriendlyAi {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        PersistedFriendlyAi::capture(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for FriendlyAi {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        PersistedFriendlyAi::deserialize(deserializer).map(PersistedFriendlyAi::into_runtime)
    }
}

#[cfg(test)]
mod tests;

/// Persisted field projection of [`AiController`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedAiController {
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
    stimulus_queue: Vec<PersistedStimulus>,
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
    outbox: PersistedAiOutbox,
    has_script_filter_override: bool,
    last_synced_focus_target: Option<AiEntityHandle>,
    initial_position: Position,
    initial_view_direction: u16,
    max_visibility: u32,
    cached_frame: u32,
    cached_in_building: bool,
}

impl PersistedAiController {
    pub fn capture(runtime: &AiController) -> Self {
        let AiController {
            me,
            owner_entity_id,
            path_id,
            alert_path_id,
            current_state,
            current_substate,
            old_state,
            current_music_alert_status,
            view_alert_status,
            substate_at_last_timer_launch,
            attitude,
            blood_alcohol,
            initial_action,
            number_of_looks,
            has_patrol_path,
            patrol_path,
            detached_patrol_path_status,
            can_move,
            stop_before_end_of_path,
            use_max_norm_to_stop_before_end_of_path,
            stop_before_end_of_path_distance,
            think_recursion_depth,
            open_end_think_frames: _,
            engine_deferred_end_think_frames: _,
            engine_completion_verdict_resolved: _,
            macro_command,
            macro_command_offset,
            macro_command_waypoint,
            number_of_remaining_macro_bytes,
            macro_in_progress,
            macro_started_in_this_frame,
            primary_target,
            friend_in_trouble,
            detected_body,
            interesting_object,
            antagonist,
            last_stimulus_actor,
            timer_is_running,
            when_does_timer_ring,
            macro_timer_is_running,
            when_does_macro_timer_ring,
            standing_around_timer,
            sorrow_level,
            last_stimulus,
            last_stimulus_multiplicity,
            is_master,
            master,
            seek_position,
            alert_soldiers_point,
            first_try,
            panic_center_x,
            panic_center_y,
            lasting_panic_runs,
            directed_panic,
            list_us,
            list_alerted_us,
            list_staying_us,
            couldnt_reachpoint,
            already_on_point,
            already_turned,
            completion_latch_inside_think,
            likes_to_sit_around,
            special_action,
            remaining_tequila_gulps,
            friends_are_alerted,
            is_stay_at_home,
            locks_flag_field,
            was_busy,
            stimulus_queue,
            script_locked,
            remember_events,
            leave_house_number,
            last_hint_actuality,
            last_hint_subject,
            my_door_index,
            looking_for_help_because_enemy_seen,
            forgotten_objects,
            object_of_desire,
            checkpoint_charly,
            synchronize_charly,
            synchronize_index,
            delta_sorrow_level,
            missed_in_action,
            frame_when_enemy_detected,
            inside_halt_method,
            synchronizing_actors,
            default_path_walking_flags,
            forbidden_remark_ids,
            initial_view_cone,
            current_remark,
            current_remark_flags,
            next_macro_rand,
            next_macro_rand_forecasted,
            current_emoticon_type,
            emoticon_expiration_date,
            emoticon_has_expiration_date,
            my_reconnaissance_report,
            knocked_out_in_money_fight,
            looted_after_money_fight,
            patrol_chief,
            patrol,
            missed_patrol_members,
            theoretical_patrol,
            patrol_stopped,
            patrol_direction,
            needs_patrol_reinit,
            got_the_beggar_trick,
            ai_log,
            debug_view_cone_enabled,
            last_goto_destination,
            last_goto_flags,
            stuck_counter,
            outbox,
            has_script_filter_override,
            last_synced_focus_target,
            initial_position,
            initial_view_direction,
            max_visibility,
            cached_frame,
            cached_in_building,
        } = runtime;
        Self {
            me: *me,
            owner_entity_id: *owner_entity_id,
            path_id: *path_id,
            alert_path_id: *alert_path_id,
            current_state: *current_state,
            current_substate: *current_substate,
            old_state: *old_state,
            current_music_alert_status: *current_music_alert_status,
            view_alert_status: *view_alert_status,
            substate_at_last_timer_launch: *substate_at_last_timer_launch,
            attitude: *attitude,
            blood_alcohol: *blood_alcohol,
            initial_action: *initial_action,
            number_of_looks: *number_of_looks,
            has_patrol_path: *has_patrol_path,
            patrol_path: patrol_path.clone(),
            detached_patrol_path_status: detached_patrol_path_status.clone(),
            can_move: *can_move,
            stop_before_end_of_path: *stop_before_end_of_path,
            use_max_norm_to_stop_before_end_of_path: *use_max_norm_to_stop_before_end_of_path,
            stop_before_end_of_path_distance: *stop_before_end_of_path_distance,
            think_recursion_depth: *think_recursion_depth,
            macro_command: macro_command.clone(),
            macro_command_offset: *macro_command_offset,
            macro_command_waypoint: *macro_command_waypoint,
            number_of_remaining_macro_bytes: *number_of_remaining_macro_bytes,
            macro_in_progress: *macro_in_progress,
            macro_started_in_this_frame: *macro_started_in_this_frame,
            primary_target: *primary_target,
            friend_in_trouble: *friend_in_trouble,
            detected_body: *detected_body,
            interesting_object: *interesting_object,
            antagonist: *antagonist,
            last_stimulus_actor: *last_stimulus_actor,
            timer_is_running: *timer_is_running,
            when_does_timer_ring: *when_does_timer_ring,
            macro_timer_is_running: *macro_timer_is_running,
            when_does_macro_timer_ring: *when_does_macro_timer_ring,
            standing_around_timer: *standing_around_timer,
            sorrow_level: *sorrow_level,
            last_stimulus: *last_stimulus,
            last_stimulus_multiplicity: *last_stimulus_multiplicity,
            is_master: *is_master,
            master: *master,
            seek_position: *seek_position,
            alert_soldiers_point: *alert_soldiers_point,
            first_try: *first_try,
            panic_center_x: *panic_center_x,
            panic_center_y: *panic_center_y,
            lasting_panic_runs: *lasting_panic_runs,
            directed_panic: *directed_panic,
            list_us: list_us.clone(),
            list_alerted_us: list_alerted_us.clone(),
            list_staying_us: list_staying_us.clone(),
            couldnt_reachpoint: *couldnt_reachpoint,
            already_on_point: *already_on_point,
            already_turned: *already_turned,
            completion_latch_inside_think: *completion_latch_inside_think,
            likes_to_sit_around: *likes_to_sit_around,
            special_action: *special_action,
            remaining_tequila_gulps: *remaining_tequila_gulps,
            friends_are_alerted: *friends_are_alerted,
            is_stay_at_home: *is_stay_at_home,
            locks_flag_field: *locks_flag_field,
            was_busy: *was_busy,
            stimulus_queue: stimulus_queue
                .iter()
                .map(PersistedStimulus::capture)
                .collect(),
            script_locked: *script_locked,
            remember_events: *remember_events,
            leave_house_number: *leave_house_number,
            last_hint_actuality: *last_hint_actuality,
            last_hint_subject: *last_hint_subject,
            my_door_index: *my_door_index,
            looking_for_help_because_enemy_seen: *looking_for_help_because_enemy_seen,
            forgotten_objects: forgotten_objects.clone(),
            object_of_desire: *object_of_desire,
            checkpoint_charly: *checkpoint_charly,
            synchronize_charly: *synchronize_charly,
            synchronize_index: *synchronize_index,
            delta_sorrow_level: *delta_sorrow_level,
            missed_in_action: missed_in_action.clone(),
            frame_when_enemy_detected: *frame_when_enemy_detected,
            inside_halt_method: *inside_halt_method,
            synchronizing_actors: synchronizing_actors.clone(),
            default_path_walking_flags: *default_path_walking_flags,
            forbidden_remark_ids: forbidden_remark_ids.clone(),
            initial_view_cone: *initial_view_cone,
            current_remark: *current_remark,
            current_remark_flags: *current_remark_flags,
            next_macro_rand: *next_macro_rand,
            next_macro_rand_forecasted: *next_macro_rand_forecasted,
            current_emoticon_type: *current_emoticon_type,
            emoticon_expiration_date: *emoticon_expiration_date,
            emoticon_has_expiration_date: *emoticon_has_expiration_date,
            my_reconnaissance_report: my_reconnaissance_report.clone(),
            knocked_out_in_money_fight: *knocked_out_in_money_fight,
            looted_after_money_fight: *looted_after_money_fight,
            patrol_chief: *patrol_chief,
            patrol: patrol.clone(),
            missed_patrol_members: missed_patrol_members.clone(),
            theoretical_patrol: theoretical_patrol.clone(),
            patrol_stopped: *patrol_stopped,
            patrol_direction: *patrol_direction,
            needs_patrol_reinit: *needs_patrol_reinit,
            got_the_beggar_trick: *got_the_beggar_trick,
            ai_log: ai_log.clone(),
            debug_view_cone_enabled: *debug_view_cone_enabled,
            last_goto_destination: *last_goto_destination,
            last_goto_flags: *last_goto_flags,
            stuck_counter: *stuck_counter,
            outbox: PersistedAiOutbox::capture(outbox),
            has_script_filter_override: *has_script_filter_override,
            last_synced_focus_target: *last_synced_focus_target,
            initial_position: *initial_position,
            initial_view_direction: *initial_view_direction,
            max_visibility: *max_visibility,
            cached_frame: *cached_frame,
            cached_in_building: *cached_in_building,
        }
    }

    pub fn into_runtime(self) -> AiController {
        AiController {
            me: self.me,
            owner_entity_id: self.owner_entity_id,
            path_id: self.path_id,
            alert_path_id: self.alert_path_id,
            current_state: self.current_state,
            current_substate: self.current_substate,
            old_state: self.old_state,
            current_music_alert_status: self.current_music_alert_status,
            view_alert_status: self.view_alert_status,
            substate_at_last_timer_launch: self.substate_at_last_timer_launch,
            attitude: self.attitude,
            blood_alcohol: self.blood_alcohol,
            initial_action: self.initial_action,
            number_of_looks: self.number_of_looks,
            has_patrol_path: self.has_patrol_path,
            patrol_path: self.patrol_path,
            detached_patrol_path_status: self.detached_patrol_path_status,
            can_move: self.can_move,
            stop_before_end_of_path: self.stop_before_end_of_path,
            use_max_norm_to_stop_before_end_of_path: self.use_max_norm_to_stop_before_end_of_path,
            stop_before_end_of_path_distance: self.stop_before_end_of_path_distance,
            think_recursion_depth: self.think_recursion_depth,
            open_end_think_frames: Default::default(),
            engine_deferred_end_think_frames: Default::default(),
            engine_completion_verdict_resolved: Default::default(),
            macro_command: self.macro_command,
            macro_command_offset: self.macro_command_offset,
            macro_command_waypoint: self.macro_command_waypoint,
            number_of_remaining_macro_bytes: self.number_of_remaining_macro_bytes,
            macro_in_progress: self.macro_in_progress,
            macro_started_in_this_frame: self.macro_started_in_this_frame,
            primary_target: self.primary_target,
            friend_in_trouble: self.friend_in_trouble,
            detected_body: self.detected_body,
            interesting_object: self.interesting_object,
            antagonist: self.antagonist,
            last_stimulus_actor: self.last_stimulus_actor,
            timer_is_running: self.timer_is_running,
            when_does_timer_ring: self.when_does_timer_ring,
            macro_timer_is_running: self.macro_timer_is_running,
            when_does_macro_timer_ring: self.when_does_macro_timer_ring,
            standing_around_timer: self.standing_around_timer,
            sorrow_level: self.sorrow_level,
            last_stimulus: self.last_stimulus,
            last_stimulus_multiplicity: self.last_stimulus_multiplicity,
            is_master: self.is_master,
            master: self.master,
            seek_position: self.seek_position,
            alert_soldiers_point: self.alert_soldiers_point,
            first_try: self.first_try,
            panic_center_x: self.panic_center_x,
            panic_center_y: self.panic_center_y,
            lasting_panic_runs: self.lasting_panic_runs,
            directed_panic: self.directed_panic,
            list_us: self.list_us,
            list_alerted_us: self.list_alerted_us,
            list_staying_us: self.list_staying_us,
            couldnt_reachpoint: self.couldnt_reachpoint,
            already_on_point: self.already_on_point,
            already_turned: self.already_turned,
            completion_latch_inside_think: self.completion_latch_inside_think,
            likes_to_sit_around: self.likes_to_sit_around,
            special_action: self.special_action,
            remaining_tequila_gulps: self.remaining_tequila_gulps,
            friends_are_alerted: self.friends_are_alerted,
            is_stay_at_home: self.is_stay_at_home,
            locks_flag_field: self.locks_flag_field,
            was_busy: self.was_busy,
            stimulus_queue: self
                .stimulus_queue
                .into_iter()
                .map(|value| value.into_runtime())
                .collect(),
            script_locked: self.script_locked,
            remember_events: self.remember_events,
            leave_house_number: self.leave_house_number,
            last_hint_actuality: self.last_hint_actuality,
            last_hint_subject: self.last_hint_subject,
            my_door_index: self.my_door_index,
            looking_for_help_because_enemy_seen: self.looking_for_help_because_enemy_seen,
            forgotten_objects: self.forgotten_objects,
            object_of_desire: self.object_of_desire,
            checkpoint_charly: self.checkpoint_charly,
            synchronize_charly: self.synchronize_charly,
            synchronize_index: self.synchronize_index,
            delta_sorrow_level: self.delta_sorrow_level,
            missed_in_action: self.missed_in_action,
            frame_when_enemy_detected: self.frame_when_enemy_detected,
            inside_halt_method: self.inside_halt_method,
            synchronizing_actors: self.synchronizing_actors,
            default_path_walking_flags: self.default_path_walking_flags,
            forbidden_remark_ids: self.forbidden_remark_ids,
            initial_view_cone: self.initial_view_cone,
            current_remark: self.current_remark,
            current_remark_flags: self.current_remark_flags,
            next_macro_rand: self.next_macro_rand,
            next_macro_rand_forecasted: self.next_macro_rand_forecasted,
            current_emoticon_type: self.current_emoticon_type,
            emoticon_expiration_date: self.emoticon_expiration_date,
            emoticon_has_expiration_date: self.emoticon_has_expiration_date,
            my_reconnaissance_report: self.my_reconnaissance_report,
            knocked_out_in_money_fight: self.knocked_out_in_money_fight,
            looted_after_money_fight: self.looted_after_money_fight,
            patrol_chief: self.patrol_chief,
            patrol: self.patrol,
            missed_patrol_members: self.missed_patrol_members,
            theoretical_patrol: self.theoretical_patrol,
            patrol_stopped: self.patrol_stopped,
            patrol_direction: self.patrol_direction,
            needs_patrol_reinit: self.needs_patrol_reinit,
            got_the_beggar_trick: self.got_the_beggar_trick,
            ai_log: self.ai_log,
            debug_view_cone_enabled: self.debug_view_cone_enabled,
            last_goto_destination: self.last_goto_destination,
            last_goto_flags: self.last_goto_flags,
            stuck_counter: self.stuck_counter,
            outbox: self.outbox.into_runtime(),
            has_script_filter_override: self.has_script_filter_override,
            last_synced_focus_target: self.last_synced_focus_target,
            initial_position: self.initial_position,
            initial_view_direction: self.initial_view_direction,
            max_visibility: self.max_visibility,
            cached_frame: self.cached_frame,
            cached_in_building: self.cached_in_building,
        }
    }
}

/// Persisted field projection of [`AiGlobalState`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedAiGlobalState {
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
}

impl PersistedAiGlobalState {
    pub fn capture(runtime: &AiGlobalState) -> Self {
        let AiGlobalState {
            green_alert_soldiers,
            yellow_alert_soldiers,
            red_alert_soldiers,
            soldier_camps,
            stupid_soldiers_cheat,
            freeze,
            overall_alert_status,
            overall_villain_alert_status,
            ambush_points,
            seek_points,
            archery_sectors,
            saved_random_seed,
            remarks_forbidden_till_frame,
            forbidden_remarks,
            screen_remarks,
            attribute_display,
            speech_display,
            golden_eye_mode,
            ezekiel_2517,
            current_speech_variant,
            repulsive_points,
            next_repulsive_point_id,
            door_seek_infos,
            reinforcement_doors,
            houses,
            door_rally_points,
            all_soldier_handles,
            primary_target_multiplicity_scratch: _,
            primary_target_multiplicity_initialized: _,
        } = runtime;
        Self {
            green_alert_soldiers: *green_alert_soldiers,
            yellow_alert_soldiers: *yellow_alert_soldiers,
            red_alert_soldiers: *red_alert_soldiers,
            soldier_camps: soldier_camps.clone(),
            stupid_soldiers_cheat: *stupid_soldiers_cheat,
            freeze: *freeze,
            overall_alert_status: *overall_alert_status,
            overall_villain_alert_status: *overall_villain_alert_status,
            ambush_points: ambush_points.clone(),
            seek_points: seek_points.clone(),
            archery_sectors: archery_sectors.clone(),
            saved_random_seed: *saved_random_seed,
            remarks_forbidden_till_frame: remarks_forbidden_till_frame.clone(),
            forbidden_remarks: forbidden_remarks.clone(),
            screen_remarks: screen_remarks.clone(),
            attribute_display: *attribute_display,
            speech_display: *speech_display,
            golden_eye_mode: *golden_eye_mode,
            ezekiel_2517: *ezekiel_2517,
            current_speech_variant: *current_speech_variant,
            repulsive_points: repulsive_points.clone(),
            next_repulsive_point_id: *next_repulsive_point_id,
            door_seek_infos: door_seek_infos.clone(),
            reinforcement_doors: reinforcement_doors.clone(),
            houses: houses.clone(),
            door_rally_points: door_rally_points.clone(),
            all_soldier_handles: all_soldier_handles.clone(),
        }
    }

    pub fn into_runtime(self) -> AiGlobalState {
        AiGlobalState {
            green_alert_soldiers: self.green_alert_soldiers,
            yellow_alert_soldiers: self.yellow_alert_soldiers,
            red_alert_soldiers: self.red_alert_soldiers,
            soldier_camps: self.soldier_camps,
            stupid_soldiers_cheat: self.stupid_soldiers_cheat,
            freeze: self.freeze,
            overall_alert_status: self.overall_alert_status,
            overall_villain_alert_status: self.overall_villain_alert_status,
            ambush_points: self.ambush_points,
            seek_points: self.seek_points,
            archery_sectors: self.archery_sectors,
            saved_random_seed: self.saved_random_seed,
            remarks_forbidden_till_frame: self.remarks_forbidden_till_frame,
            forbidden_remarks: self.forbidden_remarks,
            screen_remarks: self.screen_remarks,
            attribute_display: self.attribute_display,
            speech_display: self.speech_display,
            golden_eye_mode: self.golden_eye_mode,
            ezekiel_2517: self.ezekiel_2517,
            current_speech_variant: self.current_speech_variant,
            repulsive_points: self.repulsive_points,
            next_repulsive_point_id: self.next_repulsive_point_id,
            door_seek_infos: self.door_seek_infos,
            reinforcement_doors: self.reinforcement_doors,
            houses: self.houses,
            door_rally_points: self.door_rally_points,
            all_soldier_handles: self.all_soldier_handles,
            primary_target_multiplicity_scratch: Default::default(),
            primary_target_multiplicity_initialized: Default::default(),
        }
    }
}

/// Persisted field projection of [`QueuedSelfStimulus`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PersistedQueuedSelfStimulus {
    stimulus_type: StimulusType,
}

impl PersistedQueuedSelfStimulus {
    pub fn capture(runtime: &QueuedSelfStimulus) -> Self {
        let QueuedSelfStimulus {
            stimulus_type,
            origin: _,
        } = runtime;
        Self {
            stimulus_type: *stimulus_type,
        }
    }

    pub fn into_runtime(self) -> QueuedSelfStimulus {
        QueuedSelfStimulus {
            stimulus_type: self.stimulus_type,
            origin: Default::default(),
        }
    }
}

/// Persisted field projection of [`Stimulus`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedStimulus {
    stimulus_type: StimulusType,
    info: StimulusInfo,
    #[serde(
        serialize_with = "serialize_optional_ai_handle",
        deserialize_with = "deserialize_optional_ai_handle"
    )]
    owner: Option<AiEntityHandle>,
    to_whole_patrol: bool,
}

impl PersistedStimulus {
    pub fn capture(runtime: &Stimulus) -> Self {
        let Stimulus {
            stimulus_type,
            info,
            owner,
            to_whole_patrol,
            self_origin: _,
        } = runtime;
        Self {
            stimulus_type: *stimulus_type,
            info: *info,
            owner: *owner,
            to_whole_patrol: *to_whole_patrol,
        }
    }

    pub fn into_runtime(self) -> Stimulus {
        Stimulus {
            stimulus_type: self.stimulus_type,
            info: self.info,
            owner: self.owner,
            to_whole_patrol: self.to_whole_patrol,
            self_origin: Default::default(),
        }
    }
}

/// Persisted field projection of [`AiOutbox`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedAiOutbox {
    patrol: AiPatrolOutbox,
    detection: PersistedAiDetectionOutbox,
    reentrant: PersistedAiReentrantOutbox,
    actor: AiActorOutbox,
    recovery: AiRecoveryOutbox,
    music: AiMusicOutbox,
}

impl PersistedAiOutbox {
    pub fn capture(runtime: &AiOutbox) -> Self {
        let AiOutbox {
            patrol,
            detection,
            reentrant,
            actor,
            recovery,
            music,
        } = runtime;
        Self {
            patrol: patrol.clone(),
            detection: PersistedAiDetectionOutbox::capture(detection),
            reentrant: PersistedAiReentrantOutbox::capture(reentrant),
            actor: actor.clone(),
            recovery: recovery.clone(),
            music: music.clone(),
        }
    }

    pub fn into_runtime(self) -> AiOutbox {
        AiOutbox {
            patrol: self.patrol,
            detection: self.detection.into_runtime(),
            reentrant: self.reentrant.into_runtime(),
            actor: self.actor,
            recovery: self.recovery,
            music: self.music,
        }
    }
}

/// Persisted field projection of [`AiDetectionOutbox`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedAiDetectionOutbox {
    stimuli: Vec<PersistedStimulus>,
    mark_alerted: bool,
}

impl PersistedAiDetectionOutbox {
    pub fn capture(runtime: &AiDetectionOutbox) -> Self {
        let AiDetectionOutbox {
            stimuli,
            mark_alerted,
        } = runtime;
        Self {
            stimuli: stimuli.iter().map(PersistedStimulus::capture).collect(),
            mark_alerted: *mark_alerted,
        }
    }

    pub fn into_runtime(self) -> AiDetectionOutbox {
        AiDetectionOutbox {
            stimuli: self
                .stimuli
                .into_iter()
                .map(|value| value.into_runtime())
                .collect(),
            mark_alerted: self.mark_alerted,
        }
    }
}

/// Persisted field projection of [`AiReentrantOutbox`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedAiReentrantOutbox {
    cross_npc_actions: Vec<CrossNpcAction>,
    self_stimuli: Vec<PersistedQueuedSelfStimulus>,
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

impl PersistedAiReentrantOutbox {
    pub fn capture(runtime: &AiReentrantOutbox) -> Self {
        let AiReentrantOutbox {
            engine_drains_after_script_go_on: _,
            cross_npc_actions,
            self_stimuli,
            finish_macro_after_self_stimuli,
            owner_work,
            reconsider_approach_completion_pending,
            reconsider_approach_replaced_path_waiter,
            battle_observe_completion_pending,
            look_for_help_completion_pending,
            waypoint_script_reach_point,
            alert_soldier_completion_pending,
            dead_body_alert_completion_pending,
            tower_guard_alert_officer_completion_pending,
            civilian_report_alert_officer_completion_pending,
            brawl_hitting_completion_pending,
        } = runtime;
        Self {
            cross_npc_actions: cross_npc_actions.clone(),
            self_stimuli: self_stimuli
                .iter()
                .map(PersistedQueuedSelfStimulus::capture)
                .collect(),
            finish_macro_after_self_stimuli: *finish_macro_after_self_stimuli,
            owner_work: owner_work.clone(),
            reconsider_approach_completion_pending: *reconsider_approach_completion_pending,
            reconsider_approach_replaced_path_waiter: *reconsider_approach_replaced_path_waiter,
            battle_observe_completion_pending: *battle_observe_completion_pending,
            look_for_help_completion_pending: *look_for_help_completion_pending,
            waypoint_script_reach_point: *waypoint_script_reach_point,
            alert_soldier_completion_pending: *alert_soldier_completion_pending,
            dead_body_alert_completion_pending: *dead_body_alert_completion_pending,
            tower_guard_alert_officer_completion_pending:
                *tower_guard_alert_officer_completion_pending,
            civilian_report_alert_officer_completion_pending:
                *civilian_report_alert_officer_completion_pending,
            brawl_hitting_completion_pending: *brawl_hitting_completion_pending,
        }
    }

    pub fn into_runtime(self) -> AiReentrantOutbox {
        AiReentrantOutbox {
            engine_drains_after_script_go_on: Default::default(),
            cross_npc_actions: self.cross_npc_actions,
            self_stimuli: self
                .self_stimuli
                .into_iter()
                .map(|value| value.into_runtime())
                .collect(),
            finish_macro_after_self_stimuli: self.finish_macro_after_self_stimuli,
            owner_work: self.owner_work,
            reconsider_approach_completion_pending: self.reconsider_approach_completion_pending,
            reconsider_approach_replaced_path_waiter: self.reconsider_approach_replaced_path_waiter,
            battle_observe_completion_pending: self.battle_observe_completion_pending,
            look_for_help_completion_pending: self.look_for_help_completion_pending,
            waypoint_script_reach_point: self.waypoint_script_reach_point,
            alert_soldier_completion_pending: self.alert_soldier_completion_pending,
            dead_body_alert_completion_pending: self.dead_body_alert_completion_pending,
            tower_guard_alert_officer_completion_pending: self
                .tower_guard_alert_officer_completion_pending,
            civilian_report_alert_officer_completion_pending: self
                .civilian_report_alert_officer_completion_pending,
            brawl_hitting_completion_pending: self.brawl_hitting_completion_pending,
        }
    }
}

/// Persisted field projection of [`EnemyAi`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedEnemyAi {
    base: PersistedAiController,
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
    last_stimulus_dispatched_to_patrol: Option<PersistedStimulus>,
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

impl PersistedEnemyAi {
    pub fn capture(runtime: &EnemyAi) -> Self {
        let EnemyAi {
            base,
            pending_special_strike,
            pending_sword_strike_consideration,
            pending_combat_insult_after_strike_consideration,
            missed_pc,
            pc_missed,
            pc_gone_away_in_this_direction,
            frame_when_missed_charly,
            heard_nets,
            detected_something_there,
            investigating_distraction,
            last_seek_direction_index,
            beggar_to_examine,
            beggar_is_npc,
            current_task_priority,
            minimal_task_priority,
            new_task_priority,
            number_of_different_checkpoints,
            thirsty,
            position_change_locked_for_test,
            other_bodies_to_examine,
            beggars_to_control,
            positions_of_beggars_to_control,
            seen_dead_body,
            seeking_charly,
            my_seek_points,
            personal_seek_point_1,
            personal_seek_point_2,
            seek_center,
            actual_seek_point,
            seek_point_view_directions,
            seek_flags,
            old_odds,
            gather_position,
            gather_direction,
            gather_position_instructed,
            search_charly_way,
            officers_position,
            previous_state,
            previous_substate,
            reported_to_officer,
            missed_soldier_timer,
            old_money,
            other_seen_money,
            other_seen_ale,
            money_fight_enemies,
            money_fight_victims,
            archer_behind_me,
            shield_bearer_before_me,
            shield_bearer_direction,
            phalanx_aborted,
            changed_to_alert_path,
            already_seen_bodies,
            alerted_us,
            pending_alert_soldier_candidates,
            pending_group_instruction_candidates,
            pending_group_instruction_seek_flags,
            pending_group_instruction_clear_location_after_accept,
            my_shooting_point,
            my_archery_sector,
            my_archery_sector_index,
            my_archery_point_index,
            my_archery_point_increment,
            enemy_seen_below,
            enemy_had_this_elevation,
            known_enemy_strike_1,
            known_enemy_strike_2,
            known_enemy_strike_3,
            return_to_patrol_point,
            fleeing_seen_enemy_counter,
            last_stimulus_dispatched_to_patrol,
            character_id,
            old_life_points,
            initial_life_points,
            list_them,
            ambush_point_array_reset,
            ambush_point_status,
            forced_next_battle_decision,
            reset_battle_decision,
            soldier_profile_iq,
            soldier_profile_courage,
            soldier_profile_shooting,
            soldier_profile_vip,
            soldier_profile_bee_time,
            soldier_profile_pride,
            soldier_profile_hearing_factor,
            soldier_profile_rank,
            soldier_profile_initiative,
            soldier_profile_beer,
            ale_reliable_distraction,
            soldier_profile_money,
            soldier_profile_apple,
            soldier_profile_whistle,
            soldier_profile_duty,
            soldier_profile_endurance,
            is_vip,
            sword_range,
            hth_weapon_id,
            sword_is_charge_weapon,
            next_sword_strike_frame,
            company_number,
            left_combat_neighbour,
            right_combat_neighbour,
            attentive,
            will_be_attentive,
            forced_attentive,
            guarded_pc,
            my_line_jump,
            tower_guard,
            combat_trainer,
            is_archer_unit,
        } = runtime;
        Self {
            base: PersistedAiController::capture(base),
            pending_special_strike: *pending_special_strike,
            pending_sword_strike_consideration: *pending_sword_strike_consideration,
            pending_combat_insult_after_strike_consideration:
                *pending_combat_insult_after_strike_consideration,
            missed_pc: *missed_pc,
            pc_missed: *pc_missed,
            pc_gone_away_in_this_direction: *pc_gone_away_in_this_direction,
            frame_when_missed_charly: *frame_when_missed_charly,
            heard_nets: heard_nets.clone(),
            detected_something_there: *detected_something_there,
            investigating_distraction: *investigating_distraction,
            last_seek_direction_index: *last_seek_direction_index,
            beggar_to_examine: *beggar_to_examine,
            beggar_is_npc: *beggar_is_npc,
            current_task_priority: *current_task_priority,
            minimal_task_priority: *minimal_task_priority,
            new_task_priority: *new_task_priority,
            number_of_different_checkpoints: *number_of_different_checkpoints,
            thirsty: *thirsty,
            position_change_locked_for_test: *position_change_locked_for_test,
            other_bodies_to_examine: other_bodies_to_examine.clone(),
            beggars_to_control: beggars_to_control.clone(),
            positions_of_beggars_to_control: positions_of_beggars_to_control.clone(),
            seen_dead_body: *seen_dead_body,
            seeking_charly: *seeking_charly,
            my_seek_points: my_seek_points.clone(),
            personal_seek_point_1: personal_seek_point_1.clone(),
            personal_seek_point_2: personal_seek_point_2.clone(),
            seek_center: *seek_center,
            actual_seek_point: *actual_seek_point,
            seek_point_view_directions: seek_point_view_directions.clone(),
            seek_flags: *seek_flags,
            old_odds: *old_odds,
            gather_position: *gather_position,
            gather_direction: *gather_direction,
            gather_position_instructed: *gather_position_instructed,
            search_charly_way: search_charly_way.clone(),
            officers_position: *officers_position,
            previous_state: *previous_state,
            previous_substate: *previous_substate,
            reported_to_officer: *reported_to_officer,
            missed_soldier_timer: *missed_soldier_timer,
            old_money: *old_money,
            other_seen_money: other_seen_money.clone(),
            other_seen_ale: other_seen_ale.clone(),
            money_fight_enemies: money_fight_enemies.clone(),
            money_fight_victims: money_fight_victims.clone(),
            archer_behind_me: *archer_behind_me,
            shield_bearer_before_me: *shield_bearer_before_me,
            shield_bearer_direction: *shield_bearer_direction,
            phalanx_aborted: *phalanx_aborted,
            changed_to_alert_path: *changed_to_alert_path,
            already_seen_bodies: already_seen_bodies.clone(),
            alerted_us: alerted_us.clone(),
            pending_alert_soldier_candidates: pending_alert_soldier_candidates.clone(),
            pending_group_instruction_candidates: pending_group_instruction_candidates.clone(),
            pending_group_instruction_seek_flags: *pending_group_instruction_seek_flags,
            pending_group_instruction_clear_location_after_accept:
                *pending_group_instruction_clear_location_after_accept,
            my_shooting_point: *my_shooting_point,
            my_archery_sector: *my_archery_sector,
            my_archery_sector_index: *my_archery_sector_index,
            my_archery_point_index: *my_archery_point_index,
            my_archery_point_increment: *my_archery_point_increment,
            enemy_seen_below: *enemy_seen_below,
            enemy_had_this_elevation: *enemy_had_this_elevation,
            known_enemy_strike_1: *known_enemy_strike_1,
            known_enemy_strike_2: *known_enemy_strike_2,
            known_enemy_strike_3: *known_enemy_strike_3,
            return_to_patrol_point: *return_to_patrol_point,
            fleeing_seen_enemy_counter: *fleeing_seen_enemy_counter,
            last_stimulus_dispatched_to_patrol: last_stimulus_dispatched_to_patrol
                .as_ref()
                .map(PersistedStimulus::capture),
            character_id: *character_id,
            old_life_points: *old_life_points,
            initial_life_points: *initial_life_points,
            list_them: list_them.clone(),
            ambush_point_array_reset: *ambush_point_array_reset,
            ambush_point_status: ambush_point_status.clone(),
            forced_next_battle_decision: *forced_next_battle_decision,
            reset_battle_decision: *reset_battle_decision,
            soldier_profile_iq: *soldier_profile_iq,
            soldier_profile_courage: *soldier_profile_courage,
            soldier_profile_shooting: *soldier_profile_shooting,
            soldier_profile_vip: *soldier_profile_vip,
            soldier_profile_bee_time: *soldier_profile_bee_time,
            soldier_profile_pride: *soldier_profile_pride,
            soldier_profile_hearing_factor: *soldier_profile_hearing_factor,
            soldier_profile_rank: *soldier_profile_rank,
            soldier_profile_initiative: *soldier_profile_initiative,
            soldier_profile_beer: *soldier_profile_beer,
            ale_reliable_distraction: *ale_reliable_distraction,
            soldier_profile_money: *soldier_profile_money,
            soldier_profile_apple: *soldier_profile_apple,
            soldier_profile_whistle: *soldier_profile_whistle,
            soldier_profile_duty: *soldier_profile_duty,
            soldier_profile_endurance: *soldier_profile_endurance,
            is_vip: *is_vip,
            sword_range: *sword_range,
            hth_weapon_id: *hth_weapon_id,
            sword_is_charge_weapon: *sword_is_charge_weapon,
            next_sword_strike_frame: *next_sword_strike_frame,
            company_number: *company_number,
            left_combat_neighbour: *left_combat_neighbour,
            right_combat_neighbour: *right_combat_neighbour,
            attentive: *attentive,
            will_be_attentive: *will_be_attentive,
            forced_attentive: *forced_attentive,
            guarded_pc: *guarded_pc,
            my_line_jump: *my_line_jump,
            tower_guard: *tower_guard,
            combat_trainer: *combat_trainer,
            is_archer_unit: *is_archer_unit,
        }
    }

    pub fn into_runtime(self) -> EnemyAi {
        EnemyAi {
            base: self.base.into_runtime(),
            pending_special_strike: self.pending_special_strike,
            pending_sword_strike_consideration: self.pending_sword_strike_consideration,
            pending_combat_insult_after_strike_consideration: self
                .pending_combat_insult_after_strike_consideration,
            missed_pc: self.missed_pc,
            pc_missed: self.pc_missed,
            pc_gone_away_in_this_direction: self.pc_gone_away_in_this_direction,
            frame_when_missed_charly: self.frame_when_missed_charly,
            heard_nets: self.heard_nets,
            detected_something_there: self.detected_something_there,
            investigating_distraction: self.investigating_distraction,
            last_seek_direction_index: self.last_seek_direction_index,
            beggar_to_examine: self.beggar_to_examine,
            beggar_is_npc: self.beggar_is_npc,
            current_task_priority: self.current_task_priority,
            minimal_task_priority: self.minimal_task_priority,
            new_task_priority: self.new_task_priority,
            number_of_different_checkpoints: self.number_of_different_checkpoints,
            thirsty: self.thirsty,
            position_change_locked_for_test: self.position_change_locked_for_test,
            other_bodies_to_examine: self.other_bodies_to_examine,
            beggars_to_control: self.beggars_to_control,
            positions_of_beggars_to_control: self.positions_of_beggars_to_control,
            seen_dead_body: self.seen_dead_body,
            seeking_charly: self.seeking_charly,
            my_seek_points: self.my_seek_points,
            personal_seek_point_1: self.personal_seek_point_1,
            personal_seek_point_2: self.personal_seek_point_2,
            seek_center: self.seek_center,
            actual_seek_point: self.actual_seek_point,
            seek_point_view_directions: self.seek_point_view_directions,
            seek_flags: self.seek_flags,
            old_odds: self.old_odds,
            gather_position: self.gather_position,
            gather_direction: self.gather_direction,
            gather_position_instructed: self.gather_position_instructed,
            search_charly_way: self.search_charly_way,
            officers_position: self.officers_position,
            previous_state: self.previous_state,
            previous_substate: self.previous_substate,
            reported_to_officer: self.reported_to_officer,
            missed_soldier_timer: self.missed_soldier_timer,
            old_money: self.old_money,
            other_seen_money: self.other_seen_money,
            other_seen_ale: self.other_seen_ale,
            money_fight_enemies: self.money_fight_enemies,
            money_fight_victims: self.money_fight_victims,
            archer_behind_me: self.archer_behind_me,
            shield_bearer_before_me: self.shield_bearer_before_me,
            shield_bearer_direction: self.shield_bearer_direction,
            phalanx_aborted: self.phalanx_aborted,
            changed_to_alert_path: self.changed_to_alert_path,
            already_seen_bodies: self.already_seen_bodies,
            alerted_us: self.alerted_us,
            pending_alert_soldier_candidates: self.pending_alert_soldier_candidates,
            pending_group_instruction_candidates: self.pending_group_instruction_candidates,
            pending_group_instruction_seek_flags: self.pending_group_instruction_seek_flags,
            pending_group_instruction_clear_location_after_accept: self
                .pending_group_instruction_clear_location_after_accept,
            my_shooting_point: self.my_shooting_point,
            my_archery_sector: self.my_archery_sector,
            my_archery_sector_index: self.my_archery_sector_index,
            my_archery_point_index: self.my_archery_point_index,
            my_archery_point_increment: self.my_archery_point_increment,
            enemy_seen_below: self.enemy_seen_below,
            enemy_had_this_elevation: self.enemy_had_this_elevation,
            known_enemy_strike_1: self.known_enemy_strike_1,
            known_enemy_strike_2: self.known_enemy_strike_2,
            known_enemy_strike_3: self.known_enemy_strike_3,
            return_to_patrol_point: self.return_to_patrol_point,
            fleeing_seen_enemy_counter: self.fleeing_seen_enemy_counter,
            last_stimulus_dispatched_to_patrol: self
                .last_stimulus_dispatched_to_patrol
                .map(|value| value.into_runtime()),
            character_id: self.character_id,
            old_life_points: self.old_life_points,
            initial_life_points: self.initial_life_points,
            list_them: self.list_them,
            ambush_point_array_reset: self.ambush_point_array_reset,
            ambush_point_status: self.ambush_point_status,
            forced_next_battle_decision: self.forced_next_battle_decision,
            reset_battle_decision: self.reset_battle_decision,
            soldier_profile_iq: self.soldier_profile_iq,
            soldier_profile_courage: self.soldier_profile_courage,
            soldier_profile_shooting: self.soldier_profile_shooting,
            soldier_profile_vip: self.soldier_profile_vip,
            soldier_profile_bee_time: self.soldier_profile_bee_time,
            soldier_profile_pride: self.soldier_profile_pride,
            soldier_profile_hearing_factor: self.soldier_profile_hearing_factor,
            soldier_profile_rank: self.soldier_profile_rank,
            soldier_profile_initiative: self.soldier_profile_initiative,
            soldier_profile_beer: self.soldier_profile_beer,
            ale_reliable_distraction: self.ale_reliable_distraction,
            soldier_profile_money: self.soldier_profile_money,
            soldier_profile_apple: self.soldier_profile_apple,
            soldier_profile_whistle: self.soldier_profile_whistle,
            soldier_profile_duty: self.soldier_profile_duty,
            soldier_profile_endurance: self.soldier_profile_endurance,
            is_vip: self.is_vip,
            sword_range: self.sword_range,
            hth_weapon_id: self.hth_weapon_id,
            sword_is_charge_weapon: self.sword_is_charge_weapon,
            next_sword_strike_frame: self.next_sword_strike_frame,
            company_number: self.company_number,
            left_combat_neighbour: self.left_combat_neighbour,
            right_combat_neighbour: self.right_combat_neighbour,
            attentive: self.attentive,
            will_be_attentive: self.will_be_attentive,
            forced_attentive: self.forced_attentive,
            guarded_pc: self.guarded_pc,
            my_line_jump: self.my_line_jump,
            tower_guard: self.tower_guard,
            combat_trainer: self.combat_trainer,
            is_archer_unit: self.is_archer_unit,
        }
    }
}

/// Persisted field projection of [`FriendlyAi`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedFriendlyAi {
    base: PersistedAiController,
    beggar_dont_talk_counter: u16,
    fleeing_seen_enemy_counter: u16,
    wants_to_talk: bool,
    last_talk_partner: Option<AiEntityHandle>,
    can_go_away: bool,
}

impl PersistedFriendlyAi {
    pub fn capture(runtime: &FriendlyAi) -> Self {
        let FriendlyAi {
            base,
            beggar_dont_talk_counter,
            fleeing_seen_enemy_counter,
            wants_to_talk,
            last_talk_partner,
            can_go_away,
        } = runtime;
        Self {
            base: PersistedAiController::capture(base),
            beggar_dont_talk_counter: *beggar_dont_talk_counter,
            fleeing_seen_enemy_counter: *fleeing_seen_enemy_counter,
            wants_to_talk: *wants_to_talk,
            last_talk_partner: *last_talk_partner,
            can_go_away: *can_go_away,
        }
    }

    pub fn into_runtime(self) -> FriendlyAi {
        FriendlyAi {
            base: self.base.into_runtime(),
            beggar_dont_talk_counter: self.beggar_dont_talk_counter,
            fleeing_seen_enemy_counter: self.fleeing_seen_enemy_counter,
            wants_to_talk: self.wants_to_talk,
            last_talk_partner: self.last_talk_partner,
            can_go_away: self.can_go_away,
        }
    }
}
