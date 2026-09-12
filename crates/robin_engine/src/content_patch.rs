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
        .map_err(|error| format!("content has invalid types: {error}"))?;
    if !unknown.is_empty() {
        return Err(format!("unknown content fields: {}", unknown.join(", ")));
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
    let Some((last, preceding)) = layers.split_last() else {
        return Ok(None);
    };
    let mut document = serde_json::to_value(base).map_err(|error| error.to_string())?;
    for (index, bytes) in preceding.iter().enumerate() {
        apply_value(&mut document, bytes)
            .map_err(|error| format!("{path}, layer {index}: {error}"))?;
        // Reject a bad layer even if a later mod happens to remove its mistake.
        let _: T =
            decode(document.clone()).map_err(|error| format!("{path}, layer {index}: {error}"))?;
    }
    let index = preceding.len();
    apply_value(&mut document, last).map_err(|error| format!("{path}, layer {index}: {error}"))?;
    // The final validation is also the returned value; only intermediate
    // layers need a copy of the document for the next patch to modify.
    decode(document)
        .map(Some)
        .map_err(|error| format!("{path}, layer {index}: {error}"))
}

const NAMED_FAMILIES: [(&str, &str, &str); 4] = [
    ("characters", "filename", "character_order"),
    ("soldiers", "filename", "soldier_order"),
    ("civilians", "filename", "civilian_order"),
    ("missions", "mission_filename", "mission_order"),
];

/// Export the canonical on-disk profile document with explicit numeric slot order.
/// Internal runtime/replay serialization remains separate from authored content.
/// Duplicate filenames use `filename#<original index>`; ambiguous generated keys
/// fail explicitly. Weapon tables retain their numeric IDs.
pub fn profile_document(profiles: &crate::profiles::ProfileManager) -> Result<Value, String> {
    let mut document = serde_json::to_value(profiles).map_err(|error| error.to_string())?;
    for (family, filename_field, order_field) in NAMED_FAMILIES {
        let entries = std::mem::take(
            document[family]
                .as_array_mut()
                .expect("serialized profile family is an array"),
        );
        let mut names = std::collections::BTreeMap::<&str, usize>::new();
        for entry in &entries {
            let filename = entry[filename_field]
                .as_str()
                .expect("serialized profile filename is a string");
            *names.entry(filename).or_default() += 1;
        }
        let mut order = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            let filename = entry[filename_field].as_str().unwrap();
            let key = if names[filename] > 1 {
                format!("{filename}#{index}")
            } else {
                filename.to_owned()
            };
            order.push(Value::String(key));
        }
        // Finish borrowing filenames before moving their containing entries.
        drop(names);
        let mut keyed = serde_json::Map::new();
        for (mut entry, key) in entries.into_iter().zip(&order) {
            let key = key.as_str().expect("generated profile key is a string");
            if family == "characters" {
                entry.as_object_mut().unwrap().remove("index");
            }
            if keyed.insert(key.to_owned(), entry).is_some() {
                return Err(format!("ambiguous {family} patch key {key:?}"));
            }
        }
        document[family] = Value::Object(keyed);
        document[order_field] = Value::Array(order);
    }
    Ok(document)
}

