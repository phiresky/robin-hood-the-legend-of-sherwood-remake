//! Publish the final boot index after all dependency payloads are prepared.
use super::*;

pub(super) const ARTIFACT_STAGING_PREFIX: &str = ".robin-artifact-";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_replacement_is_complete_and_leaves_no_stage() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("datadir.bin");
        publish_bytes(&path, b"old index").unwrap();
        publish_bytes(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        let invalid = directory.path().join("directory");
        fs::create_dir(&invalid).unwrap();
        assert!(publish_bytes(&invalid, b"replacement").is_err());
        assert!(invalid.is_dir());
        assert_eq!(fs::read(path).unwrap(), b"new");
    }

    #[cfg(unix)]
    #[test]
    fn artifact_replacement_preserves_permissions_and_refuses_symlinks() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("datadir.bin");
        fs::write(&path, b"old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        publish_bytes(&path, b"new").unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        let link = directory.path().join("link");
        symlink(&path, &link).unwrap();
        assert!(publish_bytes(&link, b"bad").is_err());
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(path).unwrap(), b"new");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    }
}

/// Publish a completed converter artifact without exposing a partial write.
pub(super) fn publish_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    let directory = path.parent().context("artifact has no output directory")?;
    let existing_permissions = match fs::symlink_metadata(path) {
        Ok(metadata) => {
            anyhow::ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "artifact target is not a regular file: {}",
                path.display()
            );
            Some(metadata.permissions())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).with_context(|| format!("stat {}", path.display())),
    };
    let mut builder = tempfile::Builder::new();
    builder.prefix(ARTIFACT_STAGING_PREFIX);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        // Public converter artifacts use ordinary output permissions, masked
        // by umask, rather than tempfile's default private-file mode.
        builder.permissions(fs::Permissions::from_mode(0o666));
    }
    let mut temporary = builder
        .tempfile_in(directory)
        .context("stage converter artifact")?;
    std::io::Write::write_all(&mut temporary, bytes).context("write staged converter artifact")?;
    if let Some(permissions) = existing_permissions {
        temporary
            .as_file()
            .set_permissions(permissions)
            .context("preserve artifact permissions")?;
    }
    temporary
        .as_file()
        .sync_all()
        .context("sync staged converter artifact")?;
    temporary
        .persist(path)
        .with_context(|| format!("publish {}", path.display()))?;
    #[cfg(unix)]
    fs::File::open(directory)?
        .sync_all()
        .context("sync artifact directory")?;
    Ok(())
}

pub(super) fn publish_shipping(
    dd: &ShippingDatadir,
    data_out: &Path,
    opts: &ShippingOpts,
) -> Result<()> {
    // Serialize + compress with the configured window log.
    let out_file = data_out.join("datadir.bin");
    let blob = robin_assets::shipping_datadir::encode_native(dd);
    let compressed =
        robin_assets::shipping_datadir::zstd_compress_with_window(&blob, opts.zstd_window_log)?;
    publish_bytes(&out_file, &compressed)?;
    tracing::info!(
        "wrote {} (windowLog={}, map={:?}, audio={:?})",
        out_file.display(),
        opts.zstd_window_log,
        opts.map_format,
        opts.audio_format
    );
    Ok(())
}
