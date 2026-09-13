//! Shared positional primitives for the audited Original save grammar.

use super::payload_base::{LegacyBoundingBox2, LegacyPoint2, LegacyPoint3};
use crate::legacy_io::{LegacyContext, LegacyRead, LegacyReader, LegacyResult};

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
    field: impl Into<LegacyContext>,
) -> LegacyResult<LegacyPoint2> {
    LegacyPoint2::read_field(reader, field, &())
}

pub(super) fn read_point3(
    reader: &mut LegacyReader<'_>,
    field: impl Into<LegacyContext>,
) -> LegacyResult<LegacyPoint3> {
    LegacyPoint3::read_field(reader, field, &())
}

pub(super) fn read_box2(
    reader: &mut LegacyReader<'_>,
    field: impl Into<LegacyContext>,
) -> LegacyResult<LegacyBoundingBox2> {
    LegacyBoundingBox2::read_field(reader, field, &())
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

#[cfg(test)]
mod legacy_read_derive_tests {
    //! `#[derive(LegacyRead)]` must reproduce the hand-written reads it
    //! replaced exactly: byte order, error offsets and error field paths.

    use super::super::payload_base::LegacyElementRef;
    use super::super::test_support::{push_f32, push_u16, push_u32, with_reader};
    use crate::legacy_io::{LegacyIoError, LegacyRead, LegacyReader, LegacyResult};

    use super::{LegacyPoint2, hex16, read_count_u16};

    const FINGERPRINT: [u8; 16] = hex16("00112233445566778899aabbccddeeff");

    #[derive(Debug, PartialEq, LegacyRead)]
    struct Inner {
        a: u16,
        point: LegacyPoint2,
    }

    #[derive(Debug, PartialEq, LegacyRead)]
    #[legacy(ctx = usize, fingerprint = FINGERPRINT, expected = "test fingerprint")]
    struct Outer {
        #[legacy(offset)]
        start: u64,
        flag: bool,
        #[legacy(name = "wire_value")]
        value: u32,
        #[legacy(bytes)]
        padding: [u8; 2],
        pair: [u16; 2],
        #[legacy(scoped)]
        scoped_pair: [u16; 2],
        #[legacy(count_u32 = *ctx)]
        refs: Vec<LegacyElementRef>,
        #[legacy(count_u16 = *ctx, items)]
        inners: Vec<Inner>,
        #[legacy(when = flag, flatten)]
        flattened: Option<Inner>,
        #[legacy(value = start + 1)]
        derived: u64,
    }

    /// The hand-written equivalent of `Outer`'s derive. A struct-level
    /// fingerprint precedes every field, including `offset` fields.
    fn read_outer_by_hand(reader: &mut LegacyReader<'_>, maximum: usize) -> LegacyResult<Outer> {
        reader.read_signature("fingerprint", FINGERPRINT, "test fingerprint")?;
        let start = reader.offset();
        let flag = reader.read_bool("flag")?;
        let value = reader.read_u32("wire_value")?;
        let mut padding = [0; 2];
        reader.read_bytes("padding", &mut padding)?;
        let pair = [reader.read_u16("pair[0]")?, reader.read_u16("pair[1]")?];
        let scoped_pair = reader.scope("scoped_pair", |reader| {
            Ok([reader.read_u16("[0]")?, reader.read_u16("[1]")?])
        })?;
        let count = reader.read_count_u32("refs.count", maximum)?;
        let mut refs = Vec::new();
        super::reserve(reader, &mut refs, count, "refs")?;
        for index in 0..count {
            let raw = reader.read_u32(format!("refs[{index}]"))?;
            refs.push(LegacyElementRef((raw != u32::MAX).then_some(raw)));
        }
        let inners = reader.scope("inners", |reader| {
            let count = read_count_u16(reader, "count", maximum)?;
            let mut inners = Vec::new();
            super::reserve(reader, &mut inners, count, "items")?;
            for index in 0..count {
                inners.push(reader.scope_indexed("items", index, read_inner_by_hand)?);
            }
            Ok(inners)
        })?;
        let flattened = if flag {
            Some(read_inner_by_hand(reader)?)
        } else {
            None
        };
        Ok(Outer {
            start,
            flag,
            value,
            padding,
            pair,
            scoped_pair,
            refs,
            inners,
            flattened,
            derived: start + 1,
        })
    }

    fn read_inner_by_hand(reader: &mut LegacyReader<'_>) -> LegacyResult<Inner> {
        Ok(Inner {
            a: reader.read_u16("a")?,
            point: reader.scope("point", |reader| {
                Ok(LegacyPoint2 {
                    x: reader.read_f32("x")?,
                    y: reader.read_f32("y")?,
                })
            })?,
        })
    }

    fn push_inner(bytes: &mut Vec<u8>, seed: u16) {
        push_u16(bytes, seed);
        push_f32(bytes, f32::from(seed) + 0.5);
        push_f32(bytes, -f32::from(seed));
    }

    fn outer_bytes() -> Vec<u8> {
        let mut bytes = FINGERPRINT.to_vec();
        bytes.push(1);
        push_u32(&mut bytes, 0xdead_beef);
        bytes.extend_from_slice(&[0xaa, 0x55]);
        push_u16(&mut bytes, 3);
        push_u16(&mut bytes, 4);
        push_u16(&mut bytes, 5);
        push_u16(&mut bytes, 6);
        push_u32(&mut bytes, 2);
        push_u32(&mut bytes, 17);
        push_u32(&mut bytes, u32::MAX);
        push_u16(&mut bytes, 2);
        push_inner(&mut bytes, 7);
        push_inner(&mut bytes, 8);
        push_inner(&mut bytes, 9);
        bytes
    }

    fn error_site(error: LegacyIoError) -> (u64, String, String) {
        (error.offset, error.field, error.kind.to_string())
    }

    #[test]
    fn derive_reads_fields_in_declaration_order() {
        let bytes = outer_bytes();
        let decoded = with_reader(&bytes, |reader| {
            reader
                .scope("outer", |reader| Outer::read(reader, &8))
                .unwrap()
        });
        assert_eq!(decoded.start, 16);
        assert!(decoded.flag);
        assert_eq!(decoded.value, 0xdead_beef);
        assert_eq!(decoded.padding, [0xaa, 0x55]);
        assert_eq!(decoded.pair, [3, 4]);
        assert_eq!(decoded.scoped_pair, [5, 6]);
        assert_eq!(
            decoded.refs,
            [LegacyElementRef(Some(17)), LegacyElementRef(None)]
        );
        assert_eq!(decoded.inners.len(), 2);
        assert_eq!(decoded.inners[1].a, 8);
        assert_eq!(decoded.inners[1].point, LegacyPoint2 { x: 8.5, y: -8.0 });
        assert_eq!(decoded.flattened.as_ref().unwrap().a, 9);
        assert_eq!(decoded.derived, 17);
        let by_hand = with_reader(&bytes, |reader| read_outer_by_hand(reader, 8).unwrap());
        assert_eq!(decoded, by_hand);
    }

    #[test]
    fn derive_errors_match_hand_written_reads_at_every_truncation() {
        let bytes = outer_bytes();
        for length in 0..bytes.len() {
            let truncated = &bytes[..length];
            let derived = with_reader(truncated, |reader| {
                reader.scope("outer", |reader| Outer::read_field(reader, "body", &8))
            })
            .unwrap_err();
            let by_hand = with_reader(truncated, |reader| {
                reader.scope("outer", |reader| {
                    reader.scope("body", |reader| read_outer_by_hand(reader, 8))
                })
            })
            .unwrap_err();
            assert_eq!(
                error_site(derived),
                error_site(by_hand),
                "truncated to {length} bytes"
            );
        }
    }

    #[test]
    fn derive_reports_count_limits_and_fingerprints_like_hand_written_reads() {
        let bytes = outer_bytes();
        for maximum in [0, 1] {
            let derived = with_reader(&bytes, |reader| Outer::read(reader, &maximum)).unwrap_err();
            let by_hand =
                with_reader(&bytes, |reader| read_outer_by_hand(reader, maximum)).unwrap_err();
            assert_eq!(error_site(derived), error_site(by_hand));
        }
        let mut bad_fingerprint = outer_bytes();
        bad_fingerprint[5] ^= 0xff;
        let derived = with_reader(&bad_fingerprint, |reader| Outer::read(reader, &8)).unwrap_err();
        assert_eq!(derived.field, "fingerprint");
        assert!(derived.to_string().contains("test fingerprint"));
    }

    #[test]
    fn indexed_array_and_list_elements_render_like_formatted_names() {
        let error = with_reader(&[1, 0, 2], |reader| {
            <[u16; 2]>::read_field(reader, "values", &())
        })
        .unwrap_err();
        assert_eq!((error.offset, error.field.as_str()), (2, "values[1]"));
        let error = with_reader(&[1, 0, 2], |reader| {
            <[u16; 2]>::read_field(reader, String::from("owned"), &())
        })
        .unwrap_err();
        assert_eq!(error.field, "owned[1]");
    }
}

pub(super) fn read_count_u16(
    reader: &mut LegacyReader<'_>,
    field: impl std::fmt::Display + Copy,
    maximum: usize,
) -> LegacyResult<usize> {
    reader.read_count_u16(field, maximum)
}
