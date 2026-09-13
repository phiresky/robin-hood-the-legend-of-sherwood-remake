//! Bounds-checked little-endian reader for in-memory game data.

use std::fmt;
use std::ops::Range;

/// A malformed binary input error with the field and byte offset that failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    context: String,
    offset: usize,
    kind: ErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ErrorKind {
    Truncated {
        wanted: usize,
        remaining: usize,
    },
    ArithmeticOverflow {
        count: usize,
        item_size: usize,
    },
    NegativeCount(i32),
    CountOutOfRange {
        count: usize,
        item_size: usize,
        remaining: usize,
    },
    InvalidRange {
        length: usize,
        total: usize,
    },
}

impl Error {
    fn new(context: impl Into<String>, offset: usize, kind: ErrorKind) -> Self {
        Self {
            context: context.into(),
            offset,
            kind,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}: ", self.context, self.offset)?;
        match self.kind {
            ErrorKind::Truncated { wanted, remaining } => {
                write!(f, "wanted {wanted} bytes, only {remaining} remain")
            }
            ErrorKind::ArithmeticOverflow { count, item_size } => {
                write!(
                    f,
                    "byte count overflow for {count} items of {item_size} bytes"
                )
            }
            ErrorKind::NegativeCount(count) => write!(f, "negative count {count}"),
            ErrorKind::CountOutOfRange {
                count,
                item_size,
                remaining,
            } => write!(
                f,
                "count {count} requires at least {} bytes ({item_size} each), but only {remaining} remain",
                count.saturating_mul(item_size)
            ),
            ErrorKind::InvalidRange { length, total } => write!(
                f,
                "range starting here with length {length} exceeds buffer length {total}"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Validate a range against an external buffer using the same checked arithmetic.
pub fn checked_range(
    start: usize,
    length: usize,
    total: usize,
    context: impl Into<String>,
) -> Result<Range<usize>, Error> {
    let Some(end) = start.checked_add(length) else {
        return Err(Error::new(
            context,
            start,
            ErrorKind::ArithmeticOverflow {
                count: 1,
                item_size: length,
            },
        ));
    };
    if end > total {
        return Err(Error::new(
            context,
            start,
            ErrorKind::InvalidRange { length, total },
        ));
    }
    Ok(start..end)
}

/// Sequential reader over a byte slice.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    pub fn position(&self) -> usize {
        self.position
    }

    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    pub fn take(&mut self, length: usize, context: impl Into<String>) -> Result<&'a [u8], Error> {
        let Some(end) = self.position.checked_add(length) else {
            return Err(Error::new(
                context,
                self.position,
                ErrorKind::ArithmeticOverflow {
                    count: 1,
                    item_size: length,
                },
            ));
        };
        if end > self.bytes.len() {
            return Err(Error::new(
                context,
                self.position,
                ErrorKind::Truncated {
                    wanted: length,
                    remaining: self.remaining(),
                },
            ));
        }
        let result = &self.bytes[self.position..end];
        self.position = end;
        Ok(result)
    }

    pub fn take_array<const N: usize>(
        &mut self,
        context: impl Into<String>,
    ) -> Result<[u8; N], Error> {
        let mut result = [0; N];
        result.copy_from_slice(self.take(N, context)?);
        Ok(result)
    }

    pub fn u8(&mut self, context: impl Into<String>) -> Result<u8, Error> {
        Ok(self.take_array::<1>(context)?[0])
    }

    pub fn u16(&mut self, context: impl Into<String>) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.take_array::<2>(context)?))
    }

    pub fn u32(&mut self, context: impl Into<String>) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.take_array::<4>(context)?))
    }

    pub fn i32(&mut self, context: impl Into<String>) -> Result<i32, Error> {
        Ok(i32::from_le_bytes(self.take_array::<4>(context)?))
    }

    pub fn f32(&mut self, context: impl Into<String>) -> Result<f32, Error> {
        Ok(f32::from_le_bytes(self.take_array::<4>(context)?))
    }

    /// Read an unsigned count and verify its minimum encoded footprint before allocation.
    pub fn count_u32(
        &mut self,
        context: impl Into<String>,
        minimum_item_size: usize,
    ) -> Result<usize, Error> {
        let context = context.into();
        let offset = self.position;
        let count = self.u32(context.as_str())? as usize;
        self.validate_count(count, minimum_item_size, context, offset)?;
        Ok(count)
    }

    /// Read a signed count, rejecting negative values and impossible minimum footprints.
    pub fn count_i32(
        &mut self,
        context: impl Into<String>,
        minimum_item_size: usize,
    ) -> Result<usize, Error> {
        let context = context.into();
        let offset = self.position;
        let raw = self.i32(context.as_str())?;
        let count = usize::try_from(raw)
            .map_err(|_| Error::new(context.clone(), offset, ErrorKind::NegativeCount(raw)))?;
        self.validate_count(count, minimum_item_size, context, offset)?;
        Ok(count)
    }

    pub fn validate_count(
        &self,
        count: usize,
        minimum_item_size: usize,
        context: impl Into<String>,
        offset: usize,
    ) -> Result<(), Error> {
        let Some(minimum_bytes) = count.checked_mul(minimum_item_size) else {
            return Err(Error::new(
                context,
                offset,
                ErrorKind::ArithmeticOverflow {
                    count,
                    item_size: minimum_item_size,
                },
            ));
        };
        if minimum_bytes > self.remaining() {
            return Err(Error::new(
                context,
                offset,
                ErrorKind::CountOutOfRange {
                    count,
                    item_size: minimum_item_size,
                    remaining: self.remaining(),
                },
            ));
        }
        Ok(())
    }

    /// Return an arbitrary checked range without moving the sequential cursor.
    pub fn range(
        &self,
        start: usize,
        length: usize,
        context: impl Into<String>,
    ) -> Result<&'a [u8], Error> {
        let range = checked_range(start, length, self.bytes.len(), context)?;
        Ok(&self.bytes[range])
    }

    pub fn seek(&mut self, position: usize, context: impl Into<String>) -> Result<(), Error> {
        if position > self.bytes.len() {
            return Err(Error::new(
                context,
                position,
                ErrorKind::InvalidRange {
                    length: 0,
                    total: self.bytes.len(),
                },
            ));
        }
        self.position = position;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize, serde::Deserialize)]
    struct UnusedContext;

    impl From<UnusedContext> for String {
        fn from(_: UnusedContext) -> Self {
            panic!("successful reads must not materialize error context")
        }
    }

    #[test]
    fn successful_reads_do_not_materialize_error_context() {
        let bytes = [0; 16];
        let mut reader = Reader::new(&bytes);
        assert_eq!(checked_range(2, 3, bytes.len(), UnusedContext), Ok(2..5));
        assert_eq!(reader.range(2, 3, UnusedContext).unwrap(), &[0; 3]);
        reader.validate_count(2, 8, UnusedContext, 0).unwrap();
        assert_eq!(reader.u8(UnusedContext).unwrap(), 0);
        assert_eq!(reader.u16(UnusedContext).unwrap(), 0);
        assert_eq!(reader.u32(UnusedContext).unwrap(), 0);
        assert_eq!(reader.i32(UnusedContext).unwrap(), 0);
        assert_eq!(reader.f32(UnusedContext).unwrap(), 0.0);
        assert_eq!(reader.take(1, UnusedContext).unwrap(), &[0]);
        assert!(reader.take(0, UnusedContext).unwrap().is_empty());
        reader.seek(0, UnusedContext).unwrap();
    }

    #[test]
    fn arithmetic_failures_preserve_context_and_cursor() {
        let mut reader = Reader::new(&[0; 4]);
        reader.seek(1, "start").unwrap();
        let error = reader.take(usize::MAX, "payload").unwrap_err();
        assert_eq!(error.context, "payload");
        assert_eq!(error.offset, 1);
        assert!(matches!(error.kind, ErrorKind::ArithmeticOverflow { .. }));
        assert_eq!(reader.position(), 1);

        let error = reader
            .validate_count(usize::MAX, 2, "items", 7)
            .unwrap_err();
        assert_eq!(error.context, "items");
        assert_eq!(error.offset, 7);
        assert!(matches!(error.kind, ErrorKind::ArithmeticOverflow { .. }));
        assert_eq!(reader.position(), 1);
    }

    #[test]
    fn take_reports_context_offset_and_remaining_bytes() {
        let mut reader = Reader::new(&[1, 2, 3]);
        reader.take(2, "prefix").unwrap();
        let error = reader.take(2, "payload").unwrap_err();
        assert_eq!(
            error.to_string(),
            "payload at byte 2: wanted 2 bytes, only 1 remain"
        );
    }

    #[test]
    fn negative_and_impossible_counts_are_rejected_before_allocation() {
        let negative_bytes = (-1i32).to_le_bytes();
        let mut negative = Reader::new(&negative_bytes);
        assert!(negative.count_i32("items", 1).is_err());

        let oversized_bytes = 100u32.to_le_bytes();
        let mut oversized = Reader::new(&oversized_bytes);
        let error = oversized.count_u32("items", 8).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("count 100 requires at least 800 bytes")
        );
    }

    #[test]
    fn range_rejects_overflowing_cursor_arithmetic() {
        let reader = Reader::new(&[]);
        let error = reader.range(usize::MAX, 2, "range").unwrap_err();
        assert!(error.to_string().contains("byte count overflow"));
    }
}
