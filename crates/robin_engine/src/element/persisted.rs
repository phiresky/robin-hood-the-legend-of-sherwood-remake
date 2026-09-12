//! Save-owned projections. Nested sprite and AI state intentionally differ from rollback clones.
use super::*;

/// Explicit save-owned projection; process-local state is reconstructed here,
/// independently of raw rollback cloning and the native wire codec.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PersistedElementData {
    kind: ElementKind,

    blipped: bool,

    class_id: u16,

    active: bool,

    hidden_in_building: bool,

    sprite_id: u32,

    select_id: u32,

    position_map_delayed: bool,

    delayed_map_position: MapPoint,

    position_delayed: bool,

    delayed_position: WorldPoint3D,

    in_honolulu: bool,

    index_in_elements_list: u16,

    custom_minimap_dot: u16,

    outline_colors: [u16; OutlineColorName::COUNT],

    current_outline: OutlineColorName,

    outline_width: u16,

    unreachable: bool,

    posture: Posture,

    sprite: crate::sprite::SpriteSnapshot,

    grid_cell: Option<(u16, u16)>,
}

impl PersistedElementData {
    pub(crate) fn capture(value: &ElementData) -> Self {
        let ElementData {
            kind: _,
            blipped: _,
            class_id: _,
            active: _,
            hidden_in_building: _,
            sprite_id: _,
            select_id: _,
            position_map_delayed: _,
            delayed_map_position: _,
            position_delayed: _,
            delayed_position: _,
            in_honolulu: _,
            index_in_elements_list: _,
            custom_minimap_dot: _,
            outline_colors: _,
            current_outline: _,
            outline_width: _,
            unreachable: _,
            posture: _,
            sprite: _,
            grid_cell: _,
        } = value;
        Self {
            kind: value.kind,
            blipped: value.blipped,
            class_id: value.class_id,
            active: value.active,
            hidden_in_building: value.hidden_in_building,
            sprite_id: value.sprite_id,
            select_id: value.select_id,
            position_map_delayed: value.position_map_delayed,
            delayed_map_position: value.delayed_map_position,
            position_delayed: value.position_delayed,
            delayed_position: value.delayed_position,
            in_honolulu: value.in_honolulu,
            index_in_elements_list: value.index_in_elements_list,
            custom_minimap_dot: value.custom_minimap_dot,
            outline_colors: value.outline_colors,
            current_outline: value.current_outline,
            outline_width: value.outline_width,
            unreachable: value.unreachable,
            posture: value.posture,
            sprite: crate::sprite::SpriteSnapshot::capture(&value.sprite),
            grid_cell: value.grid_cell,
        }
    }

    pub(crate) fn into_runtime(self) -> ElementData {
        ElementData {
            kind: self.kind,
            blipped: self.blipped,
            class_id: self.class_id,
            active: self.active,
            hidden_in_building: self.hidden_in_building,
            sprite_id: self.sprite_id,
            select_id: self.select_id,
            position_map_delayed: self.position_map_delayed,
            delayed_map_position: self.delayed_map_position,
            position_delayed: self.position_delayed,
            delayed_position: self.delayed_position,
            in_honolulu: self.in_honolulu,
            index_in_elements_list: self.index_in_elements_list,
            custom_minimap_dot: self.custom_minimap_dot,
            outline_colors: self.outline_colors,
            current_outline: self.current_outline,
            outline_width: self.outline_width,
            unreachable: self.unreachable,
            posture: self.posture,
            sprite: self.sprite.into_runtime(),
            grid_cell: self.grid_cell,
        }
    }
}

