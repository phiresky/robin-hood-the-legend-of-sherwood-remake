use super::*;

/// The per-NPC AI controller state. Enemy and friendly AI extend this
/// with additional fields.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
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
    /// State sampled by `StartThink` before the script filter runs.
    /// The original serializes this independently from `current_state`.
    pub old_state: AiState,
    /// Music-side alert level — feeds the per-frame villain-alert
    /// counters and the music-mode pump.
    pub current_music_alert_status: AlertLevel,
    /// View-side alert level — what the cone tint and the
    /// `GetAIAlertStatus` script native read. `SetAlertStatus` pins this
    /// to YELLOW for soldiers with `IsForcedAttentive()` whose music
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
    pub can_move: bool,

    /// Think-method recursion depth — incremented on every `Think(...)`
    /// entry, decremented on exit. Read by `go_near` to shrink the
    /// stop-distance on deep recursion so panic/seek chains don't loop
    /// forever.
    pub think_recursion_depth: u8,

    // -- Macro system --
    /// Macro bytecode (if any) currently being executed.
    pub macro_command: Vec<u8>,
    pub macro_command_offset: usize,
    pub number_of_remaining_macro_bytes: u16,
    pub macro_in_progress: bool,
    pub macro_started_in_this_frame: bool,

    // -- Targets & relationships --
    pub primary_target: HumanHandle,
    pub friend_in_trouble: NpcHandle,
    pub detected_body: HumanHandle,
    pub interesting_object: ObjectHandle,
    pub antagonist: NpcHandle,
    pub last_stimulus_actor: Option<HumanHandle>,

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
    pub master: NpcHandle,

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

    // -- Objects --
    pub forgotten_objects: Vec<ObjectHandle>,
    pub object_of_desire: ObjectHandle,

    // -- Charly (friend-check) --
    pub checkpoint_charly: NpcHandle,
    pub synchronize_charly: NpcHandle,
    /// Synchronization waypoint index for the partner. Lives on
    /// `AiBase` because the macro VM (`InitializeFriendCheck`) needs to
    /// write it from `AiController`.
    pub synchronize_index: u16,
    /// Per-look sorrow-level decrement seeded by `InitializeFriendCheck`
    /// (`delta_sorrow_level = 1000 / number_of_looks`).
    pub delta_sorrow_level: u16,
    /// NPCs the AI has decided are missing/dead and shouldn't be checked
    /// on again. Populated by stimulus handlers (corpse sighting, charly
    /// missing) and read by `InitializeFriendCheck` to early-resume the
    /// macro.
    pub missed_in_action: Vec<NpcHandle>,
    /// Frame at which this NPC last saw an enemy. Used by
    /// `InitializeFriendCheck` to suppress redundant CheckFor work for
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
    /// that explicitly invoke `InitializePatrol()`: `init_one_ai`,
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

    // -- Engine-facing effects, grouped by their drain barrier --
    pub outbox: AiOutbox,

    /// Cached result of script binding; this is controller state rather than
    /// an effect and therefore remains outside the outbox.
    pub has_script_filter_override: bool,

    /// Last primary target reconciled into the entity-side focus state.
    /// This gates automatic focus synchronization across explicit outbox
    /// focus/unfocus effects.
    pub last_synced_focus_target: Option<HumanHandle>,

    // -- Stare target --
    /// If set, the NPC should face toward this actor for `stare_remaining` frames.
    pub stare_target_actor: Option<HumanHandle>,
    /// If set, the NPC should face toward this position for `stare_remaining` frames.
    pub stare_target_position: Option<Position>,
    /// Frames remaining for the stare behaviour. 0 = inactive.
    pub stare_remaining: u32,

    // -- Static entity context (set once at init/load) --
    /// Initial position (guard post / spawn point), set at level load.
    pub initial_position: Position,
    /// Initial facing direction (0–15), set at level load.
    pub initial_view_direction: u16,
    /// Maximum visibility across all enemy detectables this frame.
    /// Set by the engine detection tick. Used by `DefaultLookingShadow`
    /// to decide whether to keep watching.
    /// Original `muwMaximalVisibility`: the greatest integer sharpness
    /// computed during the current detection refresh.
    pub max_visibility: u32,

    // -- Cached engine state for say() / forbidden remarks --
    /// Current frame counter, set by the engine before think().
    pub cached_frame: u32,
    /// Whether this NPC is inside a building, set by the engine.
    pub cached_in_building: bool,
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
            old_state: AiState::Default,
            current_music_alert_status: AlertLevel::Green,
            view_alert_status: AlertLevel::Green,
            substate_at_last_timer_launch: Substate::DefaultOnPost,
            attitude: Attitude::Suspicious,
            blood_alcohol: 0,
            initial_action: 0,
            number_of_looks: 0,
            has_patrol_path: false,
            patrol_path: None,
            can_move: false,
            think_recursion_depth: 0,
            macro_command: Vec::new(),
            macro_command_offset: 0,
            number_of_remaining_macro_bytes: 0,
            macro_in_progress: false,
            macro_started_in_this_frame: false,
            primary_target: 0,
            friend_in_trouble: 0,
            detected_body: 0,
            interesting_object: 0,
            antagonist: 0,
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
            master: 0,
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
            friends_are_alerted: false,
            is_stay_at_home: false,
            locks_flag_field: AiLockFlags::empty(),
            was_busy: false,
            stimulus_queue: Vec::new(),
            script_locked: false,
            remember_events: false,
            leave_house_number: 0,
            forgotten_objects: Vec::new(),
            object_of_desire: 0,
            checkpoint_charly: 0,
            synchronize_charly: 0,
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
            outbox: AiOutbox::default(),
            has_script_filter_override: false,
            last_synced_focus_target: None,
            stare_target_actor: None,
            stare_target_position: None,
            stare_remaining: 0,
            initial_position: Position::default(),
            initial_view_direction: 0,
            max_visibility: 0,
            cached_frame: 0,
            cached_in_building: false,
        }
    }
}

// ---------------------------------------------------------------------------
// AiController methods (base controller logic)
// ---------------------------------------------------------------------------

impl AiController {
    pub fn new(owner: NpcHandle) -> Self {
        Self {
            me: owner,
            ..Default::default()
        }
    }

    // -- Per-NPC init (called from EngineInner::init_one_ai) --

    /// Evaluate `initial_action` and commit the matching AI-side
    /// state transition.
    ///
    /// Returns an [`InitStateSideEffects`] describing the entity-side
    /// mutations the caller (`EngineInner::init_one_ai`) must apply on
    /// NpcData / HumanData / ElementData / ActorData — fields the AI
    /// layer can't reach directly. The AI-side fields
    /// (`current_state` / `current_substate`, timer, emoticon,
    /// `likes_to_sit_around` / `special_action` / `is_stay_at_home`)
    /// are mutated in place before the return.
    ///
    /// The returned `go_to_duty` flag means: `true` — caller should
    /// run the standard "walk onto patrol path or launch a bored timer"
    /// tail; `false` — init placed the NPC in a sleeping / dead /
    /// sitting state and the caller must leave it alone.
    pub fn init_state(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        ctx: &AiContext,
    ) -> InitStateSideEffects {
        use crate::element::{ActionState, EyeStatus, Posture};
        use crate::order::OrderType;

        // Reset the three "I'm authored as X" flags; the matching
        // switch-case below flips the one that applies.
        self.likes_to_sit_around = false;
        self.special_action = false;
        self.is_stay_at_home = false;

        let mut fx = InitStateSideEffects::default();

        // Indoor NPCs stay at home. House membership is already
        // guaranteed because `ai_global.houses` is populated from
        // *every* building sector during
        // `EngineInner::initialize_buildings`, so we just flip the
        // stay-at-home flag + substate here.
        if ctx.in_building {
            self.is_stay_at_home = true;
            self.set_ai_state(AiState::Default);
            self.current_substate = Substate::DefaultHomeSweetHome;
            return fx; // go_to_duty = false
        }

        let raw = self.initial_action;
        match OrderType::try_from(raw).ok() {
            // Plain waiting variants → on-post with a bored timer;
            // return `true` so the caller layers on `ReturnToDuty`.
            Some(
                OrderType::WaitingUpright
                | OrderType::WaitingUprightBored
                | OrderType::WaitingUprightBoredRandom,
            ) => {
                self.set_ai_state(AiState::Default);
                self.current_substate = Substate::DefaultOnPost;
                let bored = self.get_bored_time(sim, ctx);
                self.launch_timer(bored as u32, ctx.frame);
                fx.go_to_duty = true;
            }

            // Sleeping-upright — close eyes, posture Upright +
            // action_state Sleeping, Zzz emoticon.
            Some(OrderType::SleepingUpright) => {
                self.set_ai_state(AiState::Sleeping);
                self.current_substate = Substate::SleepingNapping;
                self.set_emoticon(EmoticonType::Zzz);
                fx.set_eye_status = Some(EyeStatus::Closed);
                fx.set_posture = Some(Posture::Upright);
                fx.set_action_state = Some(ActionState::Sleeping);
                fx.launch_wait = true;
            }

            // Authored sitting. OnPost + bored timer, posture Sitting,
            // `likes_to_sit_around = true` so the return-to-duty branch
            // below picks the sitting placement path.
            Some(OrderType::Sitting) => {
                self.set_ai_state(AiState::Default);
                self.current_substate = Substate::DefaultOnPost;
                let bored = self.get_bored_time(sim, ctx);
                self.launch_timer(bored as u32, ctx.frame);
                self.likes_to_sit_around = true;
                fx.set_posture = Some(Posture::Sitting);
                fx.set_action_state = Some(ActionState::Waiting);
                fx.launch_wait = true;
            }

            // Dead-fallen-back — zero life points, posture DeadBack,
            // killed-by-accident (engine side, bundled with
            // `zero_life_points`).
            Some(OrderType::BeingDeadFallenBack) => {
                self.set_ai_state(AiState::Sleeping);
                self.current_substate = Substate::SleepingForever;
                fx.zero_life_points = true;
                fx.set_posture = Some(Posture::DeadBack);
                fx.set_action_state = Some(ActionState::Waiting);
                fx.launch_wait = true;
            }

            // Dead — same shape but posture Dead.
            Some(OrderType::BeingDead) => {
                self.set_ai_state(AiState::Sleeping);
                self.current_substate = Substate::SleepingForever;
                fx.zero_life_points = true;
                fx.set_posture = Some(Posture::Dead);
                fx.set_action_state = Some(ActionState::Waiting);
                fx.launch_wait = true;
            }

            // Unconscious — max concussion + `unconscious = true`,
            // posture Lying. Init-time has no script-lock / carried /
            // tied gates to honour, so we bypass the full
            // `combat::set_concussion` state machine and write the
            // fields directly on the engine side.
            Some(OrderType::BeingUnconscious) => {
                self.set_ai_state(AiState::Sleeping);
                self.current_substate = Substate::SleepingUnconscious;
                fx.concussion_max_and_unconscious = true;
                fx.set_posture = Some(Posture::Lying);
                fx.set_action_state = Some(ActionState::Waiting);
                fx.launch_wait = true;
            }

            // Special leisure — OnPost, posture Leisure,
            // `special_action = true` so the return-to-duty branch picks
            // the leisure placement path.
            Some(OrderType::Special) => {
                self.set_ai_state(AiState::Default);
                self.current_substate = Substate::DefaultOnPost;
                self.special_action = true;
                fx.set_posture = Some(Posture::Leisure);
                fx.set_action_state = Some(ActionState::Waiting);
                fx.launch_wait = true;
            }

            // Unknown initial action — log a warning and default to
            // on-post.
            _ => {
                tracing::warn!(
                    "NPC {}: InitState received unsupported initial action {} — defaulting to OnPost",
                    self.me,
                    raw,
                );
                self.set_ai_state(AiState::Default);
                self.current_substate = Substate::DefaultOnPost;
                let bored = self.get_bored_time(sim, ctx);
                self.launch_timer(bored as u32, ctx.frame);
                fx.go_to_duty = true;
            }
        }

        fx
    }

    // -- Timer --

    /// Arm the stimulus timer to fire `frames` ticks from now.
    pub fn launch_timer(&mut self, frames: u32, current_frame: u32) {
        // Clamp `frames == 0` to 1 so the timer never rings the same
        // frame it was armed.
        let frames = frames.max(1);
        self.timer_is_running = true;
        self.when_does_timer_ring = current_frame + frames;
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

    /// Script-side AI lock.
    ///
    /// Sets `script_locked` + `remember_events`, halts the NPC's
    /// current engine order (unless the lock itself is the active
    /// command), and drops any running waypoint macro. Callers invoked
    /// from the `LockAi` sequence command handler must pass
    /// `from_lockai_command = true` so the stop doesn't cancel the very
    /// command that triggered the lock; every other site passes
    /// `false`.
    pub fn script_lock(&mut self, remember_events: bool, from_lockai_command: bool) {
        self.script_locked = true;
        self.remember_events = remember_events;
        // C++ AI calls Think(EVENT_RETURN_TO_DUTY) synchronously from
        // AssignPath before the later recorded LockAI can run. In Rust
        // those AI actions are deferred through pending_* queues; once the
        // script lock lands, no pre-lock deferred return-to-duty work may
        // survive and interrupt the scripted sequence that follows.
        self.outbox.actor.orders.clear();
        self.outbox.reentrant.self_stimuli.clear();
        if !from_lockai_command {
            // Cancel the NPC's current order. The engine drains
            // `pending_halt` in post-think.
            self.outbox.actor.halt = true;
        }
        self.break_macro();
    }

    /// Clear the script lock and, unless a `EventAfterScriptGoOn` is
    /// already queued or the NPC is asleep/unconscious, schedule a
    /// `EventReturnToDuty` self-stimulus so the AI re-enters its state
    /// machine immediately. Also latches `pending_blink_all_enemies` so
    /// the next detection pass re-registers anyone still in the view
    /// cone.
    pub fn script_unlock(&mut self, is_unconscious: bool) {
        // Clear current detections so NPCs re-register view-cone
        // occupants on the next detection pass.
        self.outbox.actor.blink_all_enemies = true;

        // Skip the return-to-duty Think if a EVENT_AFTER_SCRIPT_GO_ON
        // is already queued — the script left a waypoint-continuation
        // stimulus that must drain first.
        let after_script_go_on = self
            .stimulus_queue
            .iter()
            .any(|s| s.stimulus_type == StimulusType::EventAfterScriptGoOn);

        self.script_locked = false;

        if self.current_state != AiState::Sleeping && !after_script_go_on && !is_unconscious {
            self.outbox
                .reentrant
                .self_stimuli
                .push(StimulusType::EventReturnToDuty);
        }
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
    /// (`SetPatrolChief(NULL)` + `ForceReturnToDuty` for STATE_DEFAULT
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
    /// Matches the original `DisplayLog` strings, including `*ToString`
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
                    .unwrap_or_else(|| "SUBSTATE-???".to_string())
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
            ProbabilityDistribution::GaussHighVariance => {
                // `range*0.333` truncated (three samples) and
                // `range*0.5` for the centring shift.
                let third = ((range as f32) * 0.333) as i16;
                let half = ((range as f32) * 0.5) as i16;
                let mut val: i32 = 0;
                if third > 0 {
                    val = crate::sim_rng::i16(
                        sim,
                        crate::sim_rng::RngSite::AiRandomValueGaussHigh,
                        0..third,
                    ) as i32
                        + crate::sim_rng::i16(
                            sim,
                            crate::sim_rng::RngSite::AiRandomValueGaussHigh,
                            0..third,
                        ) as i32
                        + crate::sim_rng::i16(
                            sim,
                            crate::sim_rng::RngSite::AiRandomValueGaussHigh,
                            0..third,
                        ) as i32;
                }
                val += gauss_curve_top as i32 - half as i32;
                (val.clamp(min_val as i32, max_val as i32)) as i16
            }
            ProbabilityDistribution::Gauss => {
                // `range*0.166` truncated (three samples) and
                // `range*0.25` for the centring shift.
                let sixth = ((range as f32) * 0.166) as i16;
                let quarter = ((range as f32) * 0.25) as i16;
                let mut val: i32 = 0;
                if sixth > 0 {
                    val = crate::sim_rng::i16(
                        sim,
                        crate::sim_rng::RngSite::AiRandomValueGauss,
                        0..sixth,
                    ) as i32
                        + crate::sim_rng::i16(
                            sim,
                            crate::sim_rng::RngSite::AiRandomValueGauss,
                            0..sixth,
                        ) as i32
                        + crate::sim_rng::i16(
                            sim,
                            crate::sim_rng::RngSite::AiRandomValueGauss,
                            0..sixth,
                        ) as i32;
                }
                val += gauss_curve_top as i32 - quarter as i32;
                (val.clamp(min_val as i32, max_val as i32)) as i16
            }
        }
    }

