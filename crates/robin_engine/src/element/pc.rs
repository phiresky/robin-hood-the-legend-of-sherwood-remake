//! Player-character data and local behavior.
use super::*;

/// Ammo counters owned by a live PC entity.
///
/// An original-game player actor always has a player-status reference, and
/// `SetPersistentProperty` mutates that live status directly.  Campaign mode
/// also persists the same values in [`crate::campaign::PcDescription`], but a
/// script call must not depend on a campaign object being installed.
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct PcAmmoData {
    pub ales: u16,
    pub arrows: u16,
    pub apples: u16,
    pub rations: u16,
    pub stones: u16,
    pub wasp_nests: u16,
    pub nets: u16,
    pub plants: u16,
    pub purses: u16,
}

impl PcAmmoData {
    pub fn get(&self, action: Action) -> Option<u16> {
        match action {
            Action::Ale => Some(self.ales),
            Action::Apple => Some(self.apples),
            Action::Bow => Some(self.arrows),
            Action::Eat | Action::Guzzle => Some(self.rations),
            Action::Net => Some(self.nets),
            Action::Stone => Some(self.stones),
            Action::Heal => Some(self.plants),
            Action::Purse => Some(self.purses),
            Action::WaspNest => Some(self.wasp_nests),
            _ => None,
        }
    }

    pub fn set(&mut self, action: Action, quantity: u16) -> Option<()> {
        let counter = match action {
            Action::Ale => &mut self.ales,
            Action::Apple => &mut self.apples,
            Action::Bow => &mut self.arrows,
            Action::Eat | Action::Guzzle => &mut self.rations,
            Action::Net => &mut self.nets,
            Action::Stone => &mut self.stones,
            Action::Heal => &mut self.plants,
            Action::Purse => &mut self.purses,
            Action::WaspNest => &mut self.wasp_nests,
            _ => return None,
        };
        *counter = quantity;
        Some(())
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
    Default,
)]
pub struct PcPortraitQuickIconState {
    #[serde(deserialize_with = "Option::deserialize")]
    pub titbit_id: Option<crate::titbit::TitbitId>,
    pub running: bool,
}

/// Engine-owned copy of the Original portrait state needed by save adoption.
///
/// The renderer may project this into a host widget, but simulation adoption
/// must not lose it merely because no widget exists in a headless replay.
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct PcPortraitState {
    pub quantities: [u16; 3],
    pub two_buttons_mode: bool,
    pub displayed: bool,
    pub burned: bool,
    pub open: bool,
    pub life_level: f32,
    pub trumpet_enabled: bool,
    pub quick_icons: [PcPortraitQuickIconState; 3],
}

fn default_pc_camp() -> Camp {
    Camp::Royalists
}

const fn default_hero_command_interface() -> CommandInterface {
    CommandInterface::HeroActions
}

const fn default_hero_mission_role() -> MissionRole {
    MissionRole::PlayerParty
}

