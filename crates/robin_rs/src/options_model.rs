//! Options transactions and control policies used by the shared menu pages.

use robin_engine::graphic_config::GraphicConfig;
use robin_engine::sound_config::SoundConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SoundEdit {
    pub(crate) original: SoundConfig,
    pub(crate) working: SoundConfig,
}

impl SoundEdit {
    pub(crate) fn new(config: SoundConfig) -> Self {
        Self {
            original: config,
            working: config,
        }
    }

    pub(crate) fn changed(&self) -> bool {
        !sound_eq(&self.original, &self.working)
    }

    pub(crate) fn commit(self, requested: bool, target: &mut SoundConfig) -> bool {
        if requested {
            *target = self.working;
        }
        requested
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum SoundSetting {
    FxVolume,
    DialogueVolume,
    MusicVolume,
    CommentVolume,
    CommentFrequency,
}

impl SoundSetting {
    pub(crate) const fn requires_host_authority(self) -> bool {
        matches!(self, Self::CommentFrequency)
    }
}

pub(crate) fn sound_value_mut(config: &mut SoundConfig, setting: SoundSetting) -> &mut u16 {
    match setting {
        SoundSetting::FxVolume => &mut config.fx_volume,
        SoundSetting::DialogueVolume => &mut config.dialogue_volume,
        SoundSetting::MusicVolume => &mut config.music_volume,
        SoundSetting::CommentVolume => &mut config.exclamation_volume,
        SoundSetting::CommentFrequency => &mut config.amount_of_speaking,
    }
}

pub(crate) fn sound_eq(left: &SoundConfig, right: &SoundConfig) -> bool {
    left.music_volume == right.music_volume
        && left.dialogue_volume == right.dialogue_volume
        && left.fx_volume == right.fx_volume
        && left.exclamation_volume == right.exclamation_volume
        && left.amount_of_speaking == right.amount_of_speaking
        && left.sound_3d == right.sound_3d
        && left.sound_8bit == right.sound_8bit
        && (left.master_volume - right.master_volume).abs() < f32::EPSILON
        && left.music_muted == right.music_muted
        && left.fx_muted == right.fx_muted
}

/// Graphics page transaction, committed when the user accepts the edits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct GraphicsEdit {
    pub(crate) original: GraphicConfig,
    pub(crate) working: GraphicConfig,
}

impl GraphicsEdit {
    pub(crate) fn new(config: GraphicConfig) -> Self {
        Self {
            original: config.clone(),
            working: config,
        }
    }

    pub(crate) fn changed(&self) -> bool {
        !graphic_eq(&self.original, &self.working)
    }

    pub(crate) fn resolution_changed(&self) -> bool {
        resolution_changed(&self.original, &self.working)
    }

