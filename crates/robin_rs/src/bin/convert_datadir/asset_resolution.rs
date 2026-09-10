//! Logical asset resolution and original-data dependency rules.
use super::*;

pub(super) fn add_required_rhs_rel(
    required: &mut std::collections::BTreeMap<String, BTreeSet<String>>,
    rel: impl Into<String>,
    profile: &str,
) {
    if profile.is_empty() {
        return;
    }
    required
        .entry(rel.into())
        .or_default()
        .insert(profile.into());
}

pub(super) fn add_required_animation_rhs_profile(
    required: &mut std::collections::BTreeMap<String, BTreeSet<String>>,
    ambiance: u32,
    sprite: &robin_engine::level_data::RawSpriteRef,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
) {
    if sprite.frame_profile_name.is_empty() || sprite.profile_name.is_empty() {
        return;
    }
    let rel = animation_rhs_rel_existing(ambiance, &sprite.frame_profile_name, in_path);
    if in_path(&rel).is_some() {
        add_required_rhs_rel(required, rel, &sprite.profile_name);
    } else {
        // TODO: Determine why a few stock level records name an RHS that is
        // absent from every original animation directory.
        tracing::warn!("source animation RHS is absent: {rel}");
    }
}

pub(super) fn animation_rhs_rel_existing(
    ambiance: u32,
    file: &str,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
) -> String {
    // Mission headers store the original AMBIANCE_* bit values, not a
    // zero-based enum. Keep this identical to engine::Ambiance::from_raw()
    // followed by to_sprite_ambiance(): attack/custom ambiances use Day RHS.
    let dir = match ambiance {
        2 => "Fog",
        4 => "Night",
        _ => "Day",
    };
    let primary = format!("Animations/{dir}/{file}.rhs");
    if in_path(&primary).is_some() {
        return primary;
    }
    if dir != "Day" {
        let day = format!("Animations/Day/{file}.rhs");
        if in_path(&day).is_some() {
            return day;
        }
    }
    let base = format!("Animations/{file}.rhs");
    if in_path(&base).is_some() {
        return base;
    }
    primary
}

pub(super) fn level_ambiance_directory(ambiance: u32) -> Result<&'static str> {
    match ambiance {
        1 => Ok("Day"),
        2 => Ok("Fog"),
        4 => Ok("Night"),
        8 => Ok("Attack"),
        16 => Ok("Custom1"),
        32 => Ok("Custom2"),
        64 => Ok("Custom3"),
        128 => Ok("Custom4"),
        _ => bail!("unknown mission ambiance bit value {ambiance}"),
    }
}

/// Resolve a map or minimap exactly like the original engine: selected
/// ambiance first, then Day, then the Levels root. Map and minimap are
/// resolved independently because installs may place their fallbacks at
/// different levels.
pub(super) fn level_asset_rel_existing(
    ambiance: u32,
    map: &str,
    extension: &str,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
) -> Result<String> {
    let directory = level_ambiance_directory(ambiance)?;
    let mut candidates = vec![format!("Levels/{directory}/{map}{extension}")];
    if directory != "Day" {
        candidates.push(format!("Levels/Day/{map}{extension}"));
    }
    candidates.push(format!("Levels/{map}{extension}"));
    candidates
        .into_iter()
        .find(|candidate| in_path(candidate).is_some())
        .ok_or_else(|| {
            anyhow!(
                "required level asset {map}{extension} is absent from {directory}, Day, and Levels root"
            )
        })
}

/// Cap on sampled tiles per family-base proxy measurement: enough for a
/// stable entropy estimate, small enough to keep conversion fast.
const FAMILY_PROXY_TILE_CAP: u64 = 1_500_000;

