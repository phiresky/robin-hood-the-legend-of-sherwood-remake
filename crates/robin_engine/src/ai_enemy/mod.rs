//! Enemy (villain/soldier) AI.
//!
//! This module contains the `EnemyAi` struct which extends [`AiController`]
//! with soldier-specific state: combat tactics, seek behavior, officer/archer
//! specializations, money fights, and the massive Think state machine.

mod battle;
pub(crate) use battle::increment_battle_target_multiplicity;
pub(crate) use battle::rider_charge_goal_geometry;
pub(crate) use battle::{
    BattleDecisionInputs, battle_friend_is_nearer, battle_owner_target_square_distance,
};
mod combat_positions;
mod detection;
pub(crate) use combat_positions::{SwordfightLists, is_facing_swordfight_target};
pub(crate) use combat_positions::{combat_neighbour_distance_ulong, drunk_combat_freezes};
pub(crate) use map_vec_ext::AiMapVec;
pub(crate) use util::{CombatFighterAccess, evaluate_combat_position_full};
mod event_handlers;
pub(crate) use detection::{Target180, Viewer180, detects_180_degrees_live};
mod map_vec_ext;
mod parity_trace;
mod seek;
pub(crate) use seek::SeekAreaSpec;
mod substate_handlers;
mod util;

pub use util::*;

use crate::ai::*;
use crate::entity_id::PcId;
#[cfg(test)]
use crate::parameters_ai;
use crate::sim_rng::SimulationContext;

/// Master switch for the opt-in AI decision/path diagnostic used by the
/// Save020/Save055 substate-only parity cohort. Keep this check separate so
/// disabled runs return before reading frame, owner, AI, or geometry state.
pub(crate) fn decision_path_debug_enabled() -> bool {
    decision_path_debug_gate().enabled()
}

fn decision_path_debug_gate() -> &'static crate::engine::diagnostics::ParityGate<2> {
    static GATE: std::sync::OnceLock<crate::engine::diagnostics::ParityGate<2>> =
        std::sync::OnceLock::new();
    GATE.get_or_init(|| {
        crate::ai::parity_gate::required_parity_gate(
            "PARITY_DEBUG_AI_DECISION_PATH",
            [
                "PARITY_DEBUG_AI_DECISION_PATH_FRAME",
                "PARITY_DEBUG_AI_DECISION_PATH_OWNER",
            ],
        )
    })
}

pub(crate) fn decision_path_debug_matches_raw(frame: u32, owner: u32) -> bool {
    decision_path_debug_gate().matches_required([Some(frame), Some(owner)])
}

/// Master switch for the opt-in primary-target selection/swap diagnostic.
///
/// Keep this separate from [`primary_swap_debug_matches`] so every call site
/// can return before reading AI/entity state when diagnostics are disabled.
pub(crate) fn primary_swap_debug_enabled() -> bool {
    primary_swap_debug_gate().enabled()
}

fn primary_swap_debug_gate() -> &'static crate::engine::diagnostics::ParityGate<2> {
    static GATE: std::sync::OnceLock<crate::engine::diagnostics::ParityGate<2>> =
        std::sync::OnceLock::new();
    GATE.get_or_init(|| {
        crate::ai::parity_gate::required_parity_gate(
            "PARITY_DEBUG_PRIMARY_SWAP",
            [
                "PARITY_DEBUG_PRIMARY_SWAP_FRAME",
                "PARITY_DEBUG_PRIMARY_SWAP_OWNER",
            ],
        )
    })
}

/// Apply the required exact frame/owner gate for primary-target diagnostics.
/// Invalid or incomplete enabled configurations fail loudly rather than
/// accidentally producing a broad trace.
pub(crate) fn primary_swap_debug_matches(frame: u32, owner: HumanHandle) -> bool {
    primary_swap_debug_gate().matches_required([Some(frame), Some(owner)])
}
#[cfg(test)]
use crate::position_interface::ASPECT_RATIO;

