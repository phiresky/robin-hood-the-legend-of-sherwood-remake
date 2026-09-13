//! Simulation compatibility facade for level data shared with asset loading.

pub use robin_level_data::content_patch::*;

#[cfg(test)]
mod tests {
    use crate::profiles::ProfileManager;

    #[test]
    fn mission_loader_patches_expanded_hackable_data_and_rejects_invalid_content() {
        let campaign = crate::campaign::Campaign {
            current_mission_idx: Some(0),
            missions: vec![crate::mission::Mission {
                profile_idx: Some(0),
                ..Default::default()
            }],
            ..Default::default()
        };
        let profiles = ProfileManager {
            missions: vec![crate::profiles::MissionProfile {
                mission_filename: "PatchLoaderTest".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        for (patch, succeeds) in [
            (
                r#"[{"op":"replace","path":"/mission/beam_mes/0/profile_override","value":7}]"#,
                true,
            ),
            (r#"[{"op":"add","path":"/mission/typo","value":7}]"#, false),
        ] {
            let vfs = std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new());
            vfs.install_preloaded_asset(
                "Data/Levels/PatchLoaderTest.level.json",
                br#"{
                "map_filename":"PatchLoaderMap", "spawn":[0,0],
                "walkable_polygon":[[0,0],[100,0],[0,100]]
            }"#
                .to_vec(),
            )
            .unwrap();
            vfs.install_preloaded_asset(
                "Data/Levels/PatchLoaderTest.level.patch.json",
                patch.as_bytes().to_vec(),
            )
            .unwrap();
            let files = crate::sbfile::SbFileSystem::new(vfs);
            let result = crate::engine::level_loading::load_mission_for_campaign_with_files(
                &campaign,
                &profiles,
                "Data/Levels",
                &mut |_| {},
                &files,
            );
            if succeeds {
                assert_eq!(
                    result.unwrap().mission.beam_mes[0].profile_override,
                    Some(7)
                );
            } else {
                let error = result.unwrap_err().to_string();
                assert!(
                    error.contains("PatchLoaderTest.level.patch.json"),
                    "{error}"
                );
                assert!(error.contains("typo"), "{error}");
            }
        }
    }
}
