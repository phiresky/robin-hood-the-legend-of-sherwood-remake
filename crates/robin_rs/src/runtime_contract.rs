//! Build-time contract exported for browser and Android packaging.
//!
//! Exporting compiled constants keeps packaging independent of Rust module
//! layout and spelling. Serialized payload/signature tests remain authoritative
//! for wire compatibility; this document only supplies shared version metadata.
use serde::{Deserialize, Serialize};

pub const JOIN_CODE_PREFIX: &str = "rhmp3-";
pub const JOIN_TICKET_SCHEMA: u32 = 3;
pub const BROWSER_CONTENT_SCHEMA: u32 = 2;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeContract {
    pub schema: u32,
    pub net_protocol: u32,
    pub replay_schema: u32,
    pub ticket_schema: u32,
    pub join_code_prefix: String,
    pub content_schema: u32,
    pub shipping_datadir_schema: u32,
}

impl RuntimeContract {
    pub fn current() -> Self {
        Self {
            schema: 1,
            net_protocol: robin_engine::multiplayer::NET_PROTOCOL_VERSION,
            replay_schema: robin_engine::replay::REPLAY_SCHEMA_VERSION,
            ticket_schema: JOIN_TICKET_SCHEMA,
            join_code_prefix: JOIN_CODE_PREFIX.to_owned(),
            content_schema: BROWSER_CONTENT_SCHEMA,
            shipping_datadir_schema: robin_assets::shipping_datadir::SHIPPING_DATADIR_VERSION,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RuntimeContract;

    #[test]
    fn checked_in_browser_contract_matches_compiled_engine() {
        let checked_in: RuntimeContract =
            serde_json::from_str(include_str!("../../../wasm-www/runtime-contract.json")).unwrap();
        assert_eq!(
            checked_in,
            RuntimeContract::current(),
            "regenerate wasm-www/runtime-contract.json with export_runtime_contract"
        );
    }
}