/// Conditional entropy in *total bits over all tiles* from 24-bit joint
/// (ctx<<12|sym) counts, scaled from the sampled tile count to `full_tiles`.
pub(super) fn family_proxy_bits(
    joint: &std::collections::HashMap<u32, u32>,
    ctx_totals: &std::collections::HashMap<u16, u32>,
    sampled: u64,
    full_tiles: u64,
) -> f64 {
    if sampled == 0 {
        return 0.0;
    }
    let mut bits = 0.0f64;
    for (&k, &n) in joint {
        let ctx_total = ctx_totals[&((k >> 12) as u16)] as f64;
        bits -= n as f64 * (n as f64 / ctx_total).log2();
    }
    bits / sampled as f64 * full_tiles as f64
}

/// Resolve a family hub's chunk rel (reusing an existing prep's spelling when
/// one matches case-insensitively) and ensure its full-profile script order is
/// available — either from its prep or loaded into `loaded_orders`.
pub(super) fn resolve_family_hub_rel(
    rhs_preps: &std::collections::BTreeMap<String, RhsChunkPrep>,
    loaded_orders: &mut std::collections::BTreeMap<String, Vec<u32>>,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
    hub_name: &str,
    variant_rel: &str,
) -> Result<String> {
    let disk_rel = format!("Characters/{hub_name}.rhs");
    let hub_rel = rhs_preps
        .keys()
        .find(|rel| rel.eq_ignore_ascii_case(&disk_rel))
        .cloned()
        .unwrap_or(disk_rel);
    if !rhs_preps.contains_key(&hub_rel) && !loaded_orders.contains_key(&hub_rel) {
        let path = in_path(&hub_rel).ok_or_else(|| {
            anyhow!(
                "family hub RHS {hub_rel} (for variant {variant_rel}) is missing from the datadir"
            )
        })?;
        let (_, profiles) =
            sprite_script::SpriteScriptor::load_all_profiles_legacy(&path.to_string_lossy())
                .map_err(|error| anyhow!("rhs {hub_rel}: {error}"))?;
        let mut order = Vec::new();
        for (_, info) in &profiles {
            for script in info.scripts.iter() {
                order.extend_from_slice(&script.frame_ids);
            }
        }
        loaded_orders.insert(hub_rel.clone(), order);
    }
    Ok(hub_rel)
}

#[cfg(test)]
mod hub_resolution_tests {
    use super::*;

    #[test]
    fn prepared_hubs_preserve_first_sorted_case_match_without_loading() {
        let preps = ["Characters/HERO.rhs", "Characters/Hero.rhs"]
            .map(|name| {
                (
                    name.to_owned(),
                    RhsChunkPrep {
                        rhs_data: None,
                        matched_profiles: 0,
                        script_order: vec![1, 2],
                        used_sprite_ids: BTreeSet::new(),
                        base_rel: None,
                        base_ids: Default::default(),
                        base2_rel: None,
                        base2_ids: Default::default(),
                    },
                )
            })
            .into();
        let mut loaded = Default::default();
        let resolved = resolve_family_hub_rel(
            &preps,
            &mut loaded,
            &|_| panic!("prepared hub must not be loaded again"),
            "hero",
            "Characters/variant.rhs",
        )
        .unwrap();
        assert_eq!(resolved, "Characters/HERO.rhs");
        assert!(loaded.is_empty());
    }

    #[test]
    fn cached_hubs_are_reused_and_missing_hubs_keep_context() {
        let preps = Default::default();
        let mut loaded =
            std::collections::BTreeMap::from([("Characters/Hero.rhs".to_owned(), vec![1, 2])]);
        assert_eq!(
            resolve_family_hub_rel(
                &preps,
                &mut loaded,
                &|_| panic!("cached hub must not be loaded again"),
                "Hero",
                "Characters/variant.rhs",
            )
            .unwrap(),
            "Characters/Hero.rhs"
        );
        let error = resolve_family_hub_rel(
            &preps,
            &mut loaded,
            &|_| None,
            "Missing",
            "Characters/variant.rhs",
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("Characters/Missing.rhs"));
        assert!(error.contains("Characters/variant.rhs"));
        assert_eq!(loaded.len(), 1);
    }
}

