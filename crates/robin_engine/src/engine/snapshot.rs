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
    let snapshot = FlatEngineSnapshot::deserialize(deserializer)?;
    Ok(EngineInner {
        mission_domain: snapshot.mission_domain,
        control: snapshot.control,
        ai: snapshot.ai,
        world: snapshot.world,
        script_domains: snapshot.script_domains,
        orders: snapshot.orders,
        scripts: snapshot.scripts,
        players: snapshot.players,
        feedback: snapshot.feedback,
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
