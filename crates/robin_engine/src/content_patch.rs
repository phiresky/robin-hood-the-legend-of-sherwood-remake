//! RFC 6902 patches of decoded content, applied before runtime construction.
//!
//! The JSON document is temporary: a failed operation, unknown field, invalid
//! Rust value or content validation never changes the caller's live data.

use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

pub const PROFILE_PATCH_PATH: &str = "Data/Configuration/profiles.patch.json";

/// Obsolete mod files are errors, rather than silently ignored content.
pub fn reject_legacy(
    files: &crate::sbfile::SbFileSystem,
    old: &str,
    new: &str,
) -> Result<(), String> {
    if files
        .try_exists(old)
        .map_err(|status| format!("probe {old}: file error {status}"))?
    {
        return Err(format!(
            "{old} is no longer supported; migrate to RFC 6902 operations in {new}"
        ));
    }
    Ok(())
}

/// Apply one standard operation array. Callers own domain validation and only
/// install the returned value once it succeeds.
pub fn apply<T: Serialize + DeserializeOwned>(base: &T, bytes: &[u8]) -> Result<T, String> {
    let mut document = serde_json::to_value(base).map_err(|error| error.to_string())?;
    apply_value(&mut document, bytes)?;
    decode(document)
}

fn apply_value(document: &mut Value, bytes: &[u8]) -> Result<(), String> {
    let operations: json_patch::Patch =
        serde_json::from_slice(bytes).map_err(|error| format!("invalid JSON Patch: {error}"))?;
    json_patch::patch(document, &operations).map_err(|error| error.to_string())
}

fn decode<T: DeserializeOwned>(document: Value) -> Result<T, String> {
    let mut unknown = Vec::new();
    let result = serde_ignored::deserialize(document, |path| unknown.push(path.to_string()))
        .map_err(|error| format!("patched content has invalid types: {error}"))?;
    if !unknown.is_empty() {
        return Err(format!("unknown patched fields: {}", unknown.join(", ")));
    }
    Ok(result)
}

/// Read base content patches followed by every overlay's patch, in mount order.
pub fn read_layers(
    files: &crate::sbfile::SbFileSystem,
    path: &str,
) -> Result<Vec<Vec<u8>>, String> {
    files
        .read_all_layers(path)
        .map_err(|status| format!("read {path}: file error {status}"))
}

/// Patch ordinary decoded content through every mounted layer atomically.
pub fn apply_with_files<T: Serialize + DeserializeOwned>(
    base: &T,
    files: &crate::sbfile::SbFileSystem,
    path: &str,
) -> Result<Option<T>, String> {
    let layers = read_layers(files, path)?;
    if layers.is_empty() {
        return Ok(None);
    }
    let mut document = serde_json::to_value(base).map_err(|error| error.to_string())?;
    for (index, bytes) in layers.iter().enumerate() {
        apply_value(&mut document, bytes)
            .map_err(|error| format!("{path}, layer {index}: {error}"))?;
        // Reject a bad layer even if a later mod happens to remove its mistake.
        let _: T =
            decode(document.clone()).map_err(|error| format!("{path}, layer {index}: {error}"))?;
    }
    decode(document).map(Some)
}

const NAMED_FAMILIES: [(&str, &str); 4] = [
    ("characters", "filename"),
    ("soldiers", "filename"),
    ("civilians", "filename"),
    ("missions", "mission_filename"),
];

/// Expose real profile fields with filename keys instead of unstable array
/// offsets. Duplicate filenames use `filename#<original index>`; ambiguous
/// generated keys fail explicitly. Weapon tables retain their numeric IDs.
pub fn profile_document(profiles: &crate::profiles::ProfileManager) -> Result<Value, String> {
    let mut document = serde_json::to_value(profiles).map_err(|error| error.to_string())?;
    for (family, filename_field) in NAMED_FAMILIES {
        let entries = document[family]
            .as_array()
            .expect("serialized profile family is an array");
        let mut names = std::collections::BTreeMap::<&str, usize>::new();
        for entry in entries {
            let filename = entry[filename_field]
                .as_str()
                .expect("serialized profile filename is a string");
            *names.entry(filename).or_default() += 1;
        }
        let mut keyed = serde_json::Map::new();
        for (index, entry) in entries.iter().enumerate() {
            let filename = entry[filename_field].as_str().unwrap();
            let key = if names[filename] > 1 {
                format!("{filename}#{index}")
            } else {
                filename.to_owned()
            };
            let mut entry = entry.clone();
            if family == "characters" {
                entry.as_object_mut().unwrap().remove("index");
            }
            if keyed.insert(key.clone(), entry).is_some() {
                return Err(format!("ambiguous {family} patch key {key:?}"));
            }
        }
        document[family] = Value::Object(keyed);
    }
    Ok(document)
}

