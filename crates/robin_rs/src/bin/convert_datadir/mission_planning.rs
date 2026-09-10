//! Discover mission payloads and their complete runtime dependency roots.
use super::*;

pub(super) fn plan_missions(
    dd: &mut ShippingDatadir,
    cpf: &ProfileManager,
    locale_dirs: &[LocaleSource],
    beggar_ids: &BTreeSet<u32>,
    in_path: &dyn Fn(&str) -> Option<PathBuf>,
) -> Result<std::collections::BTreeMap<String, ShippingMissionBuild>> {
    // Missions → .rhp/.rhm/.scb/.red, also follow level sprite refs.
    let mut mission_builds = std::collections::BTreeMap::<String, ShippingMissionBuild>::new();

    // Mission descriptors are authoritative campaign/UI data even when the
    // profile deliberately points at the non-loadable `Impossible_mission`
    // sentinel. Parse them independently from the RHP/RHM payload loop.
    for mp in &cpf.missions {
        let filename = res_descr::red_filename(mp.id);
        let red_rel = format!("Text/{filename}");
        if let Some(red_path) = in_path(&red_rel) {
            dd.red_files.insert(
                filename.clone(),
                res_descr::load(&red_path.to_string_lossy())?,
            );
        } else {
            // Some stock profiles have no descriptor in the source install.
            // Preserve that absence; never synthesize authoritative UI data.
            tracing::warn!(
                profile_id = mp.id,
                "source mission descriptor is absent: {red_rel}"
            );
        }
        for source in locale_dirs {
            let Some(path) = resolve_data_file(&source.data_dir, &red_rel) else {
                continue;
            };
            let descriptors = res_descr::load(&path.to_string_lossy())?;
            dd.locales
                .get_mut(source.iso)
                .expect("detected shipping locale was initialized")
                .red_files
                .insert(canonical_shipping_asset_key(&filename), descriptors);
        }
    }

    // Dialogue descriptors refer to WAVE tables in the localized Level.res.
    // Resolve those tables while building each mission's dependency closure;
    // shipping every locale's dialogue at boot would defeat split loading.
    let level_res_path =
        in_path("Text/Level.res").ok_or_else(|| anyhow!("Text/Level.res missing"))?;
    let mut level_res = ResourceManager::legacy_tool();
    level_res.attach_resource_file(&level_res_path.to_string_lossy())?;

    for mp in &cpf.missions {
        if mp.proto_level_filename.is_empty() || mp.mission_filename.is_empty() {
            continue;
        }
        let rhp_rel = format!("Levels/{}.rhp", mp.proto_level_filename);
        let rhm_rel = format!("Levels/{}.rhm", mp.mission_filename);
        let scb_rel = format!("Levels/{}.scb", mp.mission_filename);

        let Some(rhp_path) = in_path(&rhp_rel) else {
            tracing::warn!("missing: {}", rhp_rel);
            continue;
        };
        let Some(rhm_path) = in_path(&rhm_rel) else {
            tracing::warn!("missing: {}", rhm_rel);
            continue;
        };

        let (proto, mission) = parse_level_pair(&rhp_path, &rhm_path, beggar_ids)?;
        let forest_level = proto
            .misc
            .as_ref()
            .ok_or_else(|| {
                anyhow!(
                    "proto level {} has no MISC forest-level metadata",
                    mp.proto_level_filename
                )
            })?
            .forest_level;
        let mut build = ShippingMissionBuild {
            proto_filename: mp.proto_level_filename.clone(),
            forest_level,
            ambiance: mission.header.ambiance,
            ..ShippingMissionBuild::default()
        };
        build.music_names.extend(
            [&mp.green_music, &mp.yellow_music, &mp.red_music]
                .into_iter()
                .filter(|name| !name.is_empty())
                .cloned(),
        );
        build.sound_wave_ids.extend(
            proto
                .sound_sources
                .iter()
                .filter(|source| source.id >= 0)
                .map(|source| source.id as u32),
        );
        let red_filename = res_descr::red_filename(mp.id);
        if let Some(descriptors) = dd.red_files.get(&red_filename) {
            for (dialogue_index, dialogue) in descriptors.dialogues.iter().enumerate() {
                for sentence_index in 0..dialogue.portrait_ids.len() {
                    match level_res.get_sample(dialogue.sound_table_id, sentence_index) {
                        Ok(sample) if !sample.is_empty() => {
                            build
                                .dialogue_samples
                                .insert(format!("Text/{}", sample.replace('\\', "/")));
                        }
                        Ok(_) => {}
                        Err(error) => {
                            return Err(error).with_context(|| {
                                format!(
                                    "resolve dialogue sample for mission {} dialogue {dialogue_index} sentence {sentence_index}",
                                    mp.mission_filename
                                )
                            });
                        }
                    }
                }
            }
        }
        let required_rhs_profiles = &mut build.required_rhs_profiles;
        // Demo boot hardcodes its party; preserve those profiles even when
        // the mission script does not name them directly.
        if mp.mission_filename == "Dem_Lei_MP" {
            add_required_pc_profiles_for_pcs(
                required_rhs_profiles,
                cpf,
                "RJMT",
                forest_level,
                &in_path,
            );
        } else if mp.mission_filename == "Demo_Lin" {
            add_required_pc_profiles_for_pcs(
                required_rhs_profiles,
                cpf,
                "RSABC",
                forest_level,
                &in_path,
            );
        }
        // Collect sprite/map refs.
        for p in &proto.patches {
            add_required_animation_rhs_profile(
                required_rhs_profiles,
                mission.header.ambiance,
                &p.element_fx.sprite,
                &in_path,
            );
        }
        for fx in &proto.animations {
            add_required_animation_rhs_profile(
                required_rhs_profiles,
                mission.header.ambiance,
                &fx.sprite,
                &in_path,
            );
        }
        if !mission.header.map_filename.is_empty() {
            build.map_names.insert(mission.header.map_filename.clone());
        }
        for &idx in &mp.required_character_indices {
            let idx = normalize_robin_profile_index(cpf, idx as usize, forest_level)?;
            add_required_character_rhs_profiles_for_index(
                required_rhs_profiles,
                cpf,
                idx,
                &in_path,
            )?;
            if let Some(profile) = cpf.characters.get(idx)
                && profile.exclamation_id != 0
            {
                build
                    .required_exclamation_ids
                    .insert(profile.exclamation_id);
            }
        }
        for p in &mission.mission_patches {
            add_required_animation_rhs_profile(
                required_rhs_profiles,
                mission.header.ambiance,
                &p.element_fx.sprite,
                &in_path,
            );
        }
        for target in &mission.targets {
            let rel =
                animation_rhs_rel_existing(mission.header.ambiance, &target.filename, &in_path);
            if in_path(&rel).is_some() {
                add_required_rhs_rel(required_rhs_profiles, rel, &target.profile_name);
            } else {
                // TODO: Determine why a few stock level records name an RHS
                // that does not exist in any original animation directory.
                tracing::warn!(
                    mission = mp.mission_filename,
                    "source target RHS is absent: {rel}"
                );
            }
        }
        for mobile in &mission.mobile_elements {
            for fx in &mobile.sprites {
                add_required_animation_rhs_profile(
                    required_rhs_profiles,
                    mission.header.ambiance,
                    &fx.sprite,
                    &in_path,
                );
            }
        }
        for soldier in &mission.soldiers {
            if let Some(profile) = cpf.soldiers.get(soldier.profile_number as usize) {
                if profile.exclamation_id != 0 {
                    build
                        .required_exclamation_ids
                        .insert(profile.exclamation_id);
                }
                add_required_rhs_rel(
                    required_rhs_profiles,
                    format!("Characters/{}.rhs", profile.filename),
                    &profile.profile_name,
                );
            }
        }
        for civilian in &mission.civilians {
            if let Some(profile) = cpf.civilians.get(civilian.profile_number as usize) {
                if profile.exclamation_id != 0 {
                    build
                        .required_exclamation_ids
                        .insert(profile.exclamation_id);
                }
                add_required_rhs_rel(
                    required_rhs_profiles,
                    format!("Characters/{}.rhs", profile.filename),
                    &profile.profile_name,
                );
            }
        }
        for pc in &mission.pcs_to_rescue {
            let profile_index =
                normalize_robin_profile_index(cpf, pc.profile_index as usize, forest_level)?;
            add_required_character_rhs_profiles_for_index(
                required_rhs_profiles,
                cpf,
                profile_index,
                &in_path,
            )?;
            if let Some(profile) = cpf.characters.get(profile_index)
                && profile.exclamation_id != 0
            {
                build
                    .required_exclamation_ids
                    .insert(profile.exclamation_id);
            }
        }
        for bonus in &mission.bonuses {
            if let Some((file, profile)) = bonus_type_to_sprite_asset_for_shipping(bonus.bonus_type)
            {
                add_required_rhs_rel(
                    required_rhs_profiles,
                    format!("Characters/{file}.rhs"),
                    profile,
                );
            }
        }
        if !mission.scrolls.is_empty() {
            add_required_rhs_rel(
                required_rhs_profiles,
                "Characters/BONUS_Parchment.rhs",
                "BONUS Parchemin",
            );
            add_required_rhs_rel(
                required_rhs_profiles,
                "Characters/BONUS_FourLeavedClover.rhs",
                "BONUS Trefle",
            );
        }
        add_required_rhs_rel(required_rhs_profiles, "Characters/Blip00.rhs", "Blip 00");
        // The original engine creates every object master at level load.
        // These payloads are tiny and must be present now that parsed RHS is
        // authoritative and no raw-file fallback exists.
        add_all_saved_world_object_rhs_profiles(required_rhs_profiles);

        build.payload.levels.insert(
            mp.mission_filename.clone(),
            LoadedLevel {
                proto,
                mission,
                diplomacy: None,
            },
        );

        if let Some(p) = in_path(&scb_rel) {
            let parsed = scb::parse_file(&p).map_err(|e| anyhow!("scb: {e}"))?;
            build
                .payload
                .scripts
                .insert(mp.mission_filename.clone(), parsed);
        } else {
            tracing::warn!("missing: {}", scb_rel);
        }
        mission_builds.insert(mp.mission_filename.clone(), build);
    }

    Ok(mission_builds)
}