/// Decode the same document accepted on disk and by ordinary JSON Patch tools.
/// Listed keys retain numeric slots; unlisted new keys append in sorted order.
pub fn profiles_from_document(
    mut document: Value,
) -> Result<crate::profiles::ProfileManager, String> {
    for (family, _, order_field) in NAMED_FAMILIES {
        let mut keyed = document.get_mut(family).and_then(Value::as_object_mut).map(std::mem::take).ok_or_else(|| {
            format!("/{family} must be an object keyed by profile name; legacy array catalogs must be regenerated with convert_datadir or re-exported from the original CPF with cpf_to_json")
        })?;
        let order = document[order_field].as_array().ok_or_else(|| {
            format!("/{order_field} must be an array of profile keys preserving numeric slots")
        })?;
        let mut entries = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for key in order {
            let key = key
                .as_str()
                .ok_or_else(|| format!("/{order_field} entries must be strings"))?;
            if !seen.insert(key) {
                return Err(format!("/{order_field} repeats key {key:?}"));
            }
            entries.push(
                keyed
                    .remove(key)
                    .ok_or_else(|| format!("/{order_field} references missing /{family}/{key}"))?,
            );
        }
        // Do not depend on serde_json's optional preserve_order feature.
        let remaining: std::collections::BTreeMap<_, _> = keyed.into_iter().collect();
        entries.extend(remaining.into_values());
        if family == "characters" {
            for (index, entry) in entries.iter_mut().enumerate() {
                let entry = entry.as_object_mut().ok_or("character must be an object")?;
                if entry.contains_key("index") {
                    return Err("character index is assigned from character_order by the loader; remove the index field".into());
                }
                let index = u32::try_from(index).map_err(|error| error.to_string())?;
                entry.insert("index".into(), Value::from(index));
            }
        }
        document[family] = Value::Array(entries);
        document
            .as_object_mut()
            .ok_or("profile document must be an object")?
            .remove(order_field);
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

/// Patch canonical JSON without rebuilding its keys or altering its representation.
/// A failed operation or validation leaves the original document unchanged.
pub fn apply_profile_document(base: &Value, bytes: &[u8]) -> Result<Value, String> {
    profiles_from_document(base.clone())?;
    let mut document = base.clone();
    apply_value(&mut document, bytes)?;
    profiles_from_document(document.clone())?;
    for (family, _, order_field) in NAMED_FAMILIES {
        let original_order = base[order_field].as_array().expect("validated order");
        let patched_order = document[order_field].as_array().expect("validated order");
        if !patched_order.starts_with(original_order) {
            return Err(format!(
                "/{order_field} cannot remove or reorder existing numeric slots"
            ));
        }
        for key in base[family]
            .as_object()
            .expect("validated profile map")
            .keys()
        {
            if document[family].get(key).is_none() {
                return Err(format!(
                    "cannot remove /{family}/{key}: existing missions may reference its numeric slot"
                ));
            }
        }
    }
    Ok(document)
}

/// Convenience entry point for callers starting with runtime profiles.
/// Content loaders retain the canonical document across all patch layers.
pub fn apply_profiles(
    profiles: &crate::profiles::ProfileManager,
    bytes: &[u8],
) -> Result<crate::profiles::ProfileManager, String> {
    profiles_from_document(apply_profile_document(&profile_document(profiles)?, bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::{CharacterProfile, ProfileManager, SoldierProfile};
    use serde_json::json;

    #[cfg(not(target_arch = "wasm32"))]
    mod ordinary_layers {
        use super::*;
        use serde::Deserialize;
        use std::cell::Cell;

        thread_local! {
            static DECODES: Cell<usize> = const { Cell::new(0) };
        }

        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct Content {
            #[serde(deserialize_with = "counted_value")]
            value: u32,
        }

        fn counted_value<'de, D: serde::Deserializer<'de>>(
            deserializer: D,
        ) -> Result<u32, D::Error> {
            DECODES.with(|count| count.set(count.get() + 1));
            u32::deserialize(deserializer)
        }

        const PATH: &str = "ordinary.patch.json";

        fn files(layers: &[&str]) -> (crate::sbfile::SbFileSystem, Vec<tempfile::TempDir>) {
            let vfs = std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new());
            if let Some(first) = layers.first() {
                vfs.install_preloaded_asset(PATH, first.as_bytes().to_vec())
                    .unwrap();
            }
            let files = crate::sbfile::SbFileSystem::new(vfs);
            let mut roots = Vec::new();
            for layer in layers.iter().skip(1) {
                let root = tempfile::tempdir().unwrap();
                std::fs::write(root.path().join(PATH), layer).unwrap();
                assert_eq!(files.add_overlay_path(root.path().to_str().unwrap()), 0);
                roots.push(root);
            }
            (files, roots)
        }

        #[test]
        fn zero_one_and_multiple_layers_decode_once_per_layer_in_mount_order() {
            let patches = [
                r#"[{"op":"replace","path":"/value","value":2}]"#,
                r#"[{"op":"test","path":"/value","value":2},{"op":"replace","path":"/value","value":3}]"#,
                r#"[{"op":"test","path":"/value","value":3},{"op":"replace","path":"/value","value":4}]"#,
            ];
            for count in 0..=patches.len() {
                let (files, _roots) = files(&patches[..count]);
                let base = Content { value: 1 };
                DECODES.with(|value| value.set(0));
                let result = apply_with_files(&base, &files, PATH).unwrap();
                assert_eq!(
                    result,
                    (count != 0).then_some(Content {
                        value: count as u32 + 1
                    })
                );
                assert_eq!(DECODES.with(Cell::get), count);
                assert_eq!(base, Content { value: 1 });
            }
        }

        #[test]
        fn invalid_intermediate_layers_are_rejected_even_if_later_repaired() {
            for (invalid, repair, detail) in [
                (
                    r#"[{"op":"replace","path":"/value","value":"bad"}]"#,
                    r#"[{"op":"replace","path":"/value","value":2}]"#,
                    "content has invalid types",
                ),
                (
                    r#"[{"op":"add","path":"/typo","value":2}]"#,
                    r#"[{"op":"remove","path":"/typo"}]"#,
                    "unknown content fields",
                ),
            ] {
                let (files, _roots) = files(&[invalid, repair]);
                let base = Content { value: 1 };
                let error = apply_with_files(&base, &files, PATH).unwrap_err();
                assert!(
                    error.starts_with("ordinary.patch.json, layer 0:"),
                    "{error}"
                );
                assert!(error.contains(detail), "{error}");
                assert_eq!(base, Content { value: 1 });
            }
        }

        #[test]
        fn final_layer_errors_keep_path_index_and_leave_base_unchanged() {
            for invalid in [
                "not JSON",
                r#"[{"op":"remove","path":"/missing"}]"#,
                r#"[{"op":"replace","path":"/value","value":null}]"#,
                r#"[{"op":"add","path":"/typo","value":2}]"#,
            ] {
                let (files, _roots) = files(&["[]", invalid]);
                let base = Content { value: 1 };
                let error = apply_with_files(&base, &files, PATH).unwrap_err();
                assert!(
                    error.starts_with("ordinary.patch.json, layer 1:"),
                    "{error}"
                );
                assert_eq!(base, Content { value: 1 });
            }
        }
    }

    #[test]
    fn canonical_disk_document_round_trips_slots_and_supports_plain_json_patch() {
        let profiles = ProfileManager {
            soldiers: vec![
                SoldierProfile {
                    filename: "Zulu".into(),
                    life_point: 11,
                    ..Default::default()
                },
                SoldierProfile {
                    filename: "Guard".into(),
                    life_point: 22,
                    ..Default::default()
                },
                SoldierProfile {
                    filename: "Guard".into(),
                    life_point: 33,
                    ..Default::default()
                },
            ],
            characters: vec![CharacterProfile {
                filename: "Robin".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut document = profile_document(&profiles).unwrap();
        assert_eq!(
            document["soldier_order"],
            json!(["Zulu", "Guard#1", "Guard#2"])
        );
        assert!(document["characters"]["Robin"].get("index").is_none());
        assert_eq!(
            serde_json::to_value(profiles_from_document(document.clone()).unwrap()).unwrap(),
            serde_json::to_value(&profiles).unwrap(),
        );
        // Authored keys are identities in their own right, not necessarily filenames.
        let guard = document["soldiers"]
            .as_object_mut()
            .unwrap()
            .remove("Guard#1")
            .unwrap();
        document["soldiers"]["custom/guard~key"] = guard;
        document["soldier_order"][1] = json!("custom/guard~key");
        let patches = [
            br#"[{"op":"copy","from":"/soldiers/custom~1guard~0key","path":"/soldiers/Alpha"}]"#
                .as_slice(),
            br#"[{"op":"replace","path":"/soldiers/Alpha/life_point","value":99}]"#.as_slice(),
        ];
        let mut plain = document.clone();
        for bytes in patches {
            let operations: json_patch::Patch = serde_json::from_slice(bytes).unwrap();
            json_patch::patch(&mut plain, &operations).unwrap();
            document = apply_profile_document(&document, bytes).unwrap();
            assert_eq!(
                plain, document,
                "game must patch exactly the on-disk representation"
            );
        }
        let vfs = std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new());
        vfs.install_preloaded_asset(
            "canonical/profiles.json",
            serde_json::to_vec(&plain).unwrap(),
        )
        .unwrap();
        let loaded = ProfileManager::load_json_with_files(
            "canonical/profiles.json",
            &crate::sbfile::SbFileSystem::new(vfs),
        )
        .unwrap();
        assert_eq!(
            loaded
                .soldiers
                .iter()
                .map(|p| p.life_point)
                .collect::<Vec<_>>(),
            vec![11, 22, 33, 99]
        );
    }

    #[test]
    fn owned_profile_maps_preserve_validation_precedence() {
        for invalid in [
            json!(null),
            json!([]),
            json!({}),
            json!({"characters": [], "character_order": null}),
        ] {
            assert_eq!(
                profiles_from_document(invalid).unwrap_err(),
                "/characters must be an object keyed by profile name; legacy array catalogs must be regenerated with convert_datadir or re-exported from the original CPF with cpf_to_json"
            );
        }
        for (invalid, expected) in [
            (
                json!({"characters": {}, "character_order": null, "soldiers": []}),
                "/character_order must be an array of profile keys preserving numeric slots",
            ),
            (
                json!({"characters": {"Hero": null}, "character_order": [0]}),
                "/character_order entries must be strings",
            ),
            (
                json!({"characters": {"Hero": null}, "character_order": ["Hero", "Hero"]}),
                "/character_order repeats key \"Hero\"",
            ),
            (
                json!({"characters": {}, "character_order": ["Absent", "Absent"]}),
                "/character_order references missing /characters/Absent",
            ),
            (
                json!({"characters": {"Hero": null}, "character_order": ["Hero"], "soldiers": []}),
                "character must be an object",
            ),
            (
                json!({"characters": {"Hero": {"index": 0}}, "character_order": ["Hero"], "soldiers": []}),
                "character index is assigned from character_order by the loader; remove the index field",
            ),
        ] {
            assert_eq!(profiles_from_document(invalid).unwrap_err(), expected);
        }
    }

    #[test]
    fn owned_profile_maps_append_unlisted_keys_in_sorted_order() {
        let profiles = ProfileManager {
            characters: vec![CharacterProfile {
                filename: "Zulu".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut document = profile_document(&profiles).unwrap();
        for (key, name) in [("beta", "Beta"), ("alpha", "Alpha")] {
            let mut entry = document["characters"]["Zulu"].clone();
            entry["filename"] = json!(name);
            document["characters"][key] = entry;
        }
        let loaded = profiles_from_document(document).unwrap();
        assert_eq!(
            loaded
                .characters
                .iter()
                .map(|profile| (profile.filename.as_str(), profile.index))
                .collect::<Vec<_>>(),
            [("Zulu", 0), ("Alpha", 1), ("Beta", 2)]
        );
    }

    #[test]
    fn profile_export_rejects_generated_key_collisions_in_either_order() {
        for (filenames, collision) in [
            (["Guard", "Guard", "Guard#0"], "Guard#0"),
            (["Guard#1", "Guard", "Guard"], "Guard#1"),
        ] {
            let profiles = ProfileManager {
                soldiers: filenames
                    .into_iter()
                    .map(|filename| SoldierProfile {
                        filename: filename.into(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            };
            let original = serde_json::to_value(&profiles).unwrap();
            assert_eq!(
                profile_document(&profiles).unwrap_err(),
                format!("ambiguous soldiers patch key {collision:?}")
            );
            assert_eq!(serde_json::to_value(&profiles).unwrap(), original);
        }
    }

    #[test]
    fn canonical_document_rejects_ambiguous_order_and_legacy_arrays() {
        let profiles = ProfileManager {
            soldiers: vec![SoldierProfile {
                filename: "Guard".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let raw = serde_json::to_value(&profiles).unwrap();
        assert!(
            profiles_from_document(raw)
                .unwrap_err()
                .contains("re-exported")
        );
        let document = profile_document(&profiles).unwrap();
        for order in [
            json!(["Guard", "Guard"]),
            json!(["Missing"]),
            json!([3]),
            json!(null),
        ] {
            let mut invalid = document.clone();
            invalid["soldier_order"] = order;
            assert!(profiles_from_document(invalid).is_err());
        }
        for patch in [
            r#"[{"op":"replace","path":"/soldier_order","value":[]}]"#,
            r#"[{"op":"add","path":"/soldiers/Guard/typo","value":3}]"#,
            r#"[{"op":"add","path":"/characters/Robin","value":{"index":3}}]"#,
        ] {
            assert!(
                apply_profile_document(&document, patch.as_bytes()).is_err(),
                "{patch}"
            );
        }
    }

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
