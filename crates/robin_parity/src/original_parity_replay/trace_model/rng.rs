//! Recorded libc RNG draw batches.
use bitcode_parity as bitcode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraceRngBatch {
    pub(crate) first_index: usize,
    pub(crate) values: Vec<u32>,
    pub(crate) callsite_offsets: Vec<u32>,
    pub(crate) main_thread: Vec<bool>,
    pub(crate) domains: Vec<TraceRngDomain>,
}

impl TraceRngBatch {
    pub(crate) fn validate(&self) {
        let draw_count = self.values.len();
        assert_eq!(
            draw_count,
            self.callsite_offsets.len(),
            "RNG callsite stream has a different length than its values"
        );
        assert_eq!(
            draw_count,
            self.main_thread.len(),
            "RNG thread-origin stream has a different length than its values"
        );
        assert_eq!(
            draw_count,
            self.domains.len(),
            "RNG domain stream has a different length than its values"
        );
        for (index, (domain, main_thread)) in self
            .domains
            .iter()
            .copied()
            .zip(self.main_thread.iter().copied())
            .enumerate()
        {
            assert!(
                domain != TraceRngDomain::Simulation || main_thread,
                "simulation RNG draw {} (global index {}) occurred off the main thread; its global order is not deterministically replayable",
                index,
                self.first_index + index,
            );
        }
    }

    pub(crate) fn gameplay_draw_count(&self) -> usize {
        self.validate();
        self.domains
            .iter()
            .filter(|domain| **domain == TraceRngDomain::Simulation)
            .count()
    }

    pub(crate) fn gameplay_callsite_offsets(&self) -> Vec<u32> {
        self.validate();
        self.callsite_offsets
            .iter()
            .copied()
            .zip(self.domains.iter().copied())
            .filter_map(|(offset, domain)| (domain == TraceRngDomain::Simulation).then_some(offset))
            .collect()
    }

    pub(crate) fn gameplay_values(&self) -> Vec<u32> {
        self.validate();
        self.values
            .iter()
            .copied()
            .zip(self.domains.iter().copied())
            .filter_map(|(value, domain)| (domain == TraceRngDomain::Simulation).then_some(value))
            .collect()
    }
}

#[derive(
    Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, bitcode::Encode, bitcode::Decode,
)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TraceRngDomain {
    Simulation,
    Audio,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraceRngPrefix {
    #[allow(dead_code)]
    pub(crate) r#type: String,
    pub(crate) draws: TraceRngBatch,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraceRngOnly {
    #[serde(rename = "type")]
    pub(crate) record_type: String,
    pub(crate) draws: TraceRngBatch,
    pub(crate) final_frame: u64,
    pub(crate) frame_count: u64,
}