// ---------------------------------------------------------------------------
// EnemyAi — extends AiController with soldier-specific state
// ---------------------------------------------------------------------------

/// Enemy/soldier AI state. Extends [`AiController`] with villain-specific
/// fields.
///
/// Serde persists every field; `#[serde(default)]` fields may be absent
/// from older saves. See [`crate::ai::persisted`].
#[derive(
    Debug,
    Clone,
    Default,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct EnemyAi {
    /// Base AI controller (contains all common state).
    pub base: AiController,

    /// True while this soldier has a pending/in-flight special-strike
    /// sequence (prep wait + strike animation). Mirrors the observable
    /// `Substate::AttackingSwordfightSpecialStrike` while also giving
    /// cancellation reconciliation a direct sequence-lifecycle latch. Set by
    /// the live special-strike entry; cleared by per-tick reconciliation in
    /// `engine/melee.rs::tick_enemy_sword_attacks` when the sequence
    /// manager no longer has an active sword-strike element for this
    /// actor (covers both natural completion and interruption).
    pub pending_special_strike: bool,

    /// One-shot handoff from swordfight reconsideration to the engine-side
    /// strike proposer. The original game only proposes a good sword strike
    /// when that event-driven reconsideration reaches its decision tail;
    /// merely entering the swordfight substate must not authorize a draw.
    #[serde(default)]
    pub pending_sword_strike_consideration: bool,

    /// AI decisions reached the combat-insult step after swordfight reconsideration,
    /// but the engine-side strike proposer has not yet settled the one-shot
    /// consideration. Original proposes inline: a successful proposal
    /// changes to `...SPECIAL_STRIKE` and suppresses the insult, while a
    /// rejected proposal leaves `...SWORDFIGHT` and says it. The Rust port
    /// settles this latch immediately after `Think`, at the same owner
    /// boundary as `pending_sword_strike_consideration`.
    #[serde(default)]
    pub pending_combat_insult_after_strike_consideration: bool,

    // -- Private fields --
    #[serde(default, with = "crate::ai::optional_ai_handle")]
    pub missed_pc: Option<AiEntityHandle>,
    pub pc_missed: bool,
    pub pc_gone_away_in_this_direction: u16,
    /// Frame when Charly was last observed missing.
    pub frame_when_missed_charly: u32,
    /// Net objects whose sound/event this soldier has already processed.
    pub heard_nets: Vec<ObjectHandle>,
    /// Last position at which an unexplained stimulus was detected.
    pub detected_something_there: Position,
    /// The current heard-steps route was caused by an intentional stone
    /// distraction and therefore uses a running approach. This is explicit
    /// serialized AI memory: save/load and rollback must not infer it from a
    /// transient sound side effect.
    #[serde(default)]
    pub investigating_distraction: bool,
    /// Cursor into the directions of the currently examined seek point.
    pub last_seek_direction_index: u8,
    #[serde(default, with = "crate::ai::optional_ai_handle")]
    pub beggar_to_examine: Option<AiEntityHandle>,

    pub current_task_priority: u16,
    pub minimal_task_priority: u16,
    pub new_task_priority: u16,
    /// Distinct detection checkpoints still involved in the current sweep.
    pub number_of_different_checkpoints: u8,
    /// Whether this soldier is willing to interrupt duty to take ale.
    pub thirsty: bool,
    /// Original test/script latch preventing combat-position changes.
    pub position_change_locked_for_test: bool,

    pub other_bodies_to_examine: Vec<HumanHandle>,
    pub beggars_to_control: Vec<HumanHandle>,
    pub positions_of_beggars_to_control: Vec<Position>,
    pub seen_dead_body: bool,
    pub seeking_charly: bool,

    // -- Seeking --
    /// IDs of seek points to visit. Mix of global (index into
    /// AiGlobalState::seek_points) and personal (1111, 2222 sentinels).
    pub my_seek_points: Vec<u16>,
    /// Personal seek point created at the seek center (inserted at front).
    /// ID = 1111.
    pub personal_seek_point_1: Option<SeekPoint>,
    /// Personal seek point created at the seek center (inserted at back).
    /// ID = 2222.
    pub personal_seek_point_2: Option<SeekPoint>,
    pub seek_center: Position,
    /// ID of the currently examined seek point (for lock/unlock).
    pub actual_seek_point: Option<u16>,
    pub seek_point_view_directions: Vec<u16>,
    pub seek_flags: SeekFlags,

    pub old_odds: i16,

    pub gather_position: Position,
    pub gather_direction: u16,
    pub gather_position_instructed: bool,
    pub search_charly_way: Vec<Position>,
    pub officers_position: Position,

    /// Raw serialized storage for the original game's previous state.
    ///
    /// Original-game initialization leaves this field indeterminate and still
    /// serializes all four bytes. It only becomes semantically live after
    /// PC-sighting processing assigns it together with
    /// `previous_substate`. Stored as the raw word (wire-identical to `i32`);
    /// read it with [`StoredEnumWord::get`].
    pub previous_state: crate::ai::StoredEnumWord<AiState>,
    /// Raw serialized storage for the original game's previous substate; see
    /// [`Self::previous_state`].
    pub previous_substate: crate::ai::StoredEnumWord<Substate>,

    pub reported_to_officer: bool,

    pub missed_soldier_timer: u16,
    pub old_money: u16,

    pub other_seen_money: Vec<ObjectHandle>,
    pub other_seen_ale: Vec<ObjectHandle>,

    pub money_fight_enemies: Vec<NpcHandle>,
    pub money_fight_victims: Vec<NpcHandle>,

    // Archer / shield bearer (serialized by semantic entity reference)
    #[serde(default, with = "crate::ai::optional_ai_handle")]
    pub archer_behind_me: Option<AiEntityHandle>,
    #[serde(default, with = "crate::ai::optional_ai_handle")]
    pub shield_bearer_before_me: Option<AiEntityHandle>,

    pub shield_bearer_direction: u16,
    pub phalanx_aborted: bool,

    pub changed_to_alert_path: bool,

    pub already_seen_bodies: Vec<HumanHandle>,

    /// Soldiers this officer has called to a group.
    /// Populated by `alert_soldiers`, read by group coordination substates.
    pub alerted_us: Vec<HumanHandle>,

    // Archery
    /// This NPC's reserved shooting point, as `(archery_sector_idx,
    /// point_idx)`.  We store the indices (rather than a pointer) so
    /// the prior slot is always recoverable when
    /// `set_my_shooting_point` releases it.
    pub my_shooting_point: Option<(u16, u16)>,
    /// Index into `AiGlobalState::archery_sectors` for this NPC's
    /// assigned archery sector.
    pub my_archery_sector: Option<u16>,
    pub my_archery_sector_index: u16,
    pub my_archery_point_index: crate::sector::ArcheryPointIdx,
    pub my_archery_point_increment: i8,

    pub enemy_seen_below: bool,
    pub enemy_had_this_elevation: u16,

    // Known enemy strikes (swordfight pattern recognition).
    // We use Option<SwordStrike> (None = empty slot) for type safety.
    pub known_enemy_strike_1: Option<crate::weapons::SwordStrike>,
    pub known_enemy_strike_2: Option<crate::weapons::SwordStrike>,
    pub known_enemy_strike_3: Option<crate::weapons::SwordStrike>,

    pub return_to_patrol_point: Position,

    pub fleeing_seen_enemy_counter: u16,

    pub last_stimulus_dispatched_to_patrol: Option<Stimulus>,

    // -- Protected fields --
    /// Character ID cached from the soldier profile at level load.
    pub character_id: u32,

    pub old_life_points: u8,
    pub initial_life_points: u8,

    /// Enemy list in the current battle.
    pub list_them: Vec<HumanHandle>,

    pub ambush_point_array_reset: bool,
    pub ambush_point_status: Vec<AmbushPointStatus>,

    pub forced_next_battle_decision: Decision,
    pub reset_battle_decision: bool,

    // Cached scalars from `SoldierProfile` — denormalised at level
    // load so AI ticks never touch the profile table during mutable
    // entity iteration.  If you add a new field here, populate it
    // from `engine::level_loading::init_enemy_ai_from_profile`.
    pub soldier_profile_iq: u16,
    pub soldier_profile_courage: u16,
    /// Cached shooting skill — used by
    /// [`Self::get_shooting_ability`] (the `AIMING_TIME_FORMULA`
    /// driver).  Pulled from the soldier profile at level load.
    pub soldier_profile_shooting: u16,
    /// Cached VIP flag from soldier profile — VIP soldiers can only attack Robin.
    pub soldier_profile_vip: bool,
    pub soldier_profile_bee_time: u16,
    /// Cached pride value from soldier profile — determines whether
    /// this NPC considers themselves "too proud to attack" when
    /// soldiers with lower pride are nearby.
    pub soldier_profile_pride: u16,
    /// Cached hearing factor from soldier profile — multiplier for
    /// noise volume when checking acoustic detection.
    pub soldier_profile_hearing_factor: f32,
    pub soldier_profile_rank: ProfileRank,
    /// Cached initiative — used by
    /// `Q_SHALL_I_SEEK_BEFORE_ALERTING_*` and `Q_SHALL_I_SEND_OUT_SOLDIER`.
    pub soldier_profile_initiative: u16,
    /// Cached beer count — used by `Q_SHALL_I_TAKE_ALE`.
    pub soldier_profile_beer: u16,
    /// Cached eligibility for the optional zero-beer reliability rule. This
    /// is true only for a non-VIP soldier while the authoritative setting is
    /// enabled, so live menu commands affect spawned AI on the same frame.
    #[serde(default)]
    pub ale_reliable_distraction: bool,
    /// Cached money count — used by `Q_SHALL_I_TAKE_MONEY`
    /// and `Q_SHALL_I_FIGHT_FOR_MONEY`.
    pub soldier_profile_money: u16,
    /// Cached apple count — used by `Q_SHALL_I_REACT_ON_APPLE`.
    pub soldier_profile_apple: u16,
    /// Cached whistle count — used by `Q_SHALL_I_LOOK_WHISTLE`
    /// and `Q_SHALL_I_FOLLOW_WHISTLE`.
    pub soldier_profile_whistle: u16,
    /// Cached duty flag — used by several questions to prevent on-duty
    /// soldiers from wandering after stimuli.
    pub soldier_profile_duty: bool,
    /// Cached endurance — used by `Q_SHALL_I_RUN`.
    pub soldier_profile_endurance: u16,
    /// Whether this soldier is a VIP (mission-critical NPC). Cached
    /// from the soldier profile at level load.
    pub is_vip: bool,
    /// Default sword range for the soldier's weapon — pulled from
    /// `HtHWeaponProfile::distance[Default]` at level load.
    pub sword_range: u16,
    /// Cached HtH weapon profile id (index into
    /// `Profiles::hth_weapons`) — needed by the engine sword combat
    /// tick to look up the full weapon profile when applying damage.
    /// Pulled from `SoldierProfile::hth_weapon_id` at level load.
    pub hth_weapon_id: u32,
    /// Cached charge-weapon flag, gating the substate-derived
    /// charge-from-reactiontime branch in enemy approach reconsideration.
    /// Pulled from the weapon profile at level load.
    pub sword_is_charge_weapon: bool,
    /// Universal-frame counter when this soldier is next allowed to
    /// throw a sword strike.  Lets the engine sword-combat tick
    /// space attacks 1+ second apart instead of dealing damage every
    /// frame.  Collapsed into a single per-soldier cooldown rather
    /// than per-strike-sequence-element budgets.
    pub next_sword_strike_frame: u32,

    pub company_number: u16,
    #[serde(default, with = "crate::ai::optional_ai_handle")]
    pub left_combat_neighbour: Option<AiEntityHandle>,
    #[serde(default, with = "crate::ai::optional_ai_handle")]
    pub right_combat_neighbour: Option<AiEntityHandle>,

    pub attentive: bool,
    pub will_be_attentive: bool,
    pub forced_attentive: bool,

    /// PC this NPC is guarding. `None` is the original-game missing-reference case.
    pub guarded_pc: Option<PcId>,

    pub my_line_jump: Option<u32>,

    pub tower_guard: bool,
    pub combat_trainer: bool,

    // -- Added for Rust port --
    /// Whether this NPC is an archer (set during initialization from entity data).
    pub is_archer_unit: bool,
}

