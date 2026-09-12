//! Shipping conversion phases; the entry point preserves their publication order.
use super::*;

pub(super) fn load_boot_roots(
    dd: &mut ShippingDatadir,
    data_in: &Path,
    audio_assets_dir: &Path,
    locale_dirs: &[LocaleSource],
    opts: &ShippingOpts,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
) -> Result<()> {
    // ── Fixed boot roots ───────────────────────────────────────────────
    // Boot-time resource roots plus the expression/actor text
    // table and loading-screen bundle.
    for rel in [
        "Interface/DEFAULT.RES",
        "Interface/Start.sxt",
        "Text/actors.res",
        "Text/Level.res",
        "Sounds/Exclamations/actors.res",
    ] {
        if let Some(p) = in_path(rel) {
            // `.sxt` is an extension used by more than one legacy wire
            // format. Some releases store Start.sxt as an SRES text table,
            // while the authentic demo stores a standalone 1024x768
            // packed 16-bit loading image. Only attach actual SRES
            // containers; the standalone picture is validated/transcoded by
            // walk_and_bundle_small below and retained in `raw`.
            if rel.to_ascii_lowercase().ends_with(".sxt") && !sxt_is_sres(&p)? {
                continue;
            }
            let mut mgr = ResourceManager::legacy_tool();
            mgr.attach_resource_file(&p.to_string_lossy())?;
            if is_interface_path(rel)
                && let Some(q) = opts.interface_image_format.jxl_quality()
            {
                let encoded = mgr.encode_pictures_for_shipping(|pic| {
                    Ok(EncodedPicture::jxl_rgba565_keyed(
                        transcode_picture_to_jxl_rgba_keyed(pic, q)?,
                    ))
                })?;
                tracing::info!(
                    "interface res {rel}: encoded {encoded} pictures as JXL {}",
                    jxl_quality_label(q)
                );
            }
            mgr.disable_recovery_for_shipping();
            dd.res_files.insert(rel.into(), mgr);
        }
    }
    for source in locale_dirs {
        let locale = dd
            .locales
            .get_mut(source.iso)
            .expect("detected shipping locale was initialized");
        for rel in [
            "Interface/DEFAULT.RES",
            "Interface/Start.sxt",
            "Text/actors.res",
            "Text/Level.res",
            "Sounds/Exclamations/actors.res",
        ] {
            let Some(path) = resolve_data_file(&source.data_dir, rel) else {
                continue;
            };
            // See the matching default-locale loop above: a localized SXT
            // may be either an SRES archive or a standalone Sixteen image.
            if rel.to_ascii_lowercase().ends_with(".sxt") && !sxt_is_sres(&path)? {
                continue;
            }
            let mut mgr = ResourceManager::legacy_tool();
            mgr.attach_resource_file(&path.to_string_lossy())?;
            if is_interface_path(rel)
                && let Some(quality) = opts.interface_image_format.jxl_quality()
            {
                let encoded = mgr.encode_pictures_for_shipping(|picture| {
                    Ok(EncodedPicture::jxl_rgba565_keyed(
                        transcode_picture_to_jxl_rgba_keyed(picture, quality)?,
                    ))
                })?;
                tracing::info!(
                    locale = source.iso,
                    "interface res {rel}: encoded {encoded} pictures as JXL {}",
                    jxl_quality_label(quality)
                );
            }
            locale
                .res_files
                .insert(canonical_shipping_asset_key(rel), mgr);
        }
    }
    if let Some(p) = in_path("Interface/Loading.pak")
        && opts.interface_image_format != InterfaceImageFormat::Raw
    {
        let pictures = read_pak_pictures(&p)?;
        let encoded = encode_interface_pak_pictures(&pictures, opts.interface_image_format)?;
        dd.pak_files.insert("interface/loading.pak".into(), encoded);
    }
    // Menu sounds are part of the data artifact, not the wasm executable.
    // Keep them in the boot manifest because they are needed before any
    // mission dependency is selected.
    let mut boot_audio = ShippingMission::default();
    let mut menu_roots = vec![data_in.join("Sounds/Menu")];
    menu_roots.extend(
        locale_dirs
            .iter()
            .map(|locale| locale.data_dir.join("Sounds/Menu")),
    );
    for root in menu_roots {
        if !optional_directory(&root)? {
            continue;
        }
        let mut files = Vec::new();
        collect_files_recursive(&root, &mut files)?;
        files.sort();
        for path in files {
            let filename = path
                .strip_prefix(&root)
                .expect("menu audio must remain below its collection root");
            let relative = Path::new("Sounds/Menu").join(filename);
            insert_shipping_audio(
                &mut boot_audio,
                &mut dd.audio_assets,
                &audio_assets_dir,
                "menu",
                &relative.to_string_lossy(),
                &path,
                AudioKind::Effect,
                opts.audio_format,
            )?;
        }
    }
    if let Some((relative, path)) = ["wav", "ogg"].into_iter().find_map(|extension| {
        let relative = format!("Musics/Menu.{extension}");
        in_path(&relative).map(|path| (relative, path))
    }) {
        insert_shipping_audio(
            &mut boot_audio,
            &mut dd.audio_assets,
            &audio_assets_dir,
            "menu",
            &relative,
            &path,
            AudioKind::Music,
            opts.audio_format,
        )?;
    } else {
        bail!("required menu music Musics/Menu.{{wav,ogg}} is missing");
    }
    dd.raw.extend(boot_audio.payload.raw);
    dd.audio_durations_ms
        .extend(boot_audio.payload.audio_durations_ms);

    if opts.interface_image_format != InterfaceImageFormat::Raw {
        for source in locale_dirs {
            let Some(path) = resolve_data_file(&source.data_dir, "Interface/Loading.pak") else {
                continue;
            };
            let pictures = read_pak_pictures(&path)?;
            let encoded = encode_interface_pak_pictures(&pictures, opts.interface_image_format)?;
            dd.locales
                .get_mut(source.iso)
                .expect("detected shipping locale was initialized")
                .pak_files
                .insert("interface/loading.pak".into(), encoded);
        }
    }
    Ok(())
}

