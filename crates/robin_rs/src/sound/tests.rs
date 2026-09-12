use super::*;

// Frozen pre-split wire declaration: catches missing/reordered fields and
// changed historical skipped-field defaults independently of the new DTO.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename = "SoundManager")]
struct HistoricalSoundManager {
    // ── Sub-systems ──
    pub sound_cache: SoundCache,
    pub geometry_engine: SoundGeometry,

    // ── Configuration ──
    sound_enabled: bool,
    music_directory: String,
    sound_system_ready: bool,
    active: bool,
    sound_mode: SoundMode,
    use_3d_sound: bool,
    /// Backend-reported "can do positional/3D sound" capability.
    /// Cached at `initialize` time; the sounds menu reads it via
    /// [`SoundManager::can_3d_sound`] to gate the EAX radio.
    #[serde(skip)]
    can_3d_sound: bool,
    /// Backend-reported "can do EAX environmental reverb" capability.
    /// Cached at `initialize`; the sounds menu reads it via
    /// [`SoundManager::can_eax_sound`] to choose between the "EAX" and
    /// "3D" label.
    #[serde(skip)]
    can_eax_sound: bool,
    forest_level: bool,

    // ── Channel tracking ──
    num_channels: u32,
    #[serde(skip)]
    channel_info: Vec<ChannelInfo>,

    // ── Pending sounds ──
    #[serde(skip)]
    pending_sounds: Vec<PendingSoundInfo>,
    #[serde(skip)]
    fx_to_play: Vec<FxToPlay>,

    // ── Music state ──
    music_mode: MusicMode,
    loop_index: i16,
    quiet_mode_weight: u32,
    alert_mode_weight: u32,
    fight_mode_weight: u32,

    // ── Flags ──
    #[serde(skip)]
    has_mission_music: bool,
    #[serde(skip)]
    has_menu_music: bool,
    load_music: bool,
    start_music: bool,
    #[serde(skip)]
    update_music: bool,
    #[serde(skip)]
    update_pending_sounds: bool,

    // ── Jingle / Dialog ──
    #[serde(skip)]
    jingle_channel: Option<i32>,
    #[serde(skip)]
    stop_jingle: bool,
    dialog_mode: bool,
    #[serde(skip)]
    dialog_finished: bool,
    #[serde(skip)]
    has_dialog: bool,
    #[serde(skip)]
    stop_dialog: bool,

    // ── Deferred jingle ──
    #[serde(skip)]
    pending_jingle: Option<Jingle>,
}

