//! Authored mission text and localization overlays.
use super::error::ResourcePreparationError;
use robin_assets::{res_descr as assets_res_descr, resource_manager::ResourceManager};
use robin_engine::{campaign::Campaign, profiles as engine_profiles, sbfile as engine_sbfile};

/// Only a missing optional file returns `None`; probing, reading and parsing
/// failures retain their classification until startup chooses its policy.
pub(super) fn read_optional_json<T: serde::de::DeserializeOwned>(
    files: &engine_sbfile::SbFileSystem,
    path: &str,
) -> Result<Option<T>, ResourcePreparationError> {
    if !files
        .try_exists(path)
        .map_err(|status| ResourcePreparationError::unavailable(path, status))?
    {
        return Ok(None);
    }
    let bytes = files
        .read_shared(path)
        .map_err(|status| ResourcePreparationError::unavailable(path, status))?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| ResourcePreparationError::malformed(path, error))
}

pub(super) fn apply_mission_descriptor_patch(
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
    descriptors: &mut assets_res_descr::LevelDescriptors,
    files: &engine_sbfile::SbFileSystem,
) -> Result<(), ResourcePreparationError> {
    let mission_filename = campaign
        .current_mission_idx
        .and_then(|index| campaign.missions.get(index))
        .map(|mission| mission.profile(profiles).mission_filename.as_str())
        .expect("custom mission text lookup requires a current campaign mission");
    let path = format!("Data/Levels/{mission_filename}.descriptors.patch.json");
    if let Some(patched) = robin_engine::content_patch::apply_with_files(descriptors, files, &path)
        .map_err(|error| ResourcePreparationError::malformed(&path, error))?
    {
        validate_patched_descriptors(&patched, &path)?;
        *descriptors = patched;
    }
    Ok(())
}

fn validate_patched_descriptors(
    descriptors: &assets_res_descr::LevelDescriptors,
    path: &str,
) -> Result<(), ResourcePreparationError> {
    for (index, sentences) in descriptors.custom_dialogue_texts.iter().enumerate() {
        let Some(sentences) = sentences else { continue };
        let dialogue = descriptors.dialogues.get(index).ok_or_else(|| {
            ResourcePreparationError::malformed(
                path,
                format!("dialogue {index} has no base descriptor"),
            )
        })?;
        if sentences.len() != dialogue.portrait_ids.len() {
            return Err(ResourcePreparationError::malformed(
                path,
                format!(
                    "dialogue {index} requires {} sentences",
                    dialogue.portrait_ids.len()
                ),
            ));
        }
    }
    Ok(())
}

/// Load the 22-firstname / 22-surname peasant name pool from
/// `Level.res` — the civilian display-name branch.  Sub-IDs 100-121
/// hold firstnames, 122-143 surnames, under one of three menu text
/// tables (full / demo / demo2).
pub fn load_peasant_name_pool(text_res: &mut ResourceManager) -> (Vec<String>, Vec<String>) {
    use crate::ui_panel::menu_text_string;
    const FIRSTNAME_BASE: usize = 100;
    const SURNAME_BASE: usize = 122;
    const NAME_COUNT: usize = 22;
    let firstnames: Vec<String> = (0..NAME_COUNT)
        .filter_map(|i| menu_text_string(text_res, FIRSTNAME_BASE + i).map(|(s, _, _)| s))
        .collect();
    let surnames: Vec<String> = (0..NAME_COUNT)
        .filter_map(|i| menu_text_string(text_res, SURNAME_BASE + i).map(|(s, _, _)| s))
        .collect();
    (firstnames, surnames)
}

