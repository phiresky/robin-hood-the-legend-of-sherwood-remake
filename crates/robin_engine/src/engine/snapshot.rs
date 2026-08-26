//! Serialization boundary for [`EngineInner`].
//!
//! The snapshot follows the engine's current cohesive state owners. Rollback,
//! state hashing, and multiplayer snapshots therefore all describe the same
//! runtime ownership model instead of maintaining a second historical field
//! layout.

use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeStruct};

use super::{
    EngineInner,
    state::{
        AiRuntime, FeedbackRuntime, MissionDomain, OrderRuntime, PlayerRuntime, ScriptDomains,
        ScriptRuntime, SimulationControl, WorldState,
    },
};

/// Owned form of the current nested [`EngineInner`] snapshot.
///
/// This remains separate from `EngineInner` so deserialization can construct
/// every owner before exposing a live engine value.
#[derive(Deserialize)]
struct FlatEngineSnapshot {
    mission_domain: MissionDomain,
    control: SimulationControl,
    ai: AiRuntime,
    world: WorldState,
    script_domains: ScriptDomains,
    orders: OrderRuntime,
    scripts: ScriptRuntime,
    players: PlayerRuntime,
    feedback: FeedbackRuntime,
}

/// Native multiplayer snapshot envelope.
///
/// Each cohesive state owner is encoded independently. A single derived
/// encoder for the complete engine embeds every child encoder inline and is
/// large enough to overflow an ordinary Rust thread stack before it writes a
/// byte. Byte chunks keep the outer codec small while retaining native
/// bitcode for every authoritative domain.
#[derive(bitcode::Encode, bitcode::Decode)]
struct NativeEngineSnapshot {
    mission_domain: Vec<u8>,
    control: Vec<u8>,
    ai: Vec<u8>,
    world: Vec<u8>,
    script_domains: Vec<u8>,
    orders: Vec<u8>,
    scripts: Vec<u8>,
    players: Vec<u8>,
    feedback: Vec<u8>,
}

#[derive(bitcode::Encode, bitcode::Decode)]
enum NativeEntityKind {
    Pc,
    Soldier,
    Civilian,
    Fx,
    Target,
    Bonus,
    Scroll,
    Projectile,
    Net,
}

#[derive(bitcode::Encode, bitcode::Decode)]
struct NativeEntitySnapshot {
    kind: NativeEntityKind,
    payload: Vec<u8>,
}

#[derive(bitcode::Encode, bitcode::Decode)]
struct NativeWorldSnapshot {
    entities: Vec<u8>,
    pc_ids: Vec<u8>,
    original_pc_registry_ids: Vec<u8>,
    fast_grid: Vec<u8>,
    pathfinder: Vec<u8>,
    weather: Vec<u8>,
    shield: Vec<u8>,
    dynamic_sight_obstacles: Vec<u8>,
    static_sight_obstacle_active: Vec<u8>,
    mobile_elements: Vec<u8>,
    original_creation_order_by_entity: Vec<u8>,
    next_original_creation_order: u32,
    original_repulsive_point_counter: u32,
}

impl FlatEngineSnapshot {
    fn into_engine_inner(self) -> EngineInner {
        let mut feedback = self.feedback;
        let default_camera = super::CameraState::default();
        feedback.cutscene_camera.old_view_position = default_camera.old_view_position;
        feedback.cutscene_camera.old_zoom_factor = default_camera.old_zoom_factor;

        EngineInner {
            mission_domain: self.mission_domain,
            control: self.control,
            ai: self.ai,
            world: self.world,
            script_domains: self.script_domains,
            orders: self.orders,
            scripts: self.scripts,
            players: self.players,
            feedback,
        }
    }
}

