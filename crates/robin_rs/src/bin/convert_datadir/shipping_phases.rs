//! Sequencing phases of `convert_shipping`; the entry point calls them in
//! their original order so file writes, hashes and JSON output are unchanged.
use super::*;
use std::collections::BTreeMap;

/// Top-level fields retain the v4 default-resolution behavior for existing
/// consumers: base Data first, English fallback, then the remaining locale
/// dirs. Explicit per-locale maps never use this fallback resolver.
pub(super) fn default_data_path_resolver<'a>(
    data_in: &'a Path,
    locale_dirs: &'a [LocaleSource],
) -> impl Fn(&str) -> Option<PathBuf> + 'a {
    move |rel: &str| -> Option<PathBuf> {
        if let Some(resolved) = resolve_data_file(data_in, rel) {
            return Some(resolved);
        }
        for alt in locale_dirs {
            if let Some(resolved) = resolve_data_file(&alt.data_dir, rel) {
                return Some(resolved);
            }
        }
        None
    }
}

pub(super) fn register_locales(
    dd: &mut ShippingDatadir,
    locale_dirs: &[LocaleSource],
) -> Result<()> {
    for src in locale_dirs {
        tracing::info!("Locale data dir [{}]: {}", src.iso, src.data_dir.display());
        let mut aliases = BTreeSet::from([src.lcid.to_owned(), src.iso.to_owned()]);
        if src.iso == "und" {
            aliases.insert("neutral".to_owned());
        }
        let locale = ShippingLocale {
            source_lcid: Some(src.lcid.to_owned()),
            aliases,
            ..ShippingLocale::default()
        };
        if dd.locales.insert(src.iso.to_owned(), locale).is_some() {
            bail!(
                "multiple locale directories resolve to canonical locale {}",
                src.iso
            );
        }
    }
    Ok(())
}

/// Loads `profile.cpf` (the root index), records beggar civilians and returns
/// the profile pool plus each character profile's exclamation id.
pub(super) fn load_profile_index(
    beggar_ids: &mut BTreeSet<u32>,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
) -> Result<(ProfileManager, Vec<u32>)> {
    // ── profile.cpf (root index) ───────────────────────────────────────
    let cpf_path =
        in_path("Configuration/profile.cpf").ok_or_else(|| anyhow!("profile.cpf missing"))?;
    let cpf = {
        let mut file =
            SbFile::open(&cpf_path.to_string_lossy()).map_err(|e| anyhow!("open cpf: {e}"))?;
        let mut mgr = ProfileManager::new();
        mgr.load_all_legacy_cpf(&mut file)
            .map_err(|e| anyhow!("parse cpf: {e}"))?;
        mgr
    };
    let character_exclamation_ids: Vec<u32> = cpf
        .characters
        .iter()
        .map(|profile| profile.exclamation_id)
        .collect();
    for (i, c) in cpf.civilians.iter().enumerate() {
        if c.civilian_type == CivilianType::Beggar {
            beggar_ids.insert(i as u32);
        }
    }
    Ok((cpf, character_exclamation_ids))
}

pub(super) type RhsRequirements = BTreeMap<String, BTreeSet<String>>;

/// Runtime party composition is not known during conversion. Build a
/// manifest index for every character profile so the mission boundary can
/// fetch only the selected team plus eligible reinforcement candidates.
/// Each entry also carries the projectile/pickup masters enabled by that
/// profile's actions; those objects can be created during a tick and cannot
/// perform asynchronous loading themselves.
pub(super) fn character_rhs_requirements(
    cpf: &ProfileManager,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
) -> Result<BTreeMap<u32, RhsRequirements>> {
    let mut character_rhs_requirements = BTreeMap::<u32, RhsRequirements>::new();
    for (index, profile) in cpf.characters.iter().enumerate() {
        let profile_index = u32::try_from(index).context("character profile index exceeds u32")?;
        let required = character_rhs_requirements.entry(profile_index).or_default();
        add_character_rhs_profiles_for_index(required, cpf, index, in_path, false)?;
        add_character_action_rhs_profiles(
            required,
            profile
                .actions
                .into_iter()
                .chain(profile.contextual_actions),
        );
    }
    Ok(character_rhs_requirements)
}

