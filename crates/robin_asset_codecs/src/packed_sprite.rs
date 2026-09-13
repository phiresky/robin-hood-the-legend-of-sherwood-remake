//! Checked packed sprite boundaries shared by raster and opacity consumers.
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::ops::Range;

/// Metadata for a checked RLE row; literal words remain in the original bank.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RleRow {
    pub first: usize,
    pub literals: Range<usize>,
}

/// Parse one row without allocating pixel storage. Empty rows retain the
/// legacy 0/FFFF sentinels; shifted empty rows are rejected, never normalized.
pub fn read_rle_row(packed: &[u16], position: &mut usize, width: usize) -> Result<RleRow> {
    let header_end = position
        .checked_add(2)
        .ok_or_else(|| anyhow!("RLE offset overflow"))?;
    let header = packed
        .get(*position..header_end)
        .ok_or_else(|| anyhow!("RLE truncated control words"))?;
    let (first, last) = (header[0], header[1]);
    *position = header_end;
    if last == u16::MAX {
        if first != 0 && first != u16::MAX {
            bail!("RLE empty row with nonzero first={first}");
        }
        return Ok(RleRow {
            first: 0,
            literals: *position..*position,
        });
    }
    let (first, last) = (usize::from(first), usize::from(last));
    if first > last || last >= width {
        bail!("RLE bad run {first}..={last} in width {width}");
    }
    let end = position
        .checked_add(last + 1 - first)
        .ok_or_else(|| anyhow!("RLE literal offset overflow"))?;
    if end > packed.len() {
        bail!("RLE truncated literals");
    }
    let row = RleRow {
        first,
        literals: *position..end,
    };
    *position = end;
    Ok(row)
}

pub const SPRITE_PIXEL_LIMIT: usize = 64 * 1024 * 1024;

pub fn pixel_count(width: usize, height: usize) -> Result<usize> {
    let count = width
        .checked_mul(height)
        .ok_or_else(|| anyhow!("sprite dimensions overflow"))?;
    if count > SPRITE_PIXEL_LIMIT {
        bail!("sprite exceeds {SPRITE_PIXEL_LIMIT} pixels");
    }
    Ok(count)
}

pub fn validate_rle(packed: &[u16], width: usize, height: usize) -> Result<usize> {
    pixel_count(width, height)?;
    let mut position = 0;
    for _ in 0..height {
        read_rle_row(packed, &mut position, width)?;
    }
    // Trailing words are retained for exact legacy bank round trips.
    Ok(position)
}

pub fn validate_vq(
    packed: &[u16],
    width: usize,
    height: usize,
    dictionary_entries: usize,
) -> Result<()> {
    let pixels = pixel_count(width, height)?;
    if !width.is_multiple_of(4) {
        bail!("VQ sprite width {width} is not a multiple of four");
    }
    let words = pixels / 4;
    let grid = packed
        .get(..words)
        .ok_or_else(|| anyhow!("VQ grid needs {words} words, has {}", packed.len()))?;
    if let Some(index) = grid
        .iter()
        .find(|&&index| usize::from(index) >= dictionary_entries)
    {
        bail!("VQ dictionary index {index} exceeds {dictionary_entries} entries");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_storage_and_dimensions_fail_before_allocation() {
        for words in [
            &[][..],
            &[0],
            &[0, 2, 1],
            &[3, 1],
            &[0, 4, 1, 2, 3, 4, 5],
            &[1, u16::MAX],
        ] {
            assert!(validate_rle(words, 4, 1).is_err(), "{words:?}");
        }
        assert!(pixel_count(usize::MAX, 2).is_err());
        assert!(pixel_count(SPRITE_PIXEL_LIMIT + 1, 1).is_err());
        assert!(validate_vq(&[], 4, 1, 1).is_err());
        assert!(validate_vq(&[1], 4, 1, 1).is_err());
        assert!(validate_vq(&[0], 3, 1, 1).is_err());
        validate_vq(&[0], 4, 1, 1).unwrap();
    }
}
