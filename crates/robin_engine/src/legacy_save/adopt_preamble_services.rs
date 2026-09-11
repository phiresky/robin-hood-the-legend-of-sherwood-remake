//! Linux-v48 adoption for sound, messenger, and game preamble state.
//!
//! Host playback channels, stream seeking, and widget mutation are returned as
//! presentation state. They are never invoked during simulation adoption.

use thiserror::Error;

use crate::{
    coordinates::MapPoint,
    engine::EngineInner,
    profiles::Action,
    sound::SoundSimState,
    sound_geometry::SoundSourceAltitude,
    sound_source::{SoundSource, SoundSourceKind, SoundSourceManager},
};

use super::{
    LegacySaveAbiProfile,
    engine::{
        LegacyEnginePreamble, LegacyGameState, LegacySerializedSound, LegacySound,
        LegacySoundSource,
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyPreambleHostState {
    pub sound_system_ready: Option<bool>,
    pub three_d_sound: Option<bool>,
    pub sound_active: Option<bool>,
    pub dummy_channel: Option<i16>,
    pub stream_position: Option<u32>,
    /// Original presentation flag, not simulation state. Native persistence
    /// belongs to Host/GamePersistent.
    /// TODO: consume this Original-import host handoff; returning it does not
    /// currently apply the flag to the live host.
    pub draw_hidden: bool,
    pub campaign_map_displayed: bool,
    pub start_mission_widget_enabled: bool,
    pub quit_mission_widget_enabled: bool,
}

impl LegacyPreambleHostState {
    fn from_preamble_parts(
        sound: &LegacySound,
        messenger: &super::engine::LegacyMessenger,
        game: &LegacyGameState,
    ) -> Self {
        let sound = sound.state.as_ref();
        Self {
            sound_system_ready: sound.map(|state| state.sound_system_ready),
            three_d_sound: sound.map(|state| state.three_d_sound),
            sound_active: sound.map(|state| state.active),
            dummy_channel: sound.map(|state| state.dummy_channel),
            stream_position: sound.map(|state| state.stream_position),
            draw_hidden: messenger.draw_hidden,
            campaign_map_displayed: game.campaign_map_displayed,
            start_mission_widget_enabled: game.start_mission_enabled
                && !game.start_mission_disabled_temp,
            quit_mission_widget_enabled: game.quit_mission_enabled
                && !game.quit_mission_disabled_temp,
        }
    }
}

#[derive(Debug)]
pub(crate) struct LegacyPreambleServicesPlan {
    sound: Option<SoundSimState>,
    view_locked: bool,
    selected_action: Action,
    game: LegacyGameState,
    host: LegacyPreambleHostState,
}

impl LegacyPreambleServicesPlan {
    pub(crate) fn apply(self, engine: &mut EngineInner) -> LegacyPreambleHostState {
        if let Some(sound) = self.sound {
            engine.feedback.sound_sim = sound;
        }
        engine.players.view_locked = self.view_locked;
        engine.players.seats[0].selected_action = self.selected_action;
        // Original messenger restoration clears pending messages at this boundary.
        engine.orders.messenger.clear();

        let ui = &mut engine.script_domains.mission_ui;
        ui.men_to_blazon_conversion_mode = self.game.men_to_blazon_conversion;
        ui.campaign_map = self.game.campaign_map;
        ui.campaign_map_displayed = self.game.campaign_map_displayed;
        ui.game_post_initialized = self.game.post_initialized;
        ui.start_mission_disabled_temp = self.game.start_mission_disabled_temp;
        ui.quit_mission_disabled_temp = self.game.quit_mission_disabled_temp;
        ui.start_mission_enabled = self.game.start_mission_enabled;
        ui.quit_mission_enabled = self.game.quit_mission_enabled;

        self.host
    }
}

#[derive(Clone, Debug, Error, PartialEq)]
pub(crate) enum LegacyPreambleServicesError {
    #[error("saved messenger action {value} is not a known action")]
    InvalidMessengerAction { value: u16 },
    #[error("saved sound field {field} has value {value}; expected {expected}")]
    InvalidSoundField {
        field: &'static str,
        value: String,
        expected: &'static str,
    },
    #[error("saved sound slot {slot} stores slot_index {value}; expected its exact array ordinal")]
    WrongSoundSlotIndex { slot: usize, value: i16 },
    #[error(
        "saved sound slot {slot} registration id {registration_id} differs from source sample id {source_id}"
    )]
    WrongSoundRegistration {
        slot: usize,
        registration_id: u32,
        source_id: u32,
    },
    #[error(
        "saved sound slot {slot} serializes the same active member twice with different values ({first}, {second})"
    )]
    InconsistentSoundActive {
        slot: usize,
        first: bool,
        second: bool,
    },
}

