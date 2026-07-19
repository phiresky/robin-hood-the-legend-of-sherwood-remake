//! Typed I/O for binary files authored by the original game tools.
//!
//! This is deliberately separate from serde/bitcode save and snapshot data.
//! [`LegacyReader`] adds path, byte offset, and field context to the read-only
//! [`SbFile`] compatibility layer. [`LegacyWriter`] exists for authored-data
//! tools and byte-layout tests; it is not a save-game writer.

use std::fmt;
use std::io::{self, Write};

use thiserror::Error;

use crate::sbfile::SbFile;

pub type LegacyResult<T> = Result<T, LegacyIoError>;

/// A legacy binary I/O failure with enough context to locate the bad field.
#[derive(Debug, Error)]
#[error("legacy binary I/O error in {path} at byte {offset} ({field}): {kind}")]
pub struct LegacyIoError {
    pub path: String,
    pub offset: u64,
    pub field: String,
    #[source]
    pub kind: LegacyIoErrorKind,
}

#[derive(Debug, Error)]
pub enum LegacyIoErrorKind {
    #[error("SBFile compatibility error {code}")]
    SbFile { code: i32 },
    #[error("stream write failed")]
    Write(#[source] io::Error),
    #[error("invalid value {value}; expected {expected}")]
    InvalidValue {
        value: String,
        expected: &'static str,
    },
    #[error("length {length} does not fit the legacy {width}-bit length field")]
    LengthOverflow { length: usize, width: u8 },
    #[error("unable to allocate space for {count} legacy items")]
    Allocation { count: usize },
}

/// Typed, contextual reads over the read-only SBFile compatibility layer.
pub struct LegacyReader<'a> {
    file: &'a mut SbFile,
    context: Vec<String>,
}

impl<'a> LegacyReader<'a> {
    pub fn new(file: &'a mut SbFile) -> Self {
        Self {
            file,
            context: Vec::new(),
        }
    }

    pub fn path(&self) -> &str {
        self.file.path()
    }

    pub fn offset(&mut self) -> u64 {
        self.file.tell()
    }

    /// Add a field/container prefix for all errors produced by `read`.
    pub fn scope<T>(
        &mut self,
        context: impl Into<String>,
        read: impl FnOnce(&mut Self) -> LegacyResult<T>,
    ) -> LegacyResult<T> {
        self.context.push(context.into());
        let result = read(self);
        self.context.pop();
        result
    }

    pub fn invalid_value(
        &mut self,
        offset: u64,
        field: impl fmt::Display,
        value: impl fmt::Display,
        expected: &'static str,
    ) -> LegacyIoError {
        self.error_at(
            offset,
            field,
            LegacyIoErrorKind::InvalidValue {
                value: value.to_string(),
                expected,
            },
        )
    }

    pub fn allocation_error(
        &mut self,
        offset: u64,
        field: impl fmt::Display,
        count: usize,
    ) -> LegacyIoError {
        self.error_at(offset, field, LegacyIoErrorKind::Allocation { count })
    }

    pub fn read_bytes(&mut self, field: impl fmt::Display, bytes: &mut [u8]) -> LegacyResult<()> {
        let offset = self.offset();
        self.file
            .serialize_bytes(bytes)
            .map_err(|code| self.error_at(offset, field, LegacyIoErrorKind::SbFile { code }))
    }

    pub fn read_u8(&mut self, field: impl fmt::Display) -> LegacyResult<u8> {
        Ok(self.read_array::<1>(field)?[0])
    }

    pub fn read_i8(&mut self, field: impl fmt::Display) -> LegacyResult<i8> {
        Ok(self.read_u8(field)? as i8)
    }

    pub fn read_u16(&mut self, field: impl fmt::Display) -> LegacyResult<u16> {
        Ok(u16::from_le_bytes(self.read_array(field)?))
    }

    pub fn read_i16(&mut self, field: impl fmt::Display) -> LegacyResult<i16> {
        Ok(i16::from_le_bytes(self.read_array(field)?))
    }

    pub fn read_u32(&mut self, field: impl fmt::Display) -> LegacyResult<u32> {
        Ok(u32::from_le_bytes(self.read_array(field)?))
    }

    pub fn read_i32(&mut self, field: impl fmt::Display) -> LegacyResult<i32> {
        Ok(i32::from_le_bytes(self.read_array(field)?))
    }

    pub fn read_u64(&mut self, field: impl fmt::Display) -> LegacyResult<u64> {
        Ok(u64::from_le_bytes(self.read_array(field)?))
    }

    pub fn read_i64(&mut self, field: impl fmt::Display) -> LegacyResult<i64> {
        Ok(i64::from_le_bytes(self.read_array(field)?))
    }

