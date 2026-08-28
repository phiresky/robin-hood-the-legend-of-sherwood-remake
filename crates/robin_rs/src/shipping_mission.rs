//! Platform-specific fetch at the asynchronous mission-load boundary.

use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use futures::{StreamExt as _, TryStreamExt as _};
use robin_assets::shipping_datadir::{ShippingDatadir, ShippingMission, decode_mission_compressed};

const MISSION_FETCH_CONCURRENCY: usize = 8;

/// Ensure the selected mission's independently compressed shipping payload is
/// decoded and mounted before any synchronous level/resource loader runs.
pub async fn ensure_loaded(
    shipping: Option<&Arc<ShippingDatadir>>,
    mission: &str,
    campaign: &robin_engine::campaign::Campaign,
    profiles: &robin_engine::profiles::ProfileManager,
    has_decoded_saved_world: bool,
) -> Result<()> {
    let Some(datadir) = shipping else {
        return Ok(());
    };
    // An empty mission manifest is the loose-file/non-split compatibility
    // shape used by unit tests and development datadirs.
    if datadir.missions.is_empty() {
        return Ok(());
    }
    let dependencies = required_dependencies(
        datadir,
        mission,
        campaign,
        profiles,
        has_decoded_saved_world,
    )?;
    if datadir.is_mission_loaded(mission) {
        datadir
            .activate_mission(mission)
            .with_context(|| format!("activate shipping mission {mission}"))?;
        datadir.set_active_exclamation_ids(dependencies.exclamation_ids);
        return Ok(());
    }
    let files = dependencies.files;
    let exclamation_ids = dependencies.exclamation_ids;
    let mut fetched = futures::stream::iter(files.iter().cloned().map(|file| async move {
        let compressed = fetch(datadir, &file)
            .await
            .with_context(|| format!("fetch shipping file {file}"))?;
        let bytes = compressed.len();
        let payload = decode_mission_compressed(&compressed)
            .with_context(|| format!("decode shipping file {file}"))?;
        Ok::<_, anyhow::Error>((file, bytes, payload))
    }))
    .buffer_unordered(MISSION_FETCH_CONCURRENCY);
    let mut fetched_bytes = 0usize;
    let mut merged = ShippingMission::default();
    while let Some((file, bytes, payload)) = fetched.try_next().await? {
        fetched_bytes += bytes;
        tracing::debug!(mission, file, bytes, "shipping mission dependency fetched");
        merged
            .merge_part(payload)
            .with_context(|| format!("merge shipping file {file}"))?;
    }
    datadir
        .install_mission_parts(mission, std::iter::once(merged))
        .with_context(|| format!("install shipping mission {mission}"))?;
    datadir.set_active_exclamation_ids(exclamation_ids);
    let payload = datadir
        .loaded_mission(mission)
        .ok_or_else(|| anyhow!("shipping mission {mission} disappeared after installation"))?;
    tracing::info!(
        mission,
        files = files.len(),
        fetched_files = files.len(),
        bytes = fetched_bytes,
        rhs_files = payload.rhs_files.len(),
        "shipping mission payload loaded"
    );
    Ok(())
}

struct RequiredMissionDependencies {
    files: Vec<String>,
    exclamation_ids: BTreeSet<u32>,
}

