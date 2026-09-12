//! Pure dependency selection and activation-blocking sprite scheduling policy.

use std::collections::BTreeSet;

use anyhow::{Result, anyhow};
use robin_assets::shipping_datadir::ShippingDatadir;

/// The generated layout is a scheduling hint, never a correctness filter.
/// Unknown/legacy paths remain required and receive ordinary data priority.
fn mission_download_priority(path: &str) -> u8 {
    match path.split('/').next() {
        Some("missions") => 0,
        Some("terrain") => 1,
        Some("audio") => 3,
        _ => 2,
    }
}

pub(super) fn prioritize_mission_downloads(files: &mut [String]) {
    files.sort_by_key(|path| mission_download_priority(path));
}

pub(super) struct RequiredMissionDependencies {
    pub(super) files: Vec<String>,
    pub(super) exclamation_ids: BTreeSet<u32>,
}

pub(super) fn required_dependencies(
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
        character_profiles.insert(normalize_robin_profile(
            profiles,
            profile.0,
            reference.forest_level,
        )?);
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
            character_profiles.insert(normalize_robin_profile(
                profiles,
                profile_index.0,
                reference.forest_level,
            )?);
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

/// Robin's stored campaign profile may be
/// either physical variant, but level construction always selects RobinHood
/// for forests and RobinTown for towns.
fn normalize_robin_profile(
    profiles: &robin_engine::profiles::ProfileManager,
    profile_index: u32,
    forest_level: bool,
) -> Result<u32> {
    let profile = profiles
        .characters
        .get(profile_index as usize)
        .ok_or_else(|| anyhow!("required character profile {profile_index} does not exist"))?;
    if !matches!(profile.filename.as_str(), "RobinHood" | "RobinTown") {
        return Ok(profile_index);
    }
    let wanted = if forest_level {
        "RobinHood"
    } else {
        "RobinTown"
    };
    profiles
        .characters
        .iter()
        .position(|candidate| candidate.filename == wanted)
        .map(|index| index as u32)
        .ok_or_else(|| {
            anyhow!(
                "required {wanted} profile is absent while normalizing Robin for a {} mission",
                if forest_level { "forest" } else { "town" }
            )
        })
}

/// Runtime partition of the mission's VQ sprite chunks into the critical
/// set (blocks activation) and the deferrable tail (streams afterwards).
///
/// Deferrable candidates are the reinforcement-only gang characters: gang
/// profiles eligible for runtime reinforcement selection whose RHS is not
/// also required by the mission team. Names are *excluded* from the
/// candidate set — turning them critical — when the level lists them among
/// its start entities (authored soldiers, civilians, PCs to rescue), and
/// *promoted* when any critical chunk names them as a coding base (family
/// hubs must decode before their variants). Everything not in the candidate
/// set is critical by default, so a misjudged chunk can only err toward
/// blocking activation — never toward a missing start-visible sprite.
///
/// Compiled on every target (only the streaming driver is browser-only) so
/// the partition rules stay unit-testable — the safety of the whole feature
/// rests on them.
#[cfg_attr(
    not(all(target_arch = "wasm32", feature = "wasm-threads")),
    allow(dead_code)
)]
pub(super) struct SpriteDeferral {
    candidates: BTreeSet<String>,
    pub(super) parked: Vec<robin_assets::shipping_datadir::SpriteVqChunk>,
    pub(super) level_filtered: bool,
    forest_level: bool,
}

