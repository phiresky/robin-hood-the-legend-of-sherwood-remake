//! Custom colour families using the production shipping VQ bank, grouped
//! encoder, self-reference derivation, and dependency-aware materializer.
use super::*;
use crate::shipping_datadir::{RhsData, ShippingSprite, ShippingSpriteBank, SpriteVqChunk};
use crate::sprite_codec::SpriteGrid;
use anyhow::{Context, Result, ensure};
use assets_frame_holder::{FrameDictionary, RuntimeSprite, TRANSPARENT_COLOR_16};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::Arc,
};

const MAGIC: &[u8] = b"RHMODVF2";
const GROUP_TILES: usize = 1_048_576;

#[derive(Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
struct Character {
    name: String,
    metadata: HackableRhsCache,
    first: u32,
    // Shipping grids are padded to four pixels; preserve authored widths.
    widths: Vec<u16>,
}

#[derive(Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
struct Family {
    characters: Vec<Character>,
    bank: ShippingSpriteBank,
}

fn rhs(character: &Character) -> RhsData {
    let profiles = character
        .metadata
        .profiles
        .iter()
        .map(|profile| {
            let mut info = profile.info.clone();
            for script in Arc::make_mut(&mut info.scripts) {
                for id in &mut script.frame_ids {
                    *id += character.first;
                }
            }
            (profile.name.clone(), info)
        })
        .collect();
    RhsData {
        signature: 0,
        profiles,
    }
}

fn runtime_sprite(
    width: u16,
    sprite: &ShippingSprite,
    dictionary: &FrameDictionary,
) -> Result<RuntimeSprite> {
    crate::packed_sprite::validate_vq(
        &sprite.packed_data,
        sprite.width.into(),
        sprite.height.into(),
        dictionary.num_entries().into(),
    )?;
    ensure!(
        width <= sprite.width && sprite.width - width < 4,
        "invalid authored frame width"
    );
    let cols = usize::from(sprite.width / 4);
    let mut packed_data = Vec::new();
    for y in 0..usize::from(sprite.height) {
        let row: Vec<u16> = sprite.packed_data[y * cols..(y + 1) * cols]
            .iter()
            .flat_map(|&index| dictionary.lookup_pixels(index).iter().copied())
            .take(width.into())
            .collect();
        if let Some(first) = row.iter().position(|&pixel| pixel != TRANSPARENT_COLOR_16) {
            let last = row
                .iter()
                .rposition(|&pixel| pixel != TRANSPARENT_COLOR_16)
                .expect("nonempty row");
            packed_data.extend([first as u16, last as u16]);
            packed_data.extend_from_slice(&row[first..=last]);
        } else {
            packed_data.extend([u16::MAX, u16::MAX]);
        }
    }
    Ok(RuntimeSprite {
        width,
        height: sprite.height,
        packed_data,
        rgba_data: None,
    })
}

pub fn read(path: &Path) -> Result<Vec<(String, HackableRhsCache)>> {
    read_selected_bytes(&std::fs::read(path)?, None)
}