fn required_dependencies(
    datadir: &ShippingDatadir,
    mission: &str,
    campaign: &robin_engine::campaign::Campaign,
    profiles: &robin_engine::profiles::ProfileManager,
    has_decoded_saved_world: bool,
) -> Result<RequiredMissionDependencies> {
    let reference = datadir
        .mission_ref(mission)
        .ok_or_else(|| anyhow!("shipping datadir does not contain mission {mission}"))?;
    let mut files: BTreeSet<String> = reference.files.iter().cloned().collect();
    let mut exclamation_ids: BTreeSet<u32> = datadir
        .mission_exclamation_ids
        .get(mission)
        .ok_or_else(|| {
            anyhow!("shipping manifest has no authored exclamation index for mission {mission}")
        })?
        .iter()
        .copied()
        .collect();
    let mut character_profiles = BTreeSet::new();

    for &character_index in &campaign.mission_team_indices {
        let description = campaign.characters.get(character_index).ok_or_else(|| {
            anyhow!(
                "mission team references missing campaign character {character_index} while loading {mission}"
            )
        })?;
        let profile = description.character_profile_idx.ok_or_else(|| {
            anyhow!(
                "mission-team character {character_index} has no profile while loading {mission}"
            )
        })?;
        character_profiles.insert(profile.0);
    }

    // Reinforcement selection can instantiate any uninstanced, non-VIP gang
    // member during a simulation tick. Include exactly that candidate pool at
    // the asynchronous boundary; the tick itself must remain cache-only.
    for &character_index in &campaign.gang_indices {
        let description = campaign.characters.get(character_index).ok_or_else(|| {
            anyhow!(
                "gang references missing campaign character {character_index} while loading {mission}"
            )
        })?;
        if description.instanced {
            continue;
        }
        let profile_index = description.character_profile_idx.ok_or_else(|| {
            anyhow!("gang character {character_index} has no profile while loading {mission}")
        })?;
        let profile = profiles.get_character(profile_index).ok_or_else(|| {
            anyhow!(
                "gang character {character_index} references missing profile {} while loading {mission}",
                profile_index.0
            )
        })?;
        if !profile.vip {
            character_profiles.insert(profile_index.0);
        }
    }

    for profile_index in character_profiles {
        let dependencies = datadir.character_rhs_files.get(&profile_index).ok_or_else(|| {
            anyhow!(
                "shipping manifest has no RHS dependency index for required character profile {profile_index}"
            )
        })?;
        files.extend(dependencies.iter().cloned());
        let audio_dependencies = datadir
            .character_audio_files
            .get(&profile_index)
            .ok_or_else(|| {
                anyhow!(
                    "shipping manifest has no audio dependency index for required character profile {profile_index}"
                )
            })?;
        files.extend(audio_dependencies.iter().cloned());
        if let Some(&exclamation_id) = datadir.character_exclamation_ids.get(&profile_index) {
            exclamation_ids.insert(exclamation_id);
        }
    }
    if has_decoded_saved_world {
        if datadir.saved_world_rhs_files.is_empty() {
            return Err(anyhow!(
                "shipping manifest has no conservative saved-world RHS dependency set"
            ));
        }
        files.extend(datadir.saved_world_rhs_files.iter().cloned());
    }
    Ok(RequiredMissionDependencies {
        files: files.into_iter().collect(),
        exclamation_ids,
    })
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
async fn fetch(datadir: &ShippingDatadir, relative: &str) -> Result<Vec<u8>> {
    let path = datadir.source_file_path(relative)?;
    std::fs::read(&path).with_context(|| format!("read {}", path.display()))
}

#[cfg(target_os = "android")]
async fn fetch(_datadir: &ShippingDatadir, relative: &str) -> Result<Vec<u8>> {
    crate::android::read_bundled_asset(&format!("Data/{relative}"))
}

#[cfg(target_arch = "wasm32")]
async fn fetch(datadir: &ShippingDatadir, relative: &str) -> Result<Vec<u8>> {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen_futures::JsFuture;

    let base = datadir
        .remote_base_url()
        .ok_or_else(|| anyhow!("browser shipping manifest has no remote base URL"))?;
    let url = format!("{base}/{}", relative.trim_start_matches('/'));
    let window = web_sys::window().ok_or_else(|| anyhow!("browser window is unavailable"))?;
    let response = JsFuture::from(window.fetch_with_str(&url))
        .await
        .map_err(|error| anyhow!("fetch {url}: {error:?}"))?
        .dyn_into::<web_sys::Response>()
        .map_err(|_| anyhow!("fetch {url}: result is not a Response"))?;
    if !response.ok() {
        return Err(anyhow!("fetch {url}: HTTP {}", response.status()));
    }
    let buffer = response
        .array_buffer()
        .map_err(|error| anyhow!("fetch {url}: arrayBuffer: {error:?}"))?;
    let buffer = JsFuture::from(buffer)
        .await
        .map_err(|error| anyhow!("fetch {url}: read body: {error:?}"))?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

#[cfg(test)]
mod tests {
    use super::required_dependencies;
    use robin_assets::shipping_datadir::{ShippingDatadir, ShippingMissionRef};
    use robin_engine::campaign::{Campaign, PcDescription};
    use robin_engine::profiles::{CharacterProfile, CharacterProfileIdx, ProfileManager};

    fn description(profile: u32, instanced: bool) -> PcDescription {
        PcDescription {
            character_profile_idx: Some(CharacterProfileIdx(profile)),
            instanced,
            ..PcDescription::default()
        }
    }

    #[test]
    fn required_files_adds_team_and_eligible_reinforcement_profiles() {
        let mut datadir = ShippingDatadir::default();
        datadir.missions.insert(
            "H01".into(),
            ShippingMissionRef {
                files: vec!["missions/h01".into(), "rhs/static".into()],
            },
        );
        datadir
            .mission_exclamation_ids
            .insert("H01".into(), vec![91]);
        datadir
            .character_rhs_files
            .insert(0, vec!["rhs/team".into(), "rhs/shared".into()]);
        datadir
            .character_rhs_files
            .insert(2, vec!["rhs/reinforcement".into(), "rhs/shared".into()]);
        datadir
            .character_audio_files
            .insert(0, vec!["audio/team-voice".into()]);
        datadir
            .character_audio_files
            .insert(2, vec!["audio/reinforcement-voice".into()]);
        datadir.character_exclamation_ids.insert(0, 100);
        datadir.character_exclamation_ids.insert(2, 102);

        let mut profiles = ProfileManager::new();
        profiles.characters = vec![
            CharacterProfile::default(),
            CharacterProfile {
                vip: true,
                ..CharacterProfile::default()
            },
            CharacterProfile::default(),
            CharacterProfile::default(),
        ];
        let mut campaign = Campaign::default();
        campaign.characters = vec![
            description(0, false),
            description(1, false),
            description(2, false),
            description(3, true),
        ];
        campaign.mission_team_indices = vec![0];
        campaign.gang_indices = vec![1, 2, 3];

        let dependencies =
            required_dependencies(&datadir, "H01", &campaign, &profiles, false).unwrap();
        assert_eq!(
            dependencies.files,
            vec![
                "audio/reinforcement-voice",
                "audio/team-voice",
                "missions/h01",
                "rhs/reinforcement",
                "rhs/shared",
                "rhs/static",
                "rhs/team",
            ]
        );
        assert_eq!(dependencies.exclamation_ids, [91, 100, 102].into());
    }

    #[test]
    fn required_files_adds_explicit_saved_world_closure() {
        let mut datadir = ShippingDatadir::default();
        datadir.missions.insert(
            "H01".into(),
            ShippingMissionRef {
                files: vec!["missions/h01".into()],
            },
        );
        datadir
            .mission_exclamation_ids
            .insert("H01".into(), Vec::new());
        datadir.saved_world_rhs_files = vec!["rhs/all-saved-objects".into()];
        let dependencies = required_dependencies(
            &datadir,
            "H01",
            &Campaign::default(),
            &ProfileManager::new(),
            true,
        )
        .unwrap();
        assert_eq!(
            dependencies.files,
            vec!["missions/h01", "rhs/all-saved-objects"]
        );
    }

    #[test]
    fn required_files_rejects_missing_character_index_entry() {
        let mut datadir = ShippingDatadir::default();
        datadir.missions.insert(
            "H01".into(),
            ShippingMissionRef {
                files: vec!["missions/h01".into()],
            },
        );
        datadir
            .mission_exclamation_ids
            .insert("H01".into(), Vec::new());
        let mut profiles = ProfileManager::new();
        profiles.characters.push(CharacterProfile::default());
        let mut campaign = Campaign::default();
        campaign.characters.push(description(0, false));
        campaign.mission_team_indices.push(0);
        let error = required_dependencies(&datadir, "H01", &campaign, &profiles, false)
            .err()
            .expect("missing profile dependency must fail");
        assert!(error.to_string().contains("profile 0"));
    }
}