    // -- Decision support (ConsiderValue / EvaluateConsiderations) --
    // Modelled as static helpers; original engine used thread-local
    // accumulators.

    /// Interpolate between two values based on a parameter in 0..100.
    ///
    /// `0.01f * param` is cast to `u16` (truncation toward zero), so
    /// for `param ∈ [0, 99]` the cast yields `0` and the function
    /// returns `value_at_0`; only `param == 100` yields `1` and returns
    /// `value_at_100`. This is therefore a step function, not a linear
    /// interpolation, despite its name. The truncation is preserved for
    /// bit-for-bit parity with original behaviour.
    pub fn value_between(value_at_0: u16, value_at_100: u16, param: u8) -> u16 {
        debug_assert!(param <= 100);
        let p = (0.01f32 * param as f32) as u16;
        value_at_0.wrapping_add(value_at_100.wrapping_sub(value_at_0).wrapping_mul(p))
    }

    // -- Bored time --

    /// Returns the time until this NPC gets bored and does something.
    /// Officers and high-pride soldiers use longer intervals; everyone
    /// else uses the short default.
    pub fn get_bored_time(&self, sim: &crate::sim_rng::SimulationContext, ctx: &AiContext) -> u16 {
        use crate::profiles::ProfileRank;
        const AI_MIN_DEFAULT_BORED_INTERVAL: u16 = 70;
        const AI_DELTA_DEFAULT_BORED_INTERVAL: u16 = 70;
        const AI_MIN_DEFAULT_BORED_INTERVAL_OFFICER: u16 = 200;
        const AI_DELTA_DEFAULT_BORED_INTERVAL_OFFICER: u16 = 600;
        const AI_MIN_DEFAULT_BORED_INTERVAL_PRIDE: u16 = 400;
        const AI_DELTA_DEFAULT_BORED_INTERVAL_PRIDE: u16 = 800;

        let (min, delta) = if ctx.self_rank == ProfileRank::Officer {
            (
                AI_MIN_DEFAULT_BORED_INTERVAL_OFFICER,
                AI_DELTA_DEFAULT_BORED_INTERVAL_OFFICER,
            )
        } else if ctx.self_pride > 0 {
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

    // -- Pending order access --

    /// Drain all pending orders produced by AI decisions.
    /// Called by the engine each tick to dispatch them.
    pub fn take_pending_orders(&mut self) -> Vec<AiOrderIntent> {
        std::mem::take(&mut self.outbox.actor.orders)
    }

    /// Whether the AI has produced any orders this tick.
    pub fn has_pending_orders(&self) -> bool {
        !self.outbox.actor.orders.is_empty()
    }

    // -- Cross-NPC action access --

    /// Drain all pending cross-NPC actions produced by phalanx logic.
    /// Called by the engine after each think() to dispatch them.
    pub fn take_pending_cross_npc_actions(&mut self) -> Vec<CrossNpcAction> {
        std::mem::take(&mut self.outbox.reentrant.cross_npc_actions)
    }

    /// Drain direct/re-entrant `Think` calls in the exact order the owner
    /// emitted them, leaving genuinely deferred coordination mutations for
    /// the global PA-013 owner-slot batch.
    pub fn take_pending_synchronous_cross_npc_actions(&mut self) -> Vec<CrossNpcAction> {
        let mut synchronous = Vec::new();
        let mut deferred = Vec::with_capacity(self.outbox.reentrant.cross_npc_actions.len());
        for action in self.outbox.reentrant.cross_npc_actions.drain(..) {
            if matches!(
                action,
                CrossNpcAction::SendStimulus { .. }
                    | CrossNpcAction::RequestAlert { .. }
                    | CrossNpcAction::RequestThinkResult { .. }
                    | CrossNpcAction::ReportBackToOfficer { .. }
                    | CrossNpcAction::ConsiderReport { .. }
                    | CrossNpcAction::FinalizeAlertSoldiers { .. }
                    | CrossNpcAction::InstructGatherPosition { .. }
            ) {
                synchronous.push(action);
            } else {
                deferred.push(action);
            }
        }
        self.outbox.reentrant.cross_npc_actions = deferred;
        synchronous
    }

    pub fn has_pending_synchronous_cross_npc_actions(&self) -> bool {
        self.outbox
            .reentrant
            .cross_npc_actions
            .iter()
            .any(|action| {
                matches!(
                    action,
                    CrossNpcAction::SendStimulus { .. }
                        | CrossNpcAction::RequestAlert { .. }
                        | CrossNpcAction::RequestThinkResult { .. }
                        | CrossNpcAction::ReportBackToOfficer { .. }
                        | CrossNpcAction::ConsiderReport { .. }
                        | CrossNpcAction::FinalizeAlertSoldiers { .. }
                        | CrossNpcAction::InstructGatherPosition { .. }
                )
            })
    }

    /// Drain only result-bearing officer reports, leaving ordinary deferred
    /// cross-NPC actions queued for the end-of-frame batch.
    pub fn take_pending_officer_reports(&mut self) -> Vec<CrossNpcAction> {
        let mut reports = Vec::new();
        let mut deferred = Vec::with_capacity(self.outbox.reentrant.cross_npc_actions.len());
        for action in self.outbox.reentrant.cross_npc_actions.drain(..) {
            if matches!(action, CrossNpcAction::ReportBackToOfficer { .. }) {
                reports.push(action);
            } else {
                deferred.push(action);
            }
        }
        self.outbox.reentrant.cross_npc_actions = deferred;
        reports
    }

    /// Drain result-bearing `CALL_ALERT` requests while leaving ordinary
    /// cross-NPC work queued. These calls must close before the sender's
    /// handler continuation, matching direct C++ `Think` re-entry.
    pub fn take_pending_alert_requests(&mut self) -> Vec<CrossNpcAction> {
        let mut requests = Vec::new();
        let mut deferred = Vec::with_capacity(self.outbox.reentrant.cross_npc_actions.len());
        for action in self.outbox.reentrant.cross_npc_actions.drain(..) {
            if matches!(action, CrossNpcAction::RequestAlert { .. }) {
                requests.push(action);
            } else {
                deferred.push(action);
            }
        }
        self.outbox.reentrant.cross_npc_actions = deferred;
        requests
    }

    /// Drain direct recipient `Think` calls while leaving non-stimulus
    /// formation/coordination work for the ordinary cross-NPC batch.
    pub fn take_pending_synchronous_stimuli(&mut self) -> Vec<CrossNpcAction> {
        let mut synchronous = Vec::new();
        let mut deferred = Vec::with_capacity(self.outbox.reentrant.cross_npc_actions.len());
        for action in self.outbox.reentrant.cross_npc_actions.drain(..) {
            if matches!(&action, CrossNpcAction::SendStimulus { .. }) {
                synchronous.push(action);
            } else {
                deferred.push(action);
            }
        }
        self.outbox.reentrant.cross_npc_actions = deferred;
        synchronous
    }

    /// Drain self-directed stimuli queued by `say()`.
    /// The engine re-dispatches these as think() calls to the same NPC.
    pub fn take_pending_self_stimuli(&mut self) -> Vec<StimulusType> {
        std::mem::take(&mut self.outbox.reentrant.self_stimuli)
    }

    // -- Shield commands --

    /// Issue a raise-shield order toward a danger point.
    pub fn raise_shield(&mut self, danger_point: Position) {
        use crate::order::OrderType;
        self.outbox.actor.orders.push(AiOrderIntent::new(
            OrderType::RaisingShield,
            danger_point.x,
            danger_point.y,
        ));
    }

    /// Issue a lower-shield order.
    pub fn lower_shield(&mut self) {
        use crate::order::OrderType;
        self.outbox
            .actor
            .orders
            .push(AiOrderIntent::new(OrderType::LoweringShield, 0.0, 0.0));
    }

    /// Base-class virtual hook for `default_bored_standard_procedure` —
    /// returns `false`.
    ///
    /// Used as a dispatch entry from base-level call sites that want to
    /// give a subclass a chance to react to "I'm bored / nothing left
    /// to do" — currently the macro-end branch of
    /// [`Self::execute_next_macro_command`].
    ///
    /// Civilians/friendlies inherit this no-op. Soldiers override the
    /// hook, but the override gates on `Substate::DefaultOnPost`; the
    /// macro-end call site enters this hook with substate
    /// `DefaultInMacro`, so the gate fails and the soldier override
    /// observably returns false too. Returning false for everyone here
    /// matches both subclasses' behaviour.
    ///
    /// The canonical soldier-side override lives at
    /// `EnemyAi::default_bored_standard_procedure` and is invoked from
    /// the bored-timer expiry path where the substate gate can actually
    /// pass and the EnemyAi-specific `set_state` side effects
    /// (archer/shield-bearer pairing teardown) are required.
    pub fn default_bored_standard_procedure(&mut self, _ctx: &AiContext) -> bool {
        false
    }

    // -- Break macro --

    pub fn break_macro(&mut self) {
        self.macro_in_progress = false;
        self.number_of_remaining_macro_bytes = 0;
        self.macro_command.clear();
        self.macro_command_offset = 0;
        self.macro_timer_is_running = false;
        // `BreakMacro` clears `DETECTABLE_MISSED_FRIEND` and zeros
        // `sorrow_level` as side effects — route through
        // `set_checkpoint_charly` so the detectable queue + sorrow
        // reset stay consistent.
        self.set_checkpoint_charly(0);
    }

    /// Overwrites the stashed checkpoint actor and applies the
    /// detectable/sorrow bookkeeping every call:
    ///
    /// * Unconditionally enqueue `DeleteAllDetectables(MissedFriend)`.
    /// * When `target` is non-zero, enqueue an
    ///   `AddDetectable(target, MissedFriend)` so the target shows up
    ///   in the "missed friend" list.
    /// * When `target` is zero, zero `sorrow_level` and enqueue a
    ///   second delete (belt-and-braces).
    ///
    /// The engine drains `pending_delete_detectables` before
    /// `pending_add_detectables` in [`EngineInner::tick_ai_pending_*`],
    /// so a non-zero call correctly clears the list then re-adds the
    /// target.
    pub fn set_checkpoint_charly(&mut self, target: NpcHandle) {
        use crate::element::DetectableType;
        self.outbox
            .actor
            .delete_detectables
            .push(DetectableType::MissedFriend);
        self.checkpoint_charly = target;
        if target != 0 {
            self.outbox.actor.add_detectables.push((
                crate::element::EntityId::Soldier(crate::entity_id::SoldierId(target)),
                DetectableType::MissedFriend,
            ));
        } else {
            self.sorrow_level = 0;
            self.outbox
                .actor
                .delete_detectables
                .push(DetectableType::MissedFriend);
        }
    }

    /// Merge `other` into `my_reconnaissance_report` and push the
    /// detectable side effects onto the pending queues so the engine
    /// drain runs `DeleteDetectable(body, BODY)` (every newly-merged
    /// body, regardless of `UPDATE_BODIES`) and
    /// `AddDetectable(charly, MISSED_FRIEND)` (when a new charly handle
    /// is adopted under `UPDATE_CHARLY`).
    ///
    /// Flag bits:
    /// * `0x01` — `UPDATE_BODIES`: copy `seen_bodies` from `other`.
    /// * `0x02` — `UPDATE_CHARLY`: copy charly handle if we don't have one.
    /// * `0x04` — `UPDATE_TYPE`: monotonically promote report type and
    ///   seek position via `ReconnaissanceReport::update`.
    ///
    /// The DeleteDetectable side effect fires for every newly-seen body
    /// even when `UPDATE_BODIES` is clear.
    pub fn consider_report_merged(
        &mut self,
        other: &ReconnaissanceReport,
        flags: u16,
        entity_views: &crate::ai_entity_view::AiEntityViewMap,
    ) {
        use crate::element::{DetectableType, EntityId};
        const REPORT_UPDATE_BODIES: u16 = 1;
        const REPORT_UPDATE_CHARLY: u16 = 2;
        const REPORT_UPDATE_TYPE: u16 = 4;

        // Per-body merge + per-body DeleteDetectable.
        for &body in &other.seen_bodies {
            if !self.my_reconnaissance_report.is_body_seen(body) {
                if (flags & REPORT_UPDATE_BODIES) != 0 {
                    self.my_reconnaissance_report.add_seen_body(body);
                }
                // Unconditional `DeleteDetectable(body, BODY)` —
                // fires whether or not UPDATE_BODIES is set.
                let body_id = entity_views
                    .get(&body)
                    .and_then(|view| view.entity_id(body))
                    .unwrap_or_else(|| {
                        panic!("ConsiderReport body {body} has no typed live entity view")
                    });
                self.outbox
                    .actor
                    .delete_detectable_entity
                    .push((body_id, DetectableType::Body));
            }
        }

        // Charly merge + AddDetectable(MISSED_FRIEND).
        if (flags & REPORT_UPDATE_CHARLY) != 0
            && other.charly != 0
            && self.my_reconnaissance_report.charly == 0
        {
            self.my_reconnaissance_report.charly = other.charly;
            self.outbox.actor.add_detectables.push((
                EntityId::Soldier(crate::entity_id::SoldierId(other.charly)),
                DetectableType::MissedFriend,
            ));
        }

        // Monotonically update type / seek_position.
        if (flags & REPORT_UPDATE_TYPE) != 0 {
            self.my_reconnaissance_report
                .update(other.report_type, other.seek_position);
        }
    }

    /// Pick the closest seek point to flee toward when a panic-run
    /// `GoTo` is blocked.
    ///
    /// Walks `seek_points`, computes `MaxNorm` of the delta from our
    /// current position, adds `1000` for a sector change and `5000`
    /// when a directed panic would end up fleeing *toward* the panic
    /// source, and returns the index of the minimum.
    pub fn nearest_seek_point_to_flee(
        &self,
        seek_points: &[SeekPoint],
        my_pos: Position,
        my_sector: Option<crate::position_interface::SectorHandle>,
    ) -> Option<usize> {
        let mut best: Option<(usize, u32)> = None;
        for (idx, sp) in seek_points.iter().enumerate() {
            let dx = sp.position.x - my_pos.x;
            let dy = sp.position.y - my_pos.y;
            let mut distance = dx.abs().max(dy.abs()) as u32;
            if sp.position.sector != my_sector {
                distance = distance.saturating_add(1000);
            }
            if self.directed_panic {
                // Big penalty for fleeing toward the panic source:
                // (seek_delta · (panic_center - my_pos)) > 0 means
                // the seek point lies in the same half-plane as the
                // panic source.
                let cx = self.panic_center_x - my_pos.x;
                let cy = self.panic_center_y - my_pos.y;
                if dx * cx + dy * cy > 0.0 {
                    distance = distance.saturating_add(5000);
                }
            }
            if best.map(|(_, d)| distance < d).unwrap_or(true) {
                best = Some((idx, distance));
            }
        }
        best.map(|(idx, _)| idx)
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
        // Clamp `frames == 0` to 1 so a macro timer never rings the
        // same frame it was armed.
        let frames = frames.max(1);
        self.macro_timer_is_running = true;
        self.when_does_macro_timer_ring = current_frame + frames;
    }

    // -- Patrol macro helpers --

    /// Assign a new patrol path (or drop the current one). The three
    /// call shapes (sentinel `-1`, sentinel `-2`, valid index) collapse
    /// to the cases encoded in [`PatrolAssignment`].
    ///
    /// Side effects:
    /// - `BreakMacro()` prologue unconditionally.
    /// - On clear: snapshot current position/direction into
    ///   `initial_position` / `initial_view_direction` so
    ///   `return_to_duty_common_stuff` sends the NPC back to the
    ///   right anchor.
    /// - Reset `likes_to_sit_around` (per variant), `special_action`,
    ///   `is_stay_at_home` flags.
    /// - Bounds-check index variant against hiking path count; out of
    ///   range returns `false` without touching state.
    /// - When `!script_locked && current_state == Default`, fire a
    ///   self `EventReturnToDuty` so the NPC walks to the new path /
    ///   post on the next tick.
    ///
    /// Callers must supply the NPC's current map position + facing
    /// (0–15) so the initial-pos snapshot is accurate.
    pub fn assign_new_patrol_path(
        &mut self,
        assignment: PatrolAssignment,
        current_position: Position,
        current_direction: u16,
        hiking_paths: &[crate::level_data::RawHikingPath],
    ) -> bool {
        self.break_macro();

        match assignment {
            PatrolAssignment::ClearPath | PatrolAssignment::ClearPathSitAround => {
                let sits = matches!(assignment, PatrolAssignment::ClearPathSitAround);
                self.has_patrol_path = false;
                self.patrol_path = None;
                self.path_id = None;
                self.initial_position = current_position;
                self.initial_view_direction = current_direction & 0x0F;
                self.likes_to_sit_around = sits;
                self.special_action = false;
                self.is_stay_at_home = false;
                if !self.script_locked && self.current_state == AiState::Default {
                    self.fire_self_stimulus(StimulusType::EventReturnToDuty);
                }
                true
            }
            PatrolAssignment::Index(pid) => {
                let idx = pid.get() as usize;
                // Strictly greater, so `idx == count` is tolerated
                // (matches the off-by-one in the original engine).
                if idx > hiking_paths.len() {
                    tracing::warn!(
                        npc = self.me,
                        idx = pid.get(),
                        count = hiking_paths.len(),
                        "AssignNewPatrolPath: index out of range",
                    );
                    return false;
                }
                self.has_patrol_path = true;
                self.path_id = Some(pid);
                self.patrol_path = PatrolPath::new(pid, hiking_paths);
                self.likes_to_sit_around = false;
                self.special_action = false;
                if !self.script_locked && self.current_state == AiState::Default {
                    self.fire_self_stimulus(StimulusType::EventReturnToDuty);
                }
                true
            }
        }
    }

    /// Assign a new guard post.
    ///
    /// Drops any active patrol path, installs the new post as the
    /// NPC's `initial_position` / `initial_view_direction` anchor,
    /// clears the three authored flags, and — when not script-locked
    /// and in the default state — fires `EventReturnToDuty` so the
    /// NPC walks to the new post.
    pub fn assign_new_post(&mut self, post_position: Position, post_direction: u16) -> bool {
        self.break_macro();

        self.path_id = None;
        self.patrol_path = None;
        self.has_patrol_path = false;
        self.initial_position = post_position;
        self.initial_view_direction = post_direction & 0x0F;
        self.is_stay_at_home = false;
        self.likes_to_sit_around = false;
        self.special_action = false;

        if !self.script_locked && self.current_state == AiState::Default {
            self.fire_self_stimulus(StimulusType::EventReturnToDuty);
        }
        true
    }

    /// Script-driven AI state entry. Wires the per-state side effects
    /// that the bare `set_ai_state` field write omits:
    /// `Think(EVENT_RETURN_TO_DUTY)` for `Default`, `SeekArea` via
    /// `pending_script_seek_area` for `Seeking`, and `Panic` via
    /// `pending_begin_panic` for `Fleeing`.
    ///
    /// Unreachable arms (`Sleeping`, `Wondering`, `Menacing`,
    /// `Attacking`) are logged as warnings and skipped.
    pub fn script_set_ai_state(&mut self, state: AiState, current_position: Position) {
        match state {
            AiState::Default => {
                // The native barrier dispatches this through the real Think
                // path before the VM resumes.
                self.fire_self_stimulus(StimulusType::EventReturnToDuty);
            }
            AiState::Seeking => {
                self.outbox.actor.script_seek_area = Some(ScriptSeekAreaRequest {
                    center: current_position,
                    radius: crate::parameters_ai::AI_SCRIPT_SEEK_RADIUS as u16,
                });
            }
            AiState::Fleeing => {
                // Panic(AI_MACRO_PANIC_RUNS) undirected. Panic itself routes
                // through the owner's typed SetState at the engine barrier.
                let runs = crate::parameters_ai::AI_MACRO_PANIC_RUNS as u8;
                let was_already_fleeing = self.current_state == AiState::Fleeing
                    && matches!(
                        self.current_substate,
                        Substate::FleeingPanic | Substate::FleeingRunToDoor
                    );
                self.directed_panic = false;
                self.outbox.actor.begin_panic = Some(PanicRequest {
                    center: None,
                    runs,
                    alert: AlertLevel::Red,
                    is_new_panic: !was_already_fleeing,
                });
            }
            AiState::Sleeping | AiState::Wondering | AiState::Menacing | AiState::Attacking => {
                unreachable!("RHScript::SetAIState rejects {state:?} before AI dispatch")
            }
        }
    }

    /// Post-filter half of `StartThink(NO_EVENT)`. This path deliberately
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

    /// Context-free normal-depth `EndThink`. Returns `false` only for the
    /// original 100.. recursion fallback, whose ReturnToDuty needs a typed
    /// owner context.
    pub fn end_think_completion_events(&mut self) -> bool {
        assert!(
            self.think_recursion_depth > 0,
            "EndThink without StartThink"
        );
        let has_completion =
            self.couldnt_reachpoint || self.already_on_point || self.already_turned;
        if (100..111).contains(&self.think_recursion_depth) && has_completion {
            return false;
        }
        if self.think_recursion_depth >= 100 {
            self.couldnt_reachpoint = false;
            self.already_on_point = false;
            self.already_turned = false;
            self.think_recursion_depth -= 1;
            return true;
        }
        if self.couldnt_reachpoint {
            self.couldnt_reachpoint = false;
            self.outbox
                .reentrant
                .self_stimuli
                .push(StimulusType::EventCouldntReachPoint);
        }
        if self.already_on_point {
            self.already_on_point = false;
            self.outbox
                .reentrant
                .self_stimuli
                .push(StimulusType::EventReachPoint);
        }
        if self.already_turned {
            self.already_turned = false;
            self.outbox
                .reentrant
                .self_stimuli
                .push(StimulusType::EventDone);
        }
        self.think_recursion_depth -= 1;
        true
    }

    /// Broadcast a facing direction to every member of this NPC's patrol
    /// formation.
    ///
    /// The per-minion call iterates `patrol` and writes
    /// `patrol_direction` on each minion; if the minion is in
    /// `DefaultPatrolEnrouteWaiting`, it also calls `FaceTo(direction)`
    /// on the minion. Each minion call needs an `AiContext` which the
    /// chief's `AiController` doesn't have access to, so the directive is
    /// handed to the engine-facing owner drain that runs before the macro
    /// call returns.
    pub fn instruct_patrol_direction_to_patrol_members(&mut self, direction: u16) {
        self.outbox.patrol.direction_broadcast = Some(direction);
    }

    // -- Waypoint-script launch --

    /// Kick off a script-driven waypoint.
    ///
    /// Calls the waypoint's bound script class (`ReachPoint(actor)`)
    /// and, if the script didn't lock the AI into
    /// `Substate::DefaultScriptDriven`, fires `EventAfterScriptGoOn` so
    /// the stimulus queue can drain.
    ///
    /// The per-waypoint VM instance lives on `MissionScript` (keyed by
    /// `(path_idx, wp_idx)`), so we can't dispatch from the AI layer
    /// directly. Instead we record the intent on
    /// `pending_waypoint_script_reach_point`; the engine drains it
    /// right after `think()` returns, calls `ReachPoint(actor)` on the
    /// bound instance, and then fires `EventAfterScriptGoOn` unless the
    /// script put us into `DefaultScriptDriven`.
    pub fn execute_waypoint_script(&mut self, path_idx: PathId, wp_idx: u8) {
        self.outbox.reentrant.waypoint_script_reach_point = Some((path_idx, wp_idx));
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
    /// Returns `true` if a macro was launched (execution proceeded),
    /// `false` if the waypoint should be skipped via
    /// `proceed_on_path` (no matching direction block, or all
    /// probability weights fell below the roll).
    pub fn launch_waypoint_macro(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        macro_data: &[u8],
        ctx: &AiContext,
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
        self.number_of_remaining_macro_bytes = macro_byte_count;

        // Start the macro machine.
        self.set_ai_state(AiState::Default);
        self.current_substate = Substate::DefaultInMacro;
        self.macro_started_in_this_frame = true;
        self.execute_next_macro_command(sim, ctx);
        true
    }

    // -- Macro VM --

    /// Execute waypoint-macro opcodes until one blocks (wait-for-DONE,
    /// wait-for-timer) or the macro ends.
    ///
    /// Several opcodes (`REVERSE_PATH`, `RUN`, `WALK`, `PATROL_*`, ...)
    /// would tail-call back into the VM to consume the next byte. We
    /// flatten that into an explicit `'vm: loop` so the stack doesn't
    /// grow with macro length, and so `&mut self` aliasing is trivial.
    pub fn execute_next_macro_command(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        ctx: &AiContext,
    ) {
        // If we're still in STATE_DEFAULT, make sure the substate
        // reflects that we're inside the VM.
        if self.current_state == AiState::Default {
            self.set_ai_state(AiState::Default);
            self.current_substate = Substate::DefaultInMacro;
        }
        self.standing_around_timer = 0;

        let mut point_already_set = false;
        'vm: loop {
            if (self.number_of_remaining_macro_bytes as i16) > 0 {
                // -- Decode next opcode. -----------------------------
                let opcode_byte = match self.macro_command.get(self.macro_command_offset).copied() {
                    Some(b) => b,
                    None => {
                        tracing::warn!(
                            "NPC {}: macro PC out of bounds at offset {}",
                            self.me,
                            self.macro_command_offset
                        );
                        self.break_macro();
                        return;
                    }
                };
                self.macro_command_offset += 1;
                self.number_of_remaining_macro_bytes -= 1;
                self.macro_in_progress = true;

                let Some(opcode) = MacroOpcode::from_u8(opcode_byte) else {
                    // Unknown opcode: clear remaining bytes and fall
                    // into the "out of bytes" branch.
                    tracing::warn!(
                        "NPC {}: invalid macro opcode 0x{:02x}, breaking macro",
                        self.me,
                        opcode_byte
                    );
                    self.number_of_remaining_macro_bytes = 0;
                    continue 'vm;
                };

                match opcode {
                    MacroOpcode::ReversePath => {
                        if let Some(ref mut path) = self.patrol_path {
                            path.flip_forward_movement();
                        }
                        continue 'vm;
                    }

                    MacroOpcode::SkipPoint => {
                        if let Some(ref mut path) = self.patrol_path {
                            path.advance();
                        }
                        // Last command — return to patrol.
                        self.number_of_remaining_macro_bytes = 0;
                        continue 'vm;
                    }

                    MacroOpcode::GotoPoint => {
                        let Some(index) = self.read_macro_u16() else {
                            self.break_macro();
                            return;
                        };
                        if let Some(ref mut path) = self.patrol_path {
                            // index == current would be a level-designer
                            // bug; log and continue.
                            if path.current_waypoint_index as u16 == index {
                                tracing::warn!(
                                    "NPC {}: CMD_GOTO_POINT → same waypoint {}",
                                    self.me,
                                    index
                                );
                            }
                            path.set_current_index(index as u8);
                        }
                        self.number_of_remaining_macro_bytes = 0;
                        point_already_set = true;
                        continue 'vm;
                    }

                    MacroOpcode::FaceTo => {
                        let Some(direction) = self.read_macro_u16() else {
                            self.break_macro();
                            return;
                        };
                        self.current_substate = Substate::DefaultInMacroWaitingForDone;
                        self.face_direction(direction, ctx);
                        return;
                    }

                    MacroOpcode::Wait => {
                        let Some(frames) = self.read_macro_u16() else {
                            self.break_macro();
                            return;
                        };
                        self.launch_macro_timer(frames as u32, ctx.frame);
                        self.macro_started_in_this_frame = false;
                        return;
                    }

                    MacroOpcode::Check4 => {
                        let Some(friend_id) = self.read_macro_u16() else {
                            self.break_macro();
                            return;
                        };
                        let Some(frames) = self.read_macro_u16() else {
                            self.break_macro();
                            return;
                        };
                        // Civilians/royalists log a warning but still
                        // call InitializeFriendCheck and exit.
                        if !ctx.self_is_soldier {
                            tracing::warn!("NPC {}: CMD_CHECK_4 is illegal for civilians", self.me);
                        }
                        self.initialize_friend_check(sim, friend_id, frames, u16::MAX, ctx);
                        self.macro_started_in_this_frame = false;
                        return;
                    }

                    MacroOpcode::Check4Sync => {
                        let Some(friend_id) = self.read_macro_u16() else {
                            self.break_macro();
                            return;
                        };
                        let Some(frames) = self.read_macro_u16() else {
                            self.break_macro();
                            return;
                        };
                        let Some(index) = self.read_macro_u16() else {
                            self.break_macro();
                            return;
                        };
                        // Log-and-proceed for civilians.
                        if !ctx.self_is_soldier {
                            tracing::warn!(
                                "NPC {}: CMD_CHECK_4_SYNC is illegal for civilians",
                                self.me
                            );
                        }
                        self.initialize_friend_check(sim, friend_id, frames, index, ctx);
                        self.macro_started_in_this_frame = false;
                        return;
                    }

                    MacroOpcode::StayHere => {
                        // CMD_STAY_HERE → AssignNewPatrolPath(ClearPath)
                        // then exit. The helper already handles
                        // BreakMacro + initial-pos snapshot +
                        // EventReturnToDuty dispatch, so just exit
                        // after. (Falling through to the out-of-bytes
                        // branch would re-run path-advance on top of
                        // the reset, which is wrong.)
                        self.assign_new_patrol_path(
                            PatrolAssignment::ClearPath,
                            ctx.position,
                            ctx.direction,
                            &ctx.hiking_paths,
                        );
                        return;
                    }

                    MacroOpcode::ChangeWay => {
                        let Some(index) = self.read_macro_u16() else {
                            self.break_macro();
                            return;
                        };
                        // The helper runs break_macro + bounds check
                        // + gated EventReturnToDuty.  Out-of-range
                        // indices bail without further effect.
                        let assignment = match PathId::new(index) {
                            Some(pid) => PatrolAssignment::Index(pid),
                            None => PatrolAssignment::ClearPath,
                        };
                        self.assign_new_patrol_path(
                            assignment,
                            ctx.position,
                            ctx.direction,
                            &ctx.hiking_paths,
                        );
                        return;
                    }

                    MacroOpcode::Run => {
                        self.default_path_walking_flags |= GotoFlags::RUN;
                        // Sanitise forbidden-civilian flags after the
                        // flag flip — only CMD_RUN/CMD_WALK touch these
                        // flags.
                        if !ctx.self_is_soldier
                            && self
                                .default_path_walking_flags
                                .intersects(GotoFlags::FORBIDDEN_CIVILIANS)
                        {
                            tracing::warn!(
                                me = self.me,
                                "civilian CMD_RUN with forbidden GoTo flags — masking",
                            );
                            self.default_path_walking_flags -= GotoFlags::FORBIDDEN_CIVILIANS;
                        }
                        continue 'vm;
                    }

                    MacroOpcode::Walk => {
                        self.default_path_walking_flags -= GotoFlags::RUN;
                        // Same civilian sanitation as CMD_RUN.
                        if !ctx.self_is_soldier
                            && self
                                .default_path_walking_flags
                                .intersects(GotoFlags::FORBIDDEN_CIVILIANS)
                        {
                            tracing::warn!(
                                me = self.me,
                                "civilian CMD_WALK with forbidden GoTo flags — masking",
                            );
                            self.default_path_walking_flags -= GotoFlags::FORBIDDEN_CIVILIANS;
                        }
                        continue 'vm;
                    }

                    MacroOpcode::LookLeft => {
                        // Log-and-proceed for civilians.
                        if !ctx.self_is_soldier {
                            tracing::warn!(
                                "NPC {}: CMD_LOOK_LEFT is illegal for civilians",
                                self.me
                            );
                        }
                        self.outbox.actor.look_sidewards = Some(LookDirection::Left);
                        self.current_substate = Substate::DefaultInMacroWaitingForDone;
                        self.macro_started_in_this_frame = false;
                        return;
                    }

                    MacroOpcode::LookRight => {
                        // Log-and-proceed for civilians.
                        if !ctx.self_is_soldier {
                            tracing::warn!(
                                "NPC {}: CMD_LOOK_RIGHT is illegal for civilians",
                                self.me
                            );
                        }
                        self.outbox.actor.look_sidewards = Some(LookDirection::Right);
                        self.current_substate = Substate::DefaultInMacroWaitingForDone;
                        self.macro_started_in_this_frame = false;
                        return;
                    }

                    MacroOpcode::Bend => {
                        let Some(frames) = self.read_macro_u16() else {
                            self.break_macro();
                            return;
                        };
                        // Log-and-proceed for civilians.
                        if !ctx.self_is_soldier {
                            tracing::warn!("NPC {}: CMD_BEND is illegal for civilians", self.me);
                        }
                        self.outbox.actor.look_sidewards = Some(LookDirection::Down);
                        self.launch_macro_timer(frames as u32, ctx.frame);
                        self.macro_started_in_this_frame = false;
                        return;
                    }

                    MacroOpcode::PatrolStop => {
                        // Log-and-proceed for civilians.
                        if !ctx.self_is_soldier {
                            tracing::warn!(
                                "NPC {}: CMD_PATROL_STOP is illegal for civilians",
                                self.me
                            );
                        }
                        self.patrol_stopped = true;
                        if ctx.self_rank == crate::profiles::ProfileRank::Officer {
                            self.say(Remark::OfficerStopsPatrol);
                        }
                        continue 'vm;
                    }

                    MacroOpcode::PatrolDirection => {
                        let Some(direction) = self.read_macro_u16() else {
                            self.break_macro();
                            return;
                        };
                        // Log-and-proceed for civilians.
                        if !ctx.self_is_soldier {
                            tracing::warn!(
                                "NPC {}: CMD_PATROL_DIRECTION is illegal for civilians",
                                self.me
                            );
                        }
                        self.instruct_patrol_direction_to_patrol_members(direction);
                        continue 'vm;
                    }

                    MacroOpcode::PatrolStart => {
                        // Log-and-proceed for civilians.
                        if !ctx.self_is_soldier {
                            tracing::warn!(
                                "NPC {}: CMD_PATROL_START is illegal for civilians",
                                self.me
                            );
                        }
                        self.patrol_stopped = false;
                        if ctx.self_rank == crate::profiles::ProfileRank::Officer {
                            self.say(Remark::OfficerStartsPatrol);
                        }
                        // Also calls `InitializePatrol()` here. Raise
                        // the one-shot flag so
                        // `tick_patrol_coordination` Phase 3 clears +
                        // rebuilds the minion list on its next pass;
                        // the local `patrol.clear()` keeps the current
                        // frame's coordinate dispatch from referencing
                        // a stale list before the rebuild.
                        self.patrol.clear();
                        self.needs_patrol_reinit = true;
                        continue 'vm;
                    }
                }
            } else {
                // -- Out of macro bytes: path-advance branch. -------

                // Virtual hook for subclass overrides on macro
                // completion. Both subclasses' overrides gate on
                // `DefaultOnPost`, which the macro-end branch can't
                // enter (substate is `DefaultInMacro` here), so the
                // call observably returns false today. The hook is
                // wired anyway so a future override that doesn't share
                // that gate will be invoked from this site.
                if self.default_bored_standard_procedure(ctx) {
                    self.break_macro();
                    return;
                }

                let path_size = self.patrol_path.as_ref().map(|p| p.size).unwrap_or(0);

                if path_size == 1 {
                    if self.macro_started_in_this_frame {
                        // One-point path + started this frame → hold
                        // position via the *macro* timer, not the
                        // regular bored timer. The wake must come via
                        // `launch_macro_timer`'s `bMacroTimer = true`
                        // path so it's delivered by `ProceedMacro` as a
                        // direct `execute_next_macro_command(sim, )` call —
                        // bypassing Think entirely. A regular
                        // `launch_timer` here would fire an EventTimer
                        // that `DefaultInMacro` never handles
                        // (`DefaultInMacro` only receives EventDone),
                        // so the NPC would hang until the next
                        // reach-point event nudges it.
                        self.current_substate = Substate::DefaultInMacro;
                        self.macro_started_in_this_frame = false;
                        self.launch_macro_timer(
                            crate::parameters_ai::AI_ONE_POINT_DEFAULT_TIME as u32,
                            ctx.frame,
                        );
                    } else {
                        // Already here → synthesize a REACH_POINT event so
                        // the stimulus queue picks up the next waypoint.
                        self.macro_in_progress = false;
                        self.timer_is_running = false;
                        self.current_substate = Substate::DefaultEnroute;
                        self.fire_self_stimulus(StimulusType::EventReachPoint);
                    }
                } else {
                    if !point_already_set && let Some(ref mut path) = self.patrol_path {
                        path.advance();
                    }

                    let hiking_paths = &ctx.hiking_paths;
                    self.set_ai_state(AiState::Default);
                    self.current_substate = Substate::DefaultEnroute;
                    let will_stop = self.will_stop_at_next_waypoint(sim, hiking_paths);
                    let mut walk_flags = self.default_path_walking_flags;
                    if !will_stop {
                        walk_flags |= GotoFlags::DONT_STOP;
                    }
                    if let Some(next_wp) = self
                        .patrol_path
                        .as_ref()
                        .and_then(|p| p.current_waypoint(hiking_paths))
                        .map(|wp| Position {
                            x: wp.x as f32,
                            y: wp.y as f32,
                            sector: SectorHandle::new(wp.sector),
                            level: wp.level,
                        })
                    {
                        self.go_to(next_wp, walk_flags, ctx);
                    } else {
                        self.return_to_duty_common_stuff(sim, DutyFlags::empty(), ctx);
                    }
                    self.macro_in_progress = false;
                    self.timer_is_running = false;
                }
                return;
            }
        }
    }

    /// Read a u16 LE at the macro PC cursor, advance the cursor by 2,
    /// and decrement `number_of_remaining_macro_bytes` by 2.  Returns
    /// `None` on truncation.  Used by operand-bearing opcodes inside
    /// [`Self::execute_next_macro_command`].
    fn read_macro_u16(&mut self) -> Option<u16> {
        let off = self.macro_command_offset;
        if off + 2 > self.macro_command.len() {
            return None;
        }
        let value = u16::from_le_bytes([self.macro_command[off], self.macro_command[off + 1]]);
        self.macro_command_offset += 2;
        self.number_of_remaining_macro_bytes =
            self.number_of_remaining_macro_bytes.saturating_sub(2);
        Some(value)
    }

    // -- Friend check (CheckFor comportment) --

    /// Start the "CheckFor" comportment against another NPC — the
    /// direct target of CMD_CHECK_4 / CMD_CHECK_4_SYNC from the macro
    /// VM.
    ///
    /// Steps:
    ///  (a) bounds-check `friend_id` against the all-soldier count,
    ///      assert NPC, store on `checkpoint_charly`, assert not self.
    ///  (b) early-resume the macro if the partner is already known dead
    ///      / missing (`missed_in_action`) or we recently saw an enemy
    ///      (`NO_CHECK_FOR_AFTER_CHARLY_ALERT_TIME` cooldown).
    ///  (c) **pure-synchronization branch** when `frames==0 && index!=
    ///      u16::MAX`: compare partner's current/last waypoint index
    ///      and forward-movement direction against ours and either
    ///      resume the macro or queue a `RegisterSynchronizingActor`
    ///      and switch to `Substate::DefaultSynchronizing`.
    ///  (d) waypoint / post visibility check: scan the partner's patrol
    ///      waypoints (or fall back to its initial post) via
    ///      [`AiContext::is_detecting_point_360`]. If nothing is
    ///      visible, log + resume the macro.
    ///  (e) optionally seed `synchronize_charly` + `synchronize_index`
    ///      so the wait-loop can synchronise once the friend arrives.
    ///  (f) configure the look-around wait: `number_of_looks =
    ///      frames / AI_CHECKFOR_TIME_INTERVAL + 1`,
    ///      `delta_sorrow_level = 1000 / number_of_looks`, transition
    ///      to `DefaultLookingSidewardsForCharly`, and queue
    ///      `pending_look_sidewards` with a random `LeftRight` /
    ///      `RightLeft` direction.
    pub fn initialize_friend_check(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        friend_id: u16,
        frames: u16,
        index: u16,
        ctx: &AiContext,
    ) {
        // (a) Resolve friend_id → handle. Degrade to a warn +
        // early-resume on out-of-range or non-NPC, since panicking
        // would crash the engine on a malformed mission script.
        let number_of_all = ctx.number_of_all_soldiers();
        if friend_id >= number_of_all {
            tracing::warn!(
                "NPC {}: CheckFor at ({:.0}, {:.0}): friend_id {} out of range (max {})",
                self.me,
                ctx.position.x,
                ctx.position.y,
                friend_id,
                number_of_all
            );
            self.set_checkpoint_charly(0);
            self.current_substate = Substate::DefaultInMacro;
            self.execute_next_macro_command(sim, ctx);
            return;
        }
        let target = match ctx.all_soldier_handle(friend_id) {
            Some(h) if h != 0 => h,
            _ => {
                tracing::warn!(
                    "NPC {}: CheckFor at ({:.0}, {:.0}): friend_id {} resolves to no live actor",
                    self.me,
                    ctx.position.x,
                    ctx.position.y,
                    friend_id
                );
                self.set_checkpoint_charly(0);
                self.current_substate = Substate::DefaultInMacro;
                self.execute_next_macro_command(sim, ctx);
                return;
            }
        };
        // Bail with a warn (instead of panicking) on level-data
        // issues if the resolved actor isn't an NPC.
        let target_view = match ctx.entity_view(target) {
            Some(v)
                if matches!(
                    v.kind,
                    crate::ai_entity_view::EntityKind::Soldier
                        | crate::ai_entity_view::EntityKind::Civilian
                ) =>
            {
                v.clone()
            }
            _ => {
                tracing::warn!(
                    "NPC {}: CheckFor friend_id {} → handle {} is not an NPC",
                    self.me,
                    friend_id,
                    target
                );
                self.set_checkpoint_charly(0);
                self.current_substate = Substate::DefaultInMacro;
                self.execute_next_macro_command(sim, ctx);
                return;
            }
        };
        // Store + warn if not self.
        self.set_checkpoint_charly(target);
        if target == self.me {
            tracing::warn!(
                "NPC {}: CheckFor at ({:.0}, {:.0}) applied on yourself? Funny idea...",
                self.me,
                ctx.position.x,
                ctx.position.y
            );
        }

        // (b1) friend already on the missed list → skip the check,
        // resume the macro.
        if self.missed_in_action.contains(&target) {
            self.set_checkpoint_charly(0);
            self.current_substate = Substate::DefaultInMacro;
            self.execute_next_macro_command(sim, ctx);
            return;
        }

        // (b2) recently saw an enemy → no-op.
        if self.frame_when_enemy_detected > 0
            && ctx.frame.wrapping_sub(self.frame_when_enemy_detected)
                < crate::parameters_ai::NO_CHECK_FOR_AFTER_CHARLY_ALERT_TIME
        {
            self.set_checkpoint_charly(0);
            self.current_substate = Substate::DefaultInMacro;
            self.execute_next_macro_command(sim, ctx);
            return;
        }

        // Self path direction / current waypoint — read once.
        // Forward-movement defaults to true when the path is
        // uninitialised; matches `PatrolPath::forward`.
        let my_forward = self.patrol_path.as_ref().map(|p| p.forward).unwrap_or(true);
        let my_current_wp_index = self
            .patrol_path
            .as_ref()
            .map(|p| p.current_waypoint_index as u16)
            .unwrap_or(0);

        // (c) Pure synchronization branch.
        if frames == 0 && index != u16::MAX {
            let synchronize_index = if index > 500 {
                // Relative index: my current waypoint + (index - 1000).
                // The original math is unsigned wrap-friendly; we use
                // i32 arithmetic and clamp the cast.
                let rel = (index as i32) - 1000;
                ((my_current_wp_index as i32) + rel).max(0) as u16
            } else {
                index
            };
            self.synchronize_charly = target;
            self.synchronize_index = synchronize_index;
            self.set_checkpoint_charly(0);
            debug_assert!(
                self.macro_in_progress,
                "InitializeFriendCheck pure-sync branch requires a macro to be in progress"
            );

            let target_alive_in_default =
                target_view.ai_state == AiState::Default && !target_view.is_dead;

            let friend_is_already_there = if target_alive_in_default {
                if target_view.macro_in_progress {
                    // Standing at the right waypoint?
                    if index < 500 {
                        target_view.path_current_waypoint_index as u16 == synchronize_index
                    } else if target_view.path_forward_movement != my_forward {
                        // backwards guy waits — only the forward leg proceeds
                        my_forward
                    } else if my_forward {
                        target_view.path_current_waypoint_index as u16 >= synchronize_index
                    } else {
                        target_view.path_current_waypoint_index as u16 <= synchronize_index
                    }
                } else if target_view.ai_substate == Substate::DefaultEnroute {
                    // Last waypoint was the right one?
                    if index < 500 {
                        target_view.path_last_waypoint_index as u16 == synchronize_index
                    } else if target_view.path_forward_movement != my_forward {
                        my_forward
                    } else if my_forward {
                        target_view.path_last_waypoint_index as u16 >= synchronize_index
                    } else {
                        target_view.path_last_waypoint_index as u16 <= synchronize_index
                    }
                } else {
                    false
                }
            } else {
                // Friend not in STATE_DEFAULT or dead → forget it.
                self.current_substate = Substate::DefaultInMacro;
                self.execute_next_macro_command(sim, ctx);
                return;
            };

            if friend_is_already_there {
                self.current_substate = Substate::DefaultInMacro;
                self.execute_next_macro_command(sim, ctx);
            } else {
                // Not yet there — wait, register us.
                self.outbox.reentrant.cross_npc_actions.push(
                    CrossNpcAction::RegisterSynchronizingActor {
                        target,
                        actor: self.me,
                    },
                );
                self.current_substate = Substate::DefaultSynchronizing;
            }
            return;
        }

        // (d) Visibility check.
        if !target_view.has_patrol_path {
            // Post-only friend. Try the post, then post + 15 Z; if
            // neither is visible, warn and continue into the wait
            // setup anyway.
            let post = crate::coordinates::WorldPoint3D {
                x: target_view.initial_position.x,
                y: target_view.initial_position.y,
                z: target_view.elevation,
            };
            if !ctx.is_detecting_point_360(post) {
                let mut elevated = post;
                elevated.z += 15.0;
                if !ctx.is_detecting_point_360(elevated) {
                    tracing::warn!(
                        "NPC {}: CheckFor at ({:.0}, {:.0}): partner's post at ({:.0}, {:.0}) not visible",
                        self.me,
                        ctx.position.x,
                        ctx.position.y,
                        target_view.initial_position.x,
                        target_view.initial_position.y
                    );
                }
            }
            if index != u16::MAX {
                tracing::warn!(
                    "NPC {}: CheckForSynch at ({:.0}, {:.0}): can't synchronise with a partner that has no path",
                    self.me,
                    ctx.position.x,
                    ctx.position.y
                );
            }
        } else {
            // Scan the partner's patrol waypoints for at least one
            // that we can see.
            let hiking_paths = &ctx.hiking_paths;
            let mut visible_point_found = false;
            if let Some(path_id) = target_view.patrol_hiking_path_index
                && let Some(raw_path) = hiking_paths.get(path_id.get() as usize)
            {
                for wp in raw_path.waypoints.iter() {
                    let mut pt = ctx.position_to_point_3d(Position {
                        x: wp.x as f32,
                        y: wp.y as f32,
                        sector: SectorHandle::new(wp.sector),
                        level: wp.level,
                    });
                    pt.z += 15.0;
                    if ctx.is_detecting_point_360(pt) {
                        visible_point_found = true;
                        break;
                    }
                }
            }
            if !visible_point_found {
                // No waypoint visible → log + resume macro.
                tracing::trace!(
                    "NPC {}: CheckFor at ({:.0}, {:.0}): no waypoint of partner's path is visible",
                    self.me,
                    ctx.position.x,
                    ctx.position.y
                );
                self.current_substate = Substate::DefaultInMacro;
                self.execute_next_macro_command(sim, ctx);
                return;
            }
        }
        // (e) Maybe prepare for later sync.
        if index == u16::MAX {
            self.synchronize_charly = 0;
            self.synchronize_index = u16::MAX;
        } else {
            self.synchronize_charly = target;
            self.synchronize_index = if index > 500 {
                let rel = (index as i32) - 1000;
                ((my_current_wp_index as i32) + rel).max(0) as u16
            } else {
                index
            };
        }

        // (f) Begin to wait.
        let interval = crate::parameters_ai::AI_CHECKFOR_TIME_INTERVAL.max(1) as u16;
        self.number_of_looks = ((frames / interval) + 1).min(u8::MAX as u16) as u8;
        let looks_for_div = self.number_of_looks.max(1) as u16;
        self.delta_sorrow_level = 1000 / looks_for_div;
        self.current_substate = Substate::DefaultLookingSidewardsForCharly;
        self.outbox.actor.look_sidewards = Some(
            if crate::sim_rng::u32(sim, crate::sim_rng::RngSite::CheckForLookDirection, 0..2) != 0 {
                LookDirection::LeftRight
            } else {
                LookDirection::RightLeft
            },
        );
    }

    // -- Stop all --

    /// Halts the actor's current active sequence element via the engine
    /// (equivalent to `Stop(PREFERENCE)`), breaks the macro, and clears
    /// the AI-side timers. The actual halt happens in the engine
    /// post-think drain where it can borrow `&mut Engine`; see
    /// [`AiController::outbox`] actor-preemption barrier.
    pub fn stop_all(&mut self) {
        // When in a CheckFor look-around, clear the checkpoint
        // *before* the halt so the missed-friend detectable list and
        // `sorrow_level` reset side-effects fire.
        let in_charly_look = matches!(
            self.current_substate,
            Substate::DefaultLookingForCharly | Substate::DefaultLookingSidewardsForCharly
        );
        if in_charly_look {
            self.set_checkpoint_charly(0);
        }
        self.outbox.actor.halt = true;
        // Skip BreakMacro when we're in a CheckFor look or being
        // instructed by an officer — these substates need the
        // in-flight macro to survive the halt.
        let skip_break_macro =
            in_charly_look || self.current_substate == Substate::SeekingGroupGetInstructedByOfficer;
        if !skip_break_macro {
            self.break_macro();
        }
    }

    /// Drop every queued `pending_*` intent that a prior `think()` set
    /// but the engine hasn't yet drained.
    ///
    /// These fields exist because Rust's borrow checker forbids holding
    /// a `&mut Engine` during `think()`, so engine-side calls
    /// (`SetState`, `EnterSwordfight`, `GoTo`, …) become `pending_*`
    /// flags on the AiController that the engine drains after think
    /// returns. `handle_death_with_damage_element` needs to clear every
    /// one of them so stale intents from the pre-death think don't fire
    /// on a corpse; replacing the complete outbox keeps that cauterisation
    /// exhaustive as new effect fields are introduced.
    pub fn clear_all_pending(&mut self) {
        // Replace the entire outbox so death/teardown clears every barrier,
        // including detection, re-entrant, recovery, speech, and music work.
        // This is deliberately exhaustive-by-construction: adding a new
        // effect field to AiOutbox cannot silently escape this cauterisation.
        self.outbox = AiOutbox::default();
    }

    // -- Movement commands --
    // These record intent and produce an Order for the engine to dispatch.

    /// Build a movement order from destination + flags.
    ///
    /// Maps `GotoFlags` to the appropriate `OrderType` and `MoveFlags`:
    /// - `RIDER_CHARGE_HIT` → `OrderType::RiderCharging` (charge with hit zone)
    /// - `RIDER_CHARGE` → `MoveFlags::RIDER_CHARGE` (running, fires galopp events)
    ///
    pub(crate) fn make_move_order(destination: &Position, flags: GotoFlags) -> AiOrderIntent {
        use crate::order::OrderType;
        use crate::sequence::MoveFlags;

        // Determine movement action.
        let order_type = if flags.contains(GotoFlags::RIDER_CHARGE_HIT) {
            OrderType::RiderCharging
        } else if flags.contains(GotoFlags::RUN) {
            OrderType::RunningUpright
        } else {
            OrderType::WalkingUpright
        };

        let mut order = AiOrderIntent::new(order_type, destination.x, destination.y);
        order.target_sector = destination.sector;
        order.target_layer = Some(destination.level);
        order.reverse = flags.contains(GotoFlags::BACK);
        order.compute_direction = !flags.contains(GotoFlags::STRAIGHT);
        // Preserve the authored flag in the intent. The shipped C++ GoTo
        // pre-launch Halt gate is accidentally dead because of operator
        // precedence (`flags & GOTO_NOHALT == 0`), so movement dispatch does
        // not act on this value; other intent families still use `no_halt`.
        order.no_halt = flags.contains(GotoFlags::NO_HALT);

        // Set movement-sequence flags derived from GoTo flags.
        // `GOTO_SWORD` always adds `FORCE_SWORD_MOVEMENT`, even when
        // the actor was already in a sword action-state; this keeps
        // combat spacing and step-back dodges out of ordinary walk/run
        // animation.
        if flags.contains(GotoFlags::RIDER_CHARGE) {
            order.move_flags = MoveFlags::RIDER_CHARGE.bits() as u16;
        }
        if flags.contains(GotoFlags::SWORD) {
            order.move_flags |= MoveFlags::FORCE_SWORD_MOVEMENT.bits() as u16;
        }
        if flags.contains(GotoFlags::STRAIGHT) {
            order.move_flags |= MoveFlags::STRAIGHT.bits() as u16;
        }
        // RHArtificialIntelligence::GoTo maps GOTO_DONTSTOP to
        // RHMOVE_NO_TRANSITIONS.  Route legs that flow through their next
        // waypoint must not splice in a walk/run-to-wait end transition;
        // doing so delays EventReachPoint and advances the patrol AI one
        // frame late.
        if flags.contains(GotoFlags::DONT_STOP) {
            order.move_flags |= MoveFlags::NO_TRANSITIONS.bits() as u16;
        }

        // Forward `GOTO_FINDACCESSIBLE` and `GOTO_ASKOBSTACLE` to the
        // engine drain. The drain has the FastFindGrid in hand and
        // runs `FindAutorizedPosition` / `IsStraightMovementAutorized`,
        // then either rewrites the destination, sets
        // `couldnt_reachpoint`, or both.
        order.find_accessible = flags.contains(GotoFlags::FIND_ACCESSIBLE);
        order.ask_obstacle = flags.contains(GotoFlags::ASK_OBSTACLE);

        order
    }

    /// Check if the entity is already at `destination` within `tolerance`
    /// (MaxNorm).
    fn check_already_on_point(
        &self,
        destination: &Position,
        tolerance: f32,
        ctx: &AiContext,
    ) -> bool {
        let dx = (ctx.position.x - destination.x).abs();
        let dy = (ctx.position.y - destination.y).abs();
        dx.max(dy) < tolerance
    }

    /// Low-level movement primitive — queues a movement intent without
    /// committing to a substate transition.  Prefer the `EnemyAi::go_to` /
    /// `FriendlyAi::go_to` wrappers, which enforce the Shape 1 contract
    /// (every queued movement names the new substate atomically so the
    /// halt-teardown in `process_pending_ai_orders` can't orphan the AI
    /// in a "waiting" substate). Calling this directly via
    /// `ai.base.go_to(...)` bypasses that contract and risks wedge bugs.
    pub fn go_to(&mut self, destination: Position, flags: GotoFlags, ctx: &AiContext) {
        // Record the latest destination / flags so stuck-retry replays,
        // cancellation, and the EventReachPoint re-entry path can see
        // what was most recently requested.
        self.last_goto_destination = destination;
        self.last_goto_flags = flags;
        self.couldnt_reachpoint = false;

        // Civilians must not be issued combat / rider-charge flags.
        // Mask `FORBIDDEN_CIVILIANS` silently — civilians hitting one
        // of these flags usually indicates a script or AI bug, but the
        // game keeps running.
        let mut flags = flags;
        if !ctx.self_is_soldier {
            let forbidden = flags & GotoFlags::FORBIDDEN_CIVILIANS;
            if !forbidden.is_empty() {
                tracing::warn!(
                    me = self.me,
                    ?forbidden,
                    "civilian GoTo with forbidden flags — masking",
                );
                flags -= GotoFlags::FORBIDDEN_CIVILIANS;
            }
        }

        // Already-on-point fast-exit. Gated on:
        //   - MaxNorm < 5 from the entity to the destination
        //   - `!likes_to_sit_around && !special_action`
        //   - animation state ∈ {WAITING_UPRIGHT, WAITING_ALERTED,
        //                         NONANIMATION_END}
        // When the gate fires, `end_think` drains `already_on_point`
        // into a `Think(EVENT_REACHPOINT)` re-entry.
        let idle_for_goto_short_circuit = matches!(
            ctx.self_animation,
            crate::order::OrderType::WaitingUpright
                | crate::order::OrderType::WaitingAlerted
                | crate::order::OrderType::NonanimationEnd
        );
        let may_short_circuit =
            idle_for_goto_short_circuit && !self.likes_to_sit_around && !self.special_action;
        if may_short_circuit && self.check_already_on_point(&destination, 5.0, ctx) {
            self.already_on_point = true;
            return;
        }

        // Out-of-level-bounds destinations fail fast with
        // `couldnt_reachpoint`. The non-negative half is enforced here;
        // the upper-bound `>= GetLevelSize()` half is enforced by the
        // engine drain in `preflight_ai_goto`, which has access to the
        // shared cutscene camera's level size.
        if destination.x <= 0.0 || destination.y <= 0.0 {
            self.couldnt_reachpoint = true;
            return;
        }

        // Null sector or negative layer → fail fast.
        // `Position.sector == None` represents a null sector; layer is
        // `u16` so the "negative layer" branch becomes unreachable
        // unless a caller stuffs `u16::MAX` in deliberately.
        if destination.sector.is_none() {
            self.couldnt_reachpoint = true;
            return;
        }

        // Strip `GOTO_STRAIGHT` when the destination crosses sector or
        // layer **and** the caller didn't pair it with
        // `GOTO_ASKOBSTACLE` — straight doesn't make sense across
        // sectors without an obstacle check.
        let crosses_boundary =
            destination.sector != ctx.position.sector || destination.level != ctx.position.level;
        if flags.contains(GotoFlags::STRAIGHT)
            && !flags.contains(GotoFlags::ASK_OBSTACLE)
            && crosses_boundary
        {
            flags -= GotoFlags::STRAIGHT;
        }

        // Prepend the appropriate action-state teardown before the
        // move is launched. Centralised here so every caller benefits
        // — the engine drain processes these intents before
        // `launch_pending_orders_for_npc` runs the move.
        let quit_swordfight_before_move = self.apply_goto_action_state_teardown(flags, ctx);

        let mut order = Self::make_move_order(&destination, flags);
        order.quit_swordfight_before_move = quit_swordfight_before_move;
        self.outbox.actor.orders.push(order);
    }

    /// Prepend the action-state teardown for a launching GoTo / GoNear /
    /// GoToSpeed:
    ///
    ///   * `GOTO_SWORD` set, not currently in a sword action-state →
    ///     prepend `ENTER_SWORDFIGHT` (raise-sword pose, no opponent).
    ///   * `GOTO_SWORD` not set, currently in a sword action-state →
    ///     prepend `QUIT_SWORDFIGHT` (sheath sword + clear opponents).
    ///   * `GOTO_SWORD` not set, currently `Menacing` → prepend
    ///     `STOP_MENACE` (menacing → waiting-sword → sword-down).
    ///
    /// Shield is also handled here: an actor mid-shield-raise that
    /// receives a `GoTo` first prepends a `Command::LowerShield` element
    /// so the shield drops before the move launches.
    fn apply_goto_action_state_teardown(&mut self, flags: GotoFlags, ctx: &AiContext) -> bool {
        let action_state = ctx.self_action_state;
        let mut quit_swordfight_before_move = false;
        if flags.contains(GotoFlags::SWORD) {
            // GOTO_SWORD branch — already-in-sword is a no-op,
            // otherwise prepend ENTER_SWORDFIGHT without an opponent.
            if !action_state.is_sword() && self.outbox.actor.enter_swordfight.is_none() {
                self.outbox.actor.enter_swordfight = Some(EnterSwordfightRequest::RaiseSword);
                self.outbox.actor.enter_swordfight_jump_line = None;
            }
        } else if action_state.is_sword() {
            // Leaving a sword fight to walk somewhere without GOTO_SWORD:
            // the engine must put QuitSwordfight and Move in one ordered
            // sequence. A standalone outbox effect would clear relationships
            // and then let the independent movement preempt the lowering
            // animation in the same drain.
            quit_swordfight_before_move = true;
        } else if action_state == crate::element::ActionState::Menacing {
            // Drop the menace pose before walking.
            self.outbox.actor.stop_menace = true;
        }

        // Orthogonal to the sword/menace switch above — the shield
        // branch fires whenever the actor is in any shield action-state,
        // regardless of GOTO_SWORD. Prepend a `Command::LowerShield`
        // element so the shield drops (and the parry geometry stops
        // being armed) before the queued move runs.
        if action_state.is_shield() {
            self.outbox.actor.lower_shield = true;
        }
        quit_swordfight_before_move
    }

    /// Low-level movement primitive (speed variant) — see
    /// [`AiController::go_to`] for the Shape 1 contract caveat.  Prefer
    /// `EnemyAi::go_to_speed` / `FriendlyAi::go_to_speed`.
    pub fn go_to_speed(
        &mut self,
        destination: Position,
        flags: GotoFlags,
        speed: f32,
        ctx: &AiContext,
    ) {
        self.last_goto_destination = destination;
        self.last_goto_flags = flags;
        self.couldnt_reachpoint = false;
        // This is the same Original GoTo overload as the default-speed
        // wrapper. Its close-point shortcut is legal only from one of the
        // idle animations; a running patrol member may pass within five
        // units of a newly coordinated formation point and must still book
        // the replacement walk rather than synthesize EventReachPoint.
        let idle_for_goto_short_circuit = matches!(
            ctx.self_animation,
            crate::order::OrderType::WaitingUpright
                | crate::order::OrderType::WaitingAlerted
                | crate::order::OrderType::NonanimationEnd
        );
        let may_short_circuit =
            idle_for_goto_short_circuit && !self.likes_to_sit_around && !self.special_action;
        if may_short_circuit && self.check_already_on_point(&destination, 5.0, ctx) {
            self.already_on_point = true;
            return;
        }
        let quit_swordfight_before_move = self.apply_goto_action_state_teardown(flags, ctx);
        let mut order = Self::make_move_order(&destination, flags);
        order.speed_factor = speed;
        order.quit_swordfight_before_move = quit_swordfight_before_move;
        self.outbox.actor.orders.push(order);
    }

    /// Queue the direct map-exit movement used by
    /// `SUBSTATE_FLEEING_MERRY_MAN_RUN_TO_LEAVE_MAP` after the NPC
    /// reaches the reinforcement door. C++ launches an
    /// `RHSequenceElementMovement(..., RHANIMATION_RUNNING_UPRIGHT,
    /// pointOut, NULL, 0.f, RHMOVE_MAP)` rather than a regular GoTo.
    pub fn run_to_map_exit(&mut self, destination: Position) {
        self.last_goto_destination = destination;
        self.last_goto_flags = GotoFlags::RUN;
        self.couldnt_reachpoint = false;

        let mut order = Self::make_move_order(&destination, GotoFlags::RUN);
        // This is deliberately a local RHMOVE_MAP element, not the full
        // RHposition-aware GoTo overload.
        order.target_sector = None;
        order.target_layer = None;
        order.move_flags |= crate::sequence::MoveFlags::MAP.bits() as u16;
        self.outbox.actor.orders.push(order);
    }

    /// Low-level movement primitive (go-near variant) — see
    /// [`AiController::go_to`] for the Shape 1 contract caveat. Prefer
    /// `EnemyAi::go_near` / `FriendlyAi::go_near`.
    ///
    /// Pre-scales the tolerance under deep recursion (release-build
    /// mitigation), then tail-calls `go_to` with the NEAR flag OR'd in
    /// so `last_goto_flags` preserves the near semantics for
    /// stuck-retry replays.
    pub fn go_near(
        &mut self,
        destination: Position,
        distance: i32,
        flags: GotoFlags,
        ctx: &AiContext,
    ) {
        // Deep recursion shrinks the stop-distance toward zero so the
        // actor doesn't loop on Think() recursion. Always applied — a
        // mitigation, not a behaviour knob.
        let effective_distance = if self.think_recursion_depth < 10 {
            distance
        } else {
            let depth = self.think_recursion_depth as i32;
            (((100 - depth) * distance) / 100).max(0)
        };

        // The near-distance early-out also requires same-layer — a
        // different-layer destination falls through to the full launch
        // path. Apply that gate here before `go_to`'s own MaxNorm-5
        // check fires, since `go_to`'s check has no layer guard and the
        // tolerance argument we'd want isn't visible downstream.
        self.last_goto_destination = destination;
        self.last_goto_flags = flags | GotoFlags::NEAR;
        self.couldnt_reachpoint = false;

        let same_layer = destination.level == ctx.position.level;
        if same_layer && self.check_already_on_point(&destination, effective_distance as f32, ctx) {
            self.already_on_point = true;
            return;
        }

        let quit_swordfight_before_move = self.apply_goto_action_state_teardown(flags, ctx);
        let mut order = Self::make_move_order(&destination, flags);
        order.tolerance = effective_distance as f32;
        order.quit_swordfight_before_move = quit_swordfight_before_move;
        self.outbox.actor.orders.push(order);
    }

    // -- Facing commands --

    /// Turn to face a position, without an `AiContext` available.
    ///
    /// Queues a plain Turn order; does not honour the same-frame
    /// `already_turned` short-circuit (no access to the current
    /// direction / action state). Prefer [`Self::face_position_with_ctx`]
    /// at call sites that have a ctx.
    pub fn face_position(&mut self, pos: Position) {
        self.outbox
            .actor
            .orders
            .push(AiOrderIntent::face_toward(pos.x, pos.y));
    }

    /// Internal helper — all `face_*_with_ctx` / `face_entity[_fast]`
    /// variants funnel through this so the short-circuit and elevation
    /// handling live in exactly one place.
    ///
    /// - `elevation_delta`: `target_elevation - ctx.elevation`. Pass
    ///   `0.0` for 2D-only faces. The target's elevation shifts the
    ///   effective dy before the aspect-ratio scale.
    pub(super) fn face_position_impl(
        &mut self,
        pos: Position,
        ctx: &AiContext,
        elevation_delta: f32,
        fast: bool,
    ) {
        let dx = pos.x - ctx.position.x;
        let dy = (pos.y - ctx.position.y) + elevation_delta;
        let target_dir = crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy);
        // legacy implementation FaceTo only short-circuits same-direction turns while
        // WAITING or BORED. Other action states still launch Turn so
        // Halt() semantics are preserved.
        let may_short_circuit = Self::face_to_same_direction_can_short_circuit(ctx);
        tracing::trace!(
            me = self.me,
            target_dir,
            current_dir = ctx.direction,
            ?ctx.posture,
            ctx.is_swordfighting,
            ?ctx.self_action_state,
            may_short_circuit,
            already_matches = (target_dir as u16 == ctx.direction),
            elevation_delta,
            "face_position_impl"
        );
        if target_dir as u16 == ctx.direction && may_short_circuit {
            self.already_turned = true;
            return;
        }
        let mut intent = AiOrderIntent::face_direction(target_dir);
        intent.fast_turn = fast;
        self.outbox.actor.orders.push(intent);
    }

    /// Turn to face a position (2D — no elevation adjustment). Honours
    /// the `already_turned` same-frame short-circuit.
    pub fn face_position_with_ctx(&mut self, pos: Position, ctx: &AiContext) {
        self.face_position_impl(pos, ctx, 0.0, false);
    }

    /// Face an `RHposition` through the Original `PositionToPoint3D`
    /// projection. Unlike the explicitly 2D overload above, this preserves
    /// the target sector/layer elevation in world-horizontal Y before
    /// selecting the isometric direction.
    pub fn face_position_3d_with_ctx(&mut self, pos: Position, ctx: &AiContext) {
        let target = ctx.position_to_point_3d(pos);
        self.face_position_impl(pos, ctx, target.z - ctx.elevation, false);
    }

    /// Turn to face another entity. Feeds the target's elevation into
    /// the 2D projection so the face accounts for height differences.
    ///
    /// Silently drops if the handle is `0` or the entity is no longer
    /// present in the snapshot.
    pub fn face_entity(&mut self, handle: NpcHandle, ctx: &AiContext) {
        let Some(view) = ctx.entity_view(handle) else {
            return;
        };
        let elevation_delta = view.elevation - ctx.elevation;
        let target_pos = view.position;
        self.face_position_impl(target_pos, ctx, elevation_delta, false);
    }

    /// Turn quickly to face another entity (`Face(element, true)`).
    pub fn face_entity_fast(&mut self, handle: NpcHandle, ctx: &AiContext) {
        let Some(view) = ctx.entity_view(handle) else {
            return;
        };
        let elevation_delta = view.elevation - ctx.elevation;
        self.face_position_impl(view.position, ctx, elevation_delta, true);
    }

    /// Match a direct Original `RHElement::SetDirection` toward an entity:
    /// update only the progressive direction goal and do not launch a Turn
    /// sequence. The currently selected animation may perform the turn itself.
    pub fn set_direction_toward_entity(&mut self, handle: NpcHandle, ctx: &AiContext) {
        let Some(view) = ctx.entity_view(handle) else {
            return;
        };
        let dx = view.position.x - ctx.position.x;
        let dy = view.position.y - ctx.position.y;
        let direction = crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy);
        self.outbox.actor.set_direction = Some(direction as i16);
    }