impl AiRole for EnemyAi {
    fn base_mut(&mut self) -> &mut AiController {
        &mut self.base
    }

    /// Soldier alert setter: threads the forced-attentive view override.
    fn role_set_alert_status(&mut self, level: AlertLevel) {
        EnemyAi::set_alert_status(self, level);
    }
}

impl EnemyAi {
    /// Unwrap a field the current substate requires to be set, panicking with
    /// the owner, the field (`what`, e.g. "an antagonist") and the substate
    /// `context` otherwise.
    #[track_caller]
    fn required<T>(&self, value: Option<T>, what: &'static str, context: &'static str) -> T {
        value.unwrap_or_else(|| panic!("enemy AI {} requires {what} while {context}", self.base.me))
    }

    /// Clear both combat-neighbour links and synchronously request the two
    /// reciprocal clears performed by soldier-actor state changes.
    ///
    /// Keeping this as one operation matters: a one-sided stale link can be
    /// consumed by a later phalanx insertion and detach an otherwise valid
    /// formation chain.
    pub(crate) fn clear_combat_neighbours(&mut self) {
        if let Some(left) = self.left_combat_neighbour {
            self.base.outbox.reentrant.cross_npc_actions.push(
                CrossNpcAction::SetRightCombatNeighbour {
                    target: left.get(),
                    neighbour: None,
                },
            );
        }
        if let Some(right) = self.right_combat_neighbour {
            self.base.outbox.reentrant.cross_npc_actions.push(
                CrossNpcAction::SetLeftCombatNeighbour {
                    target: right.get(),
                    neighbour: None,
                },
            );
        }
        self.left_combat_neighbour = None;
        self.right_combat_neighbour = None;
    }