pub(super) fn write_common_audio(
    dd: &mut ShippingDatadir,
    mission_builds: &BTreeMap<String, ShippingMissionBuild>,
    data_in: &Path,
    audio_assets_dir: &Path,
    audio_dir: &Path,
    opts: &ShippingOpts,
) -> Result<Option<String>> {
    // Source-format audio remains in dependency payloads for native builds.
    // Opus browser audio is cataloged as standalone content-addressed files;
    // these payloads then retain only small blocking metadata such as FXG and
    // exclamation DAT files.
    let mut common_audio = ShippingMission::default();
    // Dialogue and `snd_NNN` source waves receive exact per-mission metadata
    // below. Keeping either in this shared payload would make active warmup
    // falsely treat every campaign mission's speech/ambience as required.
    let mission_dialogue_keys: BTreeSet<String> = mission_builds
        .values()
        .flat_map(|build| build.dialogue_samples.iter())
        .map(|path| robin_util::asset_fs::bundle_key(Path::new(path)))
        .collect();
    let sounds_root = data_in.join("Sounds");
    if optional_directory(&sounds_root)? {
        let mut files = Vec::new();
        collect_files_recursive(&sounds_root, &mut files)?;
        files.sort();
        for path in files {
            let relative = path
                .strip_prefix(&sounds_root)
                .expect("collected sound must remain below Sounds")
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase();
            if !is_common_audio_member(&relative, &mission_dialogue_keys) {
                continue;
            }
            insert_shipping_audio(
                &mut common_audio,
                &mut dd.audio_assets,
                &audio_assets_dir,
                "common",
                &format!("Sounds/{relative}"),
                &path,
                AudioKind::Effect,
                opts.audio_format,
            )?;
        }
    }
    write_shipping_dependency(
        &audio_dir,
        "common-sfx",
        &common_audio,
        opts.zstd_window_log,
        opts.resume,
    )
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ExclamationDependencies {
    pub(super) metadata_file: Option<String>,
    pub(super) actor_voice_files: BTreeMap<u32, String>,
}

pub(super) fn write_exclamation_audio(
    dd: &mut ShippingDatadir,
    mission_builds: &BTreeMap<String, ShippingMissionBuild>,
    data_in: &Path,
    audio_assets_dir: &Path,
    audio_dir: &Path,
    opts: &ShippingOpts,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
    character_exclamation_ids: Vec<u32>,
) -> Result<ExclamationDependencies> {
    // Mission-authored exclamation profiles must resolve completely; ids
    // that only appear in the all-profiles character manifest index may be
    // absent from a trimmed (demo) datadir and are then dropped from the
    // manifest instead of failing the conversion.
    let mission_exclamation_ids: BTreeSet<u32> = mission_builds
        .values()
        .flat_map(|build| build.required_exclamation_ids.iter().copied())
        .collect();
    let mut dropped_exclamation_ids = BTreeSet::<u32>::new();
    let mut required_exclamation_ids: BTreeSet<u32> = mission_builds
        .values()
        .flat_map(|build| build.required_exclamation_ids.iter().copied())
        .chain(
            character_exclamation_ids
                .iter()
                .copied()
                .filter(|id| *id != 0),
        )
        .collect();
    let mut exclamation_metadata = ShippingMission::default();
    let exclamation_root = data_in.join("Sounds/Exclamations");
    if optional_directory(&exclamation_root)? {
        let mut files = Vec::new();
        collect_files_recursive(&exclamation_root, &mut files)?;
        files.sort();
        for path in files {
            // actors.res is already represented authoritatively in
            // `ShippingDatadir::res_files`; voice WAVs are actor chunks below.
            let extension = path
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or_default();
            if extension.eq_ignore_ascii_case("dat") {
                let relative = path
                    .strip_prefix(&data_in)
                    .expect("base exclamation metadata must remain below Data")
                    .to_string_lossy();
                insert_shipping_raw(&mut exclamation_metadata, &relative, &path)?;
            }
        }
    }
    // A localized install may put actor tables in its locale overlay rather
    // than the base Exclamations directory. Ensure every referenced table is
    // mounted under the logical path used by the runtime.
    for exclamation_id in &required_exclamation_ids {
        let dat_rel = format!(
            "Sounds/Exclamations/{}",
            exclamation_dat_filename(*exclamation_id)
        );
        if let Some(path) = in_path(&dat_rel) {
            insert_shipping_raw(&mut exclamation_metadata, &dat_rel, &path)?;
        }
    }
    let actors_res_path = in_path("Sounds/Exclamations/actors.res")
        .ok_or_else(|| anyhow!("Sounds/Exclamations/actors.res missing"))?;
    let mut actors_res = ResourceManager::legacy_tool();
    actors_res.attach_resource_file(&actors_res_path.to_string_lossy())?;
    let mut actor_samples = std::collections::BTreeMap::<u32, Vec<(String, PathBuf)>>::new();
    let mut sample_profile_counts = std::collections::BTreeMap::<String, usize>::new();
    'ids: for &exclamation_id in &required_exclamation_ids {
        let strict = mission_exclamation_ids.contains(&exclamation_id);
        let dat_filename = exclamation_dat_filename(exclamation_id);
        let dat_rel = format!("Sounds/Exclamations/{dat_filename}");
        let dat_path = match in_path(&dat_rel) {
            Some(path) => path,
            None if strict => bail!(
                "required exclamation profile {exclamation_id:#010x} is missing metadata {dat_rel}"
            ),
            None => {
                tracing::warn!(
                    "exclamation profile {exclamation_id:#010x} has no metadata {dat_rel} in this datadir; omitting from manifest index"
                );
                dropped_exclamation_ids.insert(exclamation_id);
                continue 'ids;
            }
        };
        let dat = fs::read(&dat_path)
            .with_context(|| format!("read exclamation metadata {}", dat_path.display()))?;
        let prefix_id = exclamation_id & 0xffff_0000;
        let (table_id, exclamations) =
            robin_engine::sound_cache::parse_exclamation_file(&dat, prefix_id)
                .map_err(|error| anyhow!("parse exclamation metadata {dat_filename}: {error}"))?;
        let variant_indices: BTreeSet<u32> = exclamations
            .into_iter()
            .flat_map(|(_, variants)| variants)
            .collect();
        let mut samples = Vec::with_capacity(variant_indices.len());
        for variant_index in variant_indices {
            let sample = actors_res
                .get_sample(table_id as i32, variant_index as usize)
                .with_context(|| {
                    format!("resolve exclamation {exclamation_id:#010x} variant {variant_index}")
                })?
                .replace('\\', "/");
            let sample_rel = format!("Sounds/Exclamations/{sample}");
            let sample_path = match in_path(&sample_rel) {
                Some(path) => path,
                None if strict => bail!(
                    "required exclamation profile {exclamation_id:#010x} variant {variant_index} references missing sample {sample_rel}"
                ),
                None => {
                    tracing::warn!(
                        "exclamation profile {exclamation_id:#010x} references missing sample {sample_rel} in this datadir; omitting from manifest index"
                    );
                    dropped_exclamation_ids.insert(exclamation_id);
                    continue 'ids;
                }
            };
            samples.push((sample_rel.clone(), sample_path));
        }
        for (sample_rel, _) in &samples {
            *sample_profile_counts.entry(sample_rel.clone()).or_default() += 1;
        }
        actor_samples.insert(exclamation_id, samples);
    }
    required_exclamation_ids.retain(|id| !dropped_exclamation_ids.contains(id));

    // A handful of generic samples (notably x_empty.wav) are referenced by
    // multiple actor tables. Store those once in the shared exclamation
    // payload rather than downloading duplicate bytes or mounting duplicate
    // VFS keys from several actor chunks.
    for (sample_rel, profile_count) in &sample_profile_counts {
        if *profile_count > 1 {
            let sample_path = in_path(sample_rel).ok_or_else(|| {
                anyhow!("shared exclamation sample disappeared during conversion: {sample_rel}")
            })?;
            insert_shipping_audio(
                &mut exclamation_metadata,
                &mut dd.audio_assets,
                &audio_assets_dir,
                "voice-shared",
                sample_rel,
                &sample_path,
                AudioKind::Voice,
                opts.audio_format,
            )?;
        }
    }
    let exclamation_metadata_file = write_shipping_dependency(
        &audio_dir,
        "exclamation-metadata",
        &exclamation_metadata,
        opts.zstd_window_log,
        opts.resume,
    )?;

    let mut actor_voice_files = std::collections::BTreeMap::<u32, String>::new();
    for exclamation_id in required_exclamation_ids {
        let mut actor_audio = ShippingMission::default();
        let samples = actor_samples.remove(&exclamation_id).ok_or_else(|| {
            anyhow!("missing resolved sample set for exclamation profile {exclamation_id:#010x}")
        })?;
        for (sample_rel, sample_path) in samples {
            if sample_profile_counts.get(&sample_rel).copied().unwrap_or(0) == 1 {
                insert_shipping_audio(
                    &mut actor_audio,
                    &mut dd.audio_assets,
                    &audio_assets_dir,
                    &format!("voice-{exclamation_id:08x}"),
                    &sample_rel,
                    &sample_path,
                    AudioKind::Voice,
                    opts.audio_format,
                )?;
            }
        }
        if let Some(relative) = write_shipping_dependency(
            &audio_dir,
            &format!("voice-{exclamation_id:08x}"),
            &actor_audio,
            opts.zstd_window_log,
            opts.resume,
        )? {
            actor_voice_files.insert(exclamation_id, relative);
        }
    }

    for (profile_index, exclamation_id) in character_exclamation_ids.into_iter().enumerate() {
        let profile_index =
            u32::try_from(profile_index).context("character profile index exceeds u32")?;
        let files = actor_voice_files
            .get(&exclamation_id)
            .cloned()
            .into_iter()
            .collect();
        dd.character_audio_files.insert(profile_index, files);
        if exclamation_id != 0 && !dropped_exclamation_ids.contains(&exclamation_id) {
            dd.character_exclamation_ids
                .insert(profile_index, exclamation_id);
        }
    }
    Ok(ExclamationDependencies {
        metadata_file: exclamation_metadata_file,
        actor_voice_files,
    })
}