    // -- Self-stimuli --

    /// Queue a stimulus to be re-dispatched to this NPC on the next tick.
    /// The engine drains `pending_self_stimuli` and re-dispatches them
    /// after the current think cycle.
    pub fn fire_self_stimulus(&mut self, stimulus_type: StimulusType) {
        self.outbox.reentrant.self_stimuli.push(stimulus_type);
    }

    /// Turn to face a direction (0–15 sector).
    ///
    /// Honours the same-direction short-circuit: if the actor is
    /// already facing the requested sector **and** WAITING or BORED,
    /// set `already_turned` so `end_think` fires a same-frame
    /// `EVENT_DONE` re-entry instead of queuing a no-op Turn order.
    pub fn face_direction(&mut self, direction: u16, ctx: &AiContext) {
        if direction == ctx.direction && Self::face_to_same_direction_can_short_circuit(ctx) {
            self.already_turned = true;
            return;
        }
        self.launch_turn_direction_unconditionally(direction);
    }

    fn launch_turn_direction_unconditionally(&mut self, direction: u16) {
        // Original `FaceTo(UWORD)` stores the authored sector directly in
        // RHFIELD_DIRECTION. Keep it discrete rather than round-tripping it
        // through a synthetic point and a later vector-to-sector conversion.
        self.outbox
            .actor
            .orders
            .push(AiOrderIntent::face_direction(direction as i16));
    }

