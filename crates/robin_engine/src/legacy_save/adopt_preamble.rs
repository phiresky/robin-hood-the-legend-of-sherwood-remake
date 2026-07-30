//! Validated adoption of the unambiguous Original Linux-v48 engine scalars.
//!
//! This intentionally stops short of a complete save adoption.  Camera,
//! sound, messenger, host UI, element identity, and global counter state all
//! need their own conversion contracts.  Keeping this slice narrow prevents a
//! partially-understood Original field from being silently defaulted while
//! still allowing the complete adopter to prepare this state before its
//! atomic engine swap.

use std::collections::BTreeSet;

use thiserror::Error;

use crate::{engine::EngineInner, short_briefings::ShortBriefings};

use super::{
    LegacySaveAbiProfile,
    engine::{LegacyEnginePreamble, LegacyShortBriefing, LegacyShortBriefings},
};

const MAX_SCROLL_SPEED_INDEX: u16 = 31;

/// Converted engine-owned state with a direct Original Linux-v48 mapping.
///
/// Construction validates the complete slice before [`EngineInner`] is
/// mutated. Applying this value is consequently infallible and cannot leave a
/// subset of these fields updated.
#[derive(Clone, Debug)]
pub(crate) struct LegacyLinuxPreambleState {
    cheat_used_flags: u32,
    freeze_all: bool,
    frame_counter: u32,
    speed: f32,
    speed_index: u16,
    lock_engine: bool,
    mission_won: bool,
    mission_won_first_time: bool,
    short_briefings: ShortBriefings,
}

#[derive(Clone, Debug, Error, PartialEq)]
pub(crate) enum LegacyPreambleAdoptionError {
    #[error("legacy preamble adoption currently supports Linux i386 v48 only, not {actual:?}")]
    UnsupportedAbi { actual: LegacySaveAbiProfile },
    #[error("legacy engine speed must be finite and non-negative, got {value}")]
    InvalidSpeed { value: f32 },
    #[error("legacy scroll speed index {value} exceeds the Original 32-entry scrolling table")]
    ScrollSpeedIndexOutOfRange { value: u16 },
    #[error("legacy short briefing id {id} occurs more than once")]
    DuplicateShortBriefing { id: u32 },
}

impl LegacyLinuxPreambleState {
    /// Validate and convert the fields whose destination is entirely
    /// engine-owned and semantically identical in the Original Linux port.
    pub(crate) fn try_from_v48(
        abi: LegacySaveAbiProfile,
        preamble: &LegacyEnginePreamble,
    ) -> Result<Self, LegacyPreambleAdoptionError> {
        Self::try_from_fields(
            abi,
            LegacyPreambleFields {
                cheat_used_flags: preamble.cheat_used_flags,
                freeze_all: preamble.freeze_all,
                frame_counter: preamble.universal_frame_counter,
                speed: preamble.speed,
                speed_index: preamble.speed_index,
                lock_engine: preamble.lock_engine,
                mission_won: preamble.mission_won,
                mission_won_first_time: preamble.mission_won_first_time,
                short_briefings: &preamble.short_briefings,
            },
        )
    }

    fn try_from_fields(
        abi: LegacySaveAbiProfile,
        fields: LegacyPreambleFields<'_>,
    ) -> Result<Self, LegacyPreambleAdoptionError> {
        if abi != LegacySaveAbiProfile::PortLinuxI386V48 {
            return Err(LegacyPreambleAdoptionError::UnsupportedAbi { actual: abi });
        }
        if !fields.speed.is_finite() || fields.speed < 0.0 {
            return Err(LegacyPreambleAdoptionError::InvalidSpeed {
                value: fields.speed,
            });
        }
        if fields.speed_index > MAX_SCROLL_SPEED_INDEX {
            return Err(LegacyPreambleAdoptionError::ScrollSpeedIndexOutOfRange {
                value: fields.speed_index,
            });
        }

        let short_briefings = convert_short_briefings(fields.short_briefings)?;
        Ok(Self {
            cheat_used_flags: fields.cheat_used_flags,
            freeze_all: fields.freeze_all,
            frame_counter: fields.frame_counter,
            speed: fields.speed,
            speed_index: fields.speed_index,
            lock_engine: fields.lock_engine,
            mission_won: fields.mission_won,
            mission_won_first_time: fields.mission_won_first_time,
            short_briefings,
        })
    }
}

struct LegacyPreambleFields<'a> {
    cheat_used_flags: u32,
    freeze_all: bool,
    frame_counter: u32,
    speed: f32,
    speed_index: u16,
    lock_engine: bool,
    mission_won: bool,
    mission_won_first_time: bool,
    short_briefings: &'a LegacyShortBriefings,
}

fn convert_short_briefings(
    source: &LegacyShortBriefings,
) -> Result<ShortBriefings, LegacyPreambleAdoptionError> {
    let mut seen = BTreeSet::new();
    let mut converted = ShortBriefings::default();
    append_short_briefings(&mut converted, &mut seen, &source.primaries, true)?;
    append_short_briefings(&mut converted, &mut seen, &source.secondaries, false)?;
    Ok(converted)
}

fn append_short_briefings(
    destination: &mut ShortBriefings,
    seen: &mut BTreeSet<u32>,
    source: &[LegacyShortBriefing],
    primary: bool,
) -> Result<(), LegacyPreambleAdoptionError> {
    for entry in source {
        if !seen.insert(entry.id) {
            return Err(LegacyPreambleAdoptionError::DuplicateShortBriefing { id: entry.id });
        }
        let inserted = destination.add(entry.id, primary);
        debug_assert!(inserted, "duplicate briefing was validated above");
        if entry.done {
            destination.mark_done(entry.id);
        }
    }
    Ok(())
}