pub(crate) fn preflight_v48_preamble_services(
    _abi: LegacySaveAbiProfile,
    preamble: &LegacyEnginePreamble,
) -> Result<LegacyPreambleServicesPlan, LegacyPreambleServicesError> {
    let selected_action = convert_messenger_action(preamble.messenger.action)?;
    let sound = convert_sound(&preamble.sound)?;
    let host = LegacyPreambleHostState::from_preamble_parts(
        &preamble.sound,
        &preamble.messenger,
        &preamble.game,
    );

    Ok(LegacyPreambleServicesPlan {
        sound,
        view_locked: preamble.messenger.lock_view,
        selected_action,
        game: preamble.game,
        host,
    })
}

fn convert_messenger_action(value: u16) -> Result<Action, LegacyPreambleServicesError> {
    Action::try_from(u32::from(value))
        .map_err(|_| LegacyPreambleServicesError::InvalidMessengerAction { value })
}

fn convert_sound(
    saved: &LegacySound,
) -> Result<Option<SoundSimState>, LegacyPreambleServicesError> {
    match (&saved.serialized, &saved.state) {
        (false, None) => Ok(None),
        (true, Some(state)) => convert_serialized_sound(state).map(Some),
        (serialized, state) => Err(invalid_sound(
            "serialized/state",
            format!("{serialized}/{state:?}"),
            "false/None or true/Some",
        )),
    }
}

fn convert_serialized_sound(
    saved: &LegacySerializedSound,
) -> Result<SoundSimState, LegacyPreambleServicesError> {
    validate_finite("geometry.listen_point.x", saved.geometry.listen_point.x)?;
    validate_finite("geometry.listen_point.y", saved.geometry.listen_point.y)?;
    validate_finite("geometry.zoom_factor", saved.geometry.zoom_factor)?;
    if saved.geometry.zoom_factor <= 0.0 {
        return Err(invalid_sound(
            "geometry.zoom_factor",
            saved.geometry.zoom_factor,
            "a finite positive zoom",
        ));
    }
    match saved.music_mode {
        0..=2 => {}
        value => {
            return Err(invalid_sound(
                "music_mode",
                value,
                "MODE_QUIET..=MODE_FIGHT (0..=2)",
            ));
        }
    }
    let mut sources = SoundSourceManager::new();
    for (slot, entry) in saved.source_manager.slots.iter().enumerate() {
        let Some(entry) = entry else {
            sources.sources_push_none();
            continue;
        };
        let expected_slot = (slot as u16) as i16;
        if entry.slot_index != expected_slot {
            return Err(LegacyPreambleServicesError::WrongSoundSlotIndex {
                slot,
                value: entry.slot_index,
            });
        }
        if entry.registration_id != entry.source.id {
            return Err(LegacyPreambleServicesError::WrongSoundRegistration {
                slot,
                registration_id: entry.registration_id,
                source_id: entry.source.id,
            });
        }
        sources.sources_push_some(convert_source(slot, &entry.source)?);
    }

    Ok(SoundSimState {
        sources,
        // TODO: separately adopt Original music/geometry into the live host
        // director. Keeping an unread imported copy here never applied it;
        // raw parser data and the existing host handoff retain their roles.
        // The original game tears down all backend completion work during deactivation or clearing.
        finished_exclamations: Vec::new(),
        playing_exclamations: Vec::new(),
        pending_exclamations: Vec::new(),
        resolved_exclamations: Vec::new(),
        replay_injected_resolved_exclamations: false,
        playing_sources: Vec::new(),
        suspended_active_sources: Vec::new(),
    })
}