    pub fn new(owner: NpcHandle) -> Self {
        // The derived malignity constructor overrides two fields after
        // the base-class defaults: `attitude = Hostile` and
        // `reset_battle_decision = true`.
        let mut base = AiController::new(owner);
        base.attitude = Attitude::Hostile;
        Self {
            base,
            reset_battle_decision: true,
            // Original base-class constructor values that differ from the
            // zero/empty `Default` of their field types.
            thirsty: true,
            previous_state: crate::ai::StoredEnumWord::new(AiState::Default),
            previous_substate: crate::ai::StoredEnumWord::new(Substate::DefaultOnPost),
            soldier_profile_iq: 50,
            soldier_profile_courage: 50,
            soldier_profile_shooting: 50,
            sword_range: 40, // default before profile lookup
            soldier_profile_hearing_factor: 1.0,
            soldier_profile_initiative: 50,
            ..Default::default()
        }
    }

    /// Soldier-side wrapper for `AiController::set_alert_status_with_flags`.
    ///
    /// Threads `self.forced_attentive` into the view-override
    /// (Green music ⇒ Yellow view for forced-attentive soldiers).  Use
    /// this in place of `self.base.set_alert_status(level)` from any
    /// soldier-side path so the view field stays correct.
    pub fn set_alert_status(&mut self, level: crate::ai::AlertLevel) {
        self.base.set_alert_status_with_flags(
            level,
            crate::ai::AlertFlags::empty(),
            self.forced_attentive,
        );
    }