impl EngineInner {
    /// Apply a fully validated preamble slice in one infallible mutation.
    ///
    /// Original `RHEngine::Serialize` explicitly clears all three quit
    /// latches in read mode. They are therefore reset here rather than copied
    /// from the initialized mission.
    pub(crate) fn apply_legacy_linux_preamble_state(&mut self, state: LegacyLinuxPreambleState) {
        let LegacyLinuxPreambleState {
            cheat_used_flags,
            freeze_all,
            frame_counter,
            speed,
            speed_index,
            lock_engine,
            mission_won,
            mission_won_first_time,
            short_briefings,
        } = state;

        self.mission_domain.cheat_used_flags = cheat_used_flags;
        self.mission_domain.short_briefings = short_briefings;
        self.mission_domain.state.mission_won = mission_won;
        self.mission_domain.state.mission_won_first_time = mission_won_first_time;
        self.mission_domain.state.quit_won = false;
        self.mission_domain.state.quit_lost = false;
        self.mission_domain.state.quit_interrupted = false;

        self.control.frame_counter = frame_counter;
        self.control.speed = speed;
        self.control.speed_int = speed_index;
        self.control.set_actors_frozen(freeze_all);
        self.control.set_engine_locked(lock_engine);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn briefings() -> LegacyShortBriefings {
        LegacyShortBriefings {
            primaries: vec![
                LegacyShortBriefing { id: 10, done: true },
                LegacyShortBriefing {
                    id: 20,
                    done: false,
                },
            ],
            secondaries: vec![LegacyShortBriefing { id: 30, done: true }],
        }
    }

    fn fields(briefings: &LegacyShortBriefings) -> LegacyPreambleFields<'_> {
        LegacyPreambleFields {
            cheat_used_flags: 0x1234_5678,
            freeze_all: true,
            frame_counter: 98_765,
            speed: 4.5,
            speed_index: 17,
            lock_engine: true,
            mission_won: true,
            mission_won_first_time: true,
            short_briefings: briefings,
        }
    }

    #[test]
    fn validates_before_atomically_applying_direct_engine_fields() {
        let source_briefings = briefings();
        let converted = LegacyLinuxPreambleState::try_from_fields(
            LegacySaveAbiProfile::PortLinuxI386V48,
            fields(&source_briefings),
        )
        .expect("valid Linux preamble slice");

        let mut engine = EngineInner::new();
        engine.mission_domain.state.quit_won = true;
        engine.mission_domain.state.quit_lost = true;
        engine.mission_domain.state.quit_interrupted = true;
        engine.apply_legacy_linux_preamble_state(converted);

        assert_eq!(engine.mission_domain.cheat_used_flags, 0x1234_5678);
        assert_eq!(engine.control.frame_counter, 98_765);
        assert_eq!(engine.control.speed, 4.5);
        assert_eq!(engine.control.speed_int, 17);
        assert!(engine.control.actors_frozen());
        assert!(engine.control.engine_locked());
        assert!(engine.mission_domain.state.mission_won);
        assert!(engine.mission_domain.state.mission_won_first_time);
        assert!(!engine.mission_domain.state.quit_won);
        assert!(!engine.mission_domain.state.quit_lost);
        assert!(!engine.mission_domain.state.quit_interrupted);
        assert_eq!(engine.mission_domain.short_briefings.count(true), 2);
        assert_eq!(
            engine.mission_domain.short_briefings.get_id(true, 0),
            Some(10)
        );
        assert_eq!(
            engine.mission_domain.short_briefings.is_entry_done(true, 0),
            Some(true)
        );
        assert_eq!(
            engine.mission_domain.short_briefings.get_id(false, 0),
            Some(30)
        );
    }

    #[test]
    fn rejects_windows_and_invalid_scroll_state_before_mutation() {
        let source_briefings = briefings();
        let windows = LegacyLinuxPreambleState::try_from_fields(
            LegacySaveAbiProfile::RetailWindowsX86V48,
            fields(&source_briefings),
        );
        assert_eq!(
            windows.unwrap_err(),
            LegacyPreambleAdoptionError::UnsupportedAbi {
                actual: LegacySaveAbiProfile::RetailWindowsX86V48,
            }
        );

        let mut invalid = fields(&source_briefings);
        invalid.speed_index = 32;
        assert_eq!(
            LegacyLinuxPreambleState::try_from_fields(
                LegacySaveAbiProfile::PortLinuxI386V48,
                invalid,
            )
            .unwrap_err(),
            LegacyPreambleAdoptionError::ScrollSpeedIndexOutOfRange { value: 32 }
        );

        let mut invalid = fields(&source_briefings);
        invalid.speed = -1.0;
        assert_eq!(
            LegacyLinuxPreambleState::try_from_fields(
                LegacySaveAbiProfile::PortLinuxI386V48,
                invalid,
            )
            .unwrap_err(),
            LegacyPreambleAdoptionError::InvalidSpeed { value: -1.0 }
        );
    }

    #[test]
    fn rejects_duplicate_briefing_identity_instead_of_dropping_data() {
        let duplicate = LegacyShortBriefings {
            primaries: vec![LegacyShortBriefing { id: 7, done: false }],
            secondaries: vec![LegacyShortBriefing { id: 7, done: true }],
        };
        assert_eq!(
            LegacyLinuxPreambleState::try_from_fields(
                LegacySaveAbiProfile::PortLinuxI386V48,
                fields(&duplicate),
            )
            .unwrap_err(),
            LegacyPreambleAdoptionError::DuplicateShortBriefing { id: 7 }
        );
    }
}
