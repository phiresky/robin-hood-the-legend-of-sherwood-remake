//! Shipping codec boundary; payload wire shapes remain in the parent.
use super::*;

// Schema v10: star-2 family topology — [`SpriteVqChunk`] gains `base2_rhs` /
// `base2_ids`, letting third-and-later family members code each tile against
// TWO already-decoded siblings (schema v9 introduced the per-chunk
// `sprite_codec` blobs and single-base cross-variant coding). Both the boot
// manifest and the mission chunk layout changed, so both magics advance —
// bitcode is not self-describing, and a versioned magic mismatch is the only
// thing standing between an old binary and a misparse. (The datadir magic is
// spelled `RHDDNA10` because the tag is exactly 8 bytes; the u32 version
// beside it is the authoritative number.)
//
// Schema v11 / mission v6: identical container layout, but the VQ blobs are
// coded with PPM exclusion disabled (`EXCL_SOURCE_CAP = 0` in
// `sprite_codec`) — a decode-speed/size trade (~+1.6% rhs bytes for
// -35..43% decode time). The constant is part of the bitstream contract, so
// chunks encoded either way are mutually undecodable and both magics
// advance.
//
// Schema v12 / mission v7: binary escape coding in the VQ codec. The
// hit-vs-escape decision at each PPM chain level is now one LZMA-style
// adaptive bit (11-bit probability per SEE bucket) instead of SEE-priced
// escape mass folded into the coding interval — this removes both per-level
// divisions from the escape path (another decode-speed/size trade; the
// container layout is unchanged, but the entropy bitstream is incompatible,
// so both magics advance).
//
// Datadir v13: `ShippingAudioAsset` gains `bundle_offset` — small audio
// assets ship concatenated into one bundle per logical group instead of
// thousands of tiny standalone files. Mission chunk layout is unchanged
// (the audio catalog lives only in `datadir.bin`), so only the datadir
// magic advances.
//
// Schema v14 / mission v8: [`ShippingSpriteBank`] gains `rle_jxl_chunks` —
// lossy-JXL atlases plus lossless 2-bit class maps for RLE patch/ambient
// sprites, emitted only by the WEB recipe (`--rle-sprite-format jxl-q70`;
// the default keeps exact RLE words and native shipping stays
// byte-preserving). The mission chunk layout changed, so both magics
// advance.
//
// Datadir v15 adds `ShippingDatadir::locales`, carrying explicit,
// canonicalized multi-locale payloads. Mission payloads remain at v8 because
// locale data is confined to the boot datadir manifest.
// Datadir v16 adds ResourceData::picture_opacity for pixel-free engine setup.
// Mission payloads remain v8: they contain no ResourceManager values.
pub(super) const SHIPPING_DATADIR_MAGIC: [u8; 8] = *b"RHDDNA16";
pub(super) const SHIPPING_MISSION_MAGIC: [u8; 8] = *b"RHMISN08";
pub const SHIPPING_DATADIR_VERSION: u32 = 16;
pub const SHIPPING_MISSION_VERSION: u32 = 8;

/// Encode the versioned native-bitcode payload stored inside `datadir.bin`.
pub fn encode_native(datadir: &ShippingDatadir) -> Vec<u8> {
    let payload = bitcode::encode(datadir.payload());
    let mut encoded = Vec::with_capacity(12 + payload.len());
    encoded.extend_from_slice(&SHIPPING_DATADIR_MAGIC);
    encoded.extend_from_slice(&SHIPPING_DATADIR_VERSION.to_le_bytes());
    encoded.extend_from_slice(&payload);
    encoded
}

pub(super) fn decode_native(encoded: &[u8]) -> Result<ShippingDatadir> {
    let Some((header, payload)) = encoded.split_at_checked(12) else {
        return Err(anyhow!(
            "shipping datadir is shorter than its native header"
        ));
    };
    if header[..8] != SHIPPING_DATADIR_MAGIC {
        return Err(anyhow!(
            "shipping datadir is not native format version {SHIPPING_DATADIR_VERSION}; regenerate datadir.bin"
        ));
    }
    let version = u32::from_le_bytes(header[8..12].try_into().expect("fixed header length"));
    if version != SHIPPING_DATADIR_VERSION {
        return Err(anyhow!(
            "unsupported shipping datadir version {version}; expected {SHIPPING_DATADIR_VERSION}"
        ));
    }
    bitcode::decode(payload)
        .map(ShippingDatadir::from_payload)
        .map_err(|error| anyhow!("native bitcode decode: {error:?}"))
}

