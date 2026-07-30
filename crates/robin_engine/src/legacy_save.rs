//! Container-level reader for original-game RHSG save files.
//!
//! The Original writes this header in `RHSaveGame::SerializeHeader` before
//! serializing any campaign or engine state:
//!
//! ```text
//! 0x00  char[4]  "RHSG" (Linux i386 port) or "GSHR" (retail Win32)
//! 0x04  u32      save header version
//! 0x08  u32      mission profile id
//! 0x0c  u32      SBFile stream version
//! ```
//!
//! The header reader leaves callers at byte 16. [`campaign`] then decodes the
//! two consecutive, lengthless `RHCampaign::Serialize` streams which precede
//! the engine state.

pub mod campaign;
pub mod elements;
pub mod engine;
pub mod payload_actors;
pub mod payload_base;
pub mod payload_nonactors;
pub mod payload_objects;
pub mod payload_sequences;
pub mod payload_vm;

use serde::{Deserialize, Serialize};

use crate::legacy_io::{LegacyReader, LegacyResult};
use crate::sbfile::SbFile;

pub const PORT_LINUX_I386_MAGIC: [u8; 4] = *b"RHSG";
pub const RETAIL_WINDOWS_X86_MAGIC: [u8; 4] = *b"GSHR";
/// Compatibility name for the Linux i386 port's header magic.
pub const RHSG_MAGIC: [u8; 4] = PORT_LINUX_I386_MAGIC;
pub const RHSG_VERSION: u32 = 48;
pub const RHSG_HEADER_LEN: u64 = 16;

/// Concrete Original-save ABI identified by the serialized header.
///
/// Both supported v48 producers write little-endian streams with one-byte
/// booleans, two-byte words, and four-byte longs, enums, floats, and pointer
/// placeholders. Keeping the producer explicit prevents opaque raw pointers
/// and skipped compiler padding in the engine body from being interpreted
/// using the host Rust ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegacySaveAbiProfile {
    RetailWindowsX86V48,
    PortLinuxI386V48,
}

impl LegacySaveAbiProfile {
    /// Serialized scalar widths for both audited v48 producer ABIs.
    ///
    /// These are save-format widths, never widths of the current Rust host.
    pub const BOOL_WIDTH: u8 = 1;
    pub const WORD_WIDTH: u8 = 2;
    pub const LONG_WIDTH: u8 = 4;
    pub const ENUM_WIDTH: u8 = 4;
    pub const FLOAT_WIDTH: u8 = 4;
    pub const POINTER_PLACEHOLDER_WIDTH: u8 = 4;

    pub const fn is_little_endian(self) -> bool {
        true
    }

    pub const fn magic(self) -> [u8; 4] {
        match self {
            Self::RetailWindowsX86V48 => RETAIL_WINDOWS_X86_MAGIC,
            Self::PortLinuxI386V48 => PORT_LINUX_I386_MAGIC,
        }
    }

    fn detect(magic: [u8; 4]) -> Option<Self> {
        match magic {
            RETAIL_WINDOWS_X86_MAGIC => Some(Self::RetailWindowsX86V48),
            PORT_LINUX_I386_MAGIC => Some(Self::PortLinuxI386V48),
            _ => None,
        }
    }
}

/// Validated metadata at the front of an Original v48 save stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacySaveHeader {
    /// Exact four bytes written by the producing executable.
    pub magic: [u8; 4],
    pub abi_profile: LegacySaveAbiProfile,
    pub header_version: u32,
    pub mission_id: u32,
    pub stream_version: u32,
    /// Offset at which the serialized campaign/engine body begins.
    pub body_offset: u64,
}

impl LegacySaveHeader {
    /// Read and validate an Original v48 header, leaving `reader` at the first
    /// body byte. The reader remains owned by the caller for body parsing.
    pub fn read(reader: &mut LegacyReader<'_>) -> LegacyResult<Self> {
        reader.scope("rhsg.header", |reader| {
            let magic_offset = reader.offset();
            let mut magic = [0; 4];
            reader.read_bytes("magic", &mut magic)?;
            let Some(abi_profile) = LegacySaveAbiProfile::detect(magic) else {
                return Err(reader.invalid_value(
                    magic_offset,
                    "magic",
                    format_args!("{magic:02x?}"),
                    "ASCII magic RHSG (Linux i386) or GSHR (retail Win32)",
                ));
            };

            let header_version_offset = reader.offset();
            let header_version = reader.read_u32("header_version")?;
            if header_version != RHSG_VERSION {
                return Err(reader.invalid_value(
                    header_version_offset,
                    "header_version",
                    header_version,
                    "RHSG header version 48",
                ));
            }

            let mission_id = reader.read_u32("mission_id")?;

            let stream_version_offset = reader.offset();
            let stream_version = reader.read_version("stream_version")?;
            if stream_version != RHSG_VERSION {
                return Err(reader.invalid_value(
                    stream_version_offset,
                    "stream_version",
                    stream_version,
                    "SBFile stream version 48",
                ));
            }

            let body_offset = reader.offset();
            debug_assert_eq!(body_offset, RHSG_HEADER_LEN);
            Ok(Self {
                magic,
                abi_profile,
                header_version,
                mission_id,
                stream_version,
                body_offset,
            })
        })
    }
}