#[test]
fn persisted_projection_preserves_historical_wire_and_reconstructs_scratch() {
    let mut manager = SoundManager::new();
    manager.persisted.sound_system_ready = true;
    manager.persisted.active = true;
    manager.persisted.music_mode = MusicMode::Fight;
    manager.persisted.quiet_mode_weight = 73;
    manager.persisted.music_directory = "music/nondefault".into();
    manager
        .persisted
        .sound_cache
        .initialize_music("green", "yellow", "red");
    let cache = manager.sound_cache_mut();
    cache.fx_cache.add_entry("nondefault.wav");
    cache.fx_cache.entries[0].sample_data = Some(vec![1, 2, 3, 4]);
    cache.fx_cache.entries[0].playing = 2;
    cache.fx_cache.entries[0].time_to_live = 75;
    manager.set_listen_point(MapPoint::new(123.0, 456.0), 1.5);
    let settings = SoundSettings {
        sound_type: SoundType::Exclamation,
        position: MapPoint::default(),
        identifier: 4,
        source: SoundSettingsSource::Position { material: 0 },
    };
    manager.runtime = SoundRuntime {
        can_3d_sound: true,
        can_eax_sound: true,
        channel_info: vec![ChannelInfo::default()],
        pending_sounds: vec![PendingSoundInfo {
            settings: settings.clone(),
            channel: PendingChannel::Assigned(0),
            start_time_ms: 20,
            length_ms: 400,
            actor_id: Some(3),
            source_index: None,
            speech_variant: Some(2),
        }],
        fx_to_play: vec![FxToPlay {
            settings,
            params: PlayingParameters::default(),
        }],
        has_mission_music: true,
        has_menu_music: true,
        update_music: true,
        update_pending_sounds: true,
        jingle_channel: Some(0),
        stop_jingle: true,
        dialog_finished: true,
        has_dialog: true,
        stop_dialog: true,
        pending_jingle: Some(Jingle::MissionWon),
    };
    let historical = HistoricalSoundManager {
        sound_cache: manager.persisted.sound_cache.clone(),
        geometry_engine: manager.persisted.geometry_engine.clone(),
        sound_enabled: manager.persisted.sound_enabled,
        music_directory: manager.persisted.music_directory.clone(),
        sound_system_ready: manager.persisted.sound_system_ready,
        active: manager.persisted.active,
        sound_mode: manager.persisted.sound_mode,
        use_3d_sound: manager.persisted.use_3d_sound,
        can_3d_sound: manager.runtime.can_3d_sound,
        can_eax_sound: manager.runtime.can_eax_sound,
        forest_level: manager.persisted.forest_level,
        num_channels: manager.persisted.num_channels,
        channel_info: manager.runtime.channel_info.clone(),
        pending_sounds: manager.runtime.pending_sounds.clone(),
        fx_to_play: manager.runtime.fx_to_play.clone(),
        music_mode: manager.persisted.music_mode,
        loop_index: manager.persisted.loop_index,
        quiet_mode_weight: manager.persisted.quiet_mode_weight,
        alert_mode_weight: manager.persisted.alert_mode_weight,
        fight_mode_weight: manager.persisted.fight_mode_weight,
        has_mission_music: manager.runtime.has_mission_music,
        has_menu_music: manager.runtime.has_menu_music,
        load_music: manager.persisted.load_music,
        start_music: manager.persisted.start_music,
        update_music: manager.runtime.update_music,
        update_pending_sounds: manager.runtime.update_pending_sounds,
        jingle_channel: manager.runtime.jingle_channel,
        stop_jingle: manager.runtime.stop_jingle,
        dialog_mode: manager.persisted.dialog_mode,
        dialog_finished: manager.runtime.dialog_finished,
        has_dialog: manager.runtime.has_dialog,
        stop_dialog: manager.runtime.stop_dialog,
        pending_jingle: manager.runtime.pending_jingle,
    };
    let wire = serde_json::to_string(&historical).unwrap();
    assert_eq!(wire, serde_json::to_string(&manager).unwrap());
    let projected = PersistedSoundManager::capture(&manager);
    assert_eq!(wire, serde_json::to_string(&projected).unwrap());
    let restored = projected.into_runtime();
    assert_eq!(restored.sound_cache().fx_cache.entries[0].playing, 2);
    assert_eq!(
        restored.sound_cache().fx_cache.entries[0].sample_data,
        Some(vec![1, 2, 3, 4])
    );
    let decoded: SoundManager = serde_json::from_str(&wire).unwrap();
    let old_decoded: HistoricalSoundManager = serde_json::from_str(&wire).unwrap();
    assert_eq!(wire, serde_json::to_string(&restored).unwrap());
    assert_eq!(wire, serde_json::to_string(&decoded).unwrap());
    let empty_runtime = serde_json::to_value(SoundRuntime::default()).unwrap();
    let historical_runtime = SoundRuntime {
        can_3d_sound: old_decoded.can_3d_sound,
        can_eax_sound: old_decoded.can_eax_sound,
        channel_info: old_decoded.channel_info.clone(),
        pending_sounds: old_decoded.pending_sounds.clone(),
        fx_to_play: old_decoded.fx_to_play.clone(),
        has_mission_music: old_decoded.has_mission_music,
        has_menu_music: old_decoded.has_menu_music,
        update_music: old_decoded.update_music,
        update_pending_sounds: old_decoded.update_pending_sounds,
        jingle_channel: old_decoded.jingle_channel,
        stop_jingle: old_decoded.stop_jingle,
        dialog_finished: old_decoded.dialog_finished,
        has_dialog: old_decoded.has_dialog,
        stop_dialog: old_decoded.stop_dialog,
        pending_jingle: old_decoded.pending_jingle,
    };
    assert_eq!(
        empty_runtime,
        serde_json::to_value(historical_runtime).unwrap()
    );
    assert_eq!(
        empty_runtime,
        serde_json::to_value(&restored.runtime).unwrap()
    );
    assert_eq!(
        empty_runtime,
        serde_json::to_value(&decoded.runtime).unwrap()
    );
    assert_eq!(
        restored.runtime.dialog_finished,
        old_decoded.dialog_finished
    );
    assert!(!restored.runtime.dialog_finished);
    assert!(manager.runtime.dialog_finished);
    assert_eq!(manager.runtime.pending_sounds.len(), 1);
    assert_eq!(manager.clone().runtime.pending_sounds.len(), 1);
    assert_ne!(
        empty_runtime,
        serde_json::to_value(&manager.runtime).unwrap()
    );
}