pub fn encode_mission_native(mission: &ShippingMission) -> Vec<u8> {
    let payload = bitcode::encode(&mission.payload);
    let mut encoded = Vec::with_capacity(12 + payload.len());
    encoded.extend_from_slice(&SHIPPING_MISSION_MAGIC);
    encoded.extend_from_slice(&SHIPPING_MISSION_VERSION.to_le_bytes());
    encoded.extend_from_slice(&payload);
    encoded
}

pub fn decode_mission_compressed(compressed: &[u8]) -> Result<ShippingMission> {
    let blob = zstd_decompress(compressed)?;
    let Some((header, payload)) = blob.split_at_checked(12) else {
        return Err(anyhow!(
            "shipping mission payload is shorter than its header"
        ));
    };
    if header[..8] != SHIPPING_MISSION_MAGIC {
        return Err(anyhow!("shipping mission payload has invalid magic"));
    }
    let version = u32::from_le_bytes(header[8..12].try_into().expect("fixed header length"));
    if version != SHIPPING_MISSION_VERSION {
        return Err(anyhow!(
            "unsupported shipping mission version {version}; expected {SHIPPING_MISSION_VERSION}"
        ));
    }
    bitcode::decode(payload)
        .map(ShippingMission::from_payload)
        .map_err(|error| anyhow!("native bitcode mission decode: {error:?}"))
}

/// Upper bound on one expanded shipping manifest/mission. This is separate
/// from the zstd window cap: small-window streams may expand without bound.
/// TODO: calibrate lower per-platform aggregate resident budgets with the
/// complete shipping corpus; this cap preserves large native packages.
pub const SHIPPING_EXPANDED_BYTE_LIMIT: usize = 1024 * 1024 * 1024;

pub(super) fn zstd_decompress(compressed: &[u8]) -> Result<Vec<u8>> {
    decompress_shipping_with_limit(compressed, SHIPPING_EXPANDED_BYTE_LIMIT)
}

/// Decode a bounded stream. The sentinel byte detects limit-plus-one without
/// ever allowing an attacker-controlled stream to grow the output unboundedly.
pub fn decompress_shipping_with_limit(compressed: &[u8], limit: usize) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut decoder = zstd::stream::read::Decoder::new(compressed).context("zstd decoder init")?;
    decoder
        .window_log_max(30)
        .context("zstd window_log_max=30")?;
    let cap = u64::try_from(limit)
        .context("shipping decode limit does not fit u64")?
        .checked_add(1)
        .ok_or_else(|| anyhow!("shipping decode limit overflow"))?;
    let mut blob = Vec::new();
    decoder
        .take(cap)
        .read_to_end(&mut blob)
        .context("zstd decompress")?;
    if blob.len() > limit {
        return Err(anyhow!("shipping expanded payload exceeds {limit} bytes"));
    }
    Ok(blob)
}

/// zstd level 22 with adaptive windows capped at the native 31-bit maximum.
pub fn zstd_max_compress(bytes: &[u8]) -> Result<Vec<u8>> {
    zstd_compress_with_window(bytes, 31)
}

/// zstd level 22 with an adaptive `windowLog` capped by the caller (10..=31).
/// Pledging the input size lets zstd advertise only the window this frame can
/// actually use. Split RHS chunks consequently require at most about 16 MiB
/// instead of claiming a 1 GiB wasm decoder window, with effectively neutral
/// compressed size.
pub fn zstd_compress_with_window(bytes: &[u8], max_window_log: u32) -> Result<Vec<u8>> {
    use zstd::stream::raw::CParameter;
    use zstd::stream::write::Encoder;
    if !(10..=31).contains(&max_window_log) {
        return Err(anyhow!(
            "zstd maximum window_log must be in 10..=31, got {max_window_log}"
        ));
    }
    let content_window_log = usize::BITS - bytes.len().saturating_sub(1).leading_zeros();
    let window_log = content_window_log.clamp(10, max_window_log);
    let mut out = Vec::new();
    let mut enc = Encoder::new(&mut out, 22).context("zstd encoder")?;
    enc.set_pledged_src_size(Some(bytes.len() as u64))
        .context("zstd pledged source size")?;
    enc.set_parameter(CParameter::WindowLog(window_log))
        .with_context(|| format!("zstd window_log={window_log}"))?;
    enc.set_parameter(CParameter::EnableLongDistanceMatching(true))
        .context("zstd long=1")?;
    std::io::Write::write_all(&mut enc, bytes).context("zstd write")?;
    enc.finish().context("zstd finish")?;
    Ok(out)
}