    pub(crate) fn commit(self, accepted: bool, target: &mut GraphicConfig) -> (bool, bool) {
        // An accepted widget edit requests reapplication even when a second
        // edit restores the value present when the page opened.
        if !accepted {
            return (false, false);
        }
        let resolution_changed = self.resolution_changed();
        *target = self.working;
        (true, resolution_changed)
    }
}

fn resolution_changed(original: &GraphicConfig, working: &GraphicConfig) -> bool {
    (working.resolution_x - original.resolution_x).abs() > 0.5
        || (working.resolution_y - original.resolution_y).abs() > 0.5
        || working.adaptive_widescreen != original.adaptive_widescreen
}

pub(crate) fn graphic_eq(left: &GraphicConfig, right: &GraphicConfig) -> bool {
    left.display_anim == right.display_anim
        && left.display_shadow == right.display_shadow
        && left.framed_view_cone == right.framed_view_cone
        && left.display_titbits == right.display_titbits
        && (left.resolution_x - right.resolution_x).abs() < 0.5
        && (left.resolution_y - right.resolution_y).abs() < 0.5
        && left.fullscreen == right.fullscreen
        && left.hardware_cursor == right.hardware_cursor
        && left.scale_mode == right.scale_mode
        && left.shader_preset == right.shader_preset
        && left.texture_effect == right.texture_effect
        && left.upscale_parameters == right.upscale_parameters
        && left.texture_effect_parameters == right.texture_effect_parameters
        && left.apply_fog_to_all_sprites == right.apply_fog_to_all_sprites
        && left.adaptive_widescreen == right.adaptive_widescreen
        && left.native_refresh_presentation == right.native_refresh_presentation
        && left.diplomacy_visuals == right.diplomacy_visuals
        && left.show_mission_countdown == right.show_mission_countdown
        && left.dynamic_ambience_visuals == right.dynamic_ambience_visuals
        && left.quick_action_cursor_pulse == right.quick_action_cursor_pulse
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum GraphicsSetting {
    AlphaVisionField,
    TransparentShadows,
    EffectAnimations,
    BackgroundAnimations,
    FogNightAllSprites,
    Fullscreen,
    HardwareCursor,
    AdaptiveWidescreen,
    NativeRefreshPresentation,
    MissionCountdown,
    DynamicAmbienceVisuals,
    DiplomacyVisuals,
    QuickActionCursorPulse,
    UpscaleStrength,
    UpscaleEdgeThreshold,
    UpscaleArtifactRemoval,
    EffectScanlines,
    EffectPhosphorMask,
    EffectBloom,
    EffectCurvature,
    EffectTemporalFlicker,
}

/// Common transaction controller. Drivers own geometry, event polling, and
/// effect execution; this controller owns page entry, acceptance and rollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum OptionsPage {
    Hub,
    Graphics,
    Sounds,
    Shortcuts,
    Gameplay,
    #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
    MultiplayerPrivacy,
}

#[derive(Clone, Serialize, Deserialize)]
enum PageSnapshot {
    None,
    Graphics(GraphicConfig),
    Sounds(SoundConfig),
    Shortcuts(crate::key_config::KeyConfig, crate::key_config::KeyConfig),
    Gameplay(robin_engine::gameplay_config::GameplayConfig),
    #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
    MultiplayerPrivacy(robin_engine::multiplayer_config::MultiplayerConfig),
}

/// Changes accepted by a page, including explicit reapplication of a setting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OptionsEffects {
    pub(crate) profile_changed: bool,
    pub(crate) resolution_changed: bool,
    pub(crate) keys_changed: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct OptionsController {
    pub(crate) graphic: GraphicsEdit,
    pub(crate) sound: SoundEdit,
    pub(crate) gameplay: robin_engine::gameplay_config::GameplayConfig,
    pub(crate) multiplayer: robin_engine::multiplayer_config::MultiplayerConfig,
    pub(crate) keys: crate::key_config::KeyConfig,
    pub(crate) custom_keys: crate::key_config::KeyConfig,
    pub(crate) page: OptionsPage,
    snapshot: PageSnapshot,
}

impl OptionsController {
    pub(crate) fn new(
        graphic: GraphicConfig,
        sound: SoundConfig,
        gameplay: robin_engine::gameplay_config::GameplayConfig,
        multiplayer: robin_engine::multiplayer_config::MultiplayerConfig,
        keys: crate::key_config::KeyConfig,
        custom_keys: crate::key_config::KeyConfig,
    ) -> Self {
        Self {
            graphic: GraphicsEdit::new(graphic),
            sound: SoundEdit::new(sound),
            gameplay,
            multiplayer,
            keys,
            custom_keys,
            page: OptionsPage::Hub,
            snapshot: PageSnapshot::None,
        }
    }

    pub(crate) fn enter_page(&mut self, page: OptionsPage) {
        assert_eq!(
            self.page,
            OptionsPage::Hub,
            "close the active options page before entering another"
        );
        self.snapshot = match page {
            OptionsPage::Hub => PageSnapshot::None,
            OptionsPage::Graphics => PageSnapshot::Graphics(self.graphic.working.clone()),
            OptionsPage::Sounds => PageSnapshot::Sounds(self.sound.working),
            OptionsPage::Shortcuts => {
                PageSnapshot::Shortcuts(self.keys.clone(), self.custom_keys.clone())
            }
            OptionsPage::Gameplay => PageSnapshot::Gameplay(self.gameplay),
            #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
            OptionsPage::MultiplayerPrivacy => PageSnapshot::MultiplayerPrivacy(self.multiplayer),
        };
        self.page = page;
    }