    fn face_to_same_direction_can_short_circuit(ctx: &AiContext) -> bool {
        matches!(
            ctx.self_action_state,
            crate::element::ActionState::Waiting | crate::element::ActionState::Bored
        )
    }

    // -- Speech commands --

    /// Say a remark (no flags).
    pub fn say(&mut self, remark: Remark) {
        self.say_impl(remark, SpeechFlags::empty());
    }

    /// Say a remark with special flags.
    pub fn say_with_flags(&mut self, remark: Remark, flags: SpeechFlags) {
        self.say_impl(remark, flags);
    }

    /// Record one ordered `RHArtificialIntelligence::Say` attempt.
    ///
    /// The engine settles every attempt at the current owner return boundary,
    /// where entity/profile/global-forbid state is available. Do not collapse
    /// this into `current_remark`: the Original observes and rejects every
    /// invocation in statement order, including multiple calls in one Think.
    fn say_impl(&mut self, remark: Remark, flags: SpeechFlags) {
        self.outbox
            .reentrant
            .owner_work
            .push(AiOwnerWork::Speech(AiSpeechAttempt {
                remark,
                flags: flags.bits(),
            }));
    }

    // -- Pointing command --

    /// Point at a position (animation command).
    ///
    /// Queues two sequence elements back-to-back — TURN then POINT —
    /// so the actor first finishes the turn and only then plays the
    /// pointing animation. The order-drain / animation layer runs
    /// them in order.
    ///
    /// (A `SetViewTarget(posTarget, false)` would bias head-tracking
    /// toward the point, but the position-taking `SetViewTarget`
    /// overload is a stub in the original game as well, so we skip
    /// it.)
    pub fn point_to(&mut self, pos: Position) {
        use crate::order::OrderType;
        // Pre-turn so the pointing anim fires already facing the
        // target. The Turning-order's own `already_facing`
        // short-circuit isn't worth wiring here — callers already
        // queue `pending_halt` / `stop_all` before `point_to` via the
        // instruct flow, so the Turn will run cleanly.
        self.outbox
            .actor
            .orders
            .push(AiOrderIntent::face_toward(pos.x, pos.y));
        self.outbox
            .actor
            .orders
            .push(AiOrderIntent::new(OrderType::Pointing, pos.x, pos.y));
    }