/// Positional variant->hub frame pairing over two script frame-id orders:
/// zip, dedup, and drop variant frames that pair with conflicting hub frames
/// (those fall back to weaker contexts per sprite).
pub(super) fn positional_pair_map(
    variant_order: &[u32],
    hub_order: &[u32],
) -> std::collections::BTreeMap<u32, u32> {
    let mut pairs: Vec<(u32, u32)> = variant_order
        .iter()
        .copied()
        .zip(hub_order.iter().copied())
        .collect();
    pairs.sort_unstable();
    pairs.dedup();
    let mut pair_map = std::collections::BTreeMap::<u32, u32>::new();
    let mut conflicted = BTreeSet::<u32>::new();
    for (vid, hid) in pairs {
        match pair_map.get(&vid) {
            Some(&existing) if existing != hid => {
                conflicted.insert(vid);
            }
            Some(_) => {}
            None => {
                pair_map.insert(vid, hid);
            }
        }
    }
    for vid in &conflicted {
        pair_map.remove(vid);
    }
    pair_map
}

/// Sampled H(tile | above) * tile-count for one family member coded
/// standalone. `None` when an index doesn't fit the 12-bit proxy key.
pub(super) fn family_base_standalone_proxy(
    holder: &FrameHolder,
    script_order: &[u32],
) -> Option<f64> {
    let mut ids: Vec<u32> = script_order.to_vec();
    ids.sort_unstable();
    ids.dedup();
    let mut joint = std::collections::HashMap::<u32, u32>::new();
    let mut ctx_totals = std::collections::HashMap::<u16, u32>::new();
    let mut sampled = 0u64;
    let mut full_tiles = 0u64;
    for &id in &ids {
        let sprite = holder.sprites().get(id as usize)?;
        if sprite.dictionary_index == UNMAPPED_DICT {
            continue;
        }
        let Some(packed) = holder.packed_data(id) else {
            continue;
        };
        full_tiles += packed.len() as u64;
        if sampled >= FAMILY_PROXY_TILE_CAP {
            continue;
        }
        let cols = (sprite.width / 4) as usize;
        for (i, &x) in packed.iter().enumerate().skip(cols) {
            if x >= 4096 || packed[i - cols] >= 4096 {
                return None;
            }
            *joint
                .entry(((packed[i - cols] as u32) << 12) | x as u32)
                .or_default() += 1;
            *ctx_totals.entry(packed[i - cols]).or_default() += 1;
            sampled += 1;
        }
    }
    Some(family_proxy_bits(&joint, &ctx_totals, sampled, full_tiles))
}

/// Sampled H(member tile | candidate-base tile) * tile-count for coding
/// `member` against `candidate` (positional script pairing, mismatches
/// skipped like the real chunk builder).
pub(super) fn family_base_pair_proxy(
    holder: &FrameHolder,
    candidate_order: &[u32],
    member_order: &[u32],
) -> Option<f64> {
    let mut pairs: Vec<(u32, u32)> = member_order
        .iter()
        .copied()
        .zip(candidate_order.iter().copied())
        .collect();
    pairs.sort_unstable();
    pairs.dedup();
    let mut joint = std::collections::HashMap::<u32, u32>::new();
    let mut ctx_totals = std::collections::HashMap::<u16, u32>::new();
    let mut sampled = 0u64;
    let mut full_tiles = 0u64;
    for &(mid, cid) in &pairs {
        let (ms, cs) = (
            holder.sprites().get(mid as usize)?,
            holder.sprites().get(cid as usize)?,
        );
        if ms.dictionary_index == UNMAPPED_DICT || cs.dictionary_index == UNMAPPED_DICT {
            continue;
        }
        let (Some(mp), Some(cp)) = (holder.packed_data(mid), holder.packed_data(cid)) else {
            continue;
        };
        if (ms.width, ms.height) != (cs.width, cs.height) || mp.len() != cp.len() {
            continue;
        }
        full_tiles += mp.len() as u64;
        if sampled >= FAMILY_PROXY_TILE_CAP {
            continue;
        }
        for (&x, &b) in mp.iter().zip(cp.iter()) {
            if x >= 4096 || b >= 4096 {
                return None;
            }
            *joint.entry(((b as u32) << 12) | x as u32).or_default() += 1;
            *ctx_totals.entry(b).or_default() += 1;
            sampled += 1;
        }
    }
    Some(family_proxy_bits(&joint, &ctx_totals, sampled, full_tiles))
}

