//! Closed, typed authorities for the JavaScript tools used by official builds.
//!
//! A version string alone is not a reproducible build input. These documents
//! bind the exact upstream distribution and the exact entrypoint selected from
//! that distribution. Callers must verify both identities before execution.

use robin_run_protocol::{
    ArtifactRefV1, BuildToolAuthorityV1, CanonicalDocument as _, CanonicalDocumentError, Digest32,
    Validate, ValidationError,
};
use serde::{Deserialize, Serialize};

pub const JAVASCRIPT_BUILD_TOOL_AUTHORITY_SCHEMA_VERSION_V1: u32 = 1;
pub const NODE_VERSION_V1: &str = "24.19.0";
pub const PNPM_VERSION_V1: &str = "12.3.4";

const NODE_DISTRIBUTION_URL_V1: &str =
    "https://nodejs.org/dist/v24.19.0/node-v24.19.0-linux-x64.tar.xz";
const NODE_EXECUTABLE_RELATIVE_PATH_V1: &str = "node-v24.19.0-linux-x64/bin/node";
const PNPM_DISTRIBUTION_URL_V1: &str =
    "https://registry.npmjs.org/@pnpm/exe.linux-x64/-/exe.linux-x64-12.3.4.tgz";
const PNPM_EXECUTABLE_RELATIVE_PATH_V1: &str = "package/pnpm";

const XZ_MEDIA_TYPE: &str = "application/x-xz";
const GZIP_MEDIA_TYPE: &str = "application/gzip";
const NATIVE_EXECUTABLE_MEDIA_TYPE: &str = "application/x-executable";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JavaScriptBuildToolRoleV1 {
    Node,
    Pnpm,
}

/// Exact upstream distribution and selected executable for one JavaScript
/// build tool.
///
/// `executable_relative_path` is relative to the root of the verified archive.
/// pnpm 12 uses the Linux x64 native executable distribution; its JavaScript
/// bootstrap package is not an executable authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JavaScriptBuildToolAuthorityDocumentV1 {
    pub schema_version: u32,
    pub role: JavaScriptBuildToolRoleV1,
    pub version: String,
    pub distribution_url: String,
    pub distribution: ArtifactRefV1,
    pub executable_relative_path: String,
    pub executable: ArtifactRefV1,
}

impl JavaScriptBuildToolAuthorityDocumentV1 {
    /// Return the one admitted authority for `role`.
    pub fn official(role: JavaScriptBuildToolRoleV1) -> Self {
        match role {
            JavaScriptBuildToolRoleV1::Node => official_node_authority_v1(),
            JavaScriptBuildToolRoleV1::Pnpm => official_pnpm_authority_v1(),
        }
    }

    /// Produce the compact public-manifest binding after validating this
    /// complete authority document.
    pub fn build_tool_binding(&self) -> Result<BuildToolAuthorityV1, CanonicalDocumentError> {
        Ok(BuildToolAuthorityV1 {
            version: self.version.clone(),
            authority_sha256: self.canonical_digest()?,
        })
    }

    /// Verify identities computed from materialized archive and entrypoint
    /// files. The media types are part of the identity and cannot be changed by
    /// the caller.
    pub fn validate_materialized_artifacts(
        &self,
        distribution: &ArtifactRefV1,
        executable: &ArtifactRefV1,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        if distribution != &self.distribution {
            return Err(ValidationError::ClaimMismatch {
                field: "build.javascript_tool.distribution_artifact",
            });
        }
        if executable != &self.executable {
            return Err(ValidationError::ClaimMismatch {
                field: "build.javascript_tool.executable_artifact",
            });
        }
        Ok(())
    }

    /// Convenience check for callers that already hold the downloaded bytes.
    pub fn validate_distribution_bytes(&self, bytes: &[u8]) -> Result<(), ValidationError> {
        self.validate()?;
        validate_bytes(
            "build.javascript_tool.distribution_bytes",
            bytes,
            &self.distribution,
        )
    }

    /// Convenience check for callers that already hold the extracted
    /// entrypoint bytes.
    pub fn validate_executable_bytes(&self, bytes: &[u8]) -> Result<(), ValidationError> {
        self.validate()?;
        validate_bytes(
            "build.javascript_tool.executable_bytes",
            bytes,
            &self.executable,
        )
    }
}

impl Validate for JavaScriptBuildToolAuthorityDocumentV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != JAVASCRIPT_BUILD_TOOL_AUTHORITY_SCHEMA_VERSION_V1 {
            return Err(ValidationError::SchemaVersion {
                document: "JavaScriptBuildToolAuthorityDocumentV1",
                expected: JAVASCRIPT_BUILD_TOOL_AUTHORITY_SCHEMA_VERSION_V1,
                actual: self.schema_version,
            });
        }
        self.distribution.validate()?;
        self.executable.validate()?;

        let expected = Self::official(self.role);
        if self != &expected {
            return Err(ValidationError::ClaimMismatch {
                field: "build.javascript_tool.authority_document",
            });
        }
        Ok(())
    }
}

