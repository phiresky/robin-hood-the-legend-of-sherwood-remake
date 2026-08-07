use super::*;

// InitStateSideEffects — entity-side fallout from `AiController::init_state`
// ---------------------------------------------------------------------------

/// Entity-side mutations that [`AiController::init_state`] asks the
/// caller to apply once the AI-side state transition has been
/// committed. The non-AI side effects of `InitState` — posture /
/// action state / eye status / life points / concussion — all live on
/// the entity, not the AI brain.
///
/// The caller (`EngineInner::init_one_ai`) applies these inside a
/// mutable-entity scope after the subclass dispatch returns.
#[derive(Debug, Default, Clone)]
pub struct InitStateSideEffects {
    /// `true` when the caller should run the standard
    /// "walk onto patrol path or launch a bored timer" tail after
    /// applying the side effects. The caller still has to AND this with
    /// `!ai_is_locked() && !ai_is_script_locked()` before actually
    /// calling `ReturnToDuty`.
    pub go_to_duty: bool,
    /// Launch a fresh low-priority actor wait after applying the authored
    /// posture/action state.  The original `InitState` does this for every
    /// non-duty pose; replacing any pre-init idle element is required so its
    /// translated animation uses the new posture.
    pub launch_wait: bool,
    /// New posture — applied via
    /// `PositionInterface::set_posture` (+ a sync write-back to
    /// `ElementData::posture`).
    pub set_posture: Option<crate::element::Posture>,
    /// New action state — applied on `ActorData::action_state`.
    pub set_action_state: Option<crate::element::ActionState>,
    /// New `eye_status` — applied via
    /// `ai_vision::set_view_status`. Set to `Closed` by the
    /// sleeping-upright branch.
    pub set_eye_status: Option<crate::element::EyeStatus>,
    /// Zero out `NpcData::life_points` and flip
    /// `HumanData::killed_by_accident = true`. The two always co-occur
    /// at init.
    pub zero_life_points: bool,
    /// Seed `HumanData::concussion_of_the_brain = CONCUSSION_MAX`
    /// and flip `HumanData::unconscious = true`. Init-time has no
    /// script-lock / tied / carried gates to honour, so we bypass
    /// the full `combat::set_concussion` state machine.
    pub concussion_max_and_unconscious: bool,
}

// ---------------------------------------------------------------------------
// Base AI controller (per-NPC instance state)
// ---------------------------------------------------------------------------

