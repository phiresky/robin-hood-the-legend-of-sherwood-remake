//! Content-addressed artifact reference.

use serde::{Deserialize, Serialize};

use crate::{Digest32, Validate, ValidationError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRefV1 {
    pub sha256: Digest32,
    pub byte_length: u64,
    pub media_type: String,
}

impl Validate for ArtifactRefV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.sha256.is_zero() || self.byte_length == 0 {
            return Err(ValidationError::Zero {
                field: "artifact.sha256/byte_length",
            });
        }
        crate::validation::text("artifact.media_type", &self.media_type, 128)
    }
}