    pub(crate) fn accept_page(&mut self, reapply: bool) -> OptionsEffects {
        let mut effects = OptionsEffects::default();
        match &self.snapshot {
            PageSnapshot::Graphics(original) => {
                effects.profile_changed = reapply || !graphic_eq(original, &self.graphic.working);
                effects.resolution_changed = resolution_changed(original, &self.graphic.working);
            }
            PageSnapshot::Sounds(original) => {
                effects.profile_changed = reapply || !sound_eq(original, &self.sound.working)
            }
            PageSnapshot::Shortcuts(original, custom) => {
                effects.keys_changed =
                    reapply || original != &self.keys || custom != &self.custom_keys;
            }
            PageSnapshot::Gameplay(original) => {
                effects.profile_changed = reapply || *original != self.gameplay
            }
            #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
            PageSnapshot::MultiplayerPrivacy(original) => {
                effects.profile_changed = reapply || *original != self.multiplayer
            }
            PageSnapshot::None => {}
        }
        self.page = OptionsPage::Hub;
        self.snapshot = PageSnapshot::None;
        effects
    }

    pub(crate) fn cancel_page(&mut self) {
        match std::mem::replace(&mut self.snapshot, PageSnapshot::None) {
            PageSnapshot::Graphics(value) => self.graphic.working = value,
            PageSnapshot::Sounds(value) => self.sound.working = value,
            PageSnapshot::Shortcuts(active, custom) => {
                self.keys = active;
                self.custom_keys = custom;
            }
            PageSnapshot::Gameplay(value) => self.gameplay = value,
            #[cfg(all(feature = "multiplayer", not(target_arch = "wasm32")))]
            PageSnapshot::MultiplayerPrivacy(value) => self.multiplayer = value,
            PageSnapshot::None => {}
        }
        self.page = OptionsPage::Hub;
    }
}

#[cfg(test)]
pub(crate) fn shortcut_keys(
    config: &crate::key_config::KeyConfig,
) -> Vec<Option<winit::keyboard::KeyCode>> {
    let mut keys = vec![None; crate::key_config::REAL_KEY_COUNT as usize];
    config.get_keys_array(&mut keys);
    keys
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ShortcutPreset {
    Default,
    Alternate,
    Custom,
}

/// Clear every conflicting action,
/// except the intentional shared Shift binding for doors and quick planning.
pub(crate) fn assign_shortcut(
    config: &mut crate::key_config::KeyConfig,
    target: u16,
    key: winit::keyboard::KeyCode,
) {
    use crate::key_config::{GO_BEHIND_BUILDINGS_INDEX, PLAN_QUICK_ACTIONS_INDEX, REAL_KEY_COUNT};
    use winit::keyboard::KeyCode;
    assert!(target < REAL_KEY_COUNT, "shortcut action index must exist");
    for conflict in 0..REAL_KEY_COUNT {
        if conflict == target || config.get_key_by_index(conflict) != Some(key) {
            continue;
        }
        let shared_shift = matches!(key, KeyCode::ShiftLeft | KeyCode::ShiftRight)
            && matches!(
                (conflict, target),
                (GO_BEHIND_BUILDINGS_INDEX, PLAN_QUICK_ACTIONS_INDEX)
                    | (PLAN_QUICK_ACTIONS_INDEX, GO_BEHIND_BUILDINGS_INDEX)
            );
        if !shared_shift {
            config.set_key_by_index(conflict, None);
        }
    }
    config.set_key_by_index(target, Some(key));
    config.key_type = 1;
}

pub(crate) fn promote_shortcut_edits(
    active: &crate::key_config::KeyConfig,
    custom: &mut crate::key_config::KeyConfig,
    dirty: &mut bool,
) {
    if *dirty {
        *custom = active.clone();
        *dirty = false;
    }
}

/// Built-in presets preserve pending edits in the custom slot. Selecting Custom
/// restores that slot instead, intentionally discarding unpromoted edits.
pub(crate) fn select_shortcut_preset(
    active: &mut crate::key_config::KeyConfig,
    custom: &mut crate::key_config::KeyConfig,
    dirty: &mut bool,
    preset: ShortcutPreset,
) {
    use crate::key_config::KeyConfig;
    if preset != ShortcutPreset::Custom {
        promote_shortcut_edits(active, custom, dirty);
    }
    *active = match preset {
        ShortcutPreset::Default => KeyConfig::default_preset(),
        ShortcutPreset::Alternate => KeyConfig::alternate_preset(),
        ShortcutPreset::Custom => {
            let mut config = custom.clone();
            config.key_type = 1;
            config
        }
    };
    *dirty = false;
}

pub(crate) fn is_reserved_shortcut_key(key: winit::keyboard::KeyCode) -> bool {
    use winit::keyboard::KeyCode;
    matches!(
        key,
        KeyCode::PrintScreen
            | KeyCode::Escape
            | KeyCode::SuperLeft
            | KeyCode::SuperRight
            | KeyCode::ContextMenu
    )
}

pub(crate) fn adjust_graphics_setting(
    config: &mut GraphicConfig,
    setting: GraphicsSetting,
    delta: i32,
) -> bool {
    match setting {
        GraphicsSetting::AlphaVisionField => config.framed_view_cone = !config.framed_view_cone,
        GraphicsSetting::TransparentShadows => config.display_shadow = !config.display_shadow,
        GraphicsSetting::EffectAnimations => config.display_titbits = !config.display_titbits,
        GraphicsSetting::BackgroundAnimations => config.display_anim = !config.display_anim,
        GraphicsSetting::FogNightAllSprites => {
            config.apply_fog_to_all_sprites = !config.apply_fog_to_all_sprites
        }
        GraphicsSetting::Fullscreen => config.fullscreen = !config.fullscreen,
        GraphicsSetting::HardwareCursor => config.hardware_cursor = !config.hardware_cursor,
        GraphicsSetting::AdaptiveWidescreen => {
            config.adaptive_widescreen = !config.adaptive_widescreen
        }
        GraphicsSetting::NativeRefreshPresentation => {
            config.native_refresh_presentation = !config.native_refresh_presentation
        }
        GraphicsSetting::MissionCountdown => {
            config.show_mission_countdown = !config.show_mission_countdown
        }
        GraphicsSetting::DynamicAmbienceVisuals => {
            config.dynamic_ambience_visuals = !config.dynamic_ambience_visuals
        }
        GraphicsSetting::DiplomacyVisuals => config.diplomacy_visuals = !config.diplomacy_visuals,
        GraphicsSetting::QuickActionCursorPulse => {
            config.quick_action_cursor_pulse = !config.quick_action_cursor_pulse
        }
        GraphicsSetting::UpscaleStrength => {
            adjust_percent(&mut config.upscale_parameters.strength, delta)
        }
        GraphicsSetting::UpscaleEdgeThreshold => {
            adjust_percent(&mut config.upscale_parameters.edge_threshold, delta)
        }
        GraphicsSetting::UpscaleArtifactRemoval => {
            adjust_percent(&mut config.upscale_parameters.artifact_removal, delta)
        }
        GraphicsSetting::EffectScanlines => {
            adjust_percent(&mut config.texture_effect_parameters.scanlines, delta)
        }
        GraphicsSetting::EffectPhosphorMask => {
            adjust_percent(&mut config.texture_effect_parameters.phosphor_mask, delta)
        }
        GraphicsSetting::EffectBloom => {
            adjust_percent(&mut config.texture_effect_parameters.bloom, delta)
        }
        GraphicsSetting::EffectCurvature => {
            adjust_percent(&mut config.texture_effect_parameters.curvature, delta)
        }
        GraphicsSetting::EffectTemporalFlicker => adjust_percent(
            &mut config.texture_effect_parameters.temporal_flicker,
            delta,
        ),
    }
    true
}

fn adjust_percent(value: &mut u8, delta: i32) {
    *value = if delta < 0 {
        value.saturating_sub(5)
    } else if delta > 0 {
        value.saturating_add(5).min(100)
    } else {
        *value
    };
}

#[cfg(test)]
mod tests {
    fn controller() -> super::OptionsController {
        super::OptionsController::new(
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            crate::key_config::KeyConfig::default_preset(),
            crate::key_config::KeyConfig::alternate_preset(),
        )
    }

    #[test]
    fn graphics_acceptance_preserves_resolution_threshold_and_reapply_policy() {
        use super::*;
        for delta in [-1.0, -0.5, -0.25, 0.0, 0.25, 0.5, 1.0] {
            for horizontal in [false, true] {
                for reapply in [false, true] {
                    let mut editor = controller();
                    editor.enter_page(OptionsPage::Graphics);
                    let mut legacy = GraphicsEdit::new(editor.graphic.working.clone());
                    if horizontal {
                        legacy.working.resolution_x += delta;
                    } else {
                        legacy.working.resolution_y += delta;
                    }
                    editor.graphic.working = legacy.working.clone();
                    let expected = delta.abs() > 0.5;
                    assert_eq!(legacy.resolution_changed(), expected);
                    assert_eq!(editor.accept_page(reapply).resolution_changed, expected);
                }
            }
        }
        let mut editor = controller();
        editor.enter_page(OptionsPage::Graphics);
        editor.graphic.working.display_shadow = !editor.graphic.working.display_shadow;
        let effects = editor.accept_page(false);
        assert!(effects.profile_changed);
        assert!(!effects.resolution_changed);
    }

    #[test]
    fn cancel_restores_page_entry_not_the_start_of_the_options_session() {
        use super::*;
        let mut editor = controller();
        editor.enter_page(OptionsPage::Graphics);
        editor.graphic.working.adaptive_widescreen = !editor.graphic.working.adaptive_widescreen;
        assert!(editor.accept_page(false).resolution_changed);
        let accepted = editor.graphic.working.clone();
        editor.enter_page(OptionsPage::Graphics);
        editor.graphic.working.adaptive_widescreen = !editor.graphic.working.adaptive_widescreen;
        editor.cancel_page();
        assert!(graphic_eq(&editor.graphic.working, &accepted));
        assert!(editor.graphic.changed());
    }

    #[test]
    fn custom_preset_restores_stored_bindings_without_promoting_pending_edits() {
        let mut active = crate::key_config::KeyConfig::default_preset();
        let mut custom = active.clone();
        let original = shortcut_keys(&custom);
        assign_shortcut(&mut active, 0, winit::keyboard::KeyCode::F3);
        let mut dirty = true;
        select_shortcut_preset(&mut active, &mut custom, &mut dirty, ShortcutPreset::Custom);
        assert!(!dirty);
        assert_eq!(shortcut_keys(&active), original);
        assert_eq!(shortcut_keys(&custom), original);
        assert_eq!(active.key_type, 1);
    }

    #[test]
    fn cancelling_shortcuts_restores_active_and_custom_after_preset_promotion() {
        let mut editor = controller();
        let active = shortcut_keys(&editor.keys);
        let custom = shortcut_keys(&editor.custom_keys);
        let kinds = (editor.keys.key_type, editor.custom_keys.key_type);
        editor.enter_page(OptionsPage::Shortcuts);
        assign_shortcut(&mut editor.keys, 0, winit::keyboard::KeyCode::F6);
        let mut dirty = true;
        select_shortcut_preset(
            &mut editor.keys,
            &mut editor.custom_keys,
            &mut dirty,
            ShortcutPreset::Alternate,
        );
        editor.cancel_page();
        assert_eq!(shortcut_keys(&editor.keys), active);
        assert_eq!(shortcut_keys(&editor.custom_keys), custom);
        assert_eq!((editor.keys.key_type, editor.custom_keys.key_type), kinds);
    }

    #[test]
    fn shortcut_commit_detects_custom_only_changes_and_clean_acceptance() {
        let mut editor = controller();
        editor.enter_page(OptionsPage::Shortcuts);
        assign_shortcut(&mut editor.custom_keys, 0, winit::keyboard::KeyCode::F6);
        assert!(editor.accept_page(false).keys_changed);
        editor.enter_page(OptionsPage::Shortcuts);
        assert!(!editor.accept_page(false).keys_changed);
        editor.enter_page(OptionsPage::Shortcuts);
        editor.keys.key_type = if editor.keys.key_type == 1 { 0 } else { 1 };
        assert!(editor.accept_page(false).keys_changed);
    }

    use super::*;

    #[test]
    fn reverted_toggle_is_clean_but_explicit_reapply_remains_supported() {
        let mut config = GraphicConfig::default();
        let mut edit = GraphicsEdit::new(config.clone());
        for _ in 0..2 {
            adjust_graphics_setting(
                &mut edit.working,
                GraphicsSetting::QuickActionCursorPulse,
                1,
            );
        }
        assert!(!edit.changed());
        assert_eq!(edit.commit(true, &mut config), (true, false));
    }
}
