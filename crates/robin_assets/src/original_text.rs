//! Interpretation of Original menu tables, independent of filesystem authority
//! and of the client's English presentation fallbacks.

use anyhow::{Context, Result};
use std::collections::BTreeMap;

use crate::resource_manager::{ResourceId, ResourceManager};

/// International-build locale search order (English is mounted separately).
pub const LANGUAGE_FOLDERS: &[&str] = &[
    "1031", "2047", "1036", "1040", "2070", "3082", "1049", "1041", "1029", "1045", "1046", "1028",
    "1042", "2052", "1054",
];
pub const FALLBACK_LOCALE_FOLDER: &str = "1033";
pub const MENU_TEXT_TABLE_ID: ResourceId = 1_000_507;
pub const MENU_TEXT_TABLE_ID_DEMO: ResourceId = 1_000_040;
pub const MENU_TEXT_TABLE_ID_DEMO2: ResourceId = 1_000_034;
pub const MENU_TEXT_TABLE_IDS: [ResourceId; 3] = [
    MENU_TEXT_TABLE_ID,
    MENU_TEXT_TABLE_ID_DEMO,
    MENU_TEXT_TABLE_ID_DEMO2,
];

fn effective_index(table_id: ResourceId, strings: &[String], index: usize) -> usize {
    // The first demo lacks "3D sound" at 53. At 167 the layouts converge
    // again because retail removed the old "Display entrances" option.
    if table_id == MENU_TEXT_TABLE_ID
        && (54..=166).contains(&index)
        && strings.get(53).is_some_and(|text| !text.contains("3D"))
    {
        index - 1
    } else {
        index
    }
}

/// Resolve a final-layout ID, retaining its source table and physical index.
/// Missing tables/entries are optional. A present but unreadable table is an
/// error, never silently treated as missing or replaced by another edition.
pub fn menu_text_string(
    resources: &mut ResourceManager,
    index: usize,
) -> Result<Option<(String, ResourceId, usize)>> {
    for table_id in MENU_TEXT_TABLE_IDS {
        if !resources.has_resource(table_id) {
            continue;
        }
        let strings = resources
            .get_strings(table_id)
            .with_context(|| format!("decode Original menu text table {table_id}"))?;
        let effective = effective_index(table_id, strings, index);
        if let Some(text) = strings.get(effective) {
            return Ok(Some((text.clone(), table_id, effective)));
        }
    }
    Ok(None)
}

/// Materialize canonical IDs for a menu that cannot retain the resource manager.
/// Empty entries stay empty; English fallbacks belong to the client.
pub fn load_menu_strings(resources: &mut ResourceManager) -> Result<Vec<String>> {
    let mut count = 0;
    for table_id in MENU_TEXT_TABLE_IDS {
        if resources.has_resource(table_id) {
            count = count.max(
                resources
                    .get_strings(table_id)
                    .with_context(|| format!("decode Original menu text table {table_id}"))?
                    .len(),
            );
        }
    }
    (0..count)
        .map(|index| {
            menu_text_string(resources, index).map(|value| value.map(|v| v.0).unwrap_or_default())
        })
        .collect()
}

pub fn load_peasant_name_pool(
    resources: &mut ResourceManager,
) -> Result<(Vec<String>, Vec<String>)> {
    let mut names = |range: std::ops::Range<usize>| -> Result<Vec<String>> {
        range
            .map(|index| menu_text_string(resources, index).map(|value| value.map(|v| v.0)))
            .filter_map(Result::transpose)
            .collect()
    };
    Ok((names(100..122)?, names(122..144)?))
}

