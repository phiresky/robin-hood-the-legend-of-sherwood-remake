use super::*;

#[derive(Debug)]
struct BoredBoundaryDebugConfig {
    gate: crate::engine::diagnostics::ParityGate<1>,
    owner_from: Option<u32>,
    owner_through: Option<u32>,
}

fn bored_boundary_debug_config() -> &'static BoredBoundaryDebugConfig {
    use crate::engine::diagnostics::ParityGate;
    static CONFIG: std::sync::OnceLock<BoredBoundaryDebugConfig> = std::sync::OnceLock::new();
    CONFIG.get_or_init(|| {
        let gate = ParityGate::from_env(
            "PARITY_DEBUG_BORED_BOUNDARY",
            ["PARITY_DEBUG_BORED_BOUNDARY_FRAME"],
        );
        let enabled = gate.enabled();
        let parse = |name: &str| {
            if !enabled {
                return None;
            }
            std::env::var(name).ok().map(|value| {
                value.parse::<u32>().unwrap_or_else(|error| {
                    panic!("invalid {name}={value:?} for BORED_BOUNDARY diagnostic: {error}")
                })
            })
        };
        BoredBoundaryDebugConfig {
            owner_from: parse("PARITY_DEBUG_BORED_BOUNDARY_OWNER_FROM"),
            owner_through: parse("PARITY_DEBUG_BORED_BOUNDARY_OWNER_THROUGH"),
            gate,
        }
    })
}

impl BoredBoundaryDebugConfig {
    fn matches(&self, frame: u32, owner: u32) -> bool {
        self.gate.matches([Some(frame)])
            && self.owner_from.is_none_or(|from| owner >= from)
            && self.owner_through.is_none_or(|through| owner <= through)
    }
}

fn will_stop_debug_config() -> &'static crate::engine::diagnostics::ParityGate<2> {
    use crate::engine::diagnostics::ParityGate;
    static CONFIG: std::sync::OnceLock<ParityGate<2>> = std::sync::OnceLock::new();
    CONFIG.get_or_init(|| {
        ParityGate::from_env(
            "PARITY_DEBUG_WILLSTOP",
            ["PARITY_DEBUG_WILLSTOP_FRAME", "PARITY_DEBUG_WILLSTOP_OWNER"],
        )
    })
}

fn macro_lifecycle_debug_config() -> &'static crate::engine::diagnostics::ParityGate<2> {
    use crate::engine::diagnostics::ParityGate;
    static CONFIG: std::sync::OnceLock<ParityGate<2>> = std::sync::OnceLock::new();
    CONFIG.get_or_init(|| {
        ParityGate::from_env(
            "PARITY_DEBUG_MACRO_LIFECYCLE",
            [
                "PARITY_DEBUG_MACRO_LIFECYCLE_FRAME",
                "PARITY_DEBUG_MACRO_LIFECYCLE_OWNER",
            ],
        )
    })
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum WillStopCaller {
    MacroCompletion,
    ReturnToDuty,
    SimpleWaypoint,
    ProceedOnPath,
    SetPathWalkingFlags,
}

