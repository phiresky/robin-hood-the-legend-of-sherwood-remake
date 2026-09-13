use super::*;
use crate::sound::PlayingSource;
use crate::sound_source::SoundSourceKind;
use std::collections::BTreeMap;
use std::sync::Arc;

#[test]
fn source_finish_uses_exact_metadata_duration() {
    let durations = Arc::new(BTreeMap::from([(0x1234, 9)]));
    let mut playing = Vec::<PlayingSource>::new();

    schedule_source_finish(
        &SoundSourceKind::Single,
        0x1234,
        4,
        100,
        &durations,
        &mut playing,
    );

    assert_eq!(playing.len(), 1);
    assert_eq!(playing[0].source_index, 4);
    assert_eq!(playing[0].finish_frame, 109);
}

#[test]
fn missing_source_duration_schedules_zero_length_completion() {
    let durations = Arc::new(BTreeMap::new());
    let mut playing = Vec::<PlayingSource>::new();

    schedule_source_finish(
        &SoundSourceKind::Volatile,
        0x5678,
        7,
        100,
        &durations,
        &mut playing,
    );

    assert_eq!(playing.len(), 1);
    assert_eq!(playing[0].source_index, 7);
    assert_eq!(
        playing[0].finish_frame, 100,
        "missing samples complete at the next drain, never after a fabricated 75 frames"
    );
}
