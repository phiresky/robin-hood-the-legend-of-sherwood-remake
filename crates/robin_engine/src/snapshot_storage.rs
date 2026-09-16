//! Compressed process-local checkpoints with reusable codec allocations.

use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::sync::Arc;

thread_local! {
    // One workspace per calling thread, rather than per retained snapshot.
    static CODECS: RefCell<(bitcode::Buffer, zstd::bulk::Compressor<'static>, zstd::bulk::Decompressor<'static>)> =
        RefCell::new((bitcode::Buffer::new(), zstd::bulk::Compressor::new(0).expect("create snapshot compressor"),
            zstd::bulk::Decompressor::new().expect("create snapshot decompressor")));
}

/// Internal-memory bytes, not a durable or network format. Level zero selects
/// zstd's default compression level. Immutable bytes are shared when a rollback
/// transaction copies its history; the allocation retains no spare capacity.
#[derive(Clone, Serialize, Deserialize)]
pub struct CompressedSnapshotBytes {
    bytes: Arc<[u8]>,
    decoded_len: usize,
}

impl CompressedSnapshotBytes {
    pub fn encode<T: bitcode::Encode>(value: &T) -> Result<Self, String> {
        CODECS.with(|codecs| {
            let mut codecs = codecs.borrow_mut();
            let (bitcode, compressor, _) = &mut *codecs;
            let encoded = bitcode.encode(value);
            Ok(Self {
                decoded_len: encoded.len(),
                bytes: compressor
                    .compress(encoded)
                    .map_err(|e| e.to_string())?
                    .into(),
            })
        })
    }

    pub fn decode<T: bitcode::DecodeOwned>(&self) -> Result<T, String> {
        CODECS.with(|codecs| {
            let mut codecs = codecs.borrow_mut();
            let (bitcode, _, decompressor) = &mut *codecs;
            let bytes = decompressor
                .decompress(&self.bytes, self.decoded_len)
                .map_err(|e| e.to_string())?;
            if bytes.len() != self.decoded_len {
                return Err("compressed snapshot length mismatch".into());
            }
            bitcode.decode(&bytes).map_err(|e| e.to_string())
        })
    }

    pub fn stored_bytes(&self) -> usize {
        self.bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reused_codec_does_not_overwrite_retained_bytes() {
        let first = vec![123u32; 1000];
        let second = (0..3000u32).collect::<Vec<_>>();
        let a = CompressedSnapshotBytes::encode(&first).unwrap();
        let b = CompressedSnapshotBytes::encode(&second).unwrap();
        assert_eq!(a.decode::<Vec<u32>>().unwrap(), first);
        assert_eq!(b.decode::<Vec<u32>>().unwrap(), second);
        assert_eq!(a.decode::<Vec<u32>>().unwrap(), first);
        let mut corrupt = a.clone();
        Arc::make_mut(&mut corrupt.bytes)[0] ^= 0xff;
        assert!(corrupt.decode::<Vec<u32>>().is_err());
    }
}