/// The per-NPC AI controller state. Enemy and friendly AI extend this
/// with additional fields.
///
/// Serde persists every field except the `#[serde(skip)]` runtime scratch,
/// which decodes to its default; see [`crate::ai::persisted`].
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct AiController {
    // -- Owner --
    /// The NPC that owns this brain (legacy u32 handle).
    pub me: NpcHandle,

    /// Typed entity ID of the owning NPC.  Set when the AI is attached to
    /// an entity via the element system.  `None` for AI controllers created
    /// before the entity is registered.
    pub owner_entity_id: Option<EntityId>,

    // -- Patrol path IDs --
    //
    // The AI consults these after load when switching to alert routes,
    // so they are gameplay state here.
    pub path_id: Option<PathId>,
    pub alert_path_id: Option<PathId>,

    // -- State --
    pub current_state: AiState,
    pub current_substate: Substate,
    /// Exact original-game storage word sampled before the script update
    /// filter runs. The original AI does not initialize this
    /// member, so a save made before the first decision-tick admission can contain any
    /// 32-bit value. The Original never reads it; retain the bits rather than
    /// inventing a valid `AiState`.
    pub old_state: i32,
    /// Music-side alert level — feeds the per-frame villain-alert
    /// counters and the music-mode pump.
    pub current_music_alert_status: AlertLevel,
    /// View-side alert level — what the cone tint and the
    /// `GetAIAlertStatus` script native read. Alert-status assignment pins this
    /// to YELLOW for soldiers forced attentive whose music
    /// alert just dropped to GREEN. For civilians and non-forced-attentive
    /// soldiers this stays equal to `current_music_alert_status`.
    pub view_alert_status: AlertLevel,
    pub substate_at_last_timer_launch: Substate,
    pub attitude: Attitude,
    pub blood_alcohol: u8,
    /// Initial animation to play when the NPC spawns into the world.
    /// Kept as a raw `u32` ordinal rather than `OrderType` because level
    /// data can carry animation values outside `OrderType`'s covered
    /// range — we map to an `OrderType` at spawn time via
    /// `map_pc_initial_action` and fall back to a warning rather than
    /// rejecting the level.
    pub initial_action: u32,

    pub number_of_looks: u8,

    // -- Patrol path --
    pub has_patrol_path: bool,
    /// Runtime patrol path tracking (wraps a hiking path with current waypoint).
    pub patrol_path: Option<PatrolPath>,
    /// Serialized status retained when `patrol_path` is detached. The Original
    /// preserves this state when detaching the path.
    pub detached_patrol_path_status: DetachedPatrolPathStatus,
    pub can_move: bool,

    pub stop_before_end_of_path: bool,
    pub use_max_norm_to_stop_before_end_of_path: bool,
    pub stop_before_end_of_path_distance: u16,

    // -- Macro system --
    /// Macro bytecode (if any) currently being executed.
    pub macro_command: Vec<u8>,
    pub macro_command_offset: usize,
    /// Which waypoint's authored data block `macro_command` was copied from,
    /// as `(path, waypoint index)`. The Original walks the waypoint block in
    /// place, so its cursor carries that identity implicitly; the copy here
    /// does not, and only this field can tell a cursor that still belongs to
    /// the current waypoint from one the path has since left behind.
    pub macro_command_waypoint: Option<(PathId, u8)>,
    pub number_of_remaining_macro_bytes: u16,
    pub macro_in_progress: bool,
    pub macro_started_in_this_frame: bool,

    // -- Targets & relationships --
    #[serde(with = "optional_ai_handle")]
    pub primary_target: Option<AiEntityHandle>,
    #[serde(with = "optional_ai_handle")]
    pub friend_in_trouble: Option<AiEntityHandle>,
    #[serde(with = "optional_ai_handle")]
    pub detected_body: Option<AiEntityHandle>,
    #[serde(with = "optional_ai_handle")]
    pub interesting_object: Option<AiEntityHandle>,
    #[serde(with = "optional_ai_handle")]
    pub antagonist: Option<AiEntityHandle>,
    // TODO: historically persisted untagged (bare handle), unlike its
    // neighbours; kept for save compatibility.
    pub last_stimulus_actor: Option<AiEntityHandle>,

    // -- Timers --
    pub timer_is_running: bool,
    pub when_does_timer_ring: u32,
    pub macro_timer_is_running: bool,
    pub when_does_macro_timer_ring: u32,
    pub standing_around_timer: u16,

    // -- Sorrow level (0–1000) --
    pub sorrow_level: u16,

    // -- Stimulus history (last 5) --
    pub last_stimulus: [StimulusType; 5],
    pub last_stimulus_multiplicity: [u16; 5],

    // -- Group behaviour --
    pub is_master: bool,
    #[serde(with = "optional_ai_handle")]
    pub master: Option<AiEntityHandle>,

    // -- Seek & alert --
    pub seek_position: Position,
    pub alert_soldiers_point: Position,
    pub first_try: bool,

    // -- Panic --
    pub panic_center_x: f32,
    pub panic_center_y: f32,
    pub lasting_panic_runs: u8,
    pub directed_panic: bool,

    // -- Battle lists --
    /// Our side in the current battle.
    pub list_us: Vec<HumanHandle>,
    /// Alerted allies.
    pub list_alerted_us: Vec<NpcHandle>,
    /// Allies staying put.
    pub list_staying_us: Vec<NpcHandle>,

    // -- Movement failure --
    pub couldnt_reachpoint: bool,
    pub already_on_point: bool,
    pub already_turned: bool,

    // -- Sitting around --
    pub likes_to_sit_around: bool,
    pub special_action: bool,

    /// Remaining servings in the Original tequila-drinking continuation.
    pub remaining_tequila_gulps: u8,

    pub friends_are_alerted: bool,
    pub is_stay_at_home: bool,

    // -- Stimulus queue --
    pub locks_flag_field: AiLockFlags,
    pub was_busy: bool,
    pub stimulus_queue: Vec<Stimulus>,
    pub script_locked: bool,
    pub remember_events: bool,

    // -- House leaving order --
    pub leave_house_number: u16,

    // -- Hint/door/help continuation --
    pub last_hint_actuality: u32,
    pub last_hint_subject: Question,
    /// Canonical Rust runtime door-table index corresponding to Original
    /// its current door through the retained mixed-gate topology.
    pub my_door_index: Option<crate::gate::DoorIndex>,
    pub looking_for_help_because_enemy_seen: bool,

    // -- Objects --
    pub forgotten_objects: Vec<ObjectHandle>,
    #[serde(with = "optional_ai_handle")]
    pub object_of_desire: Option<AiEntityHandle>,

    // -- Charly (friend-check) --
    #[serde(with = "optional_ai_handle")]
    pub checkpoint_charly: Option<AiEntityHandle>,
    #[serde(with = "optional_ai_handle")]
    pub synchronize_charly: Option<AiEntityHandle>,
    /// Synchronization waypoint index for the partner. Lives on
    /// the AI controller because the macro VM's friend-check initialization needs to
    /// write it from `AiController`.
    pub synchronize_index: u16,
    /// Per-look sorrow-level decrement seeded by friend-check initialization
    /// (`delta_sorrow_level = 1000 / number_of_looks`).
    pub delta_sorrow_level: u16,
    /// NPCs the AI has decided are missing/dead and shouldn't be checked
    /// on again. Populated by stimulus handlers (corpse sighting, charly
    /// missing) and read by friend-check initialization to early-resume the
    /// macro.
    pub missed_in_action: Vec<NpcHandle>,
    /// Frame at which this NPC last saw an enemy. Used by
    /// friend-check initialization to suppress redundant checkpoint work for
    /// `NO_CHECK_FOR_AFTER_CHARLY_ALERT_TIME` frames after the alert.
    pub frame_when_enemy_detected: u32,

    pub inside_halt_method: bool,

    // -- Synchronizing actors --
    pub synchronizing_actors: Vec<NpcHandle>,
    pub default_path_walking_flags: GotoFlags,

    // -- Script-forbidden remarks --
    /// Remark IDs (as u32 indices into the Remark enum) that this NPC is
    /// forbidden from saying. Set by the ForbidNPCRemark script native.
    pub forbidden_remark_ids: Vec<u32>,

    // -- View cone --
    pub initial_view_cone: ViewCone,
    pub current_remark: Remark,
    pub current_remark_flags: u16,

    // -- Macro rand --
    pub next_macro_rand: u8,
    pub next_macro_rand_forecasted: bool,

    // -- Emoticon --
    pub current_emoticon_type: EmoticonType,
    pub emoticon_expiration_date: u32,
    pub emoticon_has_expiration_date: bool,

    // -- Reconnaissance report --
    pub my_reconnaissance_report: ReconnaissanceReport,
    pub knocked_out_in_money_fight: bool,
    pub looted_after_money_fight: bool,

    // -- Patrol --
    pub patrol_chief: Option<EntityId>,
    pub patrol: Vec<EntityId>,
    pub missed_patrol_members: Vec<EntityId>,
    pub theoretical_patrol: Vec<EntityId>,
    pub patrol_stopped: bool,
    pub patrol_direction: u16,

    /// One-shot trigger asking `EngineInner::tick_patrol_coordination`
    /// Phase 3 to clear `patrol`/`missed_patrol_members` and rebuild
    /// from `theoretical_patrol` on its next pass. Set by call sites
    /// that explicitly initialize patrols: `init_one_ai`,
    /// `return_to_duty`, the `CMD_PATROL_START` macro opcode, and the
    /// `Substate::DefaultGotoRoute` EVENT_REACHPOINT handler. Cleared by
    /// Phase 3 after the rebuild runs. Without the flag the rebuild gate
    /// was "both lists empty", which would silently re-initialise a
    /// chief whose minions all died/were promoted out — chiefs in that
    /// situation are intentionally kept in their early-return.
    pub needs_patrol_reinit: bool,

    pub got_the_beggar_trick: bool,

    // -- AI log (debug) --
    pub ai_log: Vec<LogLine>,
    /// Debug flag: render this NPC's view cone (toggled by EnableViewCone script).
    pub debug_view_cone_enabled: bool,

    // -- Last goto --
    pub last_goto_destination: Position,
    pub last_goto_flags: GotoFlags,
    pub stuck_counter: u16,

    /// Cached result of script binding; this is controller state rather than
    /// an execution request.
    pub has_script_filter_override: bool,

    // -- Static entity context (set once at init/load) --
    /// Initial position (guard post / spawn point), set at level load.
    pub initial_position: Position,
    /// Initial facing direction (0–15), set at level load.
    pub initial_view_direction: u16,
    /// Maximum visibility across all enemy detectables this frame.
    /// Set by the engine detection tick. Used by `DefaultLookingShadow`
    /// to decide whether to keep watching.
    /// Original-game maximal visibility: the greatest integer sharpness
    /// computed during the current detection refresh.
    pub max_visibility: u32,

    // -- Cached engine state for say() / forbidden remarks --
    /// Current frame counter, set by the engine before think().
    pub cached_frame: u32,
}

