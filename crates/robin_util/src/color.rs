//! Small color helpers used by both the renderer and sim-facing code
//! that bakes constant colors (e.g. entity outline colors).

/// Pack (r, g, b) into a 16-bit RGB565 word.
#[inline]
pub const fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 & 0xF8) << 8) | ((g as u16 & 0xFC) << 3) | ((b as u16) >> 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing_truncates_each_channel_without_bleeding() {
        for channel in 0..=u8::MAX {
            assert_eq!(rgb565(channel, 0, 0), u16::from(channel / 8) << 11);
            assert_eq!(rgb565(0, channel, 0), u16::from(channel / 4) << 5);
            assert_eq!(rgb565(0, 0, channel), u16::from(channel / 8));
        }
    }

    #[test]
    fn every_packed_color_roundtrips_with_discarded_low_bits() {
        for packed in 0..=u16::MAX {
            let r = ((packed >> 11) as u8) << 3;
            let g = (((packed >> 5) & 63) as u8) << 2;
            let b = ((packed & 31) as u8) << 3;
            assert_eq!(rgb565(r, g, b), packed);
            assert_eq!(rgb565(r | 7, g | 3, b | 7), packed);
        }
    }
}