#[cfg_attr(
    not(all(target_arch = "wasm32", feature = "wasm-threads")),
    allow(dead_code)
)]
impl SpriteDeferral {
    pub(super) fn new(
        datadir: &ShippingDatadir,
        mission: &str,
        campaign: &robin_engine::campaign::Campaign,
        profiles: &robin_engine::profiles::ProfileManager,
    ) -> Result<Self> {
        let reference = datadir
            .mission_ref(mission)
            .ok_or_else(|| anyhow!("shipping datadir does not contain mission {mission}"))?;
        let forest_level = reference.forest_level;
        // The Sherwood camp (always campaign mission 0) and its outro are
        // populated from the uninstanced gang itself — exactly the set that
        // is deferrable everywhere else — so nothing may be deferred there.
        let sherwood = campaign
            .missions
            .get(campaign.get_sherwood_mission_idx())
            .and_then(|m| m.profile_idx)
            .and_then(|idx| profiles.missions.get(idx as usize))
            .is_some_and(|profile| profile.mission_filename == mission)
            || mission == "SherwoodOutro";
        if sherwood {
            return Ok(Self {
                candidates: BTreeSet::new(),
                parked: Vec::new(),
                level_filtered: false,
                forest_level,
            });
        }
        // These lookups were all validated by `required_dependencies`
        // moments earlier; failures here are genuine data errors.
        let mut team = BTreeSet::new();
        for &character_index in &campaign.mission_team_indices {
            let description = campaign
                .characters
                .get(character_index)
                .ok_or_else(|| anyhow!("mission team references missing character"))?;
            let profile = description
                .character_profile_idx
                .ok_or_else(|| anyhow!("mission-team character has no profile"))?;
            team.insert(normalize_robin_profile(profiles, profile.0, forest_level)?);
        }
        let mut candidates = BTreeSet::new();
        for &character_index in &campaign.gang_indices {
            let description = campaign
                .characters
                .get(character_index)
                .ok_or_else(|| anyhow!("gang references missing campaign character"))?;
            if description.instanced {
                continue;
            }
            let profile_index = description
                .character_profile_idx
                .ok_or_else(|| anyhow!("gang character has no profile"))?;
            let profile = profiles
                .get_character(profile_index)
                .ok_or_else(|| anyhow!("gang character references missing profile"))?;
            if profile.vip {
                continue;
            }
            let normalized = normalize_robin_profile(profiles, profile_index.0, forest_level)?;
            if team.contains(&normalized) {
                continue;
            }
            let filename = &profiles.characters[normalized as usize].filename;
            candidates.insert(format!("Characters/{filename}.rhs"));
        }
        Ok(Self {
            candidates,
            parked: Vec::new(),
            level_filtered: false,
            forest_level,
        })
    }

    /// Sort freshly merged chunks into `pending` (critical) or the parked
    /// list, then promote coding bases named by critical chunks. Adds the
    /// blob bytes of every chunk that lands in `pending` to `decode_total`.
    pub(super) fn absorb(
        &mut self,
        incoming: &mut Vec<robin_assets::shipping_datadir::SpriteVqChunk>,
        pending: &mut Vec<robin_assets::shipping_datadir::SpriteVqChunk>,
        decode_total: &mut u64,
    ) {
        for chunk in incoming.drain(..) {
            if self.candidates.contains(&chunk.rhs) {
                self.parked.push(chunk);
            } else {
                *decode_total += chunk.blob.len() as u64;
                pending.push(chunk);
            }
        }
        self.promote_bases(pending, decode_total);
    }

    /// Turn one candidate name critical: parked chunks for it move into
    /// `pending`. No-op for names that are not (or no longer) candidates.
    fn remove_candidate(
        &mut self,
        name: &str,
        pending: &mut Vec<robin_assets::shipping_datadir::SpriteVqChunk>,
        decode_total: &mut u64,
    ) -> bool {
        if !self.candidates.remove(name) {
            return false;
        }
        let mut index = 0;
        while index < self.parked.len() {
            if self.parked[index].rhs == name {
                let chunk = self.parked.swap_remove(index);
                *decode_total += chunk.blob.len() as u64;
                pending.push(chunk);
            } else {
                index += 1;
            }
        }
        true
    }

