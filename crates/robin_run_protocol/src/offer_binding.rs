//! Exact binding between client-authored requests and server-authored offers.
use crate::{
    InitialStateExpectationV1, ScopeRequestV1, SubmissionOfferRequestV1, SubmissionOfferV1,
    ValidationError,
};

/// Check only the fields shared by a request and its offer. Each trust boundary
/// must separately validate both documents and its own local/authoritative facts.
pub fn validate_offer_binding(
    request: &SubmissionOfferRequestV1,
    offer: &SubmissionOfferV1,
) -> Result<(), ValidationError> {
    let scope_matches = match (&request.scope_request, &offer.starting_state) {
        (ScopeRequestV1::IndividualLevel, InitialStateExpectationV1::IndividualLevel { .. })
        | (ScopeRequestV1::CampaignGenesis, InitialStateExpectationV1::CampaignGenesis { .. }) => {
            true
        }
        (
            ScopeRequestV1::CampaignContinuation {
                chain_id,
                predecessor_run_id,
            },
            InitialStateExpectationV1::CampaignContinuation {
                chain_id: offered_chain_id,
                predecessor_run_id: offered_predecessor_run_id,
                ..
            },
        ) => chain_id == offered_chain_id && predecessor_run_id == offered_predecessor_run_id,
        _ => false,
    };
    let fields = [
        ("scope_request", scope_matches),
        (
            "schema_version",
            offer.schema_version == request.schema_version,
        ),
        (
            "max_concurrent_players",
            offer.max_concurrent_players == request.max_concurrent_players,
        ),
        (
            "participant_instance_count",
            offer.participant_instance_count == request.participant_instance_count,
        ),
        (
            "participant_claims",
            offer.participant_claims == request.participant_claims,
        ),
        (
            "session_genesis",
            offer.session_genesis == request.session_genesis,
        ),
        ("mission_id", offer.mission_id == request.mission_id),
        (
            "ruleset_manifest_sha256",
            offer.ruleset_manifest_sha256 == request.ruleset_manifest_sha256,
        ),
        (
            "competition_manifest_sha256",
            offer.competition_manifest_sha256 == request.competition_manifest_sha256,
        ),
    ];
    for (field, matches) in fields {
        if !matches {
            return Err(ValidationError::ClaimMismatch { field });
        }
    }
    Ok(())
}