pub(super) fn add_required_character_rhs_profiles_for_index(
    required: &mut std::collections::BTreeMap<String, BTreeSet<String>>,
    profiles: &ProfileManager,
    index: usize,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
) {
    add_character_rhs_profiles_for_index(required, profiles, index, in_path, true);
}

/// `required_on_disk = true` insists the RHS exists (mission-authored
/// characters must ship); `false` skips profiles whose RHS is absent from
/// this datadir entirely — the boot manifest indexes every CPF profile, but
/// a demo datadir only carries the files its missions can actually use.
pub(super) fn add_character_rhs_profiles_for_index(
    required: &mut std::collections::BTreeMap<String, BTreeSet<String>>,
    profiles: &ProfileManager,
    index: usize,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
    required_on_disk: bool,
) {
    let Some(profile) = profiles.characters.get(index) else {
        return;
    };
    // Character profile indices identify physical RHS files. Do not group by
    // localized profile name: RobinHood and RobinTown can share one logical
    // name in legacy profile tables but original-game PC initialization selects
    // exactly one physical variant from the level's forest flag.
    let rel = format!("Characters/{}.rhs", profile.filename);
    if required_on_disk || in_path(&rel).is_some() {
        add_required_rhs_rel(required, rel, &profile.profile_name);
    } else {
        tracing::warn!(
            "character profile '{}' ({}) has no RHS in this datadir; omitting from manifest index",
            profile.profile_name,
            profile.filename,
        );
    }
}

pub(super) fn normalize_robin_profile_index(
    profiles: &ProfileManager,
    index: usize,
    forest_level: bool,
) -> Result<usize> {
    let profile = profiles
        .characters
        .get(index)
        .ok_or_else(|| anyhow!("character profile index {index} does not exist"))?;
    if !matches!(profile.filename.as_str(), "RobinHood" | "RobinTown") {
        return Ok(index);
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
        .ok_or_else(|| {
            anyhow!(
                "required {wanted} profile is absent while normalizing Robin for a {} mission",
                if forest_level { "forest" } else { "town" }
            )
        })
}

pub(super) fn add_required_pc_profiles_for_pcs(
    required: &mut std::collections::BTreeMap<String, BTreeSet<String>>,
    profiles: &ProfileManager,
    pcs: &str,
    forest_level: bool,
    in_path: &impl Fn(&str) -> Option<PathBuf>,
) {
    for profile_name in pcs.chars().filter_map(pc_code_profile_name) {
        let profile = profiles.characters.iter().find(|profile| {
            if profile.filename == "RobinHood" || profile.filename == "RobinTown" {
                profile.filename
                    == if forest_level {
                        "RobinHood"
                    } else {
                        "RobinTown"
                    }
            } else {
                profile.profile_name == profile_name
            }
        });
        if let Some(profile) = profile {
            let rel = format!("Characters/{}.rhs", profile.filename);
            if in_path(&rel).is_some() {
                add_required_rhs_rel(required, rel, &profile.profile_name);
            } else {
                tracing::warn!("demo PC profile '{}' has no shipped RHS", profile_name);
            }
        } else {
            tracing::warn!("demo PC profile '{}' has no shipped RHS", profile_name);
        }
    }
}

pub(super) fn pc_code_profile_name(code: char) -> Option<&'static str> {
    match code.to_ascii_uppercase() {
        'R' => Some("Robin des bois"),
        'J' => Some("Petit Jean"),
        'T' => Some("Frere Tuck"),
        'S' => Some("Stutely"),
        'W' => Some("Will Ecarlate"),
        'M' => Some("Lady Marianne"),
        'A' => Some("Paysan A"),
        'B' => Some("Paysan B"),
        'C' => Some("Paysan C"),
        _ => {
            tracing::warn!("unknown demo PC code '{}'", code);
            None
        }
    }
}