    /// Fixpoint: any candidate named as `base_rhs`/`base2_rhs` by a chunk in
    /// the critical `pending` list becomes critical itself (its grids gate
    /// the critical chunk's decode). Chunks moved out of the parked list are
    /// re-scanned, so hub-of-hub chains resolve fully.
    fn promote_bases(
        &mut self,
        pending: &mut Vec<robin_assets::shipping_datadir::SpriteVqChunk>,
        decode_total: &mut u64,
    ) {
        loop {
            let referenced: Vec<String> = pending
                .iter()
                .flat_map(|chunk| {
                    chunk
                        .base_rhs
                        .iter()
                        .cloned()
                        .chain((!chunk.base2_rhs.is_empty()).then(|| chunk.base2_rhs.clone()))
                })
                .filter(|name| self.candidates.contains(name))
                .collect();
            if referenced.is_empty() {
                return;
            }
            for name in referenced {
                self.remove_candidate(&name, pending, decode_total);
            }
        }
    }

    /// Once the level payload is merged: names required by start entities
    /// (authored soldiers, civilians, PCs to rescue) become critical. A
    /// candidate misclassification can only leave a start sprite streaming
    /// briefly (safe-skip draw), so unresolved profile references warn
    /// rather than fail here.
    pub(super) fn exclude_level_requirements(
        &mut self,
        level: &robin_engine::level_data::LoadedLevel,
        profiles: &robin_engine::profiles::ProfileManager,
        pending: &mut Vec<robin_assets::shipping_datadir::SpriteVqChunk>,
        decode_total: &mut u64,
    ) {
        self.level_filtered = true;
        let names = self.level_start_rhs_names(level, profiles);
        self.exclude_names(&names, pending, decode_total);
    }

    /// RHS names of every character the level itself places at mission
    /// start. Unresolvable profile references only cost prioritization
    /// accuracy (worst case: a brief safe-skip), so they warn rather than
    /// fail.
    fn level_start_rhs_names(
        &self,
        level: &robin_engine::level_data::LoadedLevel,
        profiles: &robin_engine::profiles::ProfileManager,
    ) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for soldier in &level.mission.soldiers {
            if let Some(profile) = profiles.soldiers.get(soldier.profile_number as usize) {
                names.insert(format!("Characters/{}.rhs", profile.filename));
            }
        }
        for civilian in &level.mission.civilians {
            if let Some(profile) = profiles.civilians.get(civilian.profile_number as usize) {
                names.insert(format!("Characters/{}.rhs", profile.filename));
            }
        }
        for rescue in &level.mission.pcs_to_rescue {
            match normalize_robin_profile(profiles, rescue.profile_index, self.forest_level) {
                Ok(normalized) => {
                    if let Some(profile) = profiles.characters.get(normalized as usize) {
                        names.insert(format!("Characters/{}.rhs", profile.filename));
                    }
                }
                Err(error) => tracing::warn!(
                    profile_index = rescue.profile_index,
                    "cannot resolve rescue-PC profile for sprite prioritization: {error:#}"
                ),
            }
        }
        names
    }

    /// Make every named RHS critical, then re-run base promotion.
    fn exclude_names(
        &mut self,
        names: &BTreeSet<String>,
        pending: &mut Vec<robin_assets::shipping_datadir::SpriteVqChunk>,
        decode_total: &mut u64,
    ) {
        for name in names {
            self.remove_candidate(name, pending, decode_total);
        }
        self.promote_bases(pending, decode_total);
    }
}

/// Leave one pool worker available to short part decompression jobs. A
/// one-worker pool cannot reserve a worker and still make sprite progress.
#[cfg(any(test, all(target_arch = "wasm32", feature = "wasm-threads")))]
pub(super) fn streaming_worker_budget(threads: usize, fetching: bool, reserved: usize) -> usize {
    threads
        .saturating_sub(usize::from(fetching))
        .max(usize::from(threads > 0))
        .saturating_sub(reserved)
}

#[cfg(test)]
mod tests {
    use super::{SpriteDeferral, required_dependencies};
    use robin_assets::shipping_datadir::{ShippingDatadir, ShippingMissionRef, SpriteVqChunk};
    use robin_engine::campaign::{Campaign, PcDescription};
    use robin_engine::profiles::{CharacterProfile, CharacterProfileIdx, ProfileManager};