/// Loads the source sprite bank, optionally frequency-ranks its dictionaries
/// and records the bank header in `dd`. Returns the bank plus the rank remaps.
pub(super) fn load_sprite_bank(
    dd: &mut ShippingDatadir,
    data_in: &Path,
    opts: &ShippingOpts,
) -> Result<(FrameHolder, Option<Vec<Vec<u16>>>)> {
    // Load the source bank once. Each RHS gets one shared payload containing
    // its metadata and reachable bank slots; missions reference these files
    // instead of duplicating characters they have in common.
    let parent = data_in
        .parent()
        .ok_or_else(|| anyhow!("data dir has no parent"))?;
    let holder =
        FrameHolder::from_data_dir(&parent.to_string_lossy()).context("loading sprite bank")?;
    // Frequency-rank the dictionaries so the most used tile of each becomes
    // index 0, and remember the old→new maps to rewrite every VQ sprite's
    // indices below. A consistent permutation is invisible to the decoder.
    let dict_remaps = if opts.rank_dictionaries {
        Some(build_dictionary_rank_remaps(&holder)?)
    } else {
        None
    };
    let shipping_dictionaries = match &dict_remaps {
        Some(remaps) => holder
            .dictionaries()
            .iter()
            .zip(remaps)
            .map(|(dict, remap)| permute_dictionary(dict, remap))
            .collect(),
        None => holder.dictionaries().to_vec(),
    };
    dd.sprite_bank = Some(ShippingSpriteBank {
        signature: holder.signature(),
        dictionaries: shipping_dictionaries,
        sprite_count: holder.sprites().len() as u32,
        sprites: Vec::new(),
        vq_chunks: Vec::new(),
        rle_jxl_chunks: Vec::new(),
    });
    Ok((holder, dict_remaps))
}

/// Builds the inspectable dependency closure over missions, character
/// profiles and the saved-world compatibility set.
pub(super) fn plan_dependencies(
    mission_builds: &BTreeMap<String, ShippingMissionBuild>,
    character_rhs_requirements: &BTreeMap<u32, RhsRequirements>,
    saved_world_rhs_requirements: &RhsRequirements,
) -> DependencyPlan {
    let mut dependency_plan = DependencyPlan::default();
    for (mission, build) in mission_builds {
        dependency_plan.include(
            DependencyRoot::Mission(mission.clone()),
            &build.required_rhs_profiles,
        );
    }
    for (character, required) in character_rhs_requirements {
        dependency_plan.include(DependencyRoot::Character(*character), required);
    }
    for (mission, build) in mission_builds {
        let mut planned = dependency_plan::PlannedMission::default();
        planned.sources.insert(
            format!("Levels/{}.rhp", build.proto_filename),
            "mission proto level".into(),
        );
        planned
            .sources
            .insert(format!("Levels/{mission}.rhm"), "mission world".into());
        for map in &build.map_names {
            planned
                .sources
                .insert(map.clone(), "terrain map and minimap".into());
        }
        for music in &build.music_names {
            planned
                .sources
                .insert(format!("Musics/{music}"), "mission music".into());
        }
        for dialogue in &build.dialogue_samples {
            planned
                .sources
                .insert(dialogue.clone(), "localized mission dialogue".into());
        }
        for id in &build.sound_wave_ids {
            planned
                .sources
                .insert(format!("Sounds/snd_{id:03}"), "mission sound source".into());
        }
        for id in &build.required_exclamation_ids {
            planned.sources.insert(
                format!("exclamation:{id:08x}"),
                "actor voice profile".into(),
            );
        }
        dependency_plan.missions.insert(mission.clone(), planned);
    }
    dependency_plan.include(DependencyRoot::SavedWorld, saved_world_rhs_requirements);
    dependency_plan
}

pub(super) fn bounded_compression_pool() -> Result<rayon::ThreadPool> {
    // Max-level zstd and the VQ context-model encoder are deliberately
    // expensive and memory hungry. Bound the worker count; each completed
    // chunk is written in its worker so the result vectors retain only small
    // manifest metadata, not every compressed RHS.
    let compression_workers = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .min(4);
    rayon::ThreadPoolBuilder::new()
        .num_threads(compression_workers)
        .thread_name(|index| format!("shipping-zstd-{index}"))
        .build()
        .context("create bounded shipping compression pool")
}

/// Output directories of the shipping datadir, created up front.
pub(super) struct ShippingOutputDirs {
    pub(super) mission: PathBuf,
    pub(super) rhs: PathBuf,
    pub(super) terrain: PathBuf,
    pub(super) audio: PathBuf,
}

impl ShippingOutputDirs {
    pub(super) fn create(data_out: &Path) -> Result<Self> {
        let dirs = Self {
            mission: data_out.join("missions"),
            rhs: data_out.join("rhs"),
            terrain: data_out.join("terrain"),
            audio: data_out.join("audio"),
        };
        fs::create_dir_all(&dirs.mission)?;
        fs::create_dir_all(&dirs.rhs)?;
        fs::create_dir_all(&dirs.terrain)?;
        fs::create_dir_all(&dirs.audio)?;
        Ok(dirs)
    }
}