#[test]
fn audio_gain_contract_preserves_legacy_music_and_channel_scales() {
    assert_eq!(music_gain(0), 0.0);
    assert_eq!(music_gain(64), 0.5);
    assert_eq!(music_gain(128), 1.0);
    assert_eq!(music_gain(u16::MAX), 1.0);
    assert_eq!(channel_gain(0), 0.0);
    assert_eq!(channel_gain(255), 1.0);
    assert!(channel_gain(128) < 0.51);
    assert_eq!(channel_gain(u16::MAX), 1.0);
}

#[test]
fn audio_seek_contract_handles_invalid_and_boundary_offsets() {
    assert_eq!(playback_fraction(-1.0), 0.0);
    assert_eq!(playback_fraction(0.25), 0.25);
    assert_eq!(playback_fraction(1.0), 0.999);
    assert_eq!(playback_fraction(f32::NAN), 0.0);
    assert_eq!(playback_fraction(f32::INFINITY), 0.0);
}

#[test]
fn audio_channel_reuse_retires_previous_jingle_ownership() {
    let mut manager = SoundManager::new();
    manager.runtime.channel_info = vec![ChannelInfo::default()];
    manager.runtime.jingle_channel = Some(0);
    manager.runtime.pending_sounds.push(PendingSoundInfo {
        settings: SoundSettings {
            sound_type: SoundType::Exclamation,
            position: MapPoint::default(),
            identifier: 1,
            source: SoundSettingsSource::Position { material: 0 },
        },
        channel: PendingChannel::Assigned(0),
        start_time_ms: 0,
        length_ms: 1000,
        actor_id: Some(1),
        source_index: None,
        speech_variant: None,
    });
    manager.update_channel_info(0, SoundType::Fx, None, None);
    assert_eq!(manager.runtime.jingle_channel, None);
    assert!(manager.runtime.stop_jingle);
    assert_eq!(
        manager.runtime.pending_sounds[0].channel,
        PendingChannel::Finished
    );
}

#[test]
fn pending_expiry_preserves_survivor_order_without_stopping_mixer_channels() {
    let mut manager = SoundManager::new();
    let mut backend = MockBackend::new();
    let mut channels = Vec::new();
    for (actor, length_ms) in [(1u32, 5), (2, 20), (3, 10), (4, 30), (5, 1)] {
        let channel = backend.play_sound("speech.wav", false).unwrap();
        channels.push(channel);
        manager.runtime.pending_sounds.push(PendingSoundInfo {
            settings: SoundSettings {
                sound_type: SoundType::Exclamation,
                position: MapPoint::default(),
                identifier: actor,
                source: SoundSettingsSource::Position { material: 0 },
            },
            channel: PendingChannel::Assigned(channel),
            start_time_ms: 0,
            length_ms,
            actor_id: Some(actor),
            source_index: None,
            speech_variant: None,
        });
    }
    backend.ticks = 10;
    let resolved = manager.process_pending_sounds(
        &mut backend,
        &|_| panic!("known lengths do not need reloading"),
        &mut |_| panic!("assigned sounds do not need reselection"),
        &SoundSourceManager::new(),
    );
    assert!(resolved.is_empty());
    assert_eq!(
        manager
            .runtime
            .pending_sounds
            .iter()
            .map(|pending| pending.actor_id)
            .collect::<Vec<_>>(),
        [Some(2), Some(4)]
    );
    assert!(
        channels
            .into_iter()
            .all(|channel| backend.is_channel_playing(channel))
    );
}

