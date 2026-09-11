use super::*;
use robin_engine::replay::ReplayRecorder;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Serialize)]
struct FailingWriter {
    #[serde(skip)]
    writer: Box<dyn Write + Send>,
    #[serde(skip)]
    remaining: Arc<AtomicUsize>,
}

impl<'de> Deserialize<'de> for FailingWriter {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> std::result::Result<Self, D::Error> {
        Err(serde::de::Error::custom("test writer is process-owned"))
    }
}

impl Write for FailingWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let budget = self.remaining.load(Ordering::Relaxed);
        if budget == 0 {
            return Err(std::io::Error::other("injected replay write failure"));
        }
        let written = self.writer.write(&bytes[..bytes.len().min(budget)])?;
        self.remaining.fetch_sub(written, Ordering::Relaxed);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}

fn root(directory: &Path) -> (MissionArchive, ReplayHeader, SaveReplayLink) {
    let archive = MissionArchive::create(directory).unwrap();
    let assets =
        robin_engine::mission_assets::MissionAssetDescriptor::built_in("test", "test", "test")
            .unwrap();
    let mut recorder = ReplayRecorder::with_writer(
        archive.writer().unwrap(),
        "test".into(),
        assets,
        0,
        Default::default(),
        &Default::default(),
    )
    .unwrap();
    let marker = ReplaySaveMarker {
        state_hash: 123,
        timeline_frame: 0,
    };
    recorder.write_save_marker(0, marker);
    let link = archive.marker_link(0, marker, [7; 32]);
    assert!(recorder.write_frame(0, 0, 0, Default::default(), Vec::new(), None));
    recorder.flush().unwrap();
    archive.sync_current().unwrap();
    let header = recorder.recording_header().clone();
    (archive, header, link)
}

fn continuation_writer(
    archive: &MissionArchive,
    budget: Arc<AtomicUsize>,
) -> Box<dyn Write + Send> {
    Box::new(ChunkHeaderWriter {
        writer: Box::new(FailingWriter {
            writer: open_chunk_writer(&archive.directory.join(archive.current_chunk())).unwrap(),
            remaining: budget,
        }),
        chunk: archive.active_chunk().clone(),
        header: Some(Vec::new()),
    })
}

fn finish(archive: &mut MissionArchive, header: ReplayHeader) {
    let mut recorder =
        ReplayRecorder::continue_recording(archive.writer().unwrap(), header, 1).unwrap();
    recorder.write_load_back(1, 0, false);
    assert!(recorder.write_frame(1, 0, 0, Default::default(), Vec::new(), None));
    recorder.flush().unwrap();
    archive.publish_continuation().unwrap();
}

fn retained_orphan(directory: &Path) -> PathBuf {
    let mut paths = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".unpublished-replay-")
        });
    let retained = paths
        .next()
        .expect("unpublished bytes retained")
        .join(chunk_name(1));
    assert!(paths.next().is_none());
    retained
}

