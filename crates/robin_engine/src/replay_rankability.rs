//! Ranked-run input provenance recorded alongside deterministic replay input.
//!
//! This is evidence emitted by the official recorder, not remote attestation.
//! A clean value proves only that the recorder observed no known ineligible
//! input path. UI-shaped commands synthesized by a modified client remain
//! indistinguishable from ordinary UI commands.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// A known input path that makes a replay ineligible for ranked submission.
///
/// Variant names and [`Self::stable_code`] values are a public storage/API
/// contract. Additive changes require a replay and run-protocol schema bump;
/// unknown values must never deserialize as rankable.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(rename_all = "snake_case")]
pub enum InputTaintKind {
    HttpPlayerCommand,
    HttpSimulationStep,
    HttpStateMutation,
    ConsoleCommand,
    CheatCommand,
    HeadlessAutomation,
    ReplayPlayback,
    StateLoad,
    MissionRestart,
    DebugInputInjection,
}

impl InputTaintKind {
    pub const ALL: [Self; 10] = [
        Self::HttpPlayerCommand,
        Self::HttpSimulationStep,
        Self::HttpStateMutation,
        Self::ConsoleCommand,
        Self::CheatCommand,
        Self::HeadlessAutomation,
        Self::ReplayPlayback,
        Self::StateLoad,
        Self::MissionRestart,
        Self::DebugInputInjection,
    ];

    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::HttpPlayerCommand => "http_player_command",
            Self::HttpSimulationStep => "http_simulation_step",
            Self::HttpStateMutation => "http_state_mutation",
            Self::ConsoleCommand => "console_command",
            Self::CheatCommand => "cheat_command",
            Self::HeadlessAutomation => "headless_automation",
            Self::ReplayPlayback => "replay_playback",
            Self::StateLoad => "state_load",
            Self::MissionRestart => "mission_restart",
            Self::DebugInputInjection => "debug_input_injection",
        }
    }
}

/// First observed frame for one unique taint kind.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
pub struct InputTaint {
    pub kind: InputTaintKind,
    pub first_frame: u32,
}

/// Cumulative, canonical run-level evidence.
///
/// Entries are sorted by `kind` and each kind occurs once. `first_frame` is
/// data, not part of the canonical ordering, so engine and wire protocol
/// representations have one identical order.
/// Fields stay private so live producers can only add evidence. Deserialized
/// hostile input must call [`Self::validate`] before it is accepted.
#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReplayRankability {
    /// Current canonical recorder evidence. An empty list is rankable.
    Recorded { taints: Vec<InputTaint> },
}

impl ReplayRankability {
    pub const fn rankable() -> Self {
        Self::Recorded { taints: Vec::new() }
    }

    pub fn taints(&self) -> &[InputTaint] {
        match self {
            Self::Recorded { taints } => taints,
        }
    }

    /// Pure client/verifier eligibility verdict requiring no engine startup.
    pub fn verdict(&self) -> Result<(), RankedIneligibilityReason> {
        self.validate()
            .map_err(|_| RankedIneligibilityReason::MalformedEvidence)?;
        match self.taints().first() {
            Some(taint) => Err(RankedIneligibilityReason::Input(taint.kind)),
            None => Ok(()),
        }
    }

    /// Irreversibly include one observation, retaining its earliest frame.
    pub fn taint(&mut self, kind: InputTaintKind, first_frame: u32) {
        let Self::Recorded { taints } = self;
        if let Some(existing) = taints.iter_mut().find(|taint| taint.kind == kind) {
            existing.first_frame = existing.first_frame.min(first_frame);
        } else {
            taints.push(InputTaint { kind, first_frame });
        }
        self.canonicalize();
    }

    pub fn include(&mut self, taint: InputTaint) {
        self.taint(taint.kind, taint.first_frame);
    }

    pub fn include_all(&mut self, taints: impl IntoIterator<Item = InputTaint>) {
        for taint in taints {
            self.include(taint);
        }
    }

    pub fn validate(&self) -> Result<(), RankabilityEvidenceError> {
        let Self::Recorded { taints } = self;
        let mut kinds = BTreeSet::new();
        if taints.iter().any(|taint| !kinds.insert(taint.kind)) {
            return Err(RankabilityEvidenceError::DuplicateKind);
        }
        let mut canonical = self.clone();
        canonical.canonicalize();
        if canonical != *self {
            return Err(RankabilityEvidenceError::NonCanonicalOrder);
        }
        Ok(())
    }

    fn canonicalize(&mut self) {
        let Self::Recorded { taints } = self;
        taints.sort_by_key(|taint| taint.kind);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RankedIneligibilityReason {
    Input(InputTaintKind),
    MalformedEvidence,
}

impl RankedIneligibilityReason {
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Input(kind) => kind.stable_code(),
            Self::MalformedEvidence => "malformed_input_provenance",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RankabilityEvidenceError {
    #[error("rankability evidence contains a duplicate taint kind")]
    DuplicateKind,
    #[error("rankability evidence is not in canonical taint-kind order")]
    NonCanonicalOrder,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_codes_and_serde_names_are_frozen() {
        for kind in InputTaintKind::ALL {
            assert_eq!(
                serde_json::to_string(&kind).unwrap(),
                format!("\"{}\"", kind.stable_code())
            );
        }
        assert!(serde_json::from_str::<InputTaintKind>("\"unknown_legacy\"").is_err());
        assert!(serde_json::from_str::<InputTaintKind>("\"future_source\"").is_err());
    }

    #[test]
    fn taints_are_unique_cumulative_and_keep_the_earliest_frame() {
        let mut evidence = ReplayRankability::rankable();
        evidence.taint(InputTaintKind::ConsoleCommand, 20);
        evidence.taint(InputTaintKind::HttpStateMutation, 10);
        evidence.taint(InputTaintKind::ConsoleCommand, 5);
        assert_eq!(
            evidence.taints(),
            &[
                InputTaint {
                    kind: InputTaintKind::HttpStateMutation,
                    first_frame: 10,
                },
                InputTaint {
                    kind: InputTaintKind::ConsoleCommand,
                    first_frame: 5,
                },
            ]
        );
        assert_eq!(
            evidence.verdict(),
            Err(RankedIneligibilityReason::Input(
                InputTaintKind::HttpStateMutation
            ))
        );
    }

    #[test]
    fn rankable_verdict_needs_no_engine() {
        assert_eq!(ReplayRankability::rankable().verdict(), Ok(()));
    }

    #[test]
    fn hostile_duplicate_or_unsorted_evidence_is_rejected() {
        let duplicate = r#"{"status":"recorded","taints":[{"kind":"console_command","first_frame":1},{"kind":"console_command","first_frame":2}]}"#;
        let duplicate: ReplayRankability = serde_json::from_str(duplicate).unwrap();
        assert_eq!(
            duplicate.validate(),
            Err(RankabilityEvidenceError::DuplicateKind)
        );

        let unsorted = r#"{"status":"recorded","taints":[{"kind":"console_command","first_frame":2},{"kind":"http_state_mutation","first_frame":1}]}"#;
        let unsorted: ReplayRankability = serde_json::from_str(unsorted).unwrap();
        assert_eq!(
            unsorted.validate(),
            Err(RankabilityEvidenceError::NonCanonicalOrder)
        );

        let extra = r#"{"status":"recorded","taints":[],"future_policy":"rankable"}"#;
        assert!(serde_json::from_str::<ReplayRankability>(extra).is_err());
    }
}