    // -- Alert status --

    /// Set the NPC's alert status (affects music + view).
    ///
    /// Writes both the music-side counter
    /// (`current_music_alert_status`) and the view-side field
    /// (`view_alert_status`). This is the override-free path: callers
    /// that need the soldier `IsForcedAttentive` view override should
    /// go through `EnemyAi::set_alert_status` (or call
    /// `set_alert_status_with_flags` directly with `forced_attentive =
    /// true`).
    ///
    /// The music-system side — aggregating all soldier statuses into
    /// the overall villain alert and calling `SetMusicMode` — runs
    /// once per frame in `EngineInner::update_overall_villain_alert`.
    pub fn set_alert_status(&mut self, level: AlertLevel) {
        self.set_alert_status_with_flags(level, AlertFlags::empty(), false);
    }

    /// Full-fidelity `set_alert_status(new_status, flags)`.
    ///
    /// Always updates `current_music_alert_status`. Returns early
    /// without touching the view field when `flags` contains
    /// `ALERT_ONLY_MUSIC`. Otherwise writes the view field, applying
    /// the soldier `IsForcedAttentive` override (Green music ⇒ Yellow
    /// view) when `forced_attentive` is set.
    ///
    /// `INSTANT_MUSIC_CHANGE` is staged on `pending_instant_music_change`
    /// when the call actually changes `current_music_alert_status`, and
    /// observed by the per-frame `update_overall_villain_alert` sweep.
    pub fn set_alert_status_with_flags(
        &mut self,
        level: AlertLevel,
        flags: AlertFlags,
        forced_attentive: bool,
    ) {
        if flags.contains(AlertFlags::INSTANT_MUSIC_CHANGE)
            && level != self.current_music_alert_status
        {
            self.outbox.music.instant_change = true;
        }
        self.current_music_alert_status = level;

        if flags.contains(AlertFlags::ONLY_MUSIC) {
            return;
        }

        self.view_alert_status = if forced_attentive && level == AlertLevel::Green {
            AlertLevel::Yellow
        } else {
            level
        };
    }

