//! Byte-layout fixtures use the same reader without temporary filesystem state.

use crate::legacy_io::LegacyReader;
use crate::sbfile::SbFile;

pub(super) fn with_reader<T>(bytes: &[u8], read: impl FnOnce(&mut LegacyReader<'_>) -> T) -> T {
    let mut file = SbFile::from_owned_bytes(bytes.to_vec(), "legacy-save-test");
    read(&mut LegacyReader::new(&mut file))
}

pub(super) fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

pub(super) fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

pub(super) fn push_i32(bytes: &mut Vec<u8>, value: i32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

pub(super) fn push_f32(bytes: &mut Vec<u8>, value: f32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
