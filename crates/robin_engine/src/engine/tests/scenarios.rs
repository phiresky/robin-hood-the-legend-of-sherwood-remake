//! Shared scenario inputs; actor construction lives in the crate-wide fixtures.
pub(in crate::engine) use crate::engine::test_support::actors::{
    make_test_ai_soldier, make_test_civilian, make_test_pc, make_test_soldier,
};

pub(super) fn assets_with_test_pc_profile() -> super::LevelAssets {
    let mut profiles = crate::profiles::ProfileManager::new();
    profiles
        .characters
        .push(crate::profiles::CharacterProfile::default());
    super::LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..super::LevelAssets::new()
    }
}