/// Parse a save header from an existing `SbFile`, leaving the file positioned
/// at the first byte of the as-yet-unparsed save body.
pub fn read_header(file: &mut SbFile) -> LegacyResult<LegacySaveHeader> {
    LegacySaveHeader::read(&mut LegacyReader::new(file))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;
    use crate::legacy_io::LegacyIoErrorKind;
    use crate::sbfile::SB_FILE_READ;

    const NOTTINGHAM_MISSION_ID: u32 = 0x4153;

    fn save_bytes(
        magic: [u8; 4],
        header_version: u32,
        mission_id: u32,
        stream_version: u32,
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&magic);
        bytes.extend_from_slice(&header_version.to_le_bytes());
        bytes.extend_from_slice(&mission_id.to_le_bytes());
        bytes.extend_from_slice(&stream_version.to_le_bytes());
        bytes
    }

    fn with_reader<T>(bytes: &[u8], read: impl FnOnce(&mut LegacyReader<'_>) -> T) -> T {
        let mut temporary = NamedTempFile::new().unwrap();
        temporary.write_all(bytes).unwrap();
        temporary.flush().unwrap();
        let path = temporary.path().to_str().unwrap();
        let mut file = SbFile::open(path, SB_FILE_READ).unwrap();
        read(&mut LegacyReader::new(&mut file))
    }

    #[test]
    fn reads_nottingham_header_and_leaves_reader_at_body() {
        let mut bytes = save_bytes(
            RHSG_MAGIC,
            RHSG_VERSION,
            NOTTINGHAM_MISSION_ID,
            RHSG_VERSION,
        );
        bytes.extend_from_slice(&[0xaa, 0xbb]);

        with_reader(&bytes, |reader| {
            let header = LegacySaveHeader::read(reader).unwrap();
            assert_eq!(
                header,
                LegacySaveHeader {
                    magic: PORT_LINUX_I386_MAGIC,
                    abi_profile: LegacySaveAbiProfile::PortLinuxI386V48,
                    header_version: 48,
                    mission_id: NOTTINGHAM_MISSION_ID,
                    stream_version: 48,
                    body_offset: 16,
                }
            );
            assert_eq!(reader.offset(), RHSG_HEADER_LEN);
            assert_eq!(reader.read_u8("body.first_byte").unwrap(), 0xaa);
        });
    }

    #[test]
    fn detects_retail_windows_profile_without_normalizing_magic() {
        let bytes = save_bytes(
            RETAIL_WINDOWS_X86_MAGIC,
            RHSG_VERSION,
            NOTTINGHAM_MISSION_ID,
            RHSG_VERSION,
        );

        with_reader(&bytes, |reader| {
            let header = LegacySaveHeader::read(reader).unwrap();
            assert_eq!(header.magic, *b"GSHR");
            assert_eq!(
                header.abi_profile,
                LegacySaveAbiProfile::RetailWindowsX86V48
            );
            assert_eq!(header.abi_profile.magic(), header.magic);
            assert_eq!(reader.offset(), RHSG_HEADER_LEN);
        });
    }

    #[test]
    fn audited_v48_profiles_have_fixed_32_bit_abi_layout() {
        for profile in [
            LegacySaveAbiProfile::RetailWindowsX86V48,
            LegacySaveAbiProfile::PortLinuxI386V48,
        ] {
            assert!(profile.is_little_endian());
            assert_eq!(LegacySaveAbiProfile::BOOL_WIDTH, 1);
            assert_eq!(LegacySaveAbiProfile::WORD_WIDTH, 2);
            assert_eq!(LegacySaveAbiProfile::LONG_WIDTH, 4);
            assert_eq!(LegacySaveAbiProfile::ENUM_WIDTH, 4);
            assert_eq!(LegacySaveAbiProfile::FLOAT_WIDTH, 4);
            assert_eq!(LegacySaveAbiProfile::POINTER_PLACEHOLDER_WIDTH, 4);
        }
    }

    #[test]
    fn rejects_invalid_magic_with_field_and_offset() {
        let bytes = save_bytes(*b"NOPE", 48, NOTTINGHAM_MISSION_ID, 48);
        let error = with_reader(&bytes, |reader| LegacySaveHeader::read(reader).unwrap_err());

        assert_eq!(error.offset, 0);
        assert_eq!(error.field, "rhsg.header.magic");
        assert!(matches!(
            error.kind,
            LegacyIoErrorKind::InvalidValue {
                expected: "ASCII magic RHSG (Linux i386) or GSHR (retail Win32)",
                ..
            }
        ));
    }

    #[test]
    fn rejects_header_and_stream_versions_independently() {
        let invalid_header = save_bytes(RHSG_MAGIC, 47, NOTTINGHAM_MISSION_ID, 48);
        let error = with_reader(&invalid_header, |reader| {
            LegacySaveHeader::read(reader).unwrap_err()
        });
        assert_eq!(error.offset, 4);
        assert_eq!(error.field, "rhsg.header.header_version");

        let invalid_stream = save_bytes(RHSG_MAGIC, 48, NOTTINGHAM_MISSION_ID, 47);
        let error = with_reader(&invalid_stream, |reader| {
            LegacySaveHeader::read(reader).unwrap_err()
        });
        assert_eq!(error.offset, 12);
        assert_eq!(error.field, "rhsg.header.stream_version");
    }

    #[test]
    fn reports_truncated_header_at_the_missing_field() {
        let bytes = save_bytes(RHSG_MAGIC, 48, NOTTINGHAM_MISSION_ID, 48);
        let error = with_reader(&bytes[..14], |reader| {
            LegacySaveHeader::read(reader).unwrap_err()
        });

        assert_eq!(error.offset, 12);
        assert_eq!(error.field, "rhsg.header.stream_version");
        assert!(matches!(error.kind, LegacyIoErrorKind::SbFile { .. }));
    }
}