pub fn load_fixed_vip_name_map(
    resources: &mut ResourceManager,
) -> Result<BTreeMap<String, String>> {
    [
        "Robin des bois",
        "Robin des villes",
        "Will Ecarlate",
        "Petit Jean",
        "Frere Tuck",
        "Lady Marianne",
        "Stutely",
    ]
    .into_iter()
    .enumerate()
    .map(|(offset, profile)| {
        menu_text_string(resources, 144 + offset)
            .map(|value| value.map(|v| (profile.to_owned(), v.0)))
    })
    .filter_map(Result::transpose)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resources(tables: &[(ResourceId, Vec<String>)]) -> ResourceManager {
        let vfs = std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new());
        for (id, strings) in tables {
            let mut bytes = b"SRES".to_vec();
            bytes.extend_from_slice(&0x100u32.to_le_bytes());
            bytes.extend_from_slice(&1u32.to_le_bytes());
            bytes.extend_from_slice(b"TEXT");
            bytes.extend_from_slice(&id.to_le_bytes());
            bytes.extend_from_slice(&0u32.to_le_bytes());
            bytes.extend_from_slice(&(strings.len() as u16).to_le_bytes());
            for text in strings {
                let units: Vec<_> = text.encode_utf16().collect();
                bytes.extend_from_slice(&(units.len() as u16).to_le_bytes());
                for unit in units {
                    bytes.extend_from_slice(&unit.to_le_bytes());
                }
            }
            vfs.install_preloaded_asset(&format!("{id}.res"), bytes)
                .unwrap();
        }
        let files = std::sync::Arc::new(robin_engine::sbfile::SbFileSystem::new(vfs).snapshot());
        let mut resources = ResourceManager::with_files(files);
        for (id, _) in tables {
            resources
                .attach_resource_file(&format!("{id}.res"))
                .unwrap();
        }
        resources
    }

    #[test]
    fn client_and_simulation_names_resolve_the_same_canonical_ids_for_every_edition() {
        for (table, old_demo) in [
            (MENU_TEXT_TABLE_ID, false),
            (MENU_TEXT_TABLE_ID, true),
            (MENU_TEXT_TABLE_ID_DEMO, false),
            (MENU_TEXT_TABLE_ID_DEMO2, false),
        ] {
            let mut strings: Vec<_> = (0..180).map(|i| format!("name {i}")).collect();
            if !old_demo {
                strings[53] = "3D sound".into();
            }
            let mut resources = resources(&[(table, strings)]);
            let canonical = load_menu_strings(&mut resources).unwrap();
            let (first, last) = load_peasant_name_pool(&mut resources).unwrap();
            assert_eq!(first, canonical[100..122]);
            assert_eq!(last, canonical[122..144]);
            assert_eq!(
                load_fixed_vip_name_map(&mut resources).unwrap()["Robin des bois"],
                canonical[144]
            );
            let expected = if old_demo { 99 } else { 100 };
            assert_eq!(
                menu_text_string(&mut resources, 100).unwrap(),
                Some((format!("name {expected}"), table, expected))
            );
            assert_eq!(
                canonical[166],
                format!("name {}", if old_demo { 165 } else { 166 })
            );
            assert_eq!(canonical[167], "name 167");
        }
    }

    #[test]
    fn absent_entry_uses_next_edition_but_an_empty_entry_is_not_absent() {
        let mut resources = resources(&[
            (MENU_TEXT_TABLE_ID, vec![String::new()]),
            (
                MENU_TEXT_TABLE_ID_DEMO2,
                vec!["demo zero".into(), "demo one".into()],
            ),
        ]);
        assert_eq!(
            menu_text_string(&mut resources, 0).unwrap(),
            Some((String::new(), MENU_TEXT_TABLE_ID, 0))
        );
        assert_eq!(
            menu_text_string(&mut resources, 1).unwrap(),
            Some(("demo one".into(), MENU_TEXT_TABLE_ID_DEMO2, 1))
        );
    }

    #[test]
    fn present_resource_with_wrong_type_is_an_error_not_a_fallback() {
        let vfs = std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new());
        let mut bytes = b"SRES".to_vec();
        bytes.extend_from_slice(&0x100u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(&MENU_TEXT_TABLE_ID.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        vfs.install_preloaded_asset("wrong.res", bytes).unwrap();
        let files = std::sync::Arc::new(robin_engine::sbfile::SbFileSystem::new(vfs).snapshot());
        let mut resources = ResourceManager::with_files(files);
        resources.attach_resource_file("wrong.res").unwrap();
        assert!(resources.has_resource(MENU_TEXT_TABLE_ID));
        assert!(
            menu_text_string(&mut resources, 100)
                .unwrap_err()
                .to_string()
                .contains("decode Original menu text table")
        );
        assert!(load_peasant_name_pool(&mut resources).is_err());
        assert!(load_menu_strings(&mut resources).is_err());
    }

    #[test]
    fn old_demo_shift_is_bounded_and_only_applies_to_full_table_id() {
        let mut strings: Vec<_> = (0..180).map(|i| format!("text {i}")).collect();
        for (index, expected) in [(53, 53), (54, 53), (100, 99), (166, 165), (167, 167)] {
            assert_eq!(
                effective_index(MENU_TEXT_TABLE_ID, &strings, index),
                expected
            );
            assert_eq!(
                effective_index(MENU_TEXT_TABLE_ID_DEMO, &strings, index),
                index
            );
            assert_eq!(
                effective_index(MENU_TEXT_TABLE_ID_DEMO2, &strings, index),
                index
            );
        }
        strings[53] = "3D sound".into();
        assert_eq!(effective_index(MENU_TEXT_TABLE_ID, &strings, 100), 100);
    }

    #[test]
    fn absent_optional_tables_leave_names_empty() {
        let mut resources = ResourceManager::default();
        assert_eq!(menu_text_string(&mut resources, 100).unwrap(), None);
        assert!(load_menu_strings(&mut resources).unwrap().is_empty());
        assert_eq!(
            load_peasant_name_pool(&mut resources).unwrap(),
            (vec![], vec![])
        );
        assert!(load_fixed_vip_name_map(&mut resources).unwrap().is_empty());
    }
}
