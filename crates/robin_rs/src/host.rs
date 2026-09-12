//! Host state — non-sim, non-serialized per-client transient.
//!
//! Lived as `robin_engine::engine::Host` during the early Rust port,
//! moved to robin_rs once engine code stopped depending on it. Each
//! client owns one `Host`; rollback snapshots ignore it (each client
//! reconstructs its own from hardware context). Engine reaches host
//! state only through input parameters and `SideEffects` outputs.

use robin_assets::frame_holder::{FrameHolder, PublishedFrameHolder};
use robin_assets::shipping_datadir::ShippingDatadir;
use robin_engine::coordinates::{
    MapPoint, MapSize, ScreenPoint, ScreenSize, ScreenVec, WorldPoint3D,
};
use robin_engine::element::EntityId;
use robin_engine::engine as engine_api;
use robin_engine::engine::{
    DrawOrder, FadeToBlack, GroundMarkSpriteData, InputState, PendingBgBlit, SideEffects,
    SoundCommand,
};
use robin_engine::game_operation::GameCode;
use robin_engine::markers as engine_markers;
use robin_engine::player_command as engine_player_command;
use robin_engine::sprite_variant::SpriteVariant;
#[cfg(test)]
use robin_engine::tactical_control::TacticalFormation;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::bg_cache::BackgroundDecals;
use crate::draw_manager::DrawManager;
use crate::key_config::KeyConfig;
use crate::pc_info_overlay::PcInfoOverlay;
use crate::sound::SoundManager;

mod frontend;
#[cfg(test)]
pub(crate) use frontend::{FrontendPreferenceEffects, QueueStripAnimations};
pub use frontend::{
    FrontendPreferences, FrontendPresentation, FrontendResources, HostFrontend, HostTitbitPreview,
    InteractionReset, ItemEffectPreview, PrintScreenRequest, QueueStripAnimation,
};
pub(crate) use frontend::{HostContextSnapshot, QueueStripIdentity};
mod effects;
mod transport;
mod viewport;
pub(crate) use effects::SherwoodTradingAccess;
pub use effects::{
    DeferredAudioRequest, HostAudio, HostEffectBatches, HostModalRequest, HostSignal,
};
pub(crate) use transport::CommittedSnapshotTransition;
pub use transport::{
    HostTransport, PendingSnapshotTransition, PendingSnapshotTransitionPayload, SnapshotSave,
};
pub use viewport::ViewportState;

const PANNEL_HEIGHT: f32 = engine_api::PANNEL_HEIGHT;

pub use crate::application::{
    ApplicationContext, ApplicationContextDiagnostic, ApplicationServicesDiagnostic,
    ReadyApplicationContext,
};

impl ApplicationContext {
    pub(crate) fn host_snapshot(&self) -> Result<HostContextSnapshot, String> {
        let preferences = self.with_active_profile_and_keys(|profile, keys| {
            FrontendPreferences::new(
                keys.active.clone(),
                keys.custom.clone(),
                profile.gameplay_config,
                &profile.graphic_config,
            )
        })?;
        Ok(HostContextSnapshot {
            shipping: self.shipping_arc()?,
            preferences,
        })
    }
}

#[derive(Default)]
pub struct HostScripting {
    pub lua_session: Option<crate::lua_session::LuaSession>,
}

/// Small process-host facade. Deterministic state remains in `Engine`; these
/// owners can be borrowed independently at the existing async/tick barriers.
///
/// Frontend state is deliberately explicit: `Host` must not dereference to
/// `HostFrontend`, or callers can accidentally regain process-wide authority.
///
/// Independent owners can be borrowed without granting the whole host:
///
/// ```
/// use robin_rs::host::Host;
/// let mut host = Host::scratch(1024.0, 768.0);
/// let frontend = &mut host.frontend;
/// let effects = &mut host.effects;
/// frontend.resources.clear_background_decals();
/// robin_rs::blit_to_map::drain_pending_bg_blits(frontend, effects);
/// ```
///
/// ```compile_fail,E0609
/// use robin_rs::host::Host;
/// let mut host = Host::scratch(1024.0, 768.0);
/// host.input.controls.has_focus = false;
/// ```
///
/// ```compile_fail,E0308
/// use robin_rs::host::{Host, HostFrontend};
/// fn frontend_only(_: &mut HostFrontend) {}
/// let mut host = Host::scratch(1024.0, 768.0);
/// frontend_only(&mut host);
/// ```
#[derive(Default)]
pub struct Host {
    application_context: ApplicationContext,
    /// Launch policy belongs to this mission host, never to its rendering state.
    /// Unbound scratch/bootstrap hosts have no achievement-persistence authority.
    session_achievement_eligibility:
        Option<crate::session_achievement::SessionAchievementEligibility>,
    pub frontend: HostFrontend,
    pub transport: HostTransport,
    pub audio: HostAudio,
    pub effects: HostEffectBatches,
    pub scripting: HostScripting,
}

