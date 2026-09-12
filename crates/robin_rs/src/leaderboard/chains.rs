//! Local index of server-authored full-fidelity campaign-chain receipts.
//!
//! The store never reconstructs a campaign from totals or recent missions. A
//! continuation matches only the digest and byte length of the exact campaign
//! artifact plus its complete server-pinned content, rules, controller, and
//! participant policy.

use robin_run_protocol::{
    ArtifactRefV1, CampaignChainReceiptV1, CampaignRosterContinuityV1, Digest32,
    MAX_REPLAY_SEATS_V1, PublicKey32, RANKED_CAMPAIGN_MEDIA_TYPE_V1, Validate, ValidationError,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const STORE_SCHEMA_VERSION: u32 = 1;
const STORE_FILE: &str = "leaderboard-campaign-chains.json";
const BROWSER_STORE_KEY: &str = "robin-hood.leaderboard-campaign-chains.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignChainStore {
    schema_version: u32,
    receipts: Vec<CampaignChainReceiptV1>,
}

impl Default for CampaignChainStore {
    fn default() -> Self {
        Self::empty()
    }
}

/// Exact campaign identity shared by receipt selection and native byte admission.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CampaignContinuationKey {
    pub expected_max_concurrent_players: u16,
    pub participant_public_keys: Vec<PublicKey32>,
    pub campaign_content_manifest_sha256: Digest32,
    pub rules_config_sha256: Digest32,
    pub ruleset_manifest_sha256: Digest32,
    pub competition_manifest_sha256: Option<Digest32>,
    pub campaign_controller_public_key: PublicKey32,
}

impl CampaignChainStore {
    pub fn empty() -> Self {
        Self {
            schema_version: STORE_SCHEMA_VERSION,
            receipts: Vec::new(),
        }
    }

    pub fn receipts(&self) -> &[CampaignChainReceiptV1] {
        &self.receipts
    }

    pub fn validate(&self) -> Result<(), CampaignChainStoreError> {
        if self.schema_version != STORE_SCHEMA_VERSION {
            return Err(CampaignChainStoreError::UnsupportedSchema {
                found: self.schema_version,
                expected: STORE_SCHEMA_VERSION,
            });
        }
        for receipt in &self.receipts {
            receipt.validate()?;
        }
        for (index, receipt) in self.receipts.iter().enumerate() {
            if self.receipts[index + 1..]
                .iter()
                .any(|candidate| candidate.chain_id == receipt.chain_id)
            {
                return Err(CampaignChainStoreError::DuplicateReceipt);
            }
        }
        Ok(())
    }

    /// Record only a receipt returned after server verification. It replaces
    /// the prior head for the server-unique chain, never an unrelated chain.
    /// A verified campaign may change roster, player limit, and controller
    /// between sessions, so none of those session claims identify a head.
    pub fn accepted(
        &mut self,
        receipt: CampaignChainReceiptV1,
    ) -> Result<(), CampaignChainStoreError> {
        self.validate()?;
        receipt.validate()?;
        self.receipts
            .retain(|existing| existing.chain_id != receipt.chain_id);
        self.receipts.push(receipt);
        self.validate()
    }

    /// Resolve one exact active predecessor under the immutable ruleset's
    /// roster-continuity policy. Union-of-subsets campaigns may change the
    /// authenticated key subset between sessions, but every other campaign,
    /// controller, player-cap, ruleset, and artifact field remains exact.
    pub fn continuation_for_policy(
        &self,
        starting_campaign: &ArtifactRefV1,
        key: &CampaignContinuationKey,
        roster_continuity: CampaignRosterContinuityV1,
    ) -> Result<Option<&CampaignChainReceiptV1>, CampaignChainStoreError> {
        let expected_max_concurrent_players = key.expected_max_concurrent_players;
        let participant_public_keys = key.participant_public_keys.as_slice();
        let campaign_content_manifest_sha256 = key.campaign_content_manifest_sha256;
        let rules_config_sha256 = key.rules_config_sha256;
        let ruleset_manifest_sha256 = key.ruleset_manifest_sha256;
        let competition_manifest_sha256 = key.competition_manifest_sha256;
        let campaign_controller_public_key = key.campaign_controller_public_key;
        self.validate()?;
        starting_campaign.validate()?;
        if starting_campaign.media_type != RANKED_CAMPAIGN_MEDIA_TYPE_V1
            || starting_campaign.byte_length == 0
        {
            return Err(CampaignChainStoreError::InvalidStartingCampaign);
        }
        if expected_max_concurrent_players == 0
            || expected_max_concurrent_players > MAX_REPLAY_SEATS_V1
            || participant_public_keys.is_empty()
            || participant_public_keys.len() > usize::from(expected_max_concurrent_players)
            || participant_public_keys.iter().any(PublicKey32::is_zero)
            || !participant_public_keys
                .windows(2)
                .all(|keys| keys[0] < keys[1])
            || participant_public_keys
                .binary_search(&campaign_controller_public_key)
                .is_err()
        {
            return Err(CampaignChainStoreError::InvalidParticipantPolicy);
        }
        let mut matches = self.receipts.iter().filter(|receipt| {
            &receipt.expected_starting_campaign == starting_campaign
                && receipt.expected_max_concurrent_players == expected_max_concurrent_players
                && (roster_continuity == CampaignRosterContinuityV1::UnionOfVerifiedSessionSubsets
                    || receipt.participant_public_keys == participant_public_keys)
                && receipt.campaign_content_manifest_sha256 == campaign_content_manifest_sha256
                && receipt.rules_config_sha256 == rules_config_sha256
                && receipt.ruleset_manifest_sha256 == ruleset_manifest_sha256
                && receipt.competition_manifest_sha256 == competition_manifest_sha256
                && receipt.campaign_controller_public_key == campaign_controller_public_key
        });
        let first = matches.next();
        if matches.next().is_some() {
            return Err(CampaignChainStoreError::AmbiguousContinuation);
        }
        Ok(first)
    }

