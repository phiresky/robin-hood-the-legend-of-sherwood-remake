//! Kira-backend tests; the whole module is selected with the native backend.
use super::super::tests::one_second_wav;
use super::super::{LocatedSample, locate_sample, sample_base_paths, with_opus_fallback};
use super::*;

#[test]
fn legacy_vorbis_repair_preserves_packets_and_granules() {
    let packets = [
        b"\x01vorbis identification".as_slice(),
        LEGACY_VORBIS_COMMENT,
        b"\x05vorbis setup",
        b"audio packet one",
        b"audio packet two",
    ];
    let mut writer = ogg::PacketWriter::new(Vec::new());
    for (index, packet) in packets.iter().enumerate() {
        let end = if index == packets.len() - 1 {
            ogg::PacketWriteEndInfo::EndStream
        } else {
            ogg::PacketWriteEndInfo::EndPage
        };
        writer
            .write_packet(*packet, 123, end, index as u64 * 1024)
            .unwrap();
    }
    let original = writer.into_inner();
    let repaired = repair_legacy_vorbis_comment(original.clone()).unwrap();
    assert_ne!(repaired.as_ref(), original);
    // PacketReader validates CRCs in the rewritten container as well.
    let mut reader = ogg::PacketReader::new(Cursor::new(&repaired));
    for (index, expected) in packets.iter().enumerate() {
        let packet = reader.read_packet().unwrap().unwrap();
        assert_eq!(packet.stream_serial(), 123);
        assert_eq!(packet.absgp_page(), index as u64 * 1024);
        assert_eq!(packet.last_in_stream(), index == packets.len() - 1);
        if index == 1 {
            assert_eq!(&packet.data[..47], &expected[..47]); // vendor/count
            assert_eq!(&packet.data[47..51], &38u32.to_le_bytes());
            assert_eq!(
                &packet.data[51..],
                b"ENCODER=Sonic Foundry OggVorbis Beta 3\x01"
            );
        } else {
            assert_eq!(&packet.data, expected);
        }
    }
    assert!(reader.read_packet().unwrap().is_none());
    assert_eq!(
        repair_legacy_vorbis_comment(repaired.clone())
            .unwrap()
            .as_ref(),
        repaired.as_ref()
    );
}

#[test]
fn unknown_vorbis_metadata_is_not_rewritten() {
    let mut unknown = LEGACY_VORBIS_COMMENT.to_vec();
    unknown[51] = b'X';
    let mut writer = ogg::PacketWriter::new(Vec::new());
    writer
        .write_packet(unknown, 123, ogg::PacketWriteEndInfo::EndStream, 0)
        .unwrap();
    let original = writer.into_inner();
    assert_eq!(
        repair_legacy_vorbis_comment(original.clone())
            .unwrap()
            .as_ref(),
        original
    );
    let wav = b"RIFF non-Ogg input".to_vec();
    assert_eq!(
        repair_legacy_vorbis_comment(wav.clone()).unwrap().as_ref(),
        wav
    );
}

#[test]
#[ignore = "requires ROBINHOOD_DATA_DIR pointing to original fullgame data"]
fn legacy_vorbis_repair_preserves_decoded_original_music() {
    let root = robin_test_support::original_data::data_directory("");
    for name in ["Menu", "Cast_Fight"] {
        let path = ["DATA/Musics", "Data/Musics"]
            .into_iter()
            .flat_map(|directory| {
                ["wav", "ogg"]
                    .map(|extension| root.join(directory).join(format!("{name}.{extension}")))
            })
            .find(|path| path.is_file())
            .unwrap_or_else(|| {
                panic!(
                    "original music fixture {name} is missing under {}",
                    root.display()
                )
            });
        let bytes = std::fs::read(&path).expect("read original music");
        let repaired = repair_legacy_vorbis_comment(bytes.clone()).unwrap();
        assert_ne!(
            repaired.as_ref(),
            bytes,
            "fixture must contain legacy comment"
        );
        let original = StaticSoundData::from_cursor(Cursor::new(bytes)).unwrap();
        let normalized = StaticSoundData::from_cursor(Cursor::new(repaired)).unwrap();
        assert_eq!(normalized.sample_rate, original.sample_rate);
        assert_eq!(normalized.frames.len(), original.frames.len());
        for (left, right) in original.frames.iter().zip(normalized.frames.iter()) {
            assert_eq!(left.left.to_bits(), right.left.to_bits());
            assert_eq!(left.right.to_bits(), right.right.to_bits());
        }
    }
}

#[test]
fn audio_preparation_seeks_and_loops_without_a_device() {
    let sample = StaticSoundData {
        sample_rate: 4,
        frames: vec![kira::Frame::ZERO; 8].into(),
        settings: Default::default(),
        slice: None,
    };
    let prepared = prepare_sample(sample, 0.25, true, 255);
    assert_eq!(
        prepared.settings.start_position,
        kira::sound::PlaybackPosition::Seconds(0.5)
    );
    assert!(prepared.settings.loop_region.is_some());
}

#[test]
fn music_path_falls_back_from_wav_to_ogg() {
    let temp = tempfile::tempdir().unwrap();
    let ogg = temp.path().join("Lincoln_D.ogg");
    std::fs::write(&ogg, []).unwrap();

    let wav = temp.path().join("Lincoln_D.wav");
    let files = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
    assert_eq!(
        resolver::resolve_music(&files, wav.to_str().unwrap()).unwrap(),
        ogg
    );
}