/// Compresses and writes each payload in `output_dir` on the bounded pool.
/// Returns payload key -> file name relative to `output_dir`. Every payload is
/// attempted before the first error is reported, as before the split.
fn encode_payloads(
    compression_pool: &rayon::ThreadPool,
    payloads: BTreeMap<String, ShippingMission>,
    output_dir: &Path,
    opts: &ShippingOpts,
) -> Result<Vec<(String, String)>> {
    let encoded = compression_pool.install(|| {
        payloads
            .into_par_iter()
            .map(|(rel, payload)| {
                let (filename, compressed) = prepare_shipping_payload(
                    output_dir,
                    &rel,
                    &payload,
                    opts.zstd_window_log,
                    opts.resume,
                )?;
                write_prepared_shipping_payload(output_dir, &filename, compressed)?;
                Ok((rel, filename))
            })
            .collect::<Vec<Result<(String, String)>>>()
    });
    encoded.into_iter().collect()
}

/// Encodes terrain/loading-art payloads; values are `terrain/<file>`.
pub(super) fn encode_level_assets(
    compression_pool: &rayon::ThreadPool,
    level_asset_payloads: BTreeMap<String, ShippingMission>,
    terrain_dir: &Path,
    opts: &ShippingOpts,
) -> Result<BTreeMap<String, String>> {
    Ok(
        encode_payloads(compression_pool, level_asset_payloads, terrain_dir, opts)?
            .into_iter()
            .map(|(rel, filename)| (rel, format!("terrain/{filename}")))
            .collect(),
    )
}

/// Encoded RHS chunk files plus the family-hub dependencies between them.
pub(super) struct RhsChunkFiles {
    /// RHS rel -> `rhs/<file>`.
    files: BTreeMap<String, String>,
    /// Variant RHS rel -> hub RHS rels it is coded against.
    base_dep: BTreeMap<String, Vec<String>>,
}

impl RhsChunkFiles {
    /// Encodes the RHS payloads (after the terrain payloads, preserving order).
    pub(super) fn encode(
        compression_pool: &rayon::ThreadPool,
        rhs_payloads: BTreeMap<String, ShippingMission>,
        rhs_base_dep: BTreeMap<String, Vec<String>>,
        rhs_dir: &Path,
        opts: &ShippingOpts,
    ) -> Result<Self> {
        let files = encode_payloads(compression_pool, rhs_payloads, rhs_dir, opts)?
            .into_iter()
            .map(|(rel, filename)| (rel, format!("rhs/{filename}")))
            .collect();
        Ok(Self {
            files,
            base_dep: rhs_base_dep,
        })
    }

    /// A dependency on a family-variant chunk implies its hub chunk(s): the
    /// runtime decodes the variant's VQ grids against the hubs' at install
    /// (star-2 chunks depend on both hubs).
    pub(super) fn chunk_files(&self, rel: &str) -> Result<Vec<String>> {
        let mut chunk_files = Vec::with_capacity(3);
        let file = self
            .files
            .get(rel)
            .ok_or_else(|| anyhow!("missing shipping RHS payload {rel}"))?;
        chunk_files.push(file.clone());
        for base_rel in self.base_dep.get(rel).into_iter().flatten() {
            let base_file = self.files.get(base_rel).ok_or_else(|| {
                anyhow!("missing shipping RHS family-hub payload {base_rel} (required by {rel})")
            })?;
            chunk_files.push(base_file.clone());
        }
        Ok(chunk_files)
    }

    fn grouping(&self, rel: &str) -> String {
        match self.base_dep.get(rel).map(Vec::len).unwrap_or(0) {
            0 => "standalone".to_owned(),
            count => format!("family variant with {count} shared hub(s)"),
        }
    }
}