/// Explicit save-owned projection; process-local state is reconstructed here,
/// independently of raw rollback cloning and the native wire codec.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PersistedAiActorData {
    register_number: u16,

    number_of_arrows: u16,

    direction_old: i16,

    initial_view_direction: MapVec,

    initial_position_x: f32,

    initial_position_y: f32,

    initial_position_sector: Option<crate::position_interface::SectorHandle>,

    initial_position_level: u16,

    inform_my_friends: bool,

    money: u32,

    wasp_victim: bool,

    old_cover_noise_deafness: u16,

    old_cover_noise_deafness_frame_counter: u32,

    stuck_on_ladder_emergency_counter: u16,

    attached_scroll: Option<EntityId>,

    body_visitors: u16,

    fried_pikachu: bool,

    detectable_lists: Vec<Vec<Detectable>>,

    detection_suspects: [u16; DetectableType::COUNT],

    maximal_detection_suspect: u16,

    worst_detected_type: DetectableType,

    has_given_money_to_beggar: bool,

    custom_values: [i32; NpcCustomValue::COUNT],

    display_double_status_bar: bool,

    ai_brain: PersistedAiBrain,

    alerted: bool,

    view_radius: u16,

    eye_status: EyeStatus,

    half_aperture: f32,

    real_half_aperture: f32,

    view_angle: f32,

    view_angle_step: f32,

    view_transition: bool,

    view_half_angle_range: f32,

    view_angle_iterator: f32,

    view_angle_iterator_step: f32,

    view_radius_base: u16,

    view_radius_goal: u16,

    view_radius_step: u16,

    view_alpha_start: u16,

    view_longrange_radius_factor: f32,

    view_half_aperture_cosine: f32,

    view_future_half_aperture: f32,

    view_half_aperture_step: f32,

    view_half_aperture_changes: bool,

    view_crazy_angle_iterator: f32,

    view_crazy_angle_iterator_step: f32,

    view_crazy_color_iterator: u8,

    view_crazy_half_angle_range: f32,

    view_direction: [f32; 2],

    view_left_side: [f32; 2],

    view_right_side: [f32; 2],

    view_lean_out: bool,

    drunken_cone_iterators: [f32; 4],

    view_radius_reduction_permil: u16,

    view_sniper: bool,

    stare_point: GroundPoint,

    follow_target: Option<EntityId>,
}

impl PersistedAiActorData {
    pub(crate) fn capture(value: &AiActorData) -> Self {
        let AiActorData {
            register_number: _,
            number_of_arrows: _,
            direction_old: _,
            initial_view_direction: _,
            initial_position_x: _,
            initial_position_y: _,
            initial_position_sector: _,
            initial_position_level: _,
            inform_my_friends: _,
            money: _,
            wasp_victim: _,
            old_cover_noise_deafness: _,
            old_cover_noise_deafness_frame_counter: _,
            stuck_on_ladder_emergency_counter: _,
            attached_scroll: _,
            body_visitors: _,
            fried_pikachu: _,
            detectable_lists: _,
            detection_suspects: _,
            maximal_detection_suspect: _,
            worst_detected_type: _,
            has_given_money_to_beggar: _,
            custom_values: _,
            display_double_status_bar: _,
            ai_brain: _,
            alerted: _,
            view_radius: _,
            eye_status: _,
            half_aperture: _,
            real_half_aperture: _,
            view_angle: _,
            view_angle_step: _,
            view_transition: _,
            view_half_angle_range: _,
            view_angle_iterator: _,
            view_angle_iterator_step: _,
            view_radius_base: _,
            view_radius_goal: _,
            view_radius_step: _,
            view_alpha_start: _,
            view_longrange_radius_factor: _,
            view_half_aperture_cosine: _,
            view_future_half_aperture: _,
            view_half_aperture_step: _,
            view_half_aperture_changes: _,
            view_crazy_angle_iterator: _,
            view_crazy_angle_iterator_step: _,
            view_crazy_color_iterator: _,
            view_crazy_half_angle_range: _,
            view_direction: _,
            view_left_side: _,
            view_right_side: _,
            view_lean_out: _,
            drunken_cone_iterators: _,
            view_radius_reduction_permil: _,
            view_sniper: _,
            stare_point: _,
            follow_target: _,
        } = value;
        Self {
            register_number: value.register_number,
            number_of_arrows: value.number_of_arrows,
            direction_old: value.direction_old,
            initial_view_direction: value.initial_view_direction,
            initial_position_x: value.initial_position_x,
            initial_position_y: value.initial_position_y,
            initial_position_sector: value.initial_position_sector,
            initial_position_level: value.initial_position_level,
            inform_my_friends: value.inform_my_friends,
            money: value.money,
            wasp_victim: value.wasp_victim,
            old_cover_noise_deafness: value.old_cover_noise_deafness,
            old_cover_noise_deafness_frame_counter: value.old_cover_noise_deafness_frame_counter,
            stuck_on_ladder_emergency_counter: value.stuck_on_ladder_emergency_counter,
            attached_scroll: value.attached_scroll,
            body_visitors: value.body_visitors,
            fried_pikachu: value.fried_pikachu,
            detectable_lists: value.detectable_lists.clone(),
            detection_suspects: value.detection_suspects,
            maximal_detection_suspect: value.maximal_detection_suspect,
            worst_detected_type: value.worst_detected_type,
            has_given_money_to_beggar: value.has_given_money_to_beggar,
            custom_values: value.custom_values,
            display_double_status_bar: value.display_double_status_bar,
            ai_brain: PersistedAiBrain::capture(&value.ai_brain),
            alerted: value.alerted,
            view_radius: value.view_radius,
            eye_status: value.eye_status,
            half_aperture: value.half_aperture,
            real_half_aperture: value.real_half_aperture,
            view_angle: value.view_angle,
            view_angle_step: value.view_angle_step,
            view_transition: value.view_transition,
            view_half_angle_range: value.view_half_angle_range,
            view_angle_iterator: value.view_angle_iterator,
            view_angle_iterator_step: value.view_angle_iterator_step,
            view_radius_base: value.view_radius_base,
            view_radius_goal: value.view_radius_goal,
            view_radius_step: value.view_radius_step,
            view_alpha_start: value.view_alpha_start,
            view_longrange_radius_factor: value.view_longrange_radius_factor,
            view_half_aperture_cosine: value.view_half_aperture_cosine,
            view_future_half_aperture: value.view_future_half_aperture,
            view_half_aperture_step: value.view_half_aperture_step,
            view_half_aperture_changes: value.view_half_aperture_changes,
            view_crazy_angle_iterator: value.view_crazy_angle_iterator,
            view_crazy_angle_iterator_step: value.view_crazy_angle_iterator_step,
            view_crazy_color_iterator: value.view_crazy_color_iterator,
            view_crazy_half_angle_range: value.view_crazy_half_angle_range,
            view_direction: value.view_direction,
            view_left_side: value.view_left_side,
            view_right_side: value.view_right_side,
            view_lean_out: value.view_lean_out,
            drunken_cone_iterators: value.drunken_cone_iterators,
            view_radius_reduction_permil: value.view_radius_reduction_permil,
            view_sniper: value.view_sniper,
            stare_point: value.stare_point,
            follow_target: value.follow_target,
        }
    }

