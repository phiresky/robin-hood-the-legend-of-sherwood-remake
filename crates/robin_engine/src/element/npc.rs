//! AI and non-player actor data and local behavior.
use super::*;

/// A detectable entity tracked by NPC vision.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct Detectable {
    pub element: Option<EntityId>,
    pub detectable_type: DetectableType,
    pub seen_last_frame: bool,
    pub heard_last_frame: bool,
    pub seen_now: bool,
    pub shadow_seen_now: bool,
    pub shadow_seen_last_frame: bool,
    pub last_visibility: f32,
}

impl Default for Detectable {
    fn default() -> Self {
        Self {
            element: None,
            detectable_type: DetectableType::None,
            seen_last_frame: false,
            heard_last_frame: false,
            seen_now: false,
            shadow_seen_now: false,
            shadow_seen_last_frame: false,
            last_visibility: 0.0,
        }
    }
}

/// AI brain enum.  Each NPC owns one of these; soldiers get
/// [`EnemyAi`], civilians get [`FriendlyAi`].
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum AiBrain {
    #[default]
    None,
    Enemy(Box<EnemyAi>),
    Friendly(Box<FriendlyAi>),
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) enum PersistedAiBrain {
    None,
    Enemy(Box<crate::ai::persisted::PersistedEnemyAi>),
    Friendly(Box<crate::ai::persisted::PersistedFriendlyAi>),
}

impl PersistedAiBrain {
    pub(crate) fn capture(value: &AiBrain) -> Self {
        match value {
            AiBrain::None => Self::None,
            AiBrain::Enemy(value) => Self::Enemy(Box::new(
                crate::ai::persisted::PersistedEnemyAi::capture(value),
            )),
            AiBrain::Friendly(value) => Self::Friendly(Box::new(
                crate::ai::persisted::PersistedFriendlyAi::capture(value),
            )),
        }
    }
    pub(crate) fn into_runtime(self) -> AiBrain {
        match self {
            Self::None => AiBrain::None,
            Self::Enemy(value) => AiBrain::Enemy(Box::new(value.into_runtime())),
            Self::Friendly(value) => AiBrain::Friendly(Box::new(value.into_runtime())),
        }
    }
}

impl AiBrain {
    /// Access the base `AiController` (common to both enemy and friendly).
    pub fn base(&self) -> Option<&AiController> {
        match self {
            Self::None => None,
            Self::Enemy(e) => Some(&e.base),
            Self::Friendly(f) => Some(&f.base),
        }
    }

    /// Mutable access to the base `AiController`.
    pub fn base_mut(&mut self) -> Option<&mut AiController> {
        match self {
            Self::None => None,
            Self::Enemy(e) => Some(&mut e.base),
            Self::Friendly(f) => Some(&mut f.base),
        }
    }

    /// Access the enemy AI subclass, if this is a soldier.
    pub fn enemy(&self) -> Option<&EnemyAi> {
        match self {
            Self::Enemy(e) => Some(e),
            _ => None,
        }
    }

    /// Mutable access to the enemy AI subclass.
    pub fn enemy_mut(&mut self) -> Option<&mut EnemyAi> {
        match self {
            Self::Enemy(e) => Some(e),
            _ => None,
        }
    }

    /// Access the friendly AI subclass, if this is a civilian.
    pub fn friendly(&self) -> Option<&FriendlyAi> {
        match self {
            Self::Friendly(f) => Some(f),
            _ => None,
        }
    }