impl Serialize for EngineInner {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut snapshot = serializer.serialize_struct("EngineInner", 9)?;
        snapshot.serialize_field("mission_domain", &self.mission_domain)?;
        snapshot.serialize_field("control", &self.control)?;
        snapshot.serialize_field("ai", &self.ai)?;
        snapshot.serialize_field("world", &self.world)?;
        snapshot.serialize_field("script_domains", &self.script_domains)?;
        snapshot.serialize_field("orders", &self.orders)?;
        snapshot.serialize_field("scripts", &self.scripts)?;
        snapshot.serialize_field("players", &self.players)?;
        snapshot.serialize_field("feedback", &self.feedback)?;
        snapshot.end()
    }
}

pub(super) fn deserialize_engine_inner<'de, D>(deserializer: D) -> Result<EngineInner, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(FlatEngineSnapshot::deserialize(deserializer)?.into_engine_inner())
}

/// Encode the native snapshot without constructing the monolithic derived
/// `EngineInner` encoder on the caller's stack.
pub(super) fn encode_native_engine_inner(inner: &EngineInner) -> Vec<u8> {
    let mission_domain = bitcode::encode(&inner.mission_domain);
    let control = bitcode::encode(&inner.control);
    let ai = bitcode::encode(&inner.ai);
    let world = encode_native_world(&inner.world);
    let script_domains = bitcode::encode(&inner.script_domains);
    let orders = bitcode::encode(&inner.orders);
    let scripts = bitcode::encode(&inner.scripts);
    let players = bitcode::encode(&inner.players);
    let feedback = bitcode::encode(&inner.feedback);
    bitcode::encode(&NativeEngineSnapshot {
        mission_domain,
        control,
        ai,
        world,
        script_domains,
        orders,
        scripts,
        players,
        feedback,
    })
}

fn encode_native_world(world: &WorldState) -> Vec<u8> {
    let entities = world
        .entities
        .snapshot_slots()
        .iter()
        .map(|slot| slot.as_ref().map(encode_native_entity))
        .collect::<Vec<_>>();
    bitcode::encode(&NativeWorldSnapshot {
        entities: bitcode::encode(&entities),
        pc_ids: bitcode::encode(&world.pc_ids),
        original_pc_registry_ids: bitcode::encode(&world.original_pc_registry_ids),
        fast_grid: bitcode::encode(&world.fast_grid),
        pathfinder: bitcode::encode(&world.pathfinder),
        weather: bitcode::encode(&world.weather),
        shield: bitcode::encode(&world.shield),
        dynamic_sight_obstacles: bitcode::encode(&world.dynamic_sight_obstacles),
        static_sight_obstacle_active: bitcode::encode(&world.static_sight_obstacle_active),
        mobile_elements: bitcode::encode(&world.mobile_elements),
        original_creation_order_by_entity: bitcode::encode(
            &world.original_creation_order_by_entity,
        ),
        next_original_creation_order: world.next_original_creation_order,
        original_repulsive_point_counter: world.original_repulsive_point_counter,
    })
}

fn encode_native_entity(entity: &crate::element::Entity) -> NativeEntitySnapshot {
    use crate::element::Entity;
    let (kind, payload) = match entity {
        Entity::Pc(value) => (NativeEntityKind::Pc, bitcode::encode(value)),
        Entity::Soldier(value) => (NativeEntityKind::Soldier, bitcode::encode(value)),
        Entity::Civilian(value) => (NativeEntityKind::Civilian, bitcode::encode(value)),
        Entity::Fx(value) => (NativeEntityKind::Fx, bitcode::encode(value)),
        Entity::Target(value) => (NativeEntityKind::Target, bitcode::encode(value)),
        Entity::Bonus(value) => (NativeEntityKind::Bonus, bitcode::encode(value)),
        Entity::Scroll(value) => (NativeEntityKind::Scroll, bitcode::encode(value)),
        Entity::Projectile(value) => (NativeEntityKind::Projectile, bitcode::encode(value)),
        Entity::Net(value) => (NativeEntityKind::Net, bitcode::encode(value)),
    };
    NativeEntitySnapshot { kind, payload }
}

