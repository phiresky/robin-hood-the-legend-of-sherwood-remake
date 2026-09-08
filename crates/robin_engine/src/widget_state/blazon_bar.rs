//! Blazon-bar HUD widget state.
//!
//! Tracks the current / required / blinking blazon counts and feeds the
//! icon strip render path.
//!
//! Immediate-mode: rather than holding a cached widget instance, we
//! recompute the blazon-bar state every frame from current campaign +
//! mission state via [`build_blazon_bar_state`].  The HUD render path
//! reads the returned [`BlazonBarState`] and draws the icon strip
//! directly.  The `UpdateInformationBars` engine-command handler (see
//! `engine/script.rs`) re-derives this on demand.
//!
//! ## Mission selection
//!
//! - Men-to-blazon conversion mode → prefer the armed blazon mission,
//!   fall back to the next mission.
//! - Otherwise → the armed blazon mission, else the current mission if
//!   it produces blazons.
//! - Returns `None` when the bar should be hidden (no relevant mission).

use crate::campaign::{Campaign, CampaignValue};

/// Per-frame snapshot of blazon-bar contents.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BlazonBarState {
    /// Current blazon count (campaign `Blazon` value).
    pub current: u32,
    /// Required blazons to win the attached mission
    /// (`MissionProfile::number_of_blazons_to_win`).
    pub required: u32,
    /// Of `required`, how many must be collected *inside* the mission
    /// itself (`MissionProfile::number_of_blazons_to_be_collected`).
    /// The trailing `to_be_collected` slots render as "castle"; the
    /// middle gap (unowned slots outside that trailing range) renders
    /// as "empty".
    pub to_be_collected: u32,
    /// Number of trailing castle blazons currently flashing to the
    /// "normal" sprite.  Passed in from the caller's
    /// [`crate::engine::Engine::active_blinking_blazons`] one-shot
    /// latch.
    pub blinking: u32,
    /// Extra blazons the men-to-blazon conversion preview should add to
    /// the displayed won count.  Derived here from
    /// `peasants_to_convert / quotation` whenever men-to-blazon
    /// conversion is active.
    pub additional: u32,
}

/// Compute the blazon-bar snapshot to display this frame.  Returns
/// `None` when the bar should be hidden.
///
/// `blinking` is the currently-armed blink count, read from
/// [`crate::engine::Engine::active_blinking_blazons`] by the caller.
/// `additional` is derived here from `peasants_to_convert / quotation`
/// whenever men-to-blazon conversion is active.
pub fn build_blazon_bar_state(
    campaign: &Campaign,
    profiles: &crate::profiles::ProfileManager,
    men_to_blazon_conversion: bool,
    blinking: u32,
) -> Option<BlazonBarState> {
    let mission_idx = pick_blazon_mission(campaign, profiles, men_to_blazon_conversion)?;
    let profile = campaign.missions[mission_idx].profile(profiles);
    let required = profile.number_of_blazons_to_win as u32;
    let to_be_collected = profile.number_of_blazons_to_be_collected as u32;
    let current = campaign.get_value(CampaignValue::Blazon).max(0) as u32;
    let additional = compute_additional_blazons(campaign, profiles, men_to_blazon_conversion);

    Some(BlazonBarState {
        current,
        required,
        to_be_collected,
        blinking,
        additional,
    })
}

/// Preview blazons the pending men-to-blazon conversion would add:
/// `peasants_to_convert / peasant_to_blazon_quotation`.  Returns 0
/// outside men-to-blazon conversion mode or when no blazon mission is
/// armed (the conversion-mode flag normally implies a blazon mission
/// is set, but defensively return 0 here when it is not).
fn compute_additional_blazons(
    campaign: &Campaign,
    profiles: &crate::profiles::ProfileManager,
    men_to_blazon_conversion: bool,
) -> u32 {
    if !men_to_blazon_conversion {
        return 0;
    }
    let Some(blazon_idx) = campaign.blazon_mission_idx else {
        return 0;
    };
    let quotation = campaign.missions[blazon_idx]
        .profile(profiles)
        .peasant_to_blazon_quotation;
    if quotation == 0 {
        return 0;
    }
    let peasants = campaign.get_number_of_peasants_to_convert_to_blazons(profiles) as u32;
    peasants / quotation as u32
}

/// Picks which mission the bar should track.
fn pick_blazon_mission(
    campaign: &Campaign,
    profiles: &crate::profiles::ProfileManager,
    men_to_blazon_conversion: bool,
) -> Option<usize> {
    let blazon_idx = campaign.blazon_mission_idx;
    let current_idx = campaign.current_mission_idx;

    if men_to_blazon_conversion {
        // Prefer the blazon mission, else the next mission.
        if let Some(idx) = blazon_idx {
            return Some(idx);
        }
        return campaign.next_mission_idx;
    }

    if let Some(idx) = blazon_idx {
        return Some(idx);
    }

    // Not preparing a consuming blazon mission — only show if the
    // current mission produces blazons (ATTACK / TACTICAL).
    let idx = current_idx?;
    if campaign.missions[idx].produces_blazons(profiles) {
        Some(idx)
    } else {
        None
    }
}
