//! Deterministic payload encoding, resume validation and web manifest packaging.
use super::*;

// Recognize stages left by converter versions preceding shared publication.
const MANIFEST_STAGING_PREFIX: &str = ".robin-web-manifest-";

pub(super) fn write_web_content_manifest(
    data_out: &Path,
    edition: robin_rs::multiplayer::content_identity::WebContentEdition,
    native_content_sha256: String,
) -> Result<()> {
    use robin_rs::multiplayer::content_identity::{
        WEB_CONTENT_MANIFEST_NAME, WEB_CONTENT_MANIFEST_SCHEMA, WebContentDatadir, WebContentFile,
        WebContentFileKind, WebContentManifest,
    };

    let manifest_path = data_out.join(WEB_CONTENT_MANIFEST_NAME);
    let mut paths = Vec::new();
    let mut pending = vec![data_out.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .with_context(|| format!("enumerate web content {}", directory.display()))?
        {
            let entry =
                entry.with_context(|| format!("enumerate web content {}", directory.display()))?;
            let path = entry.path();
            if entry.file_name().to_str().is_some_and(|name| {
                (directory == data_out && name.starts_with(MANIFEST_STAGING_PREFIX))
                    || name.starts_with(publication::ARTIFACT_STAGING_PREFIX)
            }) {
                // TODO: recover abandoned stages once converter runs have an
                // exclusive ownership protocol; do not delete another run's file.
                bail!(
                    "web content has a concurrent or abandoned manifest stage: {}",
                    path.display()
                );
            }
            let metadata = fs::symlink_metadata(&path)
                .with_context(|| format!("stat web content {}", path.display()))?;
            if metadata.file_type().is_symlink() {
                bail!("web content package refuses symlink {}", path.display());
            }
            if path == manifest_path {
                if !metadata.is_file() {
                    bail!(
                        "web content manifest is not a regular file: {}",
                        path.display()
                    );
                }
                // Keep the previous publication until its replacement is ready,
                // but never include the manifest in its own content closure.
                continue;
            }
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(data_out)
                    .expect("enumerated path stays in package")
                    .to_str()
                    .ok_or_else(|| anyhow!("web content path is not UTF-8: {}", path.display()))?
                    .replace('\\', "/");
                paths.push((relative, path));
            } else {
                bail!("web content package refuses non-file {}", path.display());
            }
        }
    }
    paths.sort_by(|(left, _), (right, _)| left.cmp(right));

    let mut datadir = None;
    let mut files = Vec::new();
    let mut seen = BTreeSet::from([WEB_CONTENT_MANIFEST_NAME.to_owned()]);
    for (relative, path) in paths {
        if relative == "conversion-plan.json" {
            // Inspectable converter diagnostics are not runtime content.
            continue;
        }
        let canonical_key = relative.to_ascii_lowercase();
        if !seen.insert(canonical_key) {
            bail!("web content paths collide case-insensitively at {relative}");
        }
        let (byte_length, sha256) = digest_file(&path)?;
        if relative == "datadir.bin" {
            datadir = Some(WebContentDatadir {
                path: relative,
                byte_length,
                sha256,
            });
        } else {
            let kind = if relative.starts_with("audio/assets/")
                || relative.starts_with("audio/bundles/")
            {
                WebContentFileKind::Asset
            } else {
                WebContentFileKind::Shipping
            };
            files.push(WebContentFile {
                path: relative,
                kind,
                byte_length,
                sha256,
            });
        }
    }
    let datadir = datadir.ok_or_else(|| anyhow!("web content package has no datadir.bin"))?;
    if files.is_empty() {
        bail!("web content package has no split mission/audio files");
    }
    let manifest = WebContentManifest {
        schema: WEB_CONTENT_MANIFEST_SCHEMA,
        edition,
        engine_version: robin_rs::replay_format::ENGINE_SOURCE_COMMIT.to_string(),
        native_content_sha256,
        datadir,
        files,
    };
    let bytes = serde_json::to_vec(&manifest).context("serialize web content manifest")?;
    publication::publish_bytes(&manifest_path, &bytes)?;
    tracing::info!(manifest = %manifest_path.display(), "wrote exact web content closure");
    Ok(())
}