pub(super) fn bonus_type_to_sprite_asset_for_shipping(
    raw_bonus_type: u16,
) -> Option<(&'static str, &'static str)> {
    match raw_bonus_type {
        0 => Some(("BONUS_Arrows", "BONUS Fleches")),
        1 => Some(("BONUS_Stones", "BONUS Cailloux")),
        2 => Some(("BONUS_Apples", "BONUS Pommes")),
        3 => Some(("BONUS_Ale", "BONUS Ale")),
        4 => Some(("BONUS_LegOfLamb", "BONUS Gigots")),
        5 => Some(("BONUS_Plants", "BONUS Plantes")),
        6 => Some(("BONUS_Nets", "BONUS Filets")),
        7 => Some(("BONUS_WaspsNest", "BONUS Guepes")),
        8 => Some(("BONUS_MoneyBag", "BONUS Bourses d'argent")),
        9 => Some(("BONUS_GoldBagsRansom", "BONUS Sac d'or rancon")),
        10 => Some(("BONUS_FourLeavedClover", "BONUS Trefle")),
        11 => Some(("BONUS_Shield", "Shield")),
        12 => Some(("RELIC_Ampulla", "Huile")),
        13 => Some(("RELIC_Spoon", "Cuillere")),
        14 => Some(("RELIC_Crown", "Couronne")),
        15 => Some(("RELIC_Stamp", "Sceau")),
        16 => Some(("RELIC_Sceptre", "Sceptre")),
        17 => Some(("RELIC_Book", "Registre")),
        18 => Some(("RELIC_Sword", "Epee")),
        _ => None,
    }
}

pub(super) fn add_character_action_rhs_profiles(
    required: &mut std::collections::BTreeMap<String, BTreeSet<String>>,
    actions: impl IntoIterator<Item = Action>,
) {
    for action in actions {
        let assets: &[(&str, &str)] = match action {
            Action::Bow => &[
                ("ACCESSORIES_Arrow", "ACCESSOIRES Fleche"),
                ("BONUS_Arrows", "BONUS Fleches"),
            ],
            Action::Stone => &[
                ("ACCESSORIES_Stone", "ACCESSOIRES Cailloux"),
                ("BONUS_Stones", "BONUS Cailloux"),
            ],
            Action::Apple => &[
                ("ACCESSORIES_Apple", "ACCESSOIRES Pomme"),
                ("BONUS_Apples", "BONUS Pommes"),
            ],
            Action::Ale => &[
                ("ACCESSORIES_Ale", "ACCESSOIRES Ale"),
                ("BONUS_Ale", "BONUS Ale"),
            ],
            Action::Eat | Action::Guzzle => &[("BONUS_LegOfLamb", "BONUS Gigots")],
            Action::Heal => &[("BONUS_Plants", "BONUS Plantes")],
            Action::Net => &[
                ("ACCESSORIES_Net", "ACCESSOIRES Filet"),
                ("BONUS_Nets", "BONUS Filets"),
            ],
            Action::WaspNest => &[
                ("ACCESSORIES_Wasp", "ACCESSOIRES Guepes"),
                ("ACCESSORIES_WaspSting", "Guepe"),
                ("BONUS_WaspsNest", "BONUS Guepes"),
            ],
            Action::Purse => &[
                ("ACCESSORIES_MoneyBag", "ACCESSOIRES Bourse d'argent"),
                ("ACCESSORIES_Coin", "ACCESSOIRES Piece d'or"),
                ("BONUS_MoneyBag", "BONUS Bourses d'argent"),
            ],
            _ => &[],
        };
        for &(file, profile) in assets {
            add_required_rhs_rel(required, format!("Characters/{file}.rhs"), profile);
        }
    }
}

