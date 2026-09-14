//! Little-endian readers over in-memory byte slices of original-game formats.
//!
//! `robin_data_io::legacy_io::LegacyReader` covers streamed `SbFile` data;
//! these helpers cover formats the engine already holds as `&[u8]` (proto
//! streams, sound banks, mobile macros). Every read is bounds-checked and
//! reports a typed [`TruncatedRead`]; each format decides whether that is an
//! error or an authored-data invariant panic.
// TODO: if `robin_data_io` grows a slice cursor, route these through it.

/// A read that ran past the end of the byte slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("unexpected end of data reading {kind} at offset {offset} (data length {len})")]
pub struct TruncatedRead {
    pub kind: &'static str,
    pub offset: usize,
    pub len: usize,
}

impl From<TruncatedRead> for String {
    fn from(error: TruncatedRead) -> Self {
        error.to_string()
    }
}

/// Copy the `N` bytes at `offset` (fixed-size name/tag fields); `kind`
/// labels the field in the [`TruncatedRead`] diagnostic.
pub fn array_at<const N: usize>(
    data: &[u8],
    offset: usize,
    kind: &'static str,
) -> Result<[u8; N], TruncatedRead> {
    offset
        .checked_add(N)
        .and_then(|end| data.get(offset..end))
        .map(|bytes| bytes.try_into().expect("slice length equals N"))
        .ok_or(TruncatedRead {
            kind,
            offset,
            len: data.len(),
        })
}

pub fn u8_at(data: &[u8], offset: usize) -> Result<u8, TruncatedRead> {
    array_at::<1>(data, offset, "u8").map(|[byte]| byte)
}

pub fn u16_at(data: &[u8], offset: usize) -> Result<u16, TruncatedRead> {
    array_at(data, offset, "u16").map(u16::from_le_bytes)
}

pub fn i16_at(data: &[u8], offset: usize) -> Result<i16, TruncatedRead> {
    array_at(data, offset, "i16").map(i16::from_le_bytes)
}

pub fn u32_at(data: &[u8], offset: usize) -> Result<u32, TruncatedRead> {
    array_at(data, offset, "u32").map(u32::from_le_bytes)
}

pub fn f32_at(data: &[u8], offset: usize) -> Result<f32, TruncatedRead> {
    array_at(data, offset, "f32").map(f32::from_le_bytes)
}

/// Read at `*pos` and advance it only on success.
fn advance<T, const N: usize>(
    pos: &mut usize,
    read: impl FnOnce(usize) -> Result<T, TruncatedRead>,
) -> Result<T, TruncatedRead> {
    let value = read(*pos)?;
    *pos += N;
    Ok(value)
}

pub fn read_u8(data: &[u8], pos: &mut usize) -> Result<u8, TruncatedRead> {
    advance::<_, 1>(pos, |offset| u8_at(data, offset))
}

pub fn read_u16(data: &[u8], pos: &mut usize) -> Result<u16, TruncatedRead> {
    advance::<_, 2>(pos, |offset| u16_at(data, offset))
}

pub fn read_i16(data: &[u8], pos: &mut usize) -> Result<i16, TruncatedRead> {
    advance::<_, 2>(pos, |offset| i16_at(data, offset))
}

pub fn read_u32(data: &[u8], pos: &mut usize) -> Result<u32, TruncatedRead> {
    advance::<_, 4>(pos, |offset| u32_at(data, offset))
}

pub fn read_f32(data: &[u8], pos: &mut usize) -> Result<f32, TruncatedRead> {
    advance::<_, 4>(pos, |offset| f32_at(data, offset))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_little_endian_and_advances() {
        let data = [0x01, 0x02, 0x03, 0x04, 0x05];
        let mut pos = 0;
        assert_eq!(read_u8(&data, &mut pos), Ok(0x01));
        assert_eq!(read_u16(&data, &mut pos), Ok(0x0302));
        assert_eq!(pos, 3);
        assert_eq!(u32_at(&data, 1), Ok(0x0504_0302));
        assert_eq!(i16_at(&[0xff, 0xff], 0), Ok(-1));
        assert_eq!(f32_at(&1.5f32.to_le_bytes(), 0), Ok(1.5));
    }

    #[test]
    fn truncated_reads_report_offset_and_do_not_advance() {
        let data = [0u8; 3];
        let mut pos = 1;
        assert_eq!(
            read_u32(&data, &mut pos),
            Err(TruncatedRead {
                kind: "u32",
                offset: 1,
                len: 3
            })
        );
        assert_eq!(pos, 1);
        assert!(u16_at(&data, usize::MAX).is_err());
    }
}