pub(super) fn digest_file(path: &Path) -> Result<(u64, String)> {
    use sha2::{Digest as _, Sha256};

    let mut file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut byte_length = 0_u64;
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = match std::io::Read::read(&mut file, &mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        hasher.update(&buffer[..read]);
        byte_length = byte_length
            .checked_add(read as u64)
            .context("hashed file byte length exceeds u64")?;
    }
    Ok((
        byte_length,
        robin_rs::multiplayer::content_identity::hex_digest(hasher.finalize().into()),
    ))
}

pub(super) fn shipping_file_stem(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

pub(super) fn shipping_payload_filename(name: &str, window_log: u32, compressed: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    let digest = Sha256::digest(compressed);
    let hash = hex::encode(&digest[..6]);
    format!(
        "{}-w{window_log}-{hash}.rhmission.zst",
        shipping_file_stem(name)
    )
}

pub(super) fn encode_shipping_payload(
    payload: &ShippingMission,
    window_log: u32,
) -> Result<Vec<u8>> {
    let encoded = robin_assets::shipping_datadir::encode_mission_native(payload);
    robin_assets::shipping_datadir::zstd_compress_with_window(&encoded, window_log)
}

pub(super) fn write_prepared_shipping_payload(
    output_dir: &Path,
    filename: &str,
    compressed: Option<Vec<u8>>,
) -> Result<usize> {
    let path = output_dir.join(filename);
    if let Some(compressed) = compressed {
        let len = compressed.len();
        publication::publish_bytes(&path, &compressed)?;
        Ok(len)
    } else {
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("stat reused payload {}", path.display()))?;
        anyhow::ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "reused payload is not a regular file: {}",
            path.display()
        );
        usize::try_from(metadata.len()).context("reused payload size exceeds address space")
    }
}

