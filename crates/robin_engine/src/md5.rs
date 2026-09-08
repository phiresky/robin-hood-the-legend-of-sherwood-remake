//! MD5 digest, wrapping the `md-5` crate.
//!
//! Streaming API (`update` / `finalize`) with value semantics: the context
//! is `Clone`, so a partially-fed hasher can be copied and finalized
//! independently (used by callers that branch on a running digest).

use md5_crate::{Digest, Md5};

/// Internal state for a single MD5 context. `update`s are buffered into
/// the hasher; once `finalize` is called the 16-byte digest is stored and
/// further updates are ignored.
#[derive(Clone)]
pub struct Md5Ctx {
    hasher: Md5,
    digest: [u8; 16],
    finalized: bool,
}

impl Default for Md5Ctx {
    fn default() -> Self {
        Self::new()
    }
}

impl Md5Ctx {
    pub fn new() -> Self {
        Md5Ctx {
            hasher: Md5::new(),
            digest: [0u8; 16],
            finalized: false,
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        if self.finalized {
            return;
        }
        self.hasher.update(data);
    }

    pub fn finalize(&mut self) {
        if self.finalized {
            return;
        }
        // `finalize_reset` would also work, but we're done with the hasher.
        let result = std::mem::replace(&mut self.hasher, Md5::new()).finalize();
        self.digest.copy_from_slice(&result);
        self.finalized = true;
    }

    /// Return a copy of the 16-byte raw digest. Needed by validate_stream.
    pub fn raw_digest_bytes(&self) -> [u8; 16] {
        self.digest
    }

    /// Hex-encoded digest, used by the MD5 regression tests.
    #[cfg(test)]
    pub(crate) fn hex_digest(&self) -> [u8; 32] {
        let mut out = [0u8; 32];
        static HEX: &[u8; 16] = b"0123456789abcdef";
        for (i, &b) in self.digest.iter().enumerate() {
            out[i * 2] = HEX[(b >> 4) as usize];
            out[i * 2 + 1] = HEX[(b & 0x0f) as usize];
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest_of(input: &[u8]) -> String {
        let mut ctx = Md5Ctx::new();
        ctx.update(input);
        ctx.finalize();
        String::from_utf8(ctx.hex_digest().to_vec()).unwrap()
    }

    #[test]
    fn rfc1321_empty_string() {
        assert_eq!(digest_of(b""), "d41d8cd98f00b204e9800998ecf8427e");
    }

    #[test]
    fn rfc1321_a() {
        assert_eq!(digest_of(b"a"), "0cc175b9c0f1b6a831c399e269772661");
    }

    #[test]
    fn rfc1321_abc() {
        assert_eq!(digest_of(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn rfc1321_message_digest() {
        assert_eq!(
            digest_of(b"message digest"),
            "f96b697d7cb7938d525a2f31aaf161d0"
        );
    }

    #[test]
    fn rfc1321_alphabet() {
        assert_eq!(
            digest_of(b"abcdefghijklmnopqrstuvwxyz"),
            "c3fcd3d76192e4007dfb496cca67e13b"
        );
    }

    #[test]
    fn streaming_update_equals_single_update() {
        let mut a = Md5Ctx::new();
        a.update(b"The quick brown fox ");
        a.update(b"jumps over the lazy dog");
        a.finalize();

        let mut b = Md5Ctx::new();
        b.update(b"The quick brown fox jumps over the lazy dog");
        b.finalize();

        assert_eq!(a.hex_digest(), b.hex_digest());
        assert_eq!(
            String::from_utf8(a.hex_digest().to_vec()).unwrap(),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
    }

    #[test]
    fn clone_preserves_state() {
        let mut a = Md5Ctx::new();
        a.update(b"partial");
        let mut b = a.clone();
        a.update(b" rest");
        b.update(b" rest");
        a.finalize();
        b.finalize();
        assert_eq!(a.digest, b.digest);
    }

    #[test]
    fn update_after_finalize_is_ignored() {
        let mut a = Md5Ctx::new();
        a.update(b"abc");
        a.finalize();
        let first = a.hex_digest();
        a.update(b"more data");
        a.finalize();
        let second = a.hex_digest();
        assert_eq!(first, second);
    }

    #[test]
    fn unfinalized_digest_is_zero() {
        let ctx = Md5Ctx::new();
        assert_eq!(ctx.digest, [0u8; 16]);
        assert!(!ctx.finalized);
    }

    #[test]
    fn raw_digest_bytes() {
        let mut ctx = Md5Ctx::new();
        ctx.update(b"abc");
        ctx.finalize();
        let expected: [u8; 16] = [
            0x90, 0x01, 0x50, 0x98, 0x3c, 0xd2, 0x4f, 0xb0, 0xd6, 0x96, 0x3f, 0x7d, 0x28, 0xe1,
            0x7f, 0x72,
        ];
        assert_eq!(ctx.raw_digest_bytes(), expected);
    }
}