/// Load the fixed localized VIP names selected by
/// Original-game name generation. Keys are the canonical French profile
/// identities stored in CPF and mission data.
pub fn load_fixed_vip_name_map(
    text_res: &mut ResourceManager,
) -> std::collections::BTreeMap<String, String> {
    use crate::ui_panel::menu_text_string;
    const VIP_NAME_BASE: usize = 144;
    const PROFILE_NAMES: [&str; 7] = [
        "Robin des bois",
        "Robin des villes",
        "Will Ecarlate",
        "Petit Jean",
        "Frere Tuck",
        "Lady Marianne",
        "Stutely",
    ];

    PROFILE_NAMES
        .into_iter()
        .enumerate()
        .filter_map(|(offset, profile_name)| {
            menu_text_string(text_res, VIP_NAME_BASE + offset)
                .map(|(localized, _, _)| (profile_name.to_owned(), localized))
        })
        .collect()
}

pub(super) fn resolve_short_briefings(
    text_res: &mut ResourceManager,
    level_descriptors: Option<&assets_res_descr::LevelDescriptors>,
) -> Result<std::collections::HashMap<u32, String>, ResourcePreparationError> {
    let Some(descriptor) = level_descriptors else {
        return Ok(std::collections::HashMap::new());
    };
    let table_id = descriptor.short_briefing.text_table_id;
    let mut resolved = std::collections::HashMap::new();
    // Some demos and custom missions omit the legacy table entirely.
    // Absence is optional; a registered table that cannot be read is not.
    // In particular, never silently discard an authored text ID because
    // its entry failed to decode.
    if text_res.has_resource(table_id) {
        let table = text_res.get_strings(table_id).map_err(|error| {
            ResourcePreparationError::malformed(
                "Data/Text/Level.res",
                format!("short-briefing table {table_id}: {error:#}"),
            )
        })?;
        for (index, text) in table.iter().enumerate() {
            resolved.insert(index as u32, text.to_owned());
        }
    } else {
        tracing::debug!(
            table_id,
            "Optional short-briefing table absent from Data/Text/Level.res"
        );
    }
    for (index, text) in descriptor.custom_short_briefings.iter().enumerate() {
        if let Some(text) = text {
            resolved.insert(index as u32, text.clone());
        }
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_descriptor_patch_loads_without_legacy_text_and_rejects_invalid_dialogue_atomically()
    {
        let campaign = Campaign {
            current_mission_idx: Some(0),
            missions: vec![robin_engine::mission::Mission {
                profile_idx: Some(0),
                ..Default::default()
            }],
            ..Default::default()
        };
        let profiles = engine_profiles::ProfileManager {
            missions: vec![engine_profiles::MissionProfile {
                mission_filename: "DescriptorPatchTest".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        for (patch, succeeds) in [
            (
                r#"[{"op":"replace","path":"/custom_popup_texts","value":["new text"]}]"#,
                true,
            ),
            (
                r#"[
                {"op":"replace","path":"/custom_popup_texts","value":["new text"]},
                {"op":"replace","path":"/custom_dialogue_texts","value":[["missing dialogue"]]}
            ]"#,
                false,
            ),
        ] {
            let vfs = std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new());
            vfs.install_preloaded_asset(
                "Data/Levels/DescriptorPatchTest.descriptors.patch.json",
                patch.as_bytes().to_vec(),
            )
            .unwrap();
            let files = engine_sbfile::SbFileSystem::new(vfs);
            let mut descriptors = assets_res_descr::LevelDescriptors::default();
            descriptors.custom_popup_texts = vec![Some("original".into())];
            let result =
                apply_mission_descriptor_patch(&campaign, &profiles, &mut descriptors, &files);
            assert_eq!(result.is_ok(), succeeds);
            assert_eq!(
                descriptors.custom_popup_texts[0].as_deref(),
                Some(if succeeds { "new text" } else { "original" })
            );
        }
    }

    #[test]
    fn optional_json_absence_and_malformed_authored_content_are_distinct() {
        let vfs = std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new());
        vfs.install_preloaded_asset("bad.json", b"not json".to_vec())
            .unwrap();
        let files = engine_sbfile::SbFileSystem::new(vfs).snapshot();
        assert!(
            read_optional_json::<serde_json::Value>(&files, "missing.json")
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            read_optional_json::<serde_json::Value>(&files, "bad.json"),
            Err(ResourcePreparationError::Malformed { .. })
        ));
    }
}
