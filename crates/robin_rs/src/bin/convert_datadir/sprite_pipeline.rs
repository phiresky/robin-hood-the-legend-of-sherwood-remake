//! Plan family hubs and transform the selected RHS dependency closure.
use super::*;

fn pick_weighted_family_hub(
    costs: &[(f64, &String)],
    mission_use_count: &impl Fn(&str) -> usize,
) -> Option<String> {
    const FAMILY_HUB_PROXY_TOLERANCE: f64 = 1.05;
    let best = costs
        .iter()
        .map(|(cost, _)| *cost)
        .fold(f64::INFINITY, f64::min);
    costs
        .iter()
        .filter(|(cost, _)| *cost <= best * FAMILY_HUB_PROXY_TOLERANCE)
        .map(|(cost, name)| (cost, name, mission_use_count(name)))
        .max_by(|(ca, na, uses_a), (cb, nb, uses_b)| {
            uses_a
                .cmp(uses_b)
                .then(cb.total_cmp(ca))
                .then_with(|| nb.cmp(na))
        })
        .map(|(_, name, _)| (*name).clone())
}

pub(super) fn transform_rhs(
    data_in: &Path,
    holder: &FrameHolder,
    dict_remaps: Option<&[Vec<u16>]>,
    opts: &ShippingOpts,
    compression_pool: &rayon::ThreadPool,
    dependency_plan: &DependencyPlan,
    in_path: &dyn Fn(&str) -> Option<PathBuf>,
) -> Result<(
    std::collections::BTreeMap<String, ShippingMission>,
    std::collections::BTreeMap<String, Vec<String>>,
)> {
    // Phase A: load each required RHS and resolve which bank sprites its
    // matched profiles reach. Payload assembly happens after the family pass
    // below so variant chunks can be coded against their family base.
    let mut rhs_preps = std::collections::BTreeMap::<String, RhsChunkPrep>::new();
    for (rel, planned) in &dependency_plan.rhs {
        let required_profiles = &planned.profiles;
        if rel.is_empty() {
            continue;
        }
        let path = in_path(rel)
            .ok_or_else(|| anyhow!("required authoritative shipping RHS is missing: {rel}"))?;
        let mut used_sprite_ids = BTreeSet::<u32>::new();
        let (signature, profiles) =
            sprite_script::SpriteScriptor::load_all_profiles_legacy(&path.to_string_lossy())
                .map_err(|error| anyhow!("rhs {rel}: {error}"))?;
        let mut script_order = Vec::new();
        for (_, info) in &profiles {
            for script in info.scripts.iter() {
                script_order.extend_from_slice(&script.frame_ids);
            }
        }
        let all_profiles_required = required_profiles.contains("");
        let mut matched_profiles = BTreeSet::new();
        for (profile_name, info) in &profiles {
            if all_profiles_required || required_profiles.contains(profile_name) {
                matched_profiles.insert(profile_name.clone());
                for script in info.scripts.iter() {
                    used_sprite_ids.extend(script.frame_ids.iter().copied());
                }
            }
        }
        for required in required_profiles {
            if !required.is_empty() && !matched_profiles.contains(required) {
                bail!("authoritative shipping RHS {rel} is missing required profile '{required}'");
            }
        }
        let profiles = profiles
            .into_iter()
            .filter(|(name, _)| all_profiles_required || matched_profiles.contains(name))
            .collect();
        rhs_preps.insert(
            rel.clone(),
            RhsChunkPrep {
                rhs_data: Some(RhsData {
                    signature,
                    profiles,
                }),
                matched_profiles: matched_profiles.len(),
                script_order,
                used_sprite_ids,
                base_rel: None,
                base_ids: std::collections::BTreeMap::new(),
                base2_rel: None,
                base2_ids: std::collections::BTreeMap::new(),
            },
        );
    }

    // Phase B: variant families among Characters/*.rhs (the probe's corpus
    // rule: trailing-two-digit stem shared by more than one file; base = the
    // lexicographically first member). Variant chunks are coded against the
    // base's positionally aligned grids — measured 3.9x smaller than zstd on
    // that half of the character corpus (docs/COMPRESSION.md, 2026-08-28).
    let mut disk_character_names: Vec<String> = Vec::new();
    if let Some(dir) = resolve_case_insensitive(&data_in.join("Characters")).filter(|p| p.is_dir())
    {
        for entry in fs::read_dir(&dir).with_context(|| format!("read_dir {}", dir.display()))? {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("rhs"))
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                disk_character_names.push(stem.to_owned());
            }
        }
    }
    disk_character_names.sort();
    let family_key = |name: &str| -> Option<String> {
        let stripped = name.trim_end_matches(|c: char| c.is_ascii_digit());
        (stripped.len() + 2 == name.len() && !stripped.is_empty()).then(|| stripped.to_owned())
    };
    let mut families = std::collections::BTreeMap::<String, Vec<String>>::new();
    for name in &disk_character_names {
        if let Some(key) = family_key(name) {
            families.entry(key).or_default().push(name.clone());
        }
    }
    families.retain(|_, members| members.len() > 1);

    // Measured base selection (docs/COMPRESSION.md 2026-08-29): the
    // lexicographically-first member is the best coding hub in only 1 of 9
    // fullgame families; choosing the base that minimizes a sampled
    // conditional-entropy proxy — H(base | above) for the base itself plus
    // H(member | base tile) for every other member — recovers ~4% of the
    // family half of the corpus at zero format cost (the chunk already
    // records `base_rhs`). Falls back to the first member when the proxy
    // cannot be computed (e.g. indices beyond the 12-bit proxy key).
    let mut member_orders = std::collections::BTreeMap::<String, Vec<u32>>::new();
    for members in families.values() {
        for name in members {
            if member_orders.contains_key(name) {
                continue;
            }
            let rel = format!("Characters/{name}.rhs");
            let path = in_path(&rel)
                .ok_or_else(|| anyhow!("family member RHS {rel} missing from the datadir"))?;
            let (_, profiles) =
                sprite_script::SpriteScriptor::load_all_profiles_legacy(&path.to_string_lossy())
                    .map_err(|error| anyhow!("rhs {rel}: {error}"))?;
            let mut order = Vec::new();
            for (_, info) in &profiles {
                for script in info.scripts.iter() {
                    order.extend_from_slice(&script.frame_ids);
                }
            }
            member_orders.insert(name.clone(), order);
        }
    }
    // First-load weighting: a hub chunk that missions already require adds
    // nothing to their closures, while a dependency-only hub adds its whole
    // (large, standalone-coded) chunk — measured ~7 MB extra on H01's first
    // load with pure compression-optimal hubs. Among candidates within
    // 5% of the best compression proxy, prefer the
    // member the most missions reference.
    let mission_use_counts = dependency_plan.mission_use_counts();
    let mission_use_count = |name: &str| -> usize {
        let rel = format!("Characters/{name}.rhs").to_ascii_lowercase();
        // Dependency-only family hubs legitimately have no mission consumers.
        mission_use_counts.get(&rel).copied().unwrap_or(0)
    };
    let mut family_bases = std::collections::BTreeMap::<String, String>::new();
    for (key, members) in &families {
        let mut costs: Vec<(f64, &String)> = Vec::new();
        let mut proxy_failed = false;
        for candidate in members {
            let mut cost = match family_base_standalone_proxy(holder, &member_orders[candidate]) {
                Some(bits) => bits,
                None => {
                    proxy_failed = true;
                    break;
                }
            };
            for member in members {
                if member == candidate {
                    continue;
                }
                match family_base_pair_proxy(
                    holder,
                    &member_orders[candidate],
                    &member_orders[member],
                ) {
                    Some(bits) => cost += bits,
                    None => {
                        proxy_failed = true;
                        break;
                    }
                }
            }
            if proxy_failed {
                break;
            }
            costs.push((cost, candidate));
        }
        let base = match (
            proxy_failed,
            pick_weighted_family_hub(&costs, &mission_use_count),
        ) {
            (false, Some(name)) => name,
            _ => {
                tracing::warn!(
                    family = key.as_str(),
                    "family base proxy unavailable; falling back to first member"
                );
                members[0].clone()
            }
        };
        tracing::info!(
            family = key.as_str(),
            base = base.as_str(),
            missions_using = mission_use_count(&base),
            "selected family coding base"
        );
        family_bases.insert(key.clone(), base);
    }

    // Star-2 topology (schema v10, docs/COMPRESSION.md 2026-08-29): family
    // members after the first two code each tile against TWO already-decoded
    // siblings — measured -22..25% on third-and-later members. hub1 is the
    // proxy-selected base above; hub2 is the member (excluding hub1) that is
    // the best SECOND predictor for the remaining members: argmin over
    // candidates c != hub1 of sum over members m not in {hub1, c} of
    // H(m | c tile). hub2's own chunk keeps coding against hub1 only.
    // Two-member families have no "third-and-later" members and skip this.
    let mut family_second_bases = std::collections::BTreeMap::<String, String>::new();
    for (key, members) in &families {
        if members.len() < 3 {
            continue;
        }
        let hub1 = &family_bases[key];
        let mut costs: Vec<(f64, &String)> = Vec::new();
        let mut proxy_failed = false;
        for candidate in members {
            if candidate == hub1 {
                continue;
            }
            let mut cost = 0.0;
            for member in members {
                if member == candidate || member == hub1 {
                    continue;
                }
                match family_base_pair_proxy(
                    holder,
                    &member_orders[candidate],
                    &member_orders[member],
                ) {
                    Some(bits) => cost += bits,
                    None => {
                        proxy_failed = true;
                        break;
                    }
                }
            }
            if proxy_failed {
                break;
            }
            costs.push((cost, candidate));
        }
        match (
            proxy_failed,
            pick_weighted_family_hub(&costs, &mission_use_count),
        ) {
            (false, Some(name)) => {
                tracing::info!(
                    family = key.as_str(),
                    base2 = name.as_str(),
                    missions_using = mission_use_count(&name),
                    "selected family second base"
                );
                family_second_bases.insert(key.clone(), name);
            }
            _ => {
                tracing::warn!(
                    family = key.as_str(),
                    "family second-base proxy unavailable; coding this family star-1"
                );
            }
        }
    }

    // Lowercased variant name -> disk-cased base name. CPF-derived rels and
    // on-disk filenames can disagree in case, so matching is case-blind.
    let variant_base_names: std::collections::BTreeMap<String, &str> = families
        .iter()
        .flat_map(|(key, members)| {
            let base = family_bases[key].as_str();
            members
                .iter()
                .filter(move |name| name.as_str() != base)
                .map(move |name| (name.to_ascii_lowercase(), base))
        })
        .collect();
    // Lowercased third-and-later member name -> disk-cased second-hub name.
    // hub2 itself keeps coding against hub1 only, so it is excluded here.
    let variant_base2_names: std::collections::BTreeMap<String, &str> = families
        .iter()
        .filter_map(|(key, members)| {
            let hub2 = family_second_bases.get(key)?;
            let hub1 = &family_bases[key];
            Some(
                members
                    .iter()
                    .filter(move |name| *name != hub1 && *name != hub2)
                    .map(move |name| (name.to_ascii_lowercase(), hub2.as_str())),
            )
        })
        .flatten()
        .collect();

    let mut loaded_base_script_orders = std::collections::BTreeMap::<String, Vec<u32>>::new();
    struct PlannedVariant {
        rel: String,
        base_rel: String,
        base_ids: std::collections::BTreeMap<u32, u32>,
        base2_rel: Option<String>,
        base2_ids: std::collections::BTreeMap<u32, u32>,
        /// Hub chunk rels this variant's decode depends on at install time.
        dep_rels: Vec<String>,
    }
    let mut planned_variants = Vec::<PlannedVariant>::new();
    let mut base_extra_ids = std::collections::BTreeMap::<String, BTreeSet<u32>>::new();
    for (rel, variant_prep) in &rhs_preps {
        let Some(name) = rel
            .strip_prefix("Characters/")
            .and_then(|n| n.strip_suffix(".rhs"))
        else {
            continue;
        };
        let normalized_name = name.to_ascii_lowercase();
        let Some(base_name) = variant_base_names.get(&normalized_name) else {
            continue;
        };
        let base_rel = resolve_family_hub_rel(
            &rhs_preps,
            &mut loaded_base_script_orders,
            &in_path,
            base_name,
            rel,
        )?;
        // Second hub, when this member is third-or-later in a star-2 family.
        let base2_rel = match variant_base2_names.get(&normalized_name) {
            Some(hub2_name) => Some(resolve_family_hub_rel(
                &rhs_preps,
                &mut loaded_base_script_orders,
                &in_path,
                hub2_name,
                rel,
            )?),
            None => None,
        };
        let hub_script = |hub_rel: &str| {
            rhs_preps
                .get(hub_rel)
                .map(|prep| prep.script_order.as_slice())
                .or_else(|| loaded_base_script_orders.get(hub_rel).map(Vec::as_slice))
                .expect("hub script order resolved above")
        };
        // Positional pairing over the script frame-id tables (the variant's
        // tables mirror each hub's 1:1); duplicated variant frames that pair
        // with conflicting hub frames fall back per hub.
        let pair_base = positional_pair_map(&variant_prep.script_order, hub_script(&base_rel));
        let pair_base2 = base2_rel
            .as_deref()
            .map(|hub2_rel| positional_pair_map(&variant_prep.script_order, hub_script(hub2_rel)));
        let mut base_ids = std::collections::BTreeMap::<u32, u32>::new();
        let mut base2_ids = std::collections::BTreeMap::<u32, u32>::new();
        let mut hub1_used = BTreeSet::<u32>::new();
        let mut hub2_used = BTreeSet::<u32>::new();
        let (mut vq_total, mut unbased) = (0usize, 0usize);
        for &vid in &variant_prep.used_sprite_ids {
            let sprite = holder.sprites().get(vid as usize).ok_or_else(|| {
                anyhow!("RHS {rel} references sprite {vid} beyond the shipping bank")
            })?;
            let Some(packed) = holder.packed_data(vid) else {
                continue;
            };
            if sprite.dictionary_index == UNMAPPED_DICT
                || vq_grid_words(sprite.width, sprite.height) != Some(packed.len())
            {
                continue;
            }
            vq_total += 1;
            let aligned = |bid: &u32| {
                let Some(base_sprite) = holder.sprites().get(*bid as usize) else {
                    return false;
                };
                let Some(base_packed) = holder.packed_data(*bid) else {
                    return false;
                };
                base_sprite.dictionary_index != UNMAPPED_DICT
                    && (base_sprite.width, base_sprite.height) == (sprite.width, sprite.height)
                    && base_packed.len() == packed.len()
            };
            let b1 = pair_base.get(&vid).copied().filter(aligned);
            let b2 = pair_base2
                .as_ref()
                .and_then(|pairs| pairs.get(&vid))
                .copied()
                .filter(aligned);
            // The probe's `code3` ladder: both aligned predecessors when
            // possible; a sprite aligning with only one hub takes that hub as
            // its single base (base ids are plain bank ids, so a hub2 sprite
            // works as a primary base); otherwise standalone.
            match (b1, b2) {
                (Some(b1), Some(b2)) => {
                    base_ids.insert(vid, b1);
                    base2_ids.insert(vid, b2);
                    hub1_used.insert(b1);
                    hub2_used.insert(b2);
                }
                (Some(b1), None) => {
                    base_ids.insert(vid, b1);
                    hub1_used.insert(b1);
                }
                (None, Some(b2)) => {
                    base_ids.insert(vid, b2);
                    hub2_used.insert(b2);
                }
                (None, None) => unbased += 1,
            }
        }
        if base_ids.is_empty() || unbased * 10 > vq_total {
            tracing::info!(
                rhs = rel.as_str(),
                base = base_rel.as_str(),
                base2 = base2_rel.as_deref().unwrap_or(""),
                vq = vq_total,
                unbased,
                "family variant pairs poorly with its hubs; coding standalone"
            );
            continue;
        }
        base_extra_ids
            .entry(base_rel.clone())
            .or_default()
            .extend(hub1_used.iter().copied());
        if let Some(hub2_rel) = base2_rel.as_ref().filter(|_| !hub2_used.is_empty()) {
            base_extra_ids
                .entry(hub2_rel.clone())
                .or_default()
                .extend(hub2_used.iter().copied());
        }
        // Dependency edges: hub1 always (hub2's own chunk decodes against
        // hub1, so hub1 must be in the closure whenever hub2 is), plus hub2
        // when any of its grids are referenced.
        let mut dep_rels = vec![base_rel.clone()];
        if let Some(hub2_rel) = base2_rel.as_ref().filter(|_| !hub2_used.is_empty()) {
            dep_rels.push(hub2_rel.clone());
        }
        planned_variants.push(PlannedVariant {
            rel: rel.clone(),
            base_rel,
            base_ids,
            base2_rel: base2_rel.filter(|_| !base2_ids.is_empty()),
            base2_ids,
            dep_rels,
        });
    }
    // Variant chunk -> family-hub chunks. Every dependency list that names a
    // variant chunk must also name its hub chunks: the runtime decodes the
    // variant grids against the hubs' materialized grids at install time.
    let mut rhs_base_dep = std::collections::BTreeMap::<String, Vec<String>>::new();
    for planned in planned_variants {
        rhs_base_dep.insert(planned.rel.clone(), planned.dep_rels);
        let prep = rhs_preps
            .get_mut(&planned.rel)
            .expect("variant prep exists");
        prep.base_rel = Some(planned.base_rel);
        prep.base_ids = planned.base_ids;
        prep.base2_rel = planned.base2_rel;
        prep.base2_ids = planned.base2_ids;
    }
    // The hub grids a variant decodes against must ship in the hub chunks
    // even when no mission profile reaches them (or the whole hub RHS).
    // TODO: extra grids landing in a hub2 chunk that is itself a planned
    // variant are coded standalone within that chunk (its own base pairing
    // was fixed before the extras arrived); pairing them against hub1 too
    // would shave a little more.
    for (base_rel, extra) in base_extra_ids {
        match rhs_preps.get_mut(&base_rel) {
            Some(prep) => prep.used_sprite_ids.extend(extra),
            None => {
                tracing::info!(
                    rhs = base_rel.as_str(),
                    sprites = extra.len(),
                    "synthesizing sprite-only family-hub chunk"
                );
                rhs_preps.insert(
                    base_rel.clone(),
                    RhsChunkPrep {
                        rhs_data: None,
                        matched_profiles: 0,
                        script_order: Vec::new(),
                        used_sprite_ids: extra,
                        base_rel: None,
                        base_ids: std::collections::BTreeMap::new(),
                        base2_rel: None,
                        base2_ids: std::collections::BTreeMap::new(),
                    },
                );
            }
        }
    }

    // Sprites referenced by more than one chunk must keep exact RLE words
    // in each (independent lossy encodes of one bank slot would conflict at
    // mission merge, where duplicate rows are required to be identical).
    let multi_chunk_ids: std::collections::HashSet<u32> = {
        let mut seen = std::collections::HashSet::<u32>::new();
        let mut multi = std::collections::HashSet::<u32>::new();
        for prep in rhs_preps.values() {
            for &id in &prep.used_sprite_ids {
                if !seen.insert(id) {
                    multi.insert(id);
                }
            }
        }
        multi
    };

    // Phase C: assemble the chunk payloads. `encode_grids` dominates this
    // stage, so it runs on the bounded worker pool. Consume preparations so
    // each worker releases its input after building the corresponding payload.
    let built_payloads = compression_pool.install(|| {
        rhs_preps
            .into_par_iter()
            .map(|(rel, prep)| {
                let (payload, rle_stats) = build_rhs_chunk_payload(
                    holder,
                    dict_remaps,
                    &rel,
                    prep,
                    opts.rle_sprite_format,
                    opts.vq_group_tiles,
                    opts.rle_group_blobs,
                    &multi_chunk_ids,
                )?;
                Ok((rel, payload, rle_stats))
            })
            .collect::<Vec<Result<(String, ShippingMission, RleJxlChunkStats)>>>()
    });
    let mut rhs_payloads = std::collections::BTreeMap::<String, ShippingMission>::new();
    let mut rle_totals = RleJxlChunkStats::default();
    for built in built_payloads {
        let (rel, payload, rle_stats) = built?;
        rle_totals.add(&rle_stats);
        rhs_payloads.insert(rel, payload);
    }
    if opts.rle_sprite_format.jxl_quality().is_some() {
        tracing::info!(
            atlased = rle_totals.atlased,
            individual = rle_totals.individual,
            kept_smaller = rle_totals.kept_smaller,
            kept_small_dim = rle_totals.kept_small_dim,
            kept_shared = rle_totals.kept_shared,
            kept_irregular = rle_totals.kept_irregular,
            kept_low_psnr = rle_totals.kept_low_psnr,
            jxl_bytes = rle_totals.jxl_bytes,
            raw_bytes_replaced = 2 * rle_totals.raw_words_replaced,
            resident_atlas_bytes = 2 * rle_totals.atlas_pixels,
            resident_sprite_bytes = 2 * rle_totals.sprite_canvas_pixels,
            "RLE sprite bucket encoded as lossy JXL (web recipe)"
        );
    }

    Ok((rhs_payloads, rhs_base_dep))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_selection_weights_only_eligible_candidates_once() {
        let names = ["small", "popular", "too_large"].map(str::to_owned);
        let costs = [(100.0, &names[0]), (105.0, &names[1]), (105.01, &names[2])];
        let visited = std::cell::RefCell::new(Vec::new());
        let selected = pick_weighted_family_hub(&costs, &|name| {
            visited.borrow_mut().push(name.to_owned());
            match name {
                "small" => 1,
                "popular" => 2,
                _ => panic!("ineligible candidate must not be weighted"),
            }
        });
        assert_eq!(selected.as_deref(), Some("popular"));
        assert_eq!(*visited.borrow(), ["small", "popular"]);
    }

    #[test]
    fn hub_selection_breaks_ties_by_cost_then_name() {
        let names = ["Z", "A", "B"].map(str::to_owned);
        for costs in [
            [(100.0, &names[0]), (101.0, &names[1]), (102.0, &names[2])],
            [(102.0, &names[2]), (101.0, &names[1]), (100.0, &names[0])],
        ] {
            assert_eq!(
                pick_weighted_family_hub(&costs, &|_| 1).as_deref(),
                Some("Z")
            );
        }
        for costs in [
            [(100.0, &names[0]), (100.0, &names[1]), (100.0, &names[2])],
            [(100.0, &names[2]), (100.0, &names[1]), (100.0, &names[0])],
        ] {
            assert_eq!(
                pick_weighted_family_hub(&costs, &|_| 1).as_deref(),
                Some("A")
            );
        }
        assert_eq!(
            pick_weighted_family_hub(&[], &|_| panic!("no candidate")),
            None
        );
        assert_eq!(
            pick_weighted_family_hub(&[(f64::NAN, &names[0])], &|_| panic!("invalid cost")),
            None
        );
    }
}