    /// Soldier-side flag-aware setter — same as `set_alert_status` but
    /// honours `ALERT_INSTANT_MUSIC_CHANGE` / `ALERT_ONLY_MUSIC`.
    pub fn set_alert_status_with_flags(
        &mut self,
        level: crate::ai::AlertLevel,
        flags: crate::ai::AlertFlags,
    ) {
        self.base
            .set_alert_status_with_flags(level, flags, self.forced_attentive);
    }

    // -----------------------------------------------------------------------
    // Public accessors
    // -----------------------------------------------------------------------

    pub(crate) fn iq_for_difficulty(
        &self,
        difficulty: crate::player_profile::DifficultyLevel,
        hostile_to_player: bool,
    ) -> u16 {
        // Intelligence capacity scales only when the NPC's camp
        // is Lacklandists; Royalist soldiers (also EnemyAi-driven)
        // get the raw intelligence.
        if !hostile_to_player {
            return self.soldier_profile_iq;
        }
        difficulty.rules().enemy_iq(self.soldier_profile_iq, 100)
    }

    pub fn get_courage(&self) -> u16 {
        self.soldier_profile_courage
    }

    pub fn get_rank(&self) -> ProfileRank {
        self.soldier_profile_rank
    }

    pub fn is_archer(&self) -> bool {
        self.is_archer_unit
    }

