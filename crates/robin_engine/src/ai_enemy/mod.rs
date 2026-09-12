//! Enemy (villain/soldier) AI.
//!
//! This module contains the `EnemyAi` struct which extends [`AiController`]
//! with soldier-specific state: combat tactics, seek behavior, officer/archer
//! specializations, money fights, and the massive Think state machine.

mod alert;
mod archer_combat;
mod battle;
mod combat_positions;
mod detection;
mod event_handlers;
mod money_fight;
pub(crate) use detection::context_detects_180_degrees;
mod periodic;
mod seek;
mod substate_handlers;
mod util;

#[cfg(test)]
pub(crate) use alert::CommandSoldiersStart;
pub use util::*;

use crate::ai::*;
use crate::entity_id::PcId;
use crate::parameters_ai;

/// Borrowed decision inputs. Mutable global AI state remains a separate owner.
/// This is an ephemeral call context, never a save or rollback projection.
#[derive(Clone, Copy)]
struct ThinkEnv<'a> {
    sim: &'a crate::sim_rng::SimulationContext,
    ctx: &'a AiContext,
    tick: &'a AiPerTickData,
    grid: Option<&'a crate::fast_find_grid::FastFindGrid>,
}

/// Master switch for the opt-in AI decision/path diagnostic used by the
/// Save020/Save055 substate-only parity cohort. Keep this check separate so
/// disabled runs return before reading frame, owner, AI, or geometry state.
pub(crate) fn decision_path_debug_enabled() -> bool {
    std::env::var_os("PARITY_DEBUG_AI_DECISION_PATH").is_some()
}

/// Master switch for the battle-planning / phalanx / shield-timer
/// diagnostic. Prints which branch of the decision tree an NPC took and the
/// inputs that selected it, which is what a `actor.command` or `ai.substate`
/// divergence in the shield-bearer and archer families reduces to. Cached in
/// a `OnceLock` because the call sites sit on the per-stimulus AI path.
pub(crate) fn battle_decision_debug_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("PARITY_DEBUG_BATTLE_DECISION").is_some())
}

/// Exact frame/owner gate for the AI decision/path diagnostic. Enabling the
/// master switch without both filters is an operator error: broad traces make
/// same-frame re-entrant state ownership impossible to attribute reliably.
pub(crate) fn decision_path_debug_matches(frame: u32, owner: HumanHandle) -> bool {
    decision_path_debug_matches_raw(frame, owner)
}

pub(crate) fn decision_path_debug_matches_raw(frame: u32, owner: u32) -> bool {
    static FILTER: std::sync::OnceLock<(u32, u32)> = std::sync::OnceLock::new();
    let &(expected_frame, expected_owner) = FILTER.get_or_init(|| {
        let parse_required = |name: &str| {
            let value = std::env::var(name)
                .unwrap_or_else(|_| panic!("{name} is required for AI_DECISION_PATH diagnostic"));
            value.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid {name}={value:?} for AI_DECISION_PATH diagnostic: {error}")
            })
        };
        (
            parse_required("PARITY_DEBUG_AI_DECISION_PATH_FRAME"),
            parse_required("PARITY_DEBUG_AI_DECISION_PATH_OWNER"),
        )
    });
    frame == expected_frame && owner == expected_owner
}

/// Opt-in, process-local tracing for the Save018 Them-list lifecycle cohort.
/// Environment reads and stderr output deliberately stay outside serialized AI
/// state and do not consume simulation RNG.
pub(super) fn them_lifecycle_debug_matches(ctx: &AiContext) -> bool {
    use crate::engine::diagnostics::ParityGate;
    static GATE: std::sync::OnceLock<ParityGate<2>> = std::sync::OnceLock::new();
    let gate = GATE.get_or_init(|| {
        ParityGate::from_env(
            "PARITY_DEBUG_THEM_LIFECYCLE",
            [
                "PARITY_DEBUG_THEM_FRAME",
                "PARITY_DEBUG_THEM_CREATION_ORDER",
            ],
        )
    });
    gate.enabled() && gate.matches_required([Some(ctx.frame), ctx.original_creation_order])
}

/// Master switch for the opt-in primary-target selection/swap diagnostic.
///
/// Keep this separate from [`primary_swap_debug_matches`] so every call site
/// can return before reading AI/entity state when diagnostics are disabled.
pub(crate) fn primary_swap_debug_enabled() -> bool {
    std::env::var_os("PARITY_DEBUG_PRIMARY_SWAP").is_some()
}

/// Apply the required exact frame/owner gate for primary-target diagnostics.
/// Invalid or incomplete enabled configurations fail loudly rather than
/// accidentally producing a broad trace.
pub(crate) fn primary_swap_debug_matches(frame: u32, owner: HumanHandle) -> bool {
    let parse_required = |name: &str| {
        let value = std::env::var(name)
            .unwrap_or_else(|_| panic!("{name} is required for PRIMARY_SWAP diagnostic"));
        value.parse::<u32>().unwrap_or_else(|error| {
            panic!("invalid {name}={value:?} for PRIMARY_SWAP diagnostic: {error}")
        })
    };
    frame == parse_required("PARITY_DEBUG_PRIMARY_SWAP_FRAME")
        && owner == parse_required("PARITY_DEBUG_PRIMARY_SWAP_OWNER")
}
use crate::position_interface::ASPECT_RATIO;
use util::soldier_detects_position_180;

// ---------------------------------------------------------------------------
// EnemyAi — extends AiController with soldier-specific state
// ---------------------------------------------------------------------------

/// Enemy/soldier AI state. Extends [`AiController`] with villain-specific
/// fields.
#[derive(Debug, Clone, robin_state_hash_derive::StateHash, bitcode::Encode, bitcode::Decode)]
pub struct EnemyAi {
    /// Base AI controller (contains all common state).
    pub base: AiController,

    /// True while this soldier has a pending/in-flight special-strike
    /// sequence (prep wait + strike animation). Mirrors the observable
    /// `Substate::AttackingSwordfightSpecialStrike` while also giving
    /// cancellation reconciliation a direct sequence-lifecycle latch. Set by
    /// `begin_special_strike()`; cleared by per-tick reconciliation in
    /// `engine/melee.rs::tick_enemy_sword_attacks` when the sequence
    /// manager no longer has an active sword-strike element for this
    /// actor (covers both natural completion and interruption).
    pub pending_special_strike: bool,

    /// One-shot handoff from swordfight reconsideration to the engine-side
    /// strike proposer. The original game only proposes a good sword strike
    /// when that event-driven reconsideration reaches its decision tail;
    /// merely entering the swordfight substate must not authorize a draw.
    pub pending_sword_strike_consideration: bool,

    /// AI decisions reached the combat-insult step after swordfight reconsideration,
    /// but the engine-side strike proposer has not yet settled the one-shot
    /// consideration. Original proposes inline: a successful proposal
    /// changes to `...SPECIAL_STRIKE` and suppresses the insult, while a
    /// rejected proposal leaves `...SWORDFIGHT` and says it. The Rust port
    /// settles this latch immediately after `Think`, at the same owner
    /// boundary as `pending_sword_strike_consideration`.
    pub pending_combat_insult_after_strike_consideration: bool,

    // -- Private fields --
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
    pub investigating_distraction: bool,
    /// Cursor into the directions of the currently examined seek point.
    pub last_seek_direction_index: u8,
    pub beggar_to_examine: Option<AiEntityHandle>,
    /// Whether the current `beggar_to_examine` is a real NPC beggar or a
    /// PC in disguise. Set by the engine when populating `beggars_to_control`.
    /// Checked during IdentifyingBeggar1.
    pub beggar_is_npc: bool,

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
    /// `previous_substate`.
    pub previous_state: i32,
    /// Raw serialized storage for the original game's previous substate; see
    /// [`Self::previous_state`].
    pub previous_substate: i32,

    pub reported_to_officer: bool,

    pub missed_soldier_timer: u16,
    pub old_money: u16,

    pub other_seen_money: Vec<ObjectHandle>,
    pub other_seen_ale: Vec<ObjectHandle>,

    pub money_fight_enemies: Vec<NpcHandle>,
    pub money_fight_victims: Vec<NpcHandle>,

    // Archer / shield bearer (serialized by semantic entity reference)
    pub archer_behind_me: Option<AiEntityHandle>,
    pub shield_bearer_before_me: Option<AiEntityHandle>,

    pub shield_bearer_direction: u16,
    pub phalanx_aborted: bool,

    pub changed_to_alert_path: bool,

    pub already_seen_bodies: Vec<HumanHandle>,

    /// Soldiers this officer has called to a group.
    /// Populated by `alert_soldiers`, read by group coordination substates.
    pub alerted_us: Vec<HumanHandle>,
    /// AlertSoldiers candidates not yet called. The Original stops scanning
    /// at 20 successful Think returns, not 20 attempts, so calls advance one
    /// result at a time through this live continuation queue.
    pub pending_alert_soldier_candidates: Vec<HumanHandle>,
    /// Group members still awaiting the officer's synchronous
    /// `CALL_INSTRUCTION`, paired with the seek point authored for that
    /// particular attempt. The original game deletes a refusing member from the live
    /// list and retries the same list index before deciding whether to clear
    /// `SEEK_LOCATION_FIRST`; keeping this as a continuation queue preserves
    /// that source order.
    pub pending_group_instruction_candidates: Vec<(HumanHandle, Position)>,
    /// Flags for the next group instruction. Stored as bits so old serialized
    /// AI snapshots default cleanly and unknown bits cannot be invented.
    pub pending_group_instruction_seek_flags: u16,
    /// Whether the first accepted member consumes `SEEK_LOCATION_FIRST`.
    /// Charly-path distribution keeps the flag for every member instead.
    pub pending_group_instruction_clear_location_after_accept: bool,

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
    pub left_combat_neighbour: Option<AiEntityHandle>,
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

impl Default for EnemyAi {
    fn default() -> Self {
        Self {
            base: AiController::default(),
            pending_special_strike: false,
            missed_pc: None,
            pc_missed: false,
            pc_gone_away_in_this_direction: 0,
            frame_when_missed_charly: 0,
            heard_nets: Vec::new(),
            detected_something_there: Position::default(),
            investigating_distraction: false,
            last_seek_direction_index: 0,
            beggar_to_examine: None,
            beggar_is_npc: false,
            current_task_priority: task_priority::NONE,
            minimal_task_priority: task_priority::NONE,
            new_task_priority: task_priority::NONE,
            number_of_different_checkpoints: 0,
            thirsty: true,
            position_change_locked_for_test: false,
            other_bodies_to_examine: Vec::new(),
            beggars_to_control: Vec::new(),
            positions_of_beggars_to_control: Vec::new(),
            seen_dead_body: false,
            seeking_charly: false,
            my_seek_points: Vec::new(),
            personal_seek_point_1: None,
            personal_seek_point_2: None,
            seek_center: Position::default(),
            actual_seek_point: None,
            seek_point_view_directions: Vec::new(),
            seek_flags: SeekFlags::empty(),
            old_odds: 0,
            gather_position: Position::default(),
            gather_direction: 0,
            gather_position_instructed: false,
            search_charly_way: Vec::new(),
            officers_position: Position::default(),
            previous_state: AiState::Default as i32,
            previous_substate: Substate::DefaultOnPost as i32,
            reported_to_officer: false,
            missed_soldier_timer: 0,
            old_money: 0,
            other_seen_money: Vec::new(),
            other_seen_ale: Vec::new(),
            money_fight_enemies: Vec::new(),
            money_fight_victims: Vec::new(),
            archer_behind_me: None,
            shield_bearer_before_me: None,
            shield_bearer_direction: 0,
            phalanx_aborted: false,
            changed_to_alert_path: false,
            already_seen_bodies: Vec::new(),
            alerted_us: Vec::new(),
            pending_alert_soldier_candidates: Vec::new(),
            pending_group_instruction_candidates: Vec::new(),
            pending_group_instruction_seek_flags: 0,
            pending_group_instruction_clear_location_after_accept: false,
            my_shooting_point: None,
            my_archery_sector: None,
            my_archery_sector_index: 0,
            my_archery_point_index: crate::sector::ArcheryPointIdx::default(),
            my_archery_point_increment: 0,
            enemy_seen_below: false,
            enemy_had_this_elevation: 0,
            known_enemy_strike_1: None,
            known_enemy_strike_2: None,
            known_enemy_strike_3: None,
            return_to_patrol_point: Position::default(),
            fleeing_seen_enemy_counter: 0,
            last_stimulus_dispatched_to_patrol: None,
            character_id: 0,
            old_life_points: 0,
            initial_life_points: 0,
            list_them: Vec::new(),
            ambush_point_array_reset: false,
            ambush_point_status: Vec::new(),
            forced_next_battle_decision: Decision::None,
            reset_battle_decision: false,
            soldier_profile_iq: 50,
            soldier_profile_courage: 50,
            soldier_profile_shooting: 50,
            soldier_profile_vip: false,
            sword_range: 40, // default before profile lookup
            hth_weapon_id: 0,
            sword_is_charge_weapon: false,
            next_sword_strike_frame: 0,
            pending_sword_strike_consideration: false,
            pending_combat_insult_after_strike_consideration: false,
            soldier_profile_bee_time: 0,
            soldier_profile_pride: 0,
            soldier_profile_hearing_factor: 1.0,
            soldier_profile_rank: ProfileRank::Soldier,
            soldier_profile_initiative: 50,
            soldier_profile_beer: 0,
            ale_reliable_distraction: false,
            soldier_profile_money: 0,
            soldier_profile_apple: 0,
            soldier_profile_whistle: 0,
            soldier_profile_duty: false,
            soldier_profile_endurance: 0,
            is_vip: false,
            company_number: 0,
            left_combat_neighbour: None,
            right_combat_neighbour: None,
            attentive: false,
            will_be_attentive: false,
            forced_attentive: false,
            guarded_pc: None,
            my_line_jump: None,
            tower_guard: false,
            combat_trainer: false,
            is_archer_unit: false,
        }
    }
}

impl EnemyAi {
    #[track_caller]
    fn required_primary_target(&self, context: &'static str) -> AiEntityHandle {
        self.base.primary_target.unwrap_or_else(|| {
            panic!(
                "enemy AI {} requires a primary target while {context}",
                self.base.me
            )
        })
    }

    #[track_caller]
    fn required_antagonist(&self, context: &'static str) -> AiEntityHandle {
        self.base.antagonist.unwrap_or_else(|| {
            panic!(
                "enemy AI {} requires an antagonist while {context}",
                self.base.me
            )
        })
    }

