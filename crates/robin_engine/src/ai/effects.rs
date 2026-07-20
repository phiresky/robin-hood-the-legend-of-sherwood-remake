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

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum AiOwnerWork {
    StateChange(AiStateChangeNotification),
    Speech(AiSpeechAttempt),
    RestoreDetectableObjects { knocked_out_in_money_fight: bool },
    InformResurrection,
    LaunchTimer { frames: u32, current_frame: u32 },
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
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct AiStateChangeNotification {
    pub outgoing_state: AiState,
    pub outgoing_substate: Substate,
    pub incoming_state: AiState,
    pub incoming_substate: Substate,
    pub source: AiStateChangeSource,
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
    pub stop_menace: bool,
    pub lower_shield: bool,
    pub deactivate: bool,
    pub halt: bool,
    pub broadcast_panic: bool,
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
    pub friend_primary_target_swap: Option<(EntityId, HumanHandle)>,
    pub shoot_target: Option<HumanHandle>,
    pub focus: Option<HumanHandle>,
    pub unalert_near_charly_seekers: Option<CharlySeekerTarget>,
    pub refill_bow_ammo: bool,
    pub set_reported_to_officer: Vec<(NpcHandle, bool)>,
    pub unfocus: bool,
    pub focus_point: Option<Position>,
    pub slowly_open_eyes: bool,
    pub forget_nearby_coins: Option<Position>,
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
    pub friend_primary_target_swap: Option<(EntityId, HumanHandle)>,
    pub shoot_target: Option<HumanHandle>,
    pub focus: Option<HumanHandle>,
    pub focus_point: Option<Position>,
    pub unfocus: bool,
    pub set_direction_instantly: Option<i16>,
    pub deactivate: bool,
    pub broadcast_panic: bool,
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
            || self.stop_menace
            || self.lower_shield
            || self.deactivate
            || self.halt
            || self.broadcast_panic
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
            || self.friend_primary_target_swap.is_some()
            || self.shoot_target.is_some()
            || self.focus.is_some()
            || self.unalert_near_charly_seekers.is_some()
            || self.refill_bow_ammo
            || !self.set_reported_to_officer.is_empty()
            || self.unfocus
            || self.focus_point.is_some()
            || self.slowly_open_eyes
            || self.forget_nearby_coins.is_some()
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
        std::mem::take(&mut self.halt)
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
            friend_primary_target_swap: self.friend_primary_target_swap.take(),
            shoot_target: self.shoot_target.take(),
            focus: self.focus.take(),
            focus_point: self.focus_point.take(),
            unfocus: std::mem::take(&mut self.unfocus),
            set_direction_instantly: self.set_direction_instantly.take(),
            deactivate: std::mem::take(&mut self.deactivate),
            broadcast_panic: std::mem::take(&mut self.broadcast_panic),
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
