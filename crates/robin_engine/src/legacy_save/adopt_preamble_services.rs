//! Linux-v48 adoption for sound, RHMessenger, and RHGame preamble state.
//!
//! Host playback channels, stream seeking, and widget mutation are returned as
//! presentation state. They are never invoked during simulation adoption.

use thiserror::Error;

use crate::{
    coordinates::MapPoint,
    engine::EngineInner,
    messenger::LegacyV48MessengerState,
    profiles::Action,
    sound::{LegacyV48SoundState, MusicMode, SoundSimState},
    sound_geometry::SoundSourceAltitude,
    sound_source::{SoundSource, SoundSourceKind, SoundSourceManager},
};

use super::{
    LegacySaveAbiProfile,
    engine::{
        LegacyEnginePreamble, LegacyGameState, LegacyMessenger, LegacySerializedSound, LegacySound,
        LegacySoundSource,
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LegacyPreambleHostState {
    pub(crate) sound_system_ready: Option<bool>,
    pub(crate) three_d_sound: Option<bool>,
    pub(crate) sound_active: Option<bool>,
    pub(crate) dummy_channel: Option<i16>,
    pub(crate) stream_position: Option<u32>,
    pub(crate) draw_hidden: bool,
    pub(crate) campaign_map_displayed: bool,
    pub(crate) start_mission_widget_enabled: bool,
    pub(crate) quit_mission_widget_enabled: bool,
}

#[derive(Debug)]
pub(crate) struct LegacyPreambleServicesPlan {
    sound: Option<SoundSimState>,
    messenger: LegacyV48MessengerState,
    game: LegacyGameState,
    host: LegacyPreambleHostState,
}

impl LegacyPreambleServicesPlan {
    pub(crate) fn apply(self, engine: &mut EngineInner) -> LegacyPreambleHostState {
        if let Some(sound) = self.sound {
            engine.feedback.sound_sim = sound;
        }
        engine.players.view_locked = self.messenger.lock_view;
        engine.orders.messenger.restore_v48_state(self.messenger);

        let ui = &mut engine.script_domains.mission_ui;
        ui.men_to_blazon_conversion_mode = self.game.men_to_blazon_conversion;
        ui.campaign_map = self.game.campaign_map;
        ui.campaign_map_displayed = self.game.campaign_map_displayed;
        ui.game_post_initialized = self.game.post_initialized;
        ui.start_mission_disabled_temp = self.game.start_mission_disabled_temp;
        ui.quit_mission_disabled_temp = self.game.quit_mission_disabled_temp;
        ui.start_mission_enabled = self.game.start_mission_enabled;
        ui.quit_mission_enabled = self.game.quit_mission_enabled;

        // RHGame's flag is the authoritative source at this point in Original
        // load. Rust's mission-script lifecycle consumes the equivalent flag.
        if let Some(script) = engine.scripts.mission.as_mut() {
            script.post_initialized = self.game.post_initialized;
        }
        self.host
    }
}

#[derive(Clone, Debug, Error, PartialEq)]
pub(crate) enum LegacyPreambleServicesError {
    #[error("saved messenger action {value} is not a known RHaction")]
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
    let messenger = convert_messenger(preamble.messenger)?;
    let sound = convert_sound(&preamble.sound)?;
    let host = LegacyPreambleHostState {
        sound_system_ready: preamble
            .sound
            .state
            .as_ref()
            .map(|state| state.sound_system_ready),
        three_d_sound: preamble
            .sound
            .state
            .as_ref()
            .map(|state| state.three_d_sound),
        sound_active: preamble.sound.state.as_ref().map(|state| state.active),
        dummy_channel: preamble
            .sound
            .state
            .as_ref()
            .map(|state| state.dummy_channel),
        stream_position: preamble
            .sound
            .state
            .as_ref()
            .map(|state| state.stream_position),
        draw_hidden: preamble.messenger.draw_hidden,
        campaign_map_displayed: preamble.game.campaign_map_displayed,
        start_mission_widget_enabled: preamble.game.start_mission_enabled
            && !preamble.game.start_mission_disabled_temp,
        quit_mission_widget_enabled: preamble.game.quit_mission_enabled
            && !preamble.game.quit_mission_disabled_temp,
    };

    Ok(LegacyPreambleServicesPlan {
        sound,
        messenger,
        game: preamble.game,
        host,
    })
}

fn convert_messenger(
    saved: LegacyMessenger,
) -> Result<LegacyV48MessengerState, LegacyPreambleServicesError> {
    let action = Action::try_from(u32::from(saved.action)).map_err(|_| {
        LegacyPreambleServicesError::InvalidMessengerAction {
            value: saved.action,
        }
    })?;
    Ok(LegacyV48MessengerState {
        lock_view: saved.lock_view,
        setting_watch: saved.setting_watch,
        watch_timer: saved.watch_timer,
        action,
        draw_hidden: saved.draw_hidden,
    })
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
    let music_mode = match saved.music_mode {
        0 => MusicMode::Quiet,
        1 => MusicMode::Alert,
        2 => MusicMode::Fight,
        value => {
            return Err(invalid_sound(
                "music_mode",
                value,
                "MODE_QUIET..=MODE_FIGHT (0..=2)",
            ));
        }
    };
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
        legacy_v48: Some(LegacyV48SoundState {
            sound_system_ready: saved.sound_system_ready,
            three_d_sound: saved.three_d_sound,
            active: saved.active,
            listen_point: MapPoint::new(
                saved.geometry.listen_point.x,
                saved.geometry.listen_point.y,
            ),
            zoom_factor: saved.geometry.zoom_factor,
            music_mode,
            dummy_channel: saved.dummy_channel,
            quiet_mode_weight: saved.quiet_mode_weight,
            alert_mode_weight: saved.alert_mode_weight,
            fight_mode_weight: saved.fight_mode_weight,
            loop_index: saved.loop_index,
            stream_position: saved.stream_position,
        }),
        // Original Deactivate/Clear tears down all backend completion work.
        finished_exclamations: Vec::new(),
        playing_exclamations: Vec::new(),
        pending_exclamations: Vec::new(),
        resolved_exclamations: Vec::new(),
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
        // Original's save constructor restores this omitted member to zero.
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
            music_mode: MusicMode::Alert as u8,
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
    fn converts_exact_sound_source_slots_and_director_state() {
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
        let director = converted.legacy_v48.expect("director");
        assert_eq!(director.music_mode, MusicMode::Alert);
        assert_eq!(director.alert_mode_weight, 22);
        assert_eq!(director.dummy_channel, -123);
        assert!(converted.playing_sources.is_empty());
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
    fn apply_is_atomic_and_returns_host_only_output() {
        let mut engine = EngineInner::new();
        engine
            .orders
            .messenger
            .send(Message::new(MessageType::Simple(SimpleMessage::Pause)));
        let messenger = convert_messenger(LegacyMessenger {
            lock_view: true,
            setting_watch: true,
            watch_timer: 19,
            action: Action::Bow as u16,
            draw_hidden: true,
        })
        .unwrap();
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
        let plan = LegacyPreambleServicesPlan {
            sound: Some(convert_serialized_sound(&sound()).unwrap()),
            messenger,
            game,
            host: host.clone(),
        };

        let returned = plan.apply(&mut engine);
        assert_eq!(returned, host);
        assert_eq!(engine.orders.messenger.count(), 0);
        let restored = engine.orders.messenger.v48_state().unwrap();
        assert_eq!(restored.action, Action::Bow);
        assert!(restored.lock_view);
        assert!(engine.players.view_locked);
        assert!(
            engine
                .script_domains
                .mission_ui
                .men_to_blazon_conversion_mode
        );
        assert!(engine.script_domains.mission_ui.campaign_map);
        assert!(engine.script_domains.mission_ui.start_mission_disabled_temp);
        assert_eq!(engine.feedback.sound_sim.sources.num_sources(), 2);
    }
}