/// PC-level data.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct PcData {
    /// Life points stored directly.
    pub life_points: i16,
    pub immortal: bool,
    pub robin: bool,
    pub already_selected: bool,
    pub list_index: u8,
    /// Mission-authored allegiance. Ordinary campaign PCs use the player
    /// allegiance assigned during construction.
    #[serde(default = "default_pc_camp")]
    pub cached_camp: Camp,
    /// Exact index of the original game's description reference in
    /// Campaign player-character description map.
    ///
    /// This is independent of the list index: multiple campaign
    /// descriptions may share one character profile, while the list byte is
    /// actor/UI state serialized separately for the player actor.
    #[serde(default)]
    pub campaign_description_index: Option<u32>,

    /// Whether this PC can currently be selected and controlled.
    /// Set/cleared by the `Activate` and `Deactivate` script natives,
    /// and by rescue-PC spawn logic.
    pub playable: bool,
    /// Player command surface exposed by this hero. This is independent from
    /// the decision policy: a tactical unit can accept high-level player
    /// orders while retaining EnemyAi for moment-to-moment decisions.
    #[serde(default = "default_hero_command_interface")]
    pub command_interface: CommandInterface,
    /// Mission bookkeeping role, independent from allegiance and controller.
    #[serde(default = "default_hero_mission_role")]
    pub mission_role: MissionRole,
    /// Engagement policy used by whichever controller owns combat decisions.
    #[serde(default)]
    pub combat_stance: CombatStance,
    /// Shared AI runtime used by AI-controlled heroes. Ordinary directly
    /// controlled heroes intentionally have no AI owner.
    #[serde(default)]
    pub ai: Option<Box<AiActorData>>,

    /// Whether the per-PC UI panel should be hidden.  Toggled today by
    /// the `CALL <initial> HIDEINTERFACE|DISPLAYINTERFACE` console
    /// cheat.  The HUD port reads this flag when rendering.
    pub interface_hidden: bool,

    // Actions
    pub current_action: Action,
    pub saved_action: Action,
    pub disabled_actions: Vec<bool>,
    pub disabled_actions_temp: Vec<bool>,
    /// Live ammo state used by actor-local script behavior.  In campaign
    /// mode mutations are mirrored to the campaign character status.
    #[serde(default)]
    pub ammo: PcAmmoData,

    // Quick actions
    pub quick_action_types: Vec<QuickAction>,
    /// Stored sequences for each QA slot (up to 3). When the player
    /// replays a QA, the engine launches the sequence from this slot.
    pub quick_action_sequences: Vec<Option<crate::sequence::Sequence>>,
    pub quick_seek_sequences: Vec<Option<crate::sequence::Sequence>>,
    pub quick_action_special_counts: Vec<u16>,
    pub quick_action_buttons: Vec<u16>,
    pub quick_action_interactors: Vec<Option<EntityId>>,
    pub titbits: Vec<Option<crate::titbit::TitbitId>>,
    pub portrait: PcPortraitState,

    // Detection
    pub head_seen: bool,
    pub belt_seen: bool,
    pub feet_seen: bool,

    // Teleport
    pub position_before_teleport: MapPoint,
    /// Frames remaining in the cheat-teleport hulk-rebuild fade.
    /// Decremented each frame by the per-PC render path (not yet
    /// implemented); read here by teleport setup to suppress the
    /// old-position star burst when a re-teleport fires while the
    /// previous fade is still in flight.
    pub teleport_counter: u16,
    /// Initial value of [`Self::teleport_counter`] when the most recent
    /// teleport began.  Used by the render path to compute the fade
    /// percentage.
    pub max_teleport_counter: u16,
    pub fried_psykokwack: bool,

    // Carried person
    pub carried: Option<EntityId>,
    /// Raw original-game carried-posture storage.
    ///
    /// Original-game initialization leaves it indeterminate while `carried` is
    /// null. It is validated as a [`Posture`] whenever a carried body makes
    /// the field semantically live.
    pub carried_posture: u32,

    // Shield
    pub shield_danger_point: WorldPoint3D,
    /// Map layer the player picked when raising the shield, used as the
    /// layer for the danger-point titbit.  Differs from the PC's own
    /// layer when the danger is across a chasm / off a balcony.
    pub shield_danger_point_layer: u16,
    pub shield_protected: Option<EntityId>,
    pub shield_protector: Option<EntityId>,

    // Guard
    pub guard: Option<EntityId>,

    // Reinforcement
    pub time_till_reinforcement: u32,

    // Sherwood
    pub work_icon: WorkIcon,

    // Ammo dropping
    pub last_ammo_dropping_position: MapPoint,
    pub last_dropped_ammo: Option<EntityId>,
    pub update_last_dropped_ammo: bool,
    pub last_dropping_direction: u8,

    /// References character profile.
    pub profile_index: CharacterProfileIdx,
    /// Which of the 10 playable characters this PC represents.  `None`
    /// when level load encountered a character profile whose
    /// `profile_name` string isn't one of the known French names
    /// (mirrors the previous empty-string fallback).
    pub kind: Option<crate::character_kind::CharacterKind>,
    /// Cached contextual movement permissions from the character
    /// profile.  `disabled_actions` only tracks the three quick-action
    /// slots, not these profile-level abilities.
    pub has_lockpick: bool,
    pub has_climb: bool,
    pub has_jump: bool,

    /// Beam-me spawn index for Sherwood HQ positioning.
    /// -1 = not assigned. Set by engine during level setup.
    pub beam_me_index: i16,

    /// Whether the portrait's "trumpet" replacement-available indicator
    /// should be shown.  Set by the PC kill path when a non-VIP peasant
    /// is still available in the gang to replace the killed PC.
    pub trumpet_enabled: bool,

    /// The PC's current melee target (sword opponent).
    ///
    /// Set when the PC enters a swordfight, cleared when the fight
    /// ends.  Used to populate `FighterSnapshot.principal_opponent` so
    /// the enemy AI can reason about PC combat pairings.
    pub melee_target: Option<EntityId>,

    /// Initial action set from level data (beam-me `actionInitial`).
    /// Evaluated by action initialization to set the PC's starting
    /// state.
    pub initial_action: u32,

    /// Forbidden hero expression list (expression_id, forbid_timer).
    /// Each entry counts down each frame and is removed at 0, preventing
    /// the same expression from repeating too quickly.
    pub forbidden_expressions: Vec<(u16, u16)>,

    /// Last `combat_anim` id observed by the speech-trigger tick — used
    /// to detect the START of a new animation and the DONE transition
    /// (anim cleared) for a remark played after an action.
    pub prev_combat_anim_id: u32,
    pub prev_combat_anim_ot: Option<crate::order::OrderType>,
}

