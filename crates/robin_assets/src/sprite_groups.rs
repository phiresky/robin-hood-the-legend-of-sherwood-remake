//! Independent sprite jobs for conversion and offline regrouping.
//!
//! Groups restart the adaptive VQ model only between whole grids. They keep
//! external family bases, but derive temporal references within each group.

use anyhow::{Result, ensure};
use std::ops::Range;

use crate::{
    shipping_datadir::{RhsData, SpriteRleJxlChunk, SpriteVqChunk, derive_chunk_self_refs},
    sprite_codec::{SpriteGrid, encode_grids_shipping},
};

/// Conditional entropy estimated from (joint count, context count) pairs,
/// scaled to the full tile population. Key representation and sampling policy
/// belong to the caller; no observations preserve the converter's zero estimate.
pub fn conditional_entropy_bits(
    counts: impl IntoIterator<Item = (u64, u64)>,
    sampled: u64,
    full_tiles: u64,
) -> f64 {
    if sampled == 0 {
        return 0.0;
    }
    let mut bits = 0.0;
    for (count, context_count) in counts {
        assert!(
            count > 0 && count <= context_count,
            "invalid entropy counts"
        );
        bits -= count as f64 * (count as f64 / context_count as f64).log2();
    }
    bits / sampled as f64 * full_tiles as f64
}

#[test]
fn entropy_estimate_keeps_sampling_scale_and_empty_policy() {
    assert_eq!(conditional_entropy_bits([], 0, 100), 0.0);
    assert_eq!(conditional_entropy_bits([(4, 4)], 4, 40), 0.0);
    assert_eq!(conditional_entropy_bits([(2, 4), (2, 4)], 4, 40), 40.0);
    assert_eq!(
        conditional_entropy_bits([(1, 2), (1, 2), (2, 2)], 4, 40),
        20.0
    );
}