fn convert_source(
    slot: usize,
    saved: &LegacySoundSource,
) -> Result<SoundSource, LegacyPreambleServicesError> {
    if saved.active_first != saved.active_second {
        return Err(LegacyPreambleServicesError::InconsistentSoundActive {
            slot,
            first: saved.active_first,
            second: saved.active_second,
        });
    }
    let source_kind = SoundSourceKind::from_u8(saved.kind).ok_or_else(|| {
        invalid_sound(
            "source.kind",
            saved.kind,
            "KIND_SINGLE..=KIND_VOLATILE (0..=3)",
        )
    })?;
    let altitude = match saved.altitude {
        0 => SoundSourceAltitude::Ground,
        1 => SoundSourceAltitude::Middle,
        2 => SoundSourceAltitude::Top,
        3 => SoundSourceAltitude::NoAltitude,
        value => {
            return Err(invalid_sound(
                "source.altitude",
                value,
                "ALTITUDE_GROUND..=ALTITUDE_NONE (0..=3)",
            ));
        }
    };
    let mut shape = Vec::with_capacity(saved.shape.len());
    for point in &saved.shape {
        validate_finite("source.shape.x", point.x)?;
        validate_finite("source.shape.y", point.y)?;
        shape.push(MapPoint::new(point.x, point.y));
    }

    Ok(SoundSource {
        // Original-game save loading restores this omitted value to zero.
        ambiences: 0,
        source_kind,
        id: saved.id,
        is_global: saved.global,
        inner_distance: saved.inner_distance,
        outer_distance: saved.outer_distance,
        noise_covering_distance: saved.noise_covering_distance,
        inner_volume: saved.inner_volume,
        outer_volume: saved.outer_volume,
        shape,
        altitude,
        min_delay: saved.min_delay,
        max_delay: saved.max_delay,
        delay_stepping: saved.delay_stepping,
        timer: saved.timer,
        // The same member is serialized twice; the second read wins.
        active: saved.active_second,
        // Legacy saves contain only sources admitted for their fixed
        // mission ambience.
        ambience_enabled: true,
    })
}

fn validate_finite(field: &'static str, value: f32) -> Result<(), LegacyPreambleServicesError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(invalid_sound(field, value, "a finite f32"))
    }
}