/// Return a validated existing content filename, or freshly compressed bytes
/// and their content-addressed filename. Reuse compares the complete decoded
/// native-bitcode payload, so an interrupted run cannot accidentally mix
/// schemas, source data, or converter options.
pub(super) fn prepare_shipping_payload(
    output_dir: &Path,
    label: &str,
    payload: &ShippingMission,
    window_log: u32,
    resume: bool,
) -> Result<(String, Option<Vec<u8>>)> {
    if resume {
        let prefix = format!("{}-w{window_log}-", shipping_file_stem(label));
        let expected = robin_assets::shipping_datadir::encode_mission_native(payload);
        let mut candidates = fs::read_dir(output_dir)
            .with_context(|| format!("read_dir {}", output_dir.display()))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()
            .with_context(|| format!("enumerate resume candidates in {}", output_dir.display()))?
            .into_iter()
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.starts_with(&prefix) && name.ends_with(".rhmission.zst")
                    })
            })
            .collect::<Vec<_>>();
        candidates.sort();
        for path in candidates {
            let metadata = fs::symlink_metadata(&path)
                .with_context(|| format!("stat resume candidate {}", path.display()))?;
            anyhow::ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "resume candidate is not a regular file: {}",
                path.display()
            );
            let compressed = match fs::read(&path) {
                Ok(compressed) => compressed,
                Err(error) => {
                    tracing::warn!(path = %path.display(), %error, "cannot read resume candidate; skipping");
                    continue;
                }
            };
            let decoded = match robin_assets::shipping_datadir::decode_mission_compressed(
                &compressed,
            ) {
                Ok(decoded) => decoded,
                Err(error) => {
                    tracing::warn!(path = %path.display(), %error, "invalid resume candidate; skipping");
                    continue;
                }
            };
            if robin_assets::shipping_datadir::encode_mission_native(&decoded) == expected {
                let filename = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .expect("candidate shipping filename was valid UTF-8")
                    .to_owned();
                tracing::info!(label, filename, "reused validated shipping payload");
                return Ok((filename, None));
            }
        }
    }

    let compressed = encode_shipping_payload(payload, window_log)?;
    let filename = shipping_payload_filename(label, window_log, &compressed);
    Ok((filename, Some(compressed)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrupt_regular_resume_candidates_are_reencoded() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test-w17-invalid.rhmission.zst");
        fs::write(&path, b"incomplete compressed payload").unwrap();
        let payload = ShippingMission::default();
        let (_, compressed) =
            prepare_shipping_payload(directory.path(), "test", &payload, 17, true).unwrap();
        let decoded =
            robin_assets::shipping_datadir::decode_mission_compressed(&compressed.unwrap())
                .unwrap();
        assert_eq!(
            robin_assets::shipping_datadir::encode_mission_native(&decoded),
            robin_assets::shipping_datadir::encode_mission_native(&payload)
        );
        assert_eq!(fs::read(path).unwrap(), b"incomplete compressed payload");
    }

    #[test]
    fn payload_reuse_requires_a_regular_file() {
        let directory = tempfile::tempdir().unwrap();
        let filename = "test-w17-invalid.rhmission.zst";
        let path = directory.path().join(filename);
        fs::create_dir(&path).unwrap();
        assert!(write_prepared_shipping_payload(directory.path(), filename, None).is_err());
        assert!(
            prepare_shipping_payload(
                directory.path(),
                "test",
                &ShippingMission::default(),
                17,
                true
            )
            .unwrap_err()
            .to_string()
            .contains("not a regular file")
        );
        assert!(path.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn payload_reuse_refuses_symlinks_without_reading_the_target() {
        let directory = tempfile::tempdir().unwrap();
        let target = tempfile::NamedTempFile::new().unwrap();
        fs::write(target.path(), b"retained").unwrap();
        let filename = "test-w17-invalid.rhmission.zst";
        let path = directory.path().join(filename);
        std::os::unix::fs::symlink(target.path(), &path).unwrap();
        assert!(write_prepared_shipping_payload(directory.path(), filename, None).is_err());
        assert!(
            prepare_shipping_payload(
                directory.path(),
                "test",
                &ShippingMission::default(),
                17,
                true
            )
            .unwrap_err()
            .to_string()
            .contains("not a regular file")
        );
        assert_eq!(fs::read(target.path()).unwrap(), b"retained");
        assert!(fs::symlink_metadata(path).unwrap().file_type().is_symlink());
    }

    #[test]
    fn streamed_file_digest_matches_bytes_and_length() {
        use sha2::{Digest as _, Sha256};
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("content.bin");
        for length in [0, 1, 8191, 8192, 8193, 128 * 1024, 400_000] {
            let bytes: Vec<u8> = (0..length).map(|index| (index % 251) as u8).collect();
            fs::write(&path, &bytes).unwrap();
            assert_eq!(
                digest_file(&path).unwrap(),
                (length as u64, hex::encode(Sha256::digest(&bytes)))
            );
        }
        assert!(digest_file(&temp.path().join("missing.bin")).is_err());
    }

    #[test]
    fn stale_manifest_stage_is_not_published_as_runtime_content() {
        use robin_rs::multiplayer::content_identity::{
            WEB_CONTENT_MANIFEST_NAME, WebContentEdition,
        };
        for (prefix, subdirectory) in [
            (MANIFEST_STAGING_PREFIX, ""),
            (publication::ARTIFACT_STAGING_PREFIX, ""),
            (publication::ARTIFACT_STAGING_PREFIX, "audio/assets"),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let manifest = temp.path().join(WEB_CONTENT_MANIFEST_NAME);
            let staging_directory = temp.path().join(subdirectory);
            fs::create_dir_all(&staging_directory).unwrap();
            let stage = staging_directory.join(format!("{prefix}abandoned"));
            fs::write(&manifest, b"previous publication").unwrap();
            fs::write(&stage, b"incomplete replacement").unwrap();
            let error =
                write_web_content_manifest(temp.path(), WebContentEdition::Demo, "a".repeat(64))
                    .unwrap_err();
            assert!(error.to_string().contains("manifest stage"));
            assert_eq!(fs::read(manifest).unwrap(), b"previous publication");
            assert_eq!(fs::read(stage).unwrap(), b"incomplete replacement");
        }
    }

    #[cfg(unix)]
    #[test]
    fn published_manifest_uses_normal_output_permissions() {
        use robin_rs::multiplayer::content_identity::{
            WEB_CONTENT_MANIFEST_NAME, WebContentEdition,
        };
        use std::os::unix::fs::PermissionsExt as _;
        let temp = tempfile::tempdir().unwrap();
        let datadir = temp.path().join("datadir.bin");
        fs::write(&datadir, b"boot").unwrap();
        fs::write(temp.path().join("mission.rhmission.zst"), b"mission").unwrap();
        write_web_content_manifest(temp.path(), WebContentEdition::Demo, "a".repeat(64)).unwrap();
        assert_eq!(
            fs::metadata(temp.path().join(WEB_CONTENT_MANIFEST_NAME))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            fs::metadata(datadir).unwrap().permissions().mode() & 0o777,
        );
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 3);
    }

    #[test]
    fn failed_manifest_preparation_preserves_the_previous_publication() {
        use robin_rs::multiplayer::content_identity::{
            WEB_CONTENT_MANIFEST_NAME, WebContentEdition,
        };
        let temp = tempfile::tempdir().unwrap();
        let datadir = temp.path().join("datadir.bin");
        fs::write(&datadir, b"boot").unwrap();
        fs::write(temp.path().join("mission.rhmission.zst"), b"mission").unwrap();
        write_web_content_manifest(temp.path(), WebContentEdition::Demo, "a".repeat(64)).unwrap();
        let manifest = temp.path().join(WEB_CONTENT_MANIFEST_NAME);
        let before = fs::read(&manifest).unwrap();
        fs::remove_file(&datadir).unwrap();
        assert!(
            write_web_content_manifest(temp.path(), WebContentEdition::Demo, "b".repeat(64))
                .is_err()
        );
        assert_eq!(fs::read(&manifest).unwrap(), before);
        fs::write(&datadir, b"boot").unwrap();
        write_web_content_manifest(temp.path(), WebContentEdition::Demo, "b".repeat(64)).unwrap();
        let after: serde_json::Value =
            serde_json::from_slice(&fs::read(manifest).unwrap()).unwrap();
        assert_eq!(after["native_content_sha256"], "b".repeat(64));
        assert_eq!(after["files"].as_array().unwrap().len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn manifest_publication_refuses_a_symlink_without_touching_its_target() {
        use robin_rs::multiplayer::content_identity::{
            WEB_CONTENT_MANIFEST_NAME, WebContentEdition,
        };
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        fs::write(outside.path(), b"retained").unwrap();
        let manifest = temp.path().join(WEB_CONTENT_MANIFEST_NAME);
        std::os::unix::fs::symlink(outside.path(), &manifest).unwrap();
        let error =
            write_web_content_manifest(temp.path(), WebContentEdition::Demo, "a".repeat(64))
                .unwrap_err();
        assert!(error.to_string().contains("symlink"));
        assert!(
            fs::symlink_metadata(manifest)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(outside.path()).unwrap(), b"retained");
    }

    #[test]
    fn manifest_files_keep_case_sensitive_lexical_order() {
        use robin_rs::multiplayer::content_identity::{
            WEB_CONTENT_MANIFEST_NAME, WebContentEdition, WebContentManifest,
        };
        let temp = tempfile::tempdir().unwrap();
        for name in [
            "z.rhmission.zst",
            "datadir.bin",
            "m.rhmission.zst",
            "A.rhmission.zst",
        ] {
            fs::write(temp.path().join(name), name.as_bytes()).unwrap();
        }
        write_web_content_manifest(temp.path(), WebContentEdition::Demo, "a".repeat(64)).unwrap();
        let manifest: WebContentManifest =
            serde_json::from_slice(&fs::read(temp.path().join(WEB_CONTENT_MANIFEST_NAME)).unwrap())
                .unwrap();
        assert_eq!(
            manifest
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            ["A.rhmission.zst", "m.rhmission.zst", "z.rhmission.zst"],
        );
        assert_eq!(manifest.datadir.path, "datadir.bin");
    }

    #[test]
    fn payload_filename_retains_its_truncated_lowercase_digest() {
        assert_eq!(
            shipping_payload_filename("My Mission", 17, b"abc"),
            "my_mission-w17-ba7816bf8f01.rhmission.zst"
        );
    }

    #[test]
    fn diagnostic_plan_does_not_change_web_manifest_bytes() {
        use robin_rs::multiplayer::content_identity::{
            WEB_CONTENT_MANIFEST_NAME, WebContentEdition,
        };
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("datadir.bin"), b"boot fixture").unwrap();
        fs::create_dir(temp.path().join("missions")).unwrap();
        fs::write(
            temp.path().join("missions/test.rhmission.zst"),
            b"mission fixture",
        )
        .unwrap();
        write_web_content_manifest(temp.path(), WebContentEdition::Demo, "a".repeat(64)).unwrap();
        let before = fs::read(temp.path().join(WEB_CONTENT_MANIFEST_NAME)).unwrap();
        fs::write(temp.path().join("conversion-plan.json"), b"{}").unwrap();
        write_web_content_manifest(temp.path(), WebContentEdition::Demo, "a".repeat(64)).unwrap();
        assert_eq!(
            fs::read(temp.path().join(WEB_CONTENT_MANIFEST_NAME)).unwrap(),
            before
        );
    }
}