/// Engine-facing effects produced while an AI controller is borrowed.
///
/// The nested owners name the same-frame barrier that consumes each effect.
/// This is intentionally a set of directly mutated queues/options rather than
/// a `derive_builder`, `typed-builder`, or `bon` builder: AI effects are
/// accumulated incrementally by state-machine branches, and an empty outbox is
/// a meaningful value. A builder would either invent defaults for required
/// payloads or hide the barrier and insertion order behind construction
/// boilerplate. Direct constructors for the few multi-field payloads keep the
/// production order visible at the call site.
#[derive(Debug, Default, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct AiOutbox {
    /// Drained by `tick_patrol_coordination` before per-NPC thinking.
    pub patrol: AiPatrolOutbox,
    /// Inputs and accepted-view acknowledgement at the detection/Think edge.
    pub detection: AiDetectionOutbox,
    /// Recursive/cross-NPC work drained at explicit Think return barriers.
    pub reentrant: AiReentrantOutbox,
    /// Entity/sequence mutations applied in Original call order after Think.
    pub actor: AiActorOutbox,
    /// Non-FIT_AGAIN eye repair drained by the dedicated recovery sweep.
    pub recovery: AiRecoveryOutbox,
    /// Music urgency drained by the overall villain-alert sweep.
    pub music: AiMusicOutbox,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct AiPatrolOutbox {
    pub direction_broadcast: Option<u16>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct AiDetectionOutbox {
    pub stimuli: Vec<Stimulus>,
    pub mark_alerted: bool,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct AiReentrantOutbox {
    pub cross_npc_actions: Vec<CrossNpcAction>,
    pub self_stimuli: Vec<StimulusType>,
    /// Finish an outside-Think multi-point patrol macro after its synthetic
    /// `EventReachPoint` recursion has settled. The nested reach-point path
    /// may write a new macro deadline; the outer completion then clears only
    /// the two running flags, matching Original call-stack order.
    pub finish_macro_after_self_stimuli: bool,
    /// Synchronous work produced while the AI owns its call stack.
    ///
    /// `RHArtificialIntelligence::Say` and Enemy/Friendly `SetState` are
    /// immediate calls in the Original. Rust cannot re-enter the engine while
    /// an AI controller is borrowed, so both calls share this FIFO. Keeping
    /// them in one queue preserves statement order at the owner return barrier
    /// instead of rebuilding a frame-global speech batch.
    pub owner_work: Vec<AiOwnerWork>,
    pub waypoint_script_reach_point: Option<(PathId, u8)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub enum AiOwnerWork {
    StateChange(AiStateChangeNotification),
    /// Synchronous `NearbyCiviliansPanic()` engine callback.  It shares the
    /// owner FIFO because callers can speak or change state immediately
    /// before/after it and those operations are observably ordered.
    NearbyCiviliansPanic,
    /// Continue the common `EVENT_REACHPOINT` route handler after its
    /// virtual `SetState` callback has returned and committed.
    ResumeGotoRouteReachPoint {
        /// Positions visible at the Original owner boundary where
        /// `EVENT_REACHPOINT` was dispatched. Rust moves actors in a global
        /// batch, so rebuilding these from the live world after the
        /// `FilterAIEvent` callback would expose later legacy slots one
        /// movement phase too early.
        owner_boundary_positions: Vec<(u32, Position)>,
    },
    /// Continue Enemy `ReturnToDuty` after its synchronous
    /// `InitializePatrol` engine callback has completed.
    ResumeReturnToDutyAfterPatrolInit {
        flags: DutyFlags,
        /// Original evaluates patrol geometry at this owner's legacy slot;
        /// later Rust entity slots may already have moved when the owner FIFO
        /// reaches the engine boundary.
        owner_boundary_positions: Vec<(u32, Position)>,
    },
    /// Continue `CMD_PATROL_START` after its inline `InitializePatrol` call.
    ResumeMacroAfterPatrolInit {
        owner_boundary_positions: Vec<(u32, Position)>,
    },
    Speech(AiSpeechAttempt),
    RestoreDetectableObjects {
        knocked_out_in_money_fight: bool,
    },
    InformResurrection,
    LaunchTimer {
        frames: u32,
        current_frame: u32,
    },
    SetEyeStatus(crate::element::EyeStatus),
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct AiSpeechAttempt {
    pub remark: Remark,
    pub flags: u16,
}

/// One owner-local `SetState` script notification.
///
/// The AI method has to finish its pure-Rust tail before releasing its entity
/// borrow, so the engine records both sides of the transition. The callback
/// barrier temporarily restores `outgoing_*`, invokes `FilterAIEvent`, then
/// re-resolves the typed AI owner and commits `incoming_*`.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct AiStateChangeNotification {
    pub outgoing_state: AiState,
    pub outgoing_substate: Substate,
    pub incoming_state: AiState,
    pub incoming_substate: Substate,
    pub source: AiStateChangeSource,
    /// Actor effects issued before the corresponding Original `SetState`
    /// call. The live actor outbox then contains only statements executed
    /// after SetState returned, which must remain hidden until the
    /// synchronous script callback has completed.
    pub actor_effects_before_callback: Option<AiActorOutbox>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct AiRecoveryOutbox {
    pub inform_resurrection: bool,
    pub set_eye_status: Option<crate::element::EyeStatus>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct AiMusicOutbox {
    pub instant_change: bool,
}

/// Named, serializable payload for the attentive-mode barrier. This is a
/// deliberately local replacement for the opaque
/// `(target, fast_officer_variant)` tuple.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct AttentiveModeEffect {
    pub target: bool,
    pub fast_officer_variant: bool,
}

/// Typed PC relationship delta emitted by `EnemyAi::set_guarded_pc`.
/// `None` is the original null-pointer case; using `PcId` prevents an NPC or
/// object handle from entering this PC-only relationship channel.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct GuardedPcEffect {
    pub old: Option<crate::entity_id::PcId>,
    pub new: Option<crate::entity_id::PcId>,
}

/// Typed location of an owned shooting point in the global archery tables.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct ReservedShootingPoint {
    pub sector_index: u16,
    pub point_index: crate::sector::ArcheryPointIdx,
}

impl From<(u16, u16)> for ReservedShootingPoint {
    fn from((sector_index, point_index): (u16, u16)) -> Self {
        Self {
            sector_index,
            point_index: point_index.into(),
        }
    }
}

/// Archery ownership work consumed at the post-refill/pre-unalert actor
/// barrier in `EngineInner::drain_pending_for_npc`.
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub struct ArcheryReservationRelease {
    pub shooting_point: Option<ReservedShootingPoint>,
    pub release_sector: bool,
}

impl AttentiveModeEffect {
    pub const fn new(target: bool, fast_officer_variant: bool) -> Self {
        Self {
            target,
            fast_officer_variant,
        }
    }
}

/// Effects consumed by `EngineInner::drain_pending_for_npc`.
///
/// Fields remain separated where the engine deliberately re-enters AI between
/// applications. The `take_*` methods below are the ordered drain API; callers
/// do not manually clear the underlying channels.
#[derive(Debug, Default, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct AiActorOutbox {
    pub orders: Vec<AiOrderIntent>,
    pub quit_swordfight: bool,
    /// Complete the lost-enemy battle-overview continuation only after an
    /// explicit QUIT_SWORDFIGHT launch has delivered interruption
    /// condolations from the command it replaced.
    #[serde(default)]
    pub lost_enemy_overview_after_quit: bool,
    pub stop_menace: bool,
    pub lower_shield: bool,
    pub deactivate: bool,
    pub halt: bool,
    /// Additional synchronous `Halt()` calls coalesced into this deferred
    /// outbox. The original applies every call, which matters when the first
    /// stop rewrites a movement into a waiting transition and the second stop
    /// then interrupts that transition.
    #[serde(default)]
    pub additional_halts: u8,
    /// Preserve the interrupted movement goal across the next Halt because
    /// an AI-issued RaiseShield command immediately takes ownership.
    pub preserve_goal_for_raise_shield: bool,
    pub blink_all_enemies: bool,
    pub enemy_in_house_alert: bool,
    pub add_detectables: Vec<(crate::element::EntityId, crate::element::DetectableType)>,
    pub delete_detectables: Vec<crate::element::DetectableType>,
    pub delete_detectable_entity: Vec<(crate::element::EntityId, crate::element::DetectableType)>,
    pub delete_beggar_for_all_npc: Vec<crate::element::EntityId>,
    pub enter_swordfight: Option<EnterSwordfightRequest>,
    pub enter_swordfight_jump_line: Option<u32>,
    pub stop_target: Option<HumanHandle>,
    pub set_principal: Option<HumanHandle>,
    pub friend_primary_target_swaps: Vec<(EntityId, HumanHandle)>,
    pub shoot_target: Option<HumanHandle>,
    /// Raw element-table handle passed to Original `Focus(RHElement*)`.
    /// Unlike combat targets, this may name an object (for example an ale
    /// bottle that an NPC is considering picking up).
    pub focus: Option<ElementHandle>,
    pub unalert_near_charly_seekers: Option<CharlySeekerTarget>,
    pub refill_bow_ammo: bool,
    pub set_reported_to_officer: Vec<(NpcHandle, bool)>,
    pub unfocus: bool,
    pub focus_point: Option<Position>,
    pub slowly_open_eyes: bool,
    pub forget_nearby_coins: Option<Position>,
    pub set_direction: Option<i16>,
    pub set_direction_instantly: Option<i16>,
    pub set_attentive_mode: Option<AttentiveModeEffect>,
    pub set_guarded_pc: Option<GuardedPcEffect>,
    pub launch_commands: Vec<crate::element::Command>,
    pub launch_on_target: Vec<(NpcHandle, crate::element::Command)>,
    pub launch_sequences: Vec<crate::sequence::Sequence>,
    pub look_sidewards: Option<LookDirection>,
    pub posture: Option<crate::element::Posture>,
    pub begin_panic: Option<PanicRequest>,
    pub panic_seek_fallback: bool,
    pub script_seek_area: Option<ScriptSeekAreaRequest>,
    pub archery_reservation_release: ArcheryReservationRelease,
}

impl AiActorOutbox {
    /// Queue an owner-local `SetAttentiveMode` call without replacing an
    /// earlier request for the same target state.
    ///
    /// Original updates `will_be_attentive` synchronously when the first
    /// call launches its transition element. A later call with the same
    /// target therefore returns immediately, leaving the first call's
    /// officer-fast choice authoritative. Rust drains this outbox after the
    /// AI borrow is released, so preserve that ordering explicitly.
    pub fn queue_set_attentive_mode(&mut self, request: AttentiveModeEffect) {
        if self
            .set_attentive_mode
            .is_some_and(|pending| pending.target == request.target)
        {
            return;
        }
        self.set_attentive_mode = Some(request);
    }

    /// Queue one synchronous actor `Halt()` without losing multiplicity.
    pub fn queue_halt(&mut self) {
        if self.halt {
            self.additional_halts = self
                .additional_halts
                .checked_add(1)
                .expect("too many actor Halt calls in one AI drain");
        } else {
            self.halt = true;
        }
    }

    /// Queue `Focus(element)` with Original's synchronous last-write-wins
    /// semantics. A Think call can issue `Focus(NULL)` and then focus a new
    /// target before the deferred engine drain.
    pub fn set_focus(&mut self, target: ElementHandle) {
        // `Focus(element)` with a null element is `Unfocus()` in the
        // Original; handle 0 is the AI's null-target sentinel.
        if target == 0 {
            self.set_unfocus();
            return;
        }
        self.focus = Some(target);
        self.focus_point = None;
        self.unfocus = false;
    }

    /// Queue `Focus(position)` and supersede any earlier focus operation from
    /// the same synchronous Think call.
    pub fn set_focus_point(&mut self, point: Position) {
        self.focus = None;
        self.focus_point = Some(point);
        self.unfocus = false;
    }

    /// Queue `Focus(NULL)` and supersede any earlier focus operation from the
    /// same synchronous Think call.
    pub fn set_unfocus(&mut self) {
        self.focus = None;
        self.focus_point = None;
        self.unfocus = true;
    }
}

#[derive(Debug, Default)]
pub(crate) struct AiActorPreemptionEffects {
    pub stop_menace: bool,
    pub lower_shield: bool,
}

#[derive(Debug, Default)]
pub(crate) struct AiActorCoreEffects {
    pub quit_swordfight: bool,
    pub enter_swordfight: Option<EnterSwordfightRequest>,
    pub enter_swordfight_jump_line: Option<u32>,
    pub stop_target: Option<HumanHandle>,
    pub set_principal: Option<HumanHandle>,
    pub friend_primary_target_swaps: Vec<(EntityId, HumanHandle)>,
    pub shoot_target: Option<HumanHandle>,
    pub focus: Option<ElementHandle>,
    pub focus_point: Option<Position>,
    pub unfocus: bool,
    pub set_direction_instantly: Option<i16>,
    pub deactivate: bool,
    pub launch_commands: Vec<crate::element::Command>,
    pub launch_on_target: Vec<(NpcHandle, crate::element::Command)>,
    pub launch_sequences: Vec<crate::sequence::Sequence>,
    pub look_sidewards: Option<LookDirection>,
    pub add_detectables: Vec<(crate::element::EntityId, crate::element::DetectableType)>,
    pub delete_detectables: Vec<crate::element::DetectableType>,
    pub delete_detectable_entities: Vec<(crate::element::EntityId, crate::element::DetectableType)>,
    pub slowly_open_eyes: bool,
    pub posture: Option<crate::element::Posture>,
}

impl AiActorOutbox {
    /// Whether an owner-local synchronous drain has more actor work to apply.
    /// Speech intentionally lives outside this outbox. State-script
    /// notifications and ordered engine calls live in the sibling
    /// `AiReentrantOutbox` queue and the owner fixed-point predicates check
    /// them separately.
    pub(crate) fn has_boundary_work(&self) -> bool {
        !self.orders.is_empty()
            || self.quit_swordfight
            || self.lost_enemy_overview_after_quit
            || self.stop_menace
            || self.lower_shield
            || self.deactivate
            || self.halt
            || self.blink_all_enemies
            || self.enemy_in_house_alert
            || !self.add_detectables.is_empty()
            || !self.delete_detectables.is_empty()
            || !self.delete_detectable_entity.is_empty()
            || !self.delete_beggar_for_all_npc.is_empty()
            || self.enter_swordfight.is_some()
            || self.enter_swordfight_jump_line.is_some()
            || self.stop_target.is_some()
            || self.set_principal.is_some()
            || !self.friend_primary_target_swaps.is_empty()
            || self.shoot_target.is_some()
            || self.focus.is_some()
            || self.unalert_near_charly_seekers.is_some()
            || self.refill_bow_ammo
            || !self.set_reported_to_officer.is_empty()
            || self.unfocus
            || self.focus_point.is_some()
            || self.slowly_open_eyes
            || self.forget_nearby_coins.is_some()
            || self.set_direction.is_some()
            || self.set_direction_instantly.is_some()
            || self.set_attentive_mode.is_some()
            || self.set_guarded_pc.is_some()
            || !self.launch_commands.is_empty()
            || !self.launch_on_target.is_empty()
            || !self.launch_sequences.is_empty()
            || self.look_sidewards.is_some()
            || self.posture.is_some()
            || self.begin_panic.is_some()
            || self.panic_seek_fallback
            || self.script_seek_area.is_some()
            || self.archery_reservation_release != ArcheryReservationRelease::default()
    }

    /// Drain actor halt alone. Its application can re-enter engine sequence
    /// handling, so the later movement-prefix barrier must not be taken yet.
    pub(crate) fn take_halt(&mut self) -> bool {
        self.additional_halts = 0;
        std::mem::take(&mut self.halt)
    }

    /// Drain every synchronous `Halt()` accumulated before this boundary.
    pub(crate) fn take_halt_count(&mut self) -> u8 {
        if !std::mem::take(&mut self.halt) {
            debug_assert_eq!(self.additional_halts, 0);
            return 0;
        }
        1u8.checked_add(std::mem::take(&mut self.additional_halts))
            .expect("actor Halt count overflow")
    }

    pub(crate) fn take_preserve_goal_for_raise_shield(&mut self) -> bool {
        std::mem::take(&mut self.preserve_goal_for_raise_shield)
    }

    pub(crate) fn take_lost_enemy_overview_after_quit(&mut self) -> bool {
        std::mem::take(&mut self.lost_enemy_overview_after_quit)
    }

    /// Drain a direct `SetDirection` write before the following `StopAll`
    /// barrier. Unlike Face/Turn, this only changes the direction goal.
    pub(crate) fn take_direction_goal(&mut self) -> Option<i16> {
        self.set_direction.take()
    }

    /// Drain the two movement prefixes after halt has been applied.
    pub(crate) fn take_movement_prefixes(&mut self) -> AiActorPreemptionEffects {
        AiActorPreemptionEffects {
            stop_menace: std::mem::take(&mut self.stop_menace),
            lower_shield: std::mem::take(&mut self.lower_shield),
        }
    }

    /// Drain the first contiguous post-Think application barrier.
    pub(crate) fn take_core(&mut self) -> AiActorCoreEffects {
        AiActorCoreEffects {
            quit_swordfight: std::mem::take(&mut self.quit_swordfight),
            enter_swordfight: self.enter_swordfight.take(),
            enter_swordfight_jump_line: self.enter_swordfight_jump_line.take(),
            stop_target: self.stop_target.take(),
            set_principal: self.set_principal.take(),
            friend_primary_target_swaps: std::mem::take(&mut self.friend_primary_target_swaps),
            shoot_target: self.shoot_target.take(),
            focus: self.focus.take(),
            focus_point: self.focus_point.take(),
            unfocus: std::mem::take(&mut self.unfocus),
            set_direction_instantly: self.set_direction_instantly.take(),
            deactivate: std::mem::take(&mut self.deactivate),
            launch_commands: std::mem::take(&mut self.launch_commands),
            launch_on_target: std::mem::take(&mut self.launch_on_target),
            launch_sequences: std::mem::take(&mut self.launch_sequences),
            look_sidewards: self.look_sidewards.take(),
            add_detectables: std::mem::take(&mut self.add_detectables),
            delete_detectables: std::mem::take(&mut self.delete_detectables),
            delete_detectable_entities: std::mem::take(&mut self.delete_detectable_entity),
            slowly_open_eyes: std::mem::take(&mut self.slowly_open_eyes),
            posture: self.posture.take(),
        }
    }

    /// Drain archery ownership work only at its original application point,
    /// after bow-ammo refill and before the Charly-seeker broadcast barrier.
    pub(crate) fn take_archery_reservation_release(&mut self) -> ArcheryReservationRelease {
        std::mem::take(&mut self.archery_reservation_release)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_attentive_target_preserves_first_transition_variant() {
        let mut effects = AiActorOutbox::default();

        effects.queue_set_attentive_mode(AttentiveModeEffect::new(false, true));
        effects.queue_set_attentive_mode(AttentiveModeEffect::new(false, false));

        let pending = effects
            .set_attentive_mode
            .expect("first attentive-mode request remains queued");
        assert!(!pending.target);
        assert!(pending.fast_officer_variant);
    }

    #[test]
    fn focus_operations_are_last_write_wins() {
        let mut effects = AiActorOutbox::default();

        effects.set_unfocus();
        effects.set_focus(17);
        assert_eq!(effects.focus, Some(17));
        assert_eq!(effects.focus_point, None);
        assert!(!effects.unfocus);

        let point = Position {
            x: 12.0,
            y: 34.0,
            ..Position::default()
        };
        effects.set_focus_point(point);
        assert_eq!(effects.focus, None);
        assert_eq!(effects.focus_point, Some(point));
        assert!(!effects.unfocus);

        effects.set_unfocus();
        assert_eq!(effects.focus, None);
        assert_eq!(effects.focus_point, None);
        assert!(effects.unfocus);
    }
}