    // -----------------------------------------------------------------------
    // Helper methods (internal)
    // -----------------------------------------------------------------------

    fn clear_swordstrike_experiences(&mut self) {
        self.known_enemy_strike_1 = None;
        self.known_enemy_strike_2 = None;
        self.known_enemy_strike_3 = None;
    }

    /// Fired when a special action animation begins (helbardman frame
    /// 40 or non-helbardman start-of-anim).  Two-way branch:
    ///
    ///   * Shield-bearers always speak via `SpeechFlags::ALWAYS`,
    ///     which is meant to bypass `is_remark_forbidden`.  The Rust
    ///     speech pipeline doesn't yet enforce a forbidden-list
    ///     gate, so `ALWAYS` is currently a no-op there — we set it
    ///     anyway so the wiring lands when the gate is implemented.
    ///   * Everyone else only speaks at 1-in-3 odds and only when
    ///     currently silent (the `current_remark == TheSoundOfSilence`
    ///     guard).  The silence guard is also enforced by `say_impl`
    ///     itself, but we keep the explicit check for clarity.
    pub fn make_special_action_remark(&mut self, sim: &SimulationContext, is_shield_bearer: bool) {
        if is_shield_bearer {
            self.base
                .say_with_flags(Remark::SpecialAction, crate::ai::SpeechFlags::ALWAYS);
        } else if self.base.current_remark == Remark::TheSoundOfSilence
            && crate::sim_rng::u32(sim, crate::sim_rng::RngSite::SpecialActionRemark, 0..3) == 0
        {
            self.base.say(Remark::SpecialAction);
        }
    }

    /// Kick off a directed panic — the NPC flees away from `center`.
    ///
    /// Resume door selection against live state after releasing this borrow.
    pub(crate) fn panic_from_position(&mut self, center: Position, runs: u8) {
        let was_already_fleeing = matches!(
            self.base.current_substate,
            Substate::FleeingPanic | Substate::FleeingRunToDoor
        );
        self.base.panic_center_x = center.x;
        self.base.panic_center_y = center.y;
        self.base.directed_panic = true;
        self.base.outbox.actor.begin_panic = Some(crate::ai::PanicRequest {
            center: Some(center),
            runs,
            alert: crate::ai::AlertLevel::Red,
            is_new_panic: !was_already_fleeing,
        });
    }
}

impl EnemyAi {
    // -----------------------------------------------------------------------
    // State management
    // -----------------------------------------------------------------------