    /// Mutable access to the friendly AI subclass.
    pub fn friendly_mut(&mut self) -> Option<&mut FriendlyAi> {
        match self {
            Self::Friendly(f) => Some(f),
            _ => None,
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

/// Per-actor state owned by the NPC AI runtime.
///
/// This is separate from [`NpcData`] so an AI-controlled hero can run the same
/// perception and battle-decision machinery without masquerading as an NPC or
/// carrying a second, potentially divergent copy of its body health.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct AiActorData {
    /// Persistent NPC-only construction ordinal.
    ///
    /// The original-game NPC actor assigns this from
    /// `guwNPCRegisterNumber` and uses it to stagger periodic work. It is
    /// distinct from both the entity-table slot and element creation order
    /// because non-NPC elements do not increment the counter.
    pub register_number: u16,
    pub number_of_arrows: u16,

    pub direction_old: i16,
    pub initial_view_direction: MapVec,
    pub initial_position_x: f32,
    pub initial_position_y: f32,
    pub initial_position_sector: Option<crate::position_interface::SectorHandle>,
    pub initial_position_level: u16,

    pub inform_my_friends: bool,
    pub money: u32,
    pub wasp_victim: bool,

    pub old_cover_noise_deafness: u16,
    pub old_cover_noise_deafness_frame_counter: u32,

    /// Frames spent stuck on an outdoor ladder while idle.  Bumped
    /// each tick by `tick_npc_stuck_on_ladder_for_npc` when the NPC is on a
    /// ladder in a non-building sector with command `Wait`/`MoveWaiting`
    /// and not script-locked; reset otherwise.  After 25 frames the
    /// engine forces a return to duty so NPCs that hang on outdoor
    /// ladders can self-recover.
    pub stuck_on_ladder_emergency_counter: u16,

    /// Exact dialog-scroll entity attached by Original's
    /// NPC attached scroll.
    ///
    /// Keeping the identity (rather than only a boolean) matters when a
    /// loaded NPC is clicked: the interaction must open the same scroll
    /// object which was serialized in the save.
    pub attached_scroll: Option<EntityId>,

    /// Original-game serialized body-visitor continuation counter.
    pub body_visitors: u16,

    /// Original-game serialized special script latch. The original game stores
    /// this for NPCs even though its AI tick never reads it.
    pub fried_pikachu: bool,

    /// One detection list per [`DetectableType`] (indexed 0..COUNT).
    pub detectable_lists: Vec<Vec<Detectable>>,
    pub detection_suspects: [u16; DetectableType::COUNT],
    pub maximal_detection_suspect: u16,
    pub worst_detected_type: DetectableType,

    pub has_given_money_to_beggar: bool,

    pub custom_values: [i32; NpcCustomValue::COUNT],

    pub display_double_status_bar: bool,

    // -- Cross-module reference: AI controller --
    /// The NPC's AI brain — either an enemy AI or a civilian AI.
    pub ai_brain: AiBrain,

    /// `true` once this NPC has spotted a hostile and is actively pursuing
    /// or attacking.  Kept in sync with `ai_state == Attacking` by
    /// `EngineInner::tick_enemy_ai`.  Exists as a cheap flag so combat checks
    /// don't need to crack open the full AI controller.
    pub alerted: bool,

    /// Real view radius (map units).  Initialized from
    /// the engine's standard-view-radius helper at level load —
    /// day/night dependent — and subsequently mutated by the AI
    /// (alertness,
    /// drunk cone iterator, lean-out, etc).  For now we only track the
    /// base value; the per-frame mutation logic in view refresh is
    /// not yet implemented.
    pub view_radius: u16,

    /// Eye / view status — controls whether the NPC can see at all.
    /// When set to `EyeStatus::Closed` or
    /// `EyeStatus::DieOrGetUnconscious` the vision pipeline returns
    /// 0.0 visibility.
    pub eye_status: EyeStatus,

    /// Live half-aperture (radians) used for NPC vision geometry.
    /// The *real* vision cone is built with this value; it starts at
    /// `NORMAL_HALF_APERTURE` but is mutated at runtime by alert
    /// state, drunk-cone iterator, `ViewconeGrow` status, lean-out
    /// posture, and the forest-level Royalist 180° special case.
    ///
    /// **Mutation is not yet fully implemented.** The value stays at the
    /// initial `NORMAL_HALF_APERTURE` until view-refresh logic
    /// lands.  The view cone overlay and AI vision code read this
    /// field so the port is ready to pick up the real values once
    /// mutation is wired.
    pub half_aperture: f32,

    /// "Real" half-aperture after all modifiers (stare, drunk, etc.).
    /// Updated by `ai_vision::refresh_view` each frame.
    pub real_half_aperture: f32,

    // -- View state --
    // Populated by `ai_vision::refresh_view` each frame.
    /// View angle offset from body direction (radians).  Head turns
    /// (look-left/right) and stare/follow rotate the view cone
    /// relative to the body.
    pub view_angle: f32,

    /// Per-frame angle step for view transitions (default π/16).
    pub view_angle_step: f32,

    /// Set when the body direction or eye status changes; cleared
    /// when the view angle reaches its goal.
    pub view_transition: bool,

    /// Maximum angular deviation from body direction during head turns.
    pub view_half_angle_range: f32,

    /// Serialized view-cone angle oscillator continuation state.
    pub view_angle_iterator: f32,
    pub view_angle_iterator_step: f32,

    /// Base view radius before modifiers (longrange, drunk, rider).
    /// The final computed radius is stored in `view_radius`.
    pub view_radius_base: u16,

    /// Target radius for grow / death-shrink animations.
    pub view_radius_goal: u16,

    /// Accelerating step for the death-shrink radius animation.
    pub view_radius_step: u16,

    /// Alpha intensity for the view cone overlay (0-255).
    pub view_alpha_start: u16,

    /// Long-range radius multiplier (default 1.0).
    pub view_longrange_radius_factor: f32,

    /// Serialized aperture-transition continuation state. The transition
    /// block is disabled in the shipped original game, but these members
    /// remain part of the authoritative save image.
    pub view_half_aperture_cosine: f32,
    pub view_future_half_aperture: f32,
    pub view_half_aperture_step: f32,
    pub view_half_aperture_changes: bool,

    /// Serialized "crazy" cone oscillator continuation state.
    pub view_crazy_angle_iterator: f32,
    pub view_crazy_angle_iterator_step: f32,
    pub view_crazy_color_iterator: u8,
    pub view_crazy_half_angle_range: f32,

    /// Computed view direction (body direction rotated by `view_angle`).
    /// Updated by `refresh_view` each frame.
    pub view_direction: [f32; 2],

    /// Serialized cone boundary vectors. Original consumes these in
    /// visibility tests until view refresh computes the next pair.
    pub view_left_side: [f32; 2],
    pub view_right_side: [f32; 2],

    /// Whether the NPC is currently leaning out.
    pub view_lean_out: bool,

    /// Four phase iterators for drunken vision cone wobble.
    pub drunken_cone_iterators: [f32; 4],

    /// Serialized radius-reduction and sniper view flags.
    pub view_radius_reduction_permil: u16,
    pub view_sniper: bool,

    /// Point the NPC is staring at (for `EyeStatus::Stare`).
    /// Original-game stare point: world-ground `(x, y)`, not
    /// projected map coordinates. Position focus and eye-following
    /// populate this from a 3D point's X/Y components.
    pub stare_point: GroundPoint,

    /// Entity the view cone follows (for `EyeStatus::Follow`).
    pub follow_target: Option<EntityId>,
}

/// View / eye status enum.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u8)]
pub enum EyeStatus {
    Closed = 0,
    #[default]
    LookForward,
    LookToTheLeft,
    LookToTheRight,
    LookDownwards,
    DieOrGetUnconscious,
    Follow,
    Stare,
    ViewconeGrow,
}

impl EyeStatus {
    /// `true` when the NPC's eyes are non-functional and visibility
    /// must short-circuit to 0.
    #[inline]
    pub fn is_blind(self) -> bool {
        matches!(self, Self::Closed | Self::DieOrGetUnconscious)
    }
}

impl Default for AiActorData {
    fn default() -> Self {
        Self {
            register_number: 0,
            // Seed `MAX_NPC_ARROWS` for every NPC unconditionally —
            // civilians, friendlies, hostile soldiers — even those
            // who never use a bow.  Arrows are only consumed when a
            // bow shot resolves, so the spare quiver on non-archers
            // is harmless. Without this seed, bow-carrying enemy
            // soldiers would spawn with 0 arrows and fire nothing
            // until the `FleeingRunForArrowReserves` refill path
            // triggers.
            number_of_arrows: crate::parameters_ai::MAX_NPC_ARROWS as u16,
            direction_old: 0,
            initial_view_direction: MapVec::default(),
            initial_position_x: 0.0,
            initial_position_y: 0.0,
            initial_position_sector: None,
            initial_position_level: 0,
            inform_my_friends: false,
            money: 0,
            wasp_victim: false,
            old_cover_noise_deafness: 0,
            old_cover_noise_deafness_frame_counter: 0,
            stuck_on_ladder_emergency_counter: 0,
            attached_scroll: None,
            body_visitors: 0,
            fried_pikachu: false,
            detectable_lists: vec![Vec::new(); DetectableType::COUNT],
            detection_suspects: [0; DetectableType::COUNT],
            maximal_detection_suspect: 0,
            worst_detected_type: DetectableType::None,
            has_given_money_to_beggar: false,
            custom_values: [0; NpcCustomValue::COUNT],
            display_double_status_bar: false,
            ai_brain: AiBrain::None,
            alerted: false,
            // The engine overwrites this with the correct day/night
            // view radius during level-load initialization,
            // but 400 is a safe fallback if nothing wires it up.
            view_radius: 400,
            eye_status: EyeStatus::LookForward,
            // `NORMAL_HALF_APERTURE = 0.5 rad` (~28.6°). This is the
            // initial value before per-alert mutation kicks in.
            half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
            real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
            view_angle: 0.0,
            view_angle_step: crate::ai_vision::NORMAL_ANGLE_STEP,
            view_transition: false,
            view_half_angle_range: crate::ai_vision::NORMAL_HALF_ANGLE_RANGE,
            view_angle_iterator: 0.0,
            view_angle_iterator_step: crate::ai_vision::NORMAL_ANGLE_ITERATOR_STEP,
            view_radius_base: 400,
            view_radius_goal: 400,
            // Original-game NPC initialization sets the radius step to 10. The
            // DieOrGetUnconscious view-cone collapse consumes this value on
            // its first view refresh and then accelerates it by 5 each frame.
            view_radius_step: 10,
            view_alpha_start: crate::ai_vision::ALPHA_START,
            view_longrange_radius_factor: 1.0,
            view_half_aperture_cosine: crate::ai_vision::NORMAL_HALF_APERTURE.cos(),
            view_future_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
            view_half_aperture_step: crate::parameters_ai::HALF_APERTURE_STEP,
            view_half_aperture_changes: false,
            view_crazy_angle_iterator: 0.0,
            view_crazy_angle_iterator_step: crate::parameters_ai::CRAZY_TREMBLE_ITERATOR_STEP,
            view_crazy_color_iterator: 0,
            view_crazy_half_angle_range: crate::parameters_ai::CRAZY_TREMBLE_RANGE,
            view_direction: [1.0, 0.0],
            view_left_side: [1.0, 0.0],
            view_right_side: [1.0, 0.0],
            view_lean_out: false,
            drunken_cone_iterators: [0.0; 4],
            view_radius_reduction_permil: 1000,
            view_sniper: false,
            stare_point: GroundPoint::new(0.0, 0.0),
            follow_target: None,
        }
    }
}

impl AiActorData {
    /// Original-game NPC detectable deletion removes only the first
    /// matching entry and report whether one was found. Release recordings can
    /// contain duplicates because several original-game insertion paths only
    /// guard uniqueness with a debug assertion.
    pub fn delete_detectable(
        &mut self,
        element: EntityId,
        detectable_type: DetectableType,
    ) -> bool {
        let index = detectable_type as usize;
        let list = self.detectable_lists.get_mut(index).unwrap_or_else(|| {
            panic!(
                "NPC has no {:?} detectable list at index {index}",
                detectable_type
            )
        });
        let Some(position) = list
            .iter()
            .position(|detectable| detectable.element == Some(element))
        else {
            return false;
        };
        list.remove(position);
        true
    }

    /// Current AI top-level state, read from the owning [`AiController`]
    /// (single source of truth).  NPCs without an AI brain report
    /// [`AiTopState::Default`], matching the pre-consolidation default
    /// value of the removed stored field.
    pub fn ai_state(&self) -> AiTopState {
        self.ai_brain
            .base()
            .map(|b| b.current_state)
            .unwrap_or(AiTopState::Default)
    }

    /// Current AI substate, read from the owning [`AiController`]
    /// (single source of truth).  NPCs without an AI brain report
    /// [`AiSubstate::DefaultOnPost`], matching the pre-consolidation
    /// default value of the removed stored field.
    pub fn ai_substate(&self) -> AiSubstate {
        self.ai_brain
            .base()
            .map(|b| b.current_substate)
            .unwrap_or(AiSubstate::DefaultOnPost)
    }

    /// Returns the current cover-noise deafness after applying decay.
    /// The engine should call this each frame via the hearing path.
    ///
    /// `cover_volume` is the max of every active sound source's
    /// covering-volume-at-position at the NPC's position.  The caller
    /// supplies it because `NpcData` has no access to the engine's
    /// `SoundSourceManager`; pass `0` when no sound sources should
    /// mask hearing.
    pub fn get_deafness(&mut self, current_frame: u32, cover_volume: u16) -> u16 {
        use crate::parameters_ai;

        // Same-frame short-circuit.  Only fires when the
        // function has already been called this frame; the
        // `cover_volume` argument is irrelevant here because the prior
        // call already folded the per-frame covering volume into the
        // stored value.
        if self.old_cover_noise_deafness_frame_counter == current_frame {
            return self.old_cover_noise_deafness;
        }

        // Catch-up decay loop.  Runs until the counter catches up OR
        // the deafness reaches zero.  No iteration cap — the decay
        // rate guarantees a bounded number of steps before deafness
        // reaches zero (slow decay alone needs at most
        // ceil(300 / AI_DEAFNESS_MINUS) iterations to bottom out).
        while self.old_cover_noise_deafness_frame_counter < current_frame
            && self.old_cover_noise_deafness > 0
        {
            // Stepped fast decay above 300.  The integer division
            // `(deaf / RADIUS)` evaluates BEFORE the multiplication,
            // giving a stepped reduction:
            //   deaf in [300, 599]  → subtract 50 * 1
            //   deaf in [600, 899]  → subtract 50 * 2
            //   …
            // Slow decay below 300 subtracts a flat `AI_DEAFNESS_MINUS`.
            if self.old_cover_noise_deafness > parameters_ai::AI_QUICK_DEAFNESS_RADIUS as u16 {
                let fast = (parameters_ai::AI_QUICK_DEAFNESS_MINUS as u32
                    * (self.old_cover_noise_deafness as u32
                        / parameters_ai::AI_QUICK_DEAFNESS_RADIUS as u32))
                    as u16;
                self.old_cover_noise_deafness = self.old_cover_noise_deafness.saturating_sub(fast);
            } else {
                self.old_cover_noise_deafness = self
                    .old_cover_noise_deafness
                    .saturating_sub(parameters_ai::AI_DEAFNESS_MINUS as u16);
            }
            self.old_cover_noise_deafness_frame_counter = self
                .old_cover_noise_deafness_frame_counter
                .saturating_add(1);
        }
        // If we exited the loop because deafness hit zero before the
        // counter caught up, snap the counter forward so the same-frame
        // short-circuit at the top of the function fires correctly on
        // subsequent calls this frame.
        self.old_cover_noise_deafness_frame_counter = current_frame;

        // Take the max of current deafness and the covering volume
        // from active sound sources at this position.  The caller
        // pre-computes `cover_volume` because `NpcData` lacks access
        // to the `SoundSourceManager`.
        if cover_volume > self.old_cover_noise_deafness {
            self.old_cover_noise_deafness = cover_volume;
        }

        self.old_cover_noise_deafness
    }

    /// Zeroes every per-detectable suspect accumulator and the cached
    /// worst-threat summary.  Called when the NPC transitions into
    /// unconsciousness so the pre-knockout hostility tint / blip color
    /// doesn't leak to wake-up.
    pub fn clear_all_suspects(&mut self) {
        for slot in self.detection_suspects.iter_mut() {
            *slot = 0;
        }
        self.maximal_detection_suspect = 0;
        self.worst_detected_type = DetectableType::None;
    }
}

/// Body state shared by soldier and civilian entities plus their AI runtime.
///
/// `Deref` preserves the existing `npc.ai_brain`/vision-field API while making
/// the ownership boundary explicit for actors which are not NPC bodies.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct NpcData {
    pub life_points: i16,
    #[serde(flatten)]
    pub ai: AiActorData,
}

impl Default for NpcData {
    fn default() -> Self {
        Self {
            life_points: crate::pc_status::LIFEPOINTS_PC,
            ai: AiActorData::default(),
        }
    }
}

impl std::ops::Deref for NpcData {
    type Target = AiActorData;

