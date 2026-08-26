//! Blazon-set layout + per-slot state.
//!
//! Immediate-mode: callers lay out and classify slots once per frame via
//! [`build_blazon_set_state`] and render the returned [`BlazonSetState`]
//! from the host side.
//!
//! The blazon-set widget appears as a grid of coat-of-arms icons in:
//!
//! - The Sherwood campaign-map short-mission-description hover tooltip
//!   — a tiny one-row strip.
//! - The pre-mission description modal when the mission requires
//!   blazons — a larger multi-row grid down the left side of the window.
//!
//! ## Slot classification
//!
//! Three-sprite split:
//!
//! - slots `0..owned`                         → [`BlazonSlotKind::Normal`]
//!   (won already)
//! - if `owned + to_be_collected < total`:
//!   - `owned..(total - to_be_collected)`     → [`BlazonSlotKind::Empty`]
//!     (bought/earned outside the mission)
//!   - `(total - to_be_collected)..total`     → [`BlazonSlotKind::Castle`]
//!     (must be picked up inside the mission)
//! - otherwise:
//!   - `owned..total`                         → [`BlazonSlotKind::Castle`]
//!
//! The one-shot blink latch flips the trailing `blinking` castle slots
//! back to [`BlazonSlotKind::Normal`] for `BLAZON_BLINK_TIMEOUT = 50`
//! frames.

use serde::{Deserialize, Serialize};

/// Gap between adjacent blazon icons.
pub const BLAZON_SPACING: u32 = 5;

/// Tiny icon size.
pub const TINY_BLAZON_WIDTH: u32 = 9;
pub const TINY_BLAZON_HEIGHT: u32 = 14;

/// Huge icon size.
pub const HUGE_BLAZON_WIDTH: u32 = 32;
pub const HUGE_BLAZON_HEIGHT: u32 = 42;

/// Blink latch duration in frames.
pub const BLAZON_BLINK_TIMEOUT: u32 = 50;

/// Sub-picture indices for the tiny and huge blazon resource packs.
pub const EMPTY_BLAZON_SUB: usize = 0;
pub const NORMAL_BLAZON_SUB: usize = 1;
pub const CASTLE_BLAZON_SUB: usize = 2;

/// Per-slot classification from the three-sprite split.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode,
)]
pub enum BlazonSlotKind {
    /// Already-owned blazon.
    Normal,
    /// Un-owned slot earned outside the mission.
    Empty,
    /// Un-owned slot collected inside the mission.
    Castle,
}

impl BlazonSlotKind {
    /// Sub-picture index for the tiny / huge blazon resource pack.
    pub fn sprite_sub(self) -> usize {
        match self {
            Self::Normal => NORMAL_BLAZON_SUB,
            Self::Empty => EMPTY_BLAZON_SUB,
            Self::Castle => CASTLE_BLAZON_SUB,
        }
    }
}

/// One blazon slot, laid out in screen-space with its kind already
/// resolved (blink latch applied).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode,
)]
pub struct BlazonSlotState {
    pub kind: BlazonSlotKind,
    /// Top-left x of the slot's bbox, in the same coordinate space as
    /// the `box_x` passed to [`build_blazon_set_state`].
    pub x: i32,
    /// Top-left y.
    pub y: i32,
}

/// Full immediate-mode snapshot of the blazon set.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, bitcode::Encode, bitcode::Decode,
)]
pub struct BlazonSetState {
    /// `true` when huge (32×42) sprites are in use; `false` for tiny
    /// (9×14).
    pub huge: bool,
    /// Single-slot width (icon width without spacing).
    pub slot_w: u32,
    /// Single-slot height (icon height without spacing).
    pub slot_h: u32,
    /// Per-slot layout + kind, in declaration order (0 = leftmost on
    /// the first row).
    pub slots: Vec<BlazonSlotState>,
}