    // -- Return to duty (common) --

    /// Common return-to-duty logic shared by soldiers and civilians.
    pub fn return_to_duty_common_stuff(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        flags: DutyFlags,
        ctx: &AiContext,
    ) {
        // Start with `SetAlertStatus(GREEN)` — no `BreakMacro` /
        // `RetrogradeAmnesia` here, those are called by their own
        // call-sites elsewhere in the state machine. Route through the
        // flags-aware setter so a forced-attentive soldier returning
        // to duty keeps the view cone YELLOW even though the music
        // drops to GREEN.
        self.set_alert_status_with_flags(
            AlertLevel::Green,
            AlertFlags::empty(),
            ctx.self_forced_attentive,
        );

        if !flags.contains(DutyFlags::KEEP_EMOTICON) {
            self.clear_emoticon();
        }

        // Reset patrol path history so formation rebuilds cleanly.
        if let Some(ref mut path) = self.patrol_path {
            path.reset_history();
        }
        self.my_reconnaissance_report.reset();

        // Drop any stale `detected_body` pointer once the NPC no
        // longer has outstanding `DETECTABLE_FRIEND` entries (i.e.
        // it's finished swapping reports with alerted allies). The
        // friend count rides in on `ctx` so we don't have to crack
        // open `NpcData` from inside the AI.
        if ctx.self_detectable_friend_count == 0 {
            self.detected_body = 0;
        }

        // If this NPC has a live patrol chief that's able to fight
        // *and* within 360° detection range, run to them and enter
        // `DefaultGotoChief` — let the chief re-gather the patrol as
        // the minion closes. Only abandon the goto-chief path when
        // `couldnt_reachpoint` fires (then fall through to the normal
        // return-to-post logic below).
        if let Some(chief_id) = self.patrol_chief
            && let Some(chief_view) = ctx.entity_view(chief_id.index())
            && chief_view.is_able_to_fight
        {
            // `IsDetecting360Degrees`: aspect-ratio-corrected distance
            // from me to the chief against our squared view radius.
            // Distance-only form (no LOS check), matching
            // `EnemyAi::is_detecting_360_degrees` in ai_enemy.rs.
            let dx = chief_view.position.x - ctx.position.x;
            let dy = chief_view.position.y - ctx.position.y;
            let sq_distance = crate::position_interface::vector_square_norm_iso(dx, dy);
            if sq_distance <= ctx.sq_standard_view_radius {
                self.set_ai_state(AiState::Default);
                self.current_substate = Substate::DefaultGotoChief;
                self.go_near(
                    chief_view.position,
                    crate::parameters_ai::AI_TALK_DISTANCE,
                    GotoFlags::empty(),
                    ctx,
                );
                if !self.couldnt_reachpoint {
                    return;
                }
                // Couldn't reach — reset flag and fall through to the
                // post/patrol-path logic.
                self.couldnt_reachpoint = false;
            }
        }

        let hiking_paths = &ctx.hiking_paths;

        if self.has_patrol_path {
            // Initialize patrol path if not yet done.
            if self.patrol_path.is_none() {
                self.patrol_path = self
                    .path_id
                    .and_then(|pid| PatrolPath::new(pid, hiking_paths));
                if self.patrol_path.is_none() {
                    tracing::warn!(
                        "NPC {} has_patrol_path but path_id {:?} is invalid, falling back to post",
                        self.me,
                        self.path_id,
                    );
                    self.has_patrol_path = false;
                }
            }

            if let Some(ref mut path) = self.patrol_path {
                let pos_here = ctx.position;
                let num_waypoints = path.size;

                // Find the nearest waypoint by MaxNorm distance.
                let mut best_index: u8 = 0;
                let mut min_dist = f32::MAX;
                for i in 0..num_waypoints {
                    if let Some(wp) = path.get_waypoint(i, hiking_paths) {
                        let dx = (wp.x as f32 - pos_here.x).abs();
                        let dy = (wp.y as f32 - pos_here.y).abs();
                        let dist = dx.max(dy); // MaxNorm
                        if dist < min_dist {
                            min_dist = dist;
                            best_index = i;
                        }
                    }
                }

                path.set_current_index(best_index);

                // Check whether going from here → nearest → next requires >90° turn.
                // If so, skip to the next waypoint.
                if let Some(wp) = path.current_waypoint(hiking_paths) {
                    let dir_x = wp.x as f32 - pos_here.x;
                    let dir_y = wp.y as f32 - pos_here.y;
                    let dir_norm = dir_x.abs().max(dir_y.abs());

                    if (best_index as usize) < (num_waypoints as usize).saturating_sub(1)
                        && let Some(next_wp) = path.peek_next_waypoint(hiking_paths)
                    {
                        let next_dx = next_wp.x as f32 - wp.x as f32;
                        let next_dy = next_wp.y as f32 - wp.y as f32;
                        // Dot product < 0 means >90° turn
                        let dot = dir_x * next_dx + dir_y * next_dy;
                        if dir_norm < 10.0 || dot < 0.0 {
                            path.advance();
                        }
                    }
                }
            }

            // Now that the path borrow is done, set state and issue walk order.
            if self.patrol_path.is_some() {
                self.set_ai_state(AiState::Default);
                self.current_substate = Substate::DefaultGotoRoute;

                // At frame 0, if this is a patrol chief near its start, pre-seed
                // history so minions can form up immediately.
                let is_patrol_chief = self.has_patrol();
                let is_frame_zero = ctx.frame == 0;
                if is_patrol_chief
                    && is_frame_zero
                    && let Some(ref mut path) = self.patrol_path
                {
                    let dir_norm = if let Some(wp) = path.current_waypoint(hiking_paths) {
                        let dx = (wp.x as f32 - ctx.position.x).abs();
                        let dy = (wp.y as f32 - ctx.position.y).abs();
                        dx.max(dy)
                    } else {
                        f32::MAX
                    };
                    if dir_norm < 50.0 {
                        path.initialize_history_entries_on_path(hiking_paths);
                    }
                }

                // Walk to current waypoint.
                let dest = self.patrol_path.as_ref().and_then(|path| {
                    path.current_waypoint(hiking_paths).map(|wp| Position {
                        x: wp.x as f32,
                        y: wp.y as f32,
                        sector: SectorHandle::new(wp.sector),
                        level: wp.level,
                    })
                });
                if let Some(dest) = dest {
                    let mut walk_flags = self.default_path_walking_flags;
                    if !self.will_stop_at_next_waypoint(sim, hiking_paths) {
                        walk_flags |= GotoFlags::DONT_STOP;
                    }
                    self.go_to(dest, walk_flags, ctx);
                }
            }
        } else if self.likes_to_sit_around {
            // Sitting NPCs: check if already at initial position — if
            // so, stay put; otherwise walk back with
            // `GOTO_SPECIAL_ACTION`.
            let ip = self.initial_position;
            let dx = (ctx.position.x - ip.x).abs();
            let dy = (ctx.position.y - ip.y).abs();
            if matches!(ctx.posture, crate::element::Posture::Sitting) && dx.max(dy) < 3.0 {
                // Already on sitting place.
                self.set_ai_state(AiState::Default);
                self.current_substate = Substate::DefaultOnPost;
                let bored = self.get_bored_time(sim, ctx);
                self.launch_timer(bored as u32, ctx.frame);
            } else {
                // Return to sitting place.
                self.set_ai_state(AiState::Default);
                self.current_substate = Substate::DefaultGotoPost;
                self.go_to(ip, GotoFlags::SPECIAL_ACTION, ctx);
            }
        } else if self.special_action {
            // Leisure-posture NPCs: same shape as the sitting branch
            // but keyed on posture==LEISURE and also uses
            // GOTO_SPECIAL_ACTION.
            let ip = self.initial_position;
            let dx = (ctx.position.x - ip.x).abs();
            let dy = (ctx.position.y - ip.y).abs();
            if matches!(ctx.posture, crate::element::Posture::Leisure) && dx.max(dy) < 3.0 {
                // Already on leisure place.
                self.set_ai_state(AiState::Default);
                self.current_substate = Substate::DefaultOnPost;
                let bored = self.get_bored_time(sim, ctx);
                self.launch_timer(bored as u32, ctx.frame);
            } else {
                // Return to leisure place.
                self.set_ai_state(AiState::Default);
                self.current_substate = Substate::DefaultGotoPost;
                self.go_to(ip, GotoFlags::SPECIAL_ACTION, ctx);
            }
        } else {
            // Plain return-to-post: no `GOTO_SPECIAL_ACTION`, no
            // posture gate — just the Original's bare
            // `GoTo(initial_position)` with default flags.
            let ip = self.initial_position;
            self.set_ai_state(AiState::Default);
            self.current_substate = Substate::DefaultGotoPost;
            self.go_to(ip, GotoFlags::empty(), ctx);
        }
    }