pub fn official_node_authority_v1() -> JavaScriptBuildToolAuthorityDocumentV1 {
    JavaScriptBuildToolAuthorityDocumentV1 {
        schema_version: JAVASCRIPT_BUILD_TOOL_AUTHORITY_SCHEMA_VERSION_V1,
        role: JavaScriptBuildToolRoleV1::Node,
        version: NODE_VERSION_V1.to_owned(),
        distribution_url: NODE_DISTRIBUTION_URL_V1.to_owned(),
        distribution: ArtifactRefV1 {
            sha256: Digest32::from_bytes([
                0x14, 0xb3, 0x42, 0xe7, 0x12, 0x04, 0xf8, 0x11, 0xbd, 0xe6, 0x15, 0x3b, 0xe8, 0xe0,
                0x4b, 0x62, 0xae, 0xf6, 0x3c, 0x23, 0x6f, 0xef, 0x92, 0xb5, 0x5f, 0x9c, 0x83, 0x15,
                0x4b, 0x40, 0x96, 0x47,
            ]),
            byte_length: 31_633_904,
            media_type: XZ_MEDIA_TYPE.to_owned(),
        },
        executable_relative_path: NODE_EXECUTABLE_RELATIVE_PATH_V1.to_owned(),
        executable: ArtifactRefV1 {
            sha256: Digest32::from_bytes([
                0xbc, 0x17, 0xc5, 0x08, 0xff, 0xee, 0xd0, 0xec, 0x62, 0x29, 0x34, 0xf9, 0xb7, 0xfa,
                0x72, 0xf8, 0xe7, 0x8d, 0xa6, 0x53, 0x50, 0xe6, 0x3c, 0x3e, 0xce, 0xb5, 0x6f, 0xa6,
                0x88, 0xaa, 0x5e, 0x12,
            ]),
            byte_length: 125_989_464,
            media_type: NATIVE_EXECUTABLE_MEDIA_TYPE.to_owned(),
        },
    }
}

pub fn official_pnpm_authority_v1() -> JavaScriptBuildToolAuthorityDocumentV1 {
    JavaScriptBuildToolAuthorityDocumentV1 {
        schema_version: JAVASCRIPT_BUILD_TOOL_AUTHORITY_SCHEMA_VERSION_V1,
        role: JavaScriptBuildToolRoleV1::Pnpm,
        version: PNPM_VERSION_V1.to_owned(),
        distribution_url: PNPM_DISTRIBUTION_URL_V1.to_owned(),
        distribution: ArtifactRefV1 {
            sha256: Digest32::from_bytes([
                0x9b, 0x1c, 0x95, 0xfc, 0x41, 0x36, 0x00, 0xca, 0x75, 0xa5, 0x1e, 0xbe, 0x83, 0x32,
                0xd7, 0x01, 0x9d, 0xc3, 0x56, 0x8f, 0xd4, 0x18, 0x7f, 0x5a, 0x5f, 0x2d, 0xf4, 0xa1,
                0x56, 0xef, 0xb6, 0x5c,
            ]),
            byte_length: 19_420_261,
            media_type: GZIP_MEDIA_TYPE.to_owned(),
        },
        executable_relative_path: PNPM_EXECUTABLE_RELATIVE_PATH_V1.to_owned(),
        executable: ArtifactRefV1 {
            sha256: Digest32::from_bytes([
                0xcf, 0xb8, 0xaa, 0xd5, 0x1b, 0xaa, 0x14, 0xf1, 0xa7, 0x6d, 0x42, 0x27, 0x33, 0xe0,
                0xa6, 0xd0, 0x2b, 0x09, 0x55, 0x43, 0x87, 0x52, 0xea, 0x5a, 0x92, 0x26, 0xa1, 0x41,
                0xbe, 0x91, 0x79, 0x60,
            ]),
            byte_length: 45_153_536,
            media_type: NATIVE_EXECUTABLE_MEDIA_TYPE.to_owned(),
        },
    }
}