    #[test]
    fn download_order_unblocks_terrain_and_sprites_before_audio_metadata() {
        let original = [
            "audio/voice",
            "rhs/base",
            "terrain/map",
            "missions/header",
            "custom/part",
        ];
        let mut files: Vec<String> = original.iter().map(|path| (*path).into()).collect();
        super::prioritize_mission_downloads(&mut files);
        assert_eq!(
            files,
            [
                "missions/header",
                "terrain/map",
                "rhs/base",
                "custom/part",
                "audio/voice"
            ]
        );
        let mut unchanged_set: Vec<_> = original.iter().map(|path| (*path).to_owned()).collect();
        unchanged_set.sort();
        files.sort();
        assert_eq!(
            files, unchanged_set,
            "scheduling must never omit save/audio dependencies"
        );
    }

    #[test]
    fn streaming_budget_reserves_part_and_terrain_capacity() {
        assert_eq!(super::streaming_worker_budget(8, true, 0), 7);
        assert_eq!(super::streaming_worker_budget(8, true, 1), 6);
        assert_eq!(super::streaming_worker_budget(8, false, 1), 7);
        assert_eq!(super::streaming_worker_budget(8, false, 0), 8);
        assert_eq!(super::streaming_worker_budget(1, true, 0), 1);
        assert_eq!(super::streaming_worker_budget(1, true, 1), 0);
        assert_eq!(super::streaming_worker_budget(0, false, 0), 0);
    }

    fn description(profile: u32, instanced: bool) -> PcDescription {
        PcDescription {
            character_profile_idx: Some(CharacterProfileIdx(profile)),
            instanced,
            ..PcDescription::default()
        }
    }

    // ── Critical-set partition (`SpriteDeferral`) ────────────────────

    fn chunk(rhs: &str, base: Option<&str>, base2: &str, blob_len: usize) -> SpriteVqChunk {
        SpriteVqChunk {
            rhs: rhs.to_owned(),
            base_rhs: base.map(str::to_owned),
            base2_rhs: base2.to_owned(),
            alphabet: 16,
            sprite_ids: Vec::new(),
            base_ids: Vec::new(),
            base2_ids: Vec::new(),
            self_refs: false,
            blob: vec![0; blob_len],
        }
    }

    fn named(filename: &str, vip: bool) -> CharacterProfile {
        CharacterProfile {
            filename: filename.to_owned(),
            vip,
            ..CharacterProfile::default()
        }
    }

    /// Team hero (0), reinforcement-eligible Merry Men (1, 2), a VIP gang
    /// hero (3), and an already-instanced Merry Man (4).
    fn deferral_fixture() -> (ShippingDatadir, Campaign, ProfileManager) {
        let mut datadir = ShippingDatadir::default();
        datadir.missions.insert(
            "H01".into(),
            ShippingMissionRef {
                forest_level: false,
                files: vec!["missions/h01".into()],
            },
        );
        datadir.missions.insert(
            "SherwoodOutro".into(),
            ShippingMissionRef {
                forest_level: true,
                files: vec!["missions/sherwood-outro".into()],
            },
        );
        let mut profiles = ProfileManager::new();
        profiles.characters = vec![
            named("RobinTown", true),
            named("MerryManA", false),
            named("MerryManB", false),
            named("LittleJohn", true),
            named("MerryManC", false),
        ];
        let campaign = Campaign {
            characters: vec![
                description(0, false),
                description(1, false),
                description(2, false),
                description(3, false),
                description(4, true),
            ],
            mission_team_indices: vec![0],
            gang_indices: vec![1, 2, 3, 4],
            ..Default::default()
        };
        (datadir, campaign, profiles)
    }

    fn deferral(mission: &str) -> SpriteDeferral {
        let (datadir, campaign, profiles) = deferral_fixture();
        SpriteDeferral::new(&datadir, mission, &campaign, &profiles).expect("build deferral")
    }

