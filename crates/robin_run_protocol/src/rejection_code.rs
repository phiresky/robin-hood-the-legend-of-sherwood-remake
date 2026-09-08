//! Stable persistence vocabulary for verifier rejections.
use crate::VerificationRejectionCodeV1;

impl VerificationRejectionCodeV1 {
    pub const ALL: [Self; 14] = [
        Self::MalformedReplay,
        Self::ResourceLimit,
        Self::UnsupportedSchema,
        Self::BuildNotAllowed,
        Self::ContentNotAllowed,
        Self::ConfigMismatch,
        Self::StartingStateMismatch,
        Self::CommandNotAllowed,
        Self::TimelineInvalid,
        Self::StateHashMismatch,
        Self::TerminalInvalid,
        Self::ResultInvariantMismatch,
        Self::InputProvenanceIneligible,
        Self::SimulationBudgetExceeded,
    ];
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MalformedReplay => "malformed_replay",
            Self::ResourceLimit => "resource_limit",
            Self::UnsupportedSchema => "unsupported_schema",
            Self::BuildNotAllowed => "build_not_allowed",
            Self::ContentNotAllowed => "content_not_allowed",
            Self::ConfigMismatch => "config_mismatch",
            Self::StartingStateMismatch => "starting_state_mismatch",
            Self::CommandNotAllowed => "command_not_allowed",
            Self::TimelineInvalid => "timeline_invalid",
            Self::StateHashMismatch => "state_hash_mismatch",
            Self::TerminalInvalid => "terminal_invalid",
            Self::ResultInvariantMismatch => "result_invariant_mismatch",
            Self::InputProvenanceIneligible => "input_provenance_ineligible",
            Self::SimulationBudgetExceeded => "simulation_budget_exceeded",
        }
    }
}

impl std::str::FromStr for VerificationRejectionCodeV1 {
    type Err = crate::ValidationError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|code| code.as_str() == value)
            .ok_or(crate::ValidationError::ClaimMismatch {
                field: "verification_rejection.code",
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persistence_codes_match_wire_spelling_and_round_trip() {
        for code in VerificationRejectionCodeV1::ALL {
            assert_eq!(code.as_str().parse(), Ok(code));
            assert_eq!(
                serde_json::to_value(code).unwrap(),
                serde_json::Value::String(code.as_str().into())
            );
        }
        assert!("unknown".parse::<VerificationRejectionCodeV1>().is_err());
    }
}