fn validate_bytes(
    field: &'static str,
    bytes: &[u8],
    expected: &ArtifactRefV1,
) -> Result<(), ValidationError> {
    if bytes.len() as u64 != expected.byte_length
        || Digest32::digest_bytes(bytes) != expected.sha256
    {
        return Err(ValidationError::ClaimMismatch { field });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_authorities_validate_and_produce_canonical_bindings() {
        for (authority, checked_in) in [
            (
                official_node_authority_v1(),
                include_bytes!("../../../.github/tool-authorities/node-v24.19.0.json").as_slice(),
            ),
            (
                official_pnpm_authority_v1(),
                include_bytes!("../../../.github/tool-authorities/pnpm-v12.3.4.json").as_slice(),
            ),
        ] {
            authority.validate().unwrap();
            let binding = authority.build_tool_binding().unwrap();
            assert_eq!(binding.version, authority.version);
            assert_eq!(
                binding.authority_sha256,
                authority.canonical_digest().unwrap()
            );

            let canonical = authority.canonical_bytes().unwrap();
            assert!(
                checked_in == canonical || checked_in == [canonical.as_slice(), b"\n"].concat(),
                "checked-in JavaScript authority drifted from its typed official facts"
            );
            assert_eq!(
                serde_json::from_slice::<JavaScriptBuildToolAuthorityDocumentV1>(&canonical)
                    .unwrap(),
                authority
            );
        }
    }

    #[test]
    fn exact_upstream_distribution_and_entrypoint_facts_are_closed() {
        let node = official_node_authority_v1();
        assert_eq!(node.version, "24.19.0");
        assert_eq!(
            node.distribution_url,
            "https://nodejs.org/dist/v24.19.0/node-v24.19.0-linux-x64.tar.xz"
        );
        assert_eq!(
            node.distribution.sha256.to_string(),
            "14b342e71204f811bde6153be8e04b62aef63c236fef92b55f9c83154b409647"
        );
        assert_eq!(node.distribution.byte_length, 31_633_904);
        assert_eq!(
            node.executable.sha256.to_string(),
            "bc17c508ffeed0ec622934f9b7fa72f8e78da65350e63c3eceb56fa688aa5e12"
        );
        assert_eq!(node.executable.byte_length, 125_989_464);

        let pnpm = official_pnpm_authority_v1();
        assert_eq!(pnpm.version, "12.3.4");
        assert_eq!(
            pnpm.distribution_url,
            "https://registry.npmjs.org/@pnpm/exe.linux-x64/-/exe.linux-x64-12.3.4.tgz"
        );
        assert_eq!(
            pnpm.distribution.sha256.to_string(),
            "9b1c95fc413600ca75a51ebe8332d7019dc3568fd4187f5a5f2df4a156efb65c"
        );
        assert_eq!(pnpm.distribution.byte_length, 19_420_261);
        assert_eq!(
            pnpm.executable.sha256.to_string(),
            "cfb8aad51baa14f1a76d422733e0a6d02b0955438752ea5a9226a141be917960"
        );
        assert_eq!(pnpm.executable.byte_length, 45_153_536);
    }

    #[test]
    fn serde_rejects_unknown_fields() {
        let mut value = serde_json::to_value(official_node_authority_v1()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("untrusted".into(), serde_json::Value::Bool(true));
        assert!(serde_json::from_value::<JavaScriptBuildToolAuthorityDocumentV1>(value).is_err());
    }

    #[test]
    fn every_authority_substitution_is_rejected() {
        let valid = official_node_authority_v1();
        let mutations: [Box<dyn Fn(&mut JavaScriptBuildToolAuthorityDocumentV1)>; 10] = [
            Box::new(|value| value.schema_version += 1),
            Box::new(|value| value.role = JavaScriptBuildToolRoleV1::Pnpm),
            Box::new(|value| value.version = "24.18.0".into()),
            Box::new(|value| value.distribution_url.push_str("?mirror=1")),
            Box::new(|value| value.distribution.sha256 = Digest32::digest_bytes(b"substitute")),
            Box::new(|value| value.distribution.byte_length += 1),
            Box::new(|value| value.distribution.media_type = "application/octet-stream".into()),
            Box::new(|value| value.executable_relative_path = "bin/node".into()),
            Box::new(|value| value.executable.sha256 = Digest32::digest_bytes(b"substitute")),
            Box::new(|value| value.executable.byte_length += 1),
        ];
        for mutate in mutations {
            let mut substituted = valid.clone();
            mutate(&mut substituted);
            assert!(substituted.validate().is_err());
        }

        let mut substituted = valid;
        substituted.executable.media_type = "application/octet-stream".into();
        assert!(substituted.validate().is_err());
    }

    #[test]
    fn materialized_artifact_checks_fail_closed() {
        let authority = official_pnpm_authority_v1();
        authority
            .validate_materialized_artifacts(&authority.distribution, &authority.executable)
            .unwrap();

        let mut wrong_distribution = authority.distribution.clone();
        wrong_distribution.sha256 = Digest32::digest_bytes(b"wrong distribution");
        assert!(
            authority
                .validate_materialized_artifacts(&wrong_distribution, &authority.executable)
                .is_err()
        );

        let mut wrong_executable = authority.executable.clone();
        wrong_executable.byte_length += 1;
        assert!(
            authority
                .validate_materialized_artifacts(&authority.distribution, &wrong_executable)
                .is_err()
        );
        assert!(authority.validate_distribution_bytes(b"wrong").is_err());
        assert!(authority.validate_executable_bytes(b"wrong").is_err());
    }
}
