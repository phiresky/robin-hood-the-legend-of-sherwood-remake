//! Motion-grid lines and their per-frame activity changes.
use super::scalar::TracePoint;
use bitcode_parity as bitcode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceMotionGrid {
    pub(crate) layers: Vec<TraceMotionLayer>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceMotionLayer {
    pub(crate) layer: u16,
    pub(crate) lines: Vec<TraceMotionLine>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceMotionLine {
    pub(crate) index: u16,
    pub(crate) a: TracePoint,
    pub(crate) b: TracePoint,
    pub(crate) type_mask: i32,
    pub(crate) associated_sector: i16,
    pub(crate) active: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceMotionLineChange {
    pub(crate) layer: u16,
    pub(crate) index: u16,
    pub(crate) active: bool,
}