pub(super) fn build_level_assets(
    mission_builds: &mut std::collections::BTreeMap<String, ShippingMissionBuild>,
    opts: &ShippingOpts,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
) -> Result<std::collections::BTreeMap<String, ShippingMission>> {
    // Resolve only the terrain and loading art the original runtime can open
    // for this mission. Keep each logical source asset in its own shared
    // payload so missions that reuse a city also reuse one HTTP-cache key.
    let mut encoded_level_assets = std::collections::BTreeMap::<String, Vec<u8>>::new();
    let mut level_asset_payloads = std::collections::BTreeMap::<String, ShippingMission>::new();
    for build in mission_builds.values_mut() {
        for map in &build.map_names {
            // A mission that opens a map always opens its minimap too (the
            // runtime draws both), so both land in ONE shared payload keyed
            // by the `.map` rel: one HTTP fetch / cache key per city and
            // ambiance instead of two. Runtime lookups are by the original
            // asset path inside the payload, so merging is invisible there.
            let map_rel = level_asset_rel_existing(build.ambiance, map, ".map", &in_path)?;
            for ext in [".map", ".min"] {
                let rel = level_asset_rel_existing(build.ambiance, map, ext, &in_path)?;
                let path = in_path(&rel)
                    .ok_or_else(|| anyhow!("resolved shipping level asset disappeared: {rel}"))?;
                let bytes = if let Some(bytes) = encoded_level_assets.get(&rel) {
                    bytes.clone()
                } else {
                    // Minimaps follow the map format: the runtime picture
                    // loader sniffs the JXL signature, so `.min` decodes
                    // through the same path as `.map` with no extra code.
                    let bytes = match opts.map_format.jxl_quality() {
                        Some(quality) => transcode_sixteen_to_jxl(&path, quality)?,
                        None => transcode_sixteen_drop_bzip(&path)?,
                    };
                    encoded_level_assets.insert(rel.clone(), bytes.clone());
                    bytes
                };
                level_asset_payloads
                    .entry(map_rel.clone())
                    .or_default()
                    .raw
                    .insert(rel.to_ascii_lowercase(), bytes);
            }
            build.level_asset_keys.insert(map_rel);
        }
        let rel = format!("Levels/{:02}/{}.pak", build.ambiance, build.proto_filename);
        if let Some(path) = in_path(&rel) {
            let bytes = if let Some(bytes) = encoded_level_assets.get(&rel) {
                bytes.clone()
            } else {
                let bytes = transcode_pak_drop_bzip(&path)?;
                encoded_level_assets.insert(rel.clone(), bytes.clone());
                bytes
            };
            level_asset_payloads
                .entry(rel.clone())
                .or_default()
                .raw
                .insert(rel.to_ascii_lowercase(), bytes);
            build.level_asset_keys.insert(rel);
        }
    }

    Ok(level_asset_payloads)
}

