//! Admission policy for authored portable sprite documents, not disposable caches.
use anyhow::{Context, Result, ensure};
use std::{io::Read, path::Path};

// Match the existing shipping document/resident ceilings rather than applying
// the smaller disposable-cache policy to authored content. The level-19 single
// writer uses an 8 MiB window and the level-22 family writer uses 128 MiB.
// TODO: calibrate tighter platform-specific limits against a complete custom
// asset corpus. The local datadirs do not contain portable VQ bundle fixtures.
const DOCUMENT_BYTES: usize = 1024 * 1024 * 1024;
const WINDOW_LOG: u32 = 27;
const RESIDENT_BYTES: usize = 1024 * 1024 * 1024;

fn read_limited(reader: impl Read, limit: usize) -> Result<Vec<u8>> {
    let cap = u64::try_from(limit)?
        .checked_add(1)
        .context("sprite byte limit overflow")?;
    let mut bytes = Vec::new();
    reader.take(cap).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "authored sprite document exceeds {limit} bytes"
    );
    Ok(bytes)
}

pub(super) fn read_compressed(path: &Path) -> Result<Vec<u8>> {
    read_limited(std::fs::File::open(path)?, DOCUMENT_BYTES)
        .with_context(|| format!("reading authored sprites {}", path.display()))
}

pub(super) fn decompress(compressed: &[u8]) -> Result<Vec<u8>> {
    decompress_limited(compressed, DOCUMENT_BYTES, WINDOW_LOG)
}

fn decompress_limited(compressed: &[u8], limit: usize, window_log: u32) -> Result<Vec<u8>> {
    ensure!(
        compressed.len() <= DOCUMENT_BYTES,
        "compressed authored sprite document exceeds {DOCUMENT_BYTES} bytes"
    );
    let mut decoder = zstd::stream::read::Decoder::new(compressed)?;
    decoder.window_log_max(window_log)?;
    read_limited(decoder, limit).context("decompressing authored sprites")
}

/// Charge worst-case RLE output, VQ grids and per-frame allocation metadata
/// before decoding any groups. Per-group tile limits alone do not bound a
/// document containing arbitrarily many valid groups (including empty frames).
pub(super) fn validate_frames(sizes: impl IntoIterator<Item = (u16, u16)>) -> Result<()> {
    validate_frames_with_limit(sizes, RESIDENT_BYTES)
}

fn validate_frames_with_limit(
    sizes: impl IntoIterator<Item = (u16, u16)>,
    limit: usize,
) -> Result<()> {
    let mut total = 0usize;
    for (width, height) in sizes {
        let width = usize::from(width);
        let height = usize::from(height);
        crate::packed_sprite::pixel_count(width, height)?;
        let bytes = (width + 2)
            .checked_mul(height)
            .and_then(|n| n.checked_mul(2))
            .and_then(|n| n.checked_add(width.div_ceil(4) * height * 2))
            .and_then(|n| {
                n.checked_add(std::mem::size_of::<super::assets_frame_holder::RuntimeSprite>())
            })
            .context("authored sprite resident byte count overflow")?;
        total = total
            .checked_add(bytes)
            .context("authored sprite aggregate byte count overflow")?;
        ensure!(
            total <= limit,
            "authored sprite frames exceed {limit} estimated resident bytes"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompression_accepts_exact_limit_and_rejects_one_more() {
        let encoded = zstd::stream::encode_all(&b"12345678"[..], 1).unwrap();
        assert_eq!(
            decompress_limited(&encoded, 8, WINDOW_LOG).unwrap(),
            b"12345678"
        );
        assert!(decompress_limited(&encoded, 7, WINDOW_LOG).is_err());
        let mut concatenated = encoded.clone();
        concatenated.extend(encoded);
        assert!(decompress_limited(&concatenated, 8, WINDOW_LOG).is_err());
    }

    #[test]
    fn window_limit_rejects_stream_before_expansion() {
        let encoded = zstd::stream::encode_all(&vec![1; 16384][..], 1).unwrap();
        assert!(decompress_limited(&encoded, 16384, 10).is_err());
        assert_eq!(
            decompress_limited(&encoded, 16384, WINDOW_LOG)
                .unwrap()
                .len(),
            16384
        );
    }

    #[test]
    fn aggregate_budget_charges_empty_frames_and_rle_output() {
        let frame_bytes = std::mem::size_of::<super::super::assets_frame_holder::RuntimeSprite>();
        assert!(validate_frames_with_limit([(0, 0)], frame_bytes).is_ok());
        assert!(validate_frames_with_limit([(0, 0), (0, 0)], frame_bytes).is_err());
        let bytes = frame_bytes + (5 + 2) * 2 + 2 * 2;
        assert!(validate_frames_with_limit([(5, 1)], bytes).is_ok());
        assert!(validate_frames_with_limit([(5, 1)], bytes - 1).is_err());
    }
}
