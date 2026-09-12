//! Shared positional primitives for the audited Original save grammar.

use super::payload_base::{LegacyBoundingBox2, LegacyPoint2, LegacyPoint3};
use crate::legacy_io::{LegacyReader, LegacyResult};

// Keep each section's independently configurable ceiling while naming the
// two shared defaults. Deriving Default would silently replace these with zero.
pub(super) const DEFAULT_LIST_LIMIT: usize = 4096;
pub(super) const DEFAULT_BULK_LIMIT: usize = 65_535;

pub(super) const fn hex16(value: &str) -> [u8; 16] {
    let bytes = value.as_bytes();
    let mut result = [0; 16];
    let mut index = 0;
    while index < 16 {
        result[index] = (hex_nibble(bytes[index * 2]) << 4) | hex_nibble(bytes[index * 2 + 1]);
        index += 1;
    }
    result
}

pub(super) const fn hex_nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => panic!("invalid fingerprint hex"),
    }
}

pub(super) fn read_point2(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacyPoint2> {
    reader.scope(field.to_string(), |reader| {
        Ok(LegacyPoint2 {
            x: reader.read_f32("x")?,
            y: reader.read_f32("y")?,
        })
    })
}

pub(super) fn read_point3(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacyPoint3> {
    reader.scope(field.to_string(), |reader| {
        Ok(LegacyPoint3 {
            x: reader.read_f32("x")?,
            y: reader.read_f32("y")?,
            z: reader.read_f32("z")?,
        })
    })
}

pub(super) fn read_box2(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
) -> LegacyResult<LegacyBoundingBox2> {
    reader.scope(field.to_string(), |reader| {
        Ok(LegacyBoundingBox2 {
            top_left: read_point2(reader, "top_left")?,
            bottom_right: read_point2(reader, "bottom_right")?,
            bounds_are_set: reader.read_bool("bounds_are_set")?,
        })
    })
}

pub(super) fn reserve<T>(
    reader: &mut LegacyReader<'_>,
    values: &mut Vec<T>,
    count: usize,
    field: impl std::fmt::Display,
) -> LegacyResult<()> {
    let offset = reader.offset();
    values
        .try_reserve_exact(count)
        .map_err(|_| reader.allocation_error(offset, field, count))
}

pub(super) fn reserved<T>(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display,
    count: usize,
) -> LegacyResult<Vec<T>> {
    let mut values = Vec::new();
    reserve(reader, &mut values, count, field)?;
    Ok(values)
}

pub(super) fn read_array<const N: usize>(
    reader: &mut LegacyReader<'_>,
    field: &'static str,
) -> LegacyResult<[u8; N]> {
    let mut bytes = [0; N];
    reader.read_bytes(field, &mut bytes)?;
    Ok(bytes)
}

pub(super) fn read_count_u16(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display + Copy,
    maximum: usize,
) -> LegacyResult<usize> {
    let offset = reader.offset();
    let raw = reader.read_u16(field)?;
    let count = usize::from(raw);
    if count > maximum {
        return Err(reader.invalid_value(
            offset,
            field,
            count,
            "item count within the caller-supplied limit",
        ));
    }
    Ok(count)
}