    pub(crate) fn into_runtime(self) -> AiActorData {
        AiActorData {
            register_number: self.register_number,
            number_of_arrows: self.number_of_arrows,
            direction_old: self.direction_old,
            initial_view_direction: self.initial_view_direction,
            initial_position_x: self.initial_position_x,
            initial_position_y: self.initial_position_y,
            initial_position_sector: self.initial_position_sector,
            initial_position_level: self.initial_position_level,
            inform_my_friends: self.inform_my_friends,
            money: self.money,
            wasp_victim: self.wasp_victim,
            old_cover_noise_deafness: self.old_cover_noise_deafness,
            old_cover_noise_deafness_frame_counter: self.old_cover_noise_deafness_frame_counter,
            stuck_on_ladder_emergency_counter: self.stuck_on_ladder_emergency_counter,
            attached_scroll: self.attached_scroll,
            body_visitors: self.body_visitors,
            fried_pikachu: self.fried_pikachu,
            detectable_lists: self.detectable_lists,
            detection_suspects: self.detection_suspects,
            maximal_detection_suspect: self.maximal_detection_suspect,
            worst_detected_type: self.worst_detected_type,
            has_given_money_to_beggar: self.has_given_money_to_beggar,
            custom_values: self.custom_values,
            display_double_status_bar: self.display_double_status_bar,
            ai_brain: self.ai_brain.into_runtime(),
            alerted: self.alerted,
            view_radius: self.view_radius,
            eye_status: self.eye_status,
            half_aperture: self.half_aperture,
            real_half_aperture: self.real_half_aperture,
            view_angle: self.view_angle,
            view_angle_step: self.view_angle_step,
            view_transition: self.view_transition,
            view_half_angle_range: self.view_half_angle_range,
            view_angle_iterator: self.view_angle_iterator,
            view_angle_iterator_step: self.view_angle_iterator_step,
            view_radius_base: self.view_radius_base,
            view_radius_goal: self.view_radius_goal,
            view_radius_step: self.view_radius_step,
            view_alpha_start: self.view_alpha_start,
            view_longrange_radius_factor: self.view_longrange_radius_factor,
            view_half_aperture_cosine: self.view_half_aperture_cosine,
            view_future_half_aperture: self.view_future_half_aperture,
            view_half_aperture_step: self.view_half_aperture_step,
            view_half_aperture_changes: self.view_half_aperture_changes,
            view_crazy_angle_iterator: self.view_crazy_angle_iterator,
            view_crazy_angle_iterator_step: self.view_crazy_angle_iterator_step,
            view_crazy_color_iterator: self.view_crazy_color_iterator,
            view_crazy_half_angle_range: self.view_crazy_half_angle_range,
            view_direction: self.view_direction,
            view_left_side: self.view_left_side,
            view_right_side: self.view_right_side,
            view_lean_out: self.view_lean_out,
            drunken_cone_iterators: self.drunken_cone_iterators,
            view_radius_reduction_permil: self.view_radius_reduction_permil,
            view_sniper: self.view_sniper,
            stare_point: self.stare_point,
            follow_target: self.follow_target,
        }
    }
}