    pub fn read_f32(&mut self, field: impl fmt::Display) -> LegacyResult<f32> {
        Ok(f32::from_le_bytes(self.read_array(field)?))
    }

    /// The original format stores C++ `bool` in one byte. As in SBFile and
    /// the original runtime, any non-zero authored byte normalizes to `true`.
    pub fn read_bool(&mut self, field: impl fmt::Display) -> LegacyResult<bool> {
        Ok(self.read_u8(field)? != 0)
    }

    pub fn read_version(&mut self, field: impl fmt::Display) -> LegacyResult<u32> {
        self.read_u32(field)
    }

    pub fn read_checkpoint(&mut self, field: impl fmt::Display + Copy) -> LegacyResult<()> {
        let offset = self.offset();
        let value = self.read_u16(field)?;
        if value == 0x7777 {
            Ok(())
        } else {
            Err(self.invalid_value(offset, field, format_args!("0x{value:04x}"), "0x7777"))
        }
    }

    /// Read an `SBString`: a little-endian u16 byte length followed by bytes.
    /// Shipped strings are ASCII. Lossy UTF-8 conversion preserves the prior
    /// Rust loader's behavior for non-UTF-8 authored data.
    pub fn read_string(&mut self, field: impl fmt::Display + Copy) -> LegacyResult<String> {
        let len = self.read_u16(format_args!("{field}.length"))? as usize;
        let offset = self.offset();
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(len)
            .map_err(|_| self.allocation_error(offset, field, len))?;
        bytes.resize(len, 0);
        self.read_bytes(format_args!("{field}.bytes"), &mut bytes)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn read_array<const N: usize>(&mut self, field: impl fmt::Display) -> LegacyResult<[u8; N]> {
        let mut bytes = [0; N];
        self.read_bytes(field, &mut bytes)?;
        Ok(bytes)
    }

    fn error_at(
        &self,
        offset: u64,
        field: impl fmt::Display,
        kind: LegacyIoErrorKind,
    ) -> LegacyIoError {
        LegacyIoError {
            path: self.path().to_owned(),
            offset,
            field: self.field_path(field),
            kind,
        }
    }

    fn field_path(&self, field: impl fmt::Display) -> String {
        let field = field.to_string();
        if self.context.is_empty() {
            field
        } else if field.is_empty() {
            self.context.join(".")
        } else {
            format!("{}.{}", self.context.join("."), field)
        }
    }
}

/// Typed writer for original-game authored binary layouts.
pub struct LegacyWriter<W> {
    writer: W,
    path: String,
    offset: u64,
    context: Vec<String>,
}

impl<W: Write> LegacyWriter<W> {
    pub fn new(writer: W, path: impl Into<String>) -> Self {
        Self {
            writer,
            path: path.into(),
            offset: 0,
            context: Vec::new(),
        }
    }

    pub fn offset(&self) -> u64 {
        self.offset
    }

    pub fn into_inner(self) -> W {
        self.writer
    }

    pub fn scope<T>(
        &mut self,
        context: impl Into<String>,
        write: impl FnOnce(&mut Self) -> LegacyResult<T>,
    ) -> LegacyResult<T> {
        self.context.push(context.into());
        let result = write(self);
        self.context.pop();
        result
    }

    pub fn write_bytes(&mut self, field: impl fmt::Display, mut bytes: &[u8]) -> LegacyResult<()> {
        while !bytes.is_empty() {
            match self.writer.write(bytes) {
                Ok(0) => {
                    return Err(self.error_at(
                        self.offset,
                        field,
                        LegacyIoErrorKind::Write(io::Error::new(
                            io::ErrorKind::WriteZero,
                            "failed to write the complete legacy field",
                        )),
                    ));
                }
                Ok(written) => {
                    self.offset += written as u64;
                    bytes = &bytes[written..];
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    return Err(self.error_at(self.offset, field, LegacyIoErrorKind::Write(error)));
                }
            }
        }
        Ok(())
    }

    pub fn write_u8(&mut self, field: impl fmt::Display, value: u8) -> LegacyResult<()> {
        self.write_bytes(field, &[value])
    }

    pub fn write_i8(&mut self, field: impl fmt::Display, value: i8) -> LegacyResult<()> {
        self.write_u8(field, value as u8)
    }

    pub fn write_u16(&mut self, field: impl fmt::Display, value: u16) -> LegacyResult<()> {
        self.write_bytes(field, &value.to_le_bytes())
    }

    pub fn write_i16(&mut self, field: impl fmt::Display, value: i16) -> LegacyResult<()> {
        self.write_bytes(field, &value.to_le_bytes())
    }