/// Borrowed presentation authority. Unlike `Host`, this cannot submit network
/// traffic, run scripts, enqueue application effects, or mutate audio state.
/// Process-local borrows are deliberately not serialized.
pub(crate) struct HostPresentation<'a> {
    pub(crate) frontend: &'a mut HostFrontend,
    pub(crate) sound: &'a crate::sound::SoundManager,
    pub(crate) options: &'a engine_api::GlobalOptions,
    pub(crate) local_seat: robin_engine::player_command::PlayerId,
    application: &'a ApplicationContext,
}

impl HostPresentation<'_> {
    /// Drawing cannot advance frontend state or escape into application services.
    pub(crate) fn draw(&self) -> HostDraw<'_> {
        HostDraw {
            frontend: self.frontend,
            sound: self.sound,
            options: self.options,
            local_seat: self.local_seat,
            graphic_config: self.graphic_config(),
            viewport: &self.frontend.viewport,
            draw_manager: self.frontend.presentation.draw_manager.clone(),
        }
    }

    /// Read only the active presentation settings, without granting storage,
    /// profile mutation, asset preparation, or other application authority.
    pub(crate) fn graphic_config(&self) -> robin_engine::graphic_config::GraphicConfig {
        self.application
            .with_active_profile(|profile| profile.graphic_config.clone())
            .unwrap_or_else(|error| panic!("rendering requires an active profile: {error}"))
    }
}

/// Immutable gameplay presentation inputs; GPU command buffers remain separately
/// mutable in the renderer. Serialization is diagnostic-only and cannot
/// reconstruct borrowed frontend authority.
pub(crate) struct HostDraw<'a> {
    viewport: &'a ViewportState,
    draw_manager: crate::draw_manager::DrawManager,
    pub(crate) frontend: &'a HostFrontend,
    pub(crate) sound: &'a crate::sound::SoundManager,
    pub(crate) options: &'a engine_api::GlobalOptions,
    pub(crate) local_seat: robin_engine::player_command::PlayerId,
    graphic_config: robin_engine::graphic_config::GraphicConfig,
}

impl Serialize for HostDraw<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // No live frontend, sound, options, or application state escapes.
        serializer.serialize_unit_struct("HostDraw")
    }
}

robin_util::deny_deserialize!(
    HostDraw<'_>,
    "draw authority must be borrowed from the live presentation host"
);

impl HostDraw<'_> {
    pub(crate) fn viewport(&self) -> &ViewportState {
        self.viewport
    }

    pub(crate) fn draw_manager(&self) -> &crate::draw_manager::DrawManager {
        &self.draw_manager
    }

    pub(crate) fn with_viewport<'a>(&'a self, viewport: &'a ViewportState) -> HostDraw<'a> {
        let mut draw_manager = self.draw_manager.clone();
        let view = viewport.view_position;
        let screen = viewport.screen_size;
        let zoom = viewport.zoom_factor;
        assert!(zoom > 0.0, "capture viewport requires a positive zoom");
        draw_manager.update_drawing_parameters(
            0,
            robin_engine::coordinates::MapBBox::from_coords(
                view.x,
                view.y,
                view.x + (screen.x - 1.0) / zoom,
                view.y + (screen.y - engine_api::PANNEL_HEIGHT + 1.0) / zoom,
            ),
            zoom,
        );
        HostDraw {
            frontend: self.frontend,
            sound: self.sound,
            options: self.options,
            local_seat: self.local_seat,
            graphic_config: self.graphic_config.clone(),
            viewport,
            draw_manager,
        }
    }

    pub(crate) fn graphic_config(&self) -> robin_engine::graphic_config::GraphicConfig {
        self.graphic_config.clone()
    }
}

