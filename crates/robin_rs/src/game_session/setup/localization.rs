//! Authored mission text and localization overlays.
use super::error::ResourcePreparationError;
use crate::main_entry::current_mission_id;
use robin_assets::{res_descr as assets_res_descr, resource_manager::ResourceManager};
use robin_engine::{campaign::Campaign, profiles as engine_profiles, sbfile as engine_sbfile};
use std::collections::BTreeMap;

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

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CustomMissionTextPatch {
    #[serde(default)]
    popup_texts: BTreeMap<usize, String>,
    #[serde(default)]
    short_briefings: BTreeMap<usize, String>,
    #[serde(default)]
    dialogues: BTreeMap<usize, Vec<String>>,
}

pub(super) fn descriptor_mission_id(
    campaign: &Campaign,
    profiles: &engine_profiles::ProfileManager,
    files: &engine_sbfile::SbFileSystem,
) -> Result<u32, ResourcePreparationError> {
    let current_id = current_mission_id(campaign, profiles);
    let current_profile = campaign
        .current_mission_idx
        .and_then(|index| campaign.missions.get(index))
        .map(|mission| mission.profile(profiles))
        .expect("descriptor lookup requires a current campaign mission");
    let patch_path = format!(
        "Data/Levels/{}.characters.patch.json",
        current_profile.mission_filename
    );
    let Some(patch) = read_optional_json::<serde_json::Value>(files, &patch_path)? else {
        return Ok(current_id);
    };
    let alias = patch
        .get("descriptor_mission")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            ResourcePreparationError::malformed(&patch_path, "missing descriptor_mission")
        })?;
    let mut matches = profiles
        .missions
        .iter()
        .filter(|profile| profile.mission_filename == alias);
    let id = matches
        .next()
        .ok_or_else(|| {
            ResourcePreparationError::malformed(
                &patch_path,
                format!("descriptor mission {alias:?} does not exist"),
            )
        })?
        .id;
    if matches.next().is_some() {
        return Err(ResourcePreparationError::malformed(
            &patch_path,
            format!("descriptor mission {alias:?} is ambiguous"),
        ));
    }
    Ok(id)
}

pub(super) fn apply_custom_mission_text_patch(
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
    let path = format!("Data/Levels/{mission_filename}.text.patch.json");
    let Some(patch) = read_optional_json::<CustomMissionTextPatch>(files, &path)? else {
        return Ok(());
    };
    install_text_patch(descriptors, patch, &path)?;
    tracing::info!("Applied custom mission text patch {path}");
    Ok(())
}

fn install_text_patch(
    descriptors: &mut assets_res_descr::LevelDescriptors,
    patch: CustomMissionTextPatch,
    path: &str,
) -> Result<(), ResourcePreparationError> {
    // Validate before mutating any overlay. A later invalid dialogue must
    // not leave popup/briefing edits installed on a failed preparation.
    for (&index, sentences) in &patch.dialogues {
        let expected = descriptors
            .dialogues
            .get(index)
            .ok_or_else(|| {
                ResourcePreparationError::malformed(
                    path,
                    format!("dialogue {index} has no base descriptor"),
                )
            })?
            .portrait_ids
            .len();
        if sentences.len() != expected {
            return Err(ResourcePreparationError::malformed(
                path,
                format!("dialogue {index} requires {expected} sentences"),
            ));
        }
    }
    for index in patch
        .popup_texts
        .keys()
        .chain(patch.short_briefings.keys())
        .chain(patch.dialogues.keys())
    {
        index
            .checked_add(1)
            .ok_or_else(|| ResourcePreparationError::malformed(path, "text index overflow"))?;
    }
    let install = |target: &mut Vec<Option<String>>, values: BTreeMap<usize, String>| {
        if let Some(max_index) = values.keys().next_back().copied() {
            target.resize(target.len().max(max_index + 1), None);
        }
        for (index, text) in values {
            target[index] = Some(text);
        }
    };
    install(&mut descriptors.custom_popup_texts, patch.popup_texts);
    install(
        &mut descriptors.custom_short_briefings,
        patch.short_briefings,
    );
    if let Some(max_index) = patch.dialogues.keys().next_back().copied() {
        descriptors.custom_dialogue_texts.resize(
            descriptors.custom_dialogue_texts.len().max(max_index + 1),
            None,
        );
    }
    for (index, sentences) in patch.dialogues {
        descriptors.custom_dialogue_texts[index] = Some(sentences);
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

    #[test]
    fn invalid_dialogue_patch_is_atomic() {
        let mut descriptors = assets_res_descr::LevelDescriptors::default();
        descriptors.custom_popup_texts = vec![Some("original".into())];
        let patch = CustomMissionTextPatch {
            popup_texts: BTreeMap::from([(0, "replacement".into())]),
            short_briefings: BTreeMap::from([(0, "new briefing".into())]),
            dialogues: BTreeMap::from([(99, vec!["missing dialogue".into()])]),
        };
        assert!(matches!(
            install_text_patch(&mut descriptors, patch, "mission.text.patch.json"),
            Err(ResourcePreparationError::Malformed { .. })
        ));
        assert_eq!(
            descriptors.custom_popup_texts,
            vec![Some("original".into())]
        );
        assert!(descriptors.custom_short_briefings.is_empty());
    }
}