impl Default for PcData {
    fn default() -> Self {
        Self {
            life_points: crate::pc_status::LIFEPOINTS_PC,
            immortal: false,
            robin: false,
            already_selected: false,
            list_index: 0,
            cached_camp: Camp::Royalists,
            campaign_description_index: None,
            playable: true,
            command_interface: CommandInterface::HeroActions,
            mission_role: MissionRole::PlayerParty,
            combat_stance: CombatStance::Aggressive,
            ai: None,
            interface_hidden: false,
            current_action: Action::default(),
            saved_action: Action::default(),
            disabled_actions: Vec::new(),
            disabled_actions_temp: Vec::new(),
            ammo: PcAmmoData::default(),
            quick_action_types: vec![QuickAction::None; 3],
            quick_action_sequences: vec![None, None, None],
            quick_seek_sequences: vec![None, None, None],
            quick_action_special_counts: vec![0; 3],
            quick_action_buttons: vec![0; 3],
            quick_action_interactors: vec![None; 3],
            titbits: vec![None; 3],
            portrait: PcPortraitState::default(),
            head_seen: false,
            belt_seen: false,
            feet_seen: false,
            position_before_teleport: MapPoint::default(),
            teleport_counter: 0,
            max_teleport_counter: 0,
            fried_psykokwack: false,
            carried: None,
            carried_posture: Posture::Undefined as u32,
            shield_danger_point: WorldPoint3D::default(),
            shield_danger_point_layer: 0,
            shield_protected: None,
            shield_protector: None,
            guard: None,
            time_till_reinforcement: 0xFFFF_FFFF,
            work_icon: WorkIcon::default(),
            last_ammo_dropping_position: MapPoint::default(),
            last_dropped_ammo: None,
            update_last_dropped_ammo: false,
            last_dropping_direction: 0,
            profile_index: CharacterProfileIdx(0),
            kind: None,
            has_lockpick: false,
            has_climb: false,
            has_jump: false,
            beam_me_index: -1,
            trumpet_enabled: false,
            melee_target: None,
            initial_action: 0,
            forbidden_expressions: Vec::new(),
            prev_combat_anim_id: 0,
            prev_combat_anim_ot: None,
        }
    }
}

impl PcData {
    /// Quick-action slots can be disabled permanently or for the current frame.
    /// Missing trailing slots are intentionally enabled (the authored arrays are sparse).
    pub fn action_slot_disabled(&self, index: usize) -> bool {
        self.disabled_actions.get(index).copied().unwrap_or(false)
            || self
                .disabled_actions_temp
                .get(index)
                .copied()
                .unwrap_or(false)
    }

    /// Apply the original game's player-character playability state change.
    ///
    /// In retail missions, making a PRIS rescue PC playable is also the
    /// boundary where that scripted prisoner becomes an ordinary party hero.
    /// Those facts were implicit in the original game; keep the
    /// transition together now that command surface and mission role are
    /// represented independently.
    pub fn set_playable(&mut self, playable: bool) {
        self.playable = playable;
        if playable && self.mission_role == MissionRole::RescueTarget {
            self.mission_role = MissionRole::PlayerParty;
            self.command_interface = CommandInterface::HeroActions;
            self.combat_stance = CombatStance::Aggressive;
        }
    }

    pub fn live_carried_posture(&self) -> Posture {
        Posture::try_from(self.carried_posture).unwrap_or_else(|_| {
            panic!(
                "live carried_posture contains invalid original-game enum word {}",
                self.carried_posture
            )
        })
    }

    pub fn set_live_carried_posture(&mut self, posture: Posture) {
        self.carried_posture = posture as u32;
    }

    pub fn movement_auth_from_profile(profile: &CharacterProfile) -> (bool, bool, bool) {
        (
            profile.has_contextual_action(Action::Lockpick),
            profile.has_contextual_action(Action::Climb),
            profile.has_contextual_action(Action::Jump),
        )
    }
}

impl PcData {
    /// Unconditionally save the current action and clear it; then,
    /// **only if `playable`**, mark every action temp-disabled. The
    /// widget messaging side-effect is omitted — the HUD reads
    /// `disabled_actions_temp` directly each frame.
    pub fn disable_all_actions_temp(&mut self) {
        self.saved_action = self.current_action;
        self.current_action = Action::default();
        if self.playable {
            for slot in self.disabled_actions_temp.iter_mut() {
                *slot = true;
            }
        }
    }

    /// Gated on `!is_swordfighting && playable`. Inside the guard each
    /// temp-disabled slot is conditionally cleared, and if any cleared
    /// slot's authored action matches `saved_action` (and the permanent mask
    /// is also clear), the saved action is returned for the engine to forward
    /// through `MSG_SELECT_ACTION` semantics.
    /// The widget messaging side-effect is omitted — the HUD reads
    /// state directly.
    ///
    /// `is_swordfighting` is provided by the caller because the
    /// authoritative check (`HumanData::opponents.is_empty()`) lives on
    /// the human layer and we don't take the whole `Entity` here.
    pub fn enable_all_actions_temp(
        &mut self,
        is_swordfighting: bool,
        actions: &[Action; crate::profiles::NUMBER_OF_PC_ACTIONS],
    ) -> Option<Action> {
        if is_swordfighting || !self.playable {
            return None;
        }
        let mut restore_action = None;
        for (idx, slot) in self.disabled_actions_temp.iter_mut().enumerate() {
            if *slot {
                *slot = false;
                let permanent_disabled = self.disabled_actions.get(idx).copied().unwrap_or(false);
                if !permanent_disabled
                    && actions.get(idx).copied() == Some(self.saved_action)
                    && restore_action.is_none()
                {
                    restore_action = Some(self.saved_action);
                }
            }
        }
        restore_action
    }
}
