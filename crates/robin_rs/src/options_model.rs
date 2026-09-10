//! Shared graphics/sound option vocabulary and mutation policy. Both the original
//! layout adapter and the cooperative mission adapter resolve controls here.
//!
//! The transaction controller is shared; adapters retain layout/polling and
//! share one shortcut assignment and preset policy.

use robin_engine::graphic_config::{GraphicConfig, TextureEffect};
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

pub(crate) fn adjust_sound_setting(
    config: &mut SoundConfig,
    setting: SoundSetting,
    delta: i32,
    can_3d: bool,
    host_authority: bool,
) -> bool {
    if setting.requires_host_authority() && !host_authority {
        return false;
    }
    match setting {
        SoundSetting::ThreeDimensional if can_3d => config.sound_3d = !config.sound_3d,
        SoundSetting::ThreeDimensional => return false,
        SoundSetting::EightBit => config.sound_8bit = !config.sound_8bit,
        _ => {
            let value = sound_value_mut(config, setting);
            *value = i32::from(*value).saturating_add(delta).clamp(0, 9) as u16;
        }
    }
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum SoundSetting {
    ThreeDimensional,
    EightBit,
    FxVolume,
    DialogueVolume,
    MusicVolume,
    CommentVolume,
    CommentFrequency,
}

impl SoundSetting {
    pub(crate) const ALL: [Self; 7] = [
        Self::ThreeDimensional,
        Self::EightBit,
        Self::FxVolume,
        Self::DialogueVolume,
        Self::MusicVolume,
        Self::CommentVolume,
        Self::CommentFrequency,
    ];

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
        SoundSetting::ThreeDimensional | SoundSetting::EightBit => {
            panic!("boolean sound setting has no numeric value")
        }
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

/// Shared working/original graphics transaction; adapters retain their own
/// geometry, input scheduling and renderer preview effects.
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
        (self.working.resolution_x - self.original.resolution_x).abs() > 0.5
            || (self.working.resolution_y - self.original.resolution_y).abs() > 0.5
            || self.working.adaptive_widescreen != self.original.adaptive_widescreen
    }

    pub(crate) fn commit(self, accepted: bool, target: &mut GraphicConfig) -> (bool, bool) {
        // Compatibility dialogs request re-application after any accepted
        // widget edit, even when a second edit returns to the original value.
        // Cooperative adapters use changed() for their separate dirty policy.
        if !accepted {
            return (false, false);
        }
        let resolution_changed = self.resolution_changed();
        *target = self.working;
        (true, resolution_changed)
    }
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
    Resolution,
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
    ScalingMode,
    ShaderPreset,
    TextureEffect,
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

/// Accepted interactions in the original layout can explicitly reapply a
/// reverted setting. The cooperative adapter commits final-value differences.
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
                effects.resolution_changed = GraphicsEdit {
                    original: original.clone(),
                    working: self.graphic.working.clone(),
                }
                .resolution_changed();
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

/// Both layouts use the original binding rules: clear every conflicting action,
/// except the intentional shared Shift binding for doors and quick planning.
pub(crate) fn assign_shortcut(
    config: &mut crate::key_config::KeyConfig,
    target: u16,
    key: winit::keyboard::KeyCode,
) {
    use crate::key_config::{PLAN_QUICK_ACTIONS_INDEX, REAL_KEY_COUNT};
    use winit::keyboard::KeyCode;
    assert!(target < REAL_KEY_COUNT, "shortcut action index must exist");
    for conflict in 0..REAL_KEY_COUNT {
        if conflict == target || config.get_key_by_index(conflict) != Some(key) {
            continue;
        }
        let shared_shift = matches!(key, KeyCode::ShiftLeft | KeyCode::ShiftRight)
            && matches!(
                (conflict, target),
                (16, PLAN_QUICK_ACTIONS_INDEX) | (PLAN_QUICK_ACTIONS_INDEX, 16)
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

impl GraphicsSetting {
    pub(crate) const ALL: [Self; 25] = [
        Self::Resolution,
        Self::AlphaVisionField,
        Self::TransparentShadows,
        Self::EffectAnimations,
        Self::BackgroundAnimations,
        Self::FogNightAllSprites,
        Self::Fullscreen,
        Self::HardwareCursor,
        Self::AdaptiveWidescreen,
        Self::NativeRefreshPresentation,
        Self::MissionCountdown,
        Self::DynamicAmbienceVisuals,
        Self::DiplomacyVisuals,
        Self::QuickActionCursorPulse,
        Self::ScalingMode,
        Self::ShaderPreset,
        Self::TextureEffect,
        Self::UpscaleStrength,
        Self::UpscaleEdgeThreshold,
        Self::UpscaleArtifactRemoval,
        Self::EffectScanlines,
        Self::EffectPhosphorMask,
        Self::EffectBloom,
        Self::EffectCurvature,
        Self::EffectTemporalFlicker,
    ];
}

pub(crate) fn available_graphics_settings() -> Vec<GraphicsSetting> {
    graphics_settings_for_retroarch_availability(crate::shader_preset::retroarch_runtime_available())
}

pub(crate) fn graphics_settings_for_retroarch_availability(
    retroarch_available: bool,
) -> Vec<GraphicsSetting> {
    GraphicsSetting::ALL
        .into_iter()
        .filter(|setting| *setting != GraphicsSetting::ShaderPreset || retroarch_available)
        .collect()
}

fn cycle_index(current: usize, len: usize, delta: i32) -> usize {
    assert!(len > 0, "cannot cycle an empty option list");
    (current as i128 + i128::from(delta)).rem_euclid(len as i128) as usize
}

pub(crate) fn adjust_graphics_setting(
    config: &mut GraphicConfig,
    setting: GraphicsSetting,
    delta: i32,
) -> bool {
    match setting {
        GraphicsSetting::Resolution => {
            const MODES: &[(f32, f32)] = &[(640.0, 480.0), (800.0, 600.0), (1024.0, 768.0)];
            let current = MODES
                .iter()
                .position(|(w, h)| {
                    (config.resolution_x - w).abs() < 0.5 && (config.resolution_y - h).abs() < 0.5
                })
                .unwrap_or(0);
            let next = cycle_index(current, MODES.len(), delta);
            config.set_resolution(MODES[next].0, MODES[next].1);
        }
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
        GraphicsSetting::ScalingMode => {
            let modes = crate::shader_preset::available_texture_scale_modes();
            let current = modes
                .iter()
                .position(|mode| *mode == config.scale_mode)
                .unwrap_or(0);
            config.scale_mode = modes[cycle_index(current, modes.len(), delta)];
        }
        GraphicsSetting::ShaderPreset => {
            if !crate::shader_preset::retroarch_runtime_available() {
                return false;
            }
            let presets = crate::shader_preset::retroarch_presets();
            if !presets.is_empty() {
                let current = presets
                    .iter()
                    .position(|preset| preset.id == config.shader_preset)
                    .unwrap_or(0);
                config.shader_preset = presets[cycle_index(current, presets.len(), delta)]
                    .id
                    .clone();
            }
        }
        GraphicsSetting::TextureEffect => {
            let all = TextureEffect::ALL;
            let current = all
                .iter()
                .position(|effect| *effect == config.texture_effect)
                .unwrap_or(0);
            config.texture_effect = all[cycle_index(current, all.len(), delta)];
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

pub(crate) fn graphics_setting_label(
    config: &GraphicConfig,
    preset: &str,
    setting: GraphicsSetting,
) -> String {
    match setting {
        GraphicsSetting::Resolution => format!(
            "Resolution: {}x{}",
            config.resolution_x.round(),
            config.resolution_y.round()
        ),
        GraphicsSetting::AlphaVisionField => {
            toggle_label("Alpha Vision Field", !config.framed_view_cone)
        }
        GraphicsSetting::TransparentShadows => {
            toggle_label("Transparent Shadows", config.display_shadow)
        }
        GraphicsSetting::EffectAnimations => {
            toggle_label("Effect Animations", config.display_titbits)
        }
        GraphicsSetting::BackgroundAnimations => {
            toggle_label("Background Animations", config.display_anim)
        }
        GraphicsSetting::FogNightAllSprites => {
            toggle_label("Fog/Night All Sprites", config.apply_fog_to_all_sprites)
        }
        GraphicsSetting::Fullscreen => toggle_label("Fullscreen", config.fullscreen),
        GraphicsSetting::HardwareCursor => toggle_label("Hardware Cursor", config.hardware_cursor),
        GraphicsSetting::AdaptiveWidescreen => {
            toggle_label("Adaptive Widescreen", config.adaptive_widescreen)
        }
        GraphicsSetting::NativeRefreshPresentation => toggle_label(
            "Native-Refresh Presentation",
            config.native_refresh_presentation,
        ),
        GraphicsSetting::MissionCountdown => {
            toggle_label("Mission Countdown", config.show_mission_countdown)
        }
        GraphicsSetting::DynamicAmbienceVisuals => {
            toggle_label("Dynamic Ambience Visuals", config.dynamic_ambience_visuals)
        }
        GraphicsSetting::DiplomacyVisuals => {
            toggle_label("Diplomacy Colors", config.diplomacy_visuals)
        }
        GraphicsSetting::QuickActionCursorPulse => toggle_label(
            "Quick-Action Cursor Pulse",
            config.quick_action_cursor_pulse,
        ),
        GraphicsSetting::ScalingMode => format!("Scaling: {}", config.scale_mode.label()),
        GraphicsSetting::ShaderPreset => format!("Shader Preset: {preset}"),
        GraphicsSetting::TextureEffect => {
            format!("Texture Effect: {}", config.texture_effect.label())
        }
        GraphicsSetting::UpscaleStrength => {
            format!("Upscale Strength: {}%", config.upscale_parameters.strength)
        }
        GraphicsSetting::UpscaleEdgeThreshold => format!(
            "Upscale Edge Threshold: {}%",
            config.upscale_parameters.edge_threshold
        ),
        GraphicsSetting::UpscaleArtifactRemoval => format!(
            "Upscale Artifact Removal: {}%",
            config.upscale_parameters.artifact_removal
        ),
        GraphicsSetting::EffectScanlines => format!(
            "Effect Scanlines: {}%",
            config.texture_effect_parameters.scanlines
        ),
        GraphicsSetting::EffectPhosphorMask => format!(
            "Effect Phosphor Mask: {}%",
            config.texture_effect_parameters.phosphor_mask
        ),
        GraphicsSetting::EffectBloom => {
            format!("Effect Bloom: {}%", config.texture_effect_parameters.bloom)
        }
        GraphicsSetting::EffectCurvature => format!(
            "Effect Curvature: {}%",
            config.texture_effect_parameters.curvature
        ),
        GraphicsSetting::EffectTemporalFlicker => format!(
            "Effect Temporal Flicker: {}%",
            config.texture_effect_parameters.temporal_flicker
        ),
    }
}

pub(crate) fn toggle_label(label: &str, selected: bool) -> String {
    format!("{} {label}", if selected { "[x]" } else { "[ ]" })
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
    fn both_adapters_share_transactions_for_every_graphics_and_sound_setting() {
        use super::*;
        for setting in available_graphics_settings() {
            for accepted in [false, true] {
                let mut original = controller();
                let mut cooperative = controller();
                original.enter_page(OptionsPage::Graphics);
                cooperative.enter_page(OptionsPage::Graphics);
                // The legacy screen returns only its committed values; the
                // cooperative screen edits the page transaction in place.
                let mut page = GraphicsEdit::new(original.graphic.working.clone());
                adjust_graphics_setting(&mut page.working, setting, 1);
                let (changed, _) = page.commit(accepted, &mut original.graphic.working);
                original.accept_page(changed);
                adjust_graphics_setting(&mut cooperative.graphic.working, setting, 1);
                if accepted {
                    cooperative.accept_page(false);
                } else {
                    cooperative.cancel_page();
                }
                assert!(
                    graphic_eq(&original.graphic.working, &cooperative.graphic.working),
                    "{setting:?}"
                );
                assert_eq!(original.page, OptionsPage::Hub);
                assert_eq!(cooperative.page, OptionsPage::Hub);
            }
        }
        for setting in SoundSetting::ALL {
            for accepted in [false, true] {
                let mut original = controller();
                let mut cooperative = controller();
                original.enter_page(OptionsPage::Sounds);
                cooperative.enter_page(OptionsPage::Sounds);
                let mut page = SoundEdit::new(original.sound.working);
                adjust_sound_setting(&mut page.working, setting, 1, true, true);
                let changed = page.commit(accepted, &mut original.sound.working);
                original.accept_page(changed);
                adjust_sound_setting(&mut cooperative.sound.working, setting, 1, true, true);
                if accepted {
                    cooperative.accept_page(false);
                } else {
                    cooperative.cancel_page();
                }
                assert!(
                    sound_eq(&original.sound.working, &cooperative.sound.working),
                    "{setting:?}"
                );
            }
        }
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
    fn sound_adjustment_enforces_device_and_simulation_authority() {
        let mut config = SoundConfig::default();
        let original = config;
        assert!(!adjust_sound_setting(
            &mut config,
            SoundSetting::ThreeDimensional,
            1,
            false,
            true
        ));
        assert!(!adjust_sound_setting(
            &mut config,
            SoundSetting::CommentFrequency,
            1,
            true,
            false
        ));
        assert!(sound_eq(&config, &original));
        assert!(adjust_sound_setting(
            &mut config,
            SoundSetting::FxVolume,
            100,
            false,
            false
        ));
        assert_eq!(config.fx_volume, 9);
        assert_eq!(config.amount_of_speaking, original.amount_of_speaking);
        assert!(adjust_sound_setting(
            &mut config,
            SoundSetting::FxVolume,
            -100,
            false,
            false
        ));
        assert_eq!(config.fx_volume, 0);
    }

    #[test]
    fn cancelling_every_sound_setting_preserves_original_config() {
        for setting in SoundSetting::ALL {
            let mut config = SoundConfig::default();
            let original = config;
            let mut edit = SoundEdit::new(config);
            assert!(adjust_sound_setting(
                &mut edit.working,
                setting,
                1,
                true,
                true
            ));
            assert!(!edit.commit(false, &mut config));
            assert!(sound_eq(&config, &original), "{setting:?}");
        }
    }

    #[test]
    fn cancelling_every_available_graphics_setting_preserves_original_config() {
        for setting in available_graphics_settings() {
            let mut config = GraphicConfig::default();
            let original = serde_json::to_value(&config).unwrap();
            let mut edit = GraphicsEdit::new(config.clone());
            assert!(adjust_graphics_setting(&mut edit.working, setting, 1));
            assert_eq!(edit.commit(false, &mut config), (false, false));
            assert_eq!(
                serde_json::to_value(&config).unwrap(),
                original,
                "{setting:?}"
            );
        }
    }

    #[test]
    fn accept_classifies_only_resolution_policy_changes_as_resize() {
        for setting in available_graphics_settings() {
            let mut config = GraphicConfig::default();
            let mut edit = GraphicsEdit::new(config.clone());
            adjust_graphics_setting(&mut edit.working, setting, 1);
            let expected_resize = matches!(
                setting,
                GraphicsSetting::Resolution | GraphicsSetting::AdaptiveWidescreen
            );
            let expected = serde_json::to_value(&edit.working).unwrap();
            let (_, resize) = edit.commit(true, &mut config);
            assert_eq!(resize, expected_resize, "{setting:?}");
            assert_eq!(
                serde_json::to_value(config).unwrap(),
                expected,
                "{setting:?}"
            );
        }
    }

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

#[test]
fn numeric_sound_adjustment_clamps_even_extreme_deltas() {
    for setting in [
        SoundSetting::FxVolume,
        SoundSetting::DialogueVolume,
        SoundSetting::MusicVolume,
        SoundSetting::CommentVolume,
        SoundSetting::CommentFrequency,
    ] {
        for initial in [0, 5, 9, u16::MAX] {
            for delta in [i32::MIN, -1, 0, 1, i32::MAX] {
                let mut config = SoundConfig::default();
                *sound_value_mut(&mut config, setting) = initial;
                assert!(adjust_sound_setting(
                    &mut config,
                    setting,
                    delta,
                    true,
                    true
                ));
                assert_eq!(
                    *sound_value_mut(&mut config, setting),
                    (i64::from(initial) + i64::from(delta)).clamp(0, 9) as u16
                );
            }
        }
    }
}

#[test]
fn option_cycles_wrap_without_narrowing_indices_or_overflowing_deltas() {
    for len in 1..10 {
        for current in 0..len {
            for delta in -20..20 {
                let expected = (current as i32 + delta).rem_euclid(len as i32) as usize;
                assert_eq!(cycle_index(current, len, delta), expected);
            }
        }
    }
    assert_eq!(cycle_index(2, 3, i32::MAX), 0);
    assert_eq!(cycle_index(2, 3, i32::MIN), 0);
    assert_eq!(cycle_index(usize::MAX - 1, usize::MAX, 1), 0);
    assert_eq!(cycle_index(0, usize::MAX, -1), usize::MAX - 1);
}

#[test]
#[should_panic(expected = "cannot cycle an empty option list")]
fn empty_option_cycle_is_not_a_valid_zero_index() {
    cycle_index(0, 0, 1);
}