/// Explicit save-owned projection; process-local state is reconstructed here,
/// independently of raw rollback cloning and the native wire codec.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PersistedNpcData {
    life_points: i16,
    #[serde(flatten)]
    ai: PersistedAiActorData,
}

impl PersistedNpcData {
    pub(crate) fn capture(value: &NpcData) -> Self {
        let NpcData {
            life_points: _,
            ai: _,
        } = value;
        Self {
            life_points: value.life_points,
            ai: PersistedAiActorData::capture(&value.ai),
        }
    }

    pub(crate) fn into_runtime(self) -> NpcData {
        NpcData {
            life_points: self.life_points,
            ai: self.ai.into_runtime(),
        }
    }
}

/// Explicit save-owned projection; process-local state is reconstructed here,
/// independently of raw rollback cloning and the native wire codec.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PersistedActorPc {
    element: PersistedElementData,

    actor: ActorData,

    human: HumanData,

    pc: PcData,
}

impl PersistedActorPc {
    pub(crate) fn capture(value: &ActorPc) -> Self {
        let ActorPc {
            element: _,
            actor: _,
            human: _,
            pc: _,
        } = value;
        Self {
            element: PersistedElementData::capture(&value.element),
            actor: value.actor.clone(),
            human: value.human.clone(),
            pc: value.pc.clone(),
        }
    }

    pub(crate) fn into_runtime(self) -> ActorPc {
        ActorPc {
            element: self.element.into_runtime(),
            actor: self.actor,
            human: self.human,
            pc: self.pc,
        }
    }
}

/// Explicit save-owned projection; process-local state is reconstructed here,
/// independently of raw rollback cloning and the native wire codec.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PersistedActorSoldier {
    element: PersistedElementData,

    actor: ActorData,

    human: HumanData,

    npc: PersistedNpcData,

    soldier: SoldierData,
}

impl PersistedActorSoldier {
    pub(crate) fn capture(value: &ActorSoldier) -> Self {
        let ActorSoldier {
            element: _,
            actor: _,
            human: _,
            npc: _,
            soldier: _,
        } = value;
        Self {
            element: PersistedElementData::capture(&value.element),
            actor: value.actor.clone(),
            human: value.human.clone(),
            npc: PersistedNpcData::capture(&value.npc),
            soldier: value.soldier.clone(),
        }
    }

    pub(crate) fn into_runtime(self) -> ActorSoldier {
        ActorSoldier {
            element: self.element.into_runtime(),
            actor: self.actor,
            human: self.human,
            npc: self.npc.into_runtime(),
            soldier: self.soldier,
        }
    }
}

/// Explicit save-owned projection; process-local state is reconstructed here,
/// independently of raw rollback cloning and the native wire codec.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PersistedActorCivilian {
    element: PersistedElementData,

    actor: ActorData,

    human: HumanData,

    npc: PersistedNpcData,

    civilian: CivilianData,
}

impl PersistedActorCivilian {
    pub(crate) fn capture(value: &ActorCivilian) -> Self {
        let ActorCivilian {
            element: _,
            actor: _,
            human: _,
            npc: _,
            civilian: _,
        } = value;
        Self {
            element: PersistedElementData::capture(&value.element),
            actor: value.actor.clone(),
            human: value.human.clone(),
            npc: PersistedNpcData::capture(&value.npc),
            civilian: value.civilian.clone(),
        }
    }

    pub(crate) fn into_runtime(self) -> ActorCivilian {
        ActorCivilian {
            element: self.element.into_runtime(),
            actor: self.actor,
            human: self.human,
            npc: self.npc.into_runtime(),
            civilian: self.civilian,
        }
    }
}

/// Explicit save-owned projection; process-local state is reconstructed here,
/// independently of raw rollback cloning and the native wire codec.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PersistedElementFx {
    element: PersistedElementData,

    fx: FxData,
}

impl PersistedElementFx {
    pub(crate) fn capture(value: &ElementFx) -> Self {
        let ElementFx { element: _, fx: _ } = value;
        Self {
            element: PersistedElementData::capture(&value.element),
            fx: value.fx.clone(),
        }
    }

