//! Small utility helpers — pure functions, no state, no I/O.

/// Returns the bit index of the first set bit in the low 16 bits of `data`.
/// If no bit is set, returns 0.
pub fn bit_to_index(data: u16) -> u16 {
    for bit in 0..16u16 {
        if data & (1u16 << bit) != 0 {
            return bit;
        }
    }
    0
}

/// Population count over the low 16 bits of `data`.
pub fn count_number_of_bits(data: u16) -> u16 {
    data.count_ones() as u16
}

/// Returns the offset (in bytes) into `name` of the first character after
/// the final '/'. If there is no slash, returns 0.
pub fn base_name_offset(name: &[u8]) -> usize {
    // Walk backwards from the end until we find a '/' or fall off the
    // front; return the byte after the slash, or 0 when no slash exists.
    let mut pos = name.len() as isize;
    while pos >= 0 {
        let idx = pos as usize;
        // Treat index == len as "one past the end"; that byte is never
        // '/', so we just decrement.
        if idx < name.len() && name[idx] == b'/' {
            break;
        }
        pos -= 1;
    }
    (pos + 1) as usize
}

/// Parses a hex ASCII string, accepting both upper and lower case digits.
/// Non-hex characters are silently ignored: they don't multiply or add to
/// the accumulator, they're just skipped.
pub fn hex_ascii_to_integer(s: &[u8]) -> u32 {
    let mut result: u32 = 0;
    for &c in s {
        let digit: Option<u32> = match c {
            b'0'..=b'9' => Some((c - b'0') as u32),
            b'a'..=b'f' => Some((c - b'a' + 10) as u32),
            b'A'..=b'F' => Some((c - b'A' + 10) as u32),
            _ => None,
        };
        if let Some(d) = digit {
            result = result.wrapping_mul(16).wrapping_add(d);
        }
    }
    result
}

/// Converts HSV (h in degrees, s and v in 0..=1) to RGB.  Hue is
/// expected in `[0, 360)`; out-of-range input returns black.
pub fn hsv_to_rgb(mut h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    if s == 0.0 {
        return (v, v, v);
    }

    h /= 60.0;
    let i = h.floor();
    let f = h - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));

    // Truncation toward zero of a non-negative float — callers are
    // expected to pass h in [0, 360).
    match i as u16 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        5 => (v, p, q),
        // Out-of-range input — game code never feeds hues >= 360 here.
        _ => (0.0, 0.0, 0.0),
    }
}

/// Converts RGB (all components in 0..=1) to HSV.  Pure black returns
/// `h = 360`.
pub fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let f_max = r.max(g.max(b));
    let f_min = r.min(g.min(b));
    let v = f_max;

    if f_max != 0.0 {
        let s = (f_max - f_min) / f_max;
        let delta = f_max - f_min;
        // When delta is zero (r == g == b != 0) hue is undefined; we
        // start from 0 to get deterministic behaviour.
        let mut h = 0.0f32;
        if delta != 0.0 {
            if r == f_max {
                h = (g - b) / delta;
            } else if g == f_max {
                h = 2.0 + (b - r) / delta;
            } else if b == f_max {
                h = 4.0 + (r - g) / delta;
            }
        }
        h *= 60.0;
        if h < 0.0 {
            h += 360.0;
        }
        (h, s, v)
    } else {
        // Pure black: s = 0, h = 360.
        (360.0, 0.0, v)
    }
}

// ---------- tests -------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_to_index_basic() {
        assert_eq!(bit_to_index(1), 0);
        assert_eq!(bit_to_index(0x8000), 15);
        // Multiple bits set — returns the lowest index.
        assert_eq!(bit_to_index(0x0006), 1);
        // No bits set — returns 0.
        assert_eq!(bit_to_index(0), 0);
    }

    #[test]
    fn count_number_of_bits_basic() {
        assert_eq!(count_number_of_bits(0xF0F0), 8);
        assert_eq!(count_number_of_bits(0), 0);
        assert_eq!(count_number_of_bits(0xFFFF), 16);
        assert_eq!(count_number_of_bits(0x1), 1);
    }

    #[test]
    fn hex_ascii_to_integer_basic() {
        assert_eq!(hex_ascii_to_integer(b"ff"), 255);
        assert_eq!(hex_ascii_to_integer(b"DEADBEEF"), 0xDEAD_BEEFu32);
        assert_eq!(hex_ascii_to_integer(b"0"), 0);
        assert_eq!(hex_ascii_to_integer(b"1A2b"), 0x1A2B);
        // Unknown characters are ignored.
        assert_eq!(hex_ascii_to_integer(b"zz10"), 0x10);
    }

    #[test]
    fn base_name_offset_basic() {
        let s: &[u8] = b"a/b/c.txt";
        assert_eq!(&s[base_name_offset(s)..], b"c.txt");

        let noslash: &[u8] = b"no slash";
        assert_eq!(&noslash[base_name_offset(noslash)..], b"no slash");

        // Trailing slash: basename is empty, offset == len.
        let trail: &[u8] = b"dir/";
        assert_eq!(base_name_offset(trail), trail.len());

        // Empty string.
        let empty: &[u8] = b"";
        assert_eq!(base_name_offset(empty), 0);
    }

    #[test]
    fn hsv_rgb_roundtrip() {
        // Orange-ish; s and v nonzero, h inside bucket 0.
        let (h0, s0, v0) = (30.0f32, 0.5f32, 0.8f32);
        let (r, g, b) = hsv_to_rgb(h0, s0, v0);
        let (h, s, v) = rgb_to_hsv(r, g, b);
        assert!((h - h0).abs() < 1e-3, "h {} vs {}", h, h0);
        assert!((s - s0).abs() < 1e-6, "s {} vs {}", s, s0);
        assert!((v - v0).abs() < 1e-6, "v {} vs {}", v, v0);

        // Achromatic path: s == 0 maps to r == g == b == v.
        let (r, g, b) = hsv_to_rgb(0.0, 0.0, 0.42);
        assert_eq!((r, g, b), (0.42, 0.42, 0.42));
    }
}