/// Apply operations to the named profile view, preserving all existing numeric
/// slots. New entries are appended in key order; character indices are assigned
/// by the loader, never inherited from a copied template.
pub fn apply_profiles(
    profiles: &crate::profiles::ProfileManager,
    bytes: &[u8],
) -> Result<crate::profiles::ProfileManager, String> {
    let original = profile_document(profiles)?;
    let mut document = original.clone();
    apply_value(&mut document, bytes)?;
    let raw = serde_json::to_value(profiles).map_err(|error| error.to_string())?;
    for (family, filename_field) in NAMED_FAMILIES {
        let mut keyed = document[family]
            .as_object()
            .ok_or_else(|| format!("/{family} must be an object keyed by profile filename"))?
            .clone();
        let original_keyed = original[family].as_object().unwrap();
        let original_entries = raw[family].as_array().unwrap();
        let mut entries = Vec::new();
        for (index, entry) in original_entries.iter().enumerate() {
            let filename = entry[filename_field].as_str().unwrap();
            let key = if original_keyed.contains_key(filename) {
                filename.to_owned()
            } else {
                format!("{filename}#{index}")
            };
            entries.push(keyed.remove(&key).ok_or_else(|| {
                format!(
                    "cannot remove /{family}/{key}: existing missions reference its numeric slot"
                )
            })?);
        }
        entries.extend(keyed.into_iter().map(|(_, value)| value));
        if family == "characters" {
            for (index, entry) in entries.iter_mut().enumerate() {
                let entry = entry.as_object_mut().ok_or("character must be an object")?;
                if entry.contains_key("index") {
                    return Err("character index is assigned by the loader".into());
                }
                let index = u32::try_from(index).map_err(|error| error.to_string())?;
                entry.insert("index".into(), Value::from(index));
            }
        }
        document[family] = Value::Array(entries);
    }
    let result: crate::profiles::ProfileManager = decode(document)?;
    for soldier in &result.soldiers {
        for (name, value) in [
            ("intelligence", soldier.intelligence),
            ("courage", soldier.courage),
            ("initiative", soldier.initiative),
            ("pride", soldier.pride),
            ("shooting", soldier.shooting),
            ("fighting", soldier.fighting),
            ("endurance", soldier.endurance),
        ] {
            if value > 100 {
                return Err(format!(
                    "soldier {:?} {name} must be in 0..=100",
                    soldier.filename
                ));
            }
        }
    }
    // TODO: Share comprehensive cross-reference validation with the CPF and
    // mission loaders, including weapon IDs and all authored action ranges.
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::{CharacterProfile, ProfileManager, SoldierProfile};
    use serde_json::json;

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

    #[test]
    fn standard_operations_and_pointer_escaping() {
        let original = json!({"a/b": {"~x": [1, 2]}, "copy": null});
        let patched: Value = apply(
            &original,
            br#"[
            {"op":"test","path":"/a~1b/~0x/0","value":1},
            {"op":"copy","from":"/a~1b/~0x","path":"/copy"},
            {"op":"replace","path":"/copy/0","value":3},
            {"op":"add","path":"/copy/-","value":4},
            {"op":"move","from":"/copy/0","path":"/copy/2"},
            {"op":"remove","path":"/copy/0"}
        ]"#,
        )
        .unwrap();
        assert_eq!(patched["copy"], json!([4, 3]));
        assert_eq!(original["copy"], Value::Null);
        assert!(
            apply::<Value>(
                &original,
                br#"[
            {"op":"replace","path":"/copy","value":3},
            {"op":"test","path":"/copy","value":4}
        ]"#
            )
            .is_err()
        );
        assert_eq!(original["copy"], Value::Null);
    }

    #[test]
    fn clones_actual_profiles_and_assigns_character_indices() {
        let profiles = ProfileManager {
            soldiers: vec![SoldierProfile {
                filename: "Knight03".into(),
                life_point: 50,
                ..Default::default()
            }],
            characters: vec![CharacterProfile {
                filename: "Robin".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let result = apply_profiles(
            &profiles,
            br#"[
            {"op":"replace","path":"/soldiers/Knight03/life_point","value":75},
            {"op":"copy","from":"/soldiers/Knight03","path":"/soldiers/Knight00"},
            {"op":"replace","path":"/soldiers/Knight00/filename","value":"Knight00"},
            {"op":"copy","from":"/characters/Robin","path":"/characters/Archer"},
            {"op":"replace","path":"/characters/Archer/filename","value":"Archer"}
        ]"#,
        )
        .unwrap();
        assert_eq!(result.soldiers[0].filename, "Knight03");
        assert_eq!(result.soldiers[1].filename, "Knight00");
        assert_eq!(result.soldiers[1].life_point, 75);
        assert_eq!(result.characters[1].index, 1);
        assert_eq!(profiles.soldiers[0].life_point, 50);
    }

    #[test]
    fn rejects_typos_invalid_types_and_removing_referenced_profiles() {
        let profiles = ProfileManager {
            soldiers: vec![SoldierProfile {
                filename: "Guard".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        for patch in [
            r#"[{"op":"add","path":"/soldiers/Guard/typo","value":1}]"#,
            r#"[{"op":"replace","path":"/soldiers/Guard/life_point","value":-1}]"#,
            r#"[{"op":"replace","path":"/soldiers/Guard/courage","value":101}]"#,
            r#"[{"op":"remove","path":"/soldiers/Guard"}]"#,
        ] {
            assert!(
                apply_profiles(&profiles, patch.as_bytes()).is_err(),
                "{patch}"
            );
        }
    }

    #[test]
    fn duplicate_filenames_keep_distinct_original_slots() {
        let profiles = ProfileManager {
            soldiers: vec![
                SoldierProfile {
                    filename: "Guard".into(),
                    ..Default::default()
                };
                2
            ],
            ..Default::default()
        };
        let result = apply_profiles(
            &profiles,
            br#"[
            {"op":"replace","path":"/soldiers/Guard#1/life_point","value":75}
        ]"#,
        )
        .unwrap();
        assert_eq!(result.soldiers[0].life_point, 0);
        assert_eq!(result.soldiers[1].life_point, 75);
    }
}
