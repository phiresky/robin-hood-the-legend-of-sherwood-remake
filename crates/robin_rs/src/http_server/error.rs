//! Semantic RPC failures are independent of transport status codes.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RpcErrorKind {
    InvalidRequest,
    UnavailableCapability,
    Retired,
    Cancelled,
    Deadline,
    Capacity,
    Internal,
}

/// Structured diagnostics for callers inside the application. Wire adapters
/// deliberately emit only `Display` to preserve the existing error protocol.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcError {
    pub kind: RpcErrorKind,
    pub message: String,
}

macro_rules! constructors {
    ($($name:ident => $kind:ident),+ $(,)?) => {
        impl RpcError {
            $(pub fn $name(message: impl Into<String>) -> Self {
                Self { kind: RpcErrorKind::$kind, message: message.into() }
            })+
        }
    };
}

constructors! {
    invalid_request => InvalidRequest,
    unavailable_capability => UnavailableCapability,
    retired => Retired,
    cancelled => Cancelled,
    deadline => Deadline,
    capacity => Capacity,
    internal => Internal,
}

impl RpcError {
    /// Legacy transport envelope. Category serialization is reserved for
    /// internal diagnostics until a separately versioned protocol adopts it.
    pub fn wire_body(&self) -> serde_json::Value {
        serde_json::json!({"error": self.message})
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for RpcError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn categories_survive_diagnostics_without_changing_wire_messages() {
        for error in [
            RpcError::invalid_request("detail"),
            RpcError::unavailable_capability("detail"),
            RpcError::retired("detail"),
            RpcError::cancelled("detail"),
            RpcError::deadline("detail"),
            RpcError::capacity("detail"),
            RpcError::internal("detail"),
        ] {
            let diagnostic = serde_json::to_value(&error).unwrap();
            assert_eq!(
                serde_json::from_value::<RpcError>(diagnostic).unwrap(),
                error
            );
            assert_eq!(error.wire_body(), serde_json::json!({"error": "detail"}));
        }
    }
}