#[test]
fn incomplete_children_never_publish_and_retries_preserve_orphan_bytes() {
    for failure in [
        "reserved",
        "partial header",
        "header only",
        "restore only",
        "partial restore",
    ] {
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path().join("mission");
        let (mut archive, header, link) = root(&directory);
        let manifest = std::fs::read(directory.join(MANIFEST)).unwrap();
        let original = std::fs::read(directory.join(chunk_name(0))).unwrap();
        archive.stage_continuation(1, Some(link.clone())).unwrap();
        assert!(archive.stage_continuation(1, Some(link.clone())).is_err());
        if failure != "reserved" {
            let budget = Arc::new(AtomicUsize::new(if failure == "partial header" {
                5
            } else {
                usize::MAX
            }));
            let recorder = ReplayRecorder::continue_recording(
                continuation_writer(&archive, budget.clone()),
                header.clone(),
                1,
            );
            if failure == "partial header" {
                assert!(recorder.is_err());
            } else {
                let mut recorder = recorder.unwrap();
                if failure == "restore only" || failure == "partial restore" {
                    if failure == "partial restore" {
                        budget.store(5, Ordering::Relaxed);
                    }
                    recorder.write_load_back(1, 0, false);
                    if failure == "partial restore" {
                        assert!(recorder.write_frame(
                            1,
                            0,
                            0,
                            Default::default(),
                            Vec::new(),
                            None
                        ));
                        assert!(recorder.flush().is_err());
                    } else {
                        recorder.flush().unwrap();
                    }
                }
            }
        }
        assert!(archive.publish_continuation().is_err(), "{failure}");
        assert_eq!(
            std::fs::read(directory.join(MANIFEST)).unwrap(),
            manifest,
            "{failure}"
        );
        assert_eq!(load_directory(&directory).unwrap().frame_count(), 1);
        let orphan = std::fs::read(directory.join(chunk_name(1))).unwrap();
        drop(archive);

        let mut archive = MissionArchive::open(&directory).unwrap();
        assert_eq!(archive.assembled_replay().unwrap().1.frame_count(), 1);
        archive.stage_continuation(1, Some(link)).unwrap();
        assert_eq!(std::fs::read(retained_orphan(&directory)).unwrap(), orphan);
        finish(&mut archive, header);
        assert_eq!(load_directory(&directory).unwrap().frame_count(), 2);
        assert_eq!(
            std::fs::read(directory.join(chunk_name(0))).unwrap(),
            original
        );
    }
}

#[test]
fn manifest_failure_obeys_disk_visibility_and_reopens_valid_history() {
    for visible in [false, true] {
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path().join("mission");
        let (mut archive, header, link) = root(&directory);
        archive.stage_continuation(1, Some(link.clone())).unwrap();
        let mut recorder =
            ReplayRecorder::continue_recording(archive.writer().unwrap(), header.clone(), 1)
                .unwrap();
        recorder.write_load_back(1, 0, false);
        assert!(recorder.write_frame(1, 0, 0, Default::default(), Vec::new(), None));
        recorder.flush().unwrap();
        let error = archive
            .publish_continuation_with(|directory, candidate| {
                if visible {
                    write_manifest(directory, candidate)?;
                }
                anyhow::bail!("injected manifest publication failure");
            })
            .unwrap_err();
        assert!(error.to_string().contains(if visible {
            "is visible"
        } else {
            "was not published"
        }));
        assert_eq!(archive.manifest, read_manifest(&directory).unwrap());
        assert_eq!(archive.pending.is_none(), visible);
        let expected = if visible { 2 } else { 1 };
        assert_eq!(load_directory(&directory).unwrap().frame_count(), expected);
        let unpublished = std::fs::read(directory.join(chunk_name(1))).unwrap();
        drop(recorder);
        drop(archive);
        let mut archive = MissionArchive::open(&directory).unwrap();
        assert_eq!(
            archive.assembled_replay().unwrap().1.frame_count(),
            expected
        );
        if !visible {
            archive.stage_continuation(1, Some(link)).unwrap();
            assert_eq!(
                std::fs::read(retained_orphan(&directory)).unwrap(),
                unpublished
            );
            finish(&mut archive, header);
            assert_eq!(load_directory(&directory).unwrap().frame_count(), 2);
        }
    }
}

#[test]
fn retry_does_not_move_an_unexpected_directory_or_referenced_chunk() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("mission");
    let (mut archive, _, link) = root(&directory);
    let root_bytes = std::fs::read(directory.join(chunk_name(0))).unwrap();
    std::fs::create_dir(directory.join(chunk_name(1))).unwrap();
    assert!(
        archive
            .stage_continuation(1, Some(link))
            .unwrap_err()
            .to_string()
            .contains("not a regular file")
    );
    assert!(directory.join(chunk_name(1)).is_dir());
    assert_eq!(
        std::fs::read(directory.join(chunk_name(0))).unwrap(),
        root_bytes
    );
    assert_eq!(load_directory(&directory).unwrap().frame_count(), 1);
}