    /// Assign the soldier's guarded PC *and* synchronise the
    /// reciprocal `guard` pointer on both the old and new PC.  The
    /// AI can't touch the PC entity directly, so the PC-side flip is
    /// queued in the ordered actor outbox for the engine drain.
    ///
    pub fn set_guarded_pc(&mut self, new_pc: Option<PcId>) {
        let old_pc = self.guarded_pc;
        if old_pc == new_pc {
            return;
        }
        self.guarded_pc = new_pc;
        self.base.outbox.actor.set_guarded_pc = Some(GuardedPcEffect {
            old: old_pc,
            new: new_pc,
        });
    }

    pub(crate) fn update_new_task_priority(&mut self, stimulus: &Stimulus) {
        match stimulus.stimulus_type {
            StimulusType::EventSeesObject => {
                self.new_task_priority = task_priority::STRANGE_THING;
            }
            StimulusType::CallLookThere => {
                self.new_task_priority = task_priority::DANGEROUS_THING;
            }
            StimulusType::EventMissesCharly
            | StimulusType::EventSeesCharly
            | StimulusType::EventSeesSoldier => {
                self.new_task_priority = task_priority::MISSED_FRIEND;
            }
            StimulusType::EventSeesBody => {
                self.new_task_priority = task_priority::BODY;
            }
            StimulusType::EventGetArrow => {
                self.new_task_priority = task_priority::COMBAT_NOISE;
            }
            StimulusType::EventSeesFriendInTrouble => {
                self.new_task_priority = task_priority::FRIEND_IN_TROUBLE;
            }
            StimulusType::CallHey
            | StimulusType::CallMrOfficerIAmBack
            | StimulusType::CallAlert
            | StimulusType::CallInstruction
            | StimulusType::CallHint
            | StimulusType::EventPanic => {
                self.new_task_priority = task_priority::ALERT;
            }
            StimulusType::EventView
            | StimulusType::EventEnterSwordfight
            | StimulusType::EventSwordStrike
            | StimulusType::EventGotHit
            | StimulusType::EventPcShotAtMe
            | StimulusType::CallCombatAlert => {
                self.new_task_priority = task_priority::ENEMY;
            }
            StimulusType::EventHear => {
                // Combat noises (ZINGZING) get higher priority
                if let StimulusInfo::Noise(ref noise) = stimulus.info {
                    if noise.noise_type == NoiseType::ZingZing {
                        self.new_task_priority = task_priority::COMBAT_NOISE;
                    } else {
                        self.new_task_priority = task_priority::STRANGE_THING;
                    }
                } else {
                    self.new_task_priority = task_priority::STRANGE_THING;
                }
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // Return to duty — restore default behavior
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // React — reaction delay before responding
    // Enemy reaction behavior.
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Character-based decision making
    // -----------------------------------------------------------------------

    /// New-task priority decision — shared between the
    /// indoor and outdoor branches of character-based decisions.
    pub(crate) fn has_the_new_task_priority(&self) -> bool {
        if self.new_task_priority >= self.current_task_priority {
            return true;
        }
        match self.base.current_state {
            AiState::Seeking | AiState::Wondering => false,
            _ => self.minimal_task_priority == task_priority::NONE,
        }
    }

    // -----------------------------------------------------------------------
    // Distress notification
    // -----------------------------------------------------------------------
    //
    // Distress notification has no effect in the shipped game. The hook
    // stays here because several combat paths call it unconditionally
    // when a fight starts.
    pub fn i_am_in_trouble(&mut self, _attacker: ElementHandle) {}

    // -----------------------------------------------------------------------
    // House-door passage has no additional AI effect.
    // Kept as a no-op hook for the two call sites in
    // actor leave/enter callbacks that would otherwise need to
    // branch on entity type.
    // -----------------------------------------------------------------------

    pub fn pass_house_door(&mut self, _entering: bool) {}
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