/// Fills RHS destination payloads into the plan and the per-character and
/// saved-world RHS file lists of the datadir.
pub(super) fn assign_rhs_dependencies(
    dd: &mut ShippingDatadir,
    dependency_plan: &mut DependencyPlan,
    character_rhs_requirements: BTreeMap<u32, RhsRequirements>,
    saved_world_rhs_requirements: &RhsRequirements,
    rhs_chunks: &RhsChunkFiles,
) -> Result<()> {
    for (rel, planned) in &mut dependency_plan.rhs {
        planned.destination_payloads = rhs_chunks.chunk_files(rel)?;
        planned.grouping = Some(rhs_chunks.grouping(rel));
    }

    for (profile_index, requirements) in character_rhs_requirements {
        let mut files = Vec::with_capacity(requirements.len());
        for rel in requirements.keys() {
            files.extend(rhs_chunks.chunk_files(rel).with_context(|| {
                format!("character profile {profile_index} RHS dependency {rel}")
            })?);
        }
        files.sort();
        files.dedup();
        dd.character_rhs_files.insert(profile_index, files);
    }
    for rel in saved_world_rhs_requirements.keys() {
        dd.saved_world_rhs_files.extend(
            rhs_chunks
                .chunk_files(rel)
                .with_context(|| format!("saved-world compatibility RHS dependency {rel}"))?,
        );
    }
    dd.saved_world_rhs_files.sort();
    dd.saved_world_rhs_files.dedup();
    Ok(())
}

/// A compressed mission payload plus the dependency keys packaging needs.
pub(super) struct EncodedMission {
    mission_name: String,
    filename: String,
    compressed_len: usize,
    required_rhs_profiles: RhsRequirements,
    required_exclamation_ids: BTreeSet<u32>,
    music_names: BTreeSet<String>,
    dialogue_samples: BTreeSet<String>,
    sound_wave_ids: BTreeSet<u32>,
    level_asset_keys: BTreeSet<String>,
    forest_level: bool,
}

/// Compresses and writes every mission payload on the bounded pool. Results
/// are returned unpropagated so packaging reports errors in mission order.
pub(super) fn encode_missions(
    compression_pool: &rayon::ThreadPool,
    mission_builds: BTreeMap<String, ShippingMissionBuild>,
    mission_dir: &Path,
    opts: &ShippingOpts,
) -> Vec<Result<EncodedMission>> {
    compression_pool.install(|| {
        mission_builds
            .into_par_iter()
            .map(|(mission_name, build)| {
                let ShippingMissionBuild {
                    payload,
                    required_rhs_profiles,
                    required_exclamation_ids,
                    music_names,
                    dialogue_samples,
                    sound_wave_ids,
                    level_asset_keys,
                    forest_level,
                    ..
                } = build;
                let (filename, compressed) = prepare_shipping_payload(
                    mission_dir,
                    &mission_name,
                    &payload,
                    opts.zstd_window_log,
                    opts.resume,
                )?;
                let compressed_len =
                    write_prepared_shipping_payload(mission_dir, &filename, compressed)?;
                Ok(EncodedMission {
                    mission_name,
                    filename,
                    compressed_len,
                    required_rhs_profiles,
                    required_exclamation_ids,
                    music_names,
                    dialogue_samples,
                    sound_wave_ids,
                    level_asset_keys,
                    forest_level,
                })
            })
            .collect::<Vec<Result<_>>>()
    })
}

/// Shared inputs for packaging each encoded mission into the datadir.
pub(super) struct MissionPackaging<'a> {
    pub(super) level_asset_files: &'a BTreeMap<String, String>,
    pub(super) rhs_chunks: &'a RhsChunkFiles,
    pub(super) common_audio_file: Option<&'a String>,
    pub(super) exclamation_metadata_file: Option<&'a String>,
    pub(super) actor_voice_files: &'a BTreeMap<u32, String>,
    pub(super) audio_assets_dir: &'a Path,
    pub(super) audio_dir: &'a Path,
    pub(super) opts: &'a ShippingOpts,
    pub(super) in_path: &'a dyn Fn(&str) -> Option<PathBuf>,
}

