//! Byte-layout fixtures use the same reader without temporary filesystem state.

use crate::legacy_io::LegacyReader;
use crate::sbfile::SbFile;

pub(super) fn with_reader<T>(bytes: &[u8], read: impl FnOnce(&mut LegacyReader<'_>) -> T) -> T {
    let mut file = SbFile::from_owned_bytes(bytes.to_vec(), "legacy-save-test");
    read(&mut LegacyReader::new(&mut file))
}