impl Host {
    pub(crate) fn presentation(&mut self) -> HostPresentation<'_> {
        HostPresentation {
            frontend: &mut self.frontend,
            sound: &self.audio.sound,
            options: self.application_context.options(),
            local_seat: self.transport.local_seat(),
            application: &self.application_context,
        }
    }

    pub(crate) fn bind_session_achievement_eligibility(
        &mut self,
        eligibility: crate::session_achievement::SessionAchievementEligibility,
    ) -> Result<(), String> {
        if self.session_achievement_eligibility.is_some() {
            return Err("achievement eligibility is already bound to this mission host".into());
        }
        self.session_achievement_eligibility = Some(eligibility);
        Ok(())
    }

    pub(crate) fn session_achievement_eligibility(
        &self,
    ) -> Result<crate::session_achievement::SessionAchievementEligibility, String> {
        self.session_achievement_eligibility
            .ok_or_else(|| "mission host has no achievement eligibility launch policy".into())
    }

    pub fn preparation_files(&self) -> Result<&Arc<robin_engine::sbfile::SbFileSystem>, String> {
        self.application_context.preparation_files()
    }

    /// Read-only access preserves the initialized context of production hosts.
    /// Scratch hosts deliberately expose a bootstrap context with no storage authority.
    ///
    /// ```compile_fail
    /// use robin_rs::host::{ApplicationContext, Host};
    /// fn replace_context(host: &mut Host) {
    ///     host.application_context = ApplicationContext::default();
    /// }
    /// ```
    pub fn application_context(&self) -> &ApplicationContext {
        &self.application_context
    }

    /// Construct a production host only after application initialization.
    /// Snapshot failures (such as a poisoned service lock) remain recoverable.
    ///
    /// ```compile_fail
    /// use robin_rs::host::{ApplicationContext, Host};
    /// let _ = Host::new(ApplicationContext::default(), 800.0, 600.0);
    /// ```
    ///
    /// ```no_run
    /// use robin_rs::host::{ApplicationContext, Host, ReadyApplicationContext};
    /// fn construct(context: ApplicationContext) -> Result<Host, String> {
    ///     Host::new(ReadyApplicationContext::try_from(context)?, 800.0, 600.0)
    /// }
    /// ```
    pub fn new(
        application_context: ReadyApplicationContext,
        screen_width: f32,
        screen_height: f32,
    ) -> Result<Self, String> {
        let snapshot = application_context.host_snapshot()?;
        let frontend = HostFrontend::from_snapshot(snapshot, screen_width, screen_height);
        Ok(Self {
            application_context: application_context.into(),
            frontend,
            ..Default::default()
        })
    }

    /// Construct a host for deterministic replay/test paths which never read
    /// application persistence or shipping resources.
    pub fn scratch(screen_width: f32, screen_height: f32) -> Self {
        Self {
            application_context: ApplicationContext::default(),
            frontend: HostFrontend::scratch(screen_width, screen_height),
            ..Default::default()
        }
    }

    /// Reset host-side transient state after a save-load.  Mirrors the
    /// engine-side [`robin_engine::engine::Engine::restore`] fix-ups
    /// for the host half: a stale mid-drag rubber-band, a stale
    /// `focused_entity_id` pointing at a now-missing entity, or a
    /// UI-request queue partially drained before the load would all
    /// survive into the restored session without this wipe.  Called by
    /// [`crate::save_file::GameSaveFile::apply_to`] and by any future
    /// caller that swaps engines under a live host.
    ///
    /// Does NOT touch `SoundManager` — callers replace that wholesale
    /// from the save payload.
    pub fn post_load_reset(&mut self) {
        // Clear mouse/keyboard input state — otherwise a mid-drag
        // quick-load would leave the rubber-band box active with stale
        // screen coordinates, or keep a stale `focused_entity_id` that
        // no longer exists in the reloaded entity array.
        self.frontend.restore_snapshot();

        // Drop any UI-request queues that were in flight before the
        // load.  They live host-side now — accumulated from per-tick
        // `SideEffects.pending_*` by `Host::apply_side_effects`.
        self.effects.clear();
    }

    /// Apply engine outputs using only the frontend, audio and effect queues.
    pub fn apply_side_effects(&mut self, fx: SideEffects) -> GameCode {
        self.frontend.apply_side_effects(
            fx,
            &mut self.audio,
            &mut self.effects,
            &self.application_context,
            self.transport.local_seat(),
        )
    }

    pub fn sync_sound_listener(&mut self) {
        self.audio.sound.set_listen_point(
            self.frontend.viewport.sound_listen_point(),
            self.frontend.viewport.zoom_factor,
        );
    }
}

#[cfg(test)]
pub(crate) mod test_support;
