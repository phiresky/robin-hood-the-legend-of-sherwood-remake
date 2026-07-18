//! ADPCMCheck — audio codec availability check.
//!
//! The original Windows code enumerated ACM (Audio Compression Manager)
//! drivers to find the Microsoft ADPCM codec, re-enabling it if it was
//! disabled. On non-Windows platforms the codec is handled in software
//! by the audio backend, so we unconditionally report it as available.

/// Returns `true` when an ADPCM codec is available for playback.
///
/// On the original Windows build this walked the ACM driver list; on
/// modern Linux/Emscripten builds ADPCM is decoded in software, so this
/// always succeeds.
pub fn is_codec_available() -> bool {
    // Software decode — always available.
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_always_available() {
        assert!(is_codec_available());
    }
}