/// Decode the chunked native wire layout without implementing
/// `bitcode::Decode` for the public read-only projection.
pub(super) fn decode_native_engine_inner(bytes: &[u8]) -> Result<EngineInner, bitcode::Error> {
    let snapshot = bitcode::decode::<NativeEngineSnapshot>(bytes)?;
    Ok(FlatEngineSnapshot {
        mission_domain: bitcode::decode(&snapshot.mission_domain)?,
        control: bitcode::decode(&snapshot.control)?,
        ai: bitcode::decode(&snapshot.ai)?,
        world: decode_native_world(&snapshot.world)?,
        script_domains: bitcode::decode(&snapshot.script_domains)?,
        orders: bitcode::decode(&snapshot.orders)?,
        scripts: bitcode::decode(&snapshot.scripts)?,
        players: bitcode::decode(&snapshot.players)?,
        feedback: bitcode::decode(&snapshot.feedback)?,
    }
    .into_engine_inner())
}

fn decode_native_world(bytes: &[u8]) -> Result<WorldState, bitcode::Error> {
    let snapshot = bitcode::decode::<NativeWorldSnapshot>(bytes)?;
    let entities = bitcode::decode::<Vec<Option<NativeEntitySnapshot>>>(&snapshot.entities)?
        .into_iter()
        .map(|slot| slot.map(decode_native_entity).transpose())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(WorldState {
        entities: crate::entities::Entities::from_snapshot_slots(entities),
        pc_ids: bitcode::decode(&snapshot.pc_ids)?,
        original_pc_registry_ids: bitcode::decode(&snapshot.original_pc_registry_ids)?,
        fast_grid: bitcode::decode(&snapshot.fast_grid)?,
        pathfinder: bitcode::decode(&snapshot.pathfinder)?,
        weather: bitcode::decode(&snapshot.weather)?,
        shield: bitcode::decode(&snapshot.shield)?,
        dynamic_sight_obstacles: bitcode::decode(&snapshot.dynamic_sight_obstacles)?,
        static_sight_obstacle_active: bitcode::decode(&snapshot.static_sight_obstacle_active)?,
        mobile_elements: bitcode::decode(&snapshot.mobile_elements)?,
        original_creation_order_by_entity: bitcode::decode(
            &snapshot.original_creation_order_by_entity,
        )?,
        next_original_creation_order: snapshot.next_original_creation_order,
        original_repulsive_point_counter: snapshot.original_repulsive_point_counter,
    })
}

fn decode_native_entity(
    snapshot: NativeEntitySnapshot,
) -> Result<crate::element::Entity, bitcode::Error> {
    use crate::element::Entity;
    let payload = snapshot.payload.as_slice();
    Ok(match snapshot.kind {
        NativeEntityKind::Pc => Entity::Pc(bitcode::decode(payload)?),
        NativeEntityKind::Soldier => Entity::Soldier(bitcode::decode(payload)?),
        NativeEntityKind::Civilian => Entity::Civilian(bitcode::decode(payload)?),
        NativeEntityKind::Fx => Entity::Fx(bitcode::decode(payload)?),
        NativeEntityKind::Target => Entity::Target(bitcode::decode(payload)?),
        NativeEntityKind::Bonus => Entity::Bonus(bitcode::decode(payload)?),
        NativeEntityKind::Scroll => Entity::Scroll(bitcode::decode(payload)?),
        NativeEntityKind::Projectile => Entity::Projectile(bitcode::decode(payload)?),
        NativeEntityKind::Net => Entity::Net(bitcode::decode(payload)?),
    })
}

// Low-level unit tests intentionally exercise the wire representation without
// going through the cross-crate facade. This implementation is absent from
// normal library builds, where only `Engine` may own a decoded snapshot.
#[cfg(test)]
impl<'de> Deserialize<'de> for EngineInner {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_engine_inner(deserializer)
    }
}