    #[test]
    fn only_reinforcement_eligible_gang_characters_are_deferrable() {
        let mut deferral = deferral("H01");
        // Uninstanced non-VIP gang members, and only those.
        assert_eq!(
            deferral.candidates,
            ["Characters/MerryManA.rhs", "Characters/MerryManB.rhs"]
                .map(str::to_owned)
                .into()
        );

        let mut incoming = vec![
            chunk("Characters/MerryManA.rhs", None, "", 100),
            chunk("Characters/RobinTown.rhs", None, "", 200),
            chunk("Characters/LittleJohn.rhs", None, "", 300),
            chunk("Animations/Day/Cart.rhs", None, "", 400),
        ];
        let mut pending = Vec::new();
        let mut decode_total = 0u64;
        deferral.absorb(&mut incoming, &mut pending, &mut decode_total);

        let critical: Vec<&str> = pending.iter().map(|c| c.rhs.as_str()).collect();
        assert_eq!(
            critical,
            [
                "Characters/RobinTown.rhs",
                "Characters/LittleJohn.rhs",
                "Animations/Day/Cart.rhs"
            ]
        );
        assert_eq!(decode_total, 200 + 300 + 400);
        let parked: Vec<&str> = deferral.parked.iter().map(|c| c.rhs.as_str()).collect();
        assert_eq!(parked, ["Characters/MerryManA.rhs"]);
    }

    #[test]
    fn coding_bases_of_critical_chunks_are_promoted_transitively() {
        let mut deferral = deferral("H01");
        // A critical chunk codes against MerryManB, which itself codes
        // against MerryManA: both hubs must decode before activation.
        let mut incoming = vec![
            chunk("Characters/MerryManA.rhs", None, "", 10),
            chunk(
                "Characters/MerryManB.rhs",
                Some("Characters/MerryManA.rhs"),
                "",
                20,
            ),
            chunk(
                "Characters/RobinTown.rhs",
                Some("Characters/MerryManB.rhs"),
                "",
                30,
            ),
        ];
        let mut pending = Vec::new();
        let mut decode_total = 0u64;
        deferral.absorb(&mut incoming, &mut pending, &mut decode_total);

        assert!(deferral.parked.is_empty(), "every hub must be promoted");
        assert!(deferral.candidates.is_empty());
        assert_eq!(decode_total, 10 + 20 + 30);
    }

    #[test]
    fn second_predecessor_hubs_are_promoted_too() {
        let mut deferral = deferral("H01");
        let mut incoming = vec![
            chunk("Characters/MerryManB.rhs", None, "", 20),
            chunk(
                "Characters/RobinTown.rhs",
                Some("Characters/LittleJohn.rhs"),
                "Characters/MerryManB.rhs",
                30,
            ),
        ];
        let mut pending = Vec::new();
        let mut decode_total = 0u64;
        deferral.absorb(&mut incoming, &mut pending, &mut decode_total);

        assert!(deferral.parked.is_empty(), "base2 hub must be promoted");
        assert_eq!(decode_total, 20 + 30);
    }

    /// A deferrable candidate that the level actually spawns at mission
    /// start stops being deferrable — the safety property that keeps
    /// start-visible sprites out of the streaming tail.
    #[test]
    fn level_start_entities_pull_their_chunks_back_into_the_critical_set() {
        let mut deferral = deferral("H01");
        // MerryManB is deferrable but codes against MerryManA, so making
        // MerryManA critical must also promote nothing extra; making a
        // start-spawned character critical must pull its own chunk back.
        let mut incoming = vec![
            chunk("Characters/MerryManA.rhs", None, "", 10),
            chunk("Characters/MerryManB.rhs", None, "", 20),
        ];
        let mut pending = Vec::new();
        let mut decode_total = 0u64;
        deferral.absorb(&mut incoming, &mut pending, &mut decode_total);
        assert_eq!(deferral.parked.len(), 2, "both start out deferrable");
        assert_eq!(decode_total, 0);

        // The level spawns MerryManA at mission start.
        deferral.exclude_names(
            &["Characters/MerryManA.rhs".to_owned()].into(),
            &mut pending,
            &mut decode_total,
        );

        let critical: Vec<&str> = pending.iter().map(|c| c.rhs.as_str()).collect();
        assert_eq!(critical, ["Characters/MerryManA.rhs"]);
        assert_eq!(decode_total, 10);
        let parked: Vec<&str> = deferral.parked.iter().map(|c| c.rhs.as_str()).collect();
        assert_eq!(parked, ["Characters/MerryManB.rhs"]);
    }