    #[track_caller]
    fn required_beggar_to_examine(&self, context: &'static str) -> AiEntityHandle {
        self.beggar_to_examine.unwrap_or_else(|| {
            panic!(
                "enemy AI {} requires a beggar-to-examine while {context}",
                self.base.me
            )
        })
    }

    #[track_caller]
    fn required_detected_body(&self, context: &'static str) -> AiEntityHandle {
        self.base.detected_body.unwrap_or_else(|| {
            panic!(
                "enemy AI {} requires a detected body while {context}",
                self.base.me
            )
        })
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

    pub fn get_iq(&self, ctx: &AiContext) -> u16 {
        // Intelligence capacity scales only when the NPC's camp
        // is Lacklandists; Royalist soldiers (also EnemyAi-driven)
        // get the raw intelligence.
        if !ctx.is_hostile_to_player() {
            return self.soldier_profile_iq;
        }
        ctx.difficulty
            .rules()
            .enemy_iq(self.soldier_profile_iq, 100)
    }

    pub fn get_courage(&self) -> u16 {
        self.soldier_profile_courage
    }

    /// Apply `EASY_ENEMY_FIGHTING / HARD_ENEMY_FIGHTING` modifiers
    /// when the camp is `Lacklandists` (deliberately the FIGHTING
    /// modifiers, not SHOOTING — see the comment in
    /// `EngineInner::bow_profile_and_ability`), then scale by
    /// `(1.0 - 0.01 * blood_alcohol)`.  Used by `AIMING_TIME_FORMULA`
    /// (`(110 - shooting_ability) / 2`) when launching the
    /// bow-aim timer — without this override the timer would track
    /// the soldier's *intelligence* instead of its shooting skill.
    pub fn get_shooting_ability(&self, ctx: &AiContext) -> u16 {
        let mut shooting = if ctx.is_hostile_to_player() {
            // Retail accidentally selects the fighting modifier here. The
            // classic presets keep that result exactly; Custom deliberately
            // exposes shooting as its own rule at this typed boundary.
            ctx.difficulty
                .rules()
                .enemy_shooting(self.soldier_profile_shooting, 100)
        } else {
            self.soldier_profile_shooting
        } as u32;
        if self.base.blood_alcohol > 0 {
            shooting =
                ((shooting as f32) * (1.0 - 0.01 * self.base.blood_alcohol as f32)).max(0.0) as u32;
        }
        shooting.min(u16::MAX as u32) as u16
    }

    pub fn get_rank(&self) -> ProfileRank {
        self.soldier_profile_rank
    }

    pub fn is_archer(&self) -> bool {
        self.is_archer_unit
    }

    /// High-pride soldiers stand back when lower-pride allies are
    /// already engaging the same target.
    pub fn is_too_proud_to_attack(
        &mut self,
        ctx: &AiContext,
        tick: &AiPerTickData,
        target_multiplicity: Option<&std::collections::BTreeMap<HumanHandle, u32>>,
    ) -> bool {
        if self.soldier_profile_pride == 0 {
            return false;
        }
        if self.base.blood_alcohol > 0 {
            return false; // drunk soldiers fight regardless
        }

        // Refresh primary target with the unoccupied-strongly-preferred
        // mode and write it back so downstream battle-planning paths
        // see the refreshed value.
        let new_target = self.get_new_primary_target_with_mult_override(
            PrimaryTargetFlags::UNOCCUPIED_STRONGLY_PREFERRED,
            ctx,
            tick,
            target_multiplicity,
        );
        self.base.primary_target = new_target;
        let Some(new_target) = new_target else {
            return false;
        };

        // Distance-vs-sword-range early-out.  When the target is
        // standing still within our sword reach we attack regardless
        // of pride.
        let target_swordfighting = self
            .find_fighter(new_target.get(), tick)
            .map(|f| f.is_swordfighting)
            .unwrap_or(false);
        if !target_swordfighting && let Some(target) = self.find_fighter(new_target.get(), tick) {
            // The original game uses maximum-norm distance:
            // subtract raw element world positions, stretch Y for the
            // isometric projection, then take the 3D max norm. AI Position()
            // may instead snap a door-passing target to the gate endpoint.
            let target_body = crate::coordinates::WorldPoint3D::new(
                target.raw_position.x,
                target.raw_position.y + target.elevation,
                target.elevation,
            );
            let max_norm = ai_max_norm_distance_world(&target_body, &ctx.self_body_position_world);
            let my_max_range = self
                .find_fighter(self.base.me, tick)
                .map(|f| f.sword_range_maximal as f32)
                .unwrap_or(self.sword_range as f32);
            if max_norm <= my_max_range {
                return false;
            }
        }

        // In reactiontime substates, refuse even without checking allies
        if matches!(
            self.base.current_substate,
            Substate::AttackingReactiontime | Substate::AttackingOfficerGivingOrdersWaiting
        ) {
            return true;
        }

        // Allies-loop only when target is NOT swordfighting.
        // Otherwise (target already engaged) the high-pride soldier
        // refuses to jump in — return true unconditionally.
        if target_swordfighting {
            return true;
        }

        // Check if any lower-pride ally is engaging or observing our
        // target. The original game walks the ally list — those that already
        // passed this decision's own omnidirectional-detection gate — in the
        // order battle planning inserted them, not a fresh proximity list.
        let my_pride = self.soldier_profile_pride;
        for &friend in &self.base.list_us {
            if friend == self.base.me {
                continue;
            }
            // PCs on the allied list fail the original game's soldier test and
            // are absent from the camp-soldier snapshot.
            let Some(f) = tick
                .camp_soldiers
                .iter()
                .find(|soldier| soldier.handle == friend)
            else {
                continue;
            };
            if !f.is_able_to_fight {
                continue;
            }
            // Only consider allies with lower pride
            if f.pride >= my_pride {
                continue;
            }
            // Original tests the broad `_ANY_SWORDFIGHT_SUBSTATE_` AI
            // family here, not the actor's physical sword relationship.
            // Approaching allies (RunningToEnemy/WalkingToEnemy/Charging)
            // already count as committed to the same target.
            if f.ai_substate.is_any_swordfight() && f.primary_target == self.base.primary_target {
                return true;
            }
            // Is this soldier observing our target? The 180° test runs
            // from the observing ally's own eyes, radius and facing.
            let observe_substates = [
                Substate::AttackingApproachToObserve,
                Substate::AttackingObserve,
                Substate::AttackingObserveAndMove,
            ];
            if observe_substates.contains(&f.ai_substate)
                && self.is_detecting_180_degrees_from(friend, new_target.get(), ctx, tick)
            {
                return true;
            }
        }
        false
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
    pub fn make_special_action_remark(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        is_shield_bearer: bool,
    ) {
        if is_shield_bearer {
            self.base
                .say_with_flags(Remark::SpecialAction, crate::ai::SpeechFlags::ALWAYS);
        } else if self.base.current_remark == Remark::TheSoundOfSilence
            && crate::sim_rng::u32(sim, crate::sim_rng::RngSite::SpecialActionRemark, 0..3) == 0
        {
            self.base.say(Remark::SpecialAction);
        }
    }

    /// Turn to face another NPC through the live entity snapshot.
    ///
    /// This is element-based facing: it includes the target's
    /// elevation and preserves facing commands' already-facing Waiting/Bored
    /// short-circuit.
    fn face_npc(&mut self, handle: impl IntoOptionalAiHandle, ctx: &AiContext) {
        self.base.face_entity(handle, ctx);
    }

    /// Forbid a remark on the global frame-expiry list. `flags` is a
    /// `RemarkTargetFlags` bitfield (THIS_GUY / THIS_TYPE / VILLAINS /
    /// CIVILIANS / ALL_NPC).  The caller supplies `speech_id` and
    /// `frame` since the AiController itself doesn't carry them.
    fn forbid_remark(
        &self,
        global: &mut crate::ai::AiGlobalState,
        remark: Remark,
        frames: u32,
        flags: u16,
        speech_id: u32,
        original_creation_order: u32,
        frame: u32,
    ) {
        global.forbidden_remarks.push(crate::ai::ForbiddenRemark {
            remark,
            flags,
            speech_id,
            // The original game narrows the 32-bit creation order into this 16-bit field.
            guy_index: original_creation_order as u16,
            bad_guy: true, // ai_enemy is always a soldier
            forbidden_till_frame: frame + frames,
        });
    }

    /// Reinitialize the Them list with all currently visible enemies.
    fn reinitialize_them_list(&mut self, ctx: &AiContext, _tick: &AiPerTickData) {
        // The original game rebuilds the enemy list. The
        // original deletes the old list and rebuilds it only from enemies
        // whose current enemy-seen flag is set and who are not dead. It
        // does not preserve the primary target when that target is no longer
        // visible.
        // Rebuild `list_them` from the live detectable-list snapshot, not
        // geometric tick products. This includes unconscious enemies; the
        // cleanup (`!is_able_to_fight`) lives downstream in
        // `battle_decisions`.
        let debug = them_lifecycle_debug_matches(ctx);
        if debug {
            eprintln!(
                "[THEM frame={} co={:?} me={} phase=reinitialize_before state={:?} substate={:?} list={:?} seen={:?}]",
                ctx.frame,
                ctx.original_creation_order,
                self.base.me,
                self.base.current_state,
                self.base.current_substate,
                self.list_them,
                ctx.self_seen_enemy_handles,
            );
            for &handle in &ctx.self_seen_enemy_handles {
                let Some(target) = ctx.entity_view(handle) else {
                    eprintln!(
                        "[THEM frame={} co={:?} me={} phase=reinitialize_input target={} missing=true]",
                        ctx.frame, ctx.original_creation_order, self.base.me, handle,
                    );
                    continue;
                };
                eprintln!(
                    "[THEM frame={} co={:?} me={} phase=reinitialize_input target={} dead={} unconscious={} carried={} able={}]",
                    ctx.frame,
                    ctx.original_creation_order,
                    self.base.me,
                    handle,
                    target.is_dead,
                    target.is_unconscious,
                    target.is_carried,
                    target.is_able_to_fight,
                );
            }
        }
        self.list_them.clear();
        for &handle in &ctx.self_seen_enemy_handles {
            let target = match ctx.entity_observation(handle) {
                Ok(target) => target,
                Err(reason) => {
                    // Preserve current retention for every unavailable observation.
                    // TODO: establish Original invalid-layer handling before changing it.
                    tracing::warn!(
                        me = self.base.me,
                        target = handle,
                        ?reason,
                        "omitting seen-enemy handle with unavailable spatial observation"
                    );
                    continue;
                }
            };
            if !target.is_dead {
                self.list_them.push(handle);
            }
        }
        tracing::trace!(
            me = self.base.me,
            seen_enemy_handles_len = ctx.self_seen_enemy_handles.len(),
            list_them = ?self.list_them,
            "reinitialize_them_list"
        );
        if debug {
            eprintln!(
                "[THEM frame={} co={:?} me={} phase=reinitialize_after list={:?}]",
                ctx.frame, ctx.original_creation_order, self.base.me, self.list_them,
            );
        }
    }

    fn initialize_patrol(&mut self) {
        // Patrol initialization requires engine access to resolve soldier IDs to
        // entity handles and check visibility/state. The actual initialization
        // happens in EngineInner::tick_patrol_coordination — we just raise a
        // one-shot flag the engine tick honours next pass, mirroring the
        // explicit patrol-initialization points (`init_ai`
        // / `return_to_duty`).
        self.base.needs_patrol_reinit = true;
    }

    /// Forwards a stimulus to all patrol members via
    /// CrossNpcAction::SendStimulus.  Returns `true` if dispatched
    /// (caller should NOT process the stimulus itself).
    pub(crate) fn dispatch_stimulus_to_whole_patrol(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        tracing::trace!(
            target: "patrol_relay",
            frame = ctx.frame,
            me = self.base.me as i32,
            stimulus_type = ?stimulus.stimulus_type,
            to_whole_patrol = stimulus.to_whole_patrol,
            state = ?self.base.current_state,
            substate = ?self.base.current_substate,
            chief = ?self.base.patrol_chief.map(|c| c.index()),
            patrol_size = self.base.patrol.len(),
            "dispatch enter"
        );
        // Already dispatched to whole patrol — skip
        if stimulus.to_whole_patrol {
            return false;
        }

        // Dedup gate — only consults
        // `last_stimulus_dispatched_to_patrol` for these three event
        // types, and returns `true` ("ignore this stimulus") on a
        // match so the caller stops processing.  All other event
        // types skip the dedup entirely.
        if matches!(
            stimulus.stimulus_type,
            StimulusType::EventSeesObject | StimulusType::EventHear | StimulusType::EventSeesBody
        ) && let Some(ref last) = self.last_stimulus_dispatched_to_patrol
            && last.is_similar(stimulus)
        {
            tracing::trace!(
                target: "patrol_relay",
                me = self.base.me as i32,
                "dispatch dedup hit"
            );
            return true;
        }

        // Only dispatch from DEFAULT (excluding
        // DefaultPatrolEnrouteRunning — too far from patrol) or
        // WONDERING.
        match self.base.current_state {
            AiState::Default => {
                if self.base.current_substate == Substate::DefaultPatrolEnrouteRunning {
                    return false;
                }
            }
            AiState::Wondering => {}
            _ => return false,
        }

        // Delegate to the chief only when the chief exists, is a
        // soldier, and we currently 360°-detect them.  Otherwise we
        // proceed as a would-be chief ourselves.
        if let Some(chief_id) = self.base.patrol_chief {
            let chief = chief_id.index();
            let chief_is_soldier = ctx
                .entity_view(chief)
                .map(|v| v.is_soldier())
                .unwrap_or(false);
            // Short-circuit: a non-soldier chief never reaches the LOS
            // query, so no visibility-cache traffic is generated for it.
            if chief_is_soldier && self.is_detecting_360_degrees(chief as HumanHandle, ctx) {
                self.base.outbox.reentrant.cross_npc_actions.push(
                    CrossNpcAction::RequestPatrolDispatch {
                        chief,
                        caller: self.base.me,
                        stimulus_type: stimulus.stimulus_type,
                        info: stimulus.info,
                    },
                );
                tracing::trace!(
                    target: "patrol_relay",
                    me = self.base.me as i32,
                    chief,
                    "dispatch relay to chief"
                );
                return true;
            }
        }

        // Record on the would-be chief regardless of whether the
        // patrol member loop will run.  Preserves the dedup
        // side-effect for the empty-patrol case below.
        let mut forwarded_stimulus = *stimulus;
        forwarded_stimulus.to_whole_patrol = true;
        self.last_stimulus_dispatched_to_patrol = Some(forwarded_stimulus);

        // Empty patrol — nothing to relay; return `false` so our
        // caller still runs its local handler.
        if self.base.patrol.is_empty() {
            return false;
        }

        // Snapshot the patrol before the self-call below: the broadcast walks
        // this copy even if the cascade adds or drops members.
        let members: Vec<NpcHandle> = self
            .base
            .patrol
            .iter()
            .map(|member_id| member_id.index())
            .collect();

        // `think(stimulus_for_whole_patrol)` — the chief feeds the
        // stimulus back into its own Think *before* relaying to
        // subordinates.  The recursive Think re-enters the event
        // handler, `dispatch_stimulus_to_whole_patrol` early-exits
        // via the `to_whole_patrol` guard at the top of this
        // function, and the standard-procedure handler runs for the
        // chief.  Without this self-recursion, patrol chiefs skipped
        // event_view_standard_procedure after seeing an enemy —
        // primary_target stayed 0, and the subsequent
        // begin_swordfight aborted.
        //
        // Cascade caveat: this re-entrant `think` skips the engine
        // `filter_ai_event` gate because `self` is mut-borrowed
        // here.  See the matching note in `end_think` for why that's
        // safe against shipped `fullgame` scripts.
        if self.base.has_script_filter_override {
            tracing::warn!(
                target: "filter_ai_event_divergence",
                handle = self.base.me as i32,
                stimulus_type = ?forwarded_stimulus.stimulus_type,
                "cascade think() skipped filter_ai_event gate (patrol chief re-entrant \
                 dispatch); scripted actor may see divergent behavior"
            );
        }
        self.think(sim, &forwarded_stimulus, global, ctx, tick, grid);

        // Forward to patrol members that are soldiers and within 360°
        // detection range. Queue the walk as one action rather than resolving
        // it here: the detection gate for each member belongs immediately
        // before that member's `think`, after the self-call above has finished
        // cascading.
        tracing::trace!(
            target: "patrol_relay",
            me = self.base.me as i32,
            members = ?members,
            "dispatch queue relay to members"
        );
        self.base.outbox.reentrant.cross_npc_actions.push(
            CrossNpcAction::RelayStimulusToPatrolMembers {
                members,
                stimulus_type: forwarded_stimulus.stimulus_type,
                info: forwarded_stimulus.info,
            },
        );

        true
    }

    fn nearby_civilians_panic(&mut self) {
        // The original game updates every eligible civilian synchronously. Keep
        // this engine callback in the same FIFO as speech/state changes so a caller
        // such as swordfight entry preserves panic-then-speech order.
        tracing::trace!(
            target: "parity_nearby_panic",
            owner = self.base.me,
            "queue synchronous NearbyCiviliansPanic callback"
        );
        self.base
            .outbox
            .reentrant
            .owner_work
            .push(crate::ai::AiOwnerWork::NearbyCiviliansPanic);
    }

    fn nearby_civilians_panic_180(&mut self) {
        // SUBSTATE_WONDERING_BRAWL_HITTING contains its own inline sweep in
        // original game and uses forward-half-plane detection. Do not route it through
        // the shared NearbyCiviliansPanic callback, whose detector is 360°.
        tracing::trace!(
            target: "parity_nearby_panic",
            owner = self.base.me,
            "queue synchronous brawl NearbyCiviliansPanic180 callback"
        );
        self.base
            .outbox
            .reentrant
            .owner_work
            .push(crate::ai::AiOwnerWork::NearbyCiviliansPanic180);
    }

    /// Soldier-only; walks same-camp soldiers, finds an officer in
    /// Default or MoneyReactiontime within the HEARS/SEES brawl
    /// thresholds, and dispatches EVENT_SEES_BRAWL to the first one
    /// that qualifies.
    ///
    /// 3-way gate:
    ///   - sq_dist < 200² → always reacts
    ///   - sq_dist < 350² → reacts iff detected within the forward half-plane
    ///   - otherwise → reacts iff detected within the cone with line of sight
    ///
    /// The snapshot carries each officer's live position, facing, and
    /// view-cone parameters (direction, radius, half-aperture, eye
    /// status), so all three branches evaluate the officer's view
    /// rather than approximating with the brawling soldier's own.
    fn maybe_officer_sees_me_fighting(&mut self, ctx: &AiContext, tick: &crate::ai::AiPerTickData) {
        if self.get_rank() != ProfileRank::Soldier {
            return;
        }
        const SQ_HEARS: f32 = 200.0 * 200.0;
        const SQ_SEES_180: f32 = 350.0 * 350.0;

        // Take a quick clone of camp_soldiers so we don't alias the
        // AiPerTickData across the detection call.
        let candidates: Vec<_> = tick
            .camp_soldiers
            .iter()
            .filter(|s| {
                s.rank == ProfileRank::Officer
                    && (s.ai_state == AiState::Default
                        || s.ai_substate == Substate::WonderingMoneyReactiontime)
            })
            .cloned()
            .collect();

        for officer in candidates {
            let dx = officer.position.x - ctx.position.x;
            let dy = officer.position.y - ctx.position.y;
            let sq = dx * dx + dy * dy;
            // Three bands:
            //   * `< 200²` — always reacts.
            //   * `200²..350²` — forward-half-plane detection.
            //   * `≥ 350²` — full-cone detection plus line of sight,
            //     evaluated on the officer's side at the original-game decision point.
            let react = if sq < SQ_HEARS {
                true
            } else if sq < SQ_SEES_180 {
                // Gate on the officer's own live view radius, the same
                // quantity the cone+LOS band below uses. The level's
                // standard radius belongs to nobody in particular, and
                // alertness, drunkenness and lean-out all move an
                // individual officer's radius away from it.
                soldier_detects_position_180(
                    &officer,
                    ctx.position,
                    (officer.view_radius as f32).powi(2),
                )
            } else {
                let officer_view = ctx.entity_view(officer.handle).unwrap_or_else(|| {
                    panic!(
                        "combat-observation officer {} is absent from the AI entity view",
                        officer.handle
                    )
                });
                !officer.eye_blind
                    && !officer.in_building
                    && officer.is_able_to_fight
                    && !ctx.in_building
                    && crate::ai_vision::is_detecting_target(
                        crate::coordinates::MapPoint::new(officer.position.x, officer.position.y),
                        crate::coordinates::GroundPoint::new(
                            officer.position.x,
                            officer.position.y + officer_view.elevation,
                        ),
                        officer.direction as i16,
                        (officer.view_direction[0], officer.view_direction[1]),
                        officer.real_half_aperture,
                        officer.view_radius,
                        crate::coordinates::MapPoint::new(ctx.position.x, ctx.position.y),
                        crate::coordinates::GroundPoint::new(
                            ctx.position.x,
                            ctx.position.y + ctx.elevation,
                        ),
                        ctx.position.level,
                        ctx.obstacle_list(),
                        &ctx.fast_grid,
                    )
            };
            if react {
                self.base
                    .outbox
                    .reentrant
                    .cross_npc_actions
                    .push(CrossNpcAction::SendStimulus {
                        target: officer.handle,
                        stimulus_type: StimulusType::EventSeesBrawl,
                        info: crate::ai::StimulusInfo::Human(AiEntityHandle::new(self.base.me)),
                        fallback_to_sender: None,
                        to_whole_patrol: false,
                    });
                return;
            }
        }
    }

    /// Kick off a directed panic — the NPC flees away from `center`.
    ///
    /// Stash the panic center, transition to `Fleeing / FleeingPanic`,
    /// and queue a `PanicRequest` so the engine's
    /// `process_pending_begin_panic_for` can pick a door on the far
    /// side of the center (or fall back to a random escape vector).
    fn panic_from_position(&mut self, center: Position, runs: u8) {
        let was_already_fleeing = matches!(
            self.base.current_substate,
            Substate::FleeingPanic | Substate::FleeingRunToDoor
        );
        self.base.panic_center_x = center.x;
        self.base.panic_center_y = center.y;
        self.base.directed_panic = true;
        if !was_already_fleeing {
            self.set_state(AiState::Fleeing, Substate::FleeingPanic);
        }
        self.base.outbox.actor.begin_panic = Some(crate::ai::PanicRequest {
            center: Some(center),
            runs,
            alert: crate::ai::AlertLevel::Red,
            is_new_panic: !was_already_fleeing,
        });
    }

    /// Queue a building-wide enemy alert for engine post-processing.
    ///
    /// The engine reads `pending_enemy_in_house_alert`, walks
    /// building occupants, panics civilians, and calls
    /// door-battle initialization on the camp split. Caller must have
    /// already verified `ctx.in_building`.
    fn request_enemy_in_house_alert(&mut self, ctx: &AiContext) {
        debug_assert!(
            ctx.in_building,
            "request_enemy_in_house_alert called outside a building"
        );
        self.base.outbox.actor.enemy_in_house_alert = true;
        tracing::trace!(
            me = self.base.me,
            substate = ?self.base.current_substate,
            building_sector = ?ctx.building_sector,
            "request_enemy_in_house_alert"
        );
    }
}

impl EnemyAi {
    /// Collects visible child-civilian NPCs (alive, conscious, in
    /// `STATE_DEFAULT`), picks the nearest as the antagonist,
    /// notifies the antagonist with `CALL_YOU_JUST_WAIT` and each
    /// other suspect with `EVENT_APPLE_CHASE_NEAR`, and launches the
    /// chase.  Returns `true` if a chase started.
    fn chase_childs(&mut self, ctx: &AiContext) -> bool {
        // Iterate the per-tick entity views — zero-cost filter because
        // we already have the `is_child` / `ai_state` /
        // `is_able_to_fight` fields on the view.
        let mut suspects: Vec<(NpcHandle, Position)> = Vec::new();
        let mut best_distance = f32::INFINITY;
        let mut best_handle = None;
        for (handle, view) in ctx.entity_views.iter() {
            if !view.is_civilian() || !view.is_child {
                continue;
            }
            if !view.is_able_to_fight {
                // Filter `!is_dead && !is_unconscious`.
                continue;
            }
            if view.ai_state != AiState::Default {
                continue;
            }
            // Use the directional facing+LOS variant, not 360°.
            // `is_detecting_180_degrees` is the closest standalone
            // helper we have.
            if !self.is_detecting_180_degrees(*handle as HumanHandle, ctx) {
                continue;
            }
            suspects.push((*handle as NpcHandle, view.position));
            // Maximum norm — Chebyshev distance.
            let dx = (view.position.x - ctx.position.x).abs();
            let dy = (view.position.y - ctx.position.y).abs();
            let dist = dx.max(dy);
            if dist < best_distance {
                best_distance = dist;
                best_handle = Some(AiEntityHandle::new(*handle));
            }
        }

        if suspects.is_empty() {
            return false;
        }
        let best_handle = best_handle
            .expect("non-empty child chase candidate list must have a nearest antagonist");
        self.base.antagonist = Some(best_handle);

        // Inform all suspects.
        for (handle, _pos) in &suspects {
            let stim = if *handle == best_handle.get() {
                StimulusType::CallYouJustWait
            } else {
                StimulusType::EventAppleChaseNear
            };
            self.base
                .outbox
                .reentrant
                .cross_npc_actions
                .push(CrossNpcAction::SendStimulus {
                    target: *handle,
                    stimulus_type: stim,
                    info: crate::ai::StimulusInfo::Human(AiEntityHandle::new(self.base.me)),
                    fallback_to_sender: None,
                    to_whole_patrol: false,
                });
        }

        // lasting_panic_runs = apple / 2.
        self.base.lasting_panic_runs = (self.soldier_profile_apple / 2) as u8;

        // Chase!
        self.base.set_emoticon(EmoticonType::Thunderstorm);
        self.base
            .say_with_flags(Remark::ChasesChild, crate::ai::SpeechFlags::MYTALK_1);
        let antagonist_pos = ctx
            .entity_view(best_handle)
            .map(|v| v.position)
            .unwrap_or(ctx.position);
        self.go_near(
            AiState::Wondering,
            Substate::WonderingAppleChasingChild,
            antagonist_pos,
            5,
            crate::ai::GotoFlags::RUN | crate::ai::GotoFlags::DONT_STOP,
            ctx,
        );
        self.base.launch_timer(10, ctx.frame);
        true
    }

    /// "Enemy behind me" dot-product check used by the
    /// `EVENT_OUTOFVIEW` handler for `REACTIONTIME_RUNNING` /
    /// `APPROACH_TO_OBSERVE` / `ADVANCING_WITH_SHIELD`.  If the NPC's
    /// stare vector is pointing away from the body direction, the
    /// target is "just out of view because I'm looking the wrong
    /// way" and the OUTOFVIEW is ignored.
    ///
    fn enemy_is_behind_me(&self, ctx: &AiContext) -> bool {
        // The original game subtracts the actor's ground position from its stare point.
        // in the original game. The ground position is the raw sprite point, *not*
        // the actor's AI position. The two differ only
        // while the actor is passing a door/gate, where `ctx.position` is
        // snapped to the committed gate endpoint; using the snapped point
        // there moved the stare vector far enough to flip the sign of the
        // dot product and swallow a legitimate EVENT_OUTOFVIEW.
        let actor_ground = crate::coordinates::GroundPoint::new(
            ctx.self_body_position_world.x,
            ctx.self_body_position_world.y,
        );
        let stare_dx = (ctx.self_stare_point.x - actor_ground.x) * ASPECT_RATIO;
        let stare_dy = ctx.self_stare_point.y - actor_ground.y;
        // The original game constructs this from a 16-way sector direction,
        // whose literal lookup table matters at perpendicular boundaries.
        // Reconstructing the same nominal direction through sin/cos can
        // round an exact-zero dot product slightly negative and suppress a
        // legitimate OUTOFVIEW event.
        let (look_dx, look_dy) = crate::element::direction_vector_16(ctx.direction as i16);
        let dot = look_dx * stare_dx + look_dy * stare_dy;
        tracing::trace!(
            me = self.base.me,
            frame = ctx.frame,
            direction = ctx.direction,
            stare_x = ctx.self_stare_point.x,
            stare_y = ctx.self_stare_point.y,
            actor_x = actor_ground.x,
            actor_y = actor_ground.y,
            elevation = ctx.elevation,
            eye_status = ?ctx.self_eye_status,
            dot,
            behind = dot < 0.0,
            "enemy_is_behind_me"
        );
        dot < 0.0
    }

    /// Shared body of the `EVENT_OUTOFVIEW` seek-handler.  Forecasts
    /// the target's destination, sets `missed_pc` / `pc_missed`,
    /// reinitializes the battle list, and either chases the lost
    /// enemy (via `seek_area`) or faces the last sight + runs a
    /// battle overview.
    fn out_of_view_seek_handler(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        enemy: HumanHandle,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        _grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        tracing::trace!(
            npc = self.base.me,
            frame = ctx.frame,
            state = ?self.base.current_state,
            substate = ?self.base.current_substate,
            enemy,
            "handling OUTOFVIEW through lost-enemy path"
        );
        // The original game forecasts the human carried by this out-of-view stimulus,
        // not the soldier's independently selected primary target.
        let prepared = tick
            .enemy_detectable_forecasts
            .iter()
            .find_map(|(handle, forecast)| (*handle == enemy).then_some(forecast))
            .or_else(|| {
                // Detection dispatch rebuilds the tick for the human carried
                // by this exact stimulus. A preceding queued Think may have
                // changed the AI member `primary_target`, or removed the
                // falling-edge human from the live detectable list, but the
                // target-specific snapshot still contains the authoritative
                // AI destination-forecast input for this OUTOFVIEW call.
                (Some(AiEntityHandle::new(enemy)) == tick.primary_target_snapshot_handle)
                    .then_some(tick.primary_target_forecast.as_ref())
                    .flatten()
            })
            .unwrap_or_else(|| {
                panic!(
                    "NPC {} OUTOFVIEW target {} has no prepared destination forecast",
                    self.base.me, enemy
                )
            });
        let forecast =
            prepared.resolve_retaining_direction(sim, self.pc_gone_away_in_this_direction);
        self.base.seek_position = forecast.position;
        self.pc_gone_away_in_this_direction = forecast.direction;

        self.missed_pc = Some(AiEntityHandle::new(enemy));
        self.pc_missed = true;
        self.reinitialize_them_list(ctx, tick);

        if self.list_them.is_empty() {
            let defer_overview_until_after_quit = ctx.is_swordfighting;
            if defer_overview_until_after_quit {
                self.end_swordfight(ctx, tick);
            }
            self.base.outbox.actor.set_unfocus();

            let enemy_is_pc = ctx
                .entity_view(enemy)
                .unwrap_or_else(|| {
                    panic!(
                        "NPC {} OUTOFVIEW target {} has no live entity view",
                        self.base.me, enemy
                    )
                })
                .is_pc;
            if enemy_is_pc && self.answer_question(Question::ShallIFollowLostEnemy, ctx) {
                self.base.say(Remark::HuntsEnemy);
                self.seek_area(
                    sim,
                    self.base.seek_position,
                    parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                    SeekFlags::LOCATION_FIRST | SeekFlags::HOUSE,
                    self.pc_gone_away_in_this_direction,
                    global,
                    ctx,
                    tick,
                );
            } else {
                // The lost-enemy branch snaps toward the missed human's
                // current position, then enters the ordinary (non-FAST)
                // battle overview.  The forecast is retained for a possible
                // chase, but is not the facing target here.
                let missed_position = ctx
                    .entity_view(enemy)
                    .unwrap_or_else(|| {
                        panic!(
                            "NPC {} OUTOFVIEW target {} vanished before overview facing",
                            self.base.me, enemy
                        )
                    })
                    .position;
                let dx = missed_position.x - ctx.position.x;
                let dy = missed_position.y - ctx.position.y;
                self.base.outbox.actor.set_direction_instantly =
                    Some(crate::ai_enemy::util::vec_to_sector(dx, dy) as i16);
                if defer_overview_until_after_quit {
                    // Launching a quit-swordfight sequence element interrupts the
                    // selected actor command synchronously. Its condolence
                    // re-enters Think before this outer handler continues
                    // into battle-overview evaluation.
                    self.base.outbox.actor.lost_enemy_overview_after_quit = true;
                } else {
                    self.get_battle_overview(0, ctx, tick);
                }
            }
        }
    }

    /// `radius` is the look-there radius (100 for vision-triggered
    /// alerts, 200 for noise-triggered).
    ///
    /// Returns `true` when at least one friend was called and the caller's
    /// remaining work has been parked in `continuation`: the calls are
    /// delivered synchronously, so the caller must not run its tail (state
    /// transitions included) until they have closed. Returns `false` when no
    /// friend qualified, in which case the caller falls straight through into
    /// its tail exactly as the Original does on an empty broadcast.
    #[must_use]
    fn hey_folks_look_there(
        &mut self,
        pos: &Position,
        radius: u16,
        continuation: LookThereContinuation,
        ctx: &AiContext,
    ) -> bool {
        let radius_sq = (radius as f32) * (radius as f32);
        let my_pos = ctx.self_body_position_world;
        // The engine performs the state-filtered registry walk against live
        // recipients. Use this snapshot only to avoid suspending the caller
        // when no same-camp soldier is even in range.
        let any_friend_in_range = ctx
            .entity_views
            .iter()
            .filter(|(handle, view)| {
                **handle != self.base.me && view.is_soldier() && ctx.is_allied_with(view.camp)
            })
            .any(|(_, view)| {
                // Original subtracts each friend's raw
                // the caller's raw element position
                // position. AI Position() is not interchangeable here: it
                // snaps an actor passing a door to a gate endpoint.
                let friend_pos = view.detection_position_world;
                let dx = friend_pos.x - my_pos.x;
                let dy = friend_pos.y - my_pos.y;
                let dz = friend_pos.z - my_pos.z;
                dx * dx + dy * dy + dz * dz < radius_sq
            });
        if !any_friend_in_range {
            return false;
        }

        self.base
            .outbox
            .reentrant
            .cross_npc_actions
            .push(CrossNpcAction::BroadcastLookThere {
                caller: self.base.me,
                position: *pos,
                radius,
                continuation,
            });
        true
    }

    pub(crate) fn resume_after_look_there(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        continuation: LookThereContinuation,
        global: &mut AiGlobalState,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        tracing::trace!(
            target: "look_there",
            me = self.base.me,
            state = ?self.base.current_state,
            substate = ?self.base.current_substate,
            ?continuation,
            "hey_folks_look_there: resuming caller tail"
        );
        match continuation {
            LookThereContinuation::EventView { enemy, enemy_pos } => {
                self.event_view_after_look_there(sim, enemy, enemy_pos, global, ctx, tick, grid);
            }
            LookThereContinuation::EventSeesBody {
                body,
                body_pos,
                is_charly,
            } => {
                self.event_sees_body_after_look_there(body, body_pos, is_charly, ctx, tick);
            }
            LookThereContinuation::EventGetArrow => {
                self.event_get_arrow_after_look_there(ctx, tick);
            }
            LookThereContinuation::SeekingArrowReactiontime => {
                self.base.launch_timer(200, ctx.frame);
            }
        }

        // The look-there broadcast is synchronous, so everything above
        // still runs inside the decision tick that suspended here and its completion
        // dispatches whatever completion the tail raised. Rust parks the tail
        // outside that Think, so close the completion boundary explicitly —
        // otherwise a no-op Face in the tail (`already_turned`) is discarded
        // and the actor is stranded in a *_TURNING substate waiting on an
        // EVENT_DONE that never arrives.
        self.base.finish_suspended_common_handler();
    }

    /// Default bored behavior — look sidewards randomly on post.
    /// Called from `think_expected_event` for `EventTimer` on
    /// `DefaultOnPost` before delegating to the base-class common
    /// handler.
    fn default_bored_standard_procedure(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        ctx: &AiContext,
    ) -> bool {
        // Also gate on `self_animation != WaitingUprightBoredRandom`:
        // if the bored-random idle is already playing, the NPC is
        // "bored enough" and we skip the head-turn transition.
        if self.base.current_substate == Substate::DefaultOnPost
            && ctx.self_animation != crate::order::OrderType::WaitingUprightBoredRandom
            && !self.base.likes_to_sit_around
            && !self.base.special_action
        {
            self.set_state(AiState::Default, Substate::DefaultOnPostLookingSidewards);
            self.base.stop_all();

            let dir = match crate::sim_rng::u32(sim, crate::sim_rng::RngSite::DefaultPostLook, 0..4)
            {
                0 => LookDirection::Left,
                1 => LookDirection::Right,
                2 => LookDirection::LeftRight,
                _ => LookDirection::RightLeft,
            };
            tracing::trace!(
                me = self.base.me,
                ?dir,
                "default_bored_standard_procedure: queueing look-sidewards"
            );
            self.base.outbox.actor.look_sidewards = Some(dir);
            return true;
        }
        tracing::trace!(
            me = self.base.me,
            substate = ?self.base.current_substate,
            likes_to_sit_around = self.base.likes_to_sit_around,
            special_action = self.base.special_action,
            "default_bored_standard_procedure: SKIP"
        );
        false
    }

    /// Compute how close to run towards the enemy before engaging.
    fn compute_enemy_run_distance(&self, standard_sword_range: u16) -> u16 {
        let courage_distance = 2 * (100 - self.get_courage());
        // sword_distance = standard sword range + 10
        let sword_distance: u16 = standard_sword_range + 10;
        if courage_distance < sword_distance {
            sword_distance
        } else {
            courage_distance
        }
    }

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

    #[track_caller]
    pub fn set_state(&mut self, state: AiState, substate: Substate) {
        let debug_decision_path = decision_path_debug_enabled()
            && decision_path_debug_matches(self.base.cached_frame, self.base.me);
        if debug_decision_path {
            eprintln!(
                "AIDECISION frame={} owner={} stage=set_state caller={} from={:?}/{:?} to={state:?}/{substate:?} couldnt={} already={} owner_work_before={:?}",
                self.base.cached_frame,
                self.base.me,
                std::panic::Location::caller(),
                self.base.current_state,
                self.base.current_substate,
                self.base.couldnt_reachpoint,
                self.base.already_on_point,
                self.base.outbox.reentrant.owner_work,
            );
        }
        debug_assert_eq!(
            substate.ai_state_family(),
            Some(state),
            "EnemyAi::set_state received mismatched state/substate: {state:?}/{substate:?}"
        );

        // Every state transition forgets pending timers; otherwise a
        // stale timer launched in the previous substate fires an
        // out-of-context `EventTimer` after the new substate has
        // taken effect.  `stop_all` clears it but plenty of
        // `set_state` call sites don't go through `stop_all`, so do
        // it here too.
        self.base.timer_is_running = false;

        // Leaving `STATE_MENACING` calls `set_guarded_pc(None)` so the
        // PC being menaced loses its guard pointer and the next
        // soldier that reaches the sleeping-enemy approach can see
        // the PC as unguarded again.
        if self.base.current_state == AiState::Menacing && state != AiState::Menacing {
            self.set_guarded_pc(None);
        }

        // Alert-path switch.  When leaving STATE_DEFAULT into any
        // other state and the NPC has a configured `alert_path_id`
        // it hasn't switched to yet, adopt that hiking path as the
        // patrol path.  Previously this was only handled on the
        // `SleepingAwakening` arm; departures from Default into any
        // alertable state (Wondering / Seeking / Attacking / …)
        // skipped the swap and the soldier kept patrolling on the
        // unaware path.
        if self.base.current_state == AiState::Default
            && state != AiState::Default
            && !self.changed_to_alert_path
            && let Some(alert_path_id) = self.base.alert_path_id
        {
            self.changed_to_alert_path = true;
            self.base.path_id = Some(alert_path_id);
            self.base.detach_patrol_path(Some(alert_path_id), true);
            self.base.has_patrol_path = true;
        }

        // `set_view_status(EYES_LOOK_FORWARD)` when leaving
        // STATE_SLEEPING. Reasserting LookForward for *every* sleeping
        // departure (not just `SleepingAwakening`) covers routes that drop
        // straight from a dream/blind substate into Wondering/Attacking
        // without going through the SleepingAwakening pipeline. The actual
        // write is queued below, after the state-change callback, matching the
        // statement order in the original game's state transition.
        let opens_eyes = self.base.current_state == AiState::Sleeping && state != AiState::Sleeping;

        // Break the archer-behind-me pairing when leaving any
        // substate that isn't shield-protect / phalanx /
        // running-to-phalanx.  The engine snapshot pass also
        // reconciles the reverse link, but this pre-emptive clear
        // matches the paired-teardown semantics on state
        // transitions.
        if self.archer_behind_me.is_some()
            && !matches!(
                substate,
                Substate::AttackingProtectingWithShield
                    | Substate::AttackingPhalanx
                    | Substate::AttackingRunningToPhalanx
            )
        {
            let old_archer = self
                .archer_behind_me
                .take()
                .expect("checked archer-behind-me presence");
            self.base.outbox.reentrant.cross_npc_actions.push(
                CrossNpcAction::SetShieldBearerBeforeMe {
                    target: old_archer.get(),
                    shield_bearer: None,
                },
            );
        }

        // If we had a shield-bearer pairing and are leaving a
        // bow-related substate, break the pairing.
        if self.shield_bearer_before_me.is_some() {
            match substate {
                Substate::AttackingBowShooting
                | Substate::AttackingBowLoading
                | Substate::AttackingBowAiming
                | Substate::AttackingBowObservingLoading
                | Substate::AttackingBowObserving
                | Substate::AttackingBowRunningBehindShieldBearer
                | Substate::AttackingBowCorrectingPosition => {
                    // Staying in a bow substate — keep pairing.
                }
                _ => {
                    // Leaving bow substates — clear the pairing.
                    self.update_shield_bearer_before_me(None);
                }
            }
        }

        // Combat-neighbour teardown on leaving a line mode. Original's
        // "old" and "new" switches both inspect the incoming `substate`
        // parameter (rather than mCurrentSubstate for the first switch), so
        // their modes can never differ. Preserve that quirk: links assigned
        // immediately before entering the running-to-phalanx state must survive.
        if self.left_combat_neighbour.is_some() || self.right_combat_neighbour.is_some() {
            let line_mode_for = |s: Substate| -> u8 {
                match s {
                    Substate::AttackingPhalanx
                    | Substate::AttackingRunningToPhalanx
                    | Substate::AttackingProtectingWithShield => 1,
                    s if s.is_real_swordfight() => 2,
                    _ => 0,
                }
            };
            let old_mode = line_mode_for(substate);
            let new_mode = line_mode_for(substate);
            if new_mode == 0 || new_mode != old_mode {
                // The original game clears the left combat neighbour and
                // clear the right combat neighbour, which clears the reverse
                // link on each neighbour as well as our local pointers.
                // Leaving only the local half cleared makes a former neighbour
                // incorrectly believe it is not the left/right end of a
                // phalanx on its next timer tick.
                self.clear_combat_neighbours();
            }
        }

        // Release the held shooting point and archery sector when
        // the new substate is none of the archer-wait / archer-run /
        // overview-look / bow-fire variants.  We clear
        // `my_shooting_point` synchronously so same-tick reads (e.g.
        // the `else if self.my_shooting_point` arm in
        // `battle_decisions`) see the cleared state, but stash the
        // prior slot in the ordered actor outbox so the
        // engine's post-think drain can run the `set_owner(None)`
        // write — `set_state` doesn't have `&mut AiGlobalState`.
        // The archery-sector counter is released the same way.
        self.release_archery_reservation_for_substate(substate);

        // Leaving STATE_SEEKING also runs
        // `delete_all_detectables(Beggar)` and zeroes
        // `beggar_to_examine`.  Queue the detectable scrub and clear
        // the field directly so the next seek-cycle can re-populate
        // them cleanly.
        if self.base.current_state == AiState::Seeking && state != AiState::Seeking {
            self.base
                .outbox
                .actor
                .delete_detectable_type(crate::element::DetectableType::Beggar);
            self.beggar_to_examine = None;
        }

        // Fire `filter_ai_event(source, AI_STATE_CHANGE_TO_*)`
        // inside `set_state` whenever `current_substate != substate`,
        // *before* the raw state/substate assignment so the script
        // reads the outgoing state.  Source = `primary_target` for
        // Attacking/Menacing/Fleeing, otherwise `me`.
        // Engine access isn't available here, so queue the
        // notification for the post-think dispatcher to drain in
        // order.
        if self.base.current_substate != substate {
            // Work done before changing state (most importantly stopping) belongs
            // inside the synchronous state-change boundary. Detach that prefix
            // so the engine applies it before FilterAIEvent and before the
            // attentive-mode tail below. Leaving an empty prefix as `None`
            // avoids an unnecessary recursive drain.
            let actor_effects_before_callback = self
                .base
                .outbox
                .actor
                .has_boundary_work()
                .then(|| std::mem::take(&mut self.base.outbox.actor));
            let source = match state {
                AiState::Attacking | AiState::Menacing | AiState::Fleeing => {
                    AiStateChangeSource::from_optional_human(self.base.primary_target)
                }
                _ => AiStateChangeSource::SelfActor,
            };
            self.base
                .queue_state_change(state, substate, source, actor_effects_before_callback);
        }
        if opens_eyes {
            self.base
                .outbox
                .reentrant
                .owner_work
                .push(AiOwnerWork::SetEyeStatus(
                    crate::element::EyeStatus::LookForward,
                ));
        }

        tracing::trace!(
            me = self.base.me,
            timer_ring = self.base.when_does_timer_ring,
            from_state = ?self.base.current_state,
            from_substate = ?self.base.current_substate,
            to_state = ?state,
            to_substate = ?substate,
            "set_state"
        );
        self.base.set_ai_state(state);
        self.base.current_substate = substate;

        // Pick the new `attentive` flag based on the state/substate
        // pair and call `set_attentive_mode(target, fast_officer)`.
        // We replicate the decision table here and queue the request
        // for the engine to apply (engine/ai.rs drains
        // `pending_set_attentive_mode` post-think to flip the soldier
        // flags + book the transition animation when posture is
        // Upright).
        let bfalse_if_not_forced = self.forced_attentive;
        let (target_attentive, fast_officer_variant) = match (state, substate) {
            (AiState::Sleeping, _) | (AiState::Default, _) => (bfalse_if_not_forced, false),

            (AiState::Wondering, s) => match s {
                // Take-money cascade.
                Substate::WonderingMoneyReactiontime
                | Substate::WonderingApproachingMoney
                | Substate::WonderingRunningForMoney
                | Substate::WonderingTakingMoney
                // Brawl cascade.
                | Substate::WonderingBrawlReactiontime
                | Substate::WonderingBrawlApproaching
                | Substate::WonderingBrawlHitting
                | Substate::WonderingBrawlGotHit
                | Substate::WonderingBrawlRecovering
                | Substate::WonderingApproachingToLoot
                | Substate::WonderingLooting
                | Substate::WonderingWatchingForMoreMoney
                | Substate::WonderingWatching
                | Substate::WonderingWatchingWhistling => (true, false),
                Substate::WonderingUnderNet => (bfalse_if_not_forced, false),
                _ => (bfalse_if_not_forced, false),
            },

            (AiState::Seeking | AiState::Fleeing, s) => match s {
                Substate::SeekingSoldierCalledByOfficer
                | Substate::SeekingSoldierGoToOfficer
                | Substate::SeekingSoldierGetInstructedByOfficer
                | Substate::SeekingSoldierReturnToOfficer
                | Substate::SeekingSoldierGiveReportToOfficer
                | Substate::SeekingGroupGetInstructedByOfficer
                | Substate::SeekingCharlySentToOfficer
                | Substate::SeekingCharlyGoToOfficer
                | Substate::SeekingCharlyGoToOfficerSeen
                | Substate::SeekingCharlyGetLectureByOfficer
                | Substate::SeekingCharlyGetLectureByOfficer2 => {
                    // Officer-fast transition variant.
                    (bfalse_if_not_forced, true)
                }
                Substate::SeekingLookingResurrectedCharly
                | Substate::SeekingHeardstepsPreReactiontime => (bfalse_if_not_forced, false),
                Substate::SeekingGotStopEvent => {
                    // Attentive-mode selection does nothing
                    // for GotStop, but it still falls through to the shared
                    // yellow-alert status tail.
                    self.base.outbox.actor.set_attentive_mode = None;
                    self.set_alert_status(crate::ai::AlertLevel::Yellow);
                    return self.finish_set_state(substate);
                }
                _ => (true, false),
            },

            (AiState::Menacing, _) => (true, false),

            (AiState::Attacking, s) => match s {
                Substate::AttackingTooProudToAttack
                | Substate::AttackingTooProudToAttackOverview
                | Substate::AttackingTooProudToAttackApproach => (false, false),
                _ => (true, false),
            },
        };

        // Don't pre-cache `will_be_attentive` here — `set_soldier_attentive_mode`
        // flips it when it launches the `EnterAttentiveMode` element and
        // short-circuits if the flag already matches `target`.  Pre-caching
        // skipped the element launch, which in turn meant the
        // `TransitionWaitingUprightWaitingAlerted` lean-forward animation
        // never played.  `set_attentive_mode` owns the flag flip.
        self.base
            .outbox
            .actor
            .queue_set_attentive_mode(AttentiveModeEffect::new(
                target_attentive,
                fast_officer_variant,
            ));

        // `change_alert_status` writes `alert` from the same
        // (state, substate) table and calls `set_alert_status(alert)`
        // at the end.  Without this, a soldier that briefly
        // transitioned through Wondering/Seeking/Attacking keeps
        // whatever alert they had before, and the per-frame
        // overall-alert sweep (engine/ai.rs:
        // `update_overall_villain_alert`) sees a lingering Yellow/Red
        // so the music never returns to Quiet after combat resolves.
        use crate::ai::AlertLevel;
        let alert = match state {
            AiState::Sleeping | AiState::Default => AlertLevel::Green,
            AiState::Wondering | AiState::Seeking | AiState::Fleeing | AiState::Menacing => {
                AlertLevel::Yellow
            }
            AiState::Attacking => AlertLevel::Red,
        };
        self.set_alert_status(alert);

        if debug_decision_path {
            eprintln!(
                "AIDECISION frame={} owner={} stage=set_state_done now={:?}/{:?} couldnt={} already={} owner_work_after={:?}",
                self.base.cached_frame,
                self.base.me,
                self.base.current_state,
                self.base.current_substate,
                self.base.couldnt_reachpoint,
                self.base.already_on_point,
                self.base.outbox.reentrant.owner_work,
            );
        }

        self.finish_set_state(substate)
    }

    /// Preserve the enemy state-change archery teardown when a caller reaches
    /// the common AI state assignment without entering this override.
    fn release_archery_reservation_for_substate(&mut self, substate: Substate) {
        if (self.my_shooting_point.is_none() && self.my_archery_sector.is_none())
            || matches!(
                substate,
                Substate::AttackingArcherWaitOnArcheryPath
                    | Substate::AttackingArcherWaitOnArcheryPathBending
                    | Substate::AttackingArcherRunOnShootingPath
                    | Substate::AttackingArcherRunOnShootingPathFinalSprint
                    | Substate::AttackingArcherRunOnShootingPathTurn
                    | Substate::AttackingOverviewLookLeft
                    | Substate::AttackingOverviewLookRight
                    | Substate::AttackingBowShooting
                    | Substate::AttackingBowLoading
                    | Substate::AttackingBowAiming
                    | Substate::AttackingBowObservingLoading
                    | Substate::AttackingBowObserving
            )
        {
            return;
        }
        if let Some(prior) = self.my_shooting_point.take() {
            self.base
                .outbox
                .actor
                .archery_reservation_release
                .shooting_point = Some(prior.into());
        }
        if self.my_archery_sector.is_some() {
            self.base
                .outbox
                .actor
                .archery_reservation_release
                .release_sector = true;
        }
    }

    fn finish_set_state(&mut self, substate: Substate) {
        self.base
            .register_log_line(LogLineType::ChangeState, substate as u16);
    }

    /// Flag that this soldier is about to launch (or is executing) a
    /// special-strike sequence.  Called by engine-side launchers at
    /// the two sites that begin a special-strike sequence:
    /// `tick_enemy_sword_attacks` (delayed strike) and
    /// `ConsiderToBeginParade` (counter-strike).
    ///
    /// The flag gates `tick_enemy_sword_attacks` from proposing a second
    /// strike while one is in flight, and is cleared by per-tick
    /// reconciliation once the sequence no longer exists (any reason
    /// — natural completion or interruption), making the old wedge
    /// impossible by construction.
    pub fn begin_special_strike(&mut self) {
        self.pending_special_strike = true;
        self.set_state(
            AiState::Attacking,
            Substate::AttackingSwordfightSpecialStrike,
        );
    }

    /// Reconcile `pending_special_strike` against the sequence
    /// manager.  Called once per tick from
    /// `engine/melee.rs::tick_enemy_sword_attacks`.  If the flag is
    /// set but no sword-strike sequence is active for this actor,
    /// clear the flag and relaunch the 20-frame swordfight heartbeat
    /// — this is the single chokepoint that fires on *any* path that
    /// ends the sequence (natural completion, `terminate_sequence`,
    /// `stop_owner`, `friday_evening_cleanup`), not just an EventDone
    /// path.
    pub fn reconcile_special_strike(&mut self, has_active_strike: bool, frame: u32) {
        if !self.pending_special_strike {
            return;
        }
        // This reconciler stands in for the Original's synchronous
        // completion-event decision tick only when that tick is admissible. Tick admission
        // retains EventDone under every non-script AI lock (including the
        // AILOCK_FREEZE held by a Strangle victim), so the observable
        // special-strike substate must remain unchanged until unlock.
        if self.base.ai_is_locked() {
            return;
        }
        // A synchronous combat reaction may legitimately replace the
        // special-strike state (for example, entering Parade) while stopping
        // its sequence. In that case the latch is stale cancellation
        // bookkeeping; it must not overwrite the newer state on the later
        // reconciliation pass.
        if !matches!(
            self.base.current_substate,
            Substate::AttackingSwordfight | Substate::AttackingSwordfightSpecialStrike
        ) {
            self.pending_special_strike = false;
            return;
        }
        if !has_active_strike {
            if self.base.current_substate == Substate::AttackingSwordfightSpecialStrike {
                self.finish_special_strike(frame);
            } else {
                self.pending_special_strike = false;
            }
        }
    }

    /// Match the legacy `EVENT_DONE` / `EVENT_TIMER` exit from the explicit
    /// special-strike substate. The same transition is also used by the
    /// cancellation reconciler when no completion event can be delivered.
    pub fn finish_special_strike(&mut self, frame: u32) {
        self.pending_special_strike = false;
        self.set_state(AiState::Attacking, Substate::AttackingSwordfight);
        self.base.launch_timer(20, frame);
        self.next_sword_strike_frame = frame + 20;
    }

    // -----------------------------------------------------------------------
    // Movement helpers — bundle set_state + go_to/go_near/go_to_speed
    //
    // Enforces "Shape 1" contract: every movement order issued by the AI
    // must specify the substate the AI is transitioning to.  Rationale:
    // `engine/movement.rs::process_pending_ai_orders` halts the actor
    // before dispatching the new move (`halt()` inside `go_to()`), and
    // the halt-teardown suppresses the EVENT_DONE that would normally
    // reach the AI.  Under the original contract this is safe because
    // the caller of `go_to()` also does a `set_state()` right before
    // — the AI is already in the new substate when the torn-down
    // sequence's EventDone would have arrived, so suppressing it is
    // correct.  In our port the halt fires in a separate tick,
    // decoupled from the AI's
    // set_state, so a caller that forgot to transition would leave the AI
    // wedged in a "waiting" substate (Parade/Reactiontime/etc.) with no
    // way out.  These wrappers remove the split: the substate commit is
    // in the same call as the movement intent; there's no way to queue a
    // move without naming the new substate.
    // -----------------------------------------------------------------------

    /// Transition to `(state, substate)` and queue a movement to `destination`.
    /// See the section comment above for why state+substate are required.
    #[track_caller]
    pub fn go_to(
        &mut self,
        state: AiState,
        substate: Substate,
        destination: Position,
        flags: crate::ai::GotoFlags,
        ctx: &AiContext,
    ) {
        self.set_state(state, substate);
        self.base.go_to(destination, flags, ctx);
    }

    /// Like [`EnemyAi::go_to`] but with a speed modifier.
    #[track_caller]
    pub fn go_to_speed(
        &mut self,
        state: AiState,
        substate: Substate,
        destination: Position,
        flags: crate::ai::GotoFlags,
        speed: f32,
        ctx: &AiContext,
    ) {
        self.set_state(state, substate);
        self.base.go_to_speed(destination, flags, speed, ctx);
    }

    /// Transition to `(state, substate)` and queue a "go near" movement
    /// (stops within `distance` of the destination).
    #[track_caller]
    pub fn go_near(
        &mut self,
        state: AiState,
        substate: Substate,
        destination: Position,
        distance: i32,
        flags: crate::ai::GotoFlags,
        ctx: &AiContext,
    ) {
        self.set_state(state, substate);
        self.base.go_near(destination, distance, flags, ctx);
    }

    /// Apply common patrol geometry through enemy state changes.
    /// Original's default-state transition clears alert and authors the
    /// leave-attentive element before the following movement.
    fn coordinate_patrol(
        &mut self,
        info: &StimulusInfo,
        ctx: &AiContext,
        patrol_chief_position: Position,
    ) {
        let Some(action) = self
            .base
            .prepare_patrol_coordinate(info, ctx, patrol_chief_position)
        else {
            return;
        };

        match action {
            PatrolCoordinateAction::FaceChief { target } => {
                self.base.face_position_with_ctx(target, ctx);
            }
            PatrolCoordinateAction::Walk {
                target,
                speed_factor,
            } => {
                let first_new_order = self.base.outbox.actor.orders.len();
                let flags = GotoFlags::NO_HALT
                    | GotoFlags::DONT_STOP
                    | self.base.default_path_walking_flags;
                self.go_to_speed(
                    AiState::Default,
                    Substate::DefaultPatrolEnroute,
                    target,
                    flags,
                    speed_factor,
                    ctx,
                );
                self.hold_new_orders_behind_attentive(first_new_order);
            }
            PatrolCoordinateAction::Run { target } => {
                let first_new_order = self.base.outbox.actor.orders.len();
                self.go_to(
                    AiState::Default,
                    Substate::DefaultPatrolEnrouteRunning,
                    target,
                    GotoFlags::RUN | GotoFlags::NO_HALT | GotoFlags::DONT_STOP,
                    ctx,
                );
                self.hold_new_orders_behind_attentive(first_new_order);
            }
        }
    }

    /// Entering the default state disables attentive mode before the
    /// following movement request. Only hold the movement when that call actually
    /// changes `will_be_attentive`; Original's no-change call returns without
    /// launching a transition element.
    fn hold_new_orders_behind_attentive(&mut self, first_new_order: usize) {
        let launches_transition = self
            .base
            .outbox
            .actor
            .set_attentive_mode
            .is_some_and(|request| request.target != self.will_be_attentive);
        for order in &mut self.base.outbox.actor.orders[first_new_order..] {
            order.after_attentive_mode = launches_transition;
        }
    }

    // -----------------------------------------------------------------------
    // Think — main stimulus dispatcher
    // -----------------------------------------------------------------------

    /// Main entry point for stimulus processing. Routes the stimulus
    /// to the appropriate Think sub-method based on its type.
    pub fn think(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> bool {
        // Cache engine state for say() / forbidden remarks
        self.base.cached_frame = ctx.frame;
        self.base.cached_in_building = ctx.in_building;

        let debug_decision_path =
            decision_path_debug_enabled() && decision_path_debug_matches(ctx.frame, self.base.me);
        if debug_decision_path {
            eprintln!(
                "AIDECISION frame={} owner={} co={:?} stage=think_enter depth={}/open={} stimulus={:?} state={:?}/{:?} primary={:?} rider={} couldnt={} already={} list_them={:?} owner_work={:?}",
                ctx.frame,
                self.base.me,
                ctx.original_creation_order,
                self.base.think_recursion_depth,
                self.base.open_end_think_frames,
                stimulus.stimulus_type,
                self.base.current_state,
                self.base.current_substate,
                self.base.primary_target,
                ctx.self_is_rider,
                self.base.couldnt_reachpoint,
                self.base.already_on_point,
                self.list_them,
                self.base.outbox.reentrant.owner_work,
            );
        }

        let stimulus_type = stimulus.stimulus_type;
        self.base
            .debug_macro_lifecycle(ctx, "think_enter", stimulus_type);

        tracing::trace!(
            me = self.base.me,
            frame = ctx.frame,
            ?stimulus_type,
            state = ?self.base.current_state,
            substate = ?self.base.current_substate,
            timer_ring = self.base.when_does_timer_ring,
            "think: ENTRY"
        );
        self.base
            .register_log_line(LogLineType::Event, stimulus_type as u16);

        // Pre-think: check locks, queue if busy, etc.
        if !self.start_think(stimulus, ctx, global.freeze) {
            if stimulus_type == StimulusType::EventAfterScriptGoOn {
                self.base.outbox.reentrant.engine_drains_after_script_go_on = false;
            }
            self.end_think(sim, global, ctx, tick, grid);
            self.base
                .debug_macro_lifecycle(ctx, "think_rejected_return", stimulus_type);
            return true;
        }

        // The script filter gate is applied by the engine *before*
        // this function is entered — see `Engine::filter_stimulus`.
        // Callers invoke it prior to borrowing the entity for
        // `think()`, so by the time we get here, the stimulus has
        // already passed the script's `filter_ai_event`.  Cascade
        // `self.think(sim, ...)` calls below re-dispatch
        // internally-generated stimuli and intentionally skip the
        // filter (see the cascade-divergence note on those sites).

        self.update_new_task_priority(stimulus);

        let return_value = match stimulus_type {
            // Expected events — drive state progression
            StimulusType::EventReachPoint
            | StimulusType::EventDone
            | StimulusType::EventTimer
            | StimulusType::EventSyncCharly
            | StimulusType::CallCoordinate
            | StimulusType::CallInstruction
            | StimulusType::CallReport
            | StimulusType::EventGaloppLoopEnd
            | StimulusType::EventMyTalk0
            | StimulusType::EventMyTalk1
            | StimulusType::EventMyTalk2
            | StimulusType::EventMyTalk3
            | StimulusType::CallYourTalk0
            | StimulusType::CallYourTalk1
            | StimulusType::CallYourTalk2
            | StimulusType::CallYourTalk3 => {
                self.think_expected_event(sim, stimulus, global, ctx, tick, grid)
            }

            // Unexpected events — may interrupt current behavior
            StimulusType::EventOutOfView
            | StimulusType::EventCouldntReachPoint
            | StimulusType::EventImpossible
            | StimulusType::EventMissesCharly
            | StimulusType::EventSeesCharly
            | StimulusType::EventObjectAway
            | StimulusType::EventFitAgain
            | StimulusType::EventAfterScriptGoOn
            | StimulusType::EventQuitSwordfight
            | StimulusType::EventSwordStrike
            | StimulusType::EventSeesSoldier
            | StimulusType::CallHey
            | StimulusType::CallMrOfficerIAmBack
            | StimulusType::CallAlert
            | StimulusType::CallCombatAlert
            | StimulusType::CallGoToOfficer
            | StimulusType::CallCharlyIsBack
            | StimulusType::EventWaspAway
            | StimulusType::CallPatrolCoordinate
            | StimulusType::EventNetAway
            | StimulusType::EventSeesBeggar
            | StimulusType::EventSeesBrawl
            | StimulusType::CallFinishBrawl
            | StimulusType::CallCleanUpAfterBrawl
            | StimulusType::EventAdversaryWeak
            | StimulusType::EventAfterCombatInjury
            | StimulusType::EventGoodStrike
            | StimulusType::EventLethalStrike
            | StimulusType::EventEnemyNear => {
                self.think_unexpected_event(sim, stimulus, global, ctx, tick, grid)
            }

            // Alerting events — high-priority perception
            StimulusType::EventView
            | StimulusType::EventHear
            | StimulusType::EventPcShotAtMe
            | StimulusType::EventSeesBody
            | StimulusType::EventSeesObject
            | StimulusType::EventSeesFriendInTrouble
            | StimulusType::EventGotHit
            | StimulusType::EventLoseConsciousness
            | StimulusType::EventGetArrow
            | StimulusType::EventEnterSwordfight
            | StimulusType::CallLookThere
            | StimulusType::EventApple
            | StimulusType::EventStone
            | StimulusType::CallTowerGuardAlert
            | StimulusType::CallTowerGuardCallsMe
            | StimulusType::EventDoorCombat
            | StimulusType::EventSeesShadow
            | StimulusType::EventArrowLaunched
            | StimulusType::EventStop => {
                self.think_alerting_event(sim, stimulus, global, ctx, tick, grid)
            }

            StimulusType::EventReturnToDuty => {
                self.return_to_duty_default(sim, ctx, tick);
                // This arm never assigns the return value, so it
                // returns `false` (the default).  Callers test the
                // bool to decide whether to re-dispatch / continue
                // the cascade, so the false return matters.
                false
            }

            _ => {
                tracing::warn!(
                    "Unknown stimulus type in EnemyAi::think: {:?}",
                    stimulus_type
                );
                false
            }
        };

        if !(stimulus_type == StimulusType::EventAfterScriptGoOn
            && self.base.outbox.reentrant.engine_drains_after_script_go_on)
        {
            self.end_think(sim, global, ctx, tick, grid);
        }
        self.base
            .debug_macro_lifecycle(ctx, "think_return", stimulus_type);
        return_value
    }

    // -----------------------------------------------------------------------
    // Decision-tick admission checks
    // -----------------------------------------------------------------------

    fn start_think(
        &mut self,
        stimulus: &Stimulus,
        ctx: &AiContext,
        static_ai_frozen: bool,
    ) -> bool {
        self.start_think_pre_filter(stimulus);
        self.start_think_post_filter(stimulus, ctx, static_ai_frozen)
    }

    /// Decision-tick admission work which precedes the script `FilterAIEvent` call.
    /// Kept separate so script-native SetAIState can yield through the VM at
    /// the exact callback boundary without aliasing the typed brain.
    pub(crate) fn start_think_pre_filter(&mut self, stimulus: &Stimulus) {
        let stimulus_type = stimulus.stimulus_type;

        // Reset per-think flags
        self.base.couldnt_reachpoint = false;
        self.base.already_on_point = false;
        self.base.already_turned = false;
        self.base.old_state = self.base.current_state as i32;
        self.base.think_recursion_depth += 1;

        // Track stimulus actor
        if let StimulusInfo::Human(h) = stimulus.info {
            self.base.last_stimulus_actor = Some(h);
        }

        // LOSE_CONSCIOUSNESS always goes to green alert
        if stimulus_type == StimulusType::EventLoseConsciousness {
            self.set_alert_status(AlertLevel::Green);
        }
    }

    /// Decision-tick admission work after `FilterAIEvent`. The return value is the
    /// ordinary Think admission decision; SetAIState intentionally observes
    /// these gates but ignores the bool before starting an area search or panic.
    pub(crate) fn start_think_post_filter(
        &mut self,
        stimulus: &Stimulus,
        ctx: &AiContext,
        static_ai_frozen: bool,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;

        if !self
            .base
            .admit_think_before_role_gates(stimulus, static_ai_frozen)
        {
            return false;
        }

        // Original's first unconscious gate reads the actor flag, not the AI
        // substate. That distinction matters while a postponed injury leaves
        // an unconscious actor in a non-sleeping state such as
        // DefaultScriptDriven: ordinary calls must still be refused before
        // they can relay work through a retained patrol chief.
        if ctx.self_is_unconscious {
            match stimulus_type {
                StimulusType::EventLoseConsciousness => {}
                StimulusType::EventFitAgain => {
                    if ctx.posture == crate::element::Posture::Carried {
                        self.base.register_log_line(LogLineType::EventRefused, 7);
                        return false;
                    }
                }
                _ => {
                    self.base.register_log_line(LogLineType::EventRefused, 8);
                    return false;
                }
            }
        }

        if !self.base.admit_think_after_role_gates(stimulus, ctx) {
            return false;
        }

        // Handle special events processed during decision-tick admission. In Original
        // these run after the timer, dead, and sleeping-unconscious gates;
        // notably, a second unconsciousness stimulus cannot rewrite the AI
        // state of an actor whose death transition has already completed.
        match stimulus_type {
            StimulusType::EventLoseConsciousness => {
                self.base.break_macro();
                self.base.clear_emoticon();
                if self.base.current_substate.is_take_money()
                    || self.base.current_substate.is_fight_for_money()
                {
                    self.forget_all_nearby_coins(ctx);
                }
                self.set_state(AiState::Sleeping, Substate::SleepingUnconscious);
                self.base.outbox.recovery.set_eye_status =
                    Some(crate::element::EyeStatus::DieOrGetUnconscious);
                self.set_alert_status(AlertLevel::Green);
                self.base.sorrow_level = 0;
                self.forget_attentive_mode();
                self.base.register_log_line(LogLineType::EventRefused, 13);
                return false;
            }
            StimulusType::EventWasp => {
                self.base.break_macro();
                self.base.set_emoticon(EmoticonType::Thunderstorm);
                self.set_state(AiState::Wondering, Substate::WonderingWaspInArmour);
                self.base.outbox.recovery.set_eye_status = Some(crate::element::EyeStatus::Closed);
                self.base.sorrow_level = 0;
                self.forget_attentive_mode();
                self.base.register_log_line(LogLineType::EventRefused, 14);
                return false;
            }
            StimulusType::EventNet => {
                self.base.break_macro();
                self.set_state(AiState::Wondering, Substate::WonderingUnderNet);
                self.base.outbox.recovery.set_eye_status = Some(crate::element::EyeStatus::Closed);
                self.base.sorrow_level = 0;
                self.forget_attentive_mode();
                self.base.register_log_line(LogLineType::EventRefused, 15);
                return false;
            }
            _ => {}
        }

        true
    }

    // -----------------------------------------------------------------------
    // Decision-tick completion — post-tick event dispatch
    // -----------------------------------------------------------------------

    pub(crate) fn end_think(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        _global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        _grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        // The original game's end-think phase dispatches this event here and runs the
        // script FilterAIEvent gate before dispatch. Queue these as
        // same-frame self-stimuli so the engine-side drain can apply
        // that filter without re-entering the script VM through this
        // borrowed AI object.

        let mut queued_completion = false;

        // Post a reachability-failure event if a movement request failed
        if self.base.couldnt_reachpoint {
            self.base.couldnt_reachpoint = false;
            if self.base.think_recursion_depth < 100 {
                self.base
                    .outbox
                    .reentrant
                    .self_stimuli
                    .push(StimulusType::EventCouldntReachPoint.into());
                queued_completion = true;
            } else if self.base.think_recursion_depth < 111 {
                // 100..=110 asserts and bails to return_to_duty;
                // 111+ does nothing (the assert already fired upstream).
                self.return_to_duty_default(sim, ctx, tick);
            }
        }

        // Post ReachPoint event if movement was already at destination
        if self.base.already_on_point {
            self.base.already_on_point = false;
            if self.base.think_recursion_depth < 100 {
                self.base
                    .outbox
                    .reentrant
                    .self_stimuli
                    .push(StimulusType::EventReachPoint.into());
                queued_completion = true;
            } else if self.base.think_recursion_depth < 111 {
                // 100..=110 asserts and bails to return_to_duty;
                // 111+ does nothing (the assert already fired upstream).
                self.return_to_duty_default(sim, ctx, tick);
            }
        }

        // Post Done event if Turn was already facing the right direction
        if self.base.already_turned {
            self.base.already_turned = false;
            if self.base.think_recursion_depth < 100 {
                self.base
                    .outbox
                    .reentrant
                    .self_stimuli
                    .push(StimulusType::EventDone.into());
                queued_completion = true;
            } else if self.base.think_recursion_depth < 111 {
                // 100..=110 asserts and bails to return_to_duty;
                // 111+ does nothing (the assert already fired upstream).
                self.return_to_duty_default(sim, ctx, tick);
            }
        }

        if queued_completion {
            // The original game recursively dispatches the completion update
            // *before* its decrement, so the cascade's ancestor frames stay
            // open and the recursion depth climbs one per nested Think —
            // that climb is what makes the 100.. return-to-duty failsafe
            // reachable. This frame stays open until the cascade ends (see
            // `open_end_think_frames`).
            self.base.open_end_think_frames = self.base.open_end_think_frames.saturating_add(1);
        } else if self.base.defer_end_think_for_engine_completion() {
            // Rust learns an engine-owned movement failure after releasing this
            // AI borrow. Keep the original game's end-of-tick frame alive until that
            // synchronous path verdict is surfaced.
        } else {
            // No continuation was queued: this is the innermost Think of the
            // cascade, so the entire chain of still-open ancestor frames
            // unwinds with it — the deferred equivalent of the stacked
            // completion decrements the original game performs while returning out
            // of the nested calls.
            let open = std::mem::take(&mut self.base.open_end_think_frames);
            self.base.think_recursion_depth = self
                .base
                .think_recursion_depth
                .saturating_sub(1)
                .saturating_sub(open);
        }
    }

    // -----------------------------------------------------------------------
    // Update new-task priority
    // -----------------------------------------------------------------------

    fn update_new_task_priority(&mut self, stimulus: &Stimulus) {
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

    /// Ordinary return with no special duty-transition flags.
    #[track_caller]
    fn return_to_duty_default(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
    }

    /// Change virtual enemy state before arming the incoming state's timer.
    #[track_caller]
    fn set_state_with_timer(
        &mut self,
        state: AiState,
        substate: Substate,
        frames: u32,
        ctx: &AiContext,
    ) {
        self.set_state(state, substate);
        self.base.launch_timer(frames, ctx.frame);
    }

    pub fn return_to_duty(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        flags: DutyFlags,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        self.investigating_distraction = false;

        // Removing all beggar detectables is synchronous in
        // original game. In particular, selecting the next seek point can return to duty after
        // Area search queued beggars earlier in the same borrowed AI dispatch;
        // The ordered mutation list applies those additions before this scrub.
        self.base
            .outbox
            .actor
            .delete_detectable_type(crate::element::DetectableType::Beggar);
        self.beggar_to_examine = None;
        self.beggar_is_npc = false;
        self.clear_swordstrike_experiences();
        // Release any stare/follow target before the
        // report-to-officer / look-for-help branches so the focus releases
        // on every exit path, including the early returns.
        self.base.outbox.actor.set_unfocus();
        self.fleeing_seen_enemy_counter = 0;

        // Report to officer after seeking?
        if self.seek_flags.contains(SeekFlags::REPORT_OFFICER_AFTER)
            && self.base.antagonist.is_some()
            && !flags.contains(DutyFlags::BECAUSE_COULDNT_REACHPOINT)
        {
            self.set_state(AiState::Seeking, Substate::SeekingSoldierReturnToOfficer);
            self.base.clear_emoticon();
            self.base
                .go_near(self.officers_position, 40, GotoFlags::RUN, ctx);
            if self.base.already_on_point {
                self.base.already_on_point = false;
            } else {
                self.base.launch_timer(20, ctx.frame);
                return;
            }
        }

        // Look for help after seeking?
        if self.seek_flags.contains(SeekFlags::LOOK_FOR_HELP_AFTER)
            && !flags.contains(DutyFlags::BECAUSE_COULDNT_REACHPOINT)
        {
            self.seek_flags = SeekFlags::empty();
            if self.get_rank() == ProfileRank::Soldier
                && self.alert_officer(sim, self.seek_center, 0, ctx, tick)
            {
                return;
            }
        }

        // Reset state
        self.base.friends_are_alerted = false;
        self.seek_flags = SeekFlags::empty();
        self.base.sorrow_level = 0;
        self.phalanx_aborted = false;
        self.base.antagonist = None;
        self.current_task_priority = self.minimal_task_priority;

        // "If you were searching charly, forget him." When the NPC has any
        // `DETECTABLE_MISSED_FRIEND` entries (the search-for-charly path
        // placed at least one), record the abandoned `checkpoint_charly`
        // in `missed_in_action` and clear the checkpoint pointer so
        // subsequent mission scripts querying the list see the right
        // entries.
        if ctx.self_detectable_missed_friend_count > 0
            && let Some(checkpoint_charly) = self.base.checkpoint_charly
        {
            self.base.missed_in_action.push(checkpoint_charly.get());
            self.base.set_checkpoint_charly(None);
        }

        // Did you forget some money?
        //
        // Also gates on a missing interesting object or
        // no angry officer nearby: if we still remember a specific
        // coin and an officer is sermoning a finished brawl right next to
        // it, back off (the angry officer will discipline anyone who
        // re-engages).
        let angry_officer_near_coin = self.base.interesting_object.is_some()
            && ctx
                .entity_position(self.base.interesting_object)
                .is_some_and(|p| self.is_any_angry_officer_near(p, tick));
        if (self.base.current_substate.is_take_money()
            || self.base.current_substate.is_fight_for_money())
            && self.answer_question(Question::ShallITakeMoney, ctx)
            && !flags.contains(DutyFlags::BECAUSE_COULDNT_REACHPOINT)
            && !self.other_seen_money.is_empty()
            && !angry_officer_near_coin
        {
            if self.base.interesting_object.is_none() {
                // Nearest-seen-money selection and removal: picks the
                // closest live coin (maximum norm, +300 layer malus) after
                // sweeping inactive entries, rather than popping by
                // insertion order.
                if let Some(coin) = self.get_nearest_seen_money_and_remove_it_from_list(ctx) {
                    self.base.interesting_object = Some(AiEntityHandle::new(coin));
                }
            }
            // Approach the interesting-object position. Look up the freshly
            // adopted money pickup in the per-tick view map. If the
            // pickup was swept out from under us between snapshot time
            // and now (another NPC grabbed it, script removed it), skip
            // the branch and fall through to the patrol/ale checks.
            if let Some(obj_pos) = ctx.entity_position(self.base.interesting_object) {
                self.go_near(
                    AiState::Wondering,
                    Substate::WonderingApproachingMoney,
                    obj_pos,
                    parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                    GotoFlags::FIND_ACCESSIBLE,
                    ctx,
                );
                self.base.launch_timer(5, ctx.frame);
                return;
            }
            // Stale handle — drop it so we don't re-attempt forever.
            self.base.interesting_object = None;
        }

        // Return to patrol point?
        if self.return_to_patrol_point.sector.is_some() {
            if !self.base.patrol.is_empty() {
                self.set_state(AiState::Default, Substate::DefaultPatrolChiefReturnToPatrol);
                self.base
                    .go_to(self.return_to_patrol_point, GotoFlags::empty(), ctx);
                self.return_to_patrol_point.sector = None;
                return;
            }
            self.return_to_patrol_point.sector = None;
        }

        // Remember ale?
        if !self.other_seen_ale.is_empty() && !flags.contains(DutyFlags::BECAUSE_COULDNT_REACHPOINT)
        {
            self.base.interesting_object = Some(AiEntityHandle::new(self.other_seen_ale.remove(0)));
            self.base.object_of_desire = self.base.interesting_object;
            // Same rationale as the money branch above — if the ale
            // bottle was removed before the snapshot, skip this
            // branch and fall through to `initialize_patrol`.
            if let Some(obj_pos) = ctx.entity_position(self.base.interesting_object) {
                self.go_near(
                    AiState::Wondering,
                    Substate::WonderingApproachingAle,
                    obj_pos,
                    parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                    GotoFlags::FIND_ACCESSIBLE,
                    ctx,
                );
                // Returning to duty remembers where the
                // patrol was interrupted so the soldier returns there after
                // finishing this newly remembered ale.
                self.return_to_patrol_point = ctx.position;
                self.base.launch_timer(1, ctx.frame);
                return;
            }
            self.base.interesting_object = None;
            self.base.object_of_desire = None;
        }

        // The original game initializes patrol synchronously, then immediately
        // enters common return-to-duty processing. Patrol admission needs the engine's
        // entity table and can itself issue authoritative visibility queries,
        // so suspend the tail at the owner boundary instead of setting the
        // frame-deferred `needs_patrol_reinit` flag. This also lets a patrol
        // member observe the chief assignment written moments earlier and
        // perform its reciprocal member -> chief visibility query in-order.
        //
        // Clear the reconnaissance report here rather than leaving it to the
        // suspended return-to-duty processing. Because the whole return runs
        // synchronously in the reference, callers observe a cleared report
        // the instant the return completes — next-point selection reads it on the
        // very next statement to decide whether to say "ends search", and
        // against an unreset report that decision inverts. Only this path is
        // hoisted: the early returns above never reach the common tail and
        // must leave the report standing.
        self.base.my_reconnaissance_report.reset();
        let owner_boundary_positions = ctx
            .entity_views
            .iter()
            .map(|(&handle, view)| (handle, view.position))
            .collect();
        let continuation = if (100..111).contains(&self.base.think_recursion_depth) {
            AiOwnerWork::ResumeHighRecursionReturnToDutyAfterPatrolInit {
                flags,
                owner_boundary_positions,
            }
        } else {
            AiOwnerWork::ResumeReturnToDutyAfterPatrolInit {
                flags,
                defer_clear_patrol_close_post: false,
                owner_boundary_positions,
            }
        };
        self.base.outbox.reentrant.owner_work.push(continuation);
    }

    /// Resume the non-engine half of original-game enemy return-to-duty after its
    /// inline patrol initialization has returned.
    pub fn resume_return_to_duty_after_patrol_init(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        flags: DutyFlags,
        ctx: &AiContext,
        high_recursion_failsafe: bool,
    ) {
        let outgoing_state = self.base.current_state;
        let outgoing_substate = self.base.current_substate;
        let first_new_order = self.base.outbox.actor.orders.len();

        // Returning to duty clears the previous enemy-state timer before
        // deciding whether to launch a fresh bored timer.
        self.base.timer_is_running = false;
        let resumed_depth = self.base.think_recursion_depth;
        if high_recursion_failsafe && resumed_depth == 0 {
            self.base.think_recursion_depth = 100;
        }
        self.base.return_to_duty_common_stuff(sim, flags, ctx);
        if high_recursion_failsafe && resumed_depth == 0 {
            self.base.think_recursion_depth = resumed_depth;
            // The original game's high-recursion return to duty is already inside the
            // currently executing decision-tick completion branch. Its newly-set latch is
            // not revisited by that branch; it survives the unwind and is
            // cleared by the next decision-tick admission. The deferred Rust boundary has
            // no matching completion left, so prevent the generic completion
            // surfacer from converting it into an immediate self event.
            self.base.completion_latch_inside_think = false;
        }
        let incoming_state = self.base.current_state;
        let incoming_substate = self.base.current_substate;

        // Common return-to-duty processing invokes enemy state changes in the
        // original. The shared Rust common tail assigns its state directly,
        // so run the override's shooting-point and sector release here too.
        self.release_archery_reservation_for_substate(incoming_substate);

        // The shared common routine assigns Default directly instead of
        // entering through enemy state selection. Preserve the
        // shield-bearer state transition too: leaving the three
        // protection substates clear both the rear archer and the archer's
        // reciprocal forward shield bearer before the next NPC owner runs.
        if self.archer_behind_me.is_some()
            && !matches!(
                incoming_substate,
                Substate::AttackingProtectingWithShield
                    | Substate::AttackingPhalanx
                    | Substate::AttackingRunningToPhalanx
            )
        {
            let old_archer = self
                .archer_behind_me
                .take()
                .expect("checked archer-behind-me presence");
            self.base.outbox.reentrant.cross_npc_actions.push(
                CrossNpcAction::SetShieldBearerBeforeMe {
                    target: old_archer.get(),
                    shield_bearer: None,
                },
            );
        }

        // When an archer leaves the bow substates, clear the forward shield
        // bearer and its reciprocal rear archer before the next AI owner runs.
        if self.shield_bearer_before_me.is_some()
            && !matches!(
                incoming_substate,
                Substate::AttackingBowShooting
                    | Substate::AttackingBowLoading
                    | Substate::AttackingBowAiming
                    | Substate::AttackingBowObservingLoading
                    | Substate::AttackingBowObserving
                    | Substate::AttackingBowRunningBehindShieldBearer
                    | Substate::AttackingBowCorrectingPosition
            )
        {
            self.update_shield_bearer_before_me(None);
        }

        // Returning to duty must synchronously clear relationships when
        // leaving Menacing. In particular, the
        // reciprocal PC guard must be cleared before a later NPC owner slot
        // runs detection refresh; an unobserved guarded PC is rejected before
        // the otherwise-authoritative visibility query.
        if outgoing_state == AiState::Menacing && self.base.current_state != AiState::Menacing {
            self.set_guarded_pc(None);
        }

        // Returning to duty also clears combat neighbours based on the
        // incoming substate: clear when the destination is neither a phalanx
        // nor a real swordfight substate. Queue reciprocal clears before dropping local
        // links, matching reciprocal combat-neighbour clearing.
        let incoming_keeps_combat_neighbours = matches!(
            incoming_substate,
            Substate::AttackingPhalanx
                | Substate::AttackingRunningToPhalanx
                | Substate::AttackingProtectingWithShield
        ) || incoming_substate.is_real_swordfight();
        if !incoming_keeps_combat_neighbours {
            self.clear_combat_neighbours();
        }

        // Original-game return-to-duty handling uses the enemy-specific state
        // transition. The shared Rust base performs the assignment directly.
        // Restore the Enemy override's attentive-mode tail: every Default
        // substate requests ordinary (or forced) attention, which may launch
        // LeaveAttentiveMode alongside the return route. The shared common
        // routine has already built the route because it cannot invoke the
        // Enemy override directly, so restore Original's authored
        // state-change-before-movement barrier on only the orders emitted by this
        // return-to-duty tail.
        self.base
            .outbox
            .actor
            .queue_set_attentive_mode(AttentiveModeEffect::new(self.forced_attentive, false));
        self.hold_new_orders_behind_attentive(first_new_order);

        // TODO: move the complete enemy state-change boundary into the
        // shared return-to-duty routine. This closes final owner-boundary
        // publication, but the deferred Rust model still queues the earlier
        // return-to-duty actor prefix after owner-work StateChange callbacks;
        // scripted callback observation order is not claimed exact here.

        // Preserve the corresponding script callback item explicitly.
        // Without this final FIFO entry, an older queued transition (notably
        // the init-time Default/Enroute transition) is restored after the
        // common code has already advanced the live state to
        // Default/GotoRoute.
        if outgoing_substate != incoming_substate {
            self.base
                .outbox
                .reentrant
                .owner_work
                .push(AiOwnerWork::StateChange(AiStateChangeNotification {
                    outgoing_state,
                    outgoing_substate,
                    incoming_state,
                    incoming_substate,
                    source: AiStateChangeSource::SelfActor,
                    actor_effects_before_callback: Default::default(),
                }));
        }
    }

    // -----------------------------------------------------------------------
    // React — reaction delay before responding
    // Enemy reaction behavior.
    // -----------------------------------------------------------------------

    pub fn react(&mut self, max_reactiontime: u16, ctx: &AiContext, tick: &AiPerTickData) {
        if self.is_merry_man_forest(ctx) {
            self.base.launch_timer(3, ctx.frame);
            return;
        }

        // Scaling applies only when the NPC is Lacklandist. Royalist soldiers
        // (also EnemyAi-driven) retain 1.0. The original's Easy==Hard
        // copy-paste bug remains optional for the exact Hard preset; Legendary
        // and Custom always use their resolved reaction rule.
        let modifier = if ctx.is_hostile_to_player() {
            if ctx.difficulty == crate::player_profile::DifficultyLevel::Hard
                && !tick.fix_hard_reaction_times
            {
                // Optional preservation of the retail Hard copy/paste bug.
                2.0
            } else {
                crate::player_profile::DifficultyRules::percent_as_f32(
                    ctx.difficulty.rules().reaction_time_percent,
                )
            }
        } else {
            1.0
        };

        // Use the raw profile intelligence directly — not the
        // difficulty-scaled intelligence. Using the scaled value here
        // double-applies the Easy/Hard modifier (IQ is scaled by
        // `EASY_ENEMY_IQ=0.5` on Easy, then the reaction-time formula
        // multiplies by `EASY_REACTIONTIME=2.0`), which visibly
        // stretched the reaction pause beyond what the reference
        // produces.
        let intelligence = self.soldier_profile_iq as f32;
        let time =
            ((100.0 - intelligence) * 0.01 * max_reactiontime as f32 * modifier + 1.0) as u32;
        self.base.launch_timer(time, ctx.frame);
    }

    // -----------------------------------------------------------------------
    // Select a new primary target
    // -----------------------------------------------------------------------

    pub fn get_new_primary_target(
        &mut self,
        flags: PrimaryTargetFlags,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> Option<AiEntityHandle> {
        self.get_new_primary_target_with_mult_override(flags, ctx, tick, None)
    }

    /// Variant of [`Self::get_new_primary_target`] that lets the caller
    /// substitute a locally-rebuilt `primary_target_multiplicity` map
    /// for the owner-ordered shared scratch. Swordfight observation reconsideration
    /// clears multiplicity on its rebuilt `list_them` and re-bumps from
    /// swordfighting allies in `list_us` before calling
    /// `get_new_primary_target(UNOCCUPIED_STRONGLY_PREFERRED)`.
    pub fn get_new_primary_target_with_mult_override(
        &mut self,
        flags: PrimaryTargetFlags,
        ctx: &AiContext,
        tick: &AiPerTickData,
        mult_override: Option<&std::collections::BTreeMap<HumanHandle, u32>>,
    ) -> Option<AiEntityHandle> {
        if self.list_them.is_empty() {
            return None;
        }

        let mut nearest = None;
        let mut min_distance: u16 = 65432; // Original `oo` sentinel
        let owner_view = match ctx.entity_observation(self.base.me) {
            Ok(view) => view,
            Err(reason) => {
                // TODO: establish Original invalid-layer timer-tail behavior before
                // changing this existing skip policy.
                tracing::warn!(
                    me = self.base.me,
                    ?reason,
                    "primary-target replacement skipped: owner spatial observation unavailable"
                );
                return None;
            }
        };
        let owner_world = owner_view.detection_position_world;

        for &enemy in &self.list_them {
            // Gate on `VIPS_ALLOWED || is_allowed_to_attack(enemy)`.
            // Without VIPS_ALLOWED, VIP-protection rules drop the
            // candidate (e.g. VIP soldier may only engage Robin).
            if !flags.contains(PrimaryTargetFlags::VIPS_ALLOWED)
                && !self.is_allowed_to_attack(enemy, ctx, tick)
            {
                continue;
            }

            // The original game's primary-target selection reads every persistent hostile list
            // pointer's live position. Detection snapshots are intentionally
            // incomplete on timer/reach/cross-NPC dispatches and therefore
            // cannot be used as a distance cache here.
            let target = ctx.entity_view(enemy).unwrap_or_else(|| {
                panic!(
                    "primary-target replacement owner {} has required enemy-list entry {} missing from the live entity view",
                    self.base.me, enemy
                )
            });
            // Primary-target replacement uses the raw 3D distance /
            // maximum-norm distance helpers, not AI position. `position` is
            // deliberately door-aware and can point at the committed far
            // side of a selected PassDoor; use each element's stored world
            // position for this scoring path.
            let target_world = target.detection_position_world;
            let dx = target_world.x - owner_world.x;
            let dy =
                (target_world.y - owner_world.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
            let dz = target_world.z - owner_world.z;
            // The original game's distance calculations subtract the actors'
            // world positions. World Y is map Y plus elevation,
            // so the vertical screen-plane component includes dz before the
            // isometric stretch; elevation is also retained as the 3D Z
            // component. Using map Y alone can make a target on another level
            // appear much farther away and select the wrong primary target.
            let max_norm = dx.abs().max(dy.abs()).max(dz.abs());
            if max_norm > f32::from(min_distance) {
                continue;
            }
            let mut distance = (dx * dx + dy * dy + dz * dz).sqrt() as u16;

            // Penalize already-targeted enemies.
            let mult = if let Some(map) = mult_override {
                map.get(&enemy).copied().unwrap_or(0)
            } else {
                tick.primary_target_multiplicity
                    .iter()
                    .find(|&&(h, _)| h == enemy)
                    .map(|&(_, m)| m)
                    .unwrap_or(0)
            };

            if flags.contains(PrimaryTargetFlags::UNOCCUPIED_PREFERRED) {
                distance = distance.wrapping_add((100_u16).wrapping_mul(mult as u16));
            } else if flags.contains(PrimaryTargetFlags::UNOCCUPIED_STRONGLY_PREFERRED) {
                distance = distance.wrapping_add((10_000_u16).wrapping_mul(mult as u16));
            }

            if distance < min_distance {
                min_distance = distance;
                nearest = Some(AiEntityHandle::new(enemy));
            }
        }

        nearest
    }

    // -----------------------------------------------------------------------
    // Character-based decision making
    // -----------------------------------------------------------------------

    /// `hypothetical` corresponds to the original game's hypothetical-question flag
    /// flag — when true, the outdoor branch is evaluated regardless of
    /// where the NPC currently stands. Pass `false` from live ticks and let
    /// `ctx.self_is_active` / `ctx.in_building` route to the indoor branch.
    pub fn answer_question_ex(
        &self,
        question: Question,
        ctx: &AiContext,
        hypothetical: bool,
    ) -> bool {
        // ── Drunken override ──────────────────────────────────────────
        if self.base.blood_alcohol as i32 > parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT {
            match question {
                Question::ShallITakeAle
                | Question::ShallITakeMoney
                | Question::ShallIFightForMoney
                | Question::ShallIReactOnApple
                | Question::ShallIStayOnMyPost => return true,

                Question::ShallIFollowLostEnemy
                | Question::ShallIFollowSteps
                | Question::ShallIFollowHint
                | Question::ShallIHelpFriendInTrouble
                | Question::ShallIRun
                | Question::HasTheNewTaskPriority
                | Question::ShallISeekBeforeAlertingOfficer
                | Question::ShallISeekBeforeAlertingSoldiers
                | Question::ShallISendOutSoldier
                | Question::ShallILookWhistle
                | Question::ShallIFollowWhistle => return false,
            }
        }

        // ── Outdoor / active branch ───────────────────────────────────
        // Gate: hypothetical || (active && outside building).
        if hypothetical || (ctx.self_is_active && !ctx.in_building) {
            return match question {
                Question::ShallITakeAle => {
                    self.soldier_profile_beer > 0 || self.ale_reliable_distraction
                }
                Question::ShallITakeMoney => self.soldier_profile_money > 0,
                Question::ShallIFightForMoney => self.soldier_profile_money > 0,
                Question::ShallIReactOnApple => self.soldier_profile_apple > 0,

                Question::ShallIStayOnMyPost => {
                    self.tower_guard || self.soldier_profile_duty || self.company_number == 100
                }

                Question::ShallIFollowLostEnemy => {
                    !self.combat_trainer && self.company_number != 100
                }

                Question::ShallIFollowSteps
                | Question::ShallIFollowHint
                | Question::ShallIHelpFriendInTrouble => {
                    !self.soldier_profile_duty && self.company_number != 100
                }

                Question::ShallIRun => {
                    self.soldier_profile_endurance > parameters_ai::MINVALUE_RUN as u16
                }

                Question::ShallILookWhistle => self.soldier_profile_whistle > 0,
                Question::ShallIFollowWhistle => {
                    self.soldier_profile_whistle > 1 && self.company_number != 100
                }

                Question::HasTheNewTaskPriority => self.has_the_new_task_priority(),

                Question::ShallISeekBeforeAlertingOfficer
                | Question::ShallISeekBeforeAlertingSoldiers => {
                    self.soldier_profile_initiative >= 50
                }

                Question::ShallISendOutSoldier => {
                    self.soldier_profile_initiative < 50 || !self.base.patrol.is_empty()
                }
            };
        }

        // ── Indoor branch ─────────────────────────────────────────────
        match question {
            // Asserted away upstream; safest is `false`.
            Question::ShallITakeAle
            | Question::ShallITakeMoney
            | Question::ShallIFightForMoney
            | Question::ShallIReactOnApple => false,

            Question::ShallIFollowSteps | Question::ShallIStayOnMyPost => false,

            Question::ShallIHelpFriendInTrouble
            | Question::ShallIFollowLostEnemy
            | Question::ShallIFollowHint => true,

            Question::ShallIRun => {
                self.soldier_profile_endurance > parameters_ai::MINVALUE_RUN as u16
            }

            Question::HasTheNewTaskPriority => self.has_the_new_task_priority(),

            // These five reach no indoor arm. The Original's default arm
            // asserts and then recurses on ShallIStayOnMyPost; that recursion
            // is non-hypothetical and the NPC is still indoor, so it lands on
            // the indoor ShallIStayOnMyPost arm above and yields `false`.
            //
            // The assertion is not an invariant that holds: the whistle and
            // send-out-soldier askers are reached from ordinary wondering
            // substates with no outdoor precondition, so a soldier that heard
            // whistling from inside a building trips it in the shipped debug
            // build too. Only the release-build answer is behaviour, so this
            // stays a trace rather than a panic.
            Question::ShallILookWhistle
            | Question::ShallIFollowWhistle
            | Question::ShallISeekBeforeAlertingOfficer
            | Question::ShallISeekBeforeAlertingSoldiers
            | Question::ShallISendOutSoldier => {
                tracing::trace!(
                    ?question,
                    "answer_question: indoor branch has no arm for this question; answering false"
                );
                false
            }
        }
    }

    /// Convenience wrapper matching the original Rust signature used at most
    /// call sites — defaults `hypothetical = false`.
    pub fn answer_question(&self, question: Question, ctx: &AiContext) -> bool {
        self.answer_question_ex(question, ctx, false)
    }

    /// New-task priority decision — shared between the
    /// indoor and outdoor branches of character-based decisions.
    fn has_the_new_task_priority(&self) -> bool {
        if self.new_task_priority >= self.current_task_priority {
            return true;
        }
        match self.base.current_state {
            AiState::Seeking | AiState::Wondering => false,
            _ => self.minimal_task_priority == task_priority::NONE,
        }
    }

    // -----------------------------------------------------------------------
    // AI initialization
    // -----------------------------------------------------------------------

    /// Initialize patrol membership, authored AI state, and initial duty.
    pub fn init_one_ai(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> crate::ai::InitStateSideEffects {
        // Initialize the "old odds" accumulator used by the weighted
        // decision RNG (old_odds = 50).
        self.old_odds = 50;

        // Build the minion list from patrol_ids.
        self.initialize_patrol();

        // Return to duty only if state initialization allows it and neither
        // script nor AI locks are active. Evaluate the authored
        // initial-action and commit the matching AI-side state
        // transition first — the subclass tail below only runs when
        // the authored action allows it *and* the AI isn't locked.
        let fx = self.base.init_state(sim, ctx);

        let go_to_duty =
            fx.go_to_duty && !self.base.ai_is_script_locked() && !self.base.ai_is_locked();

        // If the soldier has a patrol path, walk onto it.
        if go_to_duty && self.base.has_patrol_path {
            // Snapshot the substate-at-last-timer-launch *before* the
            // state-change / return-to-duty pair so a subsequent
            // timer-expiry-against-launch-substate check at
            // `ai_enemy.rs:2915/2920` sees this snapshot rather than
            // the default `Substate::DefaultOnPost`.
            self.base.substate_at_last_timer_launch = self.base.current_substate;
            self.set_state(AiState::Default, Substate::DefaultEnroute);
            self.return_to_duty_default(sim, ctx, tick);
        }

        // Movement setup checks `think_method_recursion_depth > 0` and
        // either sets `already_on_point` (for the enclosing decision-tick completion
        // to dispatch) or fires `Think(EVENT_REACHPOINT)` directly when
        // called outside a Think cycle.  `return_to_duty` above runs outside Think, so a
        // movement to a waypoint we're already standing on (e.g. a 1-
        // waypoint patrol where the spawn sits next to the waypoint)
        // sets `already_on_point = true` but nothing drains it — the
        // NPC never gets EVENT_REACHPOINT and the waypoint macro never
        // fires.  Queue a self-stimulus so the engine's next-tick
        // drain dispatches it (same shape as decision-tick completion's cascade).
        if self.base.already_on_point {
            self.base.already_on_point = false;
            self.base
                .fire_self_stimulus(crate::ai::StimulusType::EventReachPoint);
        }
        // A failed movement and a no-op facing command raise their latches unconditionally,
        // with no outside-Think delivery path of their own. Outside a Think the
        // next Think entry simply discards them, so drop them here rather than
        // inventing completions the actor never receives.
        self.base.couldnt_reachpoint = false;
        self.base.already_turned = false;

        // The original game stamps this after all patrol-path setup.
        self.base.last_hint_actuality = ctx.frame;

        fx
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
