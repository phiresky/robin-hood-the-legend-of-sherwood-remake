//! Original v48 post-grid hiking-guide and trajectory sections.
//!
//! `RHHikingGuide::Serialize` writes no path or waypoint counts. It walks the
//! mission-created paths, validates every `RHWaypoint`, and conditionally
//! serializes VM members for script waypoints. Consequently this grammar
//! requires the exact mission waypoint topology and compiled script schema.
//!
//! The following `RHElementProjectile` is an engine-owned helper, not an
//! element from the phase-one envelope. Historical Original builds left its
//! inherited class ID uninitialized. Its reader therefore preserves the raw
//! ID and deliberately does not use normal class dispatch.

use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyReader, LegacyResult};

use super::LegacySaveAbiProfile;
use super::payload_base::{
    LegacyBoundingBox2, LegacyElementRef, LegacyOpaquePointer32, LegacyPayloadLimits, LegacyPoint2,
    LegacyPoint3, LegacyPositionPayload, LegacySectorRef, LegacySpritePayload, read_element_ref,
    read_sector_ref, read_signed_ref,
};
use super::payload_nonactors::LegacyRepulsivePointPayload;
use super::payload_objects::LegacyTrajectoryPoint;
use super::payload_vm::{LegacyVmMemberDecoder, LegacyVmMemberSection};

const FINGERPRINT_HIKING_GUIDE: [u8; 16] = hex16("9999481162bbcc8461c8d924d0ec24cb");
const FINGERPRINT_WAYPOINT: [u8; 16] = hex16("fdf47609dbe10dab3ebc801e5ca5286a");
const FINGERPRINT_PROJECTILE: [u8; 16] = hex16("6933dbbc6be435f3249e7cf1c994fb2d");
const FINGERPRINT_OBJECT: [u8; 16] = hex16("90062155c12beef1e93d3c32cb21776f");
const FINGERPRINT_ELEMENT: [u8; 16] = hex16("7730a5b25924f7a72c4926ef69f7700f");
const FINGERPRINT_SPRITE: [u8; 16] = hex16("ef8f9051c70a8eb993b6101ac4210ca5");
const FINGERPRINT_POSITION: [u8; 16] = hex16("f41fe85b168584aa52b8bb352f8b593a");

const fn hex16(value: &str) -> [u8; 16] {
    let bytes = value.as_bytes();
    let mut result = [0; 16];
    let mut index = 0;
    while index < 16 {
        result[index] = (hex_nibble(bytes[index * 2]) << 4) | hex_nibble(bytes[index * 2 + 1]);
        index += 1;
    }
    result
}

