//! Enemy (villain/soldier) AI.
//!
//! This module contains the `EnemyAi` struct which extends [`AiController`]
//! with soldier-specific state: combat tactics, seek behavior, officer/archer
//! specializations, money fights, and the massive Think state machine.

mod alert;
mod archer_combat;
mod battle;
pub(crate) use battle::{battle_friend_is_nearer, battle_owner_target_square_distance};
mod combat_positions;
mod event_handlers;
mod periodic;
mod seek;
mod substate_handlers;
mod util;

#[cfg(test)]
pub(crate) use alert::CommandSoldiersStart;
pub use util::*;

use serde::{Deserialize, Serialize};

use crate::ai::*;
use crate::entity_id::PcId;
use crate::parameters_ai;

/// Master switch for the opt-in AI decision/path diagnostic used by the
/// Save020/Save055 substate-only parity cohort. Keep this check separate so
/// disabled runs return before reading frame, owner, AI, or geometry state.
pub(crate) fn decision_path_debug_enabled() -> bool {
    std::env::var_os("PARITY_DEBUG_AI_DECISION_PATH").is_some()
}

/// Master switch for the `BattleDecisions` / phalanx / shield-timer
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
    if std::env::var_os("PARITY_DEBUG_THEM_LIFECYCLE").is_none() {
        return false;
    }
    let parse_filter = |name: &str| {
        std::env::var(name).ok().map(|value| {
            value.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid {name}={value:?} for THEM diagnostic: {error}")
            })
        })
    };
    parse_filter("PARITY_DEBUG_THEM_FRAME").is_none_or(|frame| frame == ctx.frame)
        && parse_filter("PARITY_DEBUG_THEM_CREATION_ORDER")
            .is_none_or(|creation_order| ctx.original_creation_order == Some(creation_order))
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
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
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
    /// `begin_special_strike()`; cleared by per-tick reconciliation in
    /// `engine/melee.rs::tick_enemy_sword_attacks` when the sequence
    /// manager no longer has an active sword-strike element for this
    /// actor (covers both natural completion and interruption).
    pub pending_special_strike: bool,

    /// One-shot handoff from `ReconsiderSwordfight` to the engine-side
    /// strike proposer.  The Original only calls `ProposeGoodSwordStrike`
    /// when that event-driven reconsideration reaches its decision tail;
    /// merely entering the swordfight substate must not authorize a draw.
    #[serde(default)]
    pub pending_sword_strike_consideration: bool,

    /// `Think` reached the post-`ReconsiderSwordfight` combat-insult site,
    /// but the engine-side strike proposer has not yet settled the one-shot
    /// consideration. Original proposes inline: a successful proposal
    /// changes to `...SPECIAL_STRIKE` and suppresses the insult, while a
    /// rejected proposal leaves `...SWORDFIGHT` and says it. The Rust port
    /// settles this latch immediately after `Think`, at the same owner
    /// boundary as `pending_sword_strike_consideration`.
    #[serde(default)]
    pub pending_combat_insult_after_strike_consideration: bool,

    // -- Private fields --
    #[serde(
        default,
        serialize_with = "serialize_optional_ai_handle",
        deserialize_with = "deserialize_optional_ai_handle"
    )]
    pub missed_pc: Option<AiEntityHandle>,
    pub pc_missed: bool,
    pub pc_gone_away_in_this_direction: u16,
    /// Frame when Charly was last observed missing.
    pub frame_when_missed_charly: u32,
    /// Net objects whose sound/event this soldier has already processed.
    pub heard_nets: Vec<ObjectHandle>,
    /// Last position at which an unexplained stimulus was detected.
    pub detected_something_there: Position,
    /// Cursor into the directions of the currently examined seek point.
    pub last_seek_direction_index: u8,
    #[serde(
        default,
        serialize_with = "serialize_optional_ai_handle",
        deserialize_with = "deserialize_optional_ai_handle"
    )]
    pub beggar_to_examine: Option<AiEntityHandle>,
    /// Whether the current `beggar_to_examine` is a real NPC beggar or a
    /// PC in disguise. Set by the engine when populating `beggars_to_control`.
    /// Checked during IdentifyingBeggar1.
    pub beggar_is_npc: bool,

    pub current_task_priority: u16,
    pub minimal_task_priority: u16,
    pub new_task_priority: u16,
    /// Distinct CheckFor checkpoints still involved in the current sweep.
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

    /// Raw serialized storage for Original's `mPreviousState`.
    ///
    /// The Original constructor leaves this field indeterminate and still
    /// serializes all four bytes. It only becomes semantically live after
    /// `EventSeesCharlyStandardProcedure` assigns it together with
    /// `previous_substate`.
    pub previous_state: i32,
    /// Raw serialized storage for Original's `mPreviousSubstate`; see
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
    #[serde(
        default,
        serialize_with = "serialize_optional_ai_handle",
        deserialize_with = "deserialize_optional_ai_handle"
    )]
    pub archer_behind_me: Option<AiEntityHandle>,
    #[serde(
        default,
        serialize_with = "serialize_optional_ai_handle",
        deserialize_with = "deserialize_optional_ai_handle"
    )]
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
    /// particular attempt. Original deletes a refusing member from the live
    /// list and retries the same list index before deciding whether to clear
    /// `SEEK_LOCATION_FIRST`; keeping this as a continuation queue preserves
    /// that source order.
    #[serde(default)]
    pub pending_group_instruction_candidates: Vec<(HumanHandle, Position)>,
    /// Flags for the next group instruction. Stored as bits so old serialized
    /// AI snapshots default cleanly and unknown bits cannot be invented.
    #[serde(default)]
    pub pending_group_instruction_seek_flags: u16,
    /// Whether the first accepted member consumes `SEEK_LOCATION_FIRST`.
    /// Charly-path distribution keeps the flag for every member instead.
    #[serde(default)]
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
    /// charge-from-reactiontime branch in `ReconsiderEnemyApproach`.
    /// Pulled from the weapon profile at level load.
    pub sword_is_charge_weapon: bool,
    /// Universal-frame counter when this soldier is next allowed to
    /// throw a sword strike.  Lets the engine sword-combat tick
    /// space attacks 1+ second apart instead of dealing damage every
    /// frame.  Collapsed into a single per-soldier cooldown rather
    /// than per-strike-sequence-element budgets.
    pub next_sword_strike_frame: u32,

    pub company_number: u16,
    #[serde(
        default,
        serialize_with = "serialize_optional_ai_handle",
        deserialize_with = "deserialize_optional_ai_handle"
    )]
    pub left_combat_neighbour: Option<AiEntityHandle>,
    #[serde(
        default,
        serialize_with = "serialize_optional_ai_handle",
        deserialize_with = "deserialize_optional_ai_handle"
    )]
    pub right_combat_neighbour: Option<AiEntityHandle>,

    pub attentive: bool,
    pub will_be_attentive: bool,
    pub forced_attentive: bool,

    /// PC this NPC is guarding. `None` is the original null pointer.
    pub guarded_pc: Option<PcId>,

    pub my_line_jump: Option<u32>,

    pub tower_guard: bool,
    pub combat_trainer: bool,

    // -- Added for Rust port --
    /// Whether this NPC is an archer (set during InitOneAI from entity data).
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
    fn required_friend_in_trouble(&self, context: &'static str) -> AiEntityHandle {
        self.base.friend_in_trouble.unwrap_or_else(|| {
            panic!(
                "enemy AI {} requires a friend in trouble while {context}",
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
    /// reciprocal clears performed by `RHElementActorSoldier::
    /// UpdateLeftCombatNeighbour(NULL)` / `UpdateRightCombatNeighbour(NULL)`.
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
        // GetIQ -> GetModifiedCapacity scales only when the NPC's camp
        // is Lacklandists; Royalist soldiers (also EnemyAi-driven)
        // get the raw intelligence.
        if !ctx.camp.is_hostile_to(crate::element::Camp::Royalists) {
            return self.soldier_profile_iq;
        }
        ctx.difficulty.modify_capacity(
            self.soldier_profile_iq,
            difficulty::EASY_ENEMY_IQ,
            difficulty::HARD_ENEMY_IQ,
            100,
        )
    }

    pub fn get_courage(&self) -> u16 {
        self.soldier_profile_courage
    }

    /// Apply `EASY_ENEMY_FIGHTING / HARD_ENEMY_FIGHTING` modifiers
    /// when the camp is `Lacklandists` (deliberately the FIGHTING
    /// modifiers, not SHOOTING — see the comment in
    /// `EngineInner::bow_profile_and_ability`), then scale by
    /// `(1.0 - 0.01 * blood_alcohol)`.  Used by `AIMING_TIME_FORMULA`
    /// (`(110 - GetShootingAbility()) / 2`) when launching the
    /// bow-aim timer — without this override the timer would track
    /// the soldier's *intelligence* instead of its shooting skill.
    pub fn get_shooting_ability(&self, ctx: &AiContext) -> u16 {
        let mut shooting = if ctx.camp.is_hostile_to(crate::element::Camp::Royalists) {
            ctx.difficulty.modify_capacity(
                self.soldier_profile_shooting,
                difficulty::EASY_ENEMY_FIGHTING,
                difficulty::HARD_ENEMY_FIGHTING,
                100,
            )
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
        // mode and write it back so downstream BattleDecisions arms
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
            // Original calls RHArtificialIntelligence::MaxNormDistance:
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
        // target. Original walks `mlistUs` — the allies that already
        // passed this decision's own IsDetecting360Degrees gate — in the
        // order BattleDecisions inserted them, not a fresh proximity list.
        let my_pride = self.soldier_profile_pride;
        for &friend in &self.base.list_us {
            if friend == self.base.me {
                continue;
            }
            // PCs on the us-list fail Original's `IsSoldier()` test and
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
            if is_any_swordfight_substate(f.ai_substate as u32)
                && f.primary_target == self.base.primary_target
            {
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

    /// Returns true when this NPC is a royalist foot-soldier on a forest
    /// (Sherwood) level — gates several special behaviours: no tying,
    /// archer flee via MerryManForestCassos, 180° vision cone, fast
    /// reaction time.
    fn is_merry_man_forest(&self, ctx: &AiContext) -> bool {
        ctx.camp == crate::element::Camp::Royalists && ctx.is_forest_level && !ctx.self_is_rider
    }

    /// Returns true if any same-camp soldier (other than us) is currently
    /// in a take-money or fight-for-money substate (minus the reaction-time
    /// intro arm) and is detected by my 180° cone.
    fn there_is_another_guy_in_sight_approaching_to_money(
        &self,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        tick.camp_soldiers.iter().any(|s| {
            s.handle != self.base.me
                && (s.ai_substate.is_take_money() || s.ai_substate.is_fight_for_money())
                && s.ai_substate != Substate::WonderingMoneyReactiontime
                && self.is_detecting_180_degrees(s.handle as HumanHandle, ctx)
        })
    }

    /// A merry man in the forest flees to the nearest reinforcement door
    /// (map exit). Returns `true` if an exit was found and the flee state
    /// was set, `false` if no exit is available.
    fn merry_man_forest_cassos(&mut self, ctx: &AiContext, global: &AiGlobalState) -> bool {
        // Find nearest reinforcement door
        let my_pos = &ctx.position;
        let mut min_dist = f32::MAX;
        let mut best_door_idx: Option<usize> = None;

        for (i, door) in global.reinforcement_doors.iter().enumerate() {
            let dx = my_pos.x - door.position_in.x;
            let dy = my_pos.y - door.position_in.y;
            let dist = dx.abs().max(dy.abs()); // MaxNorm
            if dist < min_dist {
                min_dist = dist;
                best_door_idx = Some(i);
            }
        }

        let Some(idx) = best_door_idx else {
            // No way out!
            return false;
        };

        let door = &global.reinforcement_doors[idx];
        let door_pos = door.position_in;

        // Store the chosen door's canonical global index for PointOut
        // movement later.  We use the global door index (not the
        // position into `reinforcement_doors`) so a single
        // `my_door_index` semantics — a global door-table index — is
        // shared between merry-man flee, RunAndAlertSoldiers, and the
        // AlertSoldiers indoor formation flow.
        self.base.my_door_index = Some(door.door_index);

        // SetState + GoTo + LaunchTimer first.  The `couldnt_reachpoint`
        // check is deliberately *after* the GoTo so that any prior-tick
        // reachpoint failure is cleared by `go_to` on entry, and only
        // a synchronously-raised failure on the fresh GoTo bails the
        // routine.
        self.go_to(
            AiState::Fleeing,
            Substate::FleeingMerryManRunToLeaveMap,
            door_pos,
            crate::ai::GotoFlags::RUN,
            ctx,
        );
        self.base.launch_timer(30, ctx.frame);

        // Only the just-issued GoTo's out-of-bounds / null-sector
        // failure should bail.
        if self.base.couldnt_reachpoint {
            self.base.couldnt_reachpoint = false;
            return false;
        }

        true
    }

    fn forget_attentive_mode(&mut self) {
        self.attentive = false;
        self.will_be_attentive = false;
        if let Some(request) = self.base.outbox.actor.last_pending_attentive_mode_mut() {
            // SetState called SetAttentiveMode synchronously before the
            // Original reached ForgetAttentiveMode. Preserve the transition
            // launch, then restore this helper's later flag writes after the
            // deferred engine-side request settles.
            request.forget_after = true;
        }
    }

    /// Two-step purge:
    ///
    /// 1. Walk our `DETECTABLE_OBJECT` list and drop every coin entry
    ///    within MaxNorm 500 of `pos` so the soldier doesn't
    ///    immediately re-spot the same drops on the next perception
    ///    pass.  This is queued as a pending engine request because
    ///    the AI side keeps no copy of `detectable_lists`.
    /// 2. Clear the parallel `other_seen_money` list.
    fn forget_all_nearby_coins(&mut self, ctx: &AiContext) {
        self.base.outbox.actor.forget_nearby_coins = Some(ctx.position);
        self.other_seen_money.clear();
    }

    /// Drops entries from `other_seen_money` whose referenced object
    /// is no longer active, then clears `interesting_object` if it
    /// now points at an inactive coin.
    ///
    /// An inactive entity is absent from `AiContext::entity_views`,
    /// so the filter is `entity_position(handle).is_some()`.
    fn clean_up_list_of_seen_money(&mut self, ctx: &AiContext) {
        self.other_seen_money
            .retain(|handle| ctx.entity_position(*handle).is_some());

        if self.base.interesting_object.is_some()
            && ctx.entity_position(self.base.interesting_object).is_none()
        {
            self.base.interesting_object = None;
        }
    }

    /// Tests whether the beer currently held in `interesting_object`
    /// is still reachable and not being claimed by a closer friend.
    ///
    /// Returns `None` when everything is fine (the soldier should keep
    /// approaching / re-arm its poll timer).  Returns `Some(lost_pos)`
    /// when the beer is gone — either because the object became
    /// inactive, or because another friend in an ale-related substate
    /// is approaching the same bottle and is closer than us, or is
    /// already drinking it.  `lost_pos` is the position the caller
    /// should `Face()` before transitioning to `WonderingAleAway`.
    fn is_beer_still_available(&self, ctx: &AiContext) -> Option<Position> {
        let Some(interesting) = self.base.interesting_object else {
            // No beer assigned: nothing to check against.  Fall back
            // to the soldier's own position so downstream `Face()` is
            // a no-op rather than pointing at the origin.
            return Some(ctx.position);
        };

        // Object inactive → gone.  An inactive entity is absent from
        // the view map, so we fall back to the soldier's last known
        // seek target (set when it committed to this bottle) for the
        // `look_there_if_not` out-param.
        let Some(obj_pos) = ctx.entity_position(interesting) else {
            return Some(self.base.seek_position);
        };

        // My squared distance to the object.
        let dx = ctx.position.x - obj_pos.x;
        let dy = ctx.position.y - obj_pos.y;
        let my_sq_distance = dx * dx + dy * dy;

        // Walk every NPC; a friend in an ale-related substate with
        // the same `interesting_object` steals our beer if they're
        // closer (for approach substates) or already drinking it
        // (for the drinking substate).
        for (&handle, view) in ctx.entity_views.iter() {
            if handle == self.base.me {
                continue;
            }
            let beer_away = match view.ai_substate {
                Substate::WonderingApproachingAle | Substate::WonderingAleReactiontime => {
                    if view.interesting_object != Some(interesting) {
                        continue;
                    }
                    if !self.is_detecting_180_degrees(handle, ctx) {
                        continue;
                    }
                    let fx = view.position.x - obj_pos.x;
                    let fy = view.position.y - obj_pos.y;
                    fx * fx + fy * fy < my_sq_distance
                }
                Substate::WonderingDrinkingAle => {
                    view.interesting_object == Some(interesting)
                        && self.is_detecting_180_degrees(handle, ctx)
                }
                _ => continue,
            };
            if beer_away {
                return Some(view.position);
            }
        }

        None
    }

    /// Sweeps inactive entries, then picks the coin with the smallest
    /// MaxNorm distance to the soldier (with a +300 malus for coins
    /// on a different layer), removes it from `other_seen_money`, and
    /// returns it.  Returns `None` when the list is empty after the
    /// sweep.
    fn get_nearest_seen_money_and_remove_it_from_list(
        &mut self,
        ctx: &AiContext,
    ) -> Option<ObjectHandle> {
        self.clean_up_list_of_seen_money(ctx);

        let my_pos = ctx.position;
        let my_layer = my_pos.level;
        let mut best: Option<(usize, u32)> = None;
        for (idx, &handle) in self.other_seen_money.iter().enumerate() {
            let Some(coin_pos) = ctx.entity_position(handle) else {
                continue;
            };
            let dx = (coin_pos.x - my_pos.x).abs();
            let dy = (coin_pos.y - my_pos.y).abs();
            let mut distance = dx.max(dy) as u32;
            if coin_pos.level != my_layer {
                distance = distance.saturating_add(300);
            }
            match best {
                Some((_, best_d)) if distance >= best_d => {}
                _ => best = Some((idx, distance)),
            }
        }

        best.map(|(idx, _)| self.other_seen_money.remove(idx))
    }

    /// Walks same-camp soldiers (via per-tick `camp_soldiers` snapshot),
    /// sends CALL_FINISH_BRAWL to every soldier rank currently in a
    /// take-money / fight-for-money substate within detection range,
    /// stores them in `list_us`, and sets `antagonist` to the first.
    fn finish_brawl(&mut self, ctx: &AiContext, tick: &AiPerTickData) {
        debug_assert_eq!(self.get_rank(), ProfileRank::Officer);
        self.base.list_us.clear();
        self.base.antagonist = None;

        // Each `CALL_FINISH_BRAWL` send is gated on 360-degree
        // detection (radius + opaque LOS), computed lazily here only
        // for soldiers passing the cheap rank/substate filter (eager
        // pre-compute was O(N²) per tick).  The brawler is the viewer
        // and the officer the target, so the ray runs soldier→officer.
        let me = &*self;
        let targets: Vec<NpcHandle> = tick
            .camp_soldiers
            .iter()
            .filter(|s| {
                s.rank == ProfileRank::Soldier
                    && (s.ai_substate.is_take_money() || s.ai_substate.is_fight_for_money())
                    && me.is_detected_360_degrees_by(s, ctx)
            })
            .map(|s| s.handle)
            .collect();

        for h in targets {
            self.base.list_us.push(h);
            if self.base.antagonist.is_none() {
                self.base.antagonist = Some(AiEntityHandle::new(h));
            }
            self.base
                .outbox
                .reentrant
                .cross_npc_actions
                .push(CrossNpcAction::SendStimulus {
                    target: h,
                    stimulus_type: StimulusType::CallFinishBrawl,
                    // Send the officer (`me`); receiver reads it as
                    // `stimulus_info.human` for Face/antagonist.
                    info: crate::ai::StimulusInfo::Human(AiEntityHandle::new(self.base.me)),
                    fallback_to_sender: None,
                    to_whole_patrol: false,
                });
        }

        // No `friend_in_trouble` fallback: when the camp-soldier scan
        // finds nothing we leave `antagonist = 0` and skip the
        // Face/Say. This avoids over-broadcasting `CALL_FINISH_BRAWL`
        // and spurious `OfficerEndsBrawl` remarks against a cached
        // friend.

        if self.base.antagonist.is_some() {
            // Face(antagonist); Say(OfficerEndsBrawl, MyTalk1)
            self.base.face_entity(self.base.antagonist, ctx);
            self.base.say(Remark::OfficerEndsBrawl);
        }
    }

    /// Shared helper for `WonderingOfficerApproachingBrawl`: transition
    /// to `FinishingBrawl`, run the brawl walk, set mood, re-arm timer.
    fn begin_finishing_brawl(&mut self, ctx: &AiContext, tick: &AiPerTickData) {
        self.set_state(AiState::Wondering, Substate::WonderingOfficerFinishingBrawl);
        self.finish_brawl(ctx, tick);
        self.base.set_emoticon(EmoticonType::Thunderstorm);
        self.base.launch_timer(200, ctx.frame);
    }

    /// Money-fight anti-loop guard: returns true when any same-camp
    /// soldier is currently in one of the
    /// `WonderingOfficer{Seeing,Approaching,Finishing}Brawl` substates
    /// within MaxNorm < 150 of the coin.  Called before a soldier
    /// commits to picking up a coin so that once an officer has
    /// intervened in a brawl, nearby grabbers back off instead of
    /// re-engaging.
    pub fn is_any_angry_officer_near(&self, pos_money: Position, tick: &AiPerTickData) -> bool {
        for cs in &tick.camp_soldiers {
            match cs.ai_substate {
                Substate::WonderingOfficerSeeingBrawl
                | Substate::WonderingOfficerApproachingBrawl
                | Substate::WonderingOfficerFinishingBrawl => {
                    let dx = (cs.position.x - pos_money.x).abs();
                    let dy = (cs.position.y - pos_money.y).abs();
                    if dx.max(dy) < 150.0 {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// Officer-only eligibility predicate for alerting a specific
    /// soldier: rejects the candidate if it belongs to another
    /// officer's patrol (its `PatrolChief` is not me and is within
    /// MaxNorm < 700 of the soldier) or if it is already mid-dialogue
    /// with another antagonist.  Called from `alert_soldiers` and
    /// from the EVENT_SEES_SOLDIER officer→soldier arm.
    pub fn can_call_this_soldier(
        &self,
        cs: &CampSoldierInfo,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        let my_handle = self.base.me;
        let my_id = crate::element::EntityId::Soldier(crate::entity_id::SoldierId(my_handle));

        // Belongs-to-another-patrol gate.
        if let Some(chief_id) = cs.patrol_chief
            && chief_id != my_id
        {
            let chief_pos_opt = tick
                .camp_soldiers
                .iter()
                .find(|o| o.handle == chief_id.index())
                .map(|o| o.position)
                .or_else(|| ctx.entity_view(chief_id.index()).map(|v| v.position));
            if let Some(chief_pos) = chief_pos_opt {
                let ddx = (cs.position.x - chief_pos.x).abs();
                let ddy = (cs.position.y - chief_pos.y).abs();
                if ddx.max(ddy) < 700.0 {
                    return false;
                }
            }
        }

        // In-dialogue-with-someone-else gate.
        !cs
            .antagonist
            .is_some_and(|antagonist| antagonist.get() != my_handle)
    }

    /// Pops the next queued money-fight victim and approaches it;
    /// returns to duty when the queue drains.  Sets `detected_body`
    /// before going near.
    fn awake_next_money_fight_victim_if_any(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        if self.money_fight_victims.is_empty() {
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            return;
        }
        let next = self.money_fight_victims.remove(0);
        self.base.detected_body = Some(AiEntityHandle::new(next));
        // SetState(Wondering, ApproachingBrawlVictim).
        self.set_state(
            AiState::Wondering,
            Substate::WonderingApproachingBrawlVictim,
        );
        if let Some(view) = ctx.entity_view(next as HumanHandle) {
            self.base.go_near(
                view.position,
                parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                crate::ai::GotoFlags::empty(),
                ctx,
            );
        }
    }

    /// After a brawl ends the soldier scans its seen-money list for
    /// the nearest still-active coin, runs for it, or falls back to a
    /// left/right scan when nothing remains.
    fn stop_brawling_and_collect_money(&mut self, ctx: &AiContext, _tick: &AiPerTickData) {
        // CleanUpListOfSeenMoney + GetNearestSeenMoneyAndRemoveItFromList.
        if let Some(coin) = self.get_nearest_seen_money_and_remove_it_from_list(ctx) {
            // interesting_object = nearest coin.
            self.base.interesting_object = Some(AiEntityHandle::new(coin));
            if let Some(coin_pos) = ctx.entity_position(coin) {
                // SetState(Wondering, RunningForMoney) +
                // GoNear(coin, AI_STOP_BEFORE_MONEY_DISTANCE,
                //        RUN | FIND_ACCESSIBLE).
                self.go_near(
                    AiState::Wondering,
                    Substate::WonderingRunningForMoney,
                    coin_pos,
                    parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                    crate::ai::GotoFlags::RUN | crate::ai::GotoFlags::FIND_ACCESSIBLE,
                    ctx,
                );
            }
        } else {
            // No coins left — look around for more.
            self.set_state(AiState::Wondering, Substate::WonderingWatchingForMoreMoney);
            self.base.outbox.actor.look_sidewards = Some(LookDirection::LeftRight);
        }
    }

    /// Rebuilds `money_fight_victims` from the same-camp soldiers
    /// currently unconscious + alive + `was_knocked_out_in_money_fight`,
    /// gated on 360° detection, sorted ascending by squared stretch-Y
    /// distance.
    ///
    /// The engine already materialises the unconscious + alive filter
    /// into `tick.camp_unconscious_soldiers`, so we walk that instead of
    /// iterating all soldiers per call.
    ///
    /// The comparator reads the locally-computed `sq` so no
    /// per-soldier scratchpad is needed.
    fn create_list_of_near_money_fight_victims(&mut self, ctx: &AiContext, tick: &AiPerTickData) {
        // Clear the list.
        self.money_fight_victims.clear();

        // Collect (handle, stretched-Y sq_distance) for candidates that
        // pass the 360° detection gate.
        let mut candidates: Vec<(NpcHandle, f32)> = Vec::new();
        for us in tick.camp_unconscious_soldiers.iter() {
            if !us.knocked_out_in_money_fight {
                continue;
            }
            let handle = us.handle;
            if handle == self.base.me {
                continue;
            }
            let Some(victim_view) = ctx.entity_view(handle as HumanHandle) else {
                continue;
            };
            if victim_view.in_building {
                continue;
            }
            let victim_pos = crate::stealth::detection_point_xy(
                crate::coordinates::MapPoint::new(victim_view.position.x, victim_view.position.y),
                victim_view.posture,
                victim_view.direction as i16,
            );
            let viewer_eye_z = ctx.elevation
                + crate::stealth::eye_z_for_posture(
                    crate::element::Posture::Upright,
                    ctx.self_is_rider,
                );
            let target_eye_z = victim_view.elevation
                + crate::stealth::detection_z_for_posture(
                    victim_view.posture,
                    victim_view.is_rider,
                );
            let viewer_eye_ground = crate::coordinates::GroundPoint::from_map_and_z(
                crate::coordinates::MapPoint::new(ctx.position.x, ctx.position.y),
                ctx.elevation,
            );
            let target_detection_ground =
                crate::coordinates::GroundPoint::from_map_and_z(victim_pos, victim_view.elevation);
            // SquareDistance — dx² + (dy * INVERSE_ASPECT_RATIO)².
            let dx = target_detection_ground.x - viewer_eye_ground.x;
            let dy = (target_detection_ground.y - viewer_eye_ground.y)
                * crate::position_interface::INVERSE_ASPECT_RATIO;
            let dz = target_eye_z - viewer_eye_z;
            let sq = dx * dx + dy * dy + dz * dz;
            if ctx.in_building || sq > ctx.sq_standard_view_radius {
                continue;
            }
            if !crate::sight_obstacle::is_reachable_3d(
                ctx.obstacle_list(),
                [viewer_eye_ground.x, viewer_eye_ground.y, viewer_eye_z],
                [
                    target_detection_ground.x,
                    target_detection_ground.y,
                    target_eye_z,
                ],
                crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
            ) {
                continue;
            }
            candidates.push((handle, sq));
        }
        // Sort by ascending sq distance.
        candidates.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        self.money_fight_victims = candidates.into_iter().map(|(h, _)| h).collect();
    }

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
    ///     anyway so the wiring lands when the gate is ported.
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
    /// This is the `Face(RHElement*)` overload: it includes the target's
    /// elevation and preserves `FaceTo`'s already-facing Waiting/Bored
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
            // Original narrows ULONG GetCreationOrder() into this UWORD field.
            guy_index: original_creation_order as u16,
            bad_guy: true, // ai_enemy is always a soldier
            forbidden_till_frame: frame + frames,
        });
    }

    /// Reinitialize the Them list with all currently visible enemies.
    fn reinitialize_them_list(&mut self, ctx: &AiContext, _tick: &AiPerTickData) {
        // Original: RHArtificialMalignity::ReinitializeThemList
        // (`original-code/RHartificialmalignity.cpp:7015-7036`). The
        // original deletes the old list and rebuilds it only from enemies
        // whose current IsEnemySeen flag is set and who are not dead. It
        // does not preserve mpPrimaryTarget when that target is no longer
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
            let Some(target) = ctx.entity_view(handle) else {
                // Detectable removal is owner-ordered; a target killed by an
                // earlier owner can remain in this observer's retained seen
                // list until its next refresh.
                // TODO: remove dead target detectables synchronously from all
                // later observers in the same actor pass.
                tracing::warn!(
                    me = self.base.me,
                    target = handle,
                    "dropping stale seen-enemy handle missing from the live entity view"
                );
                continue;
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
        // Patrol initialization (TransformPatrolIDsToRealPatrol +
        // InitializePatrol) requires engine access to resolve soldier IDs to
        // entity handles and check visibility/state. The actual initialization
        // happens in EngineInner::tick_patrol_coordination — we just raise a
        // one-shot flag the engine tick honours next pass, mirroring the
        // explicit `InitializePatrol()` invocation points (`init_ai`
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
        // Original calls every eligible civilian's Think synchronously. Keep
        // this engine callback in the same FIFO as Say/SetState so a caller
        // such as BeginSwordfight preserves Panic -> Say statement order.
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
        // Original and uses IsDetecting180Degrees. Do not route it through
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
    ///   - sq_dist < 350² → reacts iff IsDetecting180Degrees(me)
    ///   - otherwise → reacts iff IsDetecting(me) (cone + LOS)
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
            //   * `200²..350²` — `IsDetecting180Degrees(me)`.
            //   * `≥ 350²` — `IsDetecting(me)` (full cone + LOS,
            //     officer-side), evaluated here at the Original call site.
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
                        "MaybeOfficerSeesMeFighting officer {} is absent from the AI entity view",
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

    /// Handle the thief-stole-my-coin case.  Dispatched from the
    /// `EventObjectAway` arm in `think_unexpected_event` after the
    /// 180° detection / interesting-object gate has passed on the
    /// caller side for the type check.
    fn stolen_money_standard_procedure(
        &mut self,
        thief: NpcHandle,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        // 180° gate on the thief.
        if !self.is_detecting_180_degrees(thief as HumanHandle, ctx) {
            return;
        }
        // Assert: thief cannot be me.
        if thief == self.base.me {
            return;
        }
        // Question the soldier profile.
        if !self.answer_question(Question::ShallIFightForMoney, ctx) {
            return;
        }
        // Morale check; bail and collect on no.
        if !self.wants_to_continue_money_fight(tick, ctx) {
            self.money_fight_enemies.clear();
            self.stop_brawling_and_collect_money(ctx, tick);
            return;
        }
        // Substate dispatch.
        if self.base.current_substate.is_take_money() {
            // "Hey, this coin is MINE!"
            self.base.break_macro();
            self.base.face_entity(thief, ctx);
            self.base.set_emoticon(EmoticonType::QuestionMark);
            self.set_state(AiState::Wondering, Substate::WonderingBrawlReactiontime);
            self.money_fight_enemies.push(thief);
            self.react(parameters_ai::AI_MAX_ENEMY_REACTIONTIME as u16, ctx, tick);
            self.base.friend_in_trouble = Some(AiEntityHandle::new(thief));
        } else if self.base.current_substate.is_fight_for_money() {
            // Already brawling; queue this guy.
            self.money_fight_enemies.push(thief);
        }
    }

    /// Walks `money_fight_enemies` and returns the handle of the one
    /// at minimum MaxNorm distance, adding a +300 malus when the
    /// enemy is on a different layer.  Returns `None` when the list
    /// is empty.
    fn get_nearest_money_fight_enemy(&self, ctx: &AiContext) -> Option<NpcHandle> {
        let my_layer = ctx.position.level;
        let mut best: Option<(NpcHandle, u32)> = None;
        for &handle in self.money_fight_enemies.iter() {
            let Some(view) = ctx.entity_view(handle as HumanHandle) else {
                continue;
            };
            let dx = (view.position.x - ctx.position.x).abs();
            let dy = (view.position.y - ctx.position.y).abs();
            let mut distance = dx.max(dy) as u32;
            if view.position.level != my_layer {
                distance = distance.saturating_add(300);
            }
            match best {
                Some((_, best_d)) if distance >= best_d => {}
                _ => best = Some((handle, distance)),
            }
        }
        best.map(|(h, _)| h)
    }

    /// Rebuilds `money_fight_enemies` from the current same-camp
    /// soldier snapshot — conscious, alive, 360°-detected soldiers
    /// whose substate is take/fight-for-money.
    ///
    /// `tick.camp_soldiers` is built before the creation-order AI pass, so an
    /// earlier soldier can become unconscious or die before this actor scans
    /// it.  Original reads those lifecycle flags live at this point.
    ///
    /// The 360° check comes before the substate test: it runs for every
    /// conscious camp soldier, not just the ones already brawling, and
    /// each query perturbs the shared visibility cache.
    fn create_new_list_of_money_fight_enemies(&mut self, tick: &AiPerTickData, ctx: &AiContext) {
        self.money_fight_enemies.clear();
        for cs in tick.camp_soldiers.iter() {
            if cs.handle == self.base.me {
                continue;
            }
            let view = ctx
                .expect_entity_view(cs.handle as HumanHandle, "money-fight enemy-list candidate");
            if view.is_unconscious || view.is_dead {
                continue;
            }
            if !self.is_detecting_360_degrees(cs.handle as HumanHandle, ctx) {
                continue;
            }
            if !(cs.ai_substate.is_take_money() || cs.ai_substate.is_fight_for_money()) {
                continue;
            }
            self.money_fight_enemies.push(cs.handle);
        }
    }

    /// Morale check for whether to keep brawling based on
    /// upright-vs-sleeping money-fighter ratio.
    ///
    /// This is one scan over every alive same-camp soldier in the Original
    /// camp-registry order, with a single 360° query per candidate. Query
    /// order matters because each one perturbs the shared visibility cache.
    /// Preexisting sleepers can occur only in the parallel unconscious list,
    /// while a same-frame transition can leave the same handle in both. Merge
    /// the ordered snapshots and coalesce equal handles so neither shape is
    /// lost or queried twice.
    fn wants_to_continue_money_fight(&self, tick: &AiPerTickData, ctx: &AiContext) -> bool {
        // Berserker fast path + drunken override.
        if self.soldier_profile_money == 100 || self.base.blood_alcohol > 0 {
            return true;
        }

        let mut upright: u32 = 1; // counts self
        let mut sleeping: u32 = 0;

        let mut soldiers = tick.camp_soldiers.iter().peekable();
        let mut sleepers = tick.camp_unconscious_soldiers.iter().peekable();
        loop {
            let (handle, knocked_out_in_money_fight) = match (soldiers.peek(), sleepers.peek()) {
                (Some(soldier), Some(sleeper)) if soldier.handle == sleeper.handle => {
                    let soldier = soldiers.next().expect("peeked camp soldier");
                    let sleeper = sleepers.next().expect("peeked unconscious soldier");
                    (soldier.handle, sleeper.knocked_out_in_money_fight)
                }
                (Some(soldier), Some(sleeper)) => {
                    // Both lists are ordered subsequences of Original's
                    // camp array. Creation order is the stable identity
                    // for that authored order; runtime slot numbers are
                    // not interchangeable after reuse.
                    let soldier_order = ctx
                        .expect_entity_view(
                            soldier.handle as HumanHandle,
                            "money-fight morale camp-order candidate",
                        )
                        .original_creation_order;
                    let sleeper_order = ctx
                        .expect_entity_view(
                            sleeper.handle as HumanHandle,
                            "money-fight morale sleeper-order candidate",
                        )
                        .original_creation_order;
                    if soldier_order < sleeper_order {
                        let soldier = soldiers.next().expect("peeked camp soldier");
                        (soldier.handle, soldier.knocked_out_in_money_fight)
                    } else {
                        let sleeper = sleepers.next().expect("peeked unconscious soldier");
                        (sleeper.handle, sleeper.knocked_out_in_money_fight)
                    }
                }
                (None, Some(_)) => {
                    let sleeper = sleepers.next().expect("peeked unconscious soldier");
                    (sleeper.handle, sleeper.knocked_out_in_money_fight)
                }
                (Some(_), None) => {
                    let soldier = soldiers.next().expect("peeked camp soldier");
                    (soldier.handle, soldier.knocked_out_in_money_fight)
                }
                (None, None) => break,
            };
            if handle == self.base.me {
                continue;
            }
            let live_view =
                ctx.expect_entity_view(handle as HumanHandle, "money-fight morale candidate");
            if live_view.is_dead {
                continue;
            }
            if !self.is_detecting_360_degrees(handle as HumanHandle, ctx) {
                continue;
            }
            let live_substate = live_view.ai_substate;
            if live_substate.is_take_money() || live_substate.is_fight_for_money() {
                upright += 1;
            } else if live_substate == Substate::SleepingUnconscious && knocked_out_in_money_fight {
                // TODO(parity): `AiEntityView` does not yet expose the live
                // WasKnockedOutInMoneyFight flag. The ordered camp snapshots
                // carry the boundary value; add it to the view if a proven
                // cross-actor transition can flip this flag before our scan.
                sleeping += 1;
            }
        }

        let total = upright + sleeping;
        // `total >= 1` because `upright` starts at 1.
        let knocked_out_percentage = (100 * sleeping) / total;
        knocked_out_percentage < self.soldier_profile_money as u32
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
    /// `GetBuilding()` occupants, panics civilians, and calls
    /// `InitBattleBeforeDoor` on the camp split.  Caller must have
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

    /// 360°-detection check: the NPC can "feel" a target that is
    /// within its real view radius regardless of facing direction.
    /// Used by the `EVENT_OUTOFVIEW` handler for any swordfight substate
    /// to suppress the event when the target is actually still close —
    /// the LOS drop is just a transient cone flicker, not a real loss.
    ///
    /// Approximation: stretched-Y squared distance ≤ `sq_standard_view_radius`,
    /// plus an `is_reachable` (opaque sight obstacles) LOS check via
    /// `FastFindGrid`.
    // Forward the caller location into the recorded visibility query so
    // parity dumps attribute each check to the gate that asked for it,
    // not to this shared helper.
    #[track_caller]
    fn is_detecting_360_degrees(&self, target: HumanHandle, ctx: &AiContext) -> bool {
        //   if (!viewer_active_and_outside_building || !target_active_and_outside_building)
        //       return false;
        // Viewer half: gate on the viewer's in-building flag.  Active is
        // implied by the AI tick running.
        if ctx.building_sector.is_some() {
            return false;
        }
        let Some(view) = ctx.entity_view(target) else {
            tracing::trace!(
                target,
                "is_detecting_360_degrees: entity_view lookup failed"
            );
            return false;
        };
        // Target half is the literal IsActiveAndOutsideBuilding gate. Dead,
        // unconscious, tied, and otherwise non-fighting humans remain valid
        // while their raw element is active and its current sector is not a
        // building.
        if !view.active || view.in_building {
            return false;
        }
        // Viewer's eye point (forced upright in this overload) and
        // target's detection point.  The distance is the stretched-Y
        // 3D vector between them; the Z² term is what made the prior
        // 2D-only check over-detect when viewer and target sat at
        // very different elevations (e.g. tower guard above a
        // kneeling target on the ground).
        let viewer_eye = ctx.self_upright_eye_world;
        let viewer_eye_z = viewer_eye.z;
        let target_detection = crate::stealth::detection_point_world(
            view.detection_position_world,
            view.posture,
            view.direction as i16,
            view.is_rider,
        );
        let target_eye_z = target_detection.z;
        let viewer_eye_ground = crate::coordinates::GroundPoint::new(viewer_eye.x, viewer_eye.y);
        let target_detection_ground =
            crate::coordinates::GroundPoint::new(target_detection.x, target_detection.y);
        let dx = target_detection_ground.x - viewer_eye_ground.x;
        let dy = (target_detection_ground.y - viewer_eye_ground.y)
            * crate::position_interface::INVERSE_ASPECT_RATIO;
        let dz = target_eye_z - viewer_eye_z;
        let sq_distance = dx * dx + dy * dy + dz * dz;
        if sq_distance > ctx.sq_self_view_radius {
            tracing::trace!(
                target,
                sq_distance,
                sq_view_radius = ctx.sq_self_view_radius,
                detecting = false,
                "is_detecting_360_degrees: out of range"
            );
            return false;
        }
        // C++ RHElementActorNPC::IsDetecting360Degrees(actor) checks the
        // upright eye point against the target detection point through the
        // 3D opaque sight-obstacle graph, not the 2D spatial LOS helper.
        let los_clear = crate::sight_obstacle::is_reachable_3d(
            ctx.obstacle_list(),
            [viewer_eye_ground.x, viewer_eye_ground.y, viewer_eye_z],
            [
                target_detection_ground.x,
                target_detection_ground.y,
                target_eye_z,
            ],
            crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
        );
        tracing::trace!(
            target,
            sq_distance,
            sq_view_radius = ctx.sq_self_view_radius,
            los_clear,
            detecting = los_clear,
            "is_detecting_360_degrees"
        );
        los_clear
    }

    /// Admission gate for one member of a whole-patrol broadcast: a
    /// non-soldier member short-circuits before the detection query, so it
    /// costs no visibility traffic.
    ///
    /// The broadcast walk runs in the engine, which owns both the chief and
    /// the member, so the gate is evaluated through this accessor immediately
    /// before the member's `think`.
    pub(crate) fn detects_patrol_member_360(&self, member: NpcHandle, ctx: &AiContext) -> bool {
        ctx.entity_view(member)
            .map(|v| v.is_soldier())
            .unwrap_or(false)
            && self.is_detecting_360_degrees(member as HumanHandle, ctx)
    }

    /// Reverse of [`Self::is_detecting_360_degrees`]: does `viewer`
    /// feel *me*?  The radius belongs to the viewer and the detection
    /// point to me, so the resulting ray runs viewer→me — call sites
    /// that ask "can this soldier see me" must not substitute the
    /// forward check, which would swap the ray's endpoints.
    #[track_caller]
    fn is_detected_360_degrees_by(&self, viewer: &CampSoldierInfo, ctx: &AiContext) -> bool {
        let Some(viewer_view) = ctx.entity_view(viewer.handle as HumanHandle) else {
            tracing::trace!(
                viewer = viewer.handle,
                "is_detected_360_degrees_by: entity_view lookup failed"
            );
            return false;
        };
        crate::ai_enemy::soldier_detects_target_360(
            viewer.position,
            viewer_view.elevation,
            viewer_view.is_rider,
            viewer.view_radius,
            viewer_view.in_building,
            ctx.position,
            ctx.elevation,
            ctx.posture,
            ctx.self_is_rider,
            ctx.direction as i16,
            ctx.in_building,
            ctx.obstacle_list(),
        )
    }

    /// Normal `RHElementActorNPC::IsDetecting(human)` check used by
    /// synchronous AI state-machine gates. Unlike the 360-degree helper,
    /// this uses the live post-`RefreshView` cone and opaque line of sight.
    fn is_detecting(&self, target: impl IntoOptionalAiHandle, ctx: &AiContext) -> bool {
        let target = target
            .into_optional_ai_handle()
            .expect("is_detecting requires a non-null target")
            .get();
        let view = ctx.entity_view(target).unwrap_or_else(|| {
            panic!(
                "is_detecting: NPC {} requires missing target entity view {target}",
                self.base.me
            )
        });

        // ComputeVisibility uses the sector's BUILDING flag, not the
        // broader engine-side "inside building or passing a door" helper.
        let viewer_in_building = ctx.building_sector.is_some();
        let target_in_same_building =
            viewer_in_building && ctx.building_sector == view.building_sector;

        // This gate exists only in the same-building branch. Outside,
        // bodies and unconscious humans are still valid visibility targets
        // as long as their raw element is active and outside a building.
        if viewer_in_building && (view.is_dead || view.is_unconscious || view.passing_door) {
            return false;
        }
        if !viewer_in_building && (!view.active || view.building_sector.is_some()) {
            return false;
        }

        let target_detection_xy = crate::stealth::detection_point_xy(
            view.detection_position,
            view.posture,
            view.direction as i16,
        );
        let target_detection = crate::stealth::detection_point_world(
            view.detection_position_world,
            view.posture,
            view.direction as i16,
            view.is_rider,
        );
        let sight_obstacles = ctx.obstacle_list();
        let target_obstacle = view.obstacle_idx.map(|handle| {
            sight_obstacles.get(usize::from(handle)).unwrap_or_else(|| {
                panic!("is_detecting: target {target} requires missing sight obstacle {handle}")
            })
        });
        let q = crate::ai_vision::VisibilityQuery {
            viewer_los: ctx.self_eye_position,
            viewer_world: crate::coordinates::WorldPoint3D::new(
                ctx.self_eye_position.x,
                ctx.self_eye_position.y + ctx.elevation,
                ctx.self_eye_z,
            ),
            viewer_direction: ctx.direction as i16,
            view_forward: (ctx.self_view_direction[0], ctx.self_view_direction[1]),
            view_radius: ctx.self_view_radius,
            viewer_eye_status: ctx.self_eye_status,
            real_half_aperture: ctx.self_real_half_aperture,
            viewer_in_building,
            target_in_same_building,
            forest_180_degree_view: ctx.is_forest_level
                && ctx.camp == crate::element::Camp::Royalists,
            golden_eye_mode: false,
            effective_view_radius: ctx.self_view_radius as f32,
            target_is_active_and_outside_building: view.active && view.building_sector.is_none(),
            target_los: target_detection_xy,
            target_world: target_detection,
            target_posture: view.posture,
            target_action_state: view.action_state,
            target_is_pc: view.is_pc,
            cloak_deception_applies: view.posture == crate::element::Posture::Cloaked
                && ctx.camp.is_hostile_to(view.camp),
            cloak_remembers_target: self.list_them.contains(&target)
                || self.base.primary_target == target,
            // TODO(cloak-authoring): connect this seam only when an explicit
            // modded profile schema supplies detector data.
            cloak_authored_detector: crate::cloak::SHIPPED_AUTHORED_DETECTOR,
            sight_obstacles: ctx.obstacle_list(),
            fast_grid: &ctx.fast_grid,
            layer: ctx.position.level,
            target_unconscious: view.is_unconscious,
            target_passing_door: view.passing_door,
        };
        let viewer_entity = view_radius_memo_viewer(self.base.me, ctx);
        crate::ai_vision::compute_visibility_with_effective_radius(&q, || {
            ctx.compute_view_radius_cached(viewer_entity, view.obstacle_idx, || {
                crate::ai_vision::compute_view_radius(
                    q.viewer_world,
                    ctx.self_view_radius,
                    (ctx.self_view_direction[0], ctx.self_view_direction[1]),
                    ctx.self_real_half_aperture,
                    ctx.is_night_or_fog,
                    &ctx.fast_grid,
                    sight_obstacles,
                    target_obstacle,
                )
            })
        }) > 0.0
    }

    /// Complete the synchronous Charly-to-officer call after the engine
    /// has delivered `CALL_MR_OFFICER_I_AM_BACK` and obtained the
    /// officer's real `Think` return value.
    pub(crate) fn resolve_charly_officer_report(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        accepted: bool,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        if accepted {
            self.set_state(AiState::Seeking, Substate::SeekingCharlyGoToOfficerSeen);
            self.base.launch_timer(10, ctx.frame);
        } else {
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
        }
    }

    pub(crate) fn resolve_alert_request(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        accepted: bool,
        continuation: crate::ai::AlertContinuation,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        assert!(matches!(
            continuation,
            crate::ai::AlertContinuation::SoldierSawOfficer
        ));
        if !accepted {
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            return;
        }

        self.set_state(AiState::Seeking, Substate::SeekingRunningToOfficerSeen);
        self.base
            .say_with_flags(Remark::CallsOfficer, SpeechFlags::MYTALK_0);
        let target = self.base.antagonist.unwrap_or_else(|| {
            panic!(
                "accepted soldier alert from {} requires a target officer",
                self.base.me
            )
        });
        let officer_target_pos = ctx
            .entity_view(target)
            .unwrap_or_else(|| {
                panic!(
                    "accepted soldier alert from {} requires target officer {} view",
                    self.base.me, target
                )
            })
            .forecasted_destination
            .resolve(sim)
            .position;
        self.base.go_near(
            officer_target_pos,
            parameters_ai::AI_TALK_DISTANCE,
            crate::ai::GotoFlags::RUN,
            ctx,
        );
        self.base.launch_timer(20, ctx.frame);
    }

    pub(crate) fn resolve_think_result(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        accepted: bool,
        target: NpcHandle,
        continuation: ThinkResultContinuation,
        global: &mut AiGlobalState,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        match continuation {
            ThinkResultContinuation::SoldierFinishedAlertReportStart => {
                self.set_state(
                    AiState::Seeking,
                    Substate::SeekingSoldierGiveAlertingReportToOfficerPoint,
                );
                self.base.launch_timer(100, ctx.frame);
            }
            ThinkResultContinuation::OfficerCalledSoldier => {
                if accepted {
                    self.set_state(AiState::Seeking, Substate::SeekingOfficerWaitForSoldier);
                    self.base
                        .set_transient_emoticon(EmoticonType::XMark, 20, ctx.frame);
                    self.base.say(Remark::OfficerCallsSoldier);
                    self.base.launch_timer(20, ctx.frame);
                } else {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }
            ThinkResultContinuation::OfficerSentCharlyToOfficer => {
                if accepted {
                    self.base
                        .say_with_flags(Remark::SendsCharlyToOfficer, SpeechFlags::MYTALK_2);
                    self.base.point_to(self.officers_position, ctx);
                }
            }
            ThinkResultContinuation::OfficerInstructedGroupSoldier { last } => {
                if !accepted {
                    self.alerted_us.retain(|&handle| handle != target);
                } else if self.pending_group_instruction_clear_location_after_accept {
                    self.pending_group_instruction_seek_flags &= !SeekFlags::LOCATION_FIRST.bits();
                }
                let finished = if self.pending_group_instruction_candidates.is_empty() {
                    last
                } else {
                    self.queue_next_group_instruction();
                    false
                };
                if finished {
                    self.pending_group_instruction_seek_flags = 0;
                    self.pending_group_instruction_clear_location_after_accept = false;
                    if self.alerted_us.is_empty() {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    } else {
                        self.set_state(
                            AiState::Seeking,
                            Substate::SeekingOfficerWaitForInstructedGroup,
                        );
                        self.base.launch_timer(30, ctx.frame);
                    }
                }
            }
            ThinkResultContinuation::OfficerAlertedSoldier {
                last,
                use_formation,
                failure,
            } => {
                if accepted {
                    self.alerted_us.push(target);
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::ConsiderReport {
                            target,
                            report: self.base.my_reconnaissance_report.clone(),
                            flags: ReportUpdateFlags::UPDATE_CHARLY.bits()
                                | ReportUpdateFlags::UPDATE_TYPE.bits(),
                        },
                    );
                }
                let finished = if self.alerted_us.len() >= 20 {
                    self.pending_alert_soldier_candidates.clear();
                    true
                } else if !self.pending_alert_soldier_candidates.is_empty() {
                    let next = self.pending_alert_soldier_candidates.remove(0);
                    let next_is_last = self.pending_alert_soldier_candidates.is_empty();
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::RequestThinkResult {
                            target: next,
                            caller: self.base.me,
                            stimulus_type: StimulusType::CallAlert,
                            info: StimulusInfo::Human(AiEntityHandle::new(self.base.me)),
                            continuation: ThinkResultContinuation::OfficerAlertedSoldier {
                                last: next_is_last,
                                use_formation,
                                failure,
                            },
                        },
                    );
                    false
                } else {
                    last
                };
                if finished {
                    self.pending_alert_soldier_candidates.clear();
                    if accepted {
                        // Original resumes AlertSoldiers only after the
                        // accepted recipient's ConsiderReport call returns.
                        // Keep that callback and all owner-side effects ahead
                        // of formation/state/sequence work.
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::FinalizeAlertSoldiers {
                                caller: self.base.me,
                                use_formation,
                                failure,
                            },
                        );
                    } else {
                        // A refused final call has no ConsiderReport boundary.
                        self.finalize_alert_soldiers(
                            sim,
                            failure,
                            global,
                            grid.filter(|_| use_formation),
                            ctx,
                            tick,
                        );
                    }
                }
            }
            ThinkResultContinuation::OfficerCombatAlertedSoldier {
                last,
                use_formation,
            } => {
                if accepted {
                    self.alerted_us.push(target);
                }
                if last {
                    if self.finish_command_soldiers_to_attack(
                        global,
                        grid.filter(|_| use_formation),
                        ctx,
                        tick,
                    ) {
                        self.base.say(Remark::OfficerGivesAttackOrder);
                    } else {
                        self.enter_battle_reserve(ctx, tick);
                    }
                }
            }
        }
    }

    pub(super) fn queue_next_group_instruction(&mut self) {
        let (target, seek_point) = self
            .pending_group_instruction_candidates
            .first()
            .copied()
            .expect("group instruction continuation requires a pending recipient");
        self.pending_group_instruction_candidates.remove(0);
        let last = self.pending_group_instruction_candidates.is_empty();
        self.base
            .outbox
            .reentrant
            .cross_npc_actions
            .push(CrossNpcAction::RequestThinkResult {
                target,
                caller: self.base.me,
                stimulus_type: StimulusType::CallInstruction,
                info: StimulusInfo::Hint(Hint {
                    seek_point,
                    seek_flags: self.pending_group_instruction_seek_flags,
                    who_tells_me: self.base.me,
                }),
                continuation: ThinkResultContinuation::OfficerInstructedGroupSoldier { last },
            });
    }

    fn resume_failed_alert_soldiers(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        continuation: AlertSoldiersFailureContinuation,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        match continuation {
            AlertSoldiersFailureContinuation::None => {}
            AlertSoldiersFailureContinuation::ReturnToDuty => {
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            }
            AlertSoldiersFailureContinuation::SeekBody { center, radius } => {
                self.seek_area(
                    sim,
                    center,
                    radius,
                    SeekFlags::LOCATION_END | SeekFlags::BODY_SEEK,
                    UNDEFINED_DIRECTION,
                    global,
                    ctx,
                    tick,
                );
            }
            AlertSoldiersFailureContinuation::SeekMissingInstructedSoldier => {
                self.seek_area(
                    sim,
                    ctx.position,
                    parameters_ai::AI_DEAD_BODY_SEEK_RADIUS as u16,
                    SeekFlags::LOCATION_FIRST | self.seek_flags,
                    UNDEFINED_DIRECTION,
                    global,
                    ctx,
                    tick,
                );
            }
            AlertSoldiersFailureContinuation::SeekMissedCharly { center } => {
                let charly_has_path = ctx
                    .entity_view(self.base.checkpoint_charly)
                    .is_some_and(|view| view.has_patrol_path);
                let radius = if charly_has_path {
                    parameters_ai::AI_PATROL_CHARLY_SEEK_RADIUS as u16
                } else {
                    parameters_ai::AI_FIX_CHARLY_SEEK_RADIUS as u16
                };
                self.seek_area(
                    sim,
                    center,
                    radius,
                    SeekFlags::LOCATION_FIRST | SeekFlags::CHARLY_SEEK,
                    UNDEFINED_DIRECTION,
                    global,
                    ctx,
                    tick,
                );
            }
            AlertSoldiersFailureContinuation::FleeingRunToDoor => {
                self.set_state(AiState::Fleeing, Substate::FleeingRunToDoor);
                self.base.fire_self_stimulus(StimulusType::EventReachPoint);
            }
        }
    }

    pub(crate) fn finalize_alert_soldiers(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        failure: AlertSoldiersFailureContinuation,
        global: &mut AiGlobalState,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        let first_new_order = self.base.outbox.actor.orders.len();
        if !self.finish_alert_soldiers(global, grid, ctx, tick) {
            self.resume_failed_alert_soldiers(sim, failure, global, ctx, tick);
        }
        // AlertSoldiers synchronously calls each recipient's Think before
        // returning to its caller. Rust resumes that caller tail from the
        // cross-NPC action queue, after the physical recursion counter has
        // unwound. A route authored by the resumed formation/failure tail is
        // nevertheless still inside the original enclosing Think and must
        // deliver a same-frame failure back through EndThink.
        if self.base.outbox.actor.orders.len() > first_new_order {
            self.base.completion_latch_inside_think = true;
        }
    }

    /// 180°-detection (the simple-geometry half that can be answered
    /// from AI context alone).
    ///
    /// Short-circuits:
    ///   1. viewer sector is a building → false
    ///   2. either side inactive → false
    ///   3. beyond real view radius → false
    ///   4. within 50 units and "beside me" (perpendicular > forward
    ///      component length) → true (no LOS required)
    ///   5. dot(view, forward) < 0 (target is behind me) → false
    ///   6. beyond the spherical, light-modulated view radius computed
    ///      on the target's surface → false
    ///   7. full-ray opaque LOS check → final answer
    ///
    /// Step 6 is not just a filter: at night and in fog computing the
    /// radius samples the surrounding shadow-light sectors, and the
    /// results land in the shared per-surface radius cache, so it has
    /// to run for exactly the targets that reach it.
    pub(crate) fn is_detecting_180_degrees(
        &self,
        target: impl IntoOptionalAiHandle,
        ctx: &AiContext,
    ) -> bool {
        let target = target
            .into_optional_ai_handle()
            .expect("is_detecting_180_degrees requires a non-null target")
            .get();
        tracing::trace!(
            target,
            viewer_x = ctx.position.x,
            viewer_y = ctx.position.y,
            in_building = ctx.in_building,
            "is_detecting_180_degrees: entry"
        );
        context_detects_180_degrees(self.base.me, target, ctx)
    }

    /// `IsDetecting180Degrees` evaluated on another soldier's behalf.
    ///
    /// `IsTooProudToAttack` asks whether a lower-pride ally is observing
    /// our primary target, so the viewer of that test is the ally, not the
    /// deciding soldier. The geometry comes from the ally's entity view;
    /// the post-`RefreshView` radius, cone direction and aperture come from
    /// its camp-soldier snapshot.
    fn is_detecting_180_degrees_from(
        &self,
        viewer_handle: HumanHandle,
        target: HumanHandle,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        let Some(viewer_view) = ctx.entity_view(viewer_handle) else {
            tracing::warn!(
                viewer = viewer_handle,
                target,
                "is_detecting_180_degrees_from: viewer has no entity view"
            );
            return false;
        };
        let Some(viewer_snapshot) = tick
            .camp_soldiers
            .iter()
            .find(|soldier| soldier.handle == viewer_handle)
        else {
            tracing::warn!(
                viewer = viewer_handle,
                target,
                "is_detecting_180_degrees_from: viewer is absent from the camp-soldier snapshot"
            );
            return false;
        };
        let viewer_eye = crate::stealth::eye_point_xy(
            viewer_view.detection_position,
            viewer_view.posture,
            viewer_view.direction as i16,
            false,
        );
        let viewer = Viewer180 {
            entity: view_radius_memo_viewer(viewer_handle, ctx),
            eye_ground: crate::coordinates::GroundPoint::from_map_and_z(
                viewer_eye,
                viewer_view.elevation,
            ),
            eye_z: viewer_view.elevation
                + crate::stealth::eye_z_for_posture(viewer_view.posture, viewer_view.is_rider),
            direction: viewer_view.direction,
            in_building: viewer_view.in_building,
            view_radius: viewer_snapshot.view_radius,
            sq_view_radius: (viewer_snapshot.view_radius as f32)
                * (viewer_snapshot.view_radius as f32),
            view_direction: viewer_snapshot.view_direction,
            real_half_aperture: viewer_snapshot.real_half_aperture,
        };
        detects_180_degrees(&viewer, target, ctx)
    }
}

/// Standalone actor-side `IsDetecting180Degrees`, shared with engine-owned
/// sweeps whose viewer can be a civilian and therefore has no `EnemyAi`.
pub(crate) fn context_detects_180_degrees(
    viewer_handle: HumanHandle,
    target: HumanHandle,
    ctx: &AiContext,
) -> bool {
    let viewer = Viewer180 {
        entity: view_radius_memo_viewer(viewer_handle, ctx),
        // `self_eye_position` is built directly from the element's raw
        // position by `ComputeEyesPoint`. `ctx.position` may instead be an
        // AI-facing substituted position (a door endpoint/carrier).
        eye_ground: crate::coordinates::GroundPoint::from_map_and_z(
            ctx.self_eye_position,
            ctx.elevation,
        ),
        eye_z: ctx.self_eye_z,
        direction: ctx.direction,
        in_building: ctx.in_building,
        view_radius: ctx.self_view_radius,
        sq_view_radius: ctx.sq_self_view_radius,
        view_direction: ctx.self_view_direction,
        real_half_aperture: ctx.self_real_half_aperture,
    };
    detects_180_degrees(&viewer, target, ctx)
}

/// Resolve the identity a `ComputeViewRadius` result is stored under.
/// The memo lives on the surface and records which viewer produced it, so
/// the acting NPC and any ally it reasons through must be distinguishable.
fn view_radius_memo_viewer(handle: HumanHandle, ctx: &AiContext) -> crate::element::EntityId {
    ctx.entity_id(handle).unwrap_or_else(|| {
        panic!("view-radius memo viewer {handle} is absent from the AI entity view")
    })
}

/// Viewer half of a 180° detection test, so the test can be evaluated
/// either from the acting NPC or from an ally it is reasoning about.
struct Viewer180 {
    /// Identity the surface radius memo is keyed by — the ally when the
    /// test runs through an ally's eyes, not the deciding soldier.
    entity: crate::element::EntityId,
    eye_ground: crate::coordinates::GroundPoint,
    eye_z: f32,
    direction: u16,
    in_building: bool,
    view_radius: u16,
    sq_view_radius: f32,
    view_direction: [f32; 2],
    real_half_aperture: f32,
}

fn detects_180_degrees(viewer: &Viewer180, target: HumanHandle, ctx: &AiContext) -> bool {
    // Step 1: viewer in a building — always returns false.
    if viewer.in_building {
        return false;
    }
    let Some(view) = ctx.entity_view(target) else {
        tracing::trace!(
            target,
            "is_detecting_180_degrees: entity_view lookup failed"
        );
        return false;
    };
    // Step 2: Original checks the raw RHElement::IsActive flag, not whether
    // the target can fight. An unconscious actor remains active and can
    // therefore still pass this standalone 180-degree visibility test.
    if !view.active {
        return false;
    }

    let viewer_eye_z = viewer.eye_z;
    // ComputeDetectionPoint starts from the raw element position. The
    // AI-facing `view.position` may be a substituted door endpoint/carrier.
    let target_detection_world = crate::stealth::detection_point_world(
        view.detection_position_world,
        view.posture,
        view.direction as i16,
        view.is_rider,
    );
    let target_detection_z = target_detection_world.z;
    let viewer_eye_ground = viewer.eye_ground;
    let target_detection_ground =
        crate::coordinates::GroundPoint::new(target_detection_world.x, target_detection_world.y);

    // Aspect-ratio-stretched view vector (`INVERSE_ASPECT_RATIO`
    // on the Y component), from viewer eye to target detection point.
    let dx = target_detection_ground.x - viewer_eye_ground.x;
    let dy = (target_detection_ground.y - viewer_eye_ground.y)
        * crate::position_interface::INVERSE_ASPECT_RATIO;
    let sq_distance = dx * dx + dy * dy;
    tracing::trace!(
        target,
        viewer_x = viewer_eye_ground.x,
        viewer_y = viewer_eye_ground.y,
        viewer_z = viewer_eye_z,
        target_x = target_detection_ground.x,
        target_y = target_detection_ground.y,
        sq_distance,
        sq_view_radius = viewer.sq_view_radius,
        "is_detecting_180_degrees: geometry"
    );
    if sq_distance > viewer.sq_view_radius {
        return false;
    }

    // `GetDirectionVector()` first compresses the table Y by
    // ASPECT_RATIO; Original then stretches it back here.  The shared
    // Rust table is already the resulting uncompressed unit vector, so
    // applying INVERSE_ASPECT_RATIO a second time would narrow the
    // forward half-plane incorrectly.
    let dir = crate::shadow_polygon::sector_to_direction(viewer.direction as i16);
    let fx = dir[0];
    let fy = dir[1];

    // Step 4: very-near "beside me" short-circuit.
    if sq_distance < 50.0 * 50.0 {
        let fwd_len = dx * fx + dy * fy;
        let fc_x = fx * fwd_len;
        let fc_y = fy * fwd_len;
        let perp_sq = (dx - fc_x) * (dx - fc_x) + (dy - fc_y) * (dy - fc_y);
        if perp_sq >= fwd_len {
            return true;
        }
    }

    // Step 5: forward half-plane.
    if dx * fx + dy * fy < 0.0 {
        return false;
    }

    // Step 6: second, tighter radius gate against the spherical and
    // light-modulated radius. At night and in fog this is where the
    // viewer samples the surrounding shadow-light sectors, so it must
    // run for every target that survives the gates above — and only for
    // those, since the sampling is observable through the shared
    // per-surface radius cache.
    let sight_obstacles = ctx.obstacle_list();
    let target_obstacle = view.obstacle_idx.map(|handle| {
        sight_obstacles.get(usize::from(handle)).unwrap_or_else(|| {
            panic!(
                "is_detecting_180_degrees: target {target} requires missing sight obstacle {handle}"
            )
        })
    });
    let compute_radius = || {
        crate::ai_vision::compute_view_radius(
            crate::coordinates::WorldPoint3D::new(
                viewer_eye_ground.x,
                viewer_eye_ground.y,
                viewer_eye_z,
            ),
            viewer.view_radius,
            (viewer.view_direction[0], viewer.view_direction[1]),
            viewer.real_half_aperture,
            ctx.is_night_or_fog,
            &ctx.fast_grid,
            sight_obstacles,
            target_obstacle,
        )
    };
    let effective_view_radius =
        ctx.compute_view_radius_cached(viewer.entity, view.obstacle_idx, compute_radius);
    if sq_distance > effective_view_radius * effective_view_radius {
        return false;
    }

    crate::sight_obstacle::is_reachable_3d(
        ctx.obstacle_list(),
        [viewer_eye_ground.x, viewer_eye_ground.y, viewer_eye_z],
        [
            target_detection_ground.x,
            target_detection_ground.y,
            target_detection_z,
        ],
        crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
    )
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
            // MaxNorm — Chebyshev distance.
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
        // Original: `vStareVector = mpMe->GetViewParameters()->starePoint
        // - mpMe->GetPositionGround();`
        // (`original-code/RHartificialmalignity.cpp:5381`).
        // `RHElement::GetPositionGround()` is the raw sprite point
        // (`original-code/RHElement.h:328` -> `GetPosition().mX/.mY`), *not*
        // `RHArtificialIntelligence::Position(mpMe)`. The two differ only
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
        // Original constructs this with `SBGeoVector2D::SetSector0to15`,
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
        // Original forecasts the human carried by this OUTOFVIEW stimulus,
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
                // ForecastDestinationForIA input for this OUTOFVIEW call.
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
                    // LaunchSequenceElement(QuitSwordfight) interrupts the
                    // selected actor command synchronously. Its condolence
                    // re-enters Think before this outer handler continues
                    // into GetBattleOverview.
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
        let my_camp = ctx.camp;
        let my_pos = ctx.self_body_position_world;
        // The engine performs the state-filtered registry walk against live
        // recipients. Use this snapshot only to avoid suspending the caller
        // when no same-camp soldier is even in range.
        let any_friend_in_range = ctx
            .entity_views
            .iter()
            .filter(|(handle, view)| {
                **handle != self.base.me && view.is_soldier() && view.camp == my_camp
            })
            .any(|(_, view)| {
                // Original subtracts each friend's raw
                // RHElement::GetPosition() from the caller's raw element
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

        // HeyFolksLookThere is a plain synchronous call, so everything above
        // still runs inside the Think that suspended here and its EndThink
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
        // statement order in Original SetState.
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
        // immediately before SetState(...RunningToPhalanx) must survive.
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
                // Original calls UpdateLeftCombatNeighbour(NULL) and
                // UpdateRightCombatNeighbour(NULL), which clear the reverse
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
        if (self.my_shooting_point.is_some() || self.my_archery_sector.is_some())
            && !matches!(
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

        // Leaving STATE_SEEKING also runs
        // `delete_all_detectables(Beggar)` and zeroes
        // `beggar_to_examine`.  Queue the detectable scrub and clear
        // the field directly so the next seek-cycle can re-populate
        // them cleanly.
        if self.base.current_state == AiState::Seeking && state != AiState::Seeking {
            self.base
                .outbox
                .actor
                .delete_detectables
                .push(crate::element::DetectableType::Beggar);
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
            // Calls made before SetState (most importantly StopAll) belong
            // inside the synchronous SetState boundary. Detach that prefix
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
                .outbox
                .reentrant
                .owner_work
                .push(AiOwnerWork::StateChange(AiStateChangeNotification {
                    outgoing_state: self.base.current_state,
                    outgoing_substate: self.base.current_substate,
                    incoming_state: state,
                    incoming_substate: substate,
                    source,
                    actor_effects_before_callback,
                }));
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
                    // Original SetState's attentive-mode switch does nothing
                    // for GotStop, but it still falls through to the shared
                    // SetAlertStatus(ALERT_YELLOW) tail.
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
        // Think(EVENT_DONE) only when that Think is admissible. StartThink
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

    /// Apply common patrol geometry through the enemy virtual `SetState`.
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

    /// `SetState(Default, ...)` calls `SetAttentiveMode(false)` before the
    /// following `GoTo`. Only hold the movement when that call actually
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
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
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
    // StartThink — pre-think checks
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

    /// `StartThink` work which precedes the script `FilterAIEvent` call.
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

    /// `StartThink` work after `FilterAIEvent`. The return value is the
    /// ordinary Think admission decision; SetAIState intentionally observes
    /// these gates but ignores the bool before running SeekArea/Panic.
    pub(crate) fn start_think_post_filter(
        &mut self,
        stimulus: &Stimulus,
        ctx: &AiContext,
        static_ai_frozen: bool,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;

        // The original clears callback-produced completion flags again after
        // an accepted filter and before evaluating the remaining gates.
        self.base.couldnt_reachpoint = false;
        self.base.already_on_point = false;
        self.base.already_turned = false;

        // Script event filtering runs at the engine dispatch site
        // before this `think()` is invoked — see
        // `Engine::filter_stimulus`.  The freeze and script-lock
        // checks below run after that.

        // Original static `RHArtificialIntelligence::mbFreeze` is distinct
        // from both engine FreezeAll and the per-NPC AILOCK_FREEZE bit. The
        // NPC still scans detection, but StartThink discards each event.
        if static_ai_frozen {
            self.base.register_log_line(LogLineType::EventRefused, 1);
            return false;
        }

        // Check script lock
        if self.base.script_locked {
            if self.base.remember_events {
                match stimulus_type {
                    StimulusType::EventDone | StimulusType::EventReachPoint => {
                        // Gameflow commands — ignore
                    }
                    _ => {
                        self.base.stimulus_queue.push(*stimulus);
                    }
                }
            }
            self.base.register_log_line(LogLineType::EventRefused, 2);
            return false;
        }

        // Every non-script AILOCK flag retains stimuli. Original's separate
        // static `mbFreeze` discard gate is not the per-NPC AILOCK_FREEZE bit;
        // engine-wide Rust freeze is handled before NPC Hourglass work.
        if !self.base.locks_flag_field.is_empty() {
            self.base.stimulus_queue.push(*stimulus);
            self.base.register_log_line(LogLineType::EventRefused, 3);
            return false;
        }

        // Special substates that block most events
        if self.base.current_substate == Substate::WonderingWaspInArmour {
            match stimulus_type {
                StimulusType::EventLoseConsciousness | StimulusType::EventWaspAway => {}
                _ => {
                    self.base.register_log_line(LogLineType::EventRefused, 4);
                    return false;
                }
            }
        }
        if self.base.current_substate == Substate::WonderingUnderNet {
            match stimulus_type {
                StimulusType::EventLoseConsciousness | StimulusType::EventNetAway => {}
                _ => {
                    self.base.register_log_line(LogLineType::EventRefused, 5);
                    return false;
                }
            }
        }
        if self.base.current_substate == Substate::FleeingMerryManLeaveMap
            && stimulus_type != StimulusType::EventReachPoint
        {
            self.base.register_log_line(LogLineType::EventRefused, 6);
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

        // Reset standing around timer
        self.base.standing_around_timer = 0;

        // Handle timer messages — ignore stale timers
        if self.base.timer_is_running {
            if self.base.current_substate != self.base.substate_at_last_timer_launch {
                // Kill stale timer
                self.base.timer_is_running = false;
            }
        } else if stimulus_type == StimulusType::EventTimer
            && self.base.current_substate != self.base.substate_at_last_timer_launch
        {
            self.base.register_log_line(LogLineType::EventRefused, 9);
            return false;
        }

        // Dead guys ignore everything.
        // Defence-in-depth: the tick loop normally skips dead entities, but
        // scripts and cross-NPC actions can still fire stimuli at a corpse.
        if ctx.self_is_dead {
            self.base.register_log_line(LogLineType::EventRefused, 10);
            return false;
        }

        // Unconscious NPCs ignore all messages except FitAgain
        if self.base.current_substate == Substate::SleepingUnconscious
            && stimulus_type != StimulusType::EventFitAgain
        {
            self.base.register_log_line(LogLineType::EventRefused, 11);
            return false;
        }

        // FitAgain only valid when unconscious or napping — and if
        // carried, refused even when unconscious ("it's a little
        // late to be awaken" when posture == Carried).
        if stimulus_type == StimulusType::EventFitAgain {
            match self.base.current_substate {
                Substate::SleepingUnconscious | Substate::SleepingNapping => {}
                _ => {
                    self.base.register_log_line(LogLineType::EventRefused, 12);
                    return false;
                }
            }
            if ctx.posture == crate::element::Posture::Carried {
                self.base.register_log_line(LogLineType::EventRefused, 7);
                return false;
            }
        }

        // Handle special events processed during StartThink. In Original
        // these run after the timer, dead, and sleeping-unconscious gates;
        // notably, a second LoseConsciousness stimulus cannot rewrite the AI
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
    // EndThink — post-think event dispatch
    // -----------------------------------------------------------------------

    pub(crate) fn end_think(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        _global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        _grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        // legacy implementation EndThink calls Think(EVENT_*) here, and Think runs the
        // script FilterAIEvent gate before dispatch. Queue these as
        // same-frame self-stimuli so the engine-side drain can apply
        // that filter without re-entering the script VM through this
        // borrowed AI object.

        let mut queued_completion = false;

        // Post CouldntReachPoint event if a GoTo failed
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
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            }
        }

        // Post ReachPoint event if GoTo was already at destination
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
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
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
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            }
        }

        if queued_completion {
            // Original EndThink dispatches the completion Think recursively
            // *before* its decrement, so the cascade's ancestor frames stay
            // open and the recursion depth climbs one per nested Think —
            // that climb is what makes the 100.. ReturnToDuty failsafe
            // reachable. This frame stays open until the cascade ends (see
            // `open_end_think_frames`).
            self.base.open_end_think_frames = self.base.open_end_think_frames.saturating_add(1);
        } else if self.base.defer_end_think_for_engine_completion() {
            // Rust learns an engine-owned GoTo failure after releasing this
            // AI borrow. Keep the Original EndThink frame alive until that
            // synchronous path verdict is surfaced.
        } else {
            // No continuation was queued: this is the innermost Think of the
            // cascade, so the entire chain of still-open ancestor frames
            // unwinds with it — the deferred equivalent of the stacked
            // EndThink decrements the Original performs while returning out
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
    // UpdateNewTaskPriority
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
    // ReturnToDuty — return to default behavior
    // -----------------------------------------------------------------------

    pub fn return_to_duty(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        flags: DutyFlags,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        // DeleteAllDetectables(DETECTABLE_BEGGAR) is synchronous in
        // Original. In particular, SeekNextPoint can call ReturnToDuty after
        // SeekArea queued beggars earlier in the same borrowed AI dispatch;
        // erase those earlier additions before queuing the bucket scrub so
        // the later outbox drain cannot replay them after the delete.
        self.base
            .outbox
            .actor
            .add_detectables
            .retain(|(_, detectable_type)| {
                *detectable_type != crate::element::DetectableType::Beggar
            });
        self.base
            .outbox
            .actor
            .delete_detectables
            .push(crate::element::DetectableType::Beggar);
        self.beggar_to_examine = None;
        self.beggar_is_npc = false;
        self.clear_swordstrike_experiences();
        // Focus(NULL) — release any stare/follow target before the
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
        // Also gates on `interesting_object == NULL ||
        // !IsAnyAngryOfficerNear(...)`: if we still remember a specific
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
                // GetNearestSeenMoneyAndRemoveItFromList: picks the
                // closest live coin (MaxNorm, +300 layer malus) after
                // sweeping inactive entries, rather than popping by
                // insertion order.
                if let Some(coin) = self.get_nearest_seen_money_and_remove_it_from_list(ctx) {
                    self.base.interesting_object = Some(AiEntityHandle::new(coin));
                }
            }
            // GoNear the interesting-object position. Look up the freshly
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
                // RHArtificialMalignity::ReturnToDuty remembers where the
                // patrol was interrupted so the soldier returns there after
                // finishing this newly remembered ale.
                self.return_to_patrol_point = ctx.position;
                self.base.launch_timer(1, ctx.frame);
                return;
            }
            self.base.interesting_object = None;
            self.base.object_of_desire = None;
        }

        // Original calls InitializePatrol synchronously, then immediately
        // enters ReturnToDutyCommonStuff. Patrol admission needs the engine's
        // entity table and can itself issue authoritative visibility queries,
        // so suspend the tail at the owner boundary instead of setting the
        // frame-deferred `needs_patrol_reinit` flag. This also lets a patrol
        // member observe the chief assignment written moments earlier and
        // perform its reciprocal member -> chief visibility query in-order.
        //
        // Clear the reconnaissance report here rather than leaving it to the
        // suspended `ReturnToDutyCommonStuff`. Because the whole return runs
        // synchronously in the reference, callers observe a cleared report
        // the instant the return completes — `SeekNextPoint` reads it on the
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

    /// Resume the non-engine half of Original Enemy `ReturnToDuty` after its
    /// inline `InitializePatrol` call has returned.
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

        // `ReturnToDutyCommonStuff` enters through virtual Enemy `SetState` in
        // Original. That override forgets the old timer before the common tail
        // decides whether to launch a fresh bored timer.
        self.base.timer_is_running = false;
        let resumed_depth = self.base.think_recursion_depth;
        if high_recursion_failsafe && resumed_depth == 0 {
            self.base.think_recursion_depth = 100;
        }
        self.base.return_to_duty_common_stuff(sim, flags, ctx);
        if high_recursion_failsafe && resumed_depth == 0 {
            self.base.think_recursion_depth = resumed_depth;
            // Original's high-recursion ReturnToDuty is already inside the
            // currently executing EndThink branch. Its newly-set latch is
            // not revisited by that branch; it survives the unwind and is
            // cleared by the next StartThink. The deferred Rust boundary has
            // no matching EndThink left, so prevent the generic completion
            // surfacer from converting it into an immediate self event.
            self.base.completion_latch_inside_think = false;
        }
        let incoming_state = self.base.current_state;
        let incoming_substate = self.base.current_substate;

        // The shared common routine assigns Default directly instead of
        // entering through RHArtificialMalignity::SetState. Preserve the
        // shield-bearer half of that virtual override too: leaving the three
        // protection substates clears both mpArcherBehindMe and the archer's
        // reciprocal mpShieldBearerBeforeMe before the next NPC owner runs.
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

        // ReturnToDutyCommonStuff calls the virtual Enemy SetState in Original.
        // When an archer leaves the bow substates, that override clears
        // mpShieldBearerBeforeMe and the shield bearer's reciprocal
        // mpArcherBehindMe before the next AI owner runs
        // (`RHartificialmalignity.cpp:9296-9314`). The shared Rust base assigns
        // the destination state directly, so restore that omitted virtual tail
        // here at the same owner boundary.
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

        // `ReturnToDutyCommonStuff` reaches its destination state through the
        // virtual `SetState` call in Original.  The shared Rust base assigns
        // that state directly, so restore the Enemy override's synchronous
        // leaving-Menacing relationship cleanup.  In particular, the
        // reciprocal PC guard must be cleared before a later NPC owner slot
        // runs RefreshDetection; an unobserved guarded PC is rejected before
        // the otherwise-authoritative visibility query.
        if outgoing_state == AiState::Menacing && self.base.current_state != AiState::Menacing {
            self.set_guarded_pc(None);
        }

        // `ReturnToDutyCommonStuff` reaches the new state through virtual
        // Enemy `SetState` in Original.  The shared Rust base assigns the
        // fields directly, so restore the override's combat-neighbour
        // teardown as well.  Original's two line-mode switches both inspect
        // the incoming substate (an authored quirk), which reduces the clear
        // condition to "the destination is not a phalanx or real swordfight
        // substate".  Queue the reciprocal clears before dropping our local
        // links, matching UpdateLeft/RightCombatNeighbour(NULL).
        let incoming_keeps_combat_neighbours = matches!(
            incoming_substate,
            Substate::AttackingPhalanx
                | Substate::AttackingRunningToPhalanx
                | Substate::AttackingProtectingWithShield
        ) || incoming_substate.is_real_swordfight();
        if !incoming_keeps_combat_neighbours {
            self.clear_combat_neighbours();
        }

        // `ReturnToDutyCommonStuff` calls the virtual Enemy `SetState` in
        // C++. The shared Rust base performs the state assignment directly.
        // Restore the Enemy override's attentive-mode tail: every Default
        // substate requests ordinary (or forced) attention, which may launch
        // LeaveAttentiveMode alongside the return route. The shared common
        // routine has already built the route because it cannot invoke the
        // Enemy override directly, so restore Original's authored
        // SetState-before-GoTo barrier on only the orders emitted by this
        // return-to-duty tail.
        self.base
            .outbox
            .actor
            .queue_set_attentive_mode(AttentiveModeEffect::new(self.forced_attentive, false));
        self.hold_new_orders_behind_attentive(first_new_order);

        // TODO: move the complete Enemy virtual SetState boundary into the
        // shared return-to-duty routine. This closes final owner-boundary
        // publication, but the deferred Rust model still queues the earlier
        // ReturnToDuty actor prefix after owner-work StateChange callbacks;
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
    // Port of RHArtificialMalignity::React
    // -----------------------------------------------------------------------

    pub fn react(&mut self, max_reactiontime: u16, ctx: &AiContext, tick: &AiPerTickData) {
        if self.is_merry_man_forest(ctx) {
            self.base.launch_timer(3, ctx.frame);
            return;
        }

        // The slowdown only
        // applies when the NPC is Lacklandist *and* difficulty is Easy or Hard.
        // Royalist soldiers (also EnemyAi-driven) and Medium difficulty leave
        // the modifier at 1.0. The original's Easy==Hard copy-paste bug is
        // optional: the gameplay tweak selects the intended Hard constant.
        let modifier = if ctx.camp.is_hostile_to(crate::element::Camp::Royalists) {
            match ctx.difficulty {
                crate::player_profile::DifficultyLevel::Easy => difficulty::EASY_REACTIONTIME,
                crate::player_profile::DifficultyLevel::Hard if tick.fix_hard_reaction_times => {
                    difficulty::HARD_REACTIONTIME
                }
                crate::player_profile::DifficultyLevel::Hard => difficulty::EASY_REACTIONTIME,
                crate::player_profile::DifficultyLevel::Medium => 1.0,
            }
        } else {
            1.0
        };

        // Use the raw profile intelligence directly — not the
        // difficulty-scaled `GetIQ()`. Using the scaled value here
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
    // GetNewPrimaryTarget
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
    /// for the owner-ordered shared scratch. ReconsiderSwordfightObservation
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
        let Some(owner_view) = ctx.entity_view(self.base.me) else {
            // Dense fights can deactivate/delete this owner earlier in the
            // same actor pass, after its normal-timer tail was admitted. A
            // dead owner cannot select a meaningful replacement target.
            // TODO: prune the admitted tail when the earlier slot removes
            // its owner instead of reaching this stale callback.
            tracing::warn!(
                me = self.base.me,
                "GetNewPrimaryTarget owner left the live entity view earlier in this frame"
            );
            return None;
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

            // Original GetNewPrimaryTarget reads every persistent Them-list
            // pointer's live position. Detection snapshots are intentionally
            // incomplete on timer/reach/cross-NPC dispatches and therefore
            // cannot be used as a distance cache here.
            let target = ctx.entity_view(enemy).unwrap_or_else(|| {
                panic!(
                    "GetNewPrimaryTarget owner {} has required Them-list entry {} missing from the live entity view",
                    self.base.me, enemy
                )
            });
            // `GetNewPrimaryTarget` calls the raw 3D `Distance` /
            // `MaxNormDistance` helpers, not AI `Position()`.  `position` is
            // deliberately door-aware and can point at the committed far
            // side of a selected PassDoor; use each element's stored world
            // position for this scoring path.
            let target_world = target.detection_position_world;
            let dx = target_world.x - owner_world.x;
            let dy =
                (target_world.y - owner_world.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
            let dz = target_world.z - owner_world.z;
            // Original Distance/MaxNormDistance subtract the actors'
            // GetPosition() values. GetPosition().y is map Y plus elevation,
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
    // AnswerQuestion — character-based decision making
    // -----------------------------------------------------------------------

    /// `hypothetical` corresponds to the original `bHypoteticalQuestion`
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
                Question::ShallITakeAle => self.soldier_profile_beer > 0,
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

    /// Port of `Q_HAS_THE_NEW_TASK_PRIORITY` body — shared between the
    /// indoor and outdoor branches of `AnswerQuestion`.
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
    // Tower guard
    // -----------------------------------------------------------------------

    /// TowerGuardCallAlert.
    /// Broadcasts a tower-guard alert: every same-camp soldier within
    /// `SQR_TOWER_GUARD_ALERT_RADIUS` that isn't itself a tower guard,
    /// isn't holed up in a building, and is able to help gets a
    /// `CALL_TOWER_GUARD_ALERT` stimulus via the synchronous owner-boundary
    /// Think queue.  The nearest reachable officer additionally gets a
    /// `CALL_TOWER_GUARD_CALLS_ME` so they come to investigate.  If no
    /// officer is in ear-shot but a "far officer" exists, the nearest
    pub fn init_one_ai(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> crate::ai::InitStateSideEffects {
        // Initialize the "old odds" accumulator used by the weighted
        // decision RNG (old_odds = 50).
        self.old_odds = 50;

        // Build the minion list from patrol_ids
        // (TransformPatrolIDsToRealPatrol).
        self.initialize_patrol();

        // go_to_duty = InitState() &&
        // !AIIsScriptLocked() && !AIIsLocked().  Evaluate the authored
        // initial-action and commit the matching AI-side state
        // transition first — the subclass tail below only runs when
        // the authored action allows it *and* the AI isn't locked.
        let fx = self.base.init_state(sim, ctx);

        let go_to_duty =
            fx.go_to_duty && !self.base.ai_is_script_locked() && !self.base.ai_is_locked();

        // If the soldier has a patrol path, walk onto it.
        if go_to_duty && self.base.has_patrol_path {
            // Snapshot the substate-at-last-timer-launch *before* the
            // SetState/ReturnToDuty pair so a subsequent
            // timer-expiry-against-launch-substate check at
            // `ai_enemy.rs:2915/2920` sees this snapshot rather than
            // the default `Substate::DefaultOnPost`.
            self.base.substate_at_last_timer_launch = self.base.current_substate;
            self.set_state(AiState::Default, Substate::DefaultEnroute);
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
        }

        // GoTo checks `think_method_recursion_depth > 0` and
        // either sets `already_on_point` (for the enclosing `EndThink`
        // to dispatch) or fires `Think(EVENT_REACHPOINT)` directly when
        // called outside a Think cycle.  `return_to_duty` above runs outside Think, so a
        // `GoTo` to a waypoint we're already standing on (e.g. a 1-
        // waypoint patrol where the spawn sits next to the waypoint)
        // sets `already_on_point = true` but nothing drains it — the
        // NPC never gets EVENT_REACHPOINT and the waypoint macro never
        // fires.  Queue a self-stimulus so the engine's next-tick
        // drain dispatches it (same shape as EndThink's cascade).
        if self.base.already_on_point {
            self.base.already_on_point = false;
            self.base
                .fire_self_stimulus(crate::ai::StimulusType::EventReachPoint);
        }
        // A failed GoTo and a no-op FaceTo raise their latches unconditionally,
        // with no outside-Think delivery path of their own. Outside a Think the
        // next Think entry simply discards them, so drop them here rather than
        // inventing completions the actor never receives.
        self.base.couldnt_reachpoint = false;
        self.base.already_turned = false;

        // Original InitOneAI stamps this after all patrol-path setup.
        self.base.last_hint_actuality = ctx.frame;

        fx
    }

    // -----------------------------------------------------------------------
    // IAmInTrouble — broadcast distress to nearby friends
    // -----------------------------------------------------------------------
    //
    // IAmInTrouble is a no-op in the shipped
    // game: the entire body is commented out behind a `/* ROBINME */`
    // block.  The hook stays here only because several combat paths
    // call it unconditionally when a fight starts.  If Pyro ever
    // un-stubs it, the reference would send
    // `EVENT_SEES_FRIEND_IN_TROUBLE` via the deferred Think queue.
    pub fn i_am_in_trouble(&mut self, _attacker: ElementHandle) {}

    // -----------------------------------------------------------------------
    // PassHouseDoor is empty ("CURRENTLY EMPTY").
    // Kept as a no-op hook for the two call sites in
    // `RHElementActor::Leave()/Enter()` that would otherwise need to
    // branch on entity type.
    // -----------------------------------------------------------------------

    pub fn pass_house_door(&mut self, _entering: bool) {}
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{DoorSeekInfo, House, cache_npc_villain_authorized_direct};
    use crate::ai_entity_view::{AiEntityView, AiEntityViewMap, EntityKind, NetCoverInfo};
    use crate::coordinates::MapPoint;
    use crate::element::{Camp, DetectableType, EyeStatus, Posture};
    use crate::entity_id::{EntityId, SoldierId};
    use crate::gate::{Door, DoorIndex, DoorType};
    use crate::order::OrderType;
    use crate::position_interface::SectorHandle;
    use crate::sight_obstacle::{ObstaclePoint, SharedSightObstacles, SightObstacle};
    use std::sync::Arc;

    fn test_position(x: f32, y: f32) -> Position {
        Position {
            x,
            y,
            sector: None,
            level: 0,
        }
    }

    fn soldier_view(pos: Position) -> AiEntityView {
        AiEntityView {
            original_creation_order: 41,
            position: pos,
            detection_position: crate::coordinates::MapPoint::new(pos.x, pos.y),
            detection_position_world: crate::coordinates::WorldPoint3D::new(pos.x, pos.y, 0.0),
            direction: 0,
            posture: Posture::Upright,
            camp: Camp::Royalists,
            is_pc: false,
            is_robin: false,
            is_vip: false,
            is_beggar: false,
            is_child: false,
            kind: EntityKind::Soldier,
            is_tower_guard: false,
            is_swordfighting: false,
            is_able_to_fight: true,
            active: true,
            is_unconscious: false,
            action_state: crate::element::ActionState::Waiting,
            is_moving_map: false,
            passing_door: false,
            obstacle_idx: None,
            in_building: false,
            building_sector: None,
            script_locked: false,
            forecasted_destination: crate::ai::PreparedForecastDestination::fixed(pos, 0),
            ai_state: AiState::Default,
            ai_substate: Substate::DefaultOnPost,
            current_animation: OrderType::WaitingUprightBored,
            elevation: 0.0,
            object_type: crate::element_kinds::ObjectType::None,
            is_dead: false,
            is_carried: false,
            is_archer: false,
            is_rider: false,
            stuck_under_net: false,
            covering_nets: Vec::new(),
            in_coma: false,
            guard: None,
            has_patrol_path: false,
            initial_position: pos,
            number_of_arrows: 0,
            rank: ProfileRank::Soldier,
            reported_to_officer: false,
            looted_after_money_fight: false,
            current_money: 0,
            macro_in_progress: false,
            path_current_waypoint_index: 0,
            path_last_waypoint_index: 0,
            path_forward_movement: true,
            patrol_hiking_path_index: None,
            interesting_object: 0,
            report_type: ReportType::Nothing,
            report_seek_position: pos,
            report_seen_bodies: Vec::new(),
            report_charly: 0,
        }
    }

    fn camp_soldier(handle: u32, position: Position) -> CampSoldierInfo {
        CampSoldierInfo {
            handle,
            active: true,
            position,
            position_world: crate::coordinates::WorldPoint3D::new(position.x, position.y, 0.0),
            direction: 0,
            rank: ProfileRank::Soldier,
            ai_state: AiState::Default,
            ai_substate: Substate::None,
            is_able_to_fight: true,
            is_dead: false,
            knocked_out_in_money_fight: false,
            primary_target: 0,
            pride: 0,
            is_able_to_help: true,
            script_locked: false,
            ai_lock_frozen: false,
            layer: 0,
            report_type: ReportType::Nothing,
            report_seek_position: Position::default(),
            report_seen_bodies: Vec::new(),
            report_charly: 0,
            alert_soldiers_point: Position::default(),
            patrol_chief: None,
            antagonist: 0,
            detected_body: 0,
            blood_alcohol: 0,
            duty_flag: false,
            is_tower_guard: false,
            company_number: 0,
            in_building: false,
            forecast_destination: None,
            detectable_bodies: Vec::new(),
            seek_position: Position::default(),
            current_task_priority: 0,
            minimal_task_priority: 0,
            view_direction: [1.0, 0.0],
            view_radius: 400,
            real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
            eye_blind: false,
        }
    }

    fn charly_to_officer_context(
        officer_position: Position,
        obstacles: Vec<SightObstacle>,
    ) -> AiContext {
        let mut officer = soldier_view(officer_position);
        officer.rank = ProfileRank::Officer;
        officer.ai_state = AiState::Default;
        officer.ai_substate = Substate::DefaultOnPost;

        // The acting Charly (handle 1) must be present in the entity view so
        // detection can resolve its viewer identity; keep its view fields in
        // lockstep with the context's self geometry below.
        let mut charly = soldier_view(test_position(0.0, 0.0));
        charly.direction = 4;

        let mut views = AiEntityViewMap::new();
        views.insert(1, charly);
        views.insert(2, officer);
        let obstacle_count = obstacles.len();
        AiContext {
            position: test_position(0.0, 0.0),
            frame: 100,
            direction: 4,
            posture: Posture::Upright,
            self_eye_position: crate::coordinates::MapPoint::new(0.0, 0.0),
            self_eye_z: 45.0,
            self_upright_eye_world: crate::coordinates::WorldPoint3D::new(0.0, 0.0, 45.0),
            self_view_direction: [1.0, 0.0],
            self_view_radius: 400,
            self_real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
            self_eye_status: EyeStatus::LookForward,
            sq_self_view_radius: 400.0 * 400.0,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            sight_obstacles: SharedSightObstacles {
                static_obstacles: Arc::new(obstacles),
                dynamic_obstacles: Arc::new(Vec::new()),
                static_active: Arc::new(vec![true; obstacle_count]),
            },
            ..AiContext::default()
        }
    }

    #[test]
    fn look_there_precheck_uses_raw_owner_geometry_during_door_transit() {
        let mut ai = EnemyAi::new(124);
        let raw_owner = crate::coordinates::WorldPoint3D::new(722.0, 1695.0, 160.0);
        let raw_friend = crate::coordinates::WorldPoint3D::new(713.0, 1663.0, 250.0);

        let mut owner_view = soldier_view(test_position(1709.0, 2228.0));
        owner_view.camp = Camp::Lacklandists;
        owner_view.passing_door = true;
        owner_view.detection_position_world = raw_owner;
        let mut friend_view = soldier_view(test_position(713.0, 1413.0));
        friend_view.camp = Camp::Lacklandists;
        friend_view.detection_position_world = raw_friend;

        let mut views = AiEntityViewMap::new();
        views.insert(124, owner_view);
        views.insert(184, friend_view);
        let ctx = AiContext {
            // AI Position() has snapped the owner to the distant gate point,
            // while RHElement::GetPosition() remains within the 100-unit
            // world radius of the raised friend.
            position: test_position(1709.0, 2228.0),
            self_body_position_world: raw_owner,
            camp: Camp::Lacklandists,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        assert!(ai.hey_folks_look_there(
            &test_position(1154.0, 1860.0),
            100,
            LookThereContinuation::EventView {
                enemy: 342,
                enemy_pos: test_position(1154.0, 1860.0),
            },
            &ctx,
        ));
        assert!(matches!(
            ai.base.outbox.reentrant.cross_npc_actions.as_slice(),
            [CrossNpcAction::BroadcastLookThere { caller: 124, .. }]
        ));
    }

    #[test]
    fn detecting_360_uses_raw_active_actor_geometry_during_door_transit() {
        let ai = EnemyAi::new(1);
        for (posture, unconscious) in [(Posture::Upright, true), (Posture::Tied, false)] {
            // AI Position() has already snapped the target to its far gate
            // endpoint, while ComputeDetectionPoint still reads the live raw
            // actor position near the observer.
            let mut target = soldier_view(test_position(900.0, 0.0));
            target.detection_position = MapPoint::new(20.0, 0.0);
            target.detection_position_world = crate::coordinates::WorldPoint3D::new(20.0, 0.0, 0.0);
            target.posture = posture;
            target.is_unconscious = unconscious;
            target.is_able_to_fight = false;
            target.passing_door = true;

            let mut views = AiEntityViewMap::new();
            views.insert(2, target.clone());
            let ctx = AiContext {
                // Self is also on a door rail: broad AI inside-building state
                // and snapped Position() must not replace the current-sector
                // or direct upright-eye geometry used by this overload.
                position: test_position(-900.0, 0.0),
                in_building: true,
                building_sector: None,
                self_upright_eye_world: crate::coordinates::WorldPoint3D::new(0.0, 0.0, 45.0),
                sq_self_view_radius: 200.0 * 200.0,
                entity_views: crate::ai_entity_view::shared_entity_views(views),
                ..AiContext::default()
            };
            assert!(
                ai.is_detecting_360_degrees(2, &ctx),
                "active {posture:?} target must use raw actor geometry"
            );

            let mut inactive = target.clone();
            inactive.active = false;
            let mut views = AiEntityViewMap::new();
            views.insert(2, inactive);
            assert!(!ai.is_detecting_360_degrees(
                2,
                &AiContext {
                    entity_views: crate::ai_entity_view::shared_entity_views(views),
                    ..ctx.clone()
                }
            ));

            let mut indoor = target;
            indoor.in_building = true;
            let mut views = AiEntityViewMap::new();
            views.insert(2, indoor);
            assert!(!ai.is_detecting_360_degrees(
                2,
                &AiContext {
                    entity_views: crate::ai_entity_view::shared_entity_views(views),
                    ..ctx
                }
            ));
        }
    }

    #[test]
    fn normal_detection_uses_raw_pass_door_target_position() {
        let ai = EnemyAi::new(1);
        let mut target = soldier_view(test_position(900.0, 0.0));
        target.detection_position = MapPoint::new(100.0, 0.0);
        target.detection_position_world = crate::coordinates::WorldPoint3D::new(100.0, 0.0, 0.0);
        target.passing_door = true;
        // Self view for the acting soldier (handle 1), matching the context's
        // self geometry so the viewer identity resolves during detection.
        let mut viewer = soldier_view(test_position(0.0, 0.0));
        viewer.direction = 4;
        let mut views = AiEntityViewMap::new();
        views.insert(1, viewer);
        views.insert(2, target);
        let ctx = AiContext {
            direction: 4,
            self_eye_position: MapPoint::ZERO,
            self_eye_z: 45.0,
            self_view_direction: [1.0, 0.0],
            self_view_radius: 400,
            self_real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
            self_eye_status: EyeStatus::LookForward,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        assert!(ai.is_detecting(2, &ctx));
    }

    fn charly_heading_to_officer() -> EnemyAi {
        let mut ai = EnemyAi::new(1);
        ai.base.antagonist = 2;
        ai.set_state(AiState::Seeking, Substate::SeekingCharlyGoToOfficer);
        ai
    }

    fn opaque_wall_across_x_axis() -> SightObstacle {
        let mut wall = SightObstacle::new_default(0);
        wall.obstacle_points = vec![
            ObstaclePoint {
                x: 95.0,
                y: -10.0,
                z_top: 80.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 105.0,
                y: -10.0,
                z_top: 80.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 105.0,
                y: 10.0,
                z_top: 80.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 95.0,
                y: 10.0,
                z_top: 80.0,
                z_bottom: 0.0,
            },
        ];
        wall.top_plane_points = [
            [95.0, -10.0, 80.0],
            [105.0, -10.0, 80.0],
            [95.0, 10.0, 80.0],
        ];
        wall.bottom_plane_points = [[95.0, -10.0, 0.0], [105.0, -10.0, 0.0], [95.0, 10.0, 0.0]];
        wall.rebuild_geometry();
        wall
    }

    #[test]
    fn detection_180_uses_raw_actor_xy_instead_of_ai_position() {
        let ai = EnemyAi::new(1);
        let mut viewer = soldier_view(test_position(0.0, 0.0));
        viewer.direction = 4;
        let mut target = soldier_view(test_position(200.0, 0.0));
        // AI Position() can be displaced from the raw element anchor, for
        // example while passing a door. Original's standalone 180° overload
        // calls ComputeDetectionPoint on the raw actor position instead.
        target.position = test_position(200.0, 30.0);

        // This wall intersects the AI-position ray (0,0)->(200,30) at
        // y=15, but not the original-compatible raw ray (0,0)->(200,0).
        let mut wall = SightObstacle::new_default(0);
        wall.obstacle_points = vec![
            ObstaclePoint {
                x: 95.0,
                y: 10.0,
                z_top: 80.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 105.0,
                y: 10.0,
                z_top: 80.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 105.0,
                y: 20.0,
                z_top: 80.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 95.0,
                y: 20.0,
                z_top: 80.0,
                z_bottom: 0.0,
            },
        ];
        wall.top_plane_points = [[95.0, 10.0, 80.0], [105.0, 10.0, 80.0], [95.0, 20.0, 80.0]];
        wall.bottom_plane_points = [[95.0, 10.0, 0.0], [105.0, 10.0, 0.0], [95.0, 20.0, 0.0]];
        wall.rebuild_geometry();

        let mut views = AiEntityViewMap::new();
        views.insert(1, viewer);
        views.insert(2, target);
        let ctx = AiContext {
            position: test_position(0.0, 0.0),
            direction: 4,
            posture: Posture::Upright,
            self_eye_position: MapPoint::ZERO,
            self_eye_z: 45.0,
            self_view_radius: 400,
            sq_self_view_radius: 400.0 * 400.0,
            self_view_direction: [1.0, 0.0],
            self_real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            sight_obstacles: SharedSightObstacles {
                static_obstacles: Arc::new(vec![wall]),
                dynamic_obstacles: Arc::new(Vec::new()),
                static_active: Arc::new(vec![true]),
            },
            ..AiContext::default()
        };

        assert!(ai.is_detecting_180_degrees(2, &ctx));
    }

    #[test]
    fn detection_180_los_uses_stored_world_point_without_projection_round_trip() {
        let ai = EnemyAi::new(1);
        let mut viewer = soldier_view(test_position(1373.0, 595.0));
        viewer.direction = 4;

        // These are representative moving-actor coordinates where
        // `(world_y - z) + z` rounds one ULP away from the stored world Y.
        // Original passes ComputeDetectionPoint's stored 3D point verbatim
        // to FastFindGrid::IsReachable.
        let raw = crate::coordinates::WorldPoint3D::new(
            1_555.961_5,
            f32::from_bits(1_143_810_793),
            46.786_65,
        );
        let mut target = soldier_view(test_position(raw.x, raw.y - raw.z));
        target.detection_position = MapPoint::from_world_xyz(raw.x, raw.y, raw.z);
        target.detection_position_world = raw;
        target.elevation = raw.z;
        assert_ne!(
            (target.detection_position.y + target.elevation).to_bits(),
            raw.y.to_bits(),
            "fixture must distinguish projection round-trip Y from stored world Y"
        );

        let mut views = AiEntityViewMap::new();
        views.insert(1, viewer);
        views.insert(2, target);
        let ctx = AiContext {
            direction: 4,
            self_eye_position: MapPoint::new(1373.0, 595.0),
            self_eye_z: 45.0,
            elevation: 45.0,
            self_view_radius: 400,
            sq_self_view_radius: 400.0 * 400.0,
            self_view_direction: [1.0, 0.0],
            self_real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        crate::sight_obstacle::begin_parity_visibility_capture();
        assert!(ai.is_detecting_180_degrees(2, &ctx));
        let queries = crate::sight_obstacle::take_parity_visibility_capture();
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].destination[0].to_bits(), raw.x.to_bits());
        assert_eq!(queries[0].destination[1].to_bits(), raw.y.to_bits());
        assert_eq!(
            queries[0].destination[2].to_bits(),
            (raw.z + crate::stealth::detection_z_for_posture(Posture::Upright, false)).to_bits()
        );
    }

    fn standalone_180_context(target: AiEntityView, radius: u16) -> AiContext {
        let mut viewer = soldier_view(test_position(0.0, 0.0));
        viewer.direction = 4;
        let mut views = AiEntityViewMap::new();
        views.insert(1, viewer);
        views.insert(2, target);
        AiContext {
            direction: 4,
            self_eye_position: MapPoint::ZERO,
            self_eye_z: 45.0,
            self_view_radius: radius,
            sq_self_view_radius: (radius as f32) * (radius as f32),
            self_view_direction: [1.0, 0.0],
            self_real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        }
    }

    #[test]
    fn standalone_180_allows_active_target_inside_building() {
        let mut target = soldier_view(test_position(100.0, 0.0));
        target.in_building = true;
        target.building_sector = SectorHandle::new(7);
        let ctx = standalone_180_context(target, 400);

        assert!(context_detects_180_degrees(1, 2, &ctx));
    }

    #[test]
    fn standalone_180_has_no_generic_standard_view_aabb() {
        // 500 is outside the generic NearbyCiviliansPanic standard-radius
        // AABB (400), but inside this civilian's live 600-unit radius.
        let target = soldier_view(test_position(500.0, 0.0));
        let ctx = standalone_180_context(target, 600);

        assert!(context_detects_180_degrees(1, 2, &ctx));
    }

    #[test]
    fn standalone_180_close_sideways_shortcut_precedes_opaque_los() {
        let target = soldier_view(test_position(0.0, 20.0));
        let mut ctx = standalone_180_context(target, 400);
        let mut wall = opaque_wall_across_x_axis();
        for point in &mut wall.obstacle_points {
            let (x, y) = (point.x, point.y);
            point.x = y;
            point.y = x - 85.0;
        }
        for point in wall
            .top_plane_points
            .iter_mut()
            .chain(wall.bottom_plane_points.iter_mut())
        {
            let (x, y) = (point[0], point[1]);
            point[0] = y;
            point[1] = x - 85.0;
        }
        wall.rebuild_geometry();
        ctx.sight_obstacles = SharedSightObstacles {
            static_obstacles: Arc::new(vec![wall]),
            dynamic_obstacles: Arc::new(Vec::new()),
            static_active: Arc::new(vec![true]),
        };

        crate::sight_obstacle::begin_parity_visibility_capture();
        assert!(context_detects_180_degrees(1, 2, &ctx));
        assert!(crate::sight_obstacle::take_parity_visibility_capture().is_empty());
    }

    #[test]
    fn standalone_180_applies_dynamic_ground_radius_before_los() {
        // Raw real radius accepts 399, while ComputeViewRadius projects the
        // 400-unit sphere at eye Z=45 to about 397.46 on the ground.
        let target = soldier_view(test_position(399.0, 0.0));
        let ctx = standalone_180_context(target, 400);

        crate::sight_obstacle::begin_parity_visibility_capture();
        assert!(!context_detects_180_degrees(1, 2, &ctx));
        assert!(crate::sight_obstacle::take_parity_visibility_capture().is_empty());
    }

    #[test]
    fn money_fight_enemy_rebuild_rechecks_current_unconscious_before_detection() {
        let mut ai = EnemyAi::new(1);
        let owner_position = test_position(0.0, 0.0);
        let candidate_position = test_position(100.0, 0.0);

        let owner = soldier_view(owner_position);
        let mut candidate = soldier_view(candidate_position);
        candidate.is_able_to_fight = false;
        candidate.is_unconscious = true;
        candidate.ai_state = AiState::Wondering;
        candidate.ai_substate = Substate::WonderingBrawlHitting;

        let mut views = AiEntityViewMap::new();
        views.insert(1, owner);
        views.insert(2, candidate);
        let ctx = AiContext {
            position: owner_position,
            self_eye_position: MapPoint::new(0.0, 0.0),
            self_eye_z: 45.0,
            self_upright_eye_world: crate::coordinates::WorldPoint3D::new(0.0, 0.0, 45.0),
            self_view_radius: 400,
            sq_self_view_radius: 400.0 * 400.0,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        // This top-of-tick entry is intentionally stale: the candidate was
        // conscious when the snapshot was built, then knocked out earlier in
        // the same creation-order AI pass.
        tick.camp_soldiers.push(CampSoldierInfo {
            handle: 2,
            active: true,
            position: candidate_position,
            position_world: crate::coordinates::WorldPoint3D::new(100.0, 0.0, 0.0),
            direction: 0,
            rank: ProfileRank::Soldier,
            ai_state: AiState::Wondering,
            ai_substate: Substate::WonderingBrawlHitting,
            is_able_to_fight: true,
            is_dead: false,
            knocked_out_in_money_fight: false,
            primary_target: 0,
            pride: 0,
            is_able_to_help: false,
            script_locked: false,
            ai_lock_frozen: false,
            layer: 0,
            report_type: ReportType::Nothing,
            report_seek_position: Position::default(),
            report_seen_bodies: Vec::new(),
            report_charly: 0,
            alert_soldiers_point: Position::default(),
            patrol_chief: None,
            antagonist: 0,
            detected_body: 0,
            blood_alcohol: 0,
            duty_flag: false,
            is_tower_guard: false,
            company_number: 0,
            in_building: false,
            forecast_destination: None,
            detectable_bodies: Vec::new(),
            seek_position: Position::default(),
            current_task_priority: 0,
            minimal_task_priority: 0,
            view_direction: [1.0, 0.0],
            view_radius: 400,
            real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
            eye_blind: false,
        });

        crate::sight_obstacle::begin_parity_visibility_capture();
        ai.create_new_list_of_money_fight_enemies(&tick, &ctx);
        let queries = crate::sight_obstacle::take_parity_visibility_capture();

        assert!(queries.is_empty(), "lifecycle gate must precede detection");
        assert!(ai.money_fight_enemies.is_empty());
    }

    #[test]
    fn money_fight_morale_coalesces_ordered_camp_and_sleeper_snapshots() {
        let mut ai = EnemyAi::new(1);
        ai.soldier_profile_money = 40;

        let owner_position = test_position(0.0, 0.0);
        let sleeping_position = test_position(300.0, 0.0);
        let fighter_position = test_position(100.0, 0.0);
        let dead_position = test_position(200.0, 0.0);
        let disjoint_sleeping_position = test_position(250.0, 0.0);

        let mut sleeping_view = soldier_view(sleeping_position);
        sleeping_view.original_creation_order = 3;
        sleeping_view.ai_substate = Substate::SleepingUnconscious;
        sleeping_view.is_unconscious = true;
        let mut fighter_view = soldier_view(fighter_position);
        fighter_view.original_creation_order = 2;
        fighter_view.ai_substate = Substate::WonderingBrawlHitting;
        let mut dead_view = soldier_view(dead_position);
        dead_view.original_creation_order = 4;
        dead_view.is_dead = true;
        let mut disjoint_sleeping_view = soldier_view(disjoint_sleeping_position);
        disjoint_sleeping_view.original_creation_order = 5;
        disjoint_sleeping_view.ai_substate = Substate::SleepingUnconscious;
        disjoint_sleeping_view.is_unconscious = true;

        let mut views = AiEntityViewMap::new();
        let mut owner_view = soldier_view(owner_position);
        owner_view.original_creation_order = 1;
        views.insert(1, owner_view);
        views.insert(2, fighter_view);
        views.insert(3, sleeping_view);
        views.insert(4, dead_view);
        views.insert(5, disjoint_sleeping_view);
        let ctx = AiContext {
            position: owner_position,
            self_eye_position: MapPoint::ZERO,
            self_eye_z: 45.0,
            self_upright_eye_world: crate::coordinates::WorldPoint3D::new(0.0, 0.0, 45.0),
            self_view_radius: 400,
            sq_self_view_radius: 400.0 * 400.0,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        let mut sleeping = camp_soldier(3, sleeping_position);
        // Deliberately stale: Original classifies the current AI substate
        // after the visibility query, not this earlier camp snapshot.
        sleeping.ai_substate = Substate::WonderingBrawlHitting;
        sleeping.knocked_out_in_money_fight = true;
        let mut fighter = camp_soldier(2, fighter_position);
        fighter.ai_substate = Substate::None;
        // Deliberately stale alive snapshot: an earlier actor killed this
        // soldier before our turn, so the current view must suppress LOS.
        let dead = camp_soldier(4, dead_position);
        let mut tick = AiPerTickData::stub();
        // Both source snapshots retain camp/handle order. Self and dead
        // entries do not query; handle 3 overlaps and must coalesce.
        tick.camp_soldiers = vec![camp_soldier(1, owner_position), fighter, sleeping, dead];
        tick.camp_unconscious_soldiers = vec![
            CampUnconsciousSoldierInfo {
                handle: 3,
                knocked_out_in_money_fight: true,
            },
            // Preexisting sleepers are absent from `camp_soldiers` in the
            // main detection builder and must still participate once.
            CampUnconsciousSoldierInfo {
                handle: 5,
                knocked_out_in_money_fight: true,
            },
        ];

        crate::sight_obstacle::begin_parity_visibility_capture();
        assert!(!ai.wants_to_continue_money_fight(&tick, &ctx));
        let queries = crate::sight_obstacle::take_parity_visibility_capture();
        assert_eq!(queries.len(), 3);
        assert_eq!(
            queries
                .iter()
                .map(|query| query.destination[0])
                .collect::<Vec<_>>(),
            vec![100.0, 300.0, 250.0],
            "one query per live candidate in authored camp-registry order"
        );
    }

    #[test]
    fn detection_180_accepts_an_active_unconscious_target() {
        let ai = EnemyAi::new(1);
        let mut viewer = soldier_view(test_position(0.0, 0.0));
        viewer.direction = 4;
        let mut target = soldier_view(test_position(100.0, 0.0));
        target.is_able_to_fight = false;
        target.is_unconscious = true;
        target.active = true;

        let mut views = AiEntityViewMap::new();
        views.insert(1, viewer);
        views.insert(2, target);
        let ctx = AiContext {
            direction: 4,
            self_eye_position: MapPoint::ZERO,
            self_eye_z: 45.0,
            self_view_radius: 400,
            sq_self_view_radius: 400.0 * 400.0,
            self_view_direction: [1.0, 0.0],
            self_real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        assert!(ai.is_detecting_180_degrees(2, &ctx));
    }

    #[test]
    fn charly_inside_view_cone_queues_synchronous_officer_report_without_transitioning() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = charly_heading_to_officer();
        let ctx = charly_to_officer_context(test_position(200.0, 0.0), Vec::new());

        ai.think_expected_event(
            sim,
            &Stimulus::new(StimulusType::EventTimer),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(ai.base.current_substate, Substate::SeekingCharlyGoToOfficer);
        assert!(matches!(
            ai.base.outbox.reentrant.cross_npc_actions.as_slice(),
            [CrossNpcAction::ReportBackToOfficer {
                officer: 2,
                charly: 1,
            }]
        ));
        assert_eq!(ai.base.when_does_timer_ring, 0);
    }

    #[test]
    fn accepted_officer_report_enters_seen_and_arms_ten_frame_timer() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = charly_heading_to_officer();
        let ctx = charly_to_officer_context(test_position(200.0, 0.0), Vec::new());

        ai.resolve_charly_officer_report(sim, true, &ctx, &AiPerTickData::stub());

        assert_eq!(
            ai.base.current_substate,
            Substate::SeekingCharlyGoToOfficerSeen
        );
        assert_eq!(ai.base.when_does_timer_ring, 110);
        assert_eq!(
            ai.base.substate_at_last_timer_launch,
            Substate::SeekingCharlyGoToOfficerSeen
        );
    }

    #[test]
    fn refused_officer_report_returns_charly_to_duty() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = charly_heading_to_officer();
        let ctx = charly_to_officer_context(test_position(200.0, 0.0), Vec::new());

        ai.resolve_charly_officer_report(sim, false, &ctx, &AiPerTickData::stub());

        // The refused report enters ReturnToDuty, which suspends its common
        // tail at the owner boundary so the engine can run InitializePatrol
        // in between. Drain that continuation directly for the unit check.
        let resume = std::mem::take(&mut ai.base.outbox.reentrant.owner_work)
            .into_iter()
            .find_map(|work| match work {
                AiOwnerWork::ResumeReturnToDutyAfterPatrolInit { flags, .. } => Some(flags),
                _ => None,
            })
            .expect("refused report queues the return-to-duty continuation");
        ai.resume_return_to_duty_after_patrol_init(sim, resume, &ctx, false);

        assert_eq!(ai.base.current_state, AiState::Default);
        assert_eq!(ai.base.current_substate, Substate::DefaultGotoPost);
        assert_eq!(ai.base.antagonist, 0);
    }

    #[test]
    fn normal_detection_uses_raw_active_outside_gate_not_able_to_fight() {
        let ai = charly_heading_to_officer();
        let mut ctx = charly_to_officer_context(test_position(200.0, 0.0), Vec::new());
        let officer = Arc::make_mut(&mut ctx.entity_views)
            .get_mut(&2)
            .expect("officer view");
        officer.is_able_to_fight = false;
        officer.is_unconscious = true;
        officer.active = true;

        assert!(ai.is_detecting(2, &ctx));

        Arc::make_mut(&mut ctx.entity_views)
            .get_mut(&2)
            .expect("officer view")
            .active = false;
        assert!(!ai.is_detecting(2, &ctx));
    }

    #[test]
    fn normal_detection_same_building_uses_exact_body_and_door_gates() {
        let ai = charly_heading_to_officer();
        let mut ctx = charly_to_officer_context(test_position(200.0, 0.0), Vec::new());
        let building = SectorHandle::new(7);
        ctx.building_sector = building;
        let officer = Arc::make_mut(&mut ctx.entity_views)
            .get_mut(&2)
            .expect("officer view");
        officer.building_sector = building;
        officer.in_building = true;
        officer.active = false;
        officer.is_able_to_fight = false;
        assert!(ai.is_detecting(2, &ctx));

        for gate in 0..3 {
            {
                let officer = Arc::make_mut(&mut ctx.entity_views)
                    .get_mut(&2)
                    .expect("officer view");
                officer.is_dead = gate == 0;
                officer.is_unconscious = gate == 1;
                officer.passing_door = gate == 2;
            }
            assert!(!ai.is_detecting(2, &ctx), "same-building gate {gate}");
            let officer = Arc::make_mut(&mut ctx.entity_views)
                .get_mut(&2)
                .expect("officer view");
            officer.is_dead = false;
            officer.is_unconscious = false;
            officer.passing_door = false;
        }
    }

    #[test]
    fn normal_detection_does_not_treat_viewer_door_transit_as_building_sector() {
        let ai = charly_heading_to_officer();
        let mut ctx = charly_to_officer_context(test_position(200.0, 0.0), Vec::new());
        ctx.in_building = true;
        ctx.building_sector = None;

        assert!(ai.is_detecting(2, &ctx));
    }

    #[test]
    fn normal_detection_projects_radius_on_target_obstacle_top_plane() {
        use crate::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA;

        let ai = charly_heading_to_officer();
        let target = test_position(380.0, 0.0);
        let clear_ctx = charly_to_officer_context(target, Vec::new());
        assert!(ai.is_detecting(2, &clear_ctx));

        let mut platform = SightObstacle::new(0, SIGHTOBSTACLE_PROJECTION_AREA);
        platform.obstacle_points = vec![
            ObstaclePoint {
                x: 350.0,
                y: -20.0,
                z_top: 200.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 410.0,
                y: -20.0,
                z_top: 200.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 410.0,
                y: 20.0,
                z_top: 200.0,
                z_bottom: 0.0,
            },
            ObstaclePoint {
                x: 350.0,
                y: 20.0,
                z_top: 200.0,
                z_bottom: 0.0,
            },
        ];
        platform.top_plane_points = [
            [350.0, -20.0, 200.0],
            [410.0, -20.0, 200.0],
            [350.0, 20.0, 200.0],
        ];
        platform.bottom_plane_points =
            [[350.0, -20.0, 0.0], [410.0, -20.0, 0.0], [350.0, 20.0, 0.0]];
        platform.rebuild_geometry();
        let mut platform_ctx = charly_to_officer_context(target, vec![platform]);
        let officer = Arc::make_mut(&mut platform_ctx.entity_views)
            .get_mut(&2)
            .expect("officer view");
        officer.elevation = 200.0;
        officer.obstacle_idx = crate::position_interface::ObstacleHandle::new(0);

        assert!(!ai.is_detecting(2, &platform_ctx));
    }

    #[test]
    fn charly_outside_view_cone_retries_after_ten_frames() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = charly_heading_to_officer();
        // Within the 360-degree radius and unobstructed, but behind Charly.
        let ctx = charly_to_officer_context(test_position(-200.0, 0.0), Vec::new());

        ai.think_expected_event(
            sim,
            &Stimulus::new(StimulusType::EventTimer),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(ai.base.current_substate, Substate::SeekingCharlyGoToOfficer);
        assert!(ai.base.outbox.reentrant.cross_npc_actions.is_empty());
        assert_eq!(
            ai.base.outbox.actor.unalert_near_charly_seekers,
            Some(CharlySeekerTarget::SelfNpc)
        );
        assert_eq!(ai.base.when_does_timer_ring, 110);
        assert_eq!(
            ai.base.substate_at_last_timer_launch,
            Substate::SeekingCharlyGoToOfficer
        );
    }

    #[test]
    fn charly_cannot_report_through_opaque_obstruction_and_retries() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = charly_heading_to_officer();
        let ctx =
            charly_to_officer_context(test_position(200.0, 0.0), vec![opaque_wall_across_x_axis()]);

        ai.think_expected_event(
            sim,
            &Stimulus::new(StimulusType::EventTimer),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(ai.base.current_substate, Substate::SeekingCharlyGoToOfficer);
        assert!(ai.base.outbox.reentrant.cross_npc_actions.is_empty());
        assert_eq!(ai.base.when_does_timer_ring, 110);
        assert_eq!(
            ai.base.substate_at_last_timer_launch,
            Substate::SeekingCharlyGoToOfficer
        );
    }

    fn run_find_door_authorization_case(
        door_type: DoorType,
        active: bool,
        locked_npc_villain: bool,
        building_full: bool,
        actor_is_rider: bool,
    ) -> EnemyAi {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let center = Position {
            x: 0.0,
            y: 0.0,
            sector: SectorHandle::new(7),
            level: 0,
        };
        let position_in = Position {
            x: 50.0,
            y: 60.0,
            sector: SectorHandle::new(8),
            level: 2,
        };
        let point_out = MapPoint::new(10.0, 0.0);

        let door = Door {
            door_type,
            active,
            locked_npc_villain,
            ..Default::default()
        };

        let mut global = AiGlobalState::default();
        global.door_seek_infos.push(DoorSeekInfo {
            door_index: DoorIndex::new(0).expect("valid door index"),
            door_type,
            point_out,
            position_in,
            sector_out: 7,
            sector_out_index: None,
            sector_in: 8,
            layer_out: 0,
            npc_villain_authorized_direct: cache_npc_villain_authorized_direct(&door),
        });
        let occupant_ids = if building_full {
            vec![EntityId::Soldier(SoldierId(0)); usize::from(u16::MAX)]
        } else {
            Vec::new()
        };
        global.houses.push(House {
            sector_index: 8,
            occupant_ids,
            ..House::default()
        });

        let mut ai = EnemyAi::new(1);
        let ctx = AiContext {
            frame: 100,
            camp: Camp::Lacklandists,
            in_building: true,
            building_sector: SectorHandle::new(9),
            self_is_rider: actor_is_rider,
            ..AiContext::default()
        };
        let seek_direction =
            crate::position_interface::vector_to_sector_0_to_15_iso(point_out.x, point_out.y)
                as u16;

        ai.seek_area(
            sim,
            center,
            0,
            SeekFlags::HOUSE | SeekFlags::LOCATION_FIRST,
            seek_direction,
            &mut global,
            &ctx,
            &AiPerTickData::stub(),
        );

        // The indoor caller must enter the three-frame watching delay after
        // selecting the personal seek point, regardless of authorization.
        // This pins the exact state/timer ordering around the door decision.
        assert_eq!(ai.my_seek_points, vec![1111]);
        assert_eq!(ai.base.current_state, AiState::Seeking);
        assert_eq!(
            ai.base.current_substate,
            Substate::SeekingSeekpointWatchingSidewards
        );
        assert!(ai.base.timer_is_running);
        assert_eq!(ai.base.when_does_timer_ring, 103);
        assert_eq!(
            ai.base.substate_at_last_timer_launch,
            Substate::SeekingSeekpointWatchingSidewards
        );

        ai
    }

    #[test]
    fn find_door_enemy_could_be_behind_applies_every_original_authorization_gate() {
        let center = Position {
            x: 0.0,
            y: 0.0,
            sector: SectorHandle::new(7),
            level: 0,
        };
        let behind_door = Position {
            x: 50.0,
            y: 60.0,
            sector: SectorHandle::new(8),
            level: 2,
        };

        let cases = [
            (
                "authorized",
                DoorType::Building,
                true,
                false,
                false,
                false,
                behind_door,
            ),
            (
                "building type",
                DoorType::Default,
                true,
                false,
                false,
                false,
                center,
            ),
            (
                "active state",
                DoorType::Building,
                false,
                false,
                false,
                false,
                center,
            ),
            (
                "building capacity",
                DoorType::Building,
                true,
                false,
                true,
                false,
                center,
            ),
            (
                "rider",
                DoorType::Building,
                true,
                false,
                false,
                true,
                center,
            ),
            (
                "villain lock",
                DoorType::Building,
                true,
                true,
                false,
                false,
                center,
            ),
        ];

        for (name, door_type, active, locked, full, rider, expected) in cases {
            let ai = run_find_door_authorization_case(door_type, active, locked, full, rider);
            assert_eq!(ai.seek_center, expected, "{name} gate");
            assert_eq!(
                ai.personal_seek_point_1
                    .as_ref()
                    .map(|point| point.position),
                Some(expected),
                "{name} gate must be applied before the personal point is created"
            );
        }
    }

    #[test]
    fn enemy_ai_defaults() {
        let ai = EnemyAi::new(42);
        assert_eq!(ai.base.me, 42);
        assert_eq!(ai.current_task_priority, task_priority::NONE);
        assert_eq!(ai.base.current_state, AiState::Default);
        assert!(!ai.tower_guard);
        assert!(!ai.combat_trainer);
    }

    #[test]
    fn repeated_directed_panic_preserves_existing_red_alert_until_engine_boundary() {
        let mut ai = EnemyAi::new(53);
        ai.base.current_state = AiState::Fleeing;
        ai.base.current_substate = Substate::FleeingPanic;
        ai.set_alert_status(crate::ai::AlertLevel::Red);

        let center = test_position(667.0, 824.0);
        let incoming_runs = crate::parameters_ai::AI_STANDARD_PANIC_RUNS as u8;
        let existing_runs = incoming_runs.saturating_add(3);
        ai.base.lasting_panic_runs = existing_runs;
        ai.panic_from_position(center, incoming_runs);

        assert_eq!(ai.base.current_state, AiState::Fleeing);
        assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
        assert_eq!(ai.base.lasting_panic_runs, existing_runs);
        assert_eq!(ai.base.view_alert_status, crate::ai::AlertLevel::Red);
        assert_eq!(
            ai.base.current_music_alert_status,
            crate::ai::AlertLevel::Red
        );
        assert!(
            ai.base.outbox.reentrant.owner_work.is_empty(),
            "Original skips SetState when Panic begins from FleeingPanic"
        );
        let request = ai
            .base
            .outbox
            .actor
            .begin_panic
            .expect("repeated panic still reaches the engine door/search boundary");
        assert_eq!(request.center, Some(center));
        assert_eq!(request.runs, incoming_runs);
        assert!(!request.is_new_panic);
    }

    #[test]
    fn reinitialize_them_list_does_not_preserve_unseen_primary_target() {
        let mut ai = EnemyAi::new(1);
        ai.base.primary_target = 2;
        ai.list_them = vec![2, 3];

        ai.reinitialize_them_list(&AiContext::default(), &AiPerTickData::stub());

        assert!(ai.list_them.is_empty());
        assert_eq!(ai.base.primary_target, 2);
    }

    #[test]
    fn set_state() {
        let mut ai = EnemyAi::new(1);
        ai.set_state(AiState::Attacking, Substate::AttackingSwordfight);
        assert_eq!(ai.base.current_state, AiState::Attacking);
        assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
        // The transition queues an inline FilterAIEvent notification
        // for the post-think dispatcher to drain (matching the reference
        // `RHArtificialMalignity::SetState` at L9226).
        let [AiOwnerWork::StateChange(notification)] =
            ai.base.outbox.reentrant.owner_work.as_slice()
        else {
            panic!("expected one SetState notification");
        };
        assert_eq!(notification.outgoing_state, AiState::Default);
        assert_eq!(notification.outgoing_substate, Substate::DefaultOnPost);
        assert_eq!(notification.incoming_state, AiState::Attacking);
        assert_eq!(
            notification.incoming_substate,
            Substate::AttackingSwordfight
        );
        assert_eq!(notification.source, AiStateChangeSource::Null);
        assert!(notification.actor_effects_before_callback.is_none());
    }

    #[test]
    fn patrol_coordinate_uses_enemy_virtual_state_before_walk_and_run() {
        for (distance, expected_substate, expected_order) in [
            (
                45.0,
                Substate::DefaultPatrolEnroute,
                crate::order::OrderType::WalkingUpright,
            ),
            (
                60.0,
                Substate::DefaultPatrolEnrouteRunning,
                crate::order::OrderType::RunningUpright,
            ),
        ] {
            let mut ai = EnemyAi::new(1);
            ai.base.patrol_chief = Some(crate::element::EntityId::Soldier(
                crate::entity_id::SoldierId(2),
            ));
            ai.base.current_state = AiState::Default;
            ai.base.current_substate = Substate::DefaultOnPost;
            ai.attentive = true;
            ai.will_be_attentive = true;
            ai.base.current_music_alert_status = AlertLevel::Yellow;
            ai.base.view_alert_status = AlertLevel::Yellow;

            let ctx = AiContext {
                position: Position {
                    x: 100.0,
                    y: 100.0,
                    sector: SectorHandle::new(1),
                    level: 0,
                },
                self_animation: crate::order::OrderType::WaitingAlerted,
                ..AiContext::default()
            };
            let target = Position {
                x: ctx.position.x + distance,
                ..ctx.position
            };

            ai.coordinate_patrol(
                &StimulusInfo::Position(target),
                &ctx,
                Position {
                    x: ctx.position.x + 100.0,
                    ..ctx.position
                },
            );

            assert_eq!(ai.base.current_state, AiState::Default);
            assert_eq!(ai.base.current_substate, expected_substate);
            assert_eq!(ai.base.current_music_alert_status, AlertLevel::Green);
            assert_eq!(ai.base.view_alert_status, AlertLevel::Green);

            let [AiOwnerWork::StateChange(notification)] =
                ai.base.outbox.reentrant.owner_work.as_slice()
            else {
                panic!("patrol SetState must retain the StopAll prefix");
            };
            let prefix = notification
                .actor_effects_before_callback
                .as_ref()
                .expect("StopAll must precede the virtual SetState callback");
            assert!(prefix.halt);

            let attentive = ai
                .base
                .outbox
                .actor
                .set_attentive_mode
                .expect("Default SetState must request leaving attentive mode");
            assert!(!attentive.target);
            let [order] = ai.base.outbox.actor.orders.as_slice() else {
                panic!("patrol coordinate must queue one replacement movement");
            };
            assert_eq!(order.order_type, expected_order);
            assert!(
                order.after_attentive_mode,
                "movement must remain behind the LeaveAttentiveMode element"
            );
        }

        let mut already_unalerted = EnemyAi::new(1);
        already_unalerted.base.patrol_chief = Some(crate::element::EntityId::Soldier(
            crate::entity_id::SoldierId(2),
        ));
        already_unalerted.base.current_state = AiState::Default;
        already_unalerted.base.current_substate = Substate::DefaultPatrolEnroute;
        let ctx = AiContext {
            position: Position {
                x: 100.0,
                y: 100.0,
                sector: SectorHandle::new(1),
                ..Position::default()
            },
            ..AiContext::default()
        };
        already_unalerted.coordinate_patrol(
            &StimulusInfo::Position(Position {
                x: ctx.position.x + 45.0,
                ..ctx.position
            }),
            &ctx,
            Position {
                x: ctx.position.x + 100.0,
                ..ctx.position
            },
        );
        let [order] = already_unalerted.base.outbox.actor.orders.as_slice() else {
            panic!("already-unalerted patrol update must retain its movement");
        };
        assert!(
            !order.after_attentive_mode,
            "a no-change SetAttentiveMode call must not defer movement instruction"
        );
    }

    #[test]
    fn set_state_rejects_mismatched_numeric_substate_family() {
        // Original guards this only under `_DEBUG`; parity/release builds do
        // not execute that assert. Verify the family predicate that feeds our
        // matching `debug_assert_eq!` without requiring release to invent a
        // runtime rejection absent from Original.
        assert_eq!(
            Substate::SleepingForever.ai_state_family(),
            Some(AiState::Sleeping)
        );
        assert_ne!(
            Substate::SleepingForever.ai_state_family(),
            Some(AiState::Default)
        );
    }

    #[test]
    fn archery_release_is_outbox_work_and_special_strike_remains_a_latch() {
        let mut ai = EnemyAi::new(1);
        ai.my_shooting_point = Some((2, 3));
        ai.my_archery_sector = Some(2);
        ai.pending_special_strike = true;

        ai.set_state(AiState::Default, Substate::DefaultOnPost);

        assert_eq!(ai.my_shooting_point, None);
        assert_eq!(ai.my_archery_sector, Some(2));
        assert_eq!(
            ai.base.outbox.actor.archery_reservation_release,
            ArcheryReservationRelease {
                shooting_point: Some(ReservedShootingPoint {
                    sector_index: 2,
                    point_index: crate::sector::ArcheryPointIdx(3),
                }),
                release_sector: true,
            }
        );

        assert!(ai.pending_special_strike);
    }

    #[test]
    fn special_strike_latch_tracks_preparation_and_in_flight_lifecycle() {
        let mut ai = EnemyAi::new(1);
        ai.set_state(AiState::Attacking, Substate::AttackingSwordfight);
        ai.base.outbox.reentrant.owner_work.clear();

        ai.begin_special_strike();
        assert!(ai.pending_special_strike);
        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingSwordfightSpecialStrike
        );

        ai.reconcile_special_strike(true, 40);
        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingSwordfightSpecialStrike
        );

        for locks in [
            crate::ai::AiLockFlags::BUSY,
            crate::ai::AiLockFlags::FREEZE,
            crate::ai::AiLockFlags::BUSY | crate::ai::AiLockFlags::FREEZE,
        ] {
            let mut locked = EnemyAi::new(1);
            locked.set_state(AiState::Attacking, Substate::AttackingSwordfight);
            locked.base.outbox.reentrant.owner_work.clear();
            locked.begin_special_strike();
            locked.base.locks_flag_field = locks;
            locked.reconcile_special_strike(false, 41);
            assert!(locked.pending_special_strike, "locks={locks:?}");
            assert_eq!(
                locked.base.current_substate,
                Substate::AttackingSwordfightSpecialStrike,
                "all non-script AI locks must retain the EventDone-driven substate edge: {locks:?}"
            );
        }

        ai.base.non_script_lock(crate::ai::AiLockFlags::FREEZE);
        let sim = crate::sim_rng::test_context();
        let mut global = AiGlobalState::default();
        let ctx = AiContext {
            frame: 41,
            ..AiContext::default()
        };
        let tick = AiPerTickData::stub();
        ai.think(
            &sim,
            &Stimulus::new(StimulusType::EventDone),
            &mut global,
            &ctx,
            &tick,
            None,
        );
        ai.reconcile_special_strike(false, 41);
        assert!(ai.pending_special_strike);
        assert_eq!(
            ai.base
                .stimulus_queue
                .iter()
                .map(|stimulus| stimulus.stimulus_type)
                .collect::<Vec<_>>(),
            vec![StimulusType::EventDone],
            "StartThink must retain the terminal event while Strangle holds FREEZE"
        );
        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingSwordfightSpecialStrike,
            "AILOCK_FREEZE must retain the EventDone-driven substate edge"
        );
        ai.base.non_script_unlock(crate::ai::AiLockFlags::FREEZE);
        let retained = ai.base.stimulus_queue.remove(0);
        let ctx = AiContext {
            frame: 42,
            ..AiContext::default()
        };
        ai.think(&sim, &retained, &mut global, &ctx, &tick, None);
        assert!(!ai.pending_special_strike);
        assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
        assert_eq!(ai.next_sword_strike_frame, 62);

        ai.begin_special_strike();
        ai.set_state(AiState::Attacking, Substate::AttackingSwordfightParade);
        ai.reconcile_special_strike(false, 62);
        assert!(!ai.pending_special_strike);
        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingSwordfightParade,
            "cancellation cleanup must preserve a newer combat reaction"
        );
    }

    #[test]
    fn guarded_pc_relationship_uses_typed_optional_ids_and_delta() {
        let mut ai = EnemyAi::new(1);
        let guarded = PcId(17);

        ai.set_guarded_pc(Some(guarded));
        assert_eq!(ai.guarded_pc, Some(guarded));
        assert_eq!(
            ai.base.outbox.actor.set_guarded_pc,
            Some(GuardedPcEffect {
                old: None,
                new: Some(guarded),
            })
        );

        ai.set_guarded_pc(None);
        assert_eq!(ai.guarded_pc, None);
        assert_eq!(
            ai.base.outbox.actor.set_guarded_pc,
            Some(GuardedPcEffect {
                old: Some(guarded),
                new: None,
            })
        );

        let encoded = serde_json::to_string(&ai).expect("serialize typed guard relationship");
        let decoded: EnemyAi =
            serde_json::from_str(&encoded).expect("deserialize typed guard relationship");
        assert_eq!(decoded.guarded_pc, None);
        assert_eq!(
            decoded.base.outbox.actor.set_guarded_pc,
            Some(GuardedPcEffect {
                old: Some(guarded),
                new: None,
            })
        );
    }

    #[test]
    fn return_to_duty_resets() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = EnemyAi::new(1);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingSwordfight;
        ai.current_task_priority = task_priority::ENEMY;
        let ctx = AiContext::default();
        let tick = AiPerTickData::stub();
        ai.return_to_duty(sim, DutyFlags::empty(), &ctx, &tick);

        assert_eq!(ai.base.current_state, AiState::Attacking);
        assert!(!ai.base.needs_patrol_reinit);
        assert!(matches!(
            ai.base.outbox.reentrant.owner_work.as_slice(),
            [AiOwnerWork::ResumeReturnToDutyAfterPatrolInit { .. }]
        ));

        ai.resume_return_to_duty_after_patrol_init(sim, DutyFlags::empty(), &ctx, false);
        assert_eq!(ai.base.current_state, AiState::Default);
        assert_eq!(ai.current_task_priority, task_priority::NONE);
    }

    #[test]
    fn return_to_duty_deletes_beggars_added_earlier_in_same_dispatch() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        let target = EntityId::Pc(PcId(171));
        ai.base
            .outbox
            .actor
            .add_detectables
            .push((target, DetectableType::Beggar));
        ai.base
            .outbox
            .actor
            .add_detectables
            .push((target, DetectableType::Enemy));

        ai.return_to_duty(
            &sim,
            DutyFlags::empty(),
            &AiContext::default(),
            &AiPerTickData::stub(),
        );

        assert_eq!(
            ai.base.outbox.actor.add_detectables,
            vec![(target, DetectableType::Enemy)]
        );
        assert!(
            ai.base
                .outbox
                .actor
                .delete_detectables
                .contains(&DetectableType::Beggar)
        );
    }

    #[test]
    fn return_to_duty_virtual_timer_reset_precedes_common_bored_timer_launch() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.base.timer_is_running = true;
        ai.base.when_does_timer_ring = 999;
        ai.base.likes_to_sit_around = true;
        let ctx = AiContext {
            frame: 254,
            posture: Posture::Sitting,
            position: ai.base.initial_position,
            ..AiContext::default()
        };

        ai.resume_return_to_duty_after_patrol_init(&sim, DutyFlags::empty(), &ctx, false);

        assert_eq!(ai.base.current_substate, Substate::DefaultOnPost);
        assert!(
            ai.base.timer_is_running,
            "the common tail's later LaunchTimer must win over virtual SetState's reset"
        );
        assert!((324..394).contains(&ai.base.when_does_timer_ring));
    }

    #[test]
    fn return_to_duty_virtual_timer_reset_stays_cleared_without_common_launch() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.base.timer_is_running = true;
        ai.base.when_does_timer_ring = 999;

        ai.resume_return_to_duty_after_patrol_init(
            &sim,
            DutyFlags::empty(),
            &AiContext::default(),
            false,
        );

        assert_eq!(ai.base.current_substate, Substate::DefaultGotoPost);
        assert!(
            !ai.base.timer_is_running,
            "virtual SetState must still clear the old timer when the common tail launches none"
        );
    }

    #[test]
    fn high_recursion_return_to_duty_keeps_close_point_as_latch_after_deferred_resume() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(90);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingReactiontime;
        ai.base.think_recursion_depth = 100;
        let ctx = AiContext {
            position: ai.base.initial_position,
            self_animation: OrderType::WaitingAlerted,
            ..AiContext::default()
        };

        ai.return_to_duty(&sim, DutyFlags::empty(), &ctx, &AiPerTickData::stub());
        let (flags, high_recursion_failsafe) =
            std::mem::take(&mut ai.base.outbox.reentrant.owner_work)
                .into_iter()
                .find_map(|work| match work {
                    AiOwnerWork::ResumeHighRecursionReturnToDutyAfterPatrolInit {
                        flags, ..
                    } => Some((flags, true)),
                    _ => None,
                })
                .expect("high-recursion ReturnToDuty must queue its common tail");
        assert!(high_recursion_failsafe);

        // Rust releases the AI borrow for InitializePatrol and may not resume
        // this owner work until the deferred recursion stack has unwound.
        ai.base.think_recursion_depth = 0;
        ai.base.open_end_think_frames = 0;
        ai.resume_return_to_duty_after_patrol_init(&sim, flags, &ctx, high_recursion_failsafe);

        assert_eq!(ai.base.current_substate, Substate::DefaultGotoPost);
        assert!(ai.base.outbox.reentrant.self_stimuli.is_empty());
        assert!(ai.base.already_on_point);
        assert!(!ai.base.completion_latch_inside_think);
        assert_eq!(ai.base.think_recursion_depth, 0);
    }

    #[test]
    fn return_to_duty_marks_only_its_new_orders_behind_real_attentive_transition() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.attentive = true;
        ai.will_be_attentive = true;
        let post = Position {
            x: 200.0,
            y: 100.0,
            sector: SectorHandle::new(1),
            level: 0,
        };
        let here = Position { x: 100.0, ..post };
        ai.base.initial_position = post;
        ai.base
            .outbox
            .actor
            .orders
            .push(crate::order::AiOrderIntent::new(
                OrderType::Turning,
                10.0,
                0.0,
            ));
        let ctx = AiContext {
            position: here,
            self_animation: OrderType::WaitingAlerted,
            ..AiContext::default()
        };

        ai.resume_return_to_duty_after_patrol_init(&sim, DutyFlags::empty(), &ctx, false);

        let [preexisting, return_to_post] = ai.base.outbox.actor.orders.as_slice() else {
            panic!("expected the pre-existing control and one return-to-duty move")
        };
        assert!(!preexisting.after_attentive_mode);
        assert_eq!(return_to_post.order_type, OrderType::WalkingUpright);
        assert!(
            return_to_post.after_attentive_mode,
            "Original virtual SetState launches LeaveAttentiveMode before its following GoTo"
        );

        let mut already_unalerted = EnemyAi::new(1);
        already_unalerted.base.initial_position = post;
        already_unalerted.resume_return_to_duty_after_patrol_init(
            &sim,
            DutyFlags::empty(),
            &AiContext {
                position: here,
                ..AiContext::default()
            },
            false,
        );
        let [order] = already_unalerted.base.outbox.actor.orders.as_slice() else {
            panic!("already-unalerted return must still author its move")
        };
        assert!(
            !order.after_attentive_mode,
            "a no-change SetAttentiveMode call launches no transition to wait behind"
        );
    }

    #[test]
    fn return_to_duty_detects_patrol_chief_at_raw_body_but_approaches_door_endpoint() {
        let sim = crate::sim_rng::test_context();
        let chief_id = crate::element::EntityId::Soldier(crate::entity_id::SoldierId(2));
        let gate_endpoint = test_position(1_000.0, 0.0);
        let mut chief = soldier_view(gate_endpoint);
        chief.passing_door = true;
        chief.detection_position_world = crate::coordinates::WorldPoint3D::new(100.0, 0.0, 0.0);

        let mut views = AiEntityViewMap::new();
        views.insert(chief_id.index(), chief);
        let ctx = AiContext {
            position: test_position(0.0, 0.0),
            self_body_position_world: crate::coordinates::WorldPoint3D::ZERO,
            self_upright_eye_world: crate::coordinates::WorldPoint3D::new(0.0, 0.0, 45.0),
            self_view_radius: 200,
            sq_self_view_radius: 200.0 * 200.0,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut ai = EnemyAi::new(1);
        ai.base.patrol_chief = Some(chief_id);

        ai.base
            .return_to_duty_common_stuff(&sim, DutyFlags::empty(), &ctx);

        assert_eq!(ai.base.current_substate, Substate::DefaultGotoChief);
        let [order] = ai.base.outbox.actor.orders.as_slice() else {
            panic!("detecting the chief must queue the approach");
        };
        assert_eq!((order.target_x, order.target_y), (1_000.0, 0.0));
    }

    #[test]
    fn return_to_duty_does_not_detect_far_raw_chief_at_near_door_endpoint() {
        let sim = crate::sim_rng::test_context();
        let chief_id = crate::element::EntityId::Soldier(crate::entity_id::SoldierId(2));
        let mut chief = soldier_view(test_position(100.0, 0.0));
        chief.passing_door = true;
        chief.detection_position_world = crate::coordinates::WorldPoint3D::new(1_000.0, 0.0, 0.0);

        let mut views = AiEntityViewMap::new();
        views.insert(chief_id.index(), chief);
        let ctx = AiContext {
            position: test_position(0.0, 0.0),
            self_body_position_world: crate::coordinates::WorldPoint3D::ZERO,
            self_upright_eye_world: crate::coordinates::WorldPoint3D::new(0.0, 0.0, 45.0),
            self_view_radius: 200,
            sq_self_view_radius: 200.0 * 200.0,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut ai = EnemyAi::new(1);
        ai.base.patrol_chief = Some(chief_id);

        ai.base
            .return_to_duty_common_stuff(&sim, DutyFlags::empty(), &ctx);

        assert_ne!(ai.base.current_substate, Substate::DefaultGotoChief);
        assert!(
            ai.base
                .outbox
                .actor
                .orders
                .iter()
                .all(|order| (order.target_x, order.target_y) != (100.0, 0.0)),
            "the near AI Position endpoint must not admit the raw-far chief"
        );
    }

    #[test]
    fn return_to_duty_remembered_ale_saves_patrol_return_point() {
        let sim = crate::sim_rng::test_context();
        let here = Position {
            x: 125.0,
            y: 250.0,
            sector: SectorHandle::new(4),
            level: 2,
        };
        let ale_position = Position {
            x: 400.0,
            y: 500.0,
            sector: SectorHandle::new(7),
            level: 0,
        };
        let ale = 77;
        let mut views = AiEntityViewMap::new();
        views.insert(ale, soldier_view(ale_position));
        let ctx = AiContext {
            position: here,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut ai = EnemyAi::new(1);
        ai.base.current_state = AiState::Wondering;
        ai.base.current_substate = Substate::WonderingDrinkingAle;
        ai.other_seen_ale.push(ale);

        ai.return_to_duty(&sim, DutyFlags::empty(), &ctx, &AiPerTickData::stub());

        assert_eq!(ai.base.current_state, AiState::Wondering);
        assert_eq!(ai.base.current_substate, Substate::WonderingApproachingAle);
        assert_eq!(ai.base.interesting_object, ale);
        assert_eq!(ai.return_to_patrol_point, here);
        assert_eq!(ai.base.last_goto_destination, ale_position);
    }

    #[test]
    fn one_point_enemy_path_dispatches_virtual_return_before_patrol_init_resume() {
        use crate::ai::{PathId, PatrolPath};
        use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

        let paths = vec![RawHikingPath {
            waypoints: vec![RawWaypoint {
                x: 699,
                y: 1464,
                sector: 50,
                level: 1,
                command: WaypointCommand::None,
            }],
        }];
        let mut ai = EnemyAi::new(142);
        ai.base.current_state = AiState::Default;
        ai.base.current_substate = Substate::DefaultGotoRouteTurn;
        ai.base.has_patrol_path = true;
        ai.base.patrol_path = PatrolPath::new(PathId::new(0).unwrap(), &paths);
        let ctx = AiContext {
            position: Position {
                x: 698.99304,
                y: 1464.0072,
                sector: SectorHandle::new(50),
                level: 1,
            },
            direction: 3,
            self_is_soldier: true,
            posture: Posture::Upright,
            self_action_state: crate::element::ActionState::Waiting,
            self_animation: OrderType::NonanimationEnd,
            hiking_paths: Arc::new(paths),
            ..AiContext::default()
        };
        let sim = crate::sim_rng::test_context();

        ai.base.think_expected_event_common_stuff(
            &sim,
            &Stimulus::new(StimulusType::EventDone),
            &ctx,
        );

        let virtual_requests = std::mem::take(&mut ai.base.outbox.reentrant.owner_work);
        assert!(matches!(
            virtual_requests.as_slice(),
            [AiOwnerWork::VirtualReturnToDuty {
                flags,
                owner_boundary_positions
            }] if flags.is_empty() && owner_boundary_positions.is_empty()
        ));
        assert!(
            !ai.base
                .outbox
                .reentrant
                .owner_work
                .iter()
                .any(|work| matches!(work, AiOwnerWork::ResumeReturnToDutyAfterPatrolInit { .. })),
            "the common controller must not skip the Enemy ReturnToDuty override"
        );

        ai.return_to_duty(&sim, DutyFlags::empty(), &ctx, &AiPerTickData::stub());

        assert!(matches!(
            ai.base.outbox.reentrant.owner_work.as_slice(),
            [AiOwnerWork::ResumeReturnToDutyAfterPatrolInit {
                flags,
                owner_boundary_positions,
                ..
            }] if flags.is_empty()
                && owner_boundary_positions.is_empty()
        ));
    }

    #[test]
    fn return_to_duty_virtual_state_tail_clears_guarded_pc() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        let guarded = PcId(17);
        ai.base.current_state = AiState::Menacing;
        ai.base.current_substate = Substate::MenacingPcInComa;
        ai.guarded_pc = Some(guarded);

        ai.resume_return_to_duty_after_patrol_init(
            &sim,
            DutyFlags::empty(),
            &AiContext::default(),
            false,
        );

        assert_eq!(ai.base.current_state, AiState::Default);
        assert_eq!(ai.guarded_pc, None);
        assert_eq!(
            ai.base.outbox.actor.set_guarded_pc,
            Some(GuardedPcEffect {
                old: Some(guarded),
                new: None,
            }),
            "the owner-boundary drain must clear the PC's reciprocal guard before later NPCs scan"
        );
    }

    #[test]
    fn return_to_duty_virtual_state_tail_clears_combat_neighbours() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(223);
        ai.base.current_state = AiState::Wondering;
        ai.base.current_substate = Substate::WonderingWatching;
        ai.left_combat_neighbour = 225;
        ai.right_combat_neighbour = 227;

        ai.resume_return_to_duty_after_patrol_init(
            &sim,
            DutyFlags::empty(),
            &AiContext::default(),
            false,
        );

        assert_eq!(ai.base.current_state, AiState::Default);
        assert_eq!(ai.left_combat_neighbour, 0);
        assert_eq!(ai.right_combat_neighbour, 0);
        assert!(
            matches!(
                ai.base.outbox.reentrant.cross_npc_actions.as_slice(),
                [
                    CrossNpcAction::SetRightCombatNeighbour {
                        target: 225,
                        neighbour: 0,
                    },
                    CrossNpcAction::SetLeftCombatNeighbour {
                        target: 227,
                        neighbour: 0,
                    },
                ]
            ),
            "virtual Enemy SetState must publish both reciprocal unlinks at the owner boundary"
        );
    }

    #[test]
    fn return_to_duty_clears_shield_pair_before_bearer_protection_timer() {
        let sim = crate::sim_rng::test_context();
        let mut archer = EnemyAi::new(64);
        archer.is_archer_unit = true;
        archer.base.current_state = AiState::Attacking;
        archer.base.current_substate = Substate::AttackingBowShooting;
        archer.shield_bearer_before_me = 58;

        archer.resume_return_to_duty_after_patrol_init(
            &sim,
            DutyFlags::empty(),
            &AiContext::default(),
            false,
        );

        assert_eq!(archer.base.current_state, AiState::Default);
        assert_eq!(archer.shield_bearer_before_me, 0);
        let reciprocal = std::mem::take(&mut archer.base.outbox.reentrant.cross_npc_actions);
        assert!(matches!(
            reciprocal.as_slice(),
            [CrossNpcAction::SetArcherBehindMe {
                target: 58,
                archer: 0
            }]
        ));

        // Drain the reciprocal write before the later shield-bearer owner. Its
        // next protection timer must now take Original's danger-over path into
        // GetBattleOverview, rather than the stale archer-behind-me arm.
        let mut bearer = EnemyAi::new(58);
        bearer.base.current_state = AiState::Attacking;
        bearer.base.current_substate = Substate::AttackingProtectingWithShield;
        bearer.base.primary_target = 101;
        bearer.archer_behind_me = 64;
        for action in reciprocal {
            if let CrossNpcAction::SetArcherBehindMe { target, archer } = action {
                assert_eq!(target, bearer.base.me);
                bearer.archer_behind_me = archer;
            }
        }
        assert_eq!(bearer.archer_behind_me, 0);

        let mut tick = AiPerTickData::stub();
        tick.fighter_registry.push(FighterSnapshot {
            handle: 58,
            action_state: crate::element::ActionState::HoldingShield,
            ..FighterSnapshot::default()
        });
        tick.fighter_registry.push(FighterSnapshot {
            handle: 101,
            action_state: crate::element::ActionState::Waiting,
            ..FighterSnapshot::default()
        });
        bearer.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventTimer),
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &tick,
            None,
        );

        assert_eq!(
            bearer.base.current_substate,
            Substate::AttackingOverviewLookLeft
        );
        assert_eq!(
            bearer.base.outbox.actor.look_sidewards,
            Some(crate::ai::LookDirection::Left)
        );
    }

    #[test]
    fn return_to_duty_clears_archer_pair_when_bearer_leaves_protection() {
        let sim = crate::sim_rng::test_context();
        let mut bearer = EnemyAi::new(58);
        bearer.base.current_state = AiState::Attacking;
        bearer.base.current_substate = Substate::AttackingProtectingWithShield;
        bearer.archer_behind_me = 64;

        bearer.resume_return_to_duty_after_patrol_init(
            &sim,
            DutyFlags::empty(),
            &AiContext::default(),
            false,
        );

        assert_eq!(bearer.base.current_state, AiState::Default);
        assert_eq!(bearer.archer_behind_me, 0);
        assert!(matches!(
            bearer.base.outbox.reentrant.cross_npc_actions.as_slice(),
            [CrossNpcAction::SetShieldBearerBeforeMe {
                target: 64,
                shield_bearer: 0,
            }]
        ));
    }

    #[test]
    fn seek_flags() {
        let flags = SeekFlags::BODY_SEEK | SeekFlags::LOOK_FOR_HELP_AFTER;
        assert!(flags.contains(SeekFlags::BODY_SEEK));
        assert!(flags.contains(SeekFlags::LOOK_FOR_HELP_AFTER));
        assert!(!flags.contains(SeekFlags::HOUSE));
    }

    #[test]
    fn able_to_help_matches_original_state_gates() {
        assert!(soldier_is_able_to_help_state(
            true,
            AiState::Default,
            Substate::None
        ));
        assert!(soldier_is_able_to_help_state(
            true,
            AiState::Wondering,
            Substate::WonderingMoneyReactiontime
        ));
        assert!(soldier_is_able_to_help_state(
            true,
            AiState::Seeking,
            Substate::SeekingRunningToOfficer
        ));
        assert!(!soldier_is_able_to_help_state(
            true,
            AiState::Seeking,
            Substate::SeekingSeekpoint
        ));
        assert!(!soldier_is_able_to_help_state(
            true,
            AiState::Attacking,
            Substate::AttackingSwordfight
        ));
        assert!(!soldier_is_able_to_help_state(
            false,
            AiState::Default,
            Substate::None
        ));
    }

    #[test]
    fn tower_guard_defers_battle_decisions_until_alert_calls_return() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.tower_guard = true;
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingTowerGuardAlert;
        ai.base.seek_position = test_position(120.0, 80.0);

        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventDone),
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
        );

        assert!(matches!(
            ai.base.outbox.reentrant.cross_npc_actions.as_slice(),
            [CrossNpcAction::ResumeTowerGuardBattleDecisions { caller: 1 }]
        ));
        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingTowerGuardAlert,
            "BattleDecisions must not run before the synchronous recipient batch"
        );
    }

    #[test]
    fn officer_detection_uses_officer_facing() {
        let officer = CampSoldierInfo {
            handle: 2,
            active: true,
            position: Position {
                x: 0.0,
                y: 0.0,
                sector: None,
                level: 0,
            },
            position_world: crate::coordinates::WorldPoint3D::ZERO,
            direction: 4,
            rank: ProfileRank::Officer,
            ai_state: AiState::Default,
            ai_substate: Substate::None,
            is_able_to_fight: true,
            is_dead: false,
            knocked_out_in_money_fight: false,
            primary_target: 0,
            pride: 0,
            is_able_to_help: true,
            script_locked: false,
            ai_lock_frozen: false,
            layer: 0,
            report_type: ReportType::Nothing,
            report_seek_position: Position::default(),
            report_seen_bodies: Vec::new(),
            report_charly: 0,
            alert_soldiers_point: Position::default(),
            patrol_chief: None,
            antagonist: 0,
            detected_body: 0,
            blood_alcohol: 0,
            duty_flag: false,
            is_tower_guard: false,
            company_number: 0,
            in_building: false,
            forecast_destination: Some(crate::ai::PreparedForecastDestination::fixed(
                Position::default(),
                0,
            )),
            detectable_bodies: Vec::new(),
            seek_position: Position::default(),
            current_task_priority: 0,
            minimal_task_priority: 0,
            view_direction: [1.0, 0.0],
            view_radius: 400,
            real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
            eye_blind: false,
        };
        let ahead = Position {
            x: 100.0,
            y: 0.0,
            sector: None,
            level: 0,
        };
        let behind = Position {
            x: -100.0,
            y: 0.0,
            sector: None,
            level: 0,
        };

        assert!(soldier_detects_position_180(&officer, ahead, 350.0 * 350.0));
        assert!(!soldier_detects_position_180(
            &officer,
            behind,
            350.0 * 350.0
        ));
    }

    #[test]
    fn task_priority_ordering() {
        const { assert!(task_priority::ENEMY > task_priority::BODY) };
        const { assert!(task_priority::BODY > task_priority::SEEKING) };
        const { assert!(task_priority::ALERT_IGNORE_ENEMY > task_priority::ENEMY) };
    }

    #[test]
    fn start_think_allows_normal_events() {
        let mut ai = EnemyAi::new(1);
        let ctx = AiContext::default();
        let stimulus = Stimulus::new(StimulusType::EventTimer);
        assert!(ai.start_think(&stimulus, &ctx, false));
        assert_eq!(ai.base.think_recursion_depth, 1);
    }

    #[test]
    fn enter_swordfight_event_does_not_reenter_swordfight() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = EnemyAi::new(1);
        let mut global = AiGlobalState::default();
        let ctx = AiContext::default();
        let tick = AiPerTickData::stub();
        let stimulus = Stimulus::with_human(StimulusType::EventEnterSwordfight, 2);

        let _ = ai.think(sim, &stimulus, &mut global, &ctx, &tick, None);

        assert_eq!(ai.base.primary_target, 2);
        assert_eq!(ai.base.current_state, AiState::Attacking);
        assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
        assert_eq!(ai.base.outbox.actor.enter_swordfight, None);
    }

    #[test]
    fn start_think_blocks_when_script_locked() {
        let mut ai = EnemyAi::new(1);
        ai.base.script_locked = true;
        ai.base.remember_events = true;
        let ctx = AiContext::default();
        let stimulus = Stimulus::new(StimulusType::EventView);
        assert!(!ai.start_think(&stimulus, &ctx, false));
        assert_eq!(ai.base.stimulus_queue.len(), 1);
    }

    #[test]
    fn start_think_retains_ailock_freeze() {
        let mut ai = EnemyAi::new(1);
        ai.base.locks_flag_field = AiLockFlags::FREEZE;
        let ctx = AiContext::default();
        let stimulus = Stimulus::new(StimulusType::EventTimer);
        assert!(!ai.start_think(&stimulus, &ctx, false));
        assert_eq!(ai.base.stimulus_queue.len(), 1);
        assert_eq!(
            ai.base.stimulus_queue[0].stimulus_type,
            StimulusType::EventTimer
        );
    }

    #[test]
    fn start_think_discards_static_ai_freeze() {
        let mut ai = EnemyAi::new(1);
        let ctx = AiContext::default();
        let stimulus = Stimulus::new(StimulusType::EventTimer);
        assert!(!ai.start_think(&stimulus, &ctx, true));
        assert!(ai.base.stimulus_queue.is_empty());
    }

    #[test]
    fn start_think_rejects_look_there_for_physically_unconscious_script_driven_actor() {
        let mut ai = EnemyAi::new(1);
        ai.base.current_state = AiState::Default;
        ai.base.current_substate = Substate::DefaultScriptDriven;
        let ctx = AiContext {
            self_is_unconscious: true,
            ..AiContext::default()
        };

        assert!(!ai.start_think(&Stimulus::new(StimulusType::CallLookThere), &ctx, false,));
        assert_eq!(ai.base.current_state, AiState::Default);
        assert_eq!(ai.base.current_substate, Substate::DefaultScriptDriven);
        assert!(ai.base.outbox.reentrant.cross_npc_actions.is_empty());
        assert!(
            ai.base
                .ai_log
                .last()
                .is_some_and(|line| line.line_type == LogLineType::EventRefused && line.info == 8)
        );
    }

    #[test]
    fn periodic_timer_restart_obeys_static_ai_freeze() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = EnemyAi::new(1);
        ai.base.current_substate = Substate::AttackingObserve;
        let ctx = AiContext::default();
        let tick = AiPerTickData::stub();
        let mut global = AiGlobalState {
            freeze: true,
            ..AiGlobalState::default()
        };

        ai.the_16th_frame(
            sim, 0, &ctx, &global, &tick, None, false, false, false, false,
        );
        assert!(!ai.base.timer_is_running);

        global.freeze = false;
        ai.the_16th_frame(
            sim, 0, &ctx, &global, &tick, None, false, false, false, false,
        );
        assert!(ai.base.timer_is_running);
    }

    #[test]
    fn periodic_bored_roll_reads_live_animation_not_action_change_history() {
        let sim = crate::sim_rng::test_context();
        let seed_before = sim.seed();
        let mut ai = EnemyAi::new(1);
        let mut stale_view = soldier_view(Position::default());
        stale_view.current_animation = OrderType::Invalid;
        let mut views = AiEntityViewMap::new();
        views.insert(1, stale_view);
        let ctx = AiContext {
            self_animation: OrderType::WaitingUprightBored,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        ai.the_16th_frame(
            &sim,
            16,
            &ctx,
            &AiGlobalState::default(),
            &AiPerTickData::stub(),
            None,
            true,
            false,
            true,
            false,
        );

        assert_ne!(
            sim.seed(),
            seed_before,
            "GetAnimation() must consume the bored-roll draw from the live sprite action"
        );
    }

    #[test]
    fn periodic_smalltalk_command_advances_reachpoint_stuck_counter() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.base.current_substate = Substate::AttackingMovingAroundOldEnemy;
        ai.base.stuck_counter = 2;
        let ctx = AiContext::default();
        let global = AiGlobalState::default();
        let tick = AiPerTickData::stub();

        ai.the_16th_frame(
            &sim, 0, &ctx, &global, &tick, None, false, false, true, false,
        );
        assert_eq!(
            ai.base.stuck_counter, 3,
            "Original monitors smalltalk strike/parry commands while waiting for EVENT_REACHPOINT"
        );

        ai.the_16th_frame(
            &sim, 0, &ctx, &global, &tick, None, false, false, false, false,
        );
        assert_eq!(
            ai.base.stuck_counter, 3,
            "an unrelated command in the same movement substate leaves the counter untouched"
        );
    }

    #[test]
    fn periodic_post_refresh_queued_goto_suppresses_stuck_counter() {
        let mut ai = EnemyAi::new(1);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingRunningToPhalanx;
        ai.base.stuck_counter = 2;

        // RefreshArrowProtection has already launched a direct GoTo. At this
        // exact Original statement boundary the movement element is on the
        // manager's to-go list, so SequenceElementIsAboutToBeLaunched(owner,
        // RHCOMMAND_NULL) is true and the watchdog resets rather than
        // counting the actor's still-selected Wait command.
        ai.the_16th_frame_after_refresh(0, &AiContext::default(), true, true);

        assert_eq!(ai.base.stuck_counter, 0);
    }

    #[test]
    fn periodic_post_refresh_without_queued_element_keeps_idle_increment() {
        let mut ai = EnemyAi::new(1);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingRunningToPhalanx;
        ai.base.stuck_counter = 2;

        // An already-on-point GoTo returns before LaunchSequence, while a
        // denied route deletes its candidate sequence. Neither case leaves a
        // manager element to suppress the live Wait/smalltalk arm.
        ai.the_16th_frame_after_refresh(0, &AiContext::default(), true, false);

        assert_eq!(ai.base.stuck_counter, 3);
    }

    #[test]
    fn periodic_phalanx_goto_does_not_hide_same_call_idle_actor() {
        // schema14 seed1000000, linux2/P002/Savegame_032/replay-008,
        // frame 17254. RefreshArrowProtection changes the soldier from
        // Reactiontime/Wait to RunningToPhalanx and launches a GoTo, but the
        // same-call stuck check still observes the actor's current Wait and
        // advances its counter. Rust must not substitute its deferred order
        // for that live actor/sequence-manager observation.
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingReactiontime;
        ai.base.stuck_counter = 2;
        ai.list_them.push(2);

        let exact_position = |x, y| Position {
            sector: SectorHandle::new(0),
            ..test_position(x, y)
        };
        let owner_position = exact_position(500.0, 500.0);
        let enemy_position = exact_position(1_500.0, 500.0);
        let mut owner_view = soldier_view(owner_position);
        owner_view.camp = Camp::Royalists;
        let mut enemy_view = soldier_view(enemy_position);
        enemy_view.camp = Camp::Lacklandists;
        enemy_view.action_state = crate::element::ActionState::AimingWithBow;
        let mut views = AiEntityViewMap::new();
        views.insert(1, owner_view);
        views.insert(2, enemy_view);
        let ctx = AiContext {
            position: owner_position,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        let mut tick = AiPerTickData::stub();
        tick.seen_last_frame_enemies.push(2);
        tick.fighter_registry.push(FighterSnapshot {
            handle: 1,
            position: owner_position,
            raw_position: owner_position,
            is_friendly: true,
            is_soldier: true,
            is_shield_bearer: true,
            ..FighterSnapshot::default()
        });
        tick.fighter_registry.push(FighterSnapshot {
            handle: 2,
            position: enemy_position,
            raw_position: enemy_position,
            is_able_to_fight: true,
            ..FighterSnapshot::default()
        });
        tick.fighter_registry.push(FighterSnapshot {
            handle: 3,
            position: exact_position(600.0, 500.0),
            raw_position: exact_position(600.0, 500.0),
            direction: 0,
            is_friendly: true,
            is_soldier: true,
            is_shield_bearer: true,
            current_substate: Substate::AttackingPhalanx as u32,
            ..FighterSnapshot::default()
        });

        ai.the_16th_frame(
            &sim,
            0,
            &ctx,
            &AiGlobalState::default(),
            &tick,
            None,
            true,
            false,
            true,
            false,
        );

        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingRunningToPhalanx
        );
        assert_ne!(ai.base.last_goto_destination, Position::default());
        assert_eq!(
            ai.base.stuck_counter, 3,
            "the deferred phalanx GoTo does not hide the current idle actor"
        );
    }

    #[test]
    fn periodic_phalanx_goto_does_not_fake_wait_during_attentive_transition() {
        // Seed3 linux2/P002/Savegame_030/replay-007 frame 6887. The shield
        // refresh queues a phalanx GoTo while EnterAttentive remains the
        // selected command. Original's subsequent GetCommand switch does not
        // enter the Wait/smalltalk stuck-counter arm.
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingReactiontime;
        ai.base.stuck_counter = 0;
        ai.list_them.push(2);

        let exact_position = |x, y| Position {
            sector: SectorHandle::new(0),
            ..test_position(x, y)
        };
        let owner_position = exact_position(500.0, 500.0);
        let enemy_position = exact_position(1_500.0, 500.0);
        let mut owner_view = soldier_view(owner_position);
        owner_view.camp = Camp::Royalists;
        let mut enemy_view = soldier_view(enemy_position);
        enemy_view.camp = Camp::Lacklandists;
        enemy_view.action_state = crate::element::ActionState::AimingWithBow;
        let mut views = AiEntityViewMap::new();
        views.insert(1, owner_view);
        views.insert(2, enemy_view);
        let ctx = AiContext {
            position: owner_position,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        let mut tick = AiPerTickData::stub();
        tick.seen_last_frame_enemies.push(2);
        tick.fighter_registry.push(FighterSnapshot {
            handle: 1,
            position: owner_position,
            raw_position: owner_position,
            is_friendly: true,
            is_soldier: true,
            is_shield_bearer: true,
            ..FighterSnapshot::default()
        });
        tick.fighter_registry.push(FighterSnapshot {
            handle: 2,
            position: enemy_position,
            raw_position: enemy_position,
            is_able_to_fight: true,
            ..FighterSnapshot::default()
        });
        tick.fighter_registry.push(FighterSnapshot {
            handle: 3,
            position: exact_position(600.0, 500.0),
            raw_position: exact_position(600.0, 500.0),
            direction: 0,
            is_friendly: true,
            is_soldier: true,
            is_shield_bearer: true,
            current_substate: Substate::AttackingPhalanx as u32,
            ..FighterSnapshot::default()
        });

        ai.the_16th_frame(
            &sim,
            0,
            &ctx,
            &AiGlobalState::default(),
            &tick,
            None,
            false,
            false,
            false,
            false,
        );

        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingRunningToPhalanx
        );
        assert!(!ai.base.outbox.actor.orders.is_empty());
        assert_eq!(
            ai.base.stuck_counter, 0,
            "a shield action must not substitute for Original's selected-command switch"
        );
    }

    #[test]
    fn periodic_phalanx_already_on_point_does_not_fake_a_pending_sequence() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingReactiontime;
        ai.base.stuck_counter = 2;
        ai.list_them.push(2);

        // Direction zero puts the open left slot 25 pixels to the left of
        // the existing shield bearer. The owner is already exactly there,
        // so RefreshArrowProtection's GoTo completes without registering a
        // movement sequence.
        let exact_position = |x, y| Position {
            sector: SectorHandle::new(0),
            ..test_position(x, y)
        };
        let owner_position = exact_position(575.0, 500.0);
        let enemy_position = exact_position(1_500.0, 500.0);
        let mut owner_view = soldier_view(owner_position);
        owner_view.camp = Camp::Royalists;
        owner_view.current_animation = OrderType::WaitingUpright;
        let mut enemy_view = soldier_view(enemy_position);
        enemy_view.camp = Camp::Lacklandists;
        enemy_view.action_state = crate::element::ActionState::AimingWithBow;
        let mut views = AiEntityViewMap::new();
        views.insert(1, owner_view);
        views.insert(2, enemy_view);
        let ctx = AiContext {
            position: owner_position,
            self_animation: OrderType::WaitingUpright,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        let mut tick = AiPerTickData::stub();
        tick.seen_last_frame_enemies.push(2);
        tick.fighter_registry.push(FighterSnapshot {
            handle: 1,
            position: owner_position,
            raw_position: owner_position,
            is_friendly: true,
            is_soldier: true,
            is_shield_bearer: true,
            ..FighterSnapshot::default()
        });
        tick.fighter_registry.push(FighterSnapshot {
            handle: 2,
            position: enemy_position,
            raw_position: enemy_position,
            is_able_to_fight: true,
            ..FighterSnapshot::default()
        });
        tick.fighter_registry.push(FighterSnapshot {
            handle: 3,
            position: exact_position(600.0, 500.0),
            raw_position: exact_position(600.0, 500.0),
            direction: 0,
            is_friendly: true,
            is_soldier: true,
            is_shield_bearer: true,
            current_substate: Substate::AttackingPhalanx as u32,
            ..FighterSnapshot::default()
        });

        ai.the_16th_frame(
            &sim,
            0,
            &ctx,
            &AiGlobalState::default(),
            &tick,
            None,
            true,
            false,
            true,
            false,
        );

        assert_eq!(
            ai.base.current_substate,
            Substate::AttackingRunningToPhalanx
        );
        assert!(ai.base.outbox.actor.orders.is_empty());
        assert!(
            ai.base
                .outbox
                .reentrant
                .self_stimuli
                .iter()
                .any(|stimulus| stimulus.stimulus_type == StimulusType::EventReachPoint)
        );
        assert_eq!(
            ai.base.stuck_counter, 3,
            "an already-on-point GoTo leaves no pending sequence to suppress Original's counter"
        );
    }

    #[test]
    fn start_think_handles_lose_consciousness() {
        let mut ai = EnemyAi::new(1);
        let ctx = AiContext::default();
        let stimulus = Stimulus::new(StimulusType::EventLoseConsciousness);
        assert!(!ai.start_think(&stimulus, &ctx, false));
        assert_eq!(ai.base.current_state, AiState::Sleeping);
        assert_eq!(ai.base.current_substate, Substate::SleepingUnconscious);
    }

    #[test]
    fn forget_attentive_events_preserve_the_forced_script_latch() {
        for stimulus_type in [
            StimulusType::EventLoseConsciousness,
            StimulusType::EventWasp,
            StimulusType::EventNet,
        ] {
            for forced_attentive in [false, true] {
                let sim = crate::sim_rng::test_context();
                let mut ai = EnemyAi::new(1);
                ai.attentive = true;
                ai.will_be_attentive = true;
                ai.forced_attentive = forced_attentive;
                let mut global = AiGlobalState::default();
                let ctx = AiContext::default();
                let tick = AiPerTickData::stub();

                ai.think(
                    &sim,
                    &Stimulus::new(stimulus_type),
                    &mut global,
                    &ctx,
                    &tick,
                    None,
                );

                assert!(!ai.attentive, "{stimulus_type:?}");
                assert!(!ai.will_be_attentive, "{stimulus_type:?}");
                assert_eq!(
                    ai.forced_attentive, forced_attentive,
                    "{stimulus_type:?} must not clear the script-owned latch"
                );
                assert!(
                    ai.base
                        .outbox
                        .actor
                        .set_attentive_mode
                        .is_some_and(|request| request.forget_after),
                    "{stimulus_type:?} must retain SetState's attentive transition before forgetting its flags"
                );

                if stimulus_type == StimulusType::EventLoseConsciousness {
                    ai.think(
                        &sim,
                        &Stimulus::new(StimulusType::EventFitAgain),
                        &mut global,
                        &ctx,
                        &tick,
                        None,
                    );
                    assert_eq!(ai.base.current_substate, Substate::SleepingAwakening);
                    assert_eq!(ai.forced_attentive, forced_attentive);
                }
            }
        }
    }

    #[test]
    fn start_think_blocks_dead() {
        let mut ai = EnemyAi::new(1);
        ai.base.current_state = AiState::Sleeping;
        ai.base.current_substate = Substate::SleepingForever;
        let ctx = AiContext {
            self_is_dead: true,
            ..AiContext::default()
        };
        let stimulus = Stimulus::new(StimulusType::EventLoseConsciousness);
        assert!(!ai.start_think(&stimulus, &ctx, false));
        assert_eq!(ai.base.current_state, AiState::Sleeping);
        assert_eq!(ai.base.current_substate, Substate::SleepingForever);
        assert_eq!(ai.base.outbox.recovery.set_eye_status, None);
    }

    #[test]
    fn start_think_blocks_fitagain_when_carried() {
        let mut ai = EnemyAi::new(1);
        ai.base.current_substate = Substate::SleepingUnconscious;
        let ctx = AiContext {
            posture: crate::element::Posture::Carried,
            ..AiContext::default()
        };
        let stimulus = Stimulus::new(StimulusType::EventFitAgain);
        assert!(!ai.start_think(&stimulus, &ctx, false));
    }

    #[test]
    fn update_task_priority_maps_correctly() {
        let mut ai = EnemyAi::new(1);
        let s = Stimulus::new(StimulusType::EventView);
        ai.update_new_task_priority(&s);
        assert_eq!(ai.new_task_priority, task_priority::ENEMY);

        let s = Stimulus::new(StimulusType::EventSeesBody);
        ai.update_new_task_priority(&s);
        assert_eq!(ai.new_task_priority, task_priority::BODY);
    }

    #[test]
    fn watching_for_more_money_skips_looted_victims_and_marks_next() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = EnemyAi::new(1);
        ai.base.me = 1;
        ai.set_state(AiState::Wondering, Substate::WonderingWatchingForMoreMoney);

        let me = test_position(0.0, 0.0);
        let mut looted = soldier_view(test_position(10.0, 0.0));
        looted.is_able_to_fight = false;
        looted.is_unconscious = true;
        looted.looted_after_money_fight = true;
        let mut unlooted = soldier_view(test_position(20.0, 0.0));
        unlooted.is_able_to_fight = false;
        unlooted.is_unconscious = true;

        let mut views = AiEntityViewMap::new();
        views.insert(1, soldier_view(me));
        views.insert(2, looted);
        views.insert(3, unlooted);
        let ctx = AiContext {
            position: me,
            sq_standard_view_radius: 500.0 * 500.0,
            sq_self_view_radius: 500.0 * 500.0,
            move_box: crate::coordinates::MoveBox::from_coords(-5.0, -5.0, 5.0, 5.0),
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.camp_unconscious_soldiers = vec![2, 3]
            .into_iter()
            .map(|handle| CampUnconsciousSoldierInfo {
                handle,
                knocked_out_in_money_fight: true,
            })
            .collect();
        let mut global = AiGlobalState::default();

        let stimulus = Stimulus::new(StimulusType::EventDone);
        let _ = ai.think(sim, &stimulus, &mut global, &ctx, &tick, None);

        assert_eq!(ai.base.detected_body, 3);
        assert_eq!(
            ai.base.current_substate,
            Substate::WonderingApproachingToLoot
        );
        assert!(matches!(
            ai.base.outbox.reentrant.cross_npc_actions.as_slice(),
            [CrossNpcAction::SetLootedAfterMoneyFight {
                target: 3,
                looted: true
            }]
        ));
    }

    #[test]
    fn run_to_examine_body_uses_stuck_under_net_cover_info() {
        let mut ai = EnemyAi::new(1);
        ai.base.me = 1;
        let me = test_position(0.0, 0.0);
        let body = test_position(40.0, 0.0);
        let net = test_position(42.0, 0.0);

        let mut victim = soldier_view(body);
        victim.is_able_to_fight = false;
        victim.is_unconscious = true;
        victim.stuck_under_net = true;
        victim.covering_nets.push(NetCoverInfo {
            handle: 77,
            position: net,
            radius: 40.0,
        });

        let mut views = AiEntityViewMap::new();
        views.insert(1, soldier_view(me));
        views.insert(2, victim);
        let ctx = AiContext {
            position: me,
            sq_standard_view_radius: 500.0 * 500.0,
            sq_self_view_radius: 500.0 * 500.0,
            move_box: crate::coordinates::MoveBox::from_coords(-5.0, -5.0, 5.0, 5.0),
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        ai.run_to_examine_body(2, &ctx, &AiPerTickData::stub(), None);

        assert_eq!(ai.base.detected_body, 2);
        assert_eq!(ai.base.interesting_object, 77);
        assert_eq!(ai.base.current_state, AiState::Seeking);
        assert_eq!(ai.base.current_substate, Substate::SeekingNet);
    }

    #[test]
    #[should_panic(expected = "soldier 1 cannot examine missing body 77")]
    fn run_to_examine_body_rejects_a_missing_required_body() {
        let mut ai = EnemyAi::new(1);
        ai.run_to_examine_body(77, &AiContext::default(), &AiPerTickData::stub(), None);
    }

    #[test]
    fn make_battle_predecisions_returns_valid() {
        crate::sim_rng::with_seed(1, |sim| {
            let mut ai = EnemyAi::new(1);
            ai.list_them.push(99);
            ai.base.list_us.push(1);
            let mut views = AiEntityViewMap::new();
            views.insert(1, soldier_view(Position::default()));
            let ctx = AiContext {
                entity_views: crate::ai_entity_view::shared_entity_views(views),
                ..AiContext::default()
            };
            let tick = AiPerTickData::stub();
            let d = ai.make_battle_predecisions(sim, &ctx, &tick);
            assert!(d == Decision::PredecisionOffensive || d == Decision::PredecisionDefensive);
        });
    }

    #[test]
    fn answer_question_task_priority() {
        let ctx = AiContext::default();
        let mut ai = EnemyAi::new(1);
        // Equal priorities → HasTheNewTaskPriority is true.
        assert!(ai.answer_question(Question::HasTheNewTaskPriority, &ctx));
        // Lower new priority while Seeking → false.
        ai.base.current_state = AiState::Seeking;
        ai.current_task_priority = 50;
        ai.new_task_priority = 10;
        assert!(!ai.answer_question(Question::HasTheNewTaskPriority, &ctx));
        // Lower new priority in Default state with NONE minimal → true.
        ai.base.current_state = AiState::Default;
        ai.minimal_task_priority = task_priority::NONE;
        assert!(ai.answer_question(Question::HasTheNewTaskPriority, &ctx));
    }

    #[test]
    fn send_out_soldier_uses_live_patrol_not_theoretical_patrol() {
        let ctx = AiContext {
            self_is_active: true,
            in_building: false,
            ..AiContext::default()
        };
        let mut ai = EnemyAi::new(1);
        ai.soldier_profile_initiative = 60;

        // RHArtificialMalignity::AnswerQuestion checks mlistPatrol here.
        // A save may retain a theoretical patrol after its live patrol has
        // emptied; that must not make the officer delegate body examination.
        ai.base
            .theoretical_patrol
            .push(EntityId::Soldier(crate::entity_id::SoldierId(2)));
        assert!(!ai.answer_question(Question::ShallISendOutSoldier, &ctx));

        ai.base
            .patrol
            .push(EntityId::Soldier(crate::entity_id::SoldierId(2)));
        assert!(ai.answer_question(Question::ShallISendOutSoldier, &ctx));
    }

    #[test]
    fn hard_reaction_time_fix_selects_the_intended_multiplier() {
        let ctx = AiContext {
            difficulty: crate::player_profile::DifficultyLevel::Hard,
            camp: crate::element::Camp::Lacklandists,
            frame: 10,
            ..AiContext::default()
        };

        let mut original = EnemyAi::new(1);
        original.soldier_profile_iq = 50;
        original.react(100, &ctx, &AiPerTickData::stub());
        assert_eq!(original.base.when_does_timer_ring, 111);

        let mut fixed_tick = AiPerTickData::stub();
        fixed_tick.fix_hard_reaction_times = true;
        let mut fixed = EnemyAi::new(1);
        fixed.soldier_profile_iq = 50;
        fixed.react(100, &ctx, &fixed_tick);
        assert_eq!(fixed.base.when_does_timer_ring, 36);
    }

    #[test]
    fn get_new_primary_target_empty() {
        let mut ai = EnemyAi::new(1);
        let ctx = AiContext::default();
        let tick = AiPerTickData::stub();
        assert_eq!(
            ai.get_new_primary_target(PrimaryTargetFlags::empty(), &ctx, &tick),
            0
        );
    }

    #[test]
    fn leaving_phalanx_clears_reciprocal_combat_neighbour_links() {
        let mut ai = EnemyAi::new(74);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingPhalanx;
        ai.left_combat_neighbour = 75;
        ai.right_combat_neighbour = 72;

        ai.set_state(AiState::Attacking, Substate::AttackingOverviewLookLeft);

        assert_eq!(ai.left_combat_neighbour, 0);
        assert_eq!(ai.right_combat_neighbour, 0);
        assert!(matches!(
            ai.base.outbox.reentrant.cross_npc_actions.as_slice(),
            [
                CrossNpcAction::SetRightCombatNeighbour {
                    target: 75,
                    neighbour: 0
                },
                CrossNpcAction::SetLeftCombatNeighbour {
                    target: 72,
                    neighbour: 0
                }
            ]
        ));
    }

    #[test]
    fn entering_phalanx_preserves_preassigned_combat_neighbour_links() {
        let mut ai = EnemyAi::new(73);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingOverviewLookLeft;
        ai.left_combat_neighbour = 72;

        ai.set_state(AiState::Attacking, Substate::AttackingRunningToPhalanx);

        assert_eq!(ai.left_combat_neighbour, 72);
        assert_eq!(ai.right_combat_neighbour, 0);
        assert!(ai.base.outbox.reentrant.cross_npc_actions.is_empty());
    }

    #[test]
    fn running_to_phalanx_preserves_existing_neighbours_null_primary_target() {
        let mut ai = EnemyAi::new(78);
        ai.right_combat_neighbour = 70;
        ai.list_them = vec![170];

        let mut tick = AiPerTickData::stub();
        tick.fighter_registry.push(FighterSnapshot {
            handle: 70,
            is_soldier: true,
            primary_target: 0,
            ..FighterSnapshot::default()
        });
        tick.fighter_registry.push(FighterSnapshot {
            handle: 170,
            is_pc: true,
            is_able_to_fight: true,
            ..FighterSnapshot::default()
        });

        assert_eq!(ai.phalanx_neighbour_primary_target(&tick), Some(0));
    }

    #[test]
    fn running_to_phalanx_uses_right_soldier_after_non_soldier_left_neighbour() {
        let mut ai = EnemyAi::new(78);
        ai.left_combat_neighbour = 169;
        ai.right_combat_neighbour = 70;

        let mut tick = AiPerTickData::stub();
        tick.fighter_registry.push(FighterSnapshot {
            handle: 169,
            is_pc: true,
            ..FighterSnapshot::default()
        });
        tick.fighter_registry.push(FighterSnapshot {
            handle: 70,
            is_soldier: true,
            primary_target: 170,
            ..FighterSnapshot::default()
        });

        assert_eq!(ai.phalanx_neighbour_primary_target(&tick), Some(170));
    }

    #[test]
    fn get_new_primary_target_uses_live_positions_when_timer_snapshot_is_incomplete() {
        let mut ai = EnemyAi::new(1);
        ai.list_them = vec![198, 199];
        let mut views = AiEntityViewMap::new();
        let mut owner = soldier_view(test_position(0.0, 0.0));
        owner.camp = Camp::Lacklandists;
        views.insert(1, owner);
        views.insert(198, soldier_view(test_position(100.0, 0.0)));
        views.insert(199, soldier_view(test_position(110.0, 0.0)));
        let ctx = AiContext {
            position: test_position(0.0, 0.0),
            camp: Camp::Lacklandists,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        // Off-detection timer contexts historically cached only the old
        // primary target. Original still scores every persistent list entry
        // from its live position.
        tick.enemy_sq_distances = vec![(198, 10_000)];
        tick.primary_target_multiplicity = vec![(198, 1)];

        let target = ai.get_new_primary_target(
            PrimaryTargetFlags::UNOCCUPIED_PREFERRED | PrimaryTargetFlags::VIPS_ALLOWED,
            &ctx,
            &tick,
        );

        assert_eq!(target, 199);
    }

    #[test]
    fn get_new_primary_target_scores_raw_world_position_not_ai_door_endpoint() {
        let mut ai = EnemyAi::new(1);
        ai.list_them = vec![170, 169];

        let mut owner = soldier_view(test_position(0.0, 0.0));
        owner.camp = Camp::Lacklandists;
        let occupied = soldier_view(test_position(60.0, 0.0));
        let mut passing_door = soldier_view(test_position(177.0, 0.0));
        passing_door.detection_position = MapPoint::new(159.0, 0.0);
        passing_door.detection_position_world =
            crate::coordinates::WorldPoint3D::new(159.0, 0.0, 0.0);
        passing_door.passing_door = true;

        let mut views = AiEntityViewMap::new();
        views.insert(1, owner);
        views.insert(170, occupied);
        views.insert(169, passing_door);
        let ctx = AiContext {
            position: test_position(0.0, 0.0),
            camp: Camp::Lacklandists,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.primary_target_multiplicity = vec![(170, 1), (169, 0)];

        assert_eq!(
            ai.get_new_primary_target(
                PrimaryTargetFlags::UNOCCUPIED_PREFERRED | PrimaryTargetFlags::VIPS_ALLOWED,
                &ctx,
                &tick,
            ),
            169
        );
    }

    #[test]
    fn too_proud_range_gate_uses_isometric_world_distance() {
        // Task 94 representative geometry: Soldier72 versus PC107 at frame
        // 815. Raw map max-norm is 41.82 (inside the 50-unit sword range),
        // while Original's isometric MaxNormDistance is 83.64 (outside).
        let me_position = test_position(621.35455, 822.2824);
        let target_position = test_position(615.2868, 780.4628);

        let mut ai = EnemyAi::new(72);
        ai.soldier_profile_pride = 1;
        ai.base.current_substate = Substate::AttackingOfficerGivingOrdersWaiting;
        ai.list_them = vec![107];

        let mut target_view = soldier_view(target_position);
        target_view.is_pc = true;
        target_view.kind = EntityKind::Pc;
        let mut views = AiEntityViewMap::new();
        let mut owner_view = soldier_view(me_position);
        owner_view.camp = Camp::Lacklandists;
        views.insert(72, owner_view);
        views.insert(107, target_view);
        let ctx = AiContext {
            position: me_position,
            self_body_position_world: crate::coordinates::WorldPoint3D::new(
                me_position.x,
                me_position.y,
                0.0,
            ),
            elevation: 0.0,
            camp: Camp::Lacklandists,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        let mut tick = AiPerTickData::stub();
        tick.fighter_registry = vec![
            FighterSnapshot {
                handle: 72,
                position: me_position,
                raw_position: me_position,
                sword_range_maximal: 50,
                ..FighterSnapshot::default()
            },
            FighterSnapshot {
                handle: 107,
                position: target_position,
                raw_position: target_position,
                is_pc: true,
                ..FighterSnapshot::default()
            },
        ];

        assert!(ai.is_too_proud_to_attack(&ctx, &tick, None));
        assert_eq!(ai.base.primary_target, 107);
    }

    #[test]
    fn too_proud_range_gate_uses_raw_target_body_during_door_transit() {
        // Save067/r005 and Save068/r005: the target's AI Position() is the
        // committed point inside gate 108, but Original MaxNormDistance()
        // calls RHElement::GetPosition() directly.  The live body is within
        // this knight's sword reach, so pride must not suppress the attack.
        let me_position = test_position(2230.0, 405.0);
        let raw_target = test_position(2268.0, 393.0);
        let snapped_door_target = test_position(2301.0, 381.0);

        let mut ai = EnemyAi::new(266);
        ai.soldier_profile_pride = 80;
        ai.base.current_substate = Substate::AttackingReactiontime;
        ai.list_them = vec![320];

        let mut target_view = soldier_view(snapped_door_target);
        target_view.is_pc = true;
        target_view.kind = EntityKind::Pc;
        target_view.detection_position = MapPoint::new(raw_target.x, raw_target.y);
        target_view.detection_position_world =
            crate::coordinates::WorldPoint3D::new(raw_target.x, raw_target.y, 0.0);
        target_view.passing_door = true;
        let mut owner_view = soldier_view(me_position);
        owner_view.camp = Camp::Lacklandists;
        let mut views = AiEntityViewMap::new();
        views.insert(266, owner_view);
        views.insert(320, target_view);
        let ctx = AiContext {
            position: me_position,
            self_body_position_world: crate::coordinates::WorldPoint3D::new(
                me_position.x,
                me_position.y,
                0.0,
            ),
            camp: Camp::Lacklandists,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        let mut tick = AiPerTickData::stub();
        tick.fighter_registry = vec![
            FighterSnapshot {
                handle: 266,
                position: me_position,
                raw_position: me_position,
                sword_range_maximal: 50,
                ..FighterSnapshot::default()
            },
            FighterSnapshot {
                handle: 320,
                position: snapped_door_target,
                raw_position: raw_target,
                is_pc: true,
                ..FighterSnapshot::default()
            },
        ];

        assert!(!ai.is_too_proud_to_attack(&ctx, &tick, None));
        assert_eq!(ai.base.primary_target, 320);
    }

    #[test]
    fn too_proud_reselection_observes_live_battle_decision_multiplicity() {
        // Task 146 / Task 61 frame 471: BattleDecisions resets its personal
        // Them-list, adds the nearby friends' claims, and only then calls
        // IsTooProudToAttack. Its strongly-unoccupied re-pick must read those
        // within-call mutations, not the owner-start tick snapshot.
        let me_position = test_position(0.0, 0.0);
        let nearer_position = test_position(300.0, 0.0);
        let unoccupied_position = test_position(350.0, 0.0);

        let mut ai = EnemyAi::new(114);
        ai.soldier_profile_pride = 1;
        ai.base.current_substate = Substate::AttackingObserve;
        ai.list_them = vec![171, 174];

        let mut nearer_view = soldier_view(nearer_position);
        nearer_view.is_pc = true;
        nearer_view.kind = EntityKind::Pc;
        let mut unoccupied_view = soldier_view(unoccupied_position);
        unoccupied_view.is_pc = true;
        unoccupied_view.kind = EntityKind::Pc;
        let mut views = AiEntityViewMap::new();
        let mut owner_view = soldier_view(me_position);
        owner_view.camp = Camp::Lacklandists;
        views.insert(114, owner_view);
        views.insert(171, nearer_view);
        views.insert(174, unoccupied_view);
        let ctx = AiContext {
            position: me_position,
            camp: Camp::Lacklandists,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        let mut tick = AiPerTickData::stub();
        tick.primary_target_multiplicity = vec![(171, 0), (174, 1)];
        tick.fighter_registry = vec![
            FighterSnapshot {
                handle: 114,
                position: me_position,
                sword_range_maximal: 50,
                ..FighterSnapshot::default()
            },
            FighterSnapshot {
                handle: 171,
                position: nearer_position,
                is_pc: true,
                ..FighterSnapshot::default()
            },
            FighterSnapshot {
                handle: 174,
                position: unoccupied_position,
                is_pc: true,
                ..FighterSnapshot::default()
            },
        ];

        let live_decision_multiplicity = std::collections::BTreeMap::from([(171, 1), (174, 0)]);
        let _ = ai.is_too_proud_to_attack(&ctx, &tick, Some(&live_decision_multiplicity));

        assert_eq!(ai.base.primary_target, 174);
    }

    fn perpendicular_out_of_view_context(stare_y: f32) -> AiContext {
        AiContext {
            frame: 920,
            position: test_position(1_546.658_2, 318.299_56),
            // `enemy_is_behind_me` reads the raw `GetPositionGround()` body
            // point, which for this ground-level fixture coincides with the
            // AI position.
            self_body_position_world: crate::coordinates::WorldPoint3D {
                x: 1_546.658_2,
                y: 318.299_56,
                z: 0.0,
            },
            direction: 14,
            self_stare_point: crate::coordinates::GroundPoint::new(1_500.696_3, stare_y),
            ..AiContext::default()
        }
    }

    #[test]
    fn direction_table_keeps_exact_perpendicular_stare_in_front() {
        let ai = EnemyAi::new(111);
        let ctx = perpendicular_out_of_view_context(344.662_23);

        assert!(
            !ai.enemy_is_behind_me(&ctx),
            "Original's literal direction-14 vector produces an exact-zero dot product"
        );
    }

    #[test]
    fn direction_table_still_rejects_stare_slightly_behind() {
        let ai = EnemyAi::new(111);
        let ctx = perpendicular_out_of_view_context(344.672_24);

        assert!(ai.enemy_is_behind_me(&ctx));
    }

    #[test]
    fn out_of_view_removes_non_primary_target_at_perpendicular_boundary() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(111);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingSwordfight;
        ai.base.primary_target = 84;
        ai.list_them = vec![84, 171];

        let mut primary = soldier_view(test_position(1_519.443_7, 309.084_5));
        primary.kind = EntityKind::Pc;
        primary.is_pc = true;
        let mut lost = soldier_view(test_position(1_226.175_4, 315.871_6));
        lost.kind = EntityKind::Pc;
        lost.is_pc = true;
        let mut views = AiEntityViewMap::new();
        views.insert(84, primary);
        views.insert(171, lost);
        let mut ctx = perpendicular_out_of_view_context(344.662_23);
        ctx.entity_views = crate::ai_entity_view::shared_entity_views(views);
        ctx.self_seen_enemy_handles = vec![84];

        let mut tick = AiPerTickData::stub();
        tick.enemy_detectable_forecasts.push((
            171,
            crate::ai::PreparedForecastDestination::fixed(test_position(1_226.175_4, 315.871_6), 4),
        ));

        ai.think_unexpected_event(
            &sim,
            &Stimulus::with_human(StimulusType::EventOutOfView, 171),
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );

        assert_eq!(ai.list_them, vec![84]);
        assert_eq!(ai.missed_pc, 171);
    }

    #[test]
    fn out_of_view_uses_exact_stimulus_target_forecast_after_detectable_removal() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(111);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingObserve;
        // A preceding queued Think selected another primary target before
        // this falling-edge OUTOFVIEW was delivered.
        ai.base.primary_target = 84;
        ai.list_them = vec![171];

        let forecast_position = test_position(1_226.175_4, 315.871_6);
        let mut lost = soldier_view(forecast_position);
        lost.kind = EntityKind::Pc;
        lost.is_pc = true;
        let mut views = AiEntityViewMap::new();
        views.insert(171, lost);
        let ctx = AiContext {
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        let mut tick = AiPerTickData::stub();
        tick.primary_target_snapshot_handle = 171;
        tick.primary_target_forecast = Some(crate::ai::PreparedForecastDestination::fixed(
            forecast_position,
            4,
        ));
        // The live detectable list has already dropped 171, so there is no
        // entry in `enemy_detectable_forecasts`.

        ai.think_unexpected_event(
            &sim,
            &Stimulus::with_human(StimulusType::EventOutOfView, 171),
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );

        assert_eq!(ai.missed_pc, 171);
        assert_eq!(ai.base.seek_position, forecast_position);
    }
}