    /// Forecast whether the actor will stop at its current waypoint.
    ///
    /// Returns `true` when the selected macro section starts with an
    /// opcode that halts the actor (`CMD_WAIT`, `CMD_FACE_TO`,
    /// `CMD_BEND`, `CMD_CHECK_4*`, `CMD_LOOK_LEFT`, `CMD_LOOK_RIGHT`,
    /// `CMD_STAY_HERE`). Returns `false` for purely-motion sections
    /// (`CMD_RUN`/`CMD_WALK`/`CMD_REVERSE_PATH`/`CMD_GOTO_POINT`/…) so
    /// the caller keeps the `DONT_STOP` flag and walks through. Takes
    /// `&mut self` so it can call [`Self::forecast_macro_rand`] (peek
    /// without consuming).
    pub fn will_stop_at_next_waypoint(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        hiking_paths: &[crate::level_data::RawHikingPath],
    ) -> bool {
        use crate::level_data::WaypointCommand;

        // Collapse the path/waypoint borrow into owned data so the rest
        // of the function can take `&mut self` for `forecast_macro_rand`.
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
                WaypointCommand::Macro(data) => (path.forward, data.clone()),
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

    // -- Patrol coordination --

    /// Handle `CALL_PATROL_COORDINATE` from the chief: walk or run to the
    /// assigned formation position.
    pub fn coordinate_patrol(
        &mut self,
        info: &StimulusInfo,
        ctx: &AiContext,
        patrol_chief_position: Position,
    ) {
        if self.patrol_chief.is_none() {
            // Can happen when stimulus was postponed on door
            return;
        }

        let target_pos = match info {
            StimulusInfo::Position(pos) => *pos,
            _ => return,
        };

        match self.current_substate {
            // From idle/walking substates: stop current activity first
            Substate::DefaultInMacro
            | Substate::DefaultEnroute
            | Substate::DefaultGotoPost
            | Substate::DefaultGotoPostTurn
            | Substate::DefaultOnPost
            | Substate::DefaultGotoChief
            | Substate::DefaultOnPostLookingSidewards => {
                self.stop_all();
                self.coordinate_patrol_walk(target_pos, ctx, patrol_chief_position);
            }
            // Already in patrol formation — just update target
            Substate::DefaultPatrolEnroute
            | Substate::DefaultPatrolEnrouteRunning
            | Substate::DefaultPatrolEnrouteWaiting => {
                self.coordinate_patrol_walk(target_pos, ctx, patrol_chief_position);
            }
            _ => {}
        }
    }

    /// Inner logic for coordinate_patrol — compute speed and walk/run to the
    /// assigned formation position.
    fn coordinate_patrol_walk(
        &mut self,
        target: Position,
        ctx: &AiContext,
        patrol_chief_position: Position,
    ) {
        let vec_to_point = [target.x - ctx.position.x, target.y - ctx.position.y];
        let vec_to_chief = [
            patrol_chief_position.x - ctx.position.x,
            patrol_chief_position.y - ctx.position.y,
        ];
        let distance =
            (vec_to_point[0] * vec_to_point[0] + vec_to_point[1] * vec_to_point[1]).sqrt();
        let speed_factor = PATROL_SPEED_BASE + distance / PATROL_SPEED_DIVISOR;

        // Avoid stepping backward on the inner side of narrow curves:
        // when distance <= 30, check if the target is opposite to the
        // chief direction.
        let near_point_backwards = if distance > 30.0 {
            false
        } else {
            // Aspect-corrected dot product (negative when vec_to_point
            // is pointing away from the chief in isometric map space).
            let inv_ar = crate::position_interface::INVERSE_ASPECT_RATIO;
            vec_to_chief[0] * vec_to_point[0] + vec_to_chief[1] * inv_ar * vec_to_point[1] * inv_ar
                < 0.0
        };

        if near_point_backwards {
            // Just turn to face the officer instead of walking backward
            self.face_position(Position {
                x: patrol_chief_position.x,
                y: patrol_chief_position.y,
                ..ctx.position
            });
            return;
        }

        if speed_factor <= 2.0 {
            self.set_ai_state(AiState::Default);
            self.current_substate = Substate::DefaultPatrolEnroute;
            let flags = GotoFlags::NO_HALT | GotoFlags::DONT_STOP | self.default_path_walking_flags;
            self.go_to_speed(target, flags, speed_factor, ctx);
        } else {
            self.set_ai_state(AiState::Default);
            self.current_substate = Substate::DefaultPatrolEnrouteRunning;
            let flags = GotoFlags::RUN | GotoFlags::NO_HALT | GotoFlags::DONT_STOP;
            self.go_to(target, flags, ctx);
        }
    }

    /// Receive a facing direction from the patrol chief.
    pub fn set_instructed_patrol_direction(&mut self, direction: u16, ctx: &AiContext) {
        self.patrol_direction = direction;
        if self.current_substate == Substate::DefaultPatrolEnrouteWaiting {
            self.face_direction(direction, ctx);
        }
    }

    // -- Common expected event handling --

    /// Handle expected events common to both soldiers and civilians.
    /// Handles default patrol, waypoint processing, macro execution,
    /// and fleeing behavior.
    pub fn think_expected_event_common_stuff(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        ctx: &AiContext,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;
        let hiking_paths = &ctx.hiking_paths;

        match self.current_substate {
            // ─── Return to post ─────────────────────────────────────
            Substate::DefaultGotoPost => {
                if stimulus_type == StimulusType::EventReachPoint {
                    // Reached post — turn to face initial direction.
                    self.face_direction(self.initial_view_direction, ctx);
                    self.set_ai_state(AiState::Default);
                    self.current_substate = Substate::DefaultGotoPostTurn;
                }
            }

            Substate::DefaultGotoPostTurn => {
                if stimulus_type == StimulusType::EventDone {
                    // When `GoTo` was launched with `GOTO_SPECIAL_ACTION`,
                    // the launched sequence already carried the
                    // post-arrival TURN element (set above as
                    // `face_direction(initial_view_direction)`) and a
                    // trailing `SIT_DOWN` / `ENTER_LEISURE` element so
                    // the seated / leisure transition animation plays.
                    // Queue the matching `Command::SitDown` /
                    // `Command::EnterLeisure` here so the engine's
                    // animation driver flips posture → Sitting / Leisure
                    // on completion. Earlier code wrote `pending_posture`
                    // directly which snapped the actor to the seated
                    // frame instead of playing the transition.
                    if self.likes_to_sit_around {
                        self.outbox
                            .actor
                            .launch_commands
                            .push(crate::element::Command::SitDown);
                    } else if self.special_action {
                        self.outbox
                            .actor
                            .launch_commands
                            .push(crate::element::Command::EnterLeisure);
                    }
                    self.set_ai_state(AiState::Default);
                    self.current_substate = Substate::DefaultOnPost;
                    let bored = self.get_bored_time(sim, ctx);
                    self.launch_timer(bored as u32, ctx.frame);
                }
            }

            // ─── On post (idle) ─────────────────────────────────────
            Substate::DefaultOnPost => {
                if stimulus_type == StimulusType::EventTimer {
                    // Enemy AI intercepts this in EnemyAi::think_expected_event
                    // and runs DefaultBoredStandardProcedure before delegating.
                    // The override only exists for soldier AI (friendlies
                    // return false), so the base-class fall-through is
                    // correct: re-launch the bored timer.
                    let bored = self.get_bored_time(sim, ctx);
                    self.launch_timer(bored as u32, ctx.frame);
                }
            }

            // ─── Return to route (has patrol path) ──────────────────
            Substate::DefaultGotoRoute => {
                if stimulus_type == StimulusType::EventReachPoint {
                    self.set_ai_state(AiState::Default);
                    self.current_substate = Substate::DefaultGotoRouteTurn;

                    // Calls `InitializePatrol()` here to rebuild the
                    // coordinate-patrol member list. Raise the
                    // one-shot flag so `tick_patrol_coordination`
                    // Phase 3 picks it up next pass.
                    self.needs_patrol_reinit = true;

                    // Turn to face the direction from the previous waypoint,
                    // but only at a waypoint that carries command data.  The
                    // Original checks both `path.Size() > 1` and
                    // `currentWaypoint->uwSizeOfData > 0`; simple waypoints
                    // synchronously feed EVENT_DONE back into the AI without
                    // launching a Turn element.
                    if let Some(ref mut path) = self.patrol_path {
                        let current_has_command =
                            path.current_waypoint(hiking_paths).is_some_and(|waypoint| {
                                !matches!(
                                    waypoint.command,
                                    crate::level_data::WaypointCommand::None
                                )
                            });
                        if path.size > 1 && current_has_command {
                            // Get the previous waypoint to compute the turn
                            // direction. Original performs `--path`, reads the
                            // waypoint, then performs `++path` on the *live*
                            // RHPath. At either endpoint that round trip also
                            // reverses the path's traversal direction, which
                            // controls DIR_FORWARD/DIR_BACKWARD waypoint
                            // macros. Preserve that iterator side effect.
                            path.retreat();
                            let previous_waypoint =
                                path.current_waypoint(hiking_paths).map(|wp| (wp.x, wp.y));
                            path.advance();
                            if let Some((prev_x, prev_y)) = previous_waypoint {
                                let dx = ctx.position.x - prev_x as f32;
                                let dy = ctx.position.y - prev_y as f32;
                                let sector =
                                    crate::position_interface::vector_to_sector_0_to_15(dx, dy);
                                // This is deliberately not `FaceTo`: Original
                                // constructs and launches RHCOMMAND_TURN
                                // directly here.  In particular, an actor
                                // already facing the route direction must
                                // still keep the Turn alive until its actor
                                // sequence completes; FaceTo's waiting/bored
                                // same-direction shortcut would recursively
                                // synthesize EVENT_DONE and enter the waypoint
                                // macro in this same owner boundary.
                                self.launch_turn_direction_unconditionally(sector as u16);
                            } else {
                                // No previous waypoint, skip turn.
                                self.think_event_done_on_self(sim, ctx);
                            }
                        } else {
                            // Single waypoint path — skip turn.
                            self.think_event_done_on_self(sim, ctx);
                        }
                    } else {
                        self.think_event_done_on_self(sim, ctx);
                    }
                }
            }

            // ─── Walking along route ────────────────────────────────
            Substate::DefaultGotoRouteTurn | Substate::DefaultEnroute => {
                let is_route_turn = self.current_substate == Substate::DefaultGotoRouteTurn;
                let is_enroute = self.current_substate == Substate::DefaultEnroute;

                if (stimulus_type == StimulusType::EventDone && is_route_turn)
                    || (stimulus_type == StimulusType::EventReachPoint && is_enroute)
                {
                    if let Some(ref mut path) = self.patrol_path {
                        if path.size == 0 {
                            // Path was eliminated (by script?) — return to duty.
                            self.return_to_duty_common_stuff(sim, DutyFlags::empty(), ctx);
                            return false;
                        }

                        // Dispatch `EventSyncCharly` to every
                        // synchronizing actor waiting on this NPC's patrol.
                        // Drop entries whose substate has already advanced
                        // away from `DefaultSynchronizing`. We dispatch
                        // via the pending-cross-npc drain so the
                        // post-dispatch re-check happens on the next
                        // arrival rather than inline. Net effect: at
                        // worst an extra redundant `EventSyncCharly`
                        // fires one cycle after the actor has left the
                        // wait state.
                        if !self.synchronizing_actors.is_empty() {
                            let wp_idx = path.current_waypoint_index;
                            let mut keep = Vec::with_capacity(self.synchronizing_actors.len());
                            for &guy in &self.synchronizing_actors {
                                let substate = ctx
                                    .entity_view(guy)
                                    .map(|v| v.ai_substate)
                                    .unwrap_or(Substate::DefaultGotoPost);
                                if substate == Substate::DefaultSynchronizing {
                                    self.outbox.reentrant.cross_npc_actions.push(
                                        CrossNpcAction::SendStimulus {
                                            target: guy,
                                            stimulus_type: StimulusType::EventSyncCharly,
                                            info: StimulusInfo::Index(wp_idx.into()),
                                            fallback_to_sender: None,
                                            to_whole_patrol: false,
                                        },
                                    );
                                    keep.push(guy);
                                }
                            }
                            self.synchronizing_actors = keep;
                        }

                        let wp_command = path
                            .current_waypoint(hiking_paths)
                            .map(|wp| wp.command.clone())
                            .unwrap_or(crate::level_data::WaypointCommand::None);

                        match wp_command {
                            crate::level_data::WaypointCommand::None => {
                                // Simple waypoint — advance to next.
                                path.advance();

                                // `DefaultBoredStandardProcedure()` would be called
                                // here, but the virtual only fires when the substate
                                // is DEFAULT_ONPOST — at this point we're in
                                // DEFAULT_GOTOROUTE_TURN / DEFAULT_ENROUTE, so the
                                // call is a guaranteed no-op. Skipping it matches
                                // observed behaviour without needing a cross-base
                                // virtual dispatch hook.

                                if let Some(next_wp) = path.current_waypoint(hiking_paths) {
                                    if path.size == 1 {
                                        // One-point path → treat as post.
                                        // Snap the post anchor to the
                                        // current location; otherwise
                                        // `return_to_duty_common_stuff`
                                        // would walk back to the
                                        // level-load spawn.
                                        self.has_patrol_path = false;
                                        self.initial_position = ctx.position;
                                        self.initial_view_direction = ctx.direction & 0x0F;
                                        self.return_to_duty_common_stuff(
                                            sim,
                                            DutyFlags::empty(),
                                            ctx,
                                        );
                                    } else {
                                        let mut walk_flags = self.default_path_walking_flags;
                                        if !self.will_stop_at_next_waypoint(sim, hiking_paths) {
                                            walk_flags |= GotoFlags::DONT_STOP;
                                        }
                                        if is_enroute {
                                            walk_flags |= GotoFlags::STRAIGHT;
                                        }
                                        self.set_ai_state(AiState::Default);
                                        self.current_substate = Substate::DefaultEnroute;
                                        let dest = Position {
                                            x: next_wp.x as f32,
                                            y: next_wp.y as f32,
                                            sector: SectorHandle::new(next_wp.sector),
                                            level: next_wp.level,
                                        };
                                        self.go_to(dest, walk_flags, ctx);
                                    }
                                } else {
                                    // No next waypoint — done.
                                    self.return_to_duty_common_stuff(sim, DutyFlags::empty(), ctx);
                                }
                            }
                            crate::level_data::WaypointCommand::Script(_script) => {
                                // Hand off to the per-waypoint VM.
                                // `execute_waypoint_script` queues a
                                // `ReachPoint(actor)` dispatch against
                                // the instance bound at level load;
                                // the engine drains it post-think and
                                // fires `EventAfterScriptGoOn` if the
                                // script didn't lock us into
                                // `DefaultScriptDriven`.
                                let path_idx = path.hiking_path_index;
                                let wp_idx = path.current_waypoint_index;
                                self.execute_waypoint_script(path_idx, wp_idx);
                            }
                            crate::level_data::WaypointCommand::Macro(macro_data) => {
                                // Full waypoint-macro dispatch. If
                                // `launch_waypoint_macro` returns false,
                                // no section matched this traversal
                                // direction / roll — proceed along the
                                // path like a simple waypoint.
                                let launched = self.launch_waypoint_macro(sim, &macro_data, ctx);
                                if !launched {
                                    self.proceed_on_path(sim, hiking_paths, ctx);
                                }
                            }
                        }
                    } else {
                        // No patrol path — fall back to post.
                        self.return_to_duty_common_stuff(sim, DutyFlags::empty(), ctx);
                        return false;
                    }
                }
            }

            // ─── In macro ───────────────────────────────────────────
            Substate::DefaultInMacro => {
                // Ignore all Done events while executing macros.
            }

            Substate::DefaultInMacroWaitingForDone => {
                if stimulus_type == StimulusType::EventDone {
                    self.execute_next_macro_command(sim, ctx);
                }
            }

            // ─── Fleeing ────────────────────────────────────────────
            Substate::FleeingRunToHide | Substate::FleeingRunToDoor => {
                if stimulus_type == StimulusType::EventReachPoint {
                    self.set_ai_state(AiState::Fleeing);
                    self.current_substate = Substate::FleeingHiding;
                    self.set_alert_status(AlertLevel::Yellow);
                    self.clear_emoticon();
                    // Face panic center and wait.
                    self.face_position(Position {
                        x: self.panic_center_x,
                        y: self.panic_center_y,
                        sector: None,
                        level: 0,
                    });
                    let hiding_time =
                        300 + crate::sim_rng::u32(sim, crate::sim_rng::RngSite::AiPanic, ..200); // AI_MIN + delta
                    self.launch_timer(hiding_time, ctx.frame);
                }
            }

            // ─── Panic-run state machine ────────────────────────────
            // On each arrival (or failed path) we either transition
            // into `FleeingHiding` (panic is spent) or pick a new run
            // direction and `GoTo` along it.
            Substate::FleeingPanic => {
                if stimulus_type != StimulusType::EventReachPoint
                    && stimulus_type != StimulusType::EventCouldntReachPoint
                {
                    return false;
                }

                if self.lasting_panic_runs == 0 {
                    // Panic is over — transition to hiding.
                    self.set_ai_state(AiState::Fleeing);
                    self.current_substate = Substate::FleeingHiding;
                    if self.directed_panic {
                        // Look back at the panic source.
                        self.face_position(Position {
                            x: self.panic_center_x,
                            y: self.panic_center_y,
                            sector: None,
                            level: 0,
                        });
                    } else {
                        // Look in a random direction.
                        self.face_direction(
                            crate::sim_rng::u32(sim, crate::sim_rng::RngSite::AiPanic, 0..16)
                                as u16,
                            ctx,
                        );
                    }
                    self.clear_emoticon();
                    self.set_alert_status(AlertLevel::Yellow);
                    // BlinkEnemy() is wired via refresh_view when the
                    // music alert status changes; nothing to do here
                    // explicitly.
                    let hiding_time = crate::parameters_ai::AI_MIN_PANIC_HIDING_TIME as u32
                        + crate::sim_rng::u32(
                            sim,
                            crate::sim_rng::RngSite::AiPanic,
                            0..crate::parameters_ai::AI_DELTA_PANIC_HIDING_TIME as u32,
                        );
                    self.launch_timer(hiding_time, ctx.frame);
                    return true;
                }

                if stimulus_type == StimulusType::EventReachPoint {
                    // Decrement panic runs and start a new GoTo toward
                    // a fresh escape vector.
                    self.lasting_panic_runs = self.lasting_panic_runs.saturating_sub(1);

                    let sector_index = if !self.directed_panic {
                        // Undirected panic — any direction.
                        (crate::sim_rng::u32(sim, crate::sim_rng::RngSite::AiPanic, 0..16) & 15)
                            as u8
                    } else {
                        // Directed panic — run away from panic center.
                        let dx = ctx.position.x - self.panic_center_x;
                        let dy = ctx.position.y - self.panic_center_y;
                        let base =
                            crate::position_interface::vector_to_sector_0_to_15(dx, dy) as u8;
                        if self.first_try {
                            // ±2 sector jitter around the base.
                            let jitter =
                                (crate::sim_rng::u32(sim, crate::sim_rng::RngSite::AiPanic, 0..5)
                                    as i32
                                    - 2)
                                .rem_euclid(16) as u8;
                            base.wrapping_add(jitter) & 15
                        } else {
                            // Previous attempt failed — rotate 90° to
                            // the side determined by creation-order
                            // parity, with ±3 sector jitter. We key off
                            // the NPC handle's low bit because it's
                            // stable, unique, and has the same parity
                            // effect as the original creation-order
                            // bit.
                            let side = if self.me & 1 != 0 { 4 } else { 12 };
                            let jitter =
                                (crate::sim_rng::u32(sim, crate::sim_rng::RngSite::AiPanic, 0..7)
                                    as i32
                                    - 3)
                                .rem_euclid(16) as u8;
                            base.wrapping_add(side).wrapping_add(jitter) & 15
                        }
                    };

                    let (vx, vy) = crate::element::direction_vector_16(sector_index as i16);
                    let segment = (crate::parameters_ai::AI_MIN_PANIC_RUN_SEGMENT_DISTANCE as u32
                        + crate::sim_rng::u32(
                            sim,
                            crate::sim_rng::RngSite::AiPanic,
                            0..crate::parameters_ai::AI_DELTA_PANIC_RUN_SEGMENT_DISTANCE as u32,
                        )) as f32;
                    let dest = Position {
                        x: ctx.position.x + vx * segment,
                        y: ctx.position.y + vy * segment,
                        sector: ctx.position.sector,
                        level: ctx.position.level,
                    };

                    // Next time around we're no longer on the first try.
                    self.first_try = true;

                    let mut flags = GotoFlags::RUN | GotoFlags::STRAIGHT | GotoFlags::ASK_OBSTACLE;
                    if self.lasting_panic_runs > 0 {
                        flags |= GotoFlags::DONT_STOP;
                    }
                    self.go_to(dest, flags, ctx);
                } else {
                    // EventCouldntReachPoint — the random direction
                    // was blocked. Flip `first_try` so the next run
                    // uses the 90° side-step branch, and queue a
                    // `SeekPoint` fallback for the engine to drain.
                    // The engine has the `seek_points` array; the
                    // `AiController` here doesn't, so we hand off via
                    // `pending_panic_seek_fallback` and let
                    // `process_pending_panic_seek_fallback_for` pick
                    // the anchor + call `go_to` (RUN|DONT_STOP mid-run,
                    // RUN on the last segment). If no seek point is
                    // found, the engine drain re-fires the self
                    // `EventReachPoint` as an emergency fall-through.
                    self.first_try = false;
                    self.outbox.actor.panic_seek_fallback = true;
                }
            }

            Substate::FleeingHiding => {
                if stimulus_type == StimulusType::EventTimer {
                    self.return_to_duty_common_stuff(sim, DutyFlags::empty(), ctx);
                }
            }

            _ => {
                tracing::trace!(
                    "AiController::think_expected_event_common_stuff: unhandled substate {:?}",
                    self.current_substate,
                );
            }
        }

        false
    }

    /// Advance past the current waypoint and continue walking.
    /// Called when a waypoint's command is handled (or skipped).
    fn proceed_on_path(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        hiking_paths: &[crate::level_data::RawHikingPath],
        ctx: &AiContext,
    ) {
        self.set_ai_state(AiState::Default);
        self.current_substate = Substate::DefaultEnroute;

        if let Some(ref mut path) = self.patrol_path {
            // One-waypoint path means "you are already there" — flag
            // `already_on_point` so the outer state machine re-fires
            // `EventReachPoint`. Do *not* advance and don't queue a
            // move.
            if path.size <= 1 {
                self.already_on_point = true;
                return;
            }
            path.advance();
            if let Some(wp) = path.current_waypoint(hiking_paths) {
                // Always pass `GOTO_STRAIGHT` here because macro-to-
                // macro waypoint transitions are straight-line (no
                // path-finder). Without this, the engine's movement
                // layer falls through to the routed direction branch.
                let mut walk_flags = self.default_path_walking_flags | GotoFlags::STRAIGHT;
                if !self.will_stop_at_next_waypoint(sim, hiking_paths) {
                    walk_flags |= GotoFlags::DONT_STOP;
                }
                let dest = Position {
                    x: wp.x as f32,
                    y: wp.y as f32,
                    sector: SectorHandle::new(wp.sector),
                    level: wp.level,
                };
                self.go_to(dest, walk_flags, ctx);
            }
        }
    }

    /// Dispatch an EventDone to ourselves (used when skipping a turn).
    fn think_event_done_on_self(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        ctx: &AiContext,
    ) {
        let done_stimulus = Stimulus::new(StimulusType::EventDone);
        self.think_expected_event_common_stuff(sim, &done_stimulus, ctx);
    }
}

// ---------------------------------------------------------------------------
// Consideration accumulator (replaces module-static accumulators)
// ---------------------------------------------------------------------------

/// Helper for the weighted-attribute decision system. Modelled as an
/// explicit struct rather than module-static accumulators.
#[derive(Debug, Default)]
pub struct ConsiderationAccumulator {
    pub sum_of_values: u32,
    pub sum_of_weights: u32,
    pub sum_of_threshold_values: i32,
    pub sum_of_threshold_weights: u32,
    pub positive_threshold_values: bool,
    pub negative_threshold_values: bool,
}

impl ConsiderationAccumulator {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Add a value to the consideration. `positive_effect` means higher
    /// values favor "yes".
    pub fn consider_value(&mut self, positive_effect: bool, value: u8, weight: u8, threshold: u8) {
        debug_assert!(weight > 0);
        if threshold == 0 {
            let contrib = if positive_effect {
                value as u32
            } else {
                MAX_ATT_VALUE as u32 - value as u32
            };
            self.sum_of_values += contrib * weight as u32;
            self.sum_of_weights += weight as u32;
        } else {
            // Threshold branch: compare the *raw* value (not inverted)
            // against the threshold, and only accumulate if
            // `value > threshold`. The polarity flag is set
            // unconditionally based on `positive_effect`.
            if value > threshold {
                let delta = (value as i32 - threshold as i32) * weight as i32;
                if positive_effect {
                    self.sum_of_threshold_values += delta;
                } else {
                    self.sum_of_threshold_values -= delta;
                }
                self.sum_of_threshold_weights += weight as u32;
            }
            if positive_effect {
                self.positive_threshold_values = true;
            } else {
                self.negative_threshold_values = true;
            }
        }
    }

    /// Evaluate all accumulated considerations and return a value in
    /// 0..100. Initial lambda, threshold correction, clamp, then
    /// consume-and-reset.
    pub fn evaluate(&mut self) -> u8 {
        #[allow(clippy::manual_checked_ops)]
        let mut lambda: i32 = if self.sum_of_weights > 0 {
            (self.sum_of_values / self.sum_of_weights) as i32
        } else if self.positive_threshold_values == self.negative_threshold_values {
            HALF_MAX_ATT_VALUE
        } else if self.positive_threshold_values {
            0
        } else {
            MAX_ATT_VALUE
        };

        if self.sum_of_threshold_weights > 0 {
            let adjusted = self.sum_of_values as i32
                + lambda * self.sum_of_threshold_weights as i32
                + self.sum_of_threshold_values;
            lambda = adjusted / (self.sum_of_weights + self.sum_of_threshold_weights) as i32;
        }

        let result = lambda.clamp(0, MAX_ATT_VALUE) as u8;
        self.reset();
        result
    }
}

// ---------------------------------------------------------------------------
