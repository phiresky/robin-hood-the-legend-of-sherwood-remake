//! Original `.res` decoding, independent of file authority and runtime cache state.

use anyhow::{Context, Result, bail};

use super::{MouseEntry, ResourceData, ResourceFileEntry, ResourceId, ResourceLifetime};
use crate::binary_reader::Reader;
use crate::picture::Picture;
use robin_engine::coordinates::CursorHotspot;

pub(super) const RES_VERSION_100: u32 = 0x0100;

// ---------------------------------------------------------------------------
// Free reader functions — parse resource payloads from a checked byte reader
// ---------------------------------------------------------------------------

fn read_picture(reader: &mut Reader<'_>, context: &str) -> Result<Picture> {
    // The original game reads the 12-byte
    // header and then exactly `ulPackedSize` payload bytes.
    let start = reader.position();
    let header: [u8; 12] = reader
        .take(12, format!("{context} Sixteen header"))?
        .try_into()
        .expect("the checked reader returned exactly 12 bytes");
    let packed_size = u32::from_le_bytes([header[8], header[9], header[10], header[11]]) as usize;
    reader.take(packed_size, format!("{context} Sixteen payload"))?;
    let length = reader.position() - start;
    let bytes = reader.range(start, length, format!("{context} Sixteen frame"))?;
    Picture::load_original_sixteen_from_bytes(bytes)
        .with_context(|| format!("{context} Sixteen frame"))
}

/// Read a single-picture resource (`PIC `).
fn read_single_picture(reader: &mut Reader<'_>, context: &str) -> Result<Vec<Option<Picture>>> {
    let _flags = reader.u32(format!("{context} flags"))?;
    let pic = read_picture(reader, &format!("{context} picture 0"))?;
    Ok(vec![Some(pic)])
}

/// Read a picture-collection resource (`PICC`).
pub(super) fn read_picture_collection(
    reader: &mut Reader<'_>,
    context: &str,
) -> Result<Vec<Option<Picture>>> {
    let _flags = reader.u32(format!("{context} flags"))?;
    read_picture_slots(reader, context)
}

fn read_picture_slots(reader: &mut Reader<'_>, context: &str) -> Result<Vec<Option<Picture>>> {
    let count = reader.count_u32(format!("{context} picture count"), 12)?;
    let mut pics = Vec::with_capacity(count);
    for picture_index in 0..count {
        pics.push(Some(read_picture(
            reader,
            &format!("{context} picture {picture_index}"),
        )?));
    }
    Ok(pics)
}

pub(super) fn flagged_picture_count(tag: &[u8; 4]) -> Option<usize> {
    match tag {
        b"BTTN" => Some(4),
        b"TOGL" => Some(5),
        b"NPTF" | b"SLID" => Some(6),
        b"RDO " => Some(7),
        _ => None,
    }
}

/// Read a "flagged" picture resource (BTTN, TOGL, NPTF, SLID, RDO).
/// `count` is the fixed number of sub-pictures for this widget type.
/// A bitmask controls which sub-pictures are actually present in the stream.
fn read_flagged_pictures(
    reader: &mut Reader<'_>,
    count: usize,
    context: &str,
) -> Result<Vec<Option<Picture>>> {
    let _flags = reader.u32(format!("{context} flags"))?;
    let bitmask = reader.u32(format!("{context} picture bitmask"))?;
    let mut pics = Vec::with_capacity(count);
    for i in 0..count {
        if bitmask & (1 << i) != 0 {
            pics.push(Some(read_picture(
                reader,
                &format!("{context} picture {i}"),
            )?));
        } else {
            pics.push(None);
        }
    }
    Ok(pics)
}

/// Read a cursor resource (`CUR `).
pub(super) fn read_cursor(
    reader: &mut Reader<'_>,
    context: &str,
) -> Result<(MouseEntry, Vec<Option<Picture>>)> {
    let _flags = reader.u32(format!("{context} flags"))?;
    let mouse_flags = reader.u16(format!("{context} mouse flags"))?;
    let x = reader.u16(format!("{context} hotspot x"))?;
    let y = reader.u16(format!("{context} hotspot y"))?;
    let frame_length = reader.u16(format!("{context} frame length"))?;
    let pics = read_picture_slots(reader, context)?;

    let entry = MouseEntry {
        hotspot: CursorHotspot::new(x as f32, y as f32),
        flags: mouse_flags,
        frame_length,
    };
    Ok((entry, pics))
}

/// Read a string-table resource (`TEXT`).
/// Strings are little-endian UTF-16 on disk; we convert to UTF-8.
pub(super) fn read_string_table(reader: &mut Reader<'_>, context: &str) -> Result<Vec<String>> {
    let _flags = reader.u32(format!("{context} flags"))?;
    let count = reader.u16(format!("{context} string count"))? as usize;
    reader.validate_count(
        count,
        2,
        format!("{context} string count"),
        reader.position() - 2,
    )?;
    let mut strings = Vec::with_capacity(count);

    // The original game stores each TEXT entry as
    // a 16-bit count followed by that many 16-bit code units.
    for string_index in 0..count {
        let char_count = reader.u16(format!("{context} string {string_index} length"))? as usize;
        reader.validate_count(
            char_count,
            2,
            format!("{context} string {string_index} UTF-16 data"),
            reader.position() - 2,
        )?;
        let encoded = reader.take(
            char_count * 2,
            format!("{context} string {string_index} UTF-16 data"),
        )?;
        let code_units = encoded
            .chunks_exact(2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]));
        strings.push(
            char::decode_utf16(code_units)
                .collect::<std::result::Result<String, _>>()
                .with_context(|| format!("{context} string {string_index}: invalid UTF-16"))?,
        );
    }
    Ok(strings)
}

