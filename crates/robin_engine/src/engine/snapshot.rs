//! Serialization boundary for [`EngineInner`].
//!
//! Persisted mission state follows the engine's cohesive owners but reconstructs
//! their runtime-only continuations and attachments. Native snapshots and raw
//! rollback retain their separate, historical runtime-state boundaries.

use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeStruct};

use super::{
    EngineInner,
    state::{
        AiRuntime, FeedbackRuntime, MissionDomain, OrderRuntime, PlayerRuntime, ScriptDomains,
        ScriptRuntime, SimulationControl, WorldState,
    },
};

const NATIVE_SNAPSHOT_MAGIC: &[u8; 4] = b"RHNS";
const NATIVE_SNAPSHOT_VERSION: u32 = 4;
const NATIVE_SNAPSHOT_HEADER_BYTES: usize = 8;

/// Native codec form of the current nested [`EngineInner`] snapshot.
///
/// This remains separate from the persisted-save projection because native
/// rollback has intentionally different survival semantics.
#[derive(Deserialize, bitcode::Decode)]
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

/// Owned persisted mission state, deliberately distinct from a raw rollback
/// clone. Canonical domains selectively copy surviving state and reconstruct
/// process-local resources without executing a serialization codec.
/// Its serde representation is the existing nine-domain engine save layout.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename = "EngineInner")]
pub struct PersistedEngineState {
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

impl PersistedEngineState {
    /// Serialize borrowed domains, avoiding an additional full-world copy for
    /// ordinary disk writes and diagnostic serialization. Runtime owners
    /// serialize their surviving fields directly.
    fn serialize_runtime<S: Serializer>(
        inner: &EngineInner,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let EngineInner {
            mission_domain,
            control,
            ai,
            world,
            script_domains,
            orders,
            scripts,
            players,
            feedback,
        } = inner;
        let mut snapshot = serializer.serialize_struct("EngineInner", 9)?;
        snapshot.serialize_field("mission_domain", mission_domain)?;
        snapshot.serialize_field("control", control)?;
        snapshot.serialize_field("ai", ai)?;
        snapshot.serialize_field("world", world)?;
        snapshot.serialize_field("script_domains", script_domains)?;
        snapshot.serialize_field("orders", orders)?;
        snapshot.serialize_field("scripts", scripts)?;
        snapshot.serialize_field("players", players)?;
        snapshot.serialize_field("feedback", feedback)?;
        snapshot.end()
    }

    pub(super) fn capture(inner: &EngineInner) -> Result<Self, String> {
        let EngineInner {
            mission_domain,
            control,
            ai,
            world,
            script_domains,
            orders,
            scripts,
            players,
            feedback,
        } = inner;
        mission_domain.campaign.validate_history_schema()?;
        scripts.spellforge.validate_snapshot()?;
        let persisted = Self {
            mission_domain: mission_domain.clone(),
            control: control.persisted_clone()?,
            ai: ai.persisted_clone(),
            world: world.persisted_clone(),
            script_domains: script_domains.persisted_clone(),
            orders: orders.persisted_clone(),
            scripts: scripts.persisted_clone()?,
            players: players.persisted_clone(),
            feedback: feedback.persisted_clone(),
        };
        robin_util::persistence_validation::validate(&persisted)
            .map_err(|error| error.to_string())?;
        Ok(persisted)
    }

    pub(super) fn into_engine_inner(self) -> EngineInner {
        EngineInner {
            mission_domain: self.mission_domain,
            control: self.control,
            ai: self.ai,
            world: self.world,
            script_domains: self.script_domains,
            orders: self.orders,
            scripts: self.scripts,
            players: self.players,
            feedback: self.feedback,
        }
    }
}

impl FlatEngineSnapshot {
    fn into_engine_inner(self) -> EngineInner {
        EngineInner {
            mission_domain: self.mission_domain,
            control: self.control,
            ai: self.ai,
            world: self.world,
            script_domains: self.script_domains,
            orders: self.orders,
            scripts: self.scripts,
            players: self.players,
            feedback: self.feedback,
        }
    }
}

pub(super) fn serialize_engine_inner<S: Serializer>(
    inner: &EngineInner,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    PersistedEngineState::serialize_runtime(inner, serializer)
}

// Low-level fixtures test codecs directly. Production read-only projections
// deliberately cannot serialize themselves into a restorable Engine snapshot.
#[cfg(test)]
impl Serialize for EngineInner {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_engine_inner(self, serializer)
    }
}

pub(super) fn deserialize_engine_inner<'de, D>(deserializer: D) -> Result<EngineInner, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(PersistedEngineState::deserialize(deserializer)?.into_engine_inner())
}

pub(super) fn encode_native_engine_inner(inner: &EngineInner) -> Vec<u8> {
    let payload = bitcode::encode(inner);
    let mut bytes = Vec::with_capacity(NATIVE_SNAPSHOT_HEADER_BYTES + payload.len());
    bytes.extend_from_slice(NATIVE_SNAPSHOT_MAGIC);
    bytes.extend_from_slice(&NATIVE_SNAPSHOT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&payload);
    bytes
}

/// Decode the direct native wire layout without implementing
/// `bitcode::Decode` for the public read-only projection.
pub(super) fn decode_native_engine_inner(bytes: &[u8]) -> Result<EngineInner, String> {
    let header = bytes
        .get(..NATIVE_SNAPSHOT_HEADER_BYTES)
        .ok_or("native snapshot has a truncated header")?;
    if &header[..4] != NATIVE_SNAPSHOT_MAGIC {
        return Err("native snapshot has invalid magic".into());
    }
    let version = u32::from_le_bytes(header[4..8].try_into().expect("fixed version field"));
    if version != NATIVE_SNAPSHOT_VERSION {
        return Err(format!(
            "unsupported native snapshot version {version}; expected {NATIVE_SNAPSHOT_VERSION}"
        ));
    }
    bitcode::decode::<FlatEngineSnapshot>(&bytes[NATIVE_SNAPSHOT_HEADER_BYTES..])
        .map(FlatEngineSnapshot::into_engine_inner)
        .map_err(|error| error.to_string())
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