impl Default for AiController {
    fn default() -> Self {
        Self {
            me: 0,
            owner_entity_id: None,
            path_id: None,
            alert_path_id: None,
            current_state: AiState::Default,
            current_substate: Substate::DefaultOnPost,
            old_state: AiState::Default as i32,
            current_music_alert_status: AlertLevel::Green,
            view_alert_status: AlertLevel::Green,
            substate_at_last_timer_launch: Substate::DefaultOnPost,
            attitude: Attitude::Suspicious,
            blood_alcohol: 0,
            initial_action: 0,
            number_of_looks: 0,
            has_patrol_path: false,
            patrol_path: None,
            detached_patrol_path_status: DetachedPatrolPathStatus::default(),
            can_move: false,
            stop_before_end_of_path: false,
            use_max_norm_to_stop_before_end_of_path: false,
            stop_before_end_of_path_distance: 0,
            macro_command: Vec::new(),
            macro_command_offset: 0,
            macro_command_waypoint: None,
            number_of_remaining_macro_bytes: 0,
            macro_in_progress: false,
            macro_started_in_this_frame: false,
            primary_target: None,
            friend_in_trouble: None,
            detected_body: None,
            interesting_object: None,
            antagonist: None,
            last_stimulus_actor: None,
            timer_is_running: false,
            when_does_timer_ring: 0,
            macro_timer_is_running: false,
            when_does_macro_timer_ring: 0,
            standing_around_timer: 0,
            sorrow_level: 0,
            last_stimulus: [StimulusType::NoEvent; 5],
            last_stimulus_multiplicity: [1; 5],
            is_master: false,
            master: None,
            seek_position: Position::default(),
            alert_soldiers_point: Position::default(),
            first_try: false,
            panic_center_x: 0.0,
            panic_center_y: 0.0,
            lasting_panic_runs: 0,
            directed_panic: false,
            list_us: Vec::new(),
            list_alerted_us: Vec::new(),
            list_staying_us: Vec::new(),
            couldnt_reachpoint: false,
            already_on_point: false,
            already_turned: false,
            likes_to_sit_around: false,
            special_action: false,
            remaining_tequila_gulps: 0,
            friends_are_alerted: false,
            is_stay_at_home: false,
            locks_flag_field: AiLockFlags::empty(),
            was_busy: false,
            stimulus_queue: Vec::new(),
            script_locked: false,
            remember_events: false,
            leave_house_number: 0,
            last_hint_actuality: 0,
            last_hint_subject: Question::ShallIStayOnMyPost,
            my_door_index: None,
            looking_for_help_because_enemy_seen: false,
            forgotten_objects: Vec::new(),
            object_of_desire: None,
            checkpoint_charly: None,
            synchronize_charly: None,
            synchronize_index: 0,
            delta_sorrow_level: 0,
            missed_in_action: Vec::new(),
            frame_when_enemy_detected: 0,
            inside_halt_method: false,
            synchronizing_actors: Vec::new(),
            default_path_walking_flags: GotoFlags::empty(),
            forbidden_remark_ids: Vec::new(),
            initial_view_cone: ViewCone::Commandoslike,
            current_remark: Remark::TheSoundOfSilence,
            current_remark_flags: 0,
            next_macro_rand: 0,
            next_macro_rand_forecasted: false,
            current_emoticon_type: EmoticonType::None,
            emoticon_expiration_date: 0,
            emoticon_has_expiration_date: false,
            my_reconnaissance_report: ReconnaissanceReport::default(),
            knocked_out_in_money_fight: false,
            looted_after_money_fight: false,
            patrol_chief: None,
            patrol: Vec::new(),
            missed_patrol_members: Vec::new(),
            theoretical_patrol: Vec::new(),
            patrol_stopped: false,
            patrol_direction: 0,
            needs_patrol_reinit: false,
            got_the_beggar_trick: false,
            ai_log: Vec::new(),
            debug_view_cone_enabled: false,
            last_goto_destination: Position::default(),
            last_goto_flags: GotoFlags::empty(),
            stuck_counter: 0,
            has_script_filter_override: false,
            initial_position: Position::default(),
            initial_view_direction: 0,
            max_visibility: 0,
            cached_frame: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// AiController methods (base controller logic)
// ---------------------------------------------------------------------------

impl AiController {
    pub(crate) fn bored_boundary_debug_matches(frame: u32, owner: u32) -> bool {
        bored_boundary_debug_config().matches(frame, owner)
    }

    pub fn new(owner: NpcHandle) -> Self {
        Self {
            me: owner,
            ..Default::default()
        }
    }

    // -- Timer --

    /// Arm the stimulus timer to fire `frames` ticks from now.
    pub fn launch_timer(&mut self, frames: u32, current_frame: u32) {
        self.timer_is_running = true;
        // Launching an AI timer clamps a zero duration
        // timer to one frame before forwarding it to the NPC actor.
        // Macro timers bypass this wrapper and intentionally retain their
        // raw duration in `launch_macro_timer`.
        let frames = frames.max(1);
        self.when_does_timer_ring = current_frame.wrapping_add(frames);
        self.substate_at_last_timer_launch = self.current_substate;
    }

    // -- State transitions --

    pub fn set_ai_state(&mut self, state: AiState) {
        // Diagnostic at trace! level: log the caller path when an NPC
        // transitions out of `Attacking`, which is the class of bug
        // (AI flip-flopping out of combat) we've debugged a few times.
        // Enable with `RUST_LOG=robin_engine::ai=trace`.
        if self.current_state == AiState::Attacking && state != AiState::Attacking {
            tracing::trace!(
                from = ?self.current_state,
                to = ?state,
                substate = ?self.current_substate,
                bt = %std::backtrace::Backtrace::force_capture(),
                "set_ai_state: leaving Attacking"
            );
        }
        self.current_state = state;
    }

    // -- Locks --

    pub fn non_script_lock(&mut self, flags: AiLockFlags) {
        self.locks_flag_field |= flags;
    }

    pub fn non_script_unlock(&mut self, flags: AiLockFlags) {
        self.locks_flag_field -= flags;
    }

    pub fn ai_is_locked(&self) -> bool {
        !self.locks_flag_field.is_empty()
    }

    /// Whether a `FilterAIEvent`-triggered script has claimed the
    /// stimulus queue and the AI must suspend until `ScriptUnlockAI`
    /// fires.
    pub fn ai_is_script_locked(&self) -> bool {
        self.script_locked
    }

    // -- Emoticon --

    pub fn set_emoticon(&mut self, emoticon: EmoticonType) {
        self.current_emoticon_type = emoticon;
        self.emoticon_has_expiration_date = false;
    }

    pub fn set_transient_emoticon(
        &mut self,
        emoticon: EmoticonType,
        frames: u16,
        current_frame: u32,
    ) {
        self.current_emoticon_type = emoticon;
        self.emoticon_has_expiration_date = true;
        self.emoticon_expiration_date = current_frame + frames as u32;
    }

    pub fn clear_emoticon(&mut self) {
        self.set_emoticon(EmoticonType::None);
    }

    // -- Master/group --

    // -- Patrol --

    pub fn has_patrol(&self) -> bool {
        !self.theoretical_patrol.is_empty()
    }

    /// Clear the chief's three patrol lists. Per-minion cleanup
    /// (clearing the patrol chief and forcing return-to-duty for the default state
    /// minions) needs the engine's entity table and runs at the
    /// `RemoveAllSubordinates` native call site.
    pub fn clear_patrol(&mut self) {
        self.theoretical_patrol.clear();
        self.missed_patrol_members.clear();
        self.patrol.clear();
    }

    // -- Stimulus history --

    /// Append a log line stamped with the current universal frame counter.
    /// The list is capped at the 26 most-recent entries.
    pub fn register_log_line(&mut self, line_type: LogLineType, info: u16) {
        self.ai_log.push(LogLine {
            line_type,
            info,
            frame: self.cached_frame,
        });
        while self.ai_log.len() > 26 {
            self.ai_log.remove(0);
        }
    }

    /// Render the per-NPC AI log via `tracing`.
    ///
    /// Each log entry becomes one `trace!` line in the `ai_log` target —
    /// the caller (engine) gates this on `ai_global.attribute_display`
    /// plus the host-side `selected_view_element`.
    ///
    /// Matches the game's AI log strings, including value-to-string
    /// fallback labels for unknown raw log info values.
    pub fn display_log(&self, current_frame: u32) {
        let any_state_change = self
            .ai_log
            .iter()
            .any(|l| l.line_type == LogLineType::ChangeState);

        // When no state-change entry is present, the first on-screen
        // line is the current substate.
        if !any_state_change {
            tracing::trace!(
                target: "ai_log",
                "[{}]",
                self.current_substate
                    .log_string()
                    .unwrap_or("SUBSTATE-???")
            );
        }

        // Quirk preserved verbatim so the line count matches the
        // original overlay: when the substate header is printed the
        // loop skips index 0.
        let start = if any_state_change { 0 } else { 1 };
        let mut last_displayed_speech_frame: u32 = 0;

        for line in self.ai_log.iter().skip(start) {
            match line.line_type {
                LogLineType::Event => {
                    tracing::trace!(
                        target: "ai_log",
                        "Event in frame {}: {}",
                        line.frame,
                        StimulusType::log_string_from_u16(line.info),
                    );
                }
                LogLineType::EventRefused => {
                    tracing::trace!(
                        target: "ai_log",
                        "     refused! Code #{}",
                        line.info,
                    );
                }
                LogLineType::ChangeState => {
                    tracing::trace!(
                        target: "ai_log",
                        "State change: {}",
                        Substate::log_string_from_u16(line.info),
                    );
                }
                LogLineType::BattleDecision => {
                    tracing::trace!(
                        target: "ai_log",
                        "Decision: {}",
                        Decision::log_string_from_u16(line.info),
                    );
                }
                LogLineType::Speak => {
                    last_displayed_speech_frame = line.frame;
                    tracing::trace!(
                        target: "ai_log",
                        "Speak: \"{}\"",
                        Remark::log_string_from_u16(line.info),
                    );
                }
                LogLineType::SpeakImpossible => {
                    tracing::trace!(
                        target: "ai_log",
                        "Speak impossible! Code #{}",
                        line.info,
                    );
                }
                LogLineType::SpeakFinished => {
                    if last_displayed_speech_frame > 0 {
                        tracing::trace!(
                            target: "ai_log",
                            "Speak finished after {} frames",
                            line.frame.saturating_sub(last_displayed_speech_frame),
                        );
                    } else {
                        tracing::trace!(
                            target: "ai_log",
                            "Speak finished after ??? frames",
                        );
                    }
                }
                LogLineType::Timer => {
                    tracing::trace!(
                        target: "ai_log",
                        "Timer launched: {} frames",
                        line.info,
                    );
                }
            }
        }

        // Trailing timer / macro-timer countdowns.
        if self.timer_is_running {
            tracing::trace!(
                target: "ai_log",
                "Timer: {}",
                self.when_does_timer_ring.saturating_sub(current_frame),
            );
        }
        if self.macro_timer_is_running {
            tracing::trace!(
                target: "ai_log",
                "Macro Timer: {}",
                self.when_does_macro_timer_ring.saturating_sub(current_frame),
            );
        }
    }

    // -- Random values --

    /// Random value in the half-open interval `[min, max)` with the
    /// given distribution. `lambda` is the pre-computed consideration
    /// score in `[0, MAX_ATT_VALUE]` — pass `MAX_ATT_VALUE as u8` for
    /// an un-biased sample.
    pub fn random_value(
        sim: &crate::sim_rng::SimulationContext,

        dist: ProbabilityDistribution,
        min_val: i16,
        max_val: i16,
        lambda: u8,
    ) -> i16 {
        debug_assert!(max_val >= min_val);
        let range = max_val - min_val;
        let lambda = lambda as i32;

        // `gauss_curve_top = min + (lambda * range) / MAX_ATT_VALUE`
        let gauss_curve_top = min_val + ((lambda * range as i32) / MAX_ATT_VALUE) as i16;

        match dist {
            ProbabilityDistribution::Dirac => gauss_curve_top,
            ProbabilityDistribution::Rectangle => {
                if range == 0 {
                    return min_val;
                }
                // Half-open `[min, max)` matches the original
                // `rand() % (max-min)` shape.
                min_val
                    + crate::sim_rng::i16(
                        sim,
                        crate::sim_rng::RngSite::AiRandomValueRectangle,
                        0..range,
                    )
            }
            ProbabilityDistribution::GaussHighVariance | ProbabilityDistribution::Gauss => {
                let (sample_scale, center_scale) = match dist {
                    ProbabilityDistribution::GaussHighVariance => (0.333_f32, 0.5_f32),
                    ProbabilityDistribution::Gauss => (0.166_f32, 0.25_f32),
                    _ => unreachable!("Gaussian distribution arm"),
                };
                let width = ((range as f32) * sample_scale) as i16;
                let center = ((range as f32) * center_scale) as i16;
                let mut val = 0_i32;
                if width > 0 {
                    // Keep each draw's site explicit for the source inventory.
                    // Both distributions draw exactly three samples, in order.
                    let sample = || match dist {
                        ProbabilityDistribution::GaussHighVariance => crate::sim_rng::i16(
                            sim,
                            crate::sim_rng::RngSite::AiRandomValueGaussHigh,
                            0..width,
                        ),
                        ProbabilityDistribution::Gauss => crate::sim_rng::i16(
                            sim,
                            crate::sim_rng::RngSite::AiRandomValueGauss,
                            0..width,
                        ),
                        _ => unreachable!("Gaussian distribution arm"),
                    };
                    val = sample() as i32 + sample() as i32 + sample() as i32;
                }
                val += gauss_curve_top as i32 - center as i32;
                val.clamp(min_val as i32, max_val as i32) as i16
            }
        }
    }

    // Decision support and consideration evaluation
    // Modelled as static helpers; original engine used thread-local
    // accumulators.

    /// Interpolate between two values based on a parameter in 0..100.
    ///
    /// Windows retail completes the interpolation in x87 extended
    /// precision before converting the nonnegative result to `u16`.
    /// Promoting the authored `0.01f` constant to `f64` preserves its
    /// exact binary32 value while avoiding intermediate binary32
    /// rounding, which reproduces that instruction sequence.
    pub fn value_between(value_at_0: u16, value_at_100: u16, param: u8) -> u16 {
        debug_assert!(param <= 100);
        let scale = 0.01f32 as f64;
        (value_at_0 as f64 + (value_at_100 as f64 - value_at_0 as f64) * scale * param as f64)
            as u16
    }

    // -- Bored time --

    pub(crate) fn get_bored_time_for(
        &self,
        sim: &crate::sim_rng::SimulationContext,
        frame: u32,
        rank: crate::profiles::ProfileRank,
        pride: u16,
    ) -> u16 {
        // Check the process-local gate before reading any diagnostic-only state.
        let debug = Self::bored_boundary_debug_matches(frame, self.me);
        use crate::profiles::ProfileRank;
        const AI_MIN_DEFAULT_BORED_INTERVAL: u16 = 70;
        const AI_DELTA_DEFAULT_BORED_INTERVAL: u16 = 70;
        const AI_MIN_DEFAULT_BORED_INTERVAL_OFFICER: u16 = 200;
        const AI_DELTA_DEFAULT_BORED_INTERVAL_OFFICER: u16 = 600;
        const AI_MIN_DEFAULT_BORED_INTERVAL_PRIDE: u16 = 400;
        const AI_DELTA_DEFAULT_BORED_INTERVAL_PRIDE: u16 = 800;

        let (min, delta) = if rank == ProfileRank::Officer {
            (
                AI_MIN_DEFAULT_BORED_INTERVAL_OFFICER,
                AI_DELTA_DEFAULT_BORED_INTERVAL_OFFICER,
            )
        } else if pride > 0 {
            (
                AI_MIN_DEFAULT_BORED_INTERVAL_PRIDE,
                AI_DELTA_DEFAULT_BORED_INTERVAL_PRIDE,
            )
        } else {
            (
                AI_MIN_DEFAULT_BORED_INTERVAL,
                AI_DELTA_DEFAULT_BORED_INTERVAL,
            )
        };
        if debug {
            crate::ai::parity_trace::BoredBoundaryGetBoredTime {
                frame: &(frame),
                owner: &(self.me),
                state: &(self.current_state),
                substate: &(self.current_substate),
                rank: &(rank),
                pride: &(pride),
                min: &(min),
                delta: &(delta),
                timer_running: &(self.timer_is_running),
                timer_deadline: &(self.when_does_timer_ring),
            }
            .emit();
        }
        // P_RECTANGLE ignores `lambda`; pass MAX_ATT_VALUE for the un-biased sample.
        min + (Self::random_value(
            sim,
            ProbabilityDistribution::Rectangle,
            0,
            delta as i16,
            MAX_ATT_VALUE as u8,
        ) as u16)
    }

    // -- Retrograde amnesia --

    /// Cancel queued inputs requiring the removed live target. Provenance
    /// (`Stimulus::owner`), perception history, and callback continuations are
    /// not live ownership: keep those for their existing dispatch policies.
    pub(crate) fn remove_entity(&mut self, id: crate::element::EntityId) {
        let keep = |stimulus: &Stimulus| {
            stimulus
                .info
                .live_target()
                .is_none_or(|target| target.get() != id.index())
        };
        self.stimulus_queue.retain(keep);
    }

    // -- Macro diagnostics --

    pub(crate) fn debug_macro_lifecycle_at(
        &self,
        frame: u32,
        original_creation_order: Option<u32>,
        phase: &str,
        reason: impl std::fmt::Debug,
    ) {
        let config = macro_lifecycle_debug_config();
        if !config.matches_required([Some(frame), original_creation_order]) {
            return;
        }
        crate::ai::parity_trace::Macrolife {
            frame: &(frame),
            owner_creation_order: &(original_creation_order),
            me: &(self.me),
            state: &(self.current_state),
            substate: &(self.current_substate),
            in_progress: &(self.macro_in_progress),
            timer_running: &(self.macro_timer_is_running),
            timer_deadline: &(self.when_does_macro_timer_ring),
            started_this_frame: &(self.macro_started_in_this_frame),
            command_offset: &(self.macro_command_offset),
            remaining_bytes: &(self.number_of_remaining_macro_bytes),
            waypoint: &(self.macro_command_waypoint),

            phase: &(phase),
            reason: &(reason),
        }
        .emit();
    }

    /// Pick the closest seek point to flee toward when a panic-run
    /// movement is blocked.
    ///
    /// Walks `seek_points`, computes the maximum norm of the delta from our
    /// current position, adds `1000` for a sector change and `5000`
    /// when a directed panic would end up fleeing *toward* the panic
    /// source, and returns the index of the minimum.
    pub fn nearest_seek_point_to_flee(
        &self,
        seek_points: &[SeekPoint],
        my_pos: Position,
        my_sector: Option<crate::position_interface::SectorHandle>,
    ) -> Option<usize> {
        // The original game initializes a 16-bit minimum to its infinity sentinel and only accepts a
        // strict improvement. A computed distance of 0xffff consequently
        // does not select a candidate.
        let mut best_index = None;
        let mut minimum_distance = u16::MAX;
        for (idx, sp) in seek_points.iter().enumerate() {
            let dx = sp.position.x - my_pos.x;
            let dy = sp.position.y - my_pos.y;
            let mut distance = (dx.abs().max(dy.abs()) as u32) as u16;
            if sp.position.sector.map(|sector| sector.reference())
                != my_sector.map(|sector| sector.reference())
            {
                distance = distance.wrapping_add(1000);
            }
            if self.directed_panic {
                // Big penalty for fleeing toward the panic source:
                // (seek_delta · (panic_center - my_pos)) > 0 means
                // the seek point lies in the same half-plane as the
                // panic source.
                let cx = self.panic_center_x - my_pos.x;
                let cy = self.panic_center_y - my_pos.y;
                if dx * cx + dy * cy > 0.0 {
                    distance = distance.wrapping_add(5000);
                }
            }
            if distance < minimum_distance {
                best_index = Some(idx);
                minimum_distance = distance;
            }
        }
        best_index
    }

    // -- Macro rand --

    /// Random value in `[1, 100]` for macro section-selection.
    /// Consumes the cached forecast if present, otherwise rolls a
    /// fresh value.
    pub fn calculate_macro_rand(&mut self, sim: &crate::sim_rng::SimulationContext) -> u8 {
        if self.next_macro_rand_forecasted {
            self.next_macro_rand_forecasted = false;
            self.next_macro_rand
        } else {
            (crate::sim_rng::u32(sim, crate::sim_rng::RngSite::MacroRand, 0..100) as u8) + 1
        }
    }

    /// Forecast the next return value of `calculate_macro_rand` without
    /// consuming it. Called when one NPC needs to peek at another's
    /// upcoming roll (section-selection coherence).
    pub fn forecast_macro_rand(&mut self, sim: &crate::sim_rng::SimulationContext) -> u8 {
        if !self.next_macro_rand_forecasted {
            self.next_macro_rand =
                (crate::sim_rng::u32(sim, crate::sim_rng::RngSite::MacroRand, 0..100) as u8) + 1;
            self.next_macro_rand_forecasted = true;
        }
        self.next_macro_rand
    }

    // -- Macro timer --

    /// Arm the macro-specific timer. When the timer rings, the engine's
    /// AI hourglass calls [`Self::execute_next_macro_command`] directly
    /// (bypassing the Think state machine).
    pub fn launch_macro_timer(&mut self, frames: u32, current_frame: u32) {
        self.macro_timer_is_running = true;
        // Match the raw integer deadline written by NPC timer launch. A
        // zero-frame macro wait can be consumed by a later macro-timer phase
        // in this same owner update.
        self.when_does_macro_timer_ring = current_frame.wrapping_add(frames);
    }

    // -- Patrol macro helpers --

    /// Detach the live hiking path while preserving the exact status fields
    /// retained when detaching a path. Supplying a new path ID models the
    /// alert-path switch, which resets only the current cursor and direction.
    pub fn detach_patrol_path(
        &mut self,
        hiking_path_index: Option<PathId>,
        reset_for_new_path: bool,
    ) {
        if let Some(path) = self.patrol_path.take() {
            self.detached_patrol_path_status = DetachedPatrolPathStatus {
                hiking_path_index,
                current_waypoint_index: path.current_waypoint_index,
                last_waypoint_index: path.last_waypoint_index,
                forward: path.forward,
                history: path.history,
            };
        } else {
            self.detached_patrol_path_status.hiking_path_index = hiking_path_index;
        }
        if reset_for_new_path {
            self.detached_patrol_path_status.current_waypoint_index = 0;
            self.detached_patrol_path_status.forward = true;
        }
    }

    /// Post-filter half of no-event decision-tick admission. This path deliberately
    /// reads only live owner state: building a global detection/forecast
    /// snapshot here would consume unrelated actors' authoritative RNG.
    pub fn start_no_event_post_filter(
        &mut self,
        static_ai_frozen: bool,
        self_is_dead: bool,
        self_is_unconscious: bool,
    ) -> bool {
        let stimulus = Stimulus::new(StimulusType::NoEvent);

        self.couldnt_reachpoint = false;
        self.already_on_point = false;
        self.already_turned = false;
        if static_ai_frozen {
            self.register_log_line(LogLineType::EventRefused, 1);
            return false;
        }
        if self.script_locked {
            if self.remember_events {
                self.stimulus_queue.push(stimulus);
            }
            self.register_log_line(LogLineType::EventRefused, 2);
            return false;
        }
        if !self.locks_flag_field.is_empty() {
            self.stimulus_queue.push(stimulus);
            self.register_log_line(LogLineType::EventRefused, 3);
            return false;
        }
        if self.current_substate == Substate::WonderingWaspInArmour {
            self.register_log_line(LogLineType::EventRefused, 4);
            return false;
        }
        if self.current_substate == Substate::WonderingUnderNet {
            self.register_log_line(LogLineType::EventRefused, 5);
            return false;
        }
        if self.current_substate == Substate::FleeingMerryManLeaveMap {
            self.register_log_line(LogLineType::EventRefused, 6);
            return false;
        }
        if self_is_unconscious {
            self.register_log_line(LogLineType::EventRefused, 8);
            return false;
        }

        self.standing_around_timer = 0;
        if self.timer_is_running && self.current_substate != self.substate_at_last_timer_launch {
            self.timer_is_running = false;
        }
        if self_is_dead {
            self.register_log_line(LogLineType::EventRefused, 10);
            return false;
        }
        if self.current_substate == Substate::SleepingUnconscious {
            self.register_log_line(LogLineType::EventRefused, 11);
            return false;
        }
        true
    }

    // -- Waypoint-macro launch --

    /// Parse the macro data block attached to a waypoint, roll a
    /// section, and start executing it.
    ///
    /// Layout of `macro_data` (all multi-byte values are little-endian,
    /// offsets relative to byte 0):
    ///
    /// ```text
    /// u16 num_direction_blocks   (1 or 2)
    /// Per direction block:
    ///     u8  direction_flag     (DIR_BOTH=0 / DIR_FORWARD=1 / DIR_BACKWARD=2)
    ///     u16 section_table_offset
    ///
    /// At section_table_offset:
    ///     u16 num_sections
    ///     Per section entry:
    ///         u8  probability_weight      (sums to 100)
    ///         u16 section_data_offset
    ///
    /// At section_data_offset:
    ///     u16 num_macro_bytes
    ///     bytes...                        (the opcode stream)
    /// ```
    ///
    /// Returns `true` if a macro stream was prepared for execution,
    /// `false` if the waypoint should be skipped via
    /// path continuation (no matching direction block, or all
    /// probability weights fell below the roll).
    pub(crate) fn prepare_waypoint_macro(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        macro_data: &[u8],
    ) -> bool {
        tracing::trace!(
            me = self.me,
            macro_len = macro_data.len(),
            path_idx = self
                .patrol_path
                .as_ref()
                .map(|p| p.hiking_path_index.get())
                .unwrap_or(0xFFFF),
            wp_idx = self
                .patrol_path
                .as_ref()
                .map(|p| p.current_waypoint_index)
                .unwrap_or(0xFF),
            "launch_waypoint_macro ENTRY"
        );
        let forward = self.patrol_path.as_ref().map(|p| p.forward).unwrap_or(true);

        // Read u16 LE at `off`, returning None on overflow.
        let read_u16 = |off: usize| -> Option<u16> {
            if off + 2 > macro_data.len() {
                None
            } else {
                Some(u16::from_le_bytes([macro_data[off], macro_data[off + 1]]))
            }
        };
        let read_u8 = |off: usize| -> Option<u8> { macro_data.get(off).copied() };

        let Some(num_dir_blocks) = read_u16(0) else {
            tracing::warn!(
                "NPC {}: malformed waypoint macro — missing num_direction_blocks",
                self.me
            );
            return false;
        };
        if num_dir_blocks == 0 || num_dir_blocks > 2 {
            tracing::warn!(
                "NPC {}: waypoint macro has invalid num_direction_blocks={}",
                self.me,
                num_dir_blocks
            );
            return false;
        }

        // Pick the direction block that matches our traversal direction.
        let direction_matches = |flag: u8| -> bool {
            match flag {
                0 => true,     // DIR_BOTH
                1 => forward,  // DIR_FORWARD
                2 => !forward, // DIR_BACKWARD
                _ => false,
            }
        };

        // Scan the block header triples `(u8 flag, u16 offset)` at
        // offsets 2, 5, ... until we find one whose direction matches.
        let mut section_table_offset: Option<usize> = None;
        for i in 0..num_dir_blocks as usize {
            let hdr_off = 2 + i * 3;
            let Some(flag) = read_u8(hdr_off) else { break };
            let Some(offset) = read_u16(hdr_off + 1) else {
                break;
            };
            if direction_matches(flag) {
                section_table_offset = Some(offset as usize);
                break;
            }
        }

        let Some(section_table_off) = section_table_offset else {
            // No applicable direction block — skip the waypoint.
            return false;
        };

        let Some(num_sections) = read_u16(section_table_off) else {
            tracing::warn!("NPC {}: waypoint macro section table is truncated", self.me);
            return false;
        };
        if num_sections == 0 {
            return false;
        }

        // Roll [1, 100] and walk the probability table.
        let initial_roll = self.calculate_macro_rand(sim);
        let mut roll = initial_roll;
        let mut section_idx: Option<usize> = None;
        let weights: Vec<u8> = (0..num_sections as usize)
            .filter_map(|i| read_u8(section_table_off + 2 + i * 3))
            .collect();
        let first_ops: Vec<u8> = (0..num_sections as usize)
            .filter_map(|i| {
                let entry_off = section_table_off + 2 + i * 3 + 1;
                let data_off = read_u16(entry_off)?;
                macro_data.get(data_off as usize + 2).copied()
            })
            .collect();
        tracing::trace!(
            me = self.me,
            num_sections,
            ?weights,
            ?first_ops,
            initial_roll,
            "launch_waypoint_macro weights"
        );
        for i in 0..num_sections as usize {
            let entry_off = section_table_off + 2 + i * 3;
            let Some(weight) = read_u8(entry_off) else {
                break;
            };
            if roll <= weight {
                section_idx = Some(i);
                break;
            }
            roll -= weight;
        }

        let Some(selected) = section_idx else {
            // Probabilities all under the roll — proceed on path without macro.
            return false;
        };

        // Read the selected section's data offset.
        let data_off_entry = section_table_off + 2 + selected * 3 + 1;
        let Some(section_data_offset) = read_u16(data_off_entry) else {
            return false;
        };
        let section_data_off = section_data_offset as usize;

        let Some(macro_byte_count) = read_u16(section_data_off) else {
            tracing::warn!("NPC {}: waypoint macro section body is truncated", self.me);
            return false;
        };

        tracing::trace!(
            me = self.me,
            section = selected,
            macro_byte_count,
            first_op = macro_data
                .get(section_data_off + 2)
                .copied()
                .unwrap_or(0xff),
            "launch_waypoint_macro picked section"
        );

        // Stash the opcode stream on the AI. We keep a copy of the whole
        // data block so the cursor (`macro_command_offset`) can walk
        // forward into it.
        self.macro_command = macro_data.to_vec();
        self.macro_command_offset = section_data_off + 2;
        self.macro_command_waypoint = self
            .patrol_path
            .as_ref()
            .map(|path| (path.hiking_path_index, path.current_waypoint_index));
        self.number_of_remaining_macro_bytes = macro_byte_count;

        true
    }

    // -- Macro VM --

    /// Apply the tail of Original's completed patrol macro without erasing a
    /// deadline written by a synchronous nested reach-point handler.
    pub(crate) fn finish_patrol_macro(&mut self) {
        self.macro_in_progress = false;
        // The original game's macro-timer cancellation leaves the normal AI
        // timer and the serialized macro deadline both remain untouched.
        self.macro_timer_is_running = false;
    }

    /// Read a u16 LE at the macro PC cursor, advance the cursor by 2,
    /// and decrement `number_of_remaining_macro_bytes` by 2.  Returns
    /// `None` on truncation.  Used by operand-bearing opcodes inside
    /// [`Self::execute_next_macro_command`].
    pub(crate) fn read_macro_u16(&mut self) -> Option<u16> {
        let value = self.peek_macro_u16()?;
        self.macro_command_offset += 2;
        self.number_of_remaining_macro_bytes =
            self.number_of_remaining_macro_bytes.saturating_sub(2);
        Some(value)
    }

    /// Read a u16 LE operand at the macro PC cursor *without* consuming
    /// it.  `CMD_GOTO_POINT` and `CMD_CHANGE_WAY` both dereference their
    /// operand and then leave the cursor and the remaining-byte counter
    /// parked on it — the macro ends immediately afterwards, so the
    /// unconsumed operand stays visible in the dormant cursor.
    pub(crate) fn peek_macro_u16(&self) -> Option<u16> {
        let off = self.macro_command_offset;
        if off + 2 > self.macro_command.len() {
            return None;
        }
        Some(u16::from_le_bytes([
            self.macro_command[off],
            self.macro_command[off + 1],
        ]))
    }

    pub(crate) fn begin_move_request(&mut self, destination: Position, flags: GotoFlags) {
        self.last_goto_destination = destination;
        self.last_goto_flags = flags;
        self.couldnt_reachpoint = false;
    }

    pub(crate) fn prepare_approach(&mut self, distance: i32, flags: GotoFlags, depth: u8) {
        let effective_distance = if depth < 10 {
            distance
        } else {
            (((100 - depth as i32) * distance) / 100).max(0)
        };
        self.stop_before_end_of_path = true;
        self.use_max_norm_to_stop_before_end_of_path = !flags.contains(GotoFlags::USE_NORM);
        self.stop_before_end_of_path_distance = effective_distance as u16;
    }

    pub(crate) fn will_stop_at_next_waypoint_at(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        hiking_paths: &[crate::level_data::RawHikingPath],
        frame: u32,
        original_creation_order: Option<u32>,
        caller: WillStopCaller,
    ) -> bool {
        let config = will_stop_debug_config();
        let debug = config.matches_required([Some(frame), original_creation_order]);
        let before = debug.then(|| {
            let path = self.patrol_path.as_ref();
            let waypoint = path.and_then(|path| path.current_waypoint(hiking_paths));
            (
                self.next_macro_rand_forecasted,
                self.next_macro_rand,
                path.map(|path| (path.hiking_path_index, path.current_waypoint_index)),
                waypoint.map(|waypoint| format!("{:?}", waypoint.command)),
            )
        });
        let result = self.will_stop_at_next_waypoint_inner(sim, hiking_paths);
        if let Some((forecasted_before, value_before, path, waypoint)) = before {
            crate::ai::parity_trace::Willstop {
                frame: &(frame),
                owner: &(original_creation_order),
                forecast_after: &(self.next_macro_rand_forecasted),
                value_after: &(self.next_macro_rand),
                caller: &(caller),
                path: &(path),
                waypoint: &(waypoint),
                forecasted_before: &(forecasted_before),
                value_before: &(value_before),
                result: &(result),
            }
            .emit();
        }
        result
    }

    fn will_stop_at_next_waypoint_inner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        hiking_paths: &[crate::level_data::RawHikingPath],
    ) -> bool {
        use crate::level_data::WaypointCommand;

        // Bytecode belongs to immutable path assets; retain that borrow while
        // advancing the controller's RNG forecast state.
        let (forward, macro_data) = {
            let Some(path) = self.patrol_path.as_ref() else {
                // No path → conservatively report "will stop".
                return true;
            };
            let Some(wp) = path.current_waypoint(hiking_paths) else {
                return true;
            };
            match &wp.command {
                // No data → won't stop.
                WaypointCommand::None => return false,
                // Script may halt → will stop.
                WaypointCommand::Script(_) => return true,
                WaypointCommand::Macro(data) => (path.forward, data.as_slice()),
            }
        };

        let read_u16 = |off: usize| -> Option<u16> {
            if off + 2 > macro_data.len() {
                None
            } else {
                Some(u16::from_le_bytes([macro_data[off], macro_data[off + 1]]))
            }
        };
        let read_u8 = |off: usize| -> Option<u8> { macro_data.get(off).copied() };

        let direction_matches = |flag: u8| -> bool {
            match flag {
                0 => true,     // DIR_BOTH
                1 => forward,  // DIR_FORWARD
                2 => !forward, // DIR_BACKWARD
                _ => false,
            }
        };

        let Some(num_dir_blocks) = read_u16(0) else {
            return false;
        };
        if num_dir_blocks == 0 || num_dir_blocks > 2 {
            return false;
        }

        // Walk the (u8 flag, u16 offset) direction block headers.
        let mut section_table_off: Option<usize> = None;
        for i in 0..num_dir_blocks as usize {
            let hdr_off = 2 + i * 3;
            let Some(flag) = read_u8(hdr_off) else { break };
            let Some(offset) = read_u16(hdr_off + 1) else {
                break;
            };
            if direction_matches(flag) {
                section_table_off = Some(offset as usize);
                break;
            }
        }
        let Some(section_table_off) = section_table_off else {
            return false;
        };

        let Some(num_sections) = read_u16(section_table_off) else {
            return false;
        };
        if num_sections == 0 {
            return false;
        }

        // Peek (don't consume) the next macro-rand for section selection.
        let mut roll = self.forecast_macro_rand(sim);
        let mut section_idx: Option<usize> = None;
        for i in 0..num_sections as usize {
            let entry_off = section_table_off + 2 + i * 3;
            let Some(weight) = read_u8(entry_off) else {
                break;
            };
            if roll <= weight {
                section_idx = Some(i);
                break;
            }
            roll -= weight;
        }
        let Some(selected) = section_idx else {
            return false;
        };

        let data_off_entry = section_table_off + 2 + selected * 3 + 1;
        let Some(section_data_offset) = read_u16(data_off_entry) else {
            return false;
        };
        let section_data_off = section_data_offset as usize;
        let Some(macro_byte_count) = read_u16(section_data_off) else {
            return false;
        };

        // Walk opcodes in the selected section, returning on the first
        // halt-or-flow-through decision. Args of halt opcodes are
        // ignored (we return immediately). Args of motion opcodes are
        // skipped: 0 bytes for RUN/WALK/PATROL_STOP/PATROL_START, 2
        // bytes for PATROL_DIRECTION.
        let mut remaining = macro_byte_count;
        let mut pc = section_data_off + 2;
        while remaining > 0 {
            let Some(op_byte) = read_u8(pc) else {
                return false;
            };
            let Some(op) = MacroOpcode::from_u8(op_byte) else {
                // Unknown opcode: bail out conservatively.
                return false;
            };
            match op {
                MacroOpcode::ReversePath
                | MacroOpcode::SkipPoint
                | MacroOpcode::GotoPoint
                | MacroOpcode::ChangeWay => return false,
                MacroOpcode::Wait
                | MacroOpcode::Check4
                | MacroOpcode::Check4Sync
                | MacroOpcode::FaceTo
                | MacroOpcode::Bend
                | MacroOpcode::StayHere
                | MacroOpcode::LookLeft
                | MacroOpcode::LookRight => return true,
                MacroOpcode::Run
                | MacroOpcode::Walk
                | MacroOpcode::PatrolStop
                | MacroOpcode::PatrolStart => {
                    remaining -= 1;
                    pc += 1;
                }
                MacroOpcode::PatrolDirection => {
                    if remaining < 3 {
                        return false;
                    }
                    remaining -= 3;
                    pc += 3;
                }
            }
        }
        false
    }

    /// Resolve the turn authored by route arrival using the post-callback path.
    /// The live --path/++path pair must retain its endpoint direction change.
    pub(crate) fn route_arrival_turn_direction(
        &mut self,
        position: Position,
        hiking_paths: &[crate::level_data::RawHikingPath],
    ) -> Option<u16> {
        let path = self.patrol_path.as_mut()?;
        let has_command = path.current_waypoint(hiking_paths).is_some_and(|waypoint| {
            !matches!(waypoint.command, crate::level_data::WaypointCommand::None)
        });
        if path.size <= 1 || !has_command {
            return None;
        }
        path.retreat();
        let previous = path.current_waypoint(hiking_paths).map(|wp| (wp.x, wp.y));
        path.advance();
        previous.map(|(x, y)| {
            crate::position_interface::vector_to_sector_0_to_15(
                position.x - x as f32,
                position.y - y as f32,
            ) as u16
        })
    }
}