#[test]
fn localized_voice_channel_is_not_cut_off_by_pending_expiry() {
    let mut manager = SoundManager::new();
    let mut backend = MockBackend::new();
    let channel = backend.play_sound("french.wav", false).unwrap();
    manager.runtime.pending_sounds.push(PendingSoundInfo {
        settings: SoundSettings {
            sound_type: SoundType::Exclamation,
            position: MapPoint::default(),
            identifier: 1,
            source: SoundSettingsSource::Position { material: 0 },
        },
        channel: PendingChannel::Assigned(channel),
        start_time_ms: 0,
        length_ms: 1000,
        actor_id: Some(1),
        source_index: None,
        speech_variant: None,
    });
    backend.ticks = 1001;
    manager.process_pending_sounds(
        &mut backend,
        &|_| panic!("already loaded"),
        &mut |_| 0,
        &SoundSourceManager::new(),
    );
    assert!(manager.runtime.pending_sounds.is_empty());
    assert!(
        backend.is_channel_playing(channel),
        "only the mixer or an explicit stop ends localized playback"
    );
}

/// Minimal mock audio backend for testing.
struct MockBackend {
    channels_playing: Vec<bool>,
    music_finished_flag: bool,
    ticks: u32,
    next_channel: i32,
    music_volume: u16,
}

impl MockBackend {
    fn new() -> Self {
        Self {
            channels_playing: vec![false; NUM_CHANNELS as usize],
            music_finished_flag: false,
            ticks: 0,
            next_channel: 0,
            music_volume: 0,
        }
    }
}

impl AudioBackend for MockBackend {
    fn play_sound(&mut self, _file: &str, _looping: bool) -> Option<i32> {
        let ch = self.next_channel;
        if (ch as usize) < self.channels_playing.len() {
            self.channels_playing[ch as usize] = true;
            self.next_channel = (ch + 1) % NUM_CHANNELS as i32;
            Some(ch)
        } else {
            None
        }
    }
    fn play_sound_at(&mut self, file: &str, looping: bool, _pos: f32) -> Option<i32> {
        self.play_sound(file, looping)
    }
    fn halt_channel(&mut self, ch: i32) {
        if let Some(v) = self.channels_playing.get_mut(ch as usize) {
            *v = false;
        }
    }
    fn set_channel_volume(&mut self, _ch: i32, _vol: u16) {}
    fn is_channel_playing(&self, ch: i32) -> bool {
        self.channels_playing
            .get(ch as usize)
            .copied()
            .unwrap_or(false)
    }
    fn pause_channels(&mut self, _ch: i32) {}
    fn resume_channels(&mut self, _ch: i32) {}
    fn play_music(&mut self, _path: &str, _looping: bool) -> bool {
        true
    }
    fn halt_music(&mut self) {}
    fn pause_music(&mut self) {}
    fn resume_music(&mut self) {}
    fn set_music_volume(&mut self, v: u16) {
        self.music_volume = v;
    }
    fn get_music_volume(&self) -> u16 {
        self.music_volume
    }
    fn take_music_finished(&mut self) -> bool {
        let v = self.music_finished_flag;
        self.music_finished_flag = false;
        v
    }
    fn play_jingle(&mut self, _path: &str) -> Option<i32> {
        self.play_sound("jingle", false)
    }
    fn free_jingle(&mut self) {}
    fn get_ticks(&self) -> u32 {
        self.ticks
    }
    fn num_channels(&self) -> u32 {
        NUM_CHANNELS
    }
}

#[test]
fn sound_manager_new() {
    let mgr = SoundManager::new();
    assert!(!mgr.is_ready());
    assert!(!mgr.is_active());
    assert_eq!(mgr.music_mode(), MusicMode::Quiet);
}

#[test]
fn initialize_and_activate() {
    let mut mgr = SoundManager::new();
    let mut backend = MockBackend::new();
    mgr.initialize(&mut backend, false).unwrap();
    assert!(mgr.is_ready());

    let sources = SoundSourceManager::new();
    mgr.activate(false, &sources);
    assert!(mgr.is_active());
}