#[test]
fn audio_cursor_reuses_shared_bytes_and_survives_asset_replacement() {
    let assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
    let path = Path::new("Data/Sounds/shared-cursor.wav");
    let bytes = one_second_wav();
    assets.install_preloaded_asset(path, bytes.clone()).unwrap();
    let files = SbFileSystem::new(assets.clone());
    let shared = files.read_shared(path.to_str().unwrap()).unwrap();
    let cursor = read_audio_cursor(&files, path).unwrap();
    assert_eq!(cursor.get_ref().as_ptr(), shared.as_ptr());
    assets
        .install_preloaded_asset(path, vec![0; bytes.len()])
        .unwrap();
    drop(shared);
    drop(files);
    drop(assets);
    assert_eq!(cursor.get_ref().as_ref(), bytes);
    assert!(StaticSoundData::from_cursor(cursor).is_ok());
}

#[test]
fn native_playback_decoders_use_explicit_vfs_without_a_device() {
    for rate in [44_100_u32, 22_050] {
        let assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
        let mut bytes = one_second_wav();
        bytes[24..28].copy_from_slice(&rate.to_le_bytes());
        bytes[28..32].copy_from_slice(&(rate * 4).to_le_bytes());
        assets
            .install_preloaded_asset("Data/Sounds/Exclamations/reader.wav", bytes.clone())
            .unwrap();
        assets
            .install_preloaded_asset("Data/Music/reader.ogg", bytes)
            .unwrap();
        let files = SbFileSystem::new(assets);
        let sample =
            resolver::resolve_sample(Path::new("Data/Sounds"), "reader.wav", &files).unwrap();
        let data = load_static_sound(&files, &sample).unwrap();
        assert_eq!(data.sample_rate, rate);
        let music = resolver::resolve_music(&files, "Data/Music/reader.wav").unwrap();
        assert_eq!(music, Path::new("Data/Music/reader.ogg"));
        assert!(load_streaming_sound(&files, &music).is_ok());
        let old_key = sample_cache_key(&files, &sample);
        files
            .set_locale_paths(Some("other-locale"), None)
            .expect("change fixture audio locale");
        assert_ne!(sample_cache_key(&files, &sample), old_key);
    }
}

#[test]
fn native_playback_decoders_do_not_reopen_forbidden_absolute_paths() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("sample.wav");
    std::fs::write(&path, one_second_wav()).unwrap();
    let files = SbFileSystem::new(Arc::new(robin_util::asset_fs::AssetVfs::new()));
    files
        .lock_ranked_verifier_primary_path(root.path())
        .expect("confine fixture playback decoders");
    assert!(load_static_sound(&files, Path::new("sample.wav")).is_ok());
    assert!(load_streaming_sound(&files, Path::new("sample.wav")).is_ok());
    assert!(load_static_sound(&files, &path).is_err());
    assert!(load_streaming_sound(&files, &path).is_err());
    assert!(load_static_sound(&files, Path::new("../sample.wav")).is_err());
    assert!(load_streaming_sound(&files, Path::new("../sample.wav")).is_err());
}

#[test]
fn audio_cache_identity_changes_with_mounts_but_frozen_readers_stay_pinned() {
    let assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
    let path = Path::new("Data/Sounds/replaced.wav");
    assets
        .install_preloaded_asset(path, one_second_wav())
        .unwrap();
    let files = SbFileSystem::new(assets.clone());
    let frozen = files.snapshot();
    let old_key = sample_cache_key(&files, path);
    let mut replacement = one_second_wav();
    replacement[24..28].copy_from_slice(&22_050u32.to_le_bytes());
    replacement[28..32].copy_from_slice(&88_200u32.to_le_bytes());
    assets.install_preloaded_asset(path, replacement).unwrap();
    assert_ne!(sample_cache_key(&files, path), old_key);
    assert_eq!(sample_cache_key(&frozen, path), old_key);
    assert_eq!(load_static_sound(&files, path).unwrap().sample_rate, 22_050);
    assert_eq!(
        load_static_sound(&frozen, path).unwrap().sample_rate,
        44_100
    );
}

/// The playback resolver and the sample loader must agree on candidate
/// precedence (authored path, Opus sibling, Exclamations, its Opus sibling).
#[test]
fn resolver_matches_sample_loader_candidate_precedence() {
    let base = Path::new("Data/Sounds");
    let expected: Vec<PathBuf> =
        with_opus_fallback(sample_base_paths(base, "Expressions\\\\voice.wav")).collect();
    for first_available in 0..expected.len() {
        let assets = Arc::new(robin_util::asset_fs::AssetVfs::new());
        for (index, path) in expected.iter().enumerate().skip(first_available) {
            assets
                .install_preloaded_asset(path.to_str().unwrap(), vec![index as u8])
                .unwrap();
        }
        let files = SbFileSystem::new(assets);
        let Some(LocatedSample::Bytes { source_path, .. }) =
            locate_sample(base, "Expressions\\\\voice.wav", &files, None)
        else {
            panic!("an installed candidate must resolve to bytes");
        };
        assert_eq!(
            resolver::resolve_sample(base, "Expressions\\\\voice.wav", &files).unwrap(),
            source_path
        );
    }
}