pub fn read_selected_bytes(
    compressed: &[u8],
    selected: Option<&std::collections::HashSet<String>>,
) -> Result<Vec<(String, HackableRhsCache)>> {
    let bytes = zstd::stream::decode_all(compressed)?;
    ensure!(
        bytes.starts_with(MAGIC),
        "unsupported custom VQ family format"
    );
    let mut family: Family = bitcode::decode(&bytes[MAGIC.len()..])?;
    if selected.is_some_and(|names| !family.characters.iter().any(|c| names.contains(&c.name))) {
        return Ok(Vec::new());
    }
    let mut rhs_files = BTreeMap::new();
    let mut expected = 0u32;
    for character in &family.characters {
        ensure!(
            character.first == expected,
            "non-contiguous character frame range"
        );
        expected = expected
            .checked_add(u32::try_from(character.widths.len())?)
            .context("frame count overflow")?;
        ensure!(
            character.metadata.version == HACKABLE_RHS_CACHE_VERSION
                && character.metadata.frames.is_empty()
                && character.metadata.sources.is_empty(),
            "invalid family metadata"
        );
        ensure!(
            rhs_files
                .insert(character.name.clone(), rhs(character))
                .is_none(),
            "duplicate character name"
        );
    }
    ensure!(
        expected == family.bank.sprite_count && family.bank.sprites.len() == expected as usize,
        "incomplete family sprite bank"
    );
    // TODO: prune unselected sibling chunks while retaining both hub closures
    // when a mission uses only a subset of a selected family.
    family.bank.materialize_vq_chunks(&rhs_files)?;
    let mut result = Vec::new();
    for mut character in family.characters {
        if selected.is_some_and(|names| !names.contains(&character.name)) {
            continue;
        }
        for (local, width) in character.widths.into_iter().enumerate() {
            let id = character.first as usize + local;
            let (stored_id, sprite) = family
                .bank
                .sprites
                .get(id)
                .context("missing family frame")?;
            ensure!(*stored_id as usize == id, "unordered family frame IDs");
            let dictionary = family
                .bank
                .dictionaries
                .get(usize::from(sprite.dictionary_index))
                .context("missing family dictionary")?;
            character
                .metadata
                .frames
                .push(runtime_sprite(width, sprite, dictionary)?);
        }
        validate_cache_frames(&character.name, &character.metadata)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        result.push((character.name, character.metadata));
    }
    Ok(result)
}

// Same sampled conditional-entropy hub selection as convert_datadir, with
// 16-bit tuple keys because lossless PNG dictionaries can exceed 4096 tiles.
fn proxy(member: &[Vec<u16>], base: Option<&[Vec<u16>]>, widths: &[u16]) -> f64 {
    let mut joint = HashMap::<(u16, u16), u32>::new();
    let mut contexts = HashMap::<u16, u32>::new();
    let mut sampled = 0usize;
    let full: usize = member.iter().map(Vec::len).sum();
    for (index, grid) in member.iter().enumerate() {
        let cols = usize::from(widths[index].div_ceil(4));
        if sampled >= 1_500_000 {
            break;
        }
        for (position, &symbol) in
            grid.iter()
                .enumerate()
                .skip(if base.is_some() { 0 } else { cols })
        {
            let context = base.map_or_else(|| grid[position - cols], |base| base[index][position]);
            *joint.entry((context, symbol)).or_default() += 1;
            *contexts.entry(context).or_default() += 1;
            sampled += 1;
        }
    }
    if sampled == 0 {
        // One-row sprites have no above-neighbour pairs. Price their actual
        // order-zero entropy instead of claiming they require no bits.
        let mut frequencies = HashMap::<u16, usize>::new();
        for &symbol in member.iter().flatten() {
            *frequencies.entry(symbol).or_default() += 1;
        }
        return frequencies
            .values()
            .map(|&count| count as f64 * (full as f64 / count as f64).log2())
            .sum();
    }
    joint
        .iter()
        .map(|(&(context, _), &count)| {
            f64::from(count) * (f64::from(contexts[&context]) / f64::from(count)).log2()
        })
        .sum::<f64>()
        * full as f64
        / sampled as f64
}