/// Compute the blazon-set layout and per-slot state.
///
/// Parameters:
/// - `(box_x, box_y, box_w, box_h)` — the enclosing layout rectangle,
///   as i32/u32 scalars so the function stays coordinate-system-agnostic.
/// - `total_blazons` — `MissionProfile::number_of_blazons_to_win`.
/// - `owned_blazons` — current owned count (clamped to `total_blazons`).
///   The caller pre-sums any additional/preview blazons into this.
/// - `to_be_collected` —
///   `MissionProfile::number_of_blazons_to_be_collected`.
/// - `blinking` — trailing castle slots to flip to Normal.  Pass 0 when
///   the latch is not armed.
///
/// Returns an empty [`BlazonSetState`] when `total_blazons == 0`.
#[allow(clippy::too_many_arguments)]
pub fn build_blazon_set_state(
    box_x: i32,
    box_y: i32,
    box_w: u32,
    box_h: u32,
    total_blazons: u32,
    owned_blazons: u32,
    to_be_collected: u32,
    blinking: u32,
) -> BlazonSetState {
    if total_blazons == 0 {
        return BlazonSetState::default();
    }

    // ── Step 1: pick huge vs tiny + per-row count ──────────────────
    //
    // Try huge first, fall back to tiny when either all blazons
    // wouldn't fit in a single huge row or the box is too short for a
    // huge icon.
    let huge_per_row = box_w / (HUGE_BLAZON_WIDTH + BLAZON_SPACING);
    let mut huge = true;
    let mut per_row = huge_per_row;
    if per_row <= total_blazons || box_h < HUGE_BLAZON_HEIGHT {
        per_row = box_w / (TINY_BLAZON_WIDTH + BLAZON_SPACING);
        huge = false;
    }
    let (slot_w, slot_h) = if huge {
        (HUGE_BLAZON_WIDTH, HUGE_BLAZON_HEIGHT)
    } else {
        (TINY_BLAZON_WIDTH, TINY_BLAZON_HEIGHT)
    };

    // In the huge case force `per_row = total` so the whole row lays
    // out inline.  Row height / blazon width are computed identically
    // for both sizes.
    if huge {
        per_row = total_blazons;
    }
    if per_row == 0 {
        // Box is narrower than even a single tiny icon.  Nothing to
        // lay out — return an empty state rather than dividing by zero.
        tracing::warn!(
            "blazon_set: box_w={} too narrow for any icon; skipping layout",
            box_w
        );
        return BlazonSetState::default();
    }
    let row_step_y = slot_h + BLAZON_SPACING;
    let col_step_x = slot_w + BLAZON_SPACING;

    let rows_full = total_blazons / per_row;
    let rows = rows_full
        + if !total_blazons.is_multiple_of(per_row) {
            1
        } else {
            0
        };

    // Assert the laid-out grid fits vertically inside the layout box.
    // Never fires in shipping data for either caller (tooltip: 1×14
    // tiny fits in 15; mission-description: rows stay within 338), but
    // keep the check as a guard.
    debug_assert!(
        (row_step_y * rows).saturating_sub(BLAZON_SPACING) <= box_h,
        "blazon_set: laid-out grid height {} exceeds box_h {}",
        (row_step_y * rows).saturating_sub(BLAZON_SPACING),
        box_h,
    );

    // ── Step 2: precompute classification thresholds ───────────────
    let owned = owned_blazons.min(total_blazons);
    let to_collect = to_be_collected.min(total_blazons);
    let castle_start = total_blazons - to_collect;
    // The blink latch is gated on `owned + to_be_collected <= total` —
    // when the two ranges overlap, drop the latch entirely.  Otherwise
    // the leading-castle flip below would still light up slots that
    // don't belong to the mission-collect range.
    let effective_blinking = if owned + to_be_collected <= total_blazons {
        blinking
    } else {
        0
    };
    let blink_active = effective_blinking > 0;
    // The blink flips the *leading* `clamped_blink` slots of the castle
    // range — indices in `[total − to_collect, total − to_collect +
    // clamped_blink)`.  Anchoring at the trailing end diverges
    // observably whenever `clamped_blink < to_collect` (the common case
    // — the sole caller passes the tactical-overflow exceeding count).
    let blink_count = effective_blinking.min(to_collect);
    let blink_end = castle_start + blink_count;

    // ── Step 3: lay out each row, centred within the box ───────────
    let mut slots = Vec::with_capacity(total_blazons as usize);
    for row in 0..rows {
        let row_blazon_count = if (row + 1) * per_row <= total_blazons {
            per_row
        } else {
            // Partial last row.
            per_row - ((row + 1) * per_row - total_blazons)
        };
        let row_w = row_blazon_count * col_step_x - BLAZON_SPACING;
        let row_x = box_x + ((box_w as i32 - row_w as i32) / 2);
        let row_y = box_y + (row_step_y * row) as i32;

        for col in 0..row_blazon_count {
            let idx = row * per_row + col;
            let kind = if idx < owned {
                BlazonSlotKind::Normal
            } else if idx < castle_start {
                // Only reachable when `owned + to_be_collected < total`;
                // otherwise `castle_start <= owned`.
                BlazonSlotKind::Empty
            } else if blink_active && idx >= castle_start && idx < blink_end {
                BlazonSlotKind::Normal
            } else {
                BlazonSlotKind::Castle
            };
            slots.push(BlazonSlotState {
                kind,
                x: row_x + (col * col_step_x) as i32,
                y: row_y,
            });
        }
    }

    BlazonSetState {
        huge,
        slot_w,
        slot_h,
        slots,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_set_when_zero_blazons() {
        let state = build_blazon_set_state(0, 0, 200, 100, 0, 0, 0, 0);
        assert!(state.slots.is_empty());
    }

    /// Single-row huge layout: box wide enough to fit all blazons at
    /// huge size on one row, which forces `per_row = total` and picks
    /// the huge sprite.
    #[test]
    fn huge_layout_fits_all_in_one_row() {
        let state = build_blazon_set_state(50, 40, 200, 100, 3, 1, 1, 0);
        assert!(state.huge);
        assert_eq!(state.slot_w, HUGE_BLAZON_WIDTH);
        assert_eq!(state.slot_h, HUGE_BLAZON_HEIGHT);
        assert_eq!(state.slots.len(), 3);
        // Row centred within the 200-wide box.
        //   row_w = 3 * (32 + 5) - 5 = 106
        //   row_x = 50 + (200 - 106)/2 = 50 + 47 = 97
        assert_eq!(state.slots[0].x, 97);
        assert_eq!(state.slots[0].y, 40);
        assert_eq!(state.slots[1].x, 97 + 32 + 5);
        assert_eq!(state.slots[2].x, 97 + 2 * (32 + 5));
        // Classification: 1 owned → Normal, 1 empty middle, 1 castle trailing.
        assert_eq!(state.slots[0].kind, BlazonSlotKind::Normal);
        assert_eq!(state.slots[1].kind, BlazonSlotKind::Empty);
        assert_eq!(state.slots[2].kind, BlazonSlotKind::Castle);
    }

    /// Narrow box forces tiny layout with multiple rows.
    #[test]
    fn tiny_layout_multi_row() {
        // 50-wide box: huge fits 50/(32+5)=1 per row, which is <= 10
        // total, so falls through to tiny.  Tiny per row: 50/(9+5) = 3.
        let state = build_blazon_set_state(0, 0, 50, 100, 10, 0, 0, 0);
        assert!(!state.huge);
        assert_eq!(state.slot_w, TINY_BLAZON_WIDTH);
        assert_eq!(state.slots.len(), 10);
        // Expect ceil(10/3) = 4 rows.  Last row has 10 - 3*3 = 1 slot.
        // Row height = 14 + 5 = 19.
        let last = state.slots.last().unwrap();
        assert_eq!(last.y, 3 * 19);
    }

    /// When `owned + to_be_collected >= total`, the else-branch sets
    /// every non-owned slot to Castle — no Empty slots.
    #[test]
    fn no_empty_when_owned_plus_collect_covers_total() {
        let state = build_blazon_set_state(0, 0, 300, 100, 4, 1, 3, 0);
        assert_eq!(
            state.slots.iter().map(|s| s.kind).collect::<Vec<_>>(),
            vec![
                BlazonSlotKind::Normal,
                BlazonSlotKind::Castle,
                BlazonSlotKind::Castle,
                BlazonSlotKind::Castle,
            ]
        );
    }

    /// The blink flashes the *leading* `blinking` castle slots back to
    /// Normal — indices in `[total − to_collect, total − to_collect +
    /// clamped_blink)`.
    #[test]
    fn blink_flips_leading_castle_to_normal() {
        let state = build_blazon_set_state(0, 0, 300, 100, 5, 1, 2, 2);
        // total=5, owned=1, to_be_collected=2 → castle at [3,4].
        // blinking=2 → both castle slots flash to Normal.
        let kinds: Vec<_> = state.slots.iter().map(|s| s.kind).collect();
        assert_eq!(
            kinds,
            vec![
                BlazonSlotKind::Normal, // owned
                BlazonSlotKind::Empty,  // middle gap
                BlazonSlotKind::Empty,
                BlazonSlotKind::Normal, // blinking castle (leading)
                BlazonSlotKind::Normal, // blinking castle
            ]
        );
    }

    /// When `clamped_blink < to_collect` the leading vs trailing
    /// distinction matters — flip is anchored at `total − to_collect`,
    /// not at `total − blinking`.  Audit case: `to_win=5,
    /// to_be_collected=3, exceeding=1` flashes the first castle slot
    /// (index 2), not the last.
    #[test]
    fn blink_anchors_at_castle_start() {
        let state = build_blazon_set_state(0, 0, 300, 100, 5, 0, 3, 1);
        // total=5, owned=0, to_be_collected=3 → castle at [2,3,4].
        // blinking=1 → flip slot 2 only (leading castle).
        let kinds: Vec<_> = state.slots.iter().map(|s| s.kind).collect();
        assert_eq!(
            kinds,
            vec![
                BlazonSlotKind::Empty,
                BlazonSlotKind::Empty,
                BlazonSlotKind::Normal, // blinking castle (leading)
                BlazonSlotKind::Castle,
                BlazonSlotKind::Castle,
            ]
        );
    }

    /// The blink-latch outer guard silently no-ops when
    /// `owned + to_be_collected > total`.  We honour that by clearing
    /// the latch before classification — without this guard the
    /// leading-castle flip would still mutate slots inside the
    /// owned-overlap range.
    #[test]
    fn blink_no_op_when_owned_overlaps_collect() {
        // total=4, owned=3, to_be_collected=2 → owned + collect = 5 > 4.
        // castle at [2,3]; blinking=1 should be ignored.
        let state = build_blazon_set_state(0, 0, 300, 100, 4, 3, 2, 1);
        let kinds: Vec<_> = state.slots.iter().map(|s| s.kind).collect();
        assert_eq!(
            kinds,
            vec![
                BlazonSlotKind::Normal, // owned
                BlazonSlotKind::Normal, // owned
                BlazonSlotKind::Normal, // owned
                BlazonSlotKind::Castle, // un-flipped — outer guard
            ]
        );
    }
}