/// Contiguous whole-grid ranges with a soft tile budget. A grid larger than
/// the budget stays intact in its own group; zero preserves one original job.
pub fn vq_group_ranges(grids: &[SpriteGrid<'_>], max_tiles: usize) -> Result<Vec<Range<usize>>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut tiles = 0usize;
    for (index, grid) in grids.iter().enumerate() {
        ensure!(
            grid.indices.len() == usize::from(grid.cols) * usize::from(grid.rows),
            "grid {index}: dimensions do not match tile count"
        );
        if max_tiles != 0 && index > start && grid.indices.len() > max_tiles.saturating_sub(tiles) {
            ranges.push(start..index);
            start = index;
            tiles = 0;
        }
        tiles = tiles
            .checked_add(grid.indices.len())
            .ok_or_else(|| anyhow::anyhow!("group tile count overflow"))?;
    }
    if start < grids.len() {
        ranges.push(start..grids.len());
    }
    Ok(ranges)
}

/// Re-encode complete groups using the template's identities. The template's
/// blob is ignored; callers must provide materialized grids and family bases.
pub fn encode_vq_groups(
    template: &SpriteVqChunk,
    grids: &[SpriteGrid<'_>],
    bases: &[Option<&[u16]>],
    bases2: &[Option<&[u16]>],
    rhs: Option<&RhsData>,
    max_tiles: usize,
) -> Result<Vec<SpriteVqChunk>> {
    let count = grids.len();
    ensure!(
        template.sprite_ids.len() == count
            && template.base_ids.len() == count
            && bases.len() == count
            && bases2.len() == count
            && (template.base2_ids.is_empty() || template.base2_ids.len() == count),
        "{}: inconsistent VQ group input lengths",
        template.rhs
    );
    ensure!(
        template.sprite_ids.windows(2).all(|ids| ids[0] < ids[1]),
        "{}: VQ sprite IDs must be strictly increasing",
        template.rhs
    );
    if template.self_refs {
        ensure!(
            rhs.is_some(),
            "{}: missing self-reference metadata",
            template.rhs
        );
    }
    for index in 0..count {
        ensure!(
            template.base_ids[index].is_some() == bases[index].is_some()
                && template.base2_ids.get(index).is_some_and(Option::is_some)
                    == bases2[index].is_some(),
            "{}: grid {index} base data does not match base identities",
            template.rhs
        );
    }
    let mut groups = Vec::new();
    for range in vq_group_ranges(grids, max_tiles)? {
        let ids = &template.sprite_ids[range.clone()];
        let selfrefs = if template.self_refs {
            derive_chunk_self_refs(&rhs.expect("metadata checked above").profiles, ids)
        } else {
            vec![None; ids.len()]
        };
        let blob = encode_grids_shipping(
            template.alphabet,
            &grids[range.clone()],
            Some(&bases[range.clone()]),
            Some(&bases2[range.clone()]),
            &selfrefs,
        )?;
        groups.push(SpriteVqChunk {
            rhs: template.rhs.clone(),
            base_rhs: template.base_rhs.clone(),
            base2_rhs: template.base2_rhs.clone(),
            alphabet: template.alphabet,
            sprite_ids: ids.to_vec(),
            base_ids: template.base_ids[range.clone()].to_vec(),
            base2_ids: if template.base2_ids.is_empty() {
                Vec::new()
            } else {
                template.base2_ids[range].to_vec()
            },
            self_refs: selfrefs.iter().any(Option::is_some),
            blob,
        });
    }
    Ok(groups)
}

/// Expose existing independent JXL atlases as scheduler jobs without changing
/// any JXL bytes, raster windows, or sprite IDs. Zero preserves the input job.
pub fn split_rle_jxl_chunk(
    chunk: SpriteRleJxlChunk,
    max_blobs: usize,
) -> Result<Vec<SpriteRleJxlChunk>> {
    ensure!(
        chunk.sprite_ids.len() == chunk.placements.len(),
        "{}: RLE placement count mismatch",
        chunk.rhs
    );
    ensure!(
        chunk
            .placements
            .iter()
            .all(|p| (p.blob as usize) < chunk.jxl_blobs.len()),
        "{}: RLE placement references an absent atlas",
        chunk.rhs
    );
    if max_blobs == 0 || chunk.jxl_blobs.len() <= max_blobs {
        return Ok(vec![chunk]);
    }
    let count = chunk.jxl_blobs.len().div_ceil(max_blobs);
    let mut groups: Vec<_> = (0..count)
        .map(|_| SpriteRleJxlChunk {
            rhs: chunk.rhs.clone(),
            jxl_blobs: Vec::new(),
            sprite_ids: Vec::new(),
            placements: Vec::new(),
        })
        .collect();
    for (index, blob) in chunk.jxl_blobs.into_iter().enumerate() {
        groups[index / max_blobs].jxl_blobs.push(blob);
    }
    for (id, mut placement) in chunk.sprite_ids.into_iter().zip(chunk.placements) {
        let group = placement.blob as usize / max_blobs;
        placement.blob = (placement.blob as usize % max_blobs) as u32;
        groups[group].sprite_ids.push(id);
        groups[group].placements.push(placement);
    }
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shipping_datadir::RleJxlPlacement;

    #[test]
    fn boundaries_preserve_oversized_grids_and_zero_budget() {
        let data = [vec![1; 3], vec![2; 12], vec![3; 2], vec![4; 3]];
        let grids: Vec<_> = data
            .iter()
            .map(|v| SpriteGrid {
                cols: 1,
                rows: v.len() as u16,
                indices: v,
            })
            .collect();
        assert_eq!(vq_group_ranges(&grids, 5).unwrap(), vec![0..1, 1..2, 2..4]);
        assert_eq!(vq_group_ranges(&grids, 0).unwrap(), vec![0..4]);
        assert!(
            vq_group_ranges(
                &[SpriteGrid {
                    cols: 2,
                    rows: 3,
                    indices: &[1]
                }],
                5
            )
            .is_err()
        );
    }

    #[test]
    fn independent_groups_decode_in_reverse_order_with_temporal_or_family_bases() {
        use crate::sprite_codec::decode_grids_shipping;
        use robin_engine::{
            coordinates::{SpriteAnchor, SpriteFrameOffset, SpriteSize},
            sprite_script::{SpriteInfo, SpriteScript},
        };
        use std::sync::Arc;
        let data: Vec<Vec<u16>> = (0..6)
            .map(|g| (0..32).map(|i| ((i + g * 3) % 16) as u16).collect())
            .collect();
        let grids: Vec<_> = data
            .iter()
            .map(|indices| SpriteGrid {
                cols: 4,
                rows: 8,
                indices,
            })
            .collect();
        let rhs = RhsData {
            signature: 1,
            profiles: vec![(
                "test".into(),
                SpriteInfo {
                    scripts: Arc::new(vec![SpriteScript {
                        frame_ids: (10..16).collect(),
                        offsets: vec![SpriteFrameOffset::ZERO; 6],
                        ..Default::default()
                    }]),
                    conversion: Arc::new(vec![]),
                    size: SpriteSize::new(16., 8.),
                    center: SpriteAnchor::ZERO,
                },
            )],
        };
        for family in [false, true] {
            let template = SpriteVqChunk {
                rhs: "test".into(),
                base_rhs: family.then(|| "base".into()),
                base2_rhs: if family {
                    "base2".into()
                } else {
                    String::new()
                },
                alphabet: 16,
                sprite_ids: (10..16).collect(),
                base_ids: (100..106).map(|id| family.then_some(id)).collect(),
                base2_ids: (200..206).map(|id| family.then_some(id)).collect(),
                self_refs: !family,
                blob: vec![],
            };
            let bases: Vec<_> = data
                .iter()
                .map(|d| family.then_some(d.as_slice()))
                .collect();
            let groups =
                encode_vq_groups(&template, &grids, &bases, &bases, Some(&rhs), 64).unwrap();
            assert_eq!(groups.len(), 3);
            for group in groups.iter().rev() {
                let start = (group.sprite_ids[0] - 10) as usize;
                let refs = if group.self_refs {
                    derive_chunk_self_refs(&rhs.profiles, &group.sprite_ids)
                } else {
                    vec![None; 2]
                };
                if !family {
                    assert!(refs[0].is_none());
                    assert_eq!(refs[1].unwrap().grid, 0);
                }
                let decoded = decode_grids_shipping(
                    16,
                    &[(4, 8); 2],
                    Some(&bases[start..start + 2]),
                    Some(&bases[start..start + 2]),
                    &refs,
                    &group.blob,
                )
                .unwrap();
                assert_eq!(decoded, data[start..start + 2]);
                assert_eq!(group.base_ids, template.base_ids[start..start + 2]);
                assert_eq!(group.base2_ids, template.base2_ids[start..start + 2]);
            }
            let mut malformed = template.clone();
            malformed.base_ids[0] = if family { None } else { Some(100) };
            assert!(encode_vq_groups(&malformed, &grids, &bases, &bases, Some(&rhs), 64).is_err());
        }
    }

    #[test]
    fn rle_groups_preserve_exact_blobs_and_placements() {
        let chunk = SpriteRleJxlChunk {
            rhs: "test".into(),
            jxl_blobs: vec![vec![1], vec![2], vec![3]],
            sprite_ids: vec![10, 11, 12],
            placements: vec![
                RleJxlPlacement {
                    blob: 2,
                    x: 4,
                    y: 5,
                },
                RleJxlPlacement {
                    blob: 0,
                    x: 6,
                    y: 7,
                },
                RleJxlPlacement {
                    blob: 2,
                    x: 8,
                    y: 9,
                },
            ],
        };
        let groups = split_rle_jxl_chunk(chunk, 1).unwrap();
        assert_eq!(groups[0].sprite_ids, vec![11]);
        assert_eq!(groups[2].sprite_ids, vec![10, 12]);
        assert_eq!(groups[2].jxl_blobs, vec![vec![3]]);
        assert_eq!(
            (
                groups[2].placements[1].blob,
                groups[2].placements[1].x,
                groups[2].placements[1].y
            ),
            (0, 8, 9)
        );
        let bad = SpriteRleJxlChunk {
            rhs: "bad".into(),
            jxl_blobs: vec![],
            sprite_ids: vec![1],
            placements: vec![RleJxlPlacement {
                blob: 0,
                x: 0,
                y: 0,
            }],
        };
        assert!(split_rle_jxl_chunk(bad, 0).is_err());
    }
}
