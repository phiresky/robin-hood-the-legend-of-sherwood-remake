//! NPC alert-level RGB colours for the view cone overlay.
//!
//! The shadow polygon tints the darkening overlay with these colours so
//! the player can read each NPC's alert state at a glance while alt-
//! hovering: bright green at rest, orange on alert, red in combat.

use crate::ai::AlertLevel;
use crate::element::EyeStatus;

/// Green alert base colour (RGB).
///
/// This is the colour the overlay fades towards when the NPC is calm.
pub const GREEN_ALERT: (u8, u8, u8) = (0x96, 0xFF, 0x64); // (150, 255, 100)

/// Yellow alert base colour — orange-ish, used when an NPC is
/// suspicious but hasn't gone hostile yet.
pub const YELLOW_ALERT: (u8, u8, u8) = (0xFF, 0xC8, 0x00); // (255, 200, 0)

/// Red alert base colour — used when the NPC is actively hostile.
pub const RED_ALERT: (u8, u8, u8) = (0xFF, 0x50, 0x00); // (255, 80, 0)

/// 32-step green→yellow interpolation table.
///
/// Filled at startup with `lambda = index / 36.0` — note the divisor is
/// 36 (not 32), so even the last entry is ~86 % toward yellow, never
/// pure yellow. We replicate that bias with integer arithmetic.
pub const GREEN_ALERT_TABLE: [(u8, u8, u8); 32] = {
    let mut table = [(0u8, 0u8, 0u8); 32];
    let (gr, gg, gb) = GREEN_ALERT;
    let (yr, yg, yb) = YELLOW_ALERT;
    let mut i = 0;
    while i < 32 {
        let w = i as u32; // lambda * 36
        let iw = 36 - w;
        table[i] = (
            ((iw * gr as u32 + w * yr as u32) / 36) as u8,
            ((iw * gg as u32 + w * yg as u32) / 36) as u8,
            ((iw * gb as u32 + w * yb as u32) / 36) as u8,
        );
        i += 1;
    }
    table
};

/// Compute the RGB tint for an NPC's shadow-polygon overlay.
///
/// `max_suspect` is `min(1000, max(maximal_detection_suspect, sorrow_level))`
/// (per `titbit_sync.rs`). `sorrow_level` is the raw sorrow level field
/// on the AI — it is re-mixed separately from `max_suspect` so we pass both.
pub fn npc_tint(
    alert: AlertLevel,
    eye_status: EyeStatus,
    view_radius: f32,
    standard_view_radius: f32,
    max_suspect: u16,
    sorrow_level: u16,
) -> (u8, u8, u8) {
    // `DieOrGetUnconscious` takes precedence over the alert level:
    // the overlay fades toward black (scaled red) as the eye status
    // degrades and the view radius shrinks.
    if eye_status == EyeStatus::DieOrGetUnconscious {
        let factor = (view_radius / standard_view_radius).clamp(0.0, 1.0);
        let factor = factor * factor;
        let (r, g, b) = RED_ALERT;
        return (
            (r as f32 * factor) as u8,
            (g as f32 * factor) as u8,
            (b as f32 * factor) as u8,
        );
    }

    match alert {
        AlertLevel::Green => {
            // Windows retail completes both floating-point scales before
            // converting to the 32-entry table index.
            // Promote the authored binary32 constant to model the
            // Windows x87 instruction sequence without intermediate
            // binary32 rounding.
            let scale = 0.001f32 as f64;
            let idx1 = (max_suspect as f64 * scale * 32.0) as usize;
            let idx2 = (sorrow_level as f64 * scale * 32.0) as usize;
            let idx1 = idx1.min(31);
            let idx2 = idx2.min(31);
            GREEN_ALERT_TABLE[idx1.max(idx2)]
        }
        AlertLevel::Yellow => YELLOW_ALERT,
        AlertLevel::Red => RED_ALERT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn green_base_value() {
        assert_eq!(GREEN_ALERT, (0x96, 0xFF, 0x64));
    }

    #[test]
    fn yellow_base_value() {
        assert_eq!(YELLOW_ALERT, (0xFF, 0xC8, 0x00));
    }

    #[test]
    fn red_base_value() {
        assert_eq!(RED_ALERT, (0xFF, 0x50, 0x00));
    }

    #[test]
    fn dying_fades_red_to_black() {
        let full = npc_tint(
            AlertLevel::Red,
            EyeStatus::DieOrGetUnconscious,
            400.0,
            400.0,
            0,
            0,
        );
        assert_eq!(full, RED_ALERT);

        let half = npc_tint(
            AlertLevel::Red,
            EyeStatus::DieOrGetUnconscious,
            200.0,
            400.0,
            0,
            0,
        );
        // factor = 0.5^2 = 0.25
        assert_eq!(half.0, (RED_ALERT.0 as f32 * 0.25) as u8);

        let zero = npc_tint(
            AlertLevel::Red,
            EyeStatus::DieOrGetUnconscious,
            0.0,
            400.0,
            0,
            0,
        );
        assert_eq!(zero, (0, 0, 0));
    }

    #[test]
    fn alert_levels_pick_right_base() {
        assert_eq!(
            npc_tint(
                AlertLevel::Green,
                EyeStatus::LookForward,
                400.0,
                400.0,
                0,
                0,
            ),
            GREEN_ALERT_TABLE[0],
        );
        assert_eq!(
            npc_tint(
                AlertLevel::Yellow,
                EyeStatus::LookForward,
                400.0,
                400.0,
                0,
                0,
            ),
            YELLOW_ALERT,
        );
        assert_eq!(
            npc_tint(AlertLevel::Red, EyeStatus::LookForward, 400.0, 400.0, 0, 0,),
            RED_ALERT,
        );
    }

    #[test]
    fn green_table_endpoints() {
        // Index 0 is pure green.
        assert_eq!(GREEN_ALERT_TABLE[0], GREEN_ALERT);
        // With `lambda = 31/36`, the last entry is ~86 % of the way
        // toward yellow but never exactly equal to YELLOW_ALERT.
        let last = GREEN_ALERT_TABLE[31];
        assert_ne!(last, YELLOW_ALERT);
        // Monotonic: red channel increases (green→yellow raises R).
        for w in GREEN_ALERT_TABLE.windows(2) {
            assert!(w[0].0 <= w[1].0);
        }
    }

    #[test]
    fn green_suspect_interpolates_across_table() {
        // Values at or above 1000 clamp to the final table entry.
        let hi = npc_tint(
            AlertLevel::Green,
            EyeStatus::LookForward,
            400.0,
            400.0,
            1500,
            0,
        );
        assert_eq!(hi, GREEN_ALERT_TABLE[31]);
        // Sorrow alone promotes the index.
        let sorrow = npc_tint(
            AlertLevel::Green,
            EyeStatus::LookForward,
            400.0,
            400.0,
            0,
            1500,
        );
        assert_eq!(sorrow, GREEN_ALERT_TABLE[31]);
        // Intermediate values select intermediate entries.
        let halfway = npc_tint(
            AlertLevel::Green,
            EyeStatus::LookForward,
            400.0,
            400.0,
            500,
            0,
        );
        assert_eq!(halfway, GREEN_ALERT_TABLE[16]);

        let calm = npc_tint(
            AlertLevel::Green,
            EyeStatus::LookForward,
            400.0,
            400.0,
            999,
            999,
        );
        assert_eq!(calm, GREEN_ALERT_TABLE[31]);
    }
}
