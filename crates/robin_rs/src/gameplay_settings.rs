//! Stable gameplay-row identities and keyed presentation metadata.
//! Persisted GameplayConfig fields and simulation mutation remain unchanged.
use crate::localization::PortTextKey;
use serde::{Deserialize, Serialize};

// Stable row identity and order. Port-owned text lives in localization/catalog.rs.
macro_rules! settings {
    ($($name:ident),* $(,)?) => {
        #[repr(usize)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        pub enum GameplaySetting { $($name),* }
        impl GameplaySetting {
            pub const ALL: [Self; [$(stringify!($name)),*].len()] = [$(Self::$name),*];
            pub const fn index(self) -> usize { self as usize }
            pub fn from_index(index: usize) -> Option<Self> { Self::ALL.get(index).copied() }
            pub(crate) const fn label_key(self) -> PortTextKey {
                match self {
                    Self::EnableSpellforgeMissions => PortTextKey::SpellforgeGameplayAllowLabel,
                    _ => PortTextKey::GameplayLabel(self),
                }
            }
            pub(crate) const fn tooltip_key(self) -> PortTextKey {
                match self {
                    Self::EnableSpellforgeMissions => PortTextKey::SpellforgeGameplayAllowTooltip,
                    _ => PortTextKey::GameplayTooltip(self),
                }
            }
        }
    };
}
settings! {
    FixHardReactionTimes,
    ControlTacticalUnits,
    EnableUnbinding,
    ShowProductionForecast,
    ReusableCloaks,
    CampaignPresentation,
    CleanHandsNpcKillsInvalidate,
    ShowDetailedXp,
    ShowSpeedrunTracker,
    ShowCleanHandsTracker,
    ShowGhostTracker,
    ShowPileOfBonesTracker,
    ShowNewAchievementTrackers,
    ShowAchievementBadges,
    ShowAchievementDebrief,
    TouchCameraGestures,
    SherwoodTrading,
    AutosaveEnabled,
    AppleCombatInterrupt,
    WaspReliableAcquisition,
    StoneGroundDistraction,
    StoneLongerRange,
    NetSelectiveImmunity,
    AleReliableDistraction,
    NoiseDistractionFeedback,
    PreviewAppleEffect,
    PreviewStoneDirectEffect,
    PreviewStoneDistractionArea,
    PreviewNetCaptureArea,
    PreviewNetCrumplePrediction,
    PreviewAleEffect,
    PreviewPurseEffect,
    PreviewWaspArea,
    DetailedSaveMetadata,
    EnableTimedMissions,
    EnableDynamicAmbience,
    Diplomacy,
    NpcFactionWars,
    MoreCombatGestures,
    GestureQualityDamage,
    ShowCombatGestureGuide,
    CombatGestureCoach,
    PlanQuickActions,
    FogOfWar,
    EnableSpellforgeMissions,
    ReversibleBackgroundPatches,
}