    fn deref(&self) -> &Self::Target {
        &self.ai
    }
}

impl std::ops::DerefMut for NpcData {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.ai
    }
}

/// Soldier-specific data.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SoldierData {
    pub apple_smell: u32,
    /// References soldier profile data.
    pub soldier_profile_index: SoldierProfileIdx,
    /// Cached from profile at creation time.
    pub cached_max_life_points: i16,
    /// Cached from profile at creation time.
    pub cached_camp: Camp,
    /// Whether this soldier is mounted on a horse.
    pub rider: bool,
    /// Optional player-facing tactical order surface. EnemyAi remains the
    /// decision policy even when this is enabled.
    #[serde(default)]
    pub command_interface: CommandInterface,
    /// Mission bookkeeping role, independent from allegiance.
    #[serde(default)]
    pub mission_role: MissionRole,
    /// Default stance before/without an explicit tactical order.
    #[serde(default)]
    pub combat_stance: CombatStance,
}

/// Civilian-specific data.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct CivilianData {
    pub current_scroll_set: u32,
    /// References civilian profile data.
    pub civilian_profile_index: CivilianProfileIdx,
    /// Cached from profile at creation time.
    pub cached_camp: Camp,
    /// Cached civilian type (Beggar/Child/Vip/Standard) from profile
    /// at load time.
    pub cached_civilian_type: crate::profiles::CivilianType,
    /// Per-scroll-set scroll IDs for beggar civilians (10 sets).
    /// `None` for non-beggar civilians.
    pub beggar_scroll_sets: Option<Vec<Vec<u16>>>,
}