const fn hex_nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => panic!("invalid fingerprint hex"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyPostHikingLimits {
    pub hiking_paths: usize,
    pub hiking_waypoints: usize,
    pub trajectory_points: usize,
    pub sprite_animation_replacements: usize,
}

impl Default for LegacyPostHikingLimits {
    fn default() -> Self {
        Self {
            hiking_paths: 65_535,
            hiking_waypoints: 65_535,
            trajectory_points: 65_535,
            sprite_animation_replacements: LegacyPayloadLimits::default()
                .sprite_animation_replacements,
        }
    }
}

/// Mission-authored shape of `RHHikingGuide::marrayHikingPathes`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyHikingGuideTopology {
    pub paths: Vec<LegacyHikingPathTopology>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyHikingPathTopology {
    pub waypoints: Vec<LegacyWaypointTopology>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyWaypointTopology {
    /// `Some` exactly when both global script serialization is enabled and
    /// the mission waypoint's `bCommandIsScript` is true.
    pub script_class: Option<String>,
}

pub trait LegacyHikingGuideDecodeContext {
    fn read_waypoint_script_members(
        &self,
        reader: &mut LegacyReader<'_>,
        path_index: usize,
        waypoint_index: usize,
        script_class: &str,
    ) -> LegacyResult<LegacyVmMemberSection>;
}

impl LegacyHikingGuideDecodeContext for LegacyVmMemberDecoder<'_> {
    fn read_waypoint_script_members(
        &self,
        reader: &mut LegacyReader<'_>,
        _path_index: usize,
        _waypoint_index: usize,
        script_class: &str,
    ) -> LegacyResult<LegacyVmMemberSection> {
        self.read_class_members(reader, script_class)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyHikingGuideState {
    pub start_offset: u64,
    pub paths: Vec<LegacyHikingPathState>,
    pub end_offset: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyHikingPathState {
    pub waypoints: Vec<LegacyWaypointState>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyWaypointState {
    pub start_offset: u64,
    pub script_members: Option<LegacyVmMemberSection>,
    pub end_offset: u64,
}

impl LegacyHikingGuideState {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        topology: &LegacyHikingGuideTopology,
        limits: &LegacyPostHikingLimits,
        context: &dyn LegacyHikingGuideDecodeContext,
    ) -> LegacyResult<Self> {
        reader.scope("hiking_guide", |reader| {
            let start_offset = reader.offset();
            validate_hiking_topology(reader, topology, limits)?;
            reader.read_signature(
                "fingerprint",
                FINGERPRINT_HIKING_GUIDE,
                "MD5(\"RHHikingGuide\")",
            )?;

            let mut paths = Vec::new();
            reserve(reader, &mut paths, topology.paths.len(), "paths")?;
            for (path_index, path) in topology.paths.iter().enumerate() {
                paths.push(reader.scope(format!("paths[{path_index}]"), |reader| {
                    let mut waypoints = Vec::new();
                    reserve(reader, &mut waypoints, path.waypoints.len(), "waypoints")?;
                    for (waypoint_index, waypoint) in path.waypoints.iter().enumerate() {
                        waypoints.push(reader.scope(
                            format!("waypoints[{waypoint_index}]"),
                            |reader| {
                                let start_offset = reader.offset();
                                reader.read_signature(
                                    "fingerprint",
                                    FINGERPRINT_WAYPOINT,
                                    "MD5(\"RHWayPoint\")",
                                )?;
                                let script_members =
                                    if let Some(script_class) = &waypoint.script_class {
                                        Some(reader.scope("script_members", |reader| {
                                            context.read_waypoint_script_members(
                                                reader,
                                                path_index,
                                                waypoint_index,
                                                script_class,
                                            )
                                        })?)
                                    } else {
                                        None
                                    };
                                Ok(LegacyWaypointState {
                                    start_offset,
                                    script_members,
                                    end_offset: reader.offset(),
                                })
                            },
                        )?);
                    }
                    Ok(LegacyHikingPathState { waypoints })
                })?);
            }

            Ok(Self {
                start_offset,
                paths,
                end_offset: reader.offset(),
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyProjectileTrajectorySection {
    pub abi_profile: LegacySaveAbiProfile,
    pub start_offset: u64,
    pub projectile: LegacyStandaloneProjectilePayload,
    pub jumper: LegacyElementRef,
    pub jumped: LegacyElementRef,
    pub end_offset: u64,
}

impl LegacyProjectileTrajectorySection {
    pub fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        limits: &LegacyPostHikingLimits,
    ) -> LegacyResult<Self> {
        reader.scope("projectile_trajectory", |reader| {
            audit_abi(abi_profile);
            let start_offset = reader.offset();
            let projectile = LegacyStandaloneProjectilePayload::read(reader, abi_profile, limits)?;
            let jumper = read_element_ref(reader, "jumper")?;
            let jumped = read_element_ref(reader, "jumped")?;
            Ok(Self {
                abi_profile,
                start_offset,
                projectile,
                jumper,
                jumped,
                end_offset: reader.offset(),
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyStandaloneProjectilePayload {
    pub flying: bool,
    pub dive: bool,
    pub magic_bullet: bool,
    pub frame_count: u16,
    pub trajectory_origin_map: LegacyPoint2,
    pub trajectory_origin_sector_pointer: LegacyOpaquePointer32,
    pub trajectory_origin_level: u16,
    pub trajectory_origin_padding: [u8; 2],
    pub trajectory_origin_sector: LegacySectorRef,
    pub flight_direction: u16,
    pub start: LegacyPoint3,
    pub end: LegacyPoint3,
    pub shooter: LegacyElementRef,
    pub trajectory: Vec<LegacyTrajectoryPoint>,
    pub object: LegacyStandaloneObjectPayload,
}

impl LegacyStandaloneProjectilePayload {
    fn read(
        reader: &mut LegacyReader<'_>,
        abi_profile: LegacySaveAbiProfile,
        limits: &LegacyPostHikingLimits,
    ) -> LegacyResult<Self> {
        reader.scope("projectile", |reader| {
            reader.read_signature(
                "fingerprint",
                FINGERPRINT_PROJECTILE,
                "MD5(\"RHElementProjectile\")",
            )?;
            let flying = reader.read_bool("flying")?;
            let dive = reader.read_bool("dive")?;
            let magic_bullet = reader.read_bool("magic_bullet")?;
            let frame_count = reader.read_u16("frame_count")?;
            let trajectory_origin_map = read_point2(reader, "trajectory_origin_map")?;
            let trajectory_origin_sector_pointer =
                LegacyOpaquePointer32(reader.read_u32("trajectory_origin_sector_pointer")?);
            let trajectory_origin_level = reader.read_u16("trajectory_origin_level")?;
            let mut trajectory_origin_padding = [0; 2];
            reader.read_bytes("trajectory_origin_padding", &mut trajectory_origin_padding)?;
            let trajectory_origin_sector = read_sector_ref(reader, "trajectory_origin_sector")?;
            let flight_direction = reader.read_u16("flight_direction")?;
            let start = read_point3(reader, "start")?;
            let end = read_point3(reader, "end")?;
            let shooter = read_element_ref(reader, "shooter")?;
            let count = reader.read_count_u32("trajectory.count", limits.trajectory_points)?;
            let mut trajectory = Vec::new();
            reserve(reader, &mut trajectory, count, "trajectory")?;
            for index in 0..count {
                trajectory.push(reader.scope(format!("trajectory[{index}]"), |reader| {
                    Ok(LegacyTrajectoryPoint {
                        time: reader.read_u16("time")?,
                        bounce: reader.read_bool("bounce")?,
                        material: reader.read_u32("material")?,
                        position: read_point3(reader, "position")?,
                    })
                })?);
            }
            let object = LegacyStandaloneObjectPayload::read(reader, abi_profile, limits)?;
            Ok(Self {
                flying,
                dive,
                magic_bullet,
                frame_count,
                trajectory_origin_map,
                trajectory_origin_sector_pointer,
                trajectory_origin_level,
                trajectory_origin_padding,
                trajectory_origin_sector,
                flight_direction,
                start,
                end,
                shooter,
                trajectory,
                object,
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyStandaloneObjectPayload {
    pub start_offset: u64,
    pub terminate: bool,
    pub register_number: u16,
    pub quantity: u16,
    pub animation: u32,
    pub object_type: u32,
    pub associated_action: u32,
    pub repulsive_point: LegacyRepulsivePointPayload,
    pub belongs_to_beggar: bool,
    pub taken: bool,
    pub element: LegacyStandaloneElementPayload,
    pub end_offset: u64,
}

impl LegacyStandaloneObjectPayload {
    fn read(
        reader: &mut LegacyReader<'_>,
        _abi_profile: LegacySaveAbiProfile,
        limits: &LegacyPostHikingLimits,
    ) -> LegacyResult<Self> {
        reader.scope("object", |reader| {
            let start_offset = reader.offset();
            reader.read_signature(
                "fingerprint",
                FINGERPRINT_OBJECT,
                "MD5(\"RHElementObject\")",
            )?;
            let terminate = reader.read_bool("terminate")?;
            let register_number = reader.read_u16("register_number")?;
            let quantity = reader.read_u16("quantity")?;
            let animation = reader.read_u32("animation")?;
            let object_type = reader.read_u32("object_type")?;
            let associated_action = reader.read_u32("associated_action")?;
            let repulsive_point = reader.scope("repulsive_point", |reader| {
                // This standalone trajectory helper is still serialized by
                // RHElementObject, whose retail stream uses narrow geometry.
                LegacyRepulsivePointPayload::read(reader, LegacySaveAbiProfile::PortLinuxI386V48)
            })?;
            let belongs_to_beggar = reader.read_bool("belongs_to_beggar")?;
            let taken = reader.read_bool("taken")?;
            let element = reader.scope("element", |reader| {
                LegacyStandaloneElementPayload::read(reader, limits)
            })?;
            Ok(Self {
                start_offset,
                terminate,
                register_number,
                quantity,
                animation,
                object_type,
                associated_action,
                repulsive_point,
                belongs_to_beggar,
                taken,
                element,
                end_offset: reader.offset(),
            })
        })
    }
}

/// Inherited `RHElement` data for the engine-owned helper.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyStandaloneElementPayload {
    pub creation_order: u32,
    pub outline_colors: [u16; 5],
    pub current_outline: u32,
    pub outline_width: u16,
    pub custom_minimap_dot: u16,
    pub active: bool,
    pub position_map_delayed: bool,
    pub position_delayed: bool,
    /// Opaque because pre-fix Original constructors did not initialize it.
    pub raw_class_id: u16,
    pub delayed_map_position: LegacyPoint2,
    pub delayed_position: LegacyPoint3,
    pub in_honolulu: bool,
    pub index_in_elements_list: u16,
    pub blipped: bool,
    pub unreachable: bool,
    pub sprite: LegacySpritePayload,
}

impl LegacyStandaloneElementPayload {
    fn read(reader: &mut LegacyReader<'_>, limits: &LegacyPostHikingLimits) -> LegacyResult<Self> {
        reader.read_signature("fingerprint", FINGERPRINT_ELEMENT, "MD5(\"RHElement\")")?;
        let creation_order = reader.read_u32("creation_order")?;
        let mut outline_colors = [0; 5];
        for (index, color) in outline_colors.iter_mut().enumerate() {
            *color = reader.read_u16(format_args!("outline_colors[{index}]"))?;
        }
        let current_outline = reader.read_u32("current_outline")?;
        let outline_width = reader.read_u16("outline_width")?;
        let custom_minimap_dot = reader.read_u16("custom_minimap_dot")?;
        let active = reader.read_bool("active")?;
        let position_map_delayed = reader.read_bool("position_map_delayed")?;
        let position_delayed = reader.read_bool("position_delayed")?;
        let raw_class_id = reader.read_u16("class_id")?;
        let delayed_map_position = read_point2(reader, "delayed_map_position")?;
        let delayed_position = read_point3(reader, "delayed_position")?;
        let in_honolulu = reader.read_bool("in_honolulu")?;
        let index_in_elements_list = reader.read_u16("index_in_elements_list")?;
        let blipped = reader.read_bool("blipped")?;
        let unreachable = reader.read_bool("unreachable")?;
        let sprite = reader.scope("sprite", |reader| read_sprite(reader, limits))?;
        Ok(Self {
            creation_order,
            outline_colors,
            current_outline,
            outline_width,
            custom_minimap_dot,
            active,
            position_map_delayed,
            position_delayed,
            raw_class_id,
            delayed_map_position,
            delayed_position,
            in_honolulu,
            index_in_elements_list,
            blipped,
            unreachable,
            sprite,
        })
    }
}

fn read_sprite(
    reader: &mut LegacyReader<'_>,
    limits: &LegacyPostHikingLimits,
) -> LegacyResult<LegacySpritePayload> {
    reader.read_signature("fingerprint", FINGERPRINT_SPRITE, "MD5(\"RHSprite\")")?;
    let current_row = reader.read_u16("current_row")?;
    let current_frame = reader.read_u16("current_frame")?;
    let frame_count = reader.read_u16("frame_count")?;
    let current_height = reader.read_u16("current_height")?;
    let current_width = reader.read_u16("current_width")?;
    let last_action = reader.read_u32("last_action")?;
    let already_decompressed = reader.read_bool("already_decompressed")?;
    let alternate_profile = reader.read_bool("alternate_profile")?;
    let masked = reader.read_bool("masked")?;
    let display_order = reader.read_f32("display_order")?;
    let legacy_display_order_dummy = reader.read_i32("legacy_display_order_dummy")?;
    let behind_display_order_reference = reader.read_bool("behind_display_order_reference")?;
    let display_order_reference = read_element_ref(reader, "display_order_reference")?;
    let action_done_frame = reader.read_u16("action_done_frame")?;
    let action_done_counter = reader.read_u16("action_done_counter")?;
    let frame_count_down = reader.read_u16("frame_count_down")?;
    let last_sound_id = reader.read_u16("last_sound_id")?;
    let last_processed_order_id = reader.read_u32("last_processed_order_id")?;
    let bounding_box = read_box2(reader, "bounding_box")?;
    let count = reader.read_count_u32(
        "animation_replacements.count",
        limits.sprite_animation_replacements,
    )?;
    let mut animation_replacements = Vec::new();
    reserve(
        reader,
        &mut animation_replacements,
        count,
        "animation_replacements",
    )?;
    for index in 0..count {
        animation_replacements.push(
            reader.scope(format!("animation_replacements[{index}]"), |reader| {
                Ok((reader.read_u32("from")?, reader.read_u32("to")?))
            })?,
        );
    }
    let position = reader.scope("position", read_position)?;
    Ok(LegacySpritePayload {
        current_row,
        current_frame,
        frame_count,
        current_height,
        current_width,
        last_action,
        already_decompressed,
        alternate_profile,
        masked,
        display_order,
        legacy_display_order_dummy,
        behind_display_order_reference,
        display_order_reference,
        action_done_frame,
        action_done_counter,
        frame_count_down,
        last_sound_id,
        last_processed_order_id,
        bounding_box,
        animation_replacements,
        position,
    })
}

fn read_position(reader: &mut LegacyReader<'_>) -> LegacyResult<LegacyPositionPayload> {
    reader.read_signature(
        "fingerprint",
        FINGERPRINT_POSITION,
        "MD5(\"RHPositionInterface\")",
    )?;
    let computed_position = reader.read_u32("computed_position")?;
    let computed_increment = reader.read_u32("computed_increment")?;
    let material = reader.read_u32("material")?;
    let posture = reader.read_u32("posture")?;
    let old_posture = reader.read_u32("old_posture")?;
    let direction = reader.read_i16("direction")?;
    let direction_goal = reader.read_i16("direction_goal")?;
    let slow_turn_count = reader.read_u8("slow_turn_count")?;
    let layer = reader.read_u16("layer")?;
    let layer_goal = reader.read_u16("layer_goal")?;
    let tolerance = reader.read_f32("tolerance")?;
    let directional_tolerance = reader.read_bool("directional_tolerance")?;
    let accumulate_movement_map = reader.read_bool("accumulate_movement_map")?;
    let anti_collision_on = reader.read_bool("anti_collision_on")?;
    let goal_next_valid = reader.read_bool("goal_next_valid")?;
    let deviated = reader.read_bool("deviated")?;
    let direction_count = reader.read_i8("direction_count")?;
    let door_direction = reader.read_bool("door_direction")?;
    let reversed_movement = reader.read_bool("reversed_movement")?;
    let blocked_count = reader.read_u16("blocked_count")?;
    let radius = reader.read_f32("radius")?;
    let use_emergency_lying_box = reader.read_bool("use_emergency_lying_box")?;
    let sector = read_sector_ref(reader, "sector")?;
    let sector_goal = read_sector_ref(reader, "sector_goal")?;
    let door = read_signed_ref(reader, "door")?;
    let obstacle = read_signed_ref(reader, "obstacle")?;
    let target_element = read_element_ref(reader, "target_element")?;
    Ok(LegacyPositionPayload {
        computed_position,
        computed_increment,
        material,
        posture,
        old_posture,
        direction,
        direction_goal,
        slow_turn_count,
        layer,
        layer_goal,
        tolerance,
        directional_tolerance,
        accumulate_movement_map,
        anti_collision_on,
        goal_next_valid,
        deviated,
        direction_count,
        door_direction,
        reversed_movement,
        blocked_count,
        radius,
        use_emergency_lying_box,
        sector,
        sector_goal,
        door,
        obstacle,
        target_element,
        position: read_point3(reader, "position")?,
        map: read_point2(reader, "map")?,
        sprite: read_point2(reader, "sprite")?,
        old_position: read_point3(reader, "old_position")?,
        old_map: read_point2(reader, "old_map")?,
        old_sprite: read_point2(reader, "old_sprite")?,
        goal_map: read_point2(reader, "goal_map")?,
        goal_next_map: read_point2(reader, "goal_next_map")?,
        goal: read_point3(reader, "goal")?,
        increment: read_point3(reader, "increment")?,
        increment_map: read_point2(reader, "increment_map")?,
        accumulated_movement_map: read_point2(reader, "accumulated_movement_map")?,
        forecasted_movement: read_point3(reader, "forecasted_movement")?,
        move_box_map: read_box2(reader, "move_box_map")?,
        blocked_box: read_box2(reader, "blocked_box")?,
    })
}

fn validate_hiking_topology(
    reader: &mut LegacyReader<'_>,
    topology: &LegacyHikingGuideTopology,
    limits: &LegacyPostHikingLimits,
) -> LegacyResult<()> {
    let offset = reader.offset();
    if topology.paths.len() > limits.hiking_paths {
        return Err(reader.invalid_value(
            offset,
            "topology.paths",
            topology.paths.len(),
            "path count within the caller-supplied limit",
        ));
    }
    let mut waypoint_count = 0_usize;
    for (path_index, path) in topology.paths.iter().enumerate() {
        waypoint_count = waypoint_count
            .checked_add(path.waypoints.len())
            .ok_or_else(|| {
                reader.invalid_value(
                    offset,
                    format!("topology.paths[{path_index}].waypoints"),
                    "usize overflow",
                    "total waypoint count representable by the host",
                )
            })?;
        if waypoint_count > limits.hiking_waypoints {
            return Err(reader.invalid_value(
                offset,
                format!("topology.paths[{path_index}].waypoints"),
                waypoint_count,
                "total waypoint count within the caller-supplied limit",
            ));
        }
        for (waypoint_index, waypoint) in path.waypoints.iter().enumerate() {
            if waypoint.script_class.as_deref() == Some("") {
                return Err(reader.invalid_value(
                    offset,
                    format!(
                        "topology.paths[{path_index}].waypoints[{waypoint_index}].script_class"
                    ),
                    "empty string",
                    "non-empty compiled VM class name",
                ));
            }
        }
    }
    Ok(())
}

fn read_point2(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacyPoint2> {
    reader.scope(field.to_string(), |reader| {
        Ok(LegacyPoint2 {
            x: reader.read_f32("x")?,
            y: reader.read_f32("y")?,
        })
    })
}

fn read_point3(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacyPoint3> {
    reader.scope(field.to_string(), |reader| {
        Ok(LegacyPoint3 {
            x: reader.read_f32("x")?,
            y: reader.read_f32("y")?,
            z: reader.read_f32("z")?,
        })
    })
}

fn read_box2(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacyBoundingBox2> {
    reader.scope(field.to_string(), |reader| {
        Ok(LegacyBoundingBox2 {
            top_left: read_point2(reader, "top_left")?,
            bottom_right: read_point2(reader, "bottom_right")?,
            bounds_are_set: reader.read_bool("bounds_are_set")?,
        })
    })
}

fn reserve<T>(
    reader: &mut LegacyReader<'_>,
    values: &mut Vec<T>,
    count: usize,
    field: impl std::fmt::Display,
) -> LegacyResult<()> {
    let offset = reader.offset();
    values
        .try_reserve_exact(count)
        .map_err(|_| reader.allocation_error(offset, field, count))
}

fn audit_abi(abi_profile: LegacySaveAbiProfile) {
    debug_assert!(abi_profile.is_little_endian());
    debug_assert_eq!(LegacySaveAbiProfile::BOOL_WIDTH, 1);
    debug_assert_eq!(LegacySaveAbiProfile::WORD_WIDTH, 2);
    debug_assert_eq!(LegacySaveAbiProfile::LONG_WIDTH, 4);
    debug_assert_eq!(LegacySaveAbiProfile::ENUM_WIDTH, 4);
    debug_assert_eq!(LegacySaveAbiProfile::FLOAT_WIDTH, 4);
    debug_assert_eq!(LegacySaveAbiProfile::POINTER_PLACEHOLDER_WIDTH, 4);
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;
    use crate::sbfile::{SB_FILE_READ, SbFile};

    struct NoScripts;

    impl LegacyHikingGuideDecodeContext for NoScripts {
        fn read_waypoint_script_members(
            &self,
            reader: &mut LegacyReader<'_>,
            _path_index: usize,
            _waypoint_index: usize,
            script_class: &str,
        ) -> LegacyResult<LegacyVmMemberSection> {
            let offset = reader.offset();
            Err(reader.invalid_value(
                offset,
                "script_class",
                script_class,
                "no scripted waypoint in this test topology",
            ))
        }
    }

    fn with_reader<T>(bytes: &[u8], read: impl FnOnce(&mut LegacyReader<'_>) -> T) -> T {
        let mut fixture = NamedTempFile::new().unwrap();
        fixture.write_all(bytes).unwrap();
        fixture.flush().unwrap();
        let path = fixture.path().to_string_lossy();
        let mut file = SbFile::open(&path, SB_FILE_READ).unwrap();
        read(&mut LegacyReader::new(&mut file))
    }

    fn empty_topology() -> LegacyHikingGuideTopology {
        LegacyHikingGuideTopology { paths: Vec::new() }
    }

    fn push_u16(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_f32(bytes: &mut Vec<u8>, value: f32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_point2(bytes: &mut Vec<u8>) {
        push_f32(bytes, 0.0);
        push_f32(bytes, 0.0);
    }

    fn push_point3(bytes: &mut Vec<u8>) {
        push_point2(bytes);
        push_f32(bytes, 0.0);
    }

    fn push_box2(bytes: &mut Vec<u8>) {
        push_point2(bytes);
        push_point2(bytes);
        bytes.push(0);
    }

    fn push_position(bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&FINGERPRINT_POSITION);
        for _ in 0..5 {
            push_u32(bytes, 0);
        }
        push_u16(bytes, 0);
        push_u16(bytes, 0);
        bytes.push(0);
        push_u16(bytes, 0);
        push_u16(bytes, 0);
        push_f32(bytes, 0.0);
        bytes.extend_from_slice(&[0; 5]);
        bytes.push(0);
        bytes.extend_from_slice(&[0; 2]);
        push_u16(bytes, 0);
        push_f32(bytes, 0.0);
        bytes.push(0);
        push_u16(bytes, u16::MAX);
        push_u16(bytes, u16::MAX);
        push_u16(bytes, u16::MAX);
        push_u16(bytes, u16::MAX);
        push_u32(bytes, u32::MAX);
        push_point3(bytes);
        push_point2(bytes);
        push_point2(bytes);
        push_point3(bytes);
        push_point2(bytes);
        push_point2(bytes);
        push_point2(bytes);
        push_point2(bytes);
        push_point3(bytes);
        push_point3(bytes);
        push_point2(bytes);
        push_point2(bytes);
        push_point3(bytes);
        push_box2(bytes);
        push_box2(bytes);
    }

    fn push_sprite(bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&FINGERPRINT_SPRITE);
        for _ in 0..5 {
            push_u16(bytes, 0);
        }
        push_u32(bytes, 0);
        bytes.extend_from_slice(&[0; 3]);
        push_f32(bytes, 0.0);
        push_u32(bytes, 0);
        bytes.push(0);
        push_u32(bytes, u32::MAX);
        for _ in 0..4 {
            push_u16(bytes, 0);
        }
        push_u32(bytes, 0);
        push_box2(bytes);
        push_u32(bytes, 0);
        push_position(bytes);
    }

    fn push_element(bytes: &mut Vec<u8>, raw_class_id: u16) {
        bytes.extend_from_slice(&FINGERPRINT_ELEMENT);
        push_u32(bytes, 23);
        for _ in 0..5 {
            push_u16(bytes, 0);
        }
        push_u32(bytes, 0);
        push_u16(bytes, 0);
        push_u16(bytes, 0);
        bytes.extend_from_slice(&[0; 3]);
        push_u16(bytes, raw_class_id);
        push_point2(bytes);
        push_point3(bytes);
        bytes.push(0);
        push_u16(bytes, 0);
        bytes.extend_from_slice(&[0; 2]);
        push_sprite(bytes);
    }

    fn push_repulsive_point(bytes: &mut Vec<u8>, abi_profile: LegacySaveAbiProfile) {
        let point_width = match abi_profile {
            LegacySaveAbiProfile::RetailWindowsX86V48 => 16,
            LegacySaveAbiProfile::PortLinuxI386V48 => 8,
        };
        bytes.resize(bytes.len() + point_width, 0);
        bytes.push(0);
        bytes.resize(bytes.len() + point_width * 2, 0);
        for _ in 0..4 {
            push_f32(bytes, 0.0);
        }
        push_u32(bytes, 0);
        bytes.extend_from_slice(&[0; 4]);
    }

    fn minimal_trajectory(abi_profile: LegacySaveAbiProfile, raw_class_id: u16) -> Vec<u8> {
        let mut bytes = FINGERPRINT_PROJECTILE.to_vec();
        bytes.extend_from_slice(&[0; 3]);
        push_u16(&mut bytes, 0);
        push_point2(&mut bytes);
        push_u32(&mut bytes, 0x1234_5678);
        push_u16(&mut bytes, 0);
        bytes.extend_from_slice(&[0xaa, 0x55]);
        push_u16(&mut bytes, u16::MAX);
        push_u16(&mut bytes, 0);
        push_point3(&mut bytes);
        push_point3(&mut bytes);
        push_u32(&mut bytes, u32::MAX);
        push_u32(&mut bytes, 0);

        bytes.extend_from_slice(&FINGERPRINT_OBJECT);
        bytes.push(0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_repulsive_point(&mut bytes, abi_profile);
        bytes.extend_from_slice(&[0; 2]);
        push_element(&mut bytes, raw_class_id);

        push_u32(&mut bytes, 12);
        push_u32(&mut bytes, u32::MAX);
        bytes
    }

    #[test]
    fn hiking_topology_drives_exact_waypoint_count() {
        let topology = LegacyHikingGuideTopology {
            paths: vec![
                LegacyHikingPathTopology {
                    waypoints: vec![LegacyWaypointTopology { script_class: None }],
                },
                LegacyHikingPathTopology {
                    waypoints: vec![
                        LegacyWaypointTopology { script_class: None },
                        LegacyWaypointTopology { script_class: None },
                    ],
                },
            ],
        };
        let mut bytes = FINGERPRINT_HIKING_GUIDE.to_vec();
        for _ in 0..3 {
            bytes.extend_from_slice(&FINGERPRINT_WAYPOINT);
        }
        bytes.extend_from_slice(&0xfeed_beef_u32.to_le_bytes());

        let state = with_reader(&bytes, |reader| {
            let state = LegacyHikingGuideState::read(
                reader,
                &topology,
                &LegacyPostHikingLimits::default(),
                &NoScripts,
            )
            .unwrap();
            assert_eq!(reader.offset(), 64);
            state
        });
        assert_eq!(state.start_offset, 0);
        assert_eq!(state.end_offset, 64);
        assert_eq!(state.paths[1].waypoints.len(), 2);
    }

    #[test]
    fn empty_hiking_guide_is_abi_independent() {
        for _abi_profile in [
            LegacySaveAbiProfile::PortLinuxI386V48,
            LegacySaveAbiProfile::RetailWindowsX86V48,
        ] {
            let state = with_reader(&FINGERPRINT_HIKING_GUIDE, |reader| {
                LegacyHikingGuideState::read(
                    reader,
                    &empty_topology(),
                    &LegacyPostHikingLimits::default(),
                    &NoScripts,
                )
            })
            .unwrap();
            assert_eq!(state.end_offset, 16);
        }
    }

    #[test]
    fn standalone_trajectory_decodes_both_abis_and_preserves_opaque_class() {
        for abi_profile in [
            LegacySaveAbiProfile::PortLinuxI386V48,
            LegacySaveAbiProfile::RetailWindowsX86V48,
        ] {
            let bytes = minimal_trajectory(abi_profile, 0xdead);
            let state = with_reader(&bytes, |reader| {
                LegacyProjectileTrajectorySection::read(
                    reader,
                    abi_profile,
                    &LegacyPostHikingLimits::default(),
                )
            })
            .unwrap();
            assert_eq!(state.end_offset as usize, bytes.len());
            assert_eq!(state.projectile.object.element.raw_class_id, 0xdead);
            assert_eq!(state.projectile.object.element.creation_order, 23);
            assert_eq!(state.jumper, LegacyElementRef(Some(12)));
            assert_eq!(state.jumped, LegacyElementRef(None));
        }
    }

    #[test]
    fn malformed_waypoint_fingerprint_stops_at_exact_signature() {
        let topology = LegacyHikingGuideTopology {
            paths: vec![LegacyHikingPathTopology {
                waypoints: vec![LegacyWaypointTopology { script_class: None }],
            }],
        };
        let mut bytes = FINGERPRINT_HIKING_GUIDE.to_vec();
        let mut bad = FINGERPRINT_WAYPOINT;
        bad[3] ^= 0xff;
        bytes.extend_from_slice(&bad);
        with_reader(&bytes, |reader| {
            let error = LegacyHikingGuideState::read(
                reader,
                &topology,
                &LegacyPostHikingLimits::default(),
                &NoScripts,
            )
            .unwrap_err();
            assert_eq!(error.offset, 16);
            assert!(error.field.ends_with("waypoints[0].fingerprint"));
            assert_eq!(reader.offset(), 32);
        });
    }

    #[test]
    fn trajectory_count_limit_fails_before_allocation_for_both_abis() {
        for abi_profile in [
            LegacySaveAbiProfile::PortLinuxI386V48,
            LegacySaveAbiProfile::RetailWindowsX86V48,
        ] {
            let mut bytes = FINGERPRINT_PROJECTILE.to_vec();
            bytes.extend_from_slice(&[0; 3]);
            push_u16(&mut bytes, 0);
            push_point2(&mut bytes);
            push_u32(&mut bytes, 0);
            push_u16(&mut bytes, 0);
            bytes.extend_from_slice(&[0; 2]);
            push_u16(&mut bytes, u16::MAX);
            push_u16(&mut bytes, 0);
            push_point3(&mut bytes);
            push_point3(&mut bytes);
            push_u32(&mut bytes, u32::MAX);
            push_u32(&mut bytes, 2);
            let limits = LegacyPostHikingLimits {
                trajectory_points: 1,
                ..LegacyPostHikingLimits::default()
            };

            with_reader(&bytes, |reader| {
                let error = LegacyProjectileTrajectorySection::read(reader, abi_profile, &limits)
                    .unwrap_err();
                assert_eq!(error.offset, 69);
                assert!(error.field.ends_with("trajectory.count"));
                assert_eq!(reader.offset(), 73);
            });
        }
    }

    #[test]
    fn topology_limit_rejection_does_not_consume_stream() {
        let topology = LegacyHikingGuideTopology {
            paths: vec![LegacyHikingPathTopology {
                waypoints: vec![LegacyWaypointTopology { script_class: None }],
            }],
        };
        let limits = LegacyPostHikingLimits {
            hiking_waypoints: 0,
            ..LegacyPostHikingLimits::default()
        };
        with_reader(&[], |reader| {
            let error =
                LegacyHikingGuideState::read(reader, &topology, &limits, &NoScripts).unwrap_err();
            assert!(error.field.contains("topology.paths[0].waypoints"));
            assert_eq!(error.offset, 0);
            assert_eq!(reader.offset(), 0);
        });
    }
}
