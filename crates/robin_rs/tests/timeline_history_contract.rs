//! Black-box contracts for checkpoint/journal lifecycle boundaries.

use robin_engine::campaign::Campaign;
use robin_engine::engine::{Engine, LevelAssets};
use robin_engine::player_command::{PlayerCommand, PlayerInput};
use robin_rs::sim_timeline::{
    CheckpointPolicy, CommandJournal, RestoreError, RestorePolicy, RetentionPolicy, TimelineHistory,
};

fn fixture_engine(assets: &mut LevelAssets) -> Engine {
    Engine::new_for_test(640.0, 480.0, Campaign::default(), assets)
        .expect("construct timeline-history fixture")
}

fn commands(label: &str) -> Vec<PlayerInput> {
    vec![PlayerInput::host(PlayerCommand::RegisterPeasantName {
        name: label.to_owned(),
    })]
}

#[test]
fn retention_prunes_commands_to_the_oldest_replayable_checkpoint() {
    let mut assets = LevelAssets::new();
    let mut engine = fixture_engine(&mut assets);
    let mut history = TimelineHistory::new(
        CheckpointPolicy::EveryFrame,
        RetentionPolicy::Latest { capacity: 2 },
    );

    for frame in 10..=12 {
        engine.test_set_frame_counter(frame);
        history.begin_frame(frame, &engine);
        assert!(history.commit_frame(commands(&format!("frame-{frame}"))));
    }

    assert_eq!(history.oldest_checkpoint_frame(), Some(11));
    assert_eq!(history.oldest_command_frame(), Some(11));
    assert!(history.commands_for(10).is_none());
    assert_eq!(history.commands_for(11).map(<[_]>::len), Some(1));
    assert_eq!(history.commands_for(12).map(<[_]>::len), Some(1));
    assert!(matches!(
        history.restore(10, RestorePolicy::LatestAtOrBefore),
        Err(RestoreError::CheckpointUnavailable {
            target_frame: 10,
            policy: RestorePolicy::LatestAtOrBefore,
        })
    ));
    let snapshot = history
        .restore(11, RestorePolicy::Exact)
        .expect("oldest retained checkpoint");
    assert_eq!(snapshot.frame, 11);
    assert_eq!(snapshot.engine.frame_counter(), 11);
}

#[test]
fn truncating_at_the_command_horizon_preserves_the_branch_checkpoint() {
    let mut assets = LevelAssets::new();
    let mut engine = fixture_engine(&mut assets);
    let mut history = TimelineHistory::new(
        CheckpointPolicy::EveryFrame,
        RetentionPolicy::Latest { capacity: 2 },
    );

    for frame in 10..=12 {
        engine.test_set_frame_counter(frame);
        history.begin_frame(frame, &engine);
        assert!(history.commit_frame(commands(&format!("old-{frame}"))));
    }

    history.truncate_future(11);
    assert_eq!(history.oldest_checkpoint_frame(), Some(11));
    assert_eq!(history.oldest_command_frame(), None);
    assert_eq!(history.next_record_frame(), 11);
    assert!(history.commands_for(11).is_none());
    assert_eq!(
        history
            .restore(11, RestorePolicy::Exact)
            .expect("pre-tick branch checkpoint remains valid")
            .frame,
        11
    );
    assert!(history.restore(12, RestorePolicy::Exact).is_err());

    engine.test_set_frame_counter(11);
    history.begin_frame(11, &engine);
    assert!(history.commit_frame(commands("new-11")));
    assert_eq!(history.oldest_command_frame(), Some(11));
    assert_eq!(history.next_record_frame(), 12);
    assert_eq!(history.commands_for(11).map(<[_]>::len), Some(1));
}

#[test]
fn truncating_before_the_retained_horizon_is_a_no_op() {
    let mut assets = LevelAssets::new();
    let mut engine = fixture_engine(&mut assets);
    let mut history = TimelineHistory::new(
        CheckpointPolicy::EveryFrame,
        RetentionPolicy::Latest { capacity: 2 },
    );

    for frame in 20..=22 {
        engine.test_set_frame_counter(frame);
        history.begin_frame(frame, &engine);
        assert!(history.commit_frame(Vec::new()));
    }

    history.truncate_future(20);
    assert_eq!(history.oldest_checkpoint_frame(), Some(21));
    assert_eq!(history.oldest_command_frame(), Some(21));
    assert_eq!(history.next_record_frame(), 23);
    assert!(history.commands_for(21).is_some());
    assert!(history.commands_for(22).is_some());
    assert!(history.restore(22, RestorePolicy::Exact).is_ok());
}

#[test]
fn periodic_history_starts_journaling_only_when_a_checkpoint_can_anchor_replay() {
    let mut assets = LevelAssets::new();
    let mut engine = fixture_engine(&mut assets);
    let mut history = TimelineHistory::new(
        CheckpointPolicy::EveryNthFrame { interval: 5 },
        RetentionPolicy::Latest { capacity: 2 },
    );

    for frame in 3..5 {
        engine.test_set_frame_counter(frame);
        history.begin_frame(frame, &engine);
        assert!(!history.commit_frame(commands(&format!("unanchored-{frame}"))));
    }
    assert_eq!(history.oldest_checkpoint_frame(), None);
    assert_eq!(history.oldest_command_frame(), None);
    assert_eq!(history.next_record_frame(), 0);

    engine.test_set_frame_counter(5);
    history.begin_frame(5, &engine);
    assert!(history.commit_frame(commands("anchored-5")));
    assert_eq!(history.oldest_checkpoint_frame(), Some(5));
    assert_eq!(history.oldest_command_frame(), Some(5));
    assert_eq!(history.next_record_frame(), 6);
    assert!(history.commands_for(3).is_none());
    assert_eq!(history.commands_for(5).map(<[_]>::len), Some(1));
}

#[test]
fn an_empty_truncated_journal_keeps_its_branch_frame_until_clear() {
    let mut journal = CommandJournal::default();
    journal.record(40, Vec::new());
    journal.record(41, Vec::new());

    journal.truncate_from(40);
    assert!(journal.is_empty());
    assert_eq!(journal.oldest_frame(), None);
    assert_eq!(journal.next_frame(), 40);

    journal.record(40, Vec::new());
    journal.discard_before(41);
    assert!(journal.is_empty());
    assert_eq!(journal.next_frame(), 41);
    journal.record(41, Vec::new());

    journal.clear();
    assert_eq!(journal.next_frame(), 0);
    journal.record(100, Vec::new());
    assert_eq!(journal.oldest_frame(), Some(100));
    assert_eq!(journal.next_frame(), 101);
}