pub(super) fn finish_profiles_and_boot_files(
    dd: &mut ShippingDatadir,
    data_in: &Path,
    locale_dirs: &[LocaleSource],
    mut cpf: ProfileManager,
    opts: &ShippingOpts,
) -> Result<()> {
    // Bake the `import_beam_mes` post-processing into the shipping
    // profile table.  Without this, runtime loaders that consume
    // `dd.profiles` see empty `required_actions` / zero
    // `number_of_beam_mes` — breaking briefing-UI glyphs and
    // auto-gang-selection (see
    // `crates/robin_rs/src/main_entry.rs::load_profiles` for the
    // non-shipping equivalent).
    if let Some(level_dir) = resolve_case_insensitive(&data_in.join("Levels"))
        .filter(|path| path.is_dir())
        .map(|path| path.to_string_lossy().into_owned())
    {
        cpf.import_beam_mes(&level_dir);
    } else {
        tracing::warn!(
            "convert_shipping: no Levels/ directory found; shipping profile will lack beam-me data"
        );
    }
    dd.profiles = Some(cpf);
    for source in locale_dirs {
        let Some(path) = resolve_data_file(&source.data_dir, "Configuration/profile.cpf") else {
            continue;
        };
        let mut file = SbFile::open(&path.to_string_lossy())
            .map_err(|error| anyhow!("open locale {} cpf: {error}", source.iso))?;
        let mut profiles = ProfileManager::new();
        profiles
            .load_all_legacy_cpf(&mut file)
            .map_err(|error| anyhow!("parse locale {} cpf: {error}", source.iso))?;
        if let Some(level_dir) = resolve_case_insensitive(&data_in.join("Levels"))
            .filter(|path| path.is_dir())
            .map(|path| path.to_string_lossy().into_owned())
        {
            profiles.import_beam_mes(&level_dir);
        }
        dd.locales
            .get_mut(source.iso)
            .expect("detected shipping locale was initialized")
            .profiles = Some(profiles);
    }

    // Bundle the small-file types the engine opens by exact path — these
    // are the items that would otherwise fan out to hundreds of tiny HTTP
    // requests on wasm and a bunch of syscalls on native.  We deliberately
    // *don't* bundle large files (audio, terrain bitmaps already handled
    // above, cinematics) so the shipping blob stays compact.
    //
    // Keyed by the path the engine passes to `SbFile::open` minus the
    // `Data/` prefix, which matches `asset_fs::bundle_key`.
    const BOOT_FILE_EXTS: &[&str] = &[
        // Fonts
        "bfn", "tfn", "fnt", // Menu / cursor / interface configuration
        "cfg", "ini", // Resource bundles (text tables, cursors, loading screens)
        "res", "sxt", "pak", "red", // Small shared resource bundles
        "cpf",
    ];
    walk_and_bundle_small(
        dd,
        &data_in,
        &data_in,
        BOOT_FILE_EXTS,
        opts.interface_image_format,
    )?;
    for alt in locale_dirs {
        walk_and_bundle_small(
            dd,
            &alt.data_dir,
            &alt.data_dir,
            BOOT_FILE_EXTS,
            opts.interface_image_format,
        )?;
    }
    // The v5 locale dimension is complete rather than boot-file-only: voice,
    // dialogue, and cinematics are language assets too. Keeping each overlay
    // self-contained lets the same in-memory VFS bundle work on desktop,
    // browser, and Android. The top-level compatibility maps above retain the
    // historical compact/default-language view for old consumers.
    for source in locale_dirs {
        let locale = dd
            .locales
            .get_mut(source.iso)
            .expect("detected shipping locale was initialized");
        walk_and_bundle_locale(
            locale,
            &source.data_dir,
            &source.data_dir,
            opts.interface_image_format,
        )?;
    }
    Ok(())
}
