//! Entity identities and bit-exact float/point scalars shared by all trace records.
use bitcode_parity as bitcode;
use robin_engine::coordinates::{MapPoint, WorldPoint3D};
use robin_engine::element::{EntityId, EntityIdKind};
use serde::{Deserialize, Serialize};

#[derive(
    Debug,
    Clone,
    Copy,
    Deserialize,
    Serialize,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    bitcode::Encode,
    bitcode::Decode,
)]
pub(crate) struct TraceEntityId {
    pub(crate) kind: TraceEntityKind,
    pub(crate) index: u32,
}

#[derive(
    Debug,
    Clone,
    Copy,
    Deserialize,
    Serialize,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TraceEntityKind {
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

impl From<TraceEntityId> for EntityId {
    fn from(value: TraceEntityId) -> Self {
        let kind = match value.kind {
            TraceEntityKind::Pc => EntityIdKind::Pc,
            TraceEntityKind::Soldier => EntityIdKind::Soldier,
            TraceEntityKind::Civilian => EntityIdKind::Civilian,
            TraceEntityKind::Fx => EntityIdKind::Fx,
            TraceEntityKind::Target => EntityIdKind::Target,
            TraceEntityKind::Bonus => EntityIdKind::Bonus,
            TraceEntityKind::Scroll => EntityIdKind::Scroll,
            TraceEntityKind::Projectile => EntityIdKind::Projectile,
            TraceEntityKind::Net => EntityIdKind::Net,
        };
        EntityId::new(value.index, kind)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceFloat {
    pub(crate) bits: u32,
}

impl TraceFloat {
    pub(crate) fn value(self) -> f32 {
        f32::from_bits(self.bits)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TracePoint {
    pub(crate) x: TraceFloat,
    pub(crate) y: TraceFloat,
}

impl From<TracePoint> for MapPoint {
    fn from(value: TracePoint) -> Self {
        Self::new(value.x.value(), value.y.value())
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TracePoint3 {
    pub(crate) x: TraceFloat,
    pub(crate) y: TraceFloat,
    pub(crate) z: TraceFloat,
}

impl From<TracePoint3> for WorldPoint3D {
    fn from(value: TracePoint3) -> Self {
        Self::new(value.x.value(), value.y.value(), value.z.value())
    }
}