/// Convert one group of positionally identical animation layouts. Uses a
/// stable dictionary per character, measured two-hub prediction, and the
/// production 1 Mi-tile grouping. Every result is verified through `read`.
pub fn encode_custom_sprite_family(sources: &[PathBuf], destination: &Path) -> Result<usize> {
    ensure!(
        !sources.is_empty() && !destination.exists(),
        "empty family or existing destination"
    );
    let mut characters = Vec::new();
    let mut originals = Vec::new();
    let mut grids = Vec::<Vec<Vec<u16>>>::new();
    let mut bank = ShippingSpriteBank {
        signature: 0,
        dictionaries: Vec::new(),
        sprite_count: 0,
        sprites: Vec::new(),
        vq_chunks: Vec::new(),
        rle_jxl_chunks: Vec::new(),
    };
    for source in sources {
        let bytes = std::fs::read(source.join("manifest.json"))?;
        let manifest: HackableRhsManifest = serde_json::from_slice(&bytes)?;
        ensure!(
            matches!(
                manifest.pixel_format,
                HackableRhsPixelFormat::LegacyColorKeys
            ),
            "VQ families require legacy_color_keys"
        );
        let mut metadata = build_hackable_cache(source, hackable_manifest_hash(&bytes), manifest)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let frames = std::mem::take(&mut metadata.frames);
        metadata.sources.clear();
        if let Some(previous) = characters.first() {
            let previous: &Character = previous;
            ensure!(
                bitcode::encode(&metadata.profiles) == bitcode::encode(&previous.metadata.profiles),
                "family animation layouts differ"
            );
            let original: &Vec<RuntimeSprite> = &originals[0];
            ensure!(
                frames.len() == original.len()
                    && frames
                        .iter()
                        .zip(original)
                        .all(|(a, b)| (a.width, a.height) == (b.width, b.height)),
                "family frame dimensions differ"
            );
        }
        let mut frequency = HashMap::<[u16; 4], usize>::new();
        for frame in &frames {
            for tile in shipping::frame_tiles(frame)? {
                *frequency.entry(tile).or_default() += 1;
            }
        }
        let mut tiles: Vec<_> = frequency.keys().copied().collect();
        tiles.sort_by_key(|tile| (std::cmp::Reverse(frequency[tile]), *tile));
        if tiles.is_empty() {
            tiles.push([TRANSPARENT_COLOR_16; 4]);
        }
        let alphabet = u16::try_from(tiles.len())
            .context("lossless character dictionary exceeds 65535 entries")?;
        let dictionary =
            FrameDictionary::from_raw(alphabet, tiles.iter().flatten().copied().collect());
        ensure!(
            tiles
                .iter()
                .enumerate()
                .all(|(index, tile)| dictionary.lookup_pixels(index as u16) == tile),
            "legacy dictionary normalization would change authored pixels"
        );
        let indices: HashMap<_, _> = tiles
            .into_iter()
            .enumerate()
            .map(|(index, tile)| (tile, index as u16))
            .collect();
        let first = bank.sprite_count;
        let dictionary_index = u16::try_from(bank.dictionaries.len())?;
        let mut character_grids = Vec::new();
        for frame in &frames {
            let grid = shipping::frame_tiles(frame)?
                .into_iter()
                .map(|tile| indices[&tile])
                .collect();
            character_grids.push(grid);
            bank.sprites.push((
                bank.sprite_count,
                ShippingSprite {
                    width: frame.width.checked_add(3).context("frame width overflow")? / 4 * 4,
                    height: frame.height,
                    dictionary_index,
                    packed_data: Arc::new(Vec::new()),
                    raster: None,
                },
            ));
            bank.sprite_count += 1;
        }
        let name = source
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".rhs.d"))
            .context("expected .rhs.d directory")?
            .to_owned();
        println!(
            "{}: {} frames, {} dictionary entries",
            name,
            frames.len(),
            alphabet
        );
        characters.push(Character {
            name,
            metadata,
            first,
            widths: frames.iter().map(|frame| frame.width).collect(),
        });
        bank.dictionaries.push(dictionary);
        grids.push(character_grids);
        originals.push(frames);
    }
    let n = characters.len();
    let pair: Vec<Vec<f64>> = (0..n)
        .map(|base| {
            (0..n)
                .map(|member| {
                    proxy(
                        &grids[member],
                        if base == member {
                            None
                        } else {
                            Some(&grids[base])
                        },
                        &characters[member].widths,
                    )
                })
                .collect()
        })
        .collect();
    let hub1 = (0..n)
        .min_by(|&a, &b| pair[a].iter().sum::<f64>().total_cmp(&pair[b].iter().sum()))
        .expect("nonempty family");
    let hub2 = (0..n).filter(|&i| i != hub1).min_by(|&a, &b| {
        let cost = |candidate: usize| {
            (0..n)
                .filter(|&m| m != hub1 && m != candidate)
                .map(|m| pair[candidate][m])
                .sum::<f64>()
        };
        cost(a).total_cmp(&cost(b))
    });
    println!(
        "Family hubs: {} / {}",
        characters[hub1].name,
        hub2.map(|i| characters[i].name.as_str()).unwrap_or("none")
    );
    for (member, character) in characters.iter().enumerate() {
        let base = (member != hub1).then_some(hub1);
        let base2 = hub2.filter(|&i| member != hub1 && member != i);
        let ids: Vec<_> =
            (character.first..character.first + character.widths.len() as u32).collect();
        let metadata = rhs(character);
        let views: Vec<_> = grids[member]
            .iter()
            .enumerate()
            .map(|(i, indices)| SpriteGrid {
                cols: character.widths[i].div_ceil(4),
                rows: originals[member][i].height,
                indices,
            })
            .collect();
        let bases: Vec<_> = (0..ids.len())
            .map(|i| base.map(|b| grids[b][i].as_slice()))
            .collect();
        let bases2: Vec<_> = (0..ids.len())
            .map(|i| base2.map(|b| grids[b][i].as_slice()))
            .collect();
        let template = SpriteVqChunk {
            rhs: character.name.clone(),
            base_rhs: base.map(|b| characters[b].name.clone()),
            base2_rhs: base2
                .map(|b| characters[b].name.clone())
                .unwrap_or_default(),
            alphabet: bank.dictionaries[member].num_entries(),
            sprite_ids: ids,
            base_ids: (0..character.widths.len())
                .map(|i| base.map(|b| characters[b].first + i as u32))
                .collect(),
            base2_ids: (0..character.widths.len())
                .map(|i| base2.map(|b| characters[b].first + i as u32))
                .collect(),
            self_refs: base.is_none(),
            blob: Vec::new(),
        };
        let chunks = crate::sprite_groups::encode_vq_groups(
            &template,
            &views,
            &bases,
            &bases2,
            Some(&metadata),
            GROUP_TILES,
        )?;
        println!(
            "{}: {} VQ bytes in {} groups",
            character.name,
            chunks.iter().map(|c| c.blob.len()).sum::<usize>(),
            chunks.len()
        );
        bank.vq_chunks.extend(chunks);
    }
    let profile_bytes: Vec<_> = characters
        .iter()
        .map(|c| bitcode::encode(&c.metadata.profiles))
        .collect();
    let mut encoded = MAGIC.to_vec();
    encoded.extend(bitcode::encode(&Family { characters, bank }));
    let compressed = zstd::stream::encode_all(encoded.as_slice(), 22)?;
    std::fs::write(destination, compressed)?;
    let decoded = read(destination)?;
    ensure!(decoded.len() == originals.len(), "character count changed");
    let mut count = 0;
    for (index, (name, cache)) in decoded.iter().enumerate() {
        ensure!(
            bitcode::encode(&cache.profiles) == profile_bytes[index],
            "{name}: animation metadata changed"
        );
        ensure!(
            cache.frames.len() == originals[index].len(),
            "{name}: frame count changed"
        );
        for (before, after) in originals[index].iter().zip(&cache.frames) {
            ensure!(
                before.width == after.width
                    && before.height == after.height
                    && before.packed_data == after.packed_data,
                "{name}: pixels changed"
            );
        }
        count += cache.frames.len();
    }
    println!(
        "Verified {count} frames: {} ({} bytes)",
        destination.display(),
        std::fs::metadata(destination)?.len()
    );
    Ok(count)
}