    pub fn continuation_for_exact_campaign_policy(
        &self,
        exact_starting_campaign_bytes: &[u8],
        key: &CampaignContinuationKey,
        roster_continuity: CampaignRosterContinuityV1,
    ) -> Result<Option<&CampaignChainReceiptV1>, CampaignChainStoreError> {
        let artifact = exact_campaign_artifact(exact_starting_campaign_bytes)?;
        self.continuation_for_policy(&artifact, key, roster_continuity)
    }
}

pub fn exact_campaign_artifact(
    exact_starting_campaign_bytes: &[u8],
) -> Result<ArtifactRefV1, CampaignChainStoreError> {
    if exact_starting_campaign_bytes.is_empty() {
        return Err(CampaignChainStoreError::InvalidStartingCampaign);
    }
    Ok(ArtifactRefV1 {
        sha256: Digest32::digest_bytes(exact_starting_campaign_bytes),
        byte_length: u64::try_from(exact_starting_campaign_bytes.len())
            .map_err(|_| CampaignChainStoreError::InvalidStartingCampaign)?,
        media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
    })
}

#[derive(Debug, thiserror::Error)]
pub enum CampaignChainStoreError {
    #[error("unsupported campaign-chain store schema {found}; expected {expected}")]
    UnsupportedSchema { found: u32, expected: u32 },
    #[error(transparent)]
    InvalidReceipt(#[from] ValidationError),
    #[error("starting campaign must be the non-empty full-fidelity bitcode artifact")]
    InvalidStartingCampaign,
    #[error("campaign continuation participant policy is invalid")]
    InvalidParticipantPolicy,
    #[error("campaign-chain store contains more than one head for the same chain")]
    DuplicateReceipt,
    #[error("more than one verified campaign chain matches this exact mission start")]
    AmbiguousContinuation,
    #[error(transparent)]
    Storage(#[from] super::store::StoreError),
    #[error("failed to decode campaign-chain store from {path}: {source}")]
    Decode {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

pub fn load() -> Result<CampaignChainStore, CampaignChainStoreError> {
    let Some(encoded) = read_store()? else {
        return Ok(CampaignChainStore::empty());
    };
    let store: CampaignChainStore =
        serde_json::from_str(&encoded).map_err(|source| CampaignChainStoreError::Decode {
            path: display_path(),
            source,
        })?;
    store.validate()?;
    Ok(store)
}

pub fn persist(store: &CampaignChainStore) -> Result<(), CampaignChainStoreError> {
    store.validate()?;
    let encoded = serde_json::to_vec_pretty(store)
        .expect("CampaignChainStore serialization cannot fail after validation");
    persist_store(&encoded)
}

fn display_path() -> PathBuf {
    super::store::display_path(STORE_FILE, BROWSER_STORE_KEY)
}

fn read_store() -> Result<Option<String>, CampaignChainStoreError> {
    Ok(super::store::read(STORE_FILE, BROWSER_STORE_KEY)?)
}
fn persist_store(encoded: &[u8]) -> Result<(), CampaignChainStoreError> {
    Ok(super::store::write(
        STORE_FILE,
        BROWSER_STORE_KEY,
        ".leaderboard-chains-",
        encoded,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_run_protocol::{CampaignChainStateV1, OpaqueId, PublicKey32, SCHEMA_VERSION_V1};

    fn receipt(campaign: &[u8], predecessor: &str) -> CampaignChainReceiptV1 {
        CampaignChainReceiptV1 {
            schema_version: SCHEMA_VERSION_V1,
            chain_id: OpaqueId::new("chain-a").unwrap(),
            predecessor_run_id: OpaqueId::new(predecessor).unwrap(),
            predecessor_verification_sha256: Digest32::from_bytes([6; 32]),
            expected_starting_campaign: exact_campaign_artifact(campaign).unwrap(),
            rules_config_sha256: Digest32::from_bytes([5; 32]),
            ruleset_manifest_sha256: Digest32::from_bytes([2; 32]),
            campaign_content_manifest_sha256: Digest32::from_bytes([3; 32]),
            competition_manifest_sha256: None,
            expected_max_concurrent_players: 1,
            participant_public_keys: vec![PublicKey32::from_bytes([4; 32])],
            campaign_controller_public_key: PublicKey32::from_bytes([4; 32]),
            state: CampaignChainStateV1::Active,
        }
    }

    fn lookup<'a>(
        store: &'a CampaignChainStore,
        campaign: &[u8],
    ) -> Result<Option<&'a CampaignChainReceiptV1>, CampaignChainStoreError> {
        store.continuation_for_exact_campaign_policy(
            campaign,
            &CampaignContinuationKey {
                expected_max_concurrent_players: 1,
                participant_public_keys: (&[PublicKey32::from_bytes([4; 32])]).to_vec(),
                campaign_content_manifest_sha256: Digest32::from_bytes([3; 32]),
                rules_config_sha256: Digest32::from_bytes([5; 32]),
                ruleset_manifest_sha256: Digest32::from_bytes([2; 32]),
                competition_manifest_sha256: None,
                campaign_controller_public_key: PublicKey32::from_bytes([4; 32]),
            },
            CampaignRosterContinuityV1::ExactSameAuthenticatedKeysEverySession,
        )
    }

    #[test]
    fn continuation_requires_every_byte_of_the_full_campaign() {
        let campaign = b"full-fidelity campaign: missions, attempts, inventory, achievements";
        let mut store = CampaignChainStore::empty();
        store.accepted(receipt(campaign, "run-1")).unwrap();
        assert_eq!(
            lookup(&store, campaign)
                .unwrap()
                .unwrap()
                .predecessor_run_id
                .as_str(),
            "run-1"
        );

        let mut changed = campaign.to_vec();
        *changed.last_mut().unwrap() ^= 1;
        assert!(lookup(&store, &changed).unwrap().is_none());
        assert!(matches!(
            lookup(&store, b""),
            Err(CampaignChainStoreError::InvalidStartingCampaign)
        ));
    }

    #[test]
    fn union_roster_policy_allows_a_new_authenticated_subset_but_remains_unambiguous() {
        let campaign = b"full-fidelity campaign";
        let controller = PublicKey32::from_bytes([4; 32]);
        let returning_peer = PublicKey32::from_bytes([5; 32]);
        let new_peer = PublicKey32::from_bytes([6; 32]);
        let mut predecessor = receipt(campaign, "run-1");
        predecessor.expected_max_concurrent_players = 2;
        predecessor.participant_public_keys = vec![controller, returning_peer];
        let mut store = CampaignChainStore::empty();
        store.accepted(predecessor).unwrap();

        let intended_roster = [controller, new_peer];
        let lookup = |policy| {
            store.continuation_for_exact_campaign_policy(
                campaign,
                &CampaignContinuationKey {
                    expected_max_concurrent_players: 2,
                    participant_public_keys: (&intended_roster).to_vec(),
                    campaign_content_manifest_sha256: Digest32::from_bytes([3; 32]),
                    rules_config_sha256: Digest32::from_bytes([5; 32]),
                    ruleset_manifest_sha256: Digest32::from_bytes([2; 32]),
                    competition_manifest_sha256: None,
                    campaign_controller_public_key: controller,
                },
                policy,
            )
        };
        assert!(
            lookup(CampaignRosterContinuityV1::ExactSameAuthenticatedKeysEverySession)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            lookup(CampaignRosterContinuityV1::UnionOfVerifiedSessionSubsets)
                .unwrap()
                .unwrap()
                .predecessor_run_id
                .as_str(),
            "run-1"
        );

        let mut ambiguous = receipt(campaign, "run-2");
        ambiguous.chain_id = OpaqueId::new("chain-b").unwrap();
        ambiguous.expected_max_concurrent_players = 2;
        ambiguous.participant_public_keys = vec![controller, returning_peer];
        store.accepted(ambiguous).unwrap();
        assert!(matches!(
            store.continuation_for_exact_campaign_policy(
                campaign,
                &CampaignContinuationKey {
                    expected_max_concurrent_players: 2,
                    participant_public_keys: (&intended_roster).to_vec(),
                    campaign_content_manifest_sha256: Digest32::from_bytes([3; 32]),
                    rules_config_sha256: Digest32::from_bytes([5; 32]),
                    ruleset_manifest_sha256: Digest32::from_bytes([2; 32]),
                    competition_manifest_sha256: None,
                    campaign_controller_public_key: controller
                },
                CampaignRosterContinuityV1::UnionOfVerifiedSessionSubsets
            ),
            Err(CampaignChainStoreError::AmbiguousContinuation)
        ));
    }

    #[test]
    fn continuation_lookup_rejects_an_invalid_intended_roster() {
        let campaign = b"full-fidelity campaign";
        let controller = PublicKey32::from_bytes([4; 32]);
        let store = CampaignChainStore::empty();
        assert!(matches!(
            store.continuation_for_exact_campaign_policy(
                campaign,
                &CampaignContinuationKey {
                    expected_max_concurrent_players: 1,
                    participant_public_keys: (&[]).to_vec(),
                    campaign_content_manifest_sha256: Digest32::from_bytes([3; 32]),
                    rules_config_sha256: Digest32::from_bytes([5; 32]),
                    ruleset_manifest_sha256: Digest32::from_bytes([2; 32]),
                    competition_manifest_sha256: None,
                    campaign_controller_public_key: controller
                },
                CampaignRosterContinuityV1::UnionOfVerifiedSessionSubsets
            ),
            Err(CampaignChainStoreError::InvalidParticipantPolicy)
        ));
    }

    #[test]
    fn receipts_reject_impossible_player_caps_and_roster_lengths() {
        let mut impossible_cap = receipt(b"campaign", "run-1");
        impossible_cap.expected_max_concurrent_players = MAX_REPLAY_SEATS_V1 + 1;
        assert!(matches!(
            CampaignChainStore::empty().accepted(impossible_cap),
            Err(CampaignChainStoreError::InvalidReceipt(_))
        ));

        let mut oversized_roster = receipt(b"campaign", "run-1");
        oversized_roster
            .participant_public_keys
            .push(PublicKey32::from_bytes([5; 32]));
        assert!(matches!(
            CampaignChainStore::empty().accepted(oversized_roster),
            Err(CampaignChainStoreError::InvalidReceipt(_))
        ));
    }

    #[test]
    fn accepted_step_replaces_same_chain_across_session_policy_changes() {
        let mut store = CampaignChainStore::empty();
        store.accepted(receipt(b"campaign-1", "run-1")).unwrap();

        let mut unrelated = receipt(b"other-campaign", "other-run");
        unrelated.chain_id = OpaqueId::new("chain-b").unwrap();
        store.accepted(unrelated.clone()).unwrap();

        let mut replacement = receipt(b"campaign-2", "run-2");
        replacement.expected_max_concurrent_players = 2;
        replacement.participant_public_keys = vec![
            PublicKey32::from_bytes([4; 32]),
            PublicKey32::from_bytes([7; 32]),
        ];
        replacement.campaign_controller_public_key = PublicKey32::from_bytes([7; 32]);
        store.accepted(replacement.clone()).unwrap();

        assert_eq!(store.receipts().len(), 2);
        assert_eq!(
            store
                .receipts()
                .iter()
                .find(|candidate| candidate.chain_id.as_str() == "chain-a"),
            Some(&replacement)
        );
        assert_eq!(
            store
                .receipts()
                .iter()
                .find(|candidate| candidate.chain_id.as_str() == "chain-b"),
            Some(&unrelated)
        );
    }

    #[test]
    fn unknown_fields_and_duplicate_receipts_fail_closed() {
        let mut value = serde_json::to_value(CampaignChainStore::empty()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("last_three_missions".to_owned(), serde_json::json!([]));
        assert!(serde_json::from_value::<CampaignChainStore>(value).is_err());

        let first = receipt(b"campaign", "run-1");
        let mut second = receipt(b"changed-campaign", "run-2");
        second.expected_max_concurrent_players = 2;
        second.participant_public_keys = vec![
            PublicKey32::from_bytes([4; 32]),
            PublicKey32::from_bytes([7; 32]),
        ];
        second.campaign_controller_public_key = PublicKey32::from_bytes([7; 32]);
        let mut store = CampaignChainStore {
            schema_version: STORE_SCHEMA_VERSION,
            receipts: vec![first, second],
        };
        assert!(matches!(
            store.validate(),
            Err(CampaignChainStoreError::DuplicateReceipt)
        ));

        let corrupt = store.clone();
        let mut unrelated = receipt(b"unrelated", "run-3");
        unrelated.chain_id = OpaqueId::new("chain-b").unwrap();
        assert!(matches!(
            store.accepted(unrelated),
            Err(CampaignChainStoreError::DuplicateReceipt)
        ));
        assert_eq!(store, corrupt);
    }
}