    pub fn write_u32(&mut self, field: impl fmt::Display, value: u32) -> LegacyResult<()> {
        self.write_bytes(field, &value.to_le_bytes())
    }

    pub fn write_i32(&mut self, field: impl fmt::Display, value: i32) -> LegacyResult<()> {
        self.write_bytes(field, &value.to_le_bytes())
    }

    pub fn write_u64(&mut self, field: impl fmt::Display, value: u64) -> LegacyResult<()> {
        self.write_bytes(field, &value.to_le_bytes())
    }

    pub fn write_i64(&mut self, field: impl fmt::Display, value: i64) -> LegacyResult<()> {
        self.write_bytes(field, &value.to_le_bytes())
    }

    pub fn write_f32(&mut self, field: impl fmt::Display, value: f32) -> LegacyResult<()> {
        self.write_bytes(field, &value.to_le_bytes())
    }

    pub fn write_bool(&mut self, field: impl fmt::Display, value: bool) -> LegacyResult<()> {
        self.write_u8(field, u8::from(value))
    }

    pub fn write_version(&mut self, field: impl fmt::Display, value: u32) -> LegacyResult<()> {
        self.write_u32(field, value)
    }

    pub fn write_checkpoint(&mut self, field: impl fmt::Display) -> LegacyResult<()> {
        self.write_u16(field, 0x7777)
    }

    pub fn write_string(
        &mut self,
        field: impl fmt::Display + Copy,
        value: &str,
    ) -> LegacyResult<()> {
        let length = u16::try_from(value.len()).map_err(|_| LegacyIoError {
            path: self.path.clone(),
            offset: self.offset,
            field: self.field_path(field),
            kind: LegacyIoErrorKind::LengthOverflow {
                length: value.len(),
                width: 16,
            },
        })?;
        self.write_u16(format_args!("{field}.length"), length)?;
        self.write_bytes(format_args!("{field}.bytes"), value.as_bytes())
    }

    fn error_at(
        &self,
        offset: u64,
        field: impl fmt::Display,
        kind: LegacyIoErrorKind,
    ) -> LegacyIoError {
        LegacyIoError {
            path: self.path.clone(),
            offset,
            field: self.field_path(field),
            kind,
        }
    }

    fn field_path(&self, field: impl fmt::Display) -> String {
        let field = field.to_string();
        if self.context.is_empty() {
            field
        } else if field.is_empty() {
            self.context.join(".")
        } else {
            format!("{}.{}", self.context.join("."), field)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sbfile::SB_FILE_READ;

    #[test]
    fn writer_preserves_legacy_little_endian_layout() {
        let mut writer = LegacyWriter::new(Vec::new(), "layout.bin");
        writer
            .scope("profile", |writer| {
                writer.write_u16("range", 0x1234)?;
                writer.write_bool("enabled", true)?;
                writer.write_f32("center.x", 1.5)?;
                writer.write_string("name", "Robin")
            })
            .unwrap();

        assert_eq!(
            writer.into_inner(),
            [
                0x34, 0x12, 0x01, 0x00, 0x00, 0xc0, 0x3f, 0x05, 0x00, b'R', b'o', b'b', b'i', b'n',
            ]
        );
    }

    #[test]
    fn writer_errors_include_path_offset_and_field_context() {
        let mut writer = LegacyWriter::new(Vec::new(), "profile.cpf");
        let value = "x".repeat(u16::MAX as usize + 1);
        let error = writer
            .scope("characters[3]", |writer| {
                writer.write_string("profile_name", &value)
            })
            .unwrap_err();

        assert_eq!(error.path, "profile.cpf");
        assert_eq!(error.offset, 0);
        assert_eq!(error.field, "characters[3].profile_name");
        assert!(matches!(
            error.kind,
            LegacyIoErrorKind::LengthOverflow { width: 16, .. }
        ));
    }

    #[test]
    fn reader_errors_include_requested_path_offset_and_field_context() {
        let mut fixture = tempfile::NamedTempFile::new().unwrap();
        fixture.write_all(&[0x34]).unwrap();
        let path = fixture.path().to_string_lossy().into_owned();
        let mut file = SbFile::open(&path, SB_FILE_READ).unwrap();
        let mut reader = LegacyReader::new(&mut file);

        let error = reader
            .scope("characters[3]", |reader| reader.read_u16("shooting"))
            .unwrap_err();

        assert_eq!(error.path, path);
        assert_eq!(error.offset, 0);
        assert_eq!(error.field, "characters[3].shooting");
        assert!(matches!(
            error.kind,
            LegacyIoErrorKind::SbFile {
                code: crate::sbfile::SBFILE_ERROR_READ
            }
        ));
    }
}