fn invalid_sound(
    field: &'static str,
    value: impl std::fmt::Display,
    expected: &'static str,
) -> LegacyPreambleServicesError {
    LegacyPreambleServicesError::InvalidSoundField {
        field,
        value: value.to_string(),
        expected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        legacy_save::engine::{
            LegacyPoint2, LegacySoundGeometry, LegacySoundSourceManager, LegacySoundSourceSlot,
        },
        messenger::{Message, MessageType, SimpleMessage},
    };

    fn source() -> LegacySoundSource {
        LegacySoundSource {
            kind: SoundSourceKind::Delayed as u8,
            altitude: 2,
            id: 77,
            global: false,
            inner_distance: 10,
            outer_distance: 20,
            noise_covering_distance: 30,
            inner_volume: 40,
            outer_volume: 50,
            min_delay: 2,
            max_delay: 8,
            delay_stepping: 3,
            timer: 6,
            active_first: true,
            active_second: true,
            // Writer garbage: explicitly ignored by adoption.
            former_need_update: true,
            shape: vec![
                LegacyPoint2 { x: 1.0, y: 2.0 },
                LegacyPoint2 { x: 3.0, y: 4.0 },
            ],
        }
    }

    fn sound() -> LegacySerializedSound {
        LegacySerializedSound {
            sound_system_ready: true,
            three_d_sound: true,
            active: false,
            geometry: LegacySoundGeometry {
                listen_point: LegacyPoint2 { x: 5.0, y: 6.0 },
                zoom_factor: 1.25,
            },
            music_mode: crate::sound::MusicMode::Alert as u8,
            dummy_channel: -123,
            quiet_mode_weight: 11,
            alert_mode_weight: 22,
            fight_mode_weight: 33,
            loop_index: 4,
            stream_position: 0,
            source_manager: LegacySoundSourceManager {
                slots: vec![
                    Some(LegacySoundSourceSlot {
                        slot_index: 0,
                        source: source(),
                        registration_id: 77,
                    }),
                    None,
                ],
            },
        }
    }

    #[test]
    fn converts_exact_sound_source_slots_without_stale_director_state() {
        let converted = convert_serialized_sound(&sound()).expect("valid sound state");
        assert_eq!(converted.sources.num_sources(), 2);
        let source = converted.sources.get(0).expect("slot zero");
        assert_eq!(source.source_kind, SoundSourceKind::Delayed);
        assert_eq!(source.altitude, SoundSourceAltitude::Top);
        assert_eq!(source.timer, 6);
        assert!(source.active);
        assert_eq!(
            source.shape,
            vec![MapPoint::new(1.0, 2.0), MapPoint::new(3.0, 4.0)]
        );
        assert!(converted.sources.get(1).is_none());
        assert!(converted.playing_sources.is_empty());
    }

    #[test]
    fn backend_and_director_variations_do_not_enter_sound_snapshots_or_hashes() {
        let original = sound();
        let baseline = convert_serialized_sound(&original).unwrap();
        let mut changed = original.clone();
        changed.sound_system_ready = false;
        changed.three_d_sound = false;
        changed.active = true;
        changed.geometry.listen_point = LegacyPoint2 { x: -4.0, y: 27.0 };
        changed.geometry.zoom_factor = 2.0;
        changed.music_mode = 2;
        changed.dummy_channel = i16::MAX;
        changed.quiet_mode_weight = 101;
        changed.alert_mode_weight = 202;
        changed.fight_mode_weight = 303;
        changed.loop_index = 9;
        changed.stream_position = 1234;
        let converted = convert_serialized_sound(&changed).unwrap();
        assert_eq!(bitcode::encode(&baseline), bitcode::encode(&converted));
        assert_eq!(
            robin_util::state_hash::compute(&baseline),
            robin_util::state_hash::compute(&converted)
        );
        let json = serde_json::to_value(&converted).unwrap();
        assert!(json.get("legacy_v48").is_none());
        let from_json: SoundSimState = serde_json::from_value(json).unwrap();
        let from_snapshot: SoundSimState = bitcode::decode(&bitcode::encode(&converted)).unwrap();
        for restored in [from_json, from_snapshot] {
            assert_eq!(bitcode::encode(&restored), bitcode::encode(&baseline));
            assert_eq!(
                robin_util::state_hash::compute(&restored),
                robin_util::state_hash::compute(&baseline)
            );
        }
        // Live imported sources remain authoritative rather than disappearing
        // along with the unused host director copy.
        changed.source_manager.slots[0]
            .as_mut()
            .unwrap()
            .source
            .timer += 1;
        assert_ne!(
            robin_util::state_hash::compute(&baseline),
            robin_util::state_hash::compute(&convert_serialized_sound(&changed).unwrap())
        );
    }

    #[test]
    fn native_sound_snapshots_preserve_live_completion_deadlines() {
        let mut state = convert_serialized_sound(&sound()).unwrap();
        state.playing_sources.push(crate::sound::PlayingSource {
            source_index: 0,
            finish_frame: 27,
        });
        state
            .playing_exclamations
            .push(crate::sound::PlayingExclamation {
                actor_id: 12,
                exclamation_id: 3,
                finish_frame: 31,
            });
        let json: SoundSimState =
            serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        let snapshot: SoundSimState = bitcode::decode(&bitcode::encode(&state)).unwrap();
        for restored in [json, snapshot] {
            assert_eq!(bitcode::encode(&restored), bitcode::encode(&state));
            assert_eq!(
                robin_util::state_hash::compute(&restored),
                robin_util::state_hash::compute(&state)
            );
            assert_eq!(restored.playing_sources[0].finish_frame, 27);
            assert_eq!(restored.playing_exclamations[0].finish_frame, 31);
        }
    }

    #[test]
    fn discarded_director_fields_and_live_source_slots_are_still_validated() {
        for (field, alter) in [
            (
                "music_mode",
                (|sound: &mut LegacySerializedSound| sound.music_mode = 3)
                    as fn(&mut LegacySerializedSound),
            ),
            (
                "geometry.listen_point.x",
                |sound: &mut LegacySerializedSound| sound.geometry.listen_point.x = f32::NAN,
            ),
            (
                "geometry.listen_point.y",
                |sound: &mut LegacySerializedSound| sound.geometry.listen_point.y = f32::INFINITY,
            ),
            (
                "geometry.zoom_factor",
                |sound: &mut LegacySerializedSound| sound.geometry.zoom_factor = f32::NAN,
            ),
            (
                "geometry.zoom_factor",
                |sound: &mut LegacySerializedSound| sound.geometry.zoom_factor = 0.0,
            ),
            (
                "geometry.zoom_factor",
                |sound: &mut LegacySerializedSound| sound.geometry.zoom_factor = -1.0,
            ),
        ] {
            let mut saved = sound();
            alter(&mut saved);
            assert!(matches!(
                convert_serialized_sound(&saved),
                Err(LegacyPreambleServicesError::InvalidSoundField { field: actual, .. }) if actual == field
            ));
        }
        let mut saved = sound();
        saved.source_manager.slots[0].as_mut().unwrap().slot_index = 1;
        assert!(matches!(
            convert_serialized_sound(&saved),
            Err(LegacyPreambleServicesError::WrongSoundSlotIndex { .. })
        ));
        let mut saved = sound();
        saved.source_manager.slots[0]
            .as_mut()
            .unwrap()
            .registration_id += 1;
        assert!(matches!(
            convert_serialized_sound(&saved),
            Err(LegacyPreambleServicesError::WrongSoundRegistration { .. })
        ));
    }

    #[test]
    fn rejects_inconsistent_duplicate_active_member() {
        let mut source = source();
        source.active_second = false;
        assert!(matches!(
            convert_source(3, &source),
            Err(LegacyPreambleServicesError::InconsistentSoundActive {
                slot: 3,
                first: true,
                second: false,
            })
        ));
    }

    #[test]
    fn rejects_unknown_messenger_action() {
        assert_eq!(
            convert_messenger_action(u16::MAX),
            Err(LegacyPreambleServicesError::InvalidMessengerAction { value: u16::MAX })
        );
    }

    #[test]
    fn apply_is_atomic_and_returns_host_only_output() {
        let mut engine = EngineInner::new();
        engine
            .orders
            .messenger
            .send(Message::new(MessageType::Simple(SimpleMessage::Pause)));
        let messenger = super::super::engine::LegacyMessenger {
            lock_view: true,
            setting_watch: true,
            watch_timer: 19,
            action: Action::Bow as u16,
            draw_hidden: true,
        };
        let game = LegacyGameState {
            men_to_blazon_conversion: true,
            campaign_map: true,
            campaign_map_displayed: false,
            post_initialized: true,
            start_mission_disabled_temp: true,
            quit_mission_disabled_temp: false,
            start_mission_enabled: true,
            quit_mission_enabled: true,
        };
        let host = LegacyPreambleHostState {
            sound_system_ready: Some(true),
            three_d_sound: Some(false),
            sound_active: Some(true),
            dummy_channel: Some(5),
            stream_position: Some(0),
            draw_hidden: true,
            campaign_map_displayed: false,
            start_mission_widget_enabled: false,
            quit_mission_widget_enabled: true,
        };
        for (ready, three_d, active, channel, position) in
            [(true, false, true, 5, 0), (false, true, false, -123, 1234)]
        {
            let mut saved = sound();
            saved.sound_system_ready = ready;
            saved.three_d_sound = three_d;
            saved.active = active;
            saved.dummy_channel = channel;
            saved.stream_position = position;
            let actual = LegacyPreambleHostState::from_preamble_parts(
                &LegacySound {
                    serialized: true,
                    state: Some(saved),
                },
                &messenger,
                &game,
            );
            assert_eq!(
                actual,
                LegacyPreambleHostState {
                    sound_system_ready: Some(ready),
                    three_d_sound: Some(three_d),
                    sound_active: Some(active),
                    dummy_channel: Some(channel),
                    stream_position: Some(position),
                    ..host.clone()
                }
            );
        }
        assert_eq!(
            LegacyPreambleHostState::from_preamble_parts(
                &LegacySound {
                    serialized: false,
                    state: None
                },
                &messenger,
                &game,
            ),
            LegacyPreambleHostState {
                sound_system_ready: None,
                three_d_sound: None,
                sound_active: None,
                dummy_channel: None,
                stream_position: None,
                ..host.clone()
            }
        );
        let plan = LegacyPreambleServicesPlan {
            sound: Some(convert_serialized_sound(&sound()).unwrap()),
            view_locked: messenger.lock_view,
            selected_action: convert_messenger_action(messenger.action).unwrap(),
            game,
            host: host.clone(),
        };

        let returned = plan.apply(&mut engine);
        assert_eq!(returned, host);
        assert_eq!(engine.orders.messenger.count(), 0);
        assert_eq!(engine.players.seats[0].selected_action, Action::Bow);
        assert!(engine.players.view_locked);
        // Live changes after import must be the only action/view state captured
        // by native saves and rollback snapshots, not a second imported copy.
        engine.players.seats[0].selected_action = Action::Stone;
        engine.players.view_locked = false;
        let players_json = serde_json::to_vec(
            &crate::engine::state::PersistedPlayerRuntime::capture(&engine.players),
        )
        .unwrap();
        let persisted: crate::engine::state::PersistedPlayerRuntime =
            serde_json::from_slice(&players_json).unwrap();
        let snapshot: crate::engine::state::PlayerRuntime =
            bitcode::decode(&bitcode::encode(&engine.players)).unwrap();
        for restored in [persisted.into_runtime(), snapshot] {
            assert_eq!(restored.seats[0].selected_action, Action::Stone);
            assert!(!restored.view_locked);
            assert_eq!(
                robin_util::state_hash::compute(&restored),
                robin_util::state_hash::compute(&engine.players)
            );
        }
        assert_eq!(
            serde_json::to_value(&engine.orders.messenger).unwrap(),
            serde_json::json!({ "queue": [] })
        );
        assert!(
            engine
                .script_domains
                .mission_ui
                .men_to_blazon_conversion_mode
        );
        assert!(engine.script_domains.mission_ui.campaign_map);
        assert!(engine.script_domains.mission_ui.start_mission_disabled_temp);
        assert_eq!(engine.feedback.sound_sim.sources.num_sources(), 2);
        assert!(engine.script_domains.mission_ui.game_post_initialized);
        assert!(engine.scripts.mission.is_none());

        // Import is authoritative even without a VM and in both directions.
        LegacyPreambleServicesPlan {
            sound: None,
            view_locked: messenger.lock_view,
            selected_action: convert_messenger_action(messenger.action).unwrap(),
            game: LegacyGameState {
                post_initialized: false,
                ..game
            },
            host,
        }
        .apply(&mut engine);
        assert!(!engine.script_domains.mission_ui.game_post_initialized);
    }
}