pub(super) fn add_all_saved_world_object_rhs_profiles(
    required: &mut std::collections::BTreeMap<String, BTreeSet<String>>,
) {
    for (file, profile) in [
        ("ACCESSORIES_Arrow", "ACCESSOIRES Fleche"),
        ("ACCESSORIES_Stone", "ACCESSOIRES Cailloux"),
        ("ACCESSORIES_Ale", "ACCESSOIRES Ale"),
        ("ACCESSORIES_Apple", "ACCESSOIRES Pomme"),
        ("ACCESSORIES_MoneyBag", "ACCESSOIRES Bourse d'argent"),
        ("ACCESSORIES_Wasp", "ACCESSOIRES Guepes"),
        ("ACCESSORIES_Coat", "Manteau"),
        ("ACCESSORIES_Net", "ACCESSOIRES Filet"),
        ("ACCESSORIES_Coin", "ACCESSOIRES Piece d'or"),
        ("ACCESSORIES_WaspSting", "Guepe"),
        ("BONUS_Arrows", "BONUS Fleches"),
        ("BONUS_Stones", "BONUS Cailloux"),
        ("BONUS_Nets", "BONUS Filets"),
        ("BONUS_WaspsNest", "BONUS Guepes"),
        ("BONUS_Apples", "BONUS Pommes"),
        ("BONUS_Ale", "BONUS Ale"),
        ("BONUS_LegOfLamb", "BONUS Gigots"),
        ("BONUS_Plants", "BONUS Plantes"),
        ("BONUS_MoneyBag", "BONUS Bourses d'argent"),
        ("BONUS_GoldBagsRansom", "BONUS Sac d'or rancon"),
        ("BONUS_Shield", "Shield"),
        ("BONUS_Parchment", "BONUS Parchemin"),
        ("BONUS_FourLeavedClover", "BONUS Trefle"),
        ("RELIC_Ampulla", "Huile"),
        ("RELIC_Spoon", "Cuillere"),
        ("RELIC_Crown", "Couronne"),
        ("RELIC_Stamp", "Sceau"),
        ("RELIC_Sceptre", "Sceptre"),
        ("RELIC_Book", "Registre"),
        ("RELIC_Sword", "Epee"),
    ] {
        add_required_rhs_rel(required, format!("Characters/{file}.rhs"), profile);
    }
}

pub(super) fn parse_level_pair(
    rhp: &Path,
    rhm: &Path,
    beggar_ids: &BTreeSet<u32>,
) -> Result<(LoadedProtoLevel, LoadedMission)> {
    let file =
        SbFile::open(&rhp.to_string_lossy(), SB_FILE_READ).map_err(|e| anyhow!("open rhp: {e}"))?;
    let mut reader = ChunkReader::new(file);
    let format = {
        let tag = reader
            .peek_next_chunk()
            .map_err(|e| anyhow!("peek: {e:?}"))?;
        LevelFormat::detect(&tag).map_err(|e| anyhow!("format: {e:?}"))?
    };
    let proto = load_proto_level(&mut reader, format).map_err(|e| anyhow!("rhp: {e:?}"))?;

    let file =
        SbFile::open(&rhm.to_string_lossy(), SB_FILE_READ).map_err(|e| anyhow!("open rhm: {e}"))?;
    let mut reader = ChunkReader::new(file);
    let mission = load_mission(&mut reader, format, &|idx| beggar_ids.contains(&idx))
        .map_err(|e| anyhow!("rhm: {e:?}"))?;
    Ok((proto, mission))
}

pub(super) fn read_pak_pictures(src: &Path) -> Result<Vec<Picture>> {
    let mut file =
        SbFile::open(&src.to_string_lossy(), SB_FILE_READ).map_err(|e| anyhow!("open pak: {e}"))?;
    let total = file.get_size();
    let mut pics = Vec::new();
    while file.tell() < total {
        pics.push(Picture::load_sixteen_from_stream(&mut file).context("pak picture")?);
    }
    Ok(pics)
}