#[test]
fn speech_stop_commands_keep_channel_and_queue_scope_distinct() {
    let mut manager = SoundManager::new();
    let mut backend = MockBackend::new();
    manager.initialize(&mut backend, false).unwrap();
    manager.activate(false, &SoundSourceManager::new());
    let mut channels = Vec::new();
    for actor in [1u32, 2] {
        let channel = backend.play_sound("speech.wav", false).unwrap();
        manager.update_channel_info(channel, SoundType::Exclamation, None, Some(actor));
        manager.play_exclamation(
            ExclamationGroup::Pc,
            0,
            1,
            EXCLAMATION_VARIANT_NONE,
            MapPoint::default(),
            Some(actor),
        );
        channels.push(channel);
    }
    manager.stop_exclamation_channel_only(1, &mut backend);
    assert!(!backend.is_channel_playing(channels[0]));
    assert!(backend.is_channel_playing(channels[1]));
    assert_eq!(manager.num_pending_sounds(), 2);

    manager.stop_exclamation(1, &mut backend);
    assert_eq!(manager.num_pending_sounds(), 1);
    assert_eq!(manager.runtime.pending_sounds[0].actor_id, Some(2));
    assert!(backend.is_channel_playing(channels[1]));
    manager.stop_exclamation(2, &mut backend);
    assert!(!backend.is_channel_playing(channels[1]));
    assert_eq!(manager.num_pending_sounds(), 0);
}

#[test]
fn dialogue_commands_update_runtime_without_status_placeholders() {
    let mut manager = SoundManager::new();
    let mut backend = MockBackend::new();
    manager.play_dialog("dialogue.wav", &mut backend);
    assert!(!manager.runtime.has_dialog);
    manager.close_dialog(&mut backend);
    manager.initialize(&mut backend, false).unwrap();
    manager.play_dialog("dialogue.wav", &mut backend);
    assert!(manager.runtime.has_dialog);
    assert!(!manager.is_dialog_finished());
    manager.close_dialog(&mut backend);
    assert!(!manager.runtime.has_dialog);
}

#[test]
fn music_mode_weights() {
    let mut mgr = SoundManager::new();
    mgr.set_music_mode(MusicMode::Quiet);
    assert_eq!(mgr.quiet_mode_weight(), MUSIC_MODE_WEIGHT);

    mgr.set_music_mode(MusicMode::Fight);
    assert_eq!(mgr.fight_mode_weight(), MUSIC_MODE_WEIGHT);

    mgr.force_music_mode(MusicMode::Alert);
    assert_eq!(mgr.quiet_mode_weight(), 0);
    assert_eq!(mgr.alert_mode_weight(), MUSIC_MODE_WEIGHT);
    assert_eq!(mgr.fight_mode_weight(), 0);
}

#[test]
fn calm_music_replaces_stale_combat_weights() {
    let mut mgr = SoundManager::new();
    mgr.persisted.music_mode = MusicMode::Fight;
    mgr.persisted.fight_mode_weight = 256;
    mgr.set_music_mode(MusicMode::Quiet);
    assert!(mgr.persisted.load_music);
    assert_eq!(mgr.fight_mode_weight(), 0);
    assert_eq!(mgr.alert_mode_weight(), 0);
    assert_eq!(mgr.quiet_mode_weight(), MUSIC_MODE_WEIGHT);
}

#[test]
fn terminal_jingle_cancels_pending_mission_music() {
    let mut mgr = SoundManager::new();
    let mut backend = MockBackend::new();
    mgr.initialize(&mut backend, false).unwrap();
    mgr.persisted.load_music = true;
    mgr.persisted.start_music = true;
    mgr.runtime.has_mission_music = true;
    mgr.play_jingle(Jingle::MissionWon, &mut backend);
    assert!(mgr.runtime.jingle_channel.is_some());
    assert!(!mgr.persisted.load_music);
    assert!(!mgr.persisted.start_music);
    assert!(!mgr.runtime.has_mission_music);
}

#[test]
fn music_mode_forest() {
    let mut mgr = SoundManager::new();
    mgr.persisted.forest_level = true;
    // In forest levels, Quiet falls through to Alert
    mgr.set_music_mode(MusicMode::Quiet);
    assert_eq!(mgr.quiet_mode_weight(), 0);
    assert_eq!(mgr.alert_mode_weight(), MUSIC_MODE_WEIGHT);
}

#[test]
fn time_elapsed_basic() {
    assert_eq!(time_elapsed(100, 200), 100);
    assert_eq!(time_elapsed(0, 0), 0);
}