/// Read a wave-table resource (`WAVE`).
/// Entries are narrow (ASCII) path strings on disk.
fn read_wave_table(reader: &mut Reader<'_>, context: &str) -> Result<Vec<String>> {
    let _flags = reader.u32(format!("{context} flags"))?;
    let count = reader.u16(format!("{context} wave count"))? as usize;
    reader.validate_count(
        count,
        2,
        format!("{context} wave count"),
        reader.position() - 2,
    )?;
    let mut waves = Vec::with_capacity(count);

    for wave_index in 0..count {
        let str_size = reader.u16(format!("{context} wave {wave_index} length"))? as usize;
        let encoded = reader.take(str_size, format!("{context} wave {wave_index} path"))?;
        // Original-game wave-table loading caps the materialized path at 4096 bytes
        // while still advancing past the full declared range.
        let buf = &encoded[..str_size.min(4096)];
        if str_size > 4096 {
            tracing::warn!("read_wave_table: string size {str_size} > 4096, truncating");
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        waves.push(String::from_utf8_lossy(&buf[..end]).to_string());
    }
    Ok(waves)
}

pub(super) fn parse(bytes: &[u8], path: &str) -> Result<(ResourceData, ResourceLifetime)> {
    let mut reader = Reader::new(bytes);

    // Validate magic
    let magic = reader.take_array::<4>("resource file magic")?;
    if &magic != b"SRES" {
        bail!(
            "not a resource file (bad magic {:?})",
            std::str::from_utf8(&magic).unwrap_or("????")
        );
    }

    let version = reader.u32("resource file version")?;

    let mut data = ResourceData::default();
    let mut lifetime = ResourceLifetime::default();
    match version {
        RES_VERSION_100 => load_file_resource_v100(&mut reader, path, &mut data, &mut lifetime)?,
        _ => bail!("unsupported resource file version: 0x{version:04X}"),
    }
    Ok((data, lifetime))
}

fn load_file_resource_v100(
    reader: &mut Reader<'_>,
    file_path: &str,
    data: &mut ResourceData,
    lifetime: &mut ResourceLifetime,
) -> Result<()> {
    let num_resources = reader.count_u32("resource file entry count", 8)?;

    for resource_index in 0..num_resources {
        let type_tag = reader.take_array::<4>(format!("resource {resource_index} type"))?;
        let id = reader.u32(format!("resource {resource_index} id"))? as ResourceId;
        let context = format!(
            "resource {id} ({})",
            std::str::from_utf8(&type_tag).unwrap_or("non-ASCII type")
        );

        // Record the payload start used to recover dismissed resources.
        let offset = u64::try_from(reader.position())
            .with_context(|| format!("{context}: payload offset does not fit u64"))?;
        decode_resource(data, reader, id, &type_tag).with_context(|| context.clone())?;

        lifetime.references.insert(id, 0);
        lifetime.file_entries.insert(
            id,
            ResourceFileEntry {
                file_path: file_path.to_string(),
                file_offset: offset,
                resource_type: type_tag,
            },
        );
    }
    Ok(())
}

/// Dispatch to the right reader based on the 4-byte type tag and store
/// the results in the appropriate map(s).
pub(super) fn decode_resource(
    data: &mut ResourceData,
    reader: &mut Reader<'_>,
    id: ResourceId,
    type_tag: &[u8; 4],
) -> Result<()> {
    let context = format!(
        "resource {id} ({})",
        std::str::from_utf8(type_tag).unwrap_or("non-ASCII type")
    );
    match type_tag {
        b"PIC " => {
            let pics = read_single_picture(reader, &context)?;
            data.remove(id);
            data.pictures.insert(id, pics);
        }
        b"PICC" => {
            let pics = read_picture_collection(reader, &context)?;
            data.remove(id);
            data.pictures.insert(id, pics);
        }
        b"BTTN" | b"TOGL" | b"NPTF" | b"SLID" | b"RDO " => {
            let count = flagged_picture_count(type_tag).expect("matched flagged picture tag");
            let pics = read_flagged_pictures(reader, count, &context)?;
            data.remove(id);
            data.pictures.insert(id, pics);
        }
        b"CUR " => {
            let (mouse, pics) = read_cursor(reader, &context)?;
            data.remove(id);
            data.pictures.insert(id, pics);
            data.mouse_entries.insert(id, mouse);
        }
        b"TEXT" => {
            let strs = read_string_table(reader, &context)?;
            data.remove(id);
            data.strings.insert(id, strs);
        }
        b"WAVE" => {
            let w = read_wave_table(reader, &context)?;
            data.remove(id);
            data.waves.insert(id, w);
        }
        _ => bail!(
            "unsupported resource type: {:?}",
            std::str::from_utf8(type_tag).unwrap_or("????")
        ),
    }
    Ok(())
}