impl MissionPackaging<'_> {
    /// Collects a mission's dependency files, writes its per-mission audio
    /// bundles and registers the mission in `dd`.
    pub(super) fn package(&self, dd: &mut ShippingDatadir, encoded: EncodedMission) -> Result<()> {
        let EncodedMission {
            mission_name,
            filename,
            compressed_len,
            required_rhs_profiles,
            required_exclamation_ids,
            music_names,
            dialogue_samples,
            sound_wave_ids,
            level_asset_keys,
            forest_level,
        } = encoded;
        let relative = format!("missions/{filename}");
        let mut files = vec![relative.clone()];
        for rel in level_asset_keys {
            let file = self.level_asset_files.get(&rel).ok_or_else(|| {
                anyhow!("shipping mission {mission_name} requires missing terrain payload {rel}")
            })?;
            files.push(file.clone());
        }
        for rel in required_rhs_profiles.keys() {
            files.extend(
                self.rhs_chunks
                    .chunk_files(rel)
                    .with_context(|| format!("shipping mission {mission_name} RHS dependency"))?,
            );
        }
        if let Some(file) = self.common_audio_file {
            files.push(file.clone());
        }
        if let Some(file) = self.exclamation_metadata_file {
            files.push(file.clone());
        }
        for exclamation_id in &required_exclamation_ids {
            if let Some(file) = self.actor_voice_files.get(exclamation_id) {
                files.push(file.clone());
            }
        }
        files.extend(self.write_dialogue_audio(dd, &mission_name, &dialogue_samples)?);
        files.extend(self.write_ambience_audio(dd, &mission_name, sound_wave_ids)?);
        files.extend(self.write_music_audio(dd, &mission_name, &music_names)?);
        tracing::info!(
            mission = mission_name,
            bytes = compressed_len,
            dependencies = files.len(),
            file = relative,
            "wrote shipping mission payload"
        );
        dd.mission_exclamation_ids.insert(
            mission_name.clone(),
            required_exclamation_ids.iter().copied().collect(),
        );
        files.sort();
        files.dedup();
        dd.missions.insert(
            mission_name,
            ShippingMissionRef {
                forest_level,
                files,
            },
        );
        Ok(())
    }

    fn write_dialogue_audio(
        &self,
        dd: &mut ShippingDatadir,
        mission_name: &str,
        dialogue_samples: &BTreeSet<String>,
    ) -> Result<Option<String>> {
        let mut dialogue_audio = ShippingMission::default();
        for sample_rel in dialogue_samples {
            let sample_path = (self.in_path)(sample_rel).ok_or_else(|| {
                anyhow!(
                    "shipping mission {mission_name} references missing dialogue sample {sample_rel}"
                )
            })?;
            insert_shipping_audio(
                &mut dialogue_audio,
                &mut dd.audio_assets,
                self.audio_assets_dir,
                &format!("dialogue-{}", shipping_file_stem(mission_name)),
                sample_rel,
                &sample_path,
                AudioKind::Voice,
                self.opts.audio_format,
            )?;
        }
        write_shipping_dependency(
            self.audio_dir,
            "mission-dialogue",
            &dialogue_audio,
            self.opts.zstd_window_log,
            self.opts.resume,
        )
    }

    fn write_ambience_audio(
        &self,
        dd: &mut ShippingDatadir,
        mission_name: &str,
        sound_wave_ids: BTreeSet<u32>,
    ) -> Result<Option<String>> {
        let mut source_audio = ShippingMission::default();
        for id in sound_wave_ids {
            let resolved = ["wav", "ogg"].into_iter().find_map(|extension| {
                let relative = format!("Sounds/snd_{id:03}.{extension}");
                (self.in_path)(&relative).map(|path| (relative, path))
            });
            let Some((relative, path)) = resolved else {
                tracing::warn!(
                    mission = mission_name,
                    id,
                    "mission sound source has no sample"
                );
                continue;
            };
            insert_shipping_audio(
                &mut source_audio,
                &mut dd.audio_assets,
                self.audio_assets_dir,
                &format!("ambience-{}", shipping_file_stem(mission_name)),
                &relative,
                &path,
                AudioKind::Effect,
                self.opts.audio_format,
            )?;
        }
        write_shipping_dependency(
            self.audio_dir,
            "mission-ambience",
            &source_audio,
            self.opts.zstd_window_log,
            self.opts.resume,
        )
    }

    fn write_music_audio(
        &self,
        dd: &mut ShippingDatadir,
        mission_name: &str,
        music_names: &BTreeSet<String>,
    ) -> Result<Option<String>> {
        let mut music_audio = ShippingMission::default();
        for name in music_names {
            // SoundManager requests `.wav`, but the Linux release ships Ogg
            // and the audio backend deliberately falls back between them.
            // Preserve whichever real file the source datadir provides.
            let (relative, path) = ["wav", "ogg"]
                .into_iter()
                .find_map(|extension| {
                    let relative = format!("Musics/{name}.{extension}");
                    (self.in_path)(&relative).map(|path| (relative, path))
                })
                .ok_or_else(|| {
                    anyhow!(
                        "shipping mission {mission_name} references missing music Musics/{name}.{{wav,ogg}}"
                    )
                })?;
            insert_shipping_audio(
                &mut music_audio,
                &mut dd.audio_assets,
                self.audio_assets_dir,
                "music",
                &relative,
                &path,
                AudioKind::Music,
                self.opts.audio_format,
            )?;
        }
        write_shipping_dependency(
            self.audio_dir,
            "mission-music",
            &music_audio,
            self.opts.zstd_window_log,
            self.opts.resume,
        )
    }
}