    pub(crate) fn into_runtime(self) -> ElementFx {
        ElementFx {
            element: self.element.into_runtime(),
            fx: self.fx,
        }
    }
}

/// Explicit save-owned projection; process-local state is reconstructed here,
/// independently of raw rollback cloning and the native wire codec.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PersistedElementTarget {
    element: PersistedElementData,

    fx: FxData,

    target: TargetData,
}

impl PersistedElementTarget {
    pub(crate) fn capture(value: &ElementTarget) -> Self {
        let ElementTarget {
            element: _,
            fx: _,
            target: _,
        } = value;
        Self {
            element: PersistedElementData::capture(&value.element),
            fx: value.fx.clone(),
            target: value.target.clone(),
        }
    }

    pub(crate) fn into_runtime(self) -> ElementTarget {
        ElementTarget {
            element: self.element.into_runtime(),
            fx: self.fx,
            target: self.target,
        }
    }
}

/// Explicit save-owned projection; process-local state is reconstructed here,
/// independently of raw rollback cloning and the native wire codec.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PersistedElementBonus {
    element: PersistedElementData,

    object: ObjectData,
}

impl PersistedElementBonus {
    pub(crate) fn capture(value: &ElementBonus) -> Self {
        let ElementBonus {
            element: _,
            object: _,
        } = value;
        Self {
            element: PersistedElementData::capture(&value.element),
            object: value.object.clone(),
        }
    }

    pub(crate) fn into_runtime(self) -> ElementBonus {
        ElementBonus {
            element: self.element.into_runtime(),
            object: self.object,
        }
    }
}

/// Explicit save-owned projection; process-local state is reconstructed here,
/// independently of raw rollback cloning and the native wire codec.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PersistedElementScroll {
    element: PersistedElementData,

    object: ObjectData,

    presence: [bool; 3],

    tutorial: bool,

    script_class: String,

    script_hourglass_timeout: u32,
}

impl PersistedElementScroll {
    pub(crate) fn capture(value: &ElementScroll) -> Self {
        let ElementScroll {
            element: _,
            object: _,
            presence: _,
            tutorial: _,
            script_class: _,
            script_hourglass_timeout: _,
        } = value;
        Self {
            element: PersistedElementData::capture(&value.element),
            object: value.object.clone(),
            presence: value.presence,
            tutorial: value.tutorial,
            script_class: value.script_class.clone(),
            script_hourglass_timeout: value.script_hourglass_timeout,
        }
    }

    pub(crate) fn into_runtime(self) -> ElementScroll {
        ElementScroll {
            element: self.element.into_runtime(),
            object: self.object,
            presence: self.presence,
            tutorial: self.tutorial,
            script_class: self.script_class,
            script_hourglass_timeout: self.script_hourglass_timeout,
        }
    }
}

/// Explicit save-owned projection; process-local state is reconstructed here,
/// independently of raw rollback cloning and the native wire codec.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PersistedElementProjectile {
    element: PersistedElementData,

    object: ObjectData,

    projectile: ProjectileData,
}

impl PersistedElementProjectile {
    pub(crate) fn capture(value: &ElementProjectile) -> Self {
        let ElementProjectile {
            element: _,
            object: _,
            projectile: _,
        } = value;
        Self {
            element: PersistedElementData::capture(&value.element),
            object: value.object.clone(),
            projectile: value.projectile.clone(),
        }
    }

    pub(crate) fn into_runtime(self) -> ElementProjectile {
        ElementProjectile {
            element: self.element.into_runtime(),
            object: self.object,
            projectile: self.projectile,
        }
    }
}

/// Explicit save-owned projection; process-local state is reconstructed here,
/// independently of raw rollback cloning and the native wire codec.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PersistedElementNet {
    element: PersistedElementData,

    object: ObjectData,

    projectile: ProjectileData,

    net: NetData,
}

impl PersistedElementNet {
    pub(crate) fn capture(value: &ElementNet) -> Self {
        let ElementNet {
            element: _,
            object: _,
            projectile: _,
            net: _,
        } = value;
        Self {
            element: PersistedElementData::capture(&value.element),
            object: value.object.clone(),
            projectile: value.projectile.clone(),
            net: value.net.clone(),
        }
    }

    pub(crate) fn into_runtime(self) -> ElementNet {
        ElementNet {
            element: self.element.into_runtime(),
            object: self.object,
            projectile: self.projectile,
            net: self.net,
        }
    }
}