    /// A start-spawned character that is itself a coding base drags its
    /// dependent chunk's hub chain along.
    #[test]
    fn excluding_a_name_promotes_its_dependent_hubs() {
        let mut deferral = deferral("H01");
        let mut incoming = vec![
            chunk("Characters/MerryManA.rhs", None, "", 10),
            chunk(
                "Characters/MerryManB.rhs",
                Some("Characters/MerryManA.rhs"),
                "",
                20,
            ),
        ];
        let mut pending = Vec::new();
        let mut decode_total = 0u64;
        deferral.absorb(&mut incoming, &mut pending, &mut decode_total);
        assert_eq!(deferral.parked.len(), 2);

        // The level spawns MerryManB; its base hub MerryManA must follow.
        deferral.exclude_names(
            &["Characters/MerryManB.rhs".to_owned()].into(),
            &mut pending,
            &mut decode_total,
        );
        assert!(deferral.parked.is_empty());
        assert_eq!(decode_total, 30);
    }

    /// Sherwood is populated from the uninstanced gang itself, so nothing
    /// there may be deferred.
    #[test]
    fn sherwood_defers_nothing() {
        let (datadir, campaign, profiles) = deferral_fixture();
        let deferral = SpriteDeferral::new(&datadir, "SherwoodOutro", &campaign, &profiles)
            .expect("build deferral");
        assert!(deferral.candidates.is_empty());
    }

    #[test]
    fn required_files_adds_team_and_eligible_reinforcement_profiles() {
        let mut datadir = ShippingDatadir::default();
        datadir.missions.insert(
            "H01".into(),
            ShippingMissionRef {
                forest_level: false,
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
        let campaign = Campaign {
            characters: vec![
                description(0, false),
                description(1, false),
                description(2, false),
                description(3, true),
            ],
            mission_team_indices: vec![0],
            gang_indices: vec![1, 2, 3],
            ..Default::default()
        };

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
                forest_level: false,
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
                forest_level: false,
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

    #[test]
    fn required_files_selects_only_the_mission_robin_variant() {
        let mut datadir = ShippingDatadir::default();
        datadir.missions.insert(
            "Forest".into(),
            ShippingMissionRef {
                forest_level: true,
                files: vec!["missions/forest".into()],
            },
        );
        datadir.missions.insert(
            "Town".into(),
            ShippingMissionRef {
                forest_level: false,
                files: vec!["missions/town".into()],
            },
        );
        datadir
            .mission_exclamation_ids
            .insert("Forest".into(), Vec::new());
        datadir
            .mission_exclamation_ids
            .insert("Town".into(), Vec::new());
        datadir
            .character_rhs_files
            .insert(0, vec!["rhs/robin-hood".into()]);
        datadir
            .character_rhs_files
            .insert(1, vec!["rhs/robin-town".into()]);
        datadir.character_audio_files.insert(0, Vec::new());
        datadir.character_audio_files.insert(1, Vec::new());

        let mut profiles = ProfileManager::new();
        profiles.characters = vec![
            CharacterProfile {
                filename: "RobinHood".into(),
                ..CharacterProfile::default()
            },
            CharacterProfile {
                filename: "RobinTown".into(),
                ..CharacterProfile::default()
            },
        ];
        let mut campaign = Campaign::default();
        campaign.characters.push(description(0, false));
        campaign.mission_team_indices.push(0);

        let forest = required_dependencies(&datadir, "Forest", &campaign, &profiles, false)
            .unwrap()
            .files;
        let town = required_dependencies(&datadir, "Town", &campaign, &profiles, false)
            .unwrap()
            .files;
        assert_eq!(
            forest,
            vec!["missions/forest".to_owned(), "rhs/robin-hood".to_owned()]
        );
        assert_eq!(
            town,
            vec!["missions/town".to_owned(), "rhs/robin-town".to_owned()]
        );
    }
}