#[test]
fn time_elapsed_wrap() {
    assert_eq!(time_elapsed(u32::MAX - 10, 5), 16);
}

#[test]
fn strike_material_table_symmetric() {
    // Indexed loop intentional: we need to compare [i][j] with the
    // transposed [j][i], so an iterator over rows alone wouldn't help.
    #[allow(clippy::needless_range_loop)]
    for i in 0..4 {
        for j in 0..4 {
            assert_eq!(
                STRIKE_MATERIAL_TABLE[i][j], STRIKE_MATERIAL_TABLE[j][i],
                "STRIKE_TABLE[{i}][{j}] != [{j}][{i}]"
            );
        }
    }
}

#[test]
fn combat_fx_strike_id_range() {
    for kind in 0..3u32 {
        for combo in 0..10u32 {
            for variant in 0..2u32 {
                let id = (kind * MAX_STRIKE_FX + combo) * 2 + variant;
                assert!(id < 60, "Strike ID {id} out of range");
            }
        }
    }
}

#[test]
fn combat_fx_impact_id_range() {
    // The PlayImpactFx quirk: `3 * MAX_STRIKE_FX` as offset, not
    // `3 * 10 * 2`. Ids land inside the parade range (30..54),
    // which is the sound players recognize as "sword hit".
    for kind in 0..2u32 {
        for combo in 0..12u32 {
            let id = 3 * MAX_STRIKE_FX + kind * MAX_IMPACT_FX + combo;
            assert!((30..54).contains(&id), "Impact ID {id} out of range 30..54");
        }
    }
}

#[test]
fn deactivate_clears_state() {
    let mut mgr = SoundManager::new();
    let mut backend = MockBackend::new();
    mgr.initialize(&mut backend, false).unwrap();
    let mut sources = SoundSourceManager::new();
    mgr.activate(false, &sources);
    assert!(mgr.is_active());

    mgr.deactivate(true, &mut backend, &mut sources);
    assert!(!mgr.is_active());
    assert_eq!(mgr.num_pending_sounds(), 0);
}

#[test]
fn jingle_files_count() {
    assert_eq!(JINGLE_FILES.len(), 8);
}

#[test]
fn dialog_finished_when_not_ready() {
    let mgr = SoundManager::new();
    assert!(mgr.is_dialog_finished());
}

#[test]
fn serde_roundtrip() {
    let mut mgr = SoundManager::new();
    mgr.persisted.music_mode = MusicMode::Fight;
    mgr.persisted.quiet_mode_weight = 100;
    mgr.persisted.forest_level = true;

    let json = serde_json::to_string(&mgr).unwrap();
    let restored: SoundManager = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.persisted.music_mode, MusicMode::Fight);
    assert_eq!(restored.persisted.quiet_mode_weight, 100);
    assert!(restored.persisted.forest_level);
    // Transient state should be default after deserialization
    assert!(restored.runtime.pending_sounds.is_empty());
    assert!(restored.runtime.channel_info.is_empty());
}

#[test]
fn material_from_u8_valid() {
    assert_eq!(material_from_u8(0), Some(Material::Ground));
    assert_eq!(material_from_u8(8), Some(Material::Hole));
    assert_eq!(material_from_u8(9), None);
    assert_eq!(material_from_u8(255), None);
}

#[test]
fn hourglass_empty_does_not_crash() {
    let mut mgr = SoundManager::new();
    let mut backend = MockBackend::new();
    mgr.initialize(&mut backend, false).unwrap();
    let sources = SoundSourceManager::new();
    mgr.activate(false, &sources);

    let loader: Box<SampleLoader> = Box::new(|_| None);
    let mut rng = |_: u32| 0u32;

    let mut pending_play = Vec::new();
    mgr.hourglass(
        &mut backend,
        &loader,
        &mut rng,
        AlertStatus::Green,
        &sources,
        &mut pending_play,
    );
    // Should not panic or crash
}

#[test]
fn listen_point_triggers_update() {
    let mut mgr = SoundManager::new();
    mgr.runtime.update_pending_sounds = false;
    mgr.set_listen_point(MapPoint::new(100.0, 200.0), 1.0);
    assert!(mgr.runtime.update_pending_sounds);
}
