//! Filesystem boundary for the private official simulation-content exporter.
//!
//! The engine owns how prepared mission inputs become semantic components.
//! This module owns only the hostile-output checks and create-new writes used
//! by the operator executable. Keeping that boundary separate lets the
//! verifier and exporter consume the same engine projection without giving an
//! ordinary game build an authoring surface.

use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail, ensure};
use robin_run_protocol::{
    CanonicalDocument as _, Digest32, OfficialContentSubjectV1,
    SimulationContentComponentDocumentV1, SimulationContentComponentKindV1, Validate as _,
    simulation_content_component_relative_path_v1,
};
use serde::{Deserialize, Serialize};

#[cfg(test)]
#[path = "../build_support/projection_static.rs"]
mod projection_static_build_policy;

/// One explicit exporter request installed by the private projection process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationContentExportRequest {
    pub output_root: PathBuf,
    pub subject: OfficialContentSubjectV1,
}

/// Owned adapter at the engine/exporter boundary.
///
/// `canonical_bytes` and `sha256` are independently checked here. The engine's
/// shared `ProjectedSimulationContentComponentV1` maps directly to these
/// fields once a mission has been prepared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalProjectionComponent {
    pub document: SimulationContentComponentDocumentV1,
    pub canonical_bytes: Vec<u8>,
    pub sha256: Digest32,
}

/// Validate a complete ordered component projection before creating any file,
/// then write every object with `create_new` semantics.
pub fn write_simulation_content_projection(
    request: &SimulationContentExportRequest,
    prepared_mission_id: &str,
    expected_order: &[SimulationContentComponentKindV1],
    components: &[CanonicalProjectionComponent],
) -> Result<Vec<PathBuf>> {
    request.subject.validate()?;
    ensure!(
        request.subject.mission_id() == prepared_mission_id,
        "prepared mission {prepared_mission_id:?} differs from exporter subject {:?}",
        request.subject.mission_id()
    );
    validate_existing_directory(&request.output_root, "projection catalog root")?;
    ensure!(
        !expected_order.is_empty() && components.len() == expected_order.len(),
        "projection component count differs from the engine-owned order"
    );

    let mut planned = Vec::with_capacity(components.len());
    let mut paths = BTreeSet::new();
    for (expected_kind, component) in expected_order.iter().zip(components) {
        component.document.validate()?;
        ensure!(
            component.document.kind == *expected_kind,
            "projection component order differs from the engine-owned order"
        );
        let canonical = component.document.canonical_bytes()?;
        ensure!(
            canonical == component.canonical_bytes,
            "projection component bytes are not the document's canonical JSON"
        );
        ensure!(
            Digest32::digest_bytes(&component.canonical_bytes) == component.sha256,
            "projection component digest differs from its canonical bytes"
        );
        let relative = simulation_content_component_relative_path_v1(
            &request.subject,
            component.document.kind,
        )?;
        ensure!(
            paths.insert(relative.clone()),
            "projection contains a duplicate output path {relative}"
        );
        let output = request.output_root.join(&relative);
        ensure!(
            !output.exists(),
            "projection output already exists: {}",
            output.display()
        );
        planned.push((relative, output, component.canonical_bytes.as_slice()));
    }

    let mut written = Vec::with_capacity(planned.len());
    for (relative, output, bytes) in planned {
        create_relative_parent_directories(&request.output_root, Path::new(&relative))?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
            .with_context(|| format!("create projection component {}", output.display()))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        written.push(output);
    }
    Ok(written)
}

fn validate_existing_directory(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect {label} {}", path.display()))?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "{label} is not a real directory: {}",
        path.display()
    );
    Ok(())
}

fn create_relative_parent_directories(root: &Path, relative_file: &Path) -> Result<()> {
    let parent = relative_file
        .parent()
        .context("projection component path has no parent")?;
    let mut current = root.to_path_buf();
    for component in parent.components() {
        let std::path::Component::Normal(component) = component else {
            bail!("projection component parent is not a safe relative path");
        };
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "projection output parent is not a real directory: {}",
                current.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).with_context(|| {
                    format!("create projection output directory {}", current.display())
                })?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_run_protocol::CanonicalValue;

    fn component(kind: SimulationContentComponentKindV1) -> CanonicalProjectionComponent {
        let document = SimulationContentComponentDocumentV1 {
            schema_version: 1,
            kind,
            component_schema_version: 1,
            payload: CanonicalValue::Null,
        };
        let canonical_bytes = document.canonical_bytes().unwrap();
        CanonicalProjectionComponent {
            sha256: Digest32::digest_bytes(&canonical_bytes),
            document,
            canonical_bytes,
        }
    }

    #[test]
    fn writes_only_an_exact_ordered_subject_projection() {
        let root = tempfile::tempdir().unwrap();
        let request = SimulationContentExportRequest {
            output_root: root.path().to_owned(),
            subject: OfficialContentSubjectV1::FieldMission {
                mission_id: "Dem_Lei_MP".into(),
            },
        };
        let order = [
            SimulationContentComponentKindV1::Profiles,
            SimulationContentComponentKindV1::LoadedLevel,
        ];
        let components = [component(order[0]), component(order[1])];
        let written =
            write_simulation_content_projection(&request, "Dem_Lei_MP", &order, &components)
                .unwrap();
        assert_eq!(written.len(), 2);
        for (kind, output) in order.into_iter().zip(written) {
            assert_eq!(fs::read(output).unwrap(), component(kind).canonical_bytes);
        }
    }

    #[test]
    fn validates_every_component_before_writing_any_file() {
        let root = tempfile::tempdir().unwrap();
        let request = SimulationContentExportRequest {
            output_root: root.path().to_owned(),
            subject: OfficialContentSubjectV1::Headquarters {
                mission_id: "Sherwood".into(),
            },
        };
        let order = [
            SimulationContentComponentKindV1::Profiles,
            SimulationContentComponentKindV1::LoadedLevel,
        ];
        let components = [component(order[0]), component(order[0])];
        assert!(
            write_simulation_content_projection(&request, "Sherwood", &order, &components).is_err()
        );
        assert!(fs::read_dir(root.path()).unwrap().next().is_none());
    }

    #[test]
    fn rejects_wrong_mission_and_existing_outputs() {
        let root = tempfile::tempdir().unwrap();
        let request = SimulationContentExportRequest {
            output_root: root.path().to_owned(),
            subject: OfficialContentSubjectV1::FieldMission {
                mission_id: "Dem_Lei_MP".into(),
            },
        };
        let order = [SimulationContentComponentKindV1::Profiles];
        let components = [component(order[0])];
        assert!(
            write_simulation_content_projection(&request, "wrong", &order, &components).is_err()
        );
        write_simulation_content_projection(&request, "Dem_Lei_MP", &order, &components).unwrap();
        assert!(
            write_simulation_content_projection(&request, "Dem_Lei_MP", &order, &components)
                .is_err()
        );
    }
}
