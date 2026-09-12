use super::*;
use std::io::Write;

#[test]
fn every_publication_stage_reports_visibility_and_allows_retry() {
    for stage in [
        PublicationStage::Prepare,
        PublicationStage::Write,
        PublicationStage::SyncFile,
        PublicationStage::Replace,
        PublicationStage::SyncDirectory,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("archive.json");
        write_json(&path, &"old").unwrap();
        let error = publish(
            &path,
            PublicationMode::Replace,
            |file| file.write_all(b"\"new\""),
            |current| {
                if current == stage {
                    Err(io::Error::other("injected failure"))
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();
        let outcome = error
            .get_ref()
            .unwrap()
            .downcast_ref::<PublicationFailure>()
            .unwrap();
        assert_eq!(outcome.stage, stage);
        let visible: String = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(visible, if outcome.published() { "new" } else { "old" });
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        write_json(&path, &"new").unwrap();
        assert_eq!(
            serde_json::from_slice::<String>(&fs::read(path).unwrap()).unwrap(),
            "new"
        );
    }
}

#[test]
fn create_new_publication_failures_preserve_visibility_and_competing_files() {
    for stage in [
        PublicationStage::Prepare,
        PublicationStage::Write,
        PublicationStage::SyncFile,
        PublicationStage::Replace,
        PublicationStage::SyncDirectory,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replay.rhrec");
        let error = publish(
            &path,
            PublicationMode::CreateNew,
            |file| file.write_all(b"complete"),
            |current| {
                if current == stage {
                    Err(io::Error::other("injected failure"))
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();
        let published = error
            .get_ref()
            .unwrap()
            .downcast_ref::<PublicationFailure>()
            .unwrap()
            .published();
        assert_eq!(path.exists(), published);
        if published {
            assert_eq!(fs::read(&path).unwrap(), b"complete");
        } else {
            write_new_bytes(&path, b"retry").unwrap();
            assert_eq!(fs::read(&path).unwrap(), b"retry");
        }
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("replay.rhrec");
    let error = publish(
        &path,
        PublicationMode::CreateNew,
        |file| file.write_all(b"ours"),
        |stage| {
            if stage == PublicationStage::Replace {
                fs::write(&path, b"concurrent winner")?;
            }
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&path).unwrap(), b"concurrent winner");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn partial_create_new_write_does_not_poison_the_final_filename() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("replay.rhrec");
    let error = publish(
        &path,
        PublicationMode::CreateNew,
        |file| {
            file.write_all(b"partial")?;
            Err(io::Error::new(
                io::ErrorKind::StorageFull,
                "injected disk full",
            ))
        },
        |_| Ok(()),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    assert!(!path.exists());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    write_new_bytes(&path, b"complete retry").unwrap();
    assert_eq!(fs::read(path).unwrap(), b"complete retry");
}

#[test]
fn encoded_bytes_are_preserved_and_failed_replacement_cleans_staging() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("datadir.txt");
    write_bytes(&path, b"old\n").unwrap();
    write_bytes(&path, b"/game/data\n").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"/game/data\n");
    let blocked = dir.path().join("directory");
    fs::create_dir(&blocked).unwrap();
    let error = write_bytes(&blocked, b"must not replace directory").unwrap_err();
    let failure = error
        .get_ref()
        .unwrap()
        .downcast_ref::<PublicationFailure>()
        .unwrap();
    assert_eq!(failure.stage, PublicationStage::Replace);
    assert!(!failure.published());
    assert!(blocked.is_dir());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn partial_write_preserves_live_archive() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("archive.json");
    write_json(&path, &"old").unwrap();
    let error = publish(
        &path,
        PublicationMode::Replace,
        |file| {
            file.write_all(b"{partial")?;
            Err(io::Error::new(
                io::ErrorKind::StorageFull,
                "injected disk full",
            ))
        },
        |_| Ok(()),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    assert_eq!(fs::read(&path).unwrap(), b"\"old\"");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn serialization_failure_preserves_live_archive() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("archive.json");
    write_json(&path, &"old").unwrap();
    // JSON cannot encode a compound map key. The map serializer has already
    // started the staged object when it discovers the unsupported key.
    let invalid = std::collections::BTreeMap::from([((1, 2), "value")]);
    let error = write_json(&path, &invalid).unwrap_err();
    assert!(
        !error
            .get_ref()
            .unwrap()
            .downcast_ref::<PublicationFailure>()
            .unwrap()
            .published()
    );
    assert_eq!(fs::read(&path).unwrap(), b"\"old\"");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn abandoned_staging_file_does_not_replace_live_archive_on_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("archive.json");
    write_json(&path, &"old").unwrap();
    fs::write(
        dir.path().join(".robin-user-store-staging-abandoned"),
        b"{partial",
    )
    .unwrap();
    assert_eq!(
        serde_json::from_slice::<String>(&fs::read(&path).unwrap()).unwrap(),
        "old"
    );
    write_json(&path, &"new").unwrap();
    assert_eq!(
        serde_json::from_slice::<String>(&fs::read(&path).unwrap()).unwrap(),
        "new"
    );
}
