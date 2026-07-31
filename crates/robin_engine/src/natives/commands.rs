//! Typed effects and engine barriers queued by script natives.
//!
//! Native functions mutate deterministic state synchronously through
//! [`NativeSessionCapabilities`](super::NativeSessionCapabilities). They only
//! queue work that crosses into presentation/external systems or needs a wider
//! `EngineInner` mutation context. The engine drains these streams after each
//! script step without changing their command order.
//!
//! - `EngineCommand` — camera, dialog, map, fade, minimap, outline, …
//! - `SoundCommand`  — sound source activate / suspend / destroy.
//! - `DeferredCommand` — wider-context game-logic follow-ups such as SelectPC,
//!   StopActor, FreezeAll, and patch application.

/// Ordered engine-bound commands queued by native functions for processing
/// after script execution. [`EngineCommand::domain`] distinguishes genuine
/// presentation from deterministic follow-up barriers.
#[derive(
    Debug, Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum EngineCommand {
    /// Smooth-scroll camera to a location's position.
    ///
    /// The native resolves and copies the point synchronously, matching the
    /// original game. Computed locations belong to the calling script VM and
    /// cannot safely be resolved later through the mission-global VM.
    /// Speed 2.0 for normal, custom for SlowlyTo variant.
    ScrollCameraTo { x: f32, y: f32, speed: f32 },
    /// Instantly jump camera to a location's synchronously copied position.
    JumpCameraTo { x: f32, y: f32 },
    /// Set desired zoom level (0.5, 1.0, or 2.0).
    SetZoomLevel { zoom: f32 },
    /// Start a dialog sequence.
    StartDialog { dialog_id: i32 },
    /// Show/hide the campaign map overlay.
    DisplayMap { show: bool },
    /// Toggle the debug console.
    DisplayConsole,
    /// Configure minimap dot appearance for an actor entity.
    CustomizeMinimapDisplay { actor_handle: i32, dot_type: i32 },
    /// Define a flat trajectory zone around a location sector.
    DefineFlatTrajectoryZone {
        location_handle: i32,
        apex_height: i32,
    },
    /// Select victory/defeat dialogue text.
    ChooseVictoryDefeatText { id: i32 },
    /// Display popup text by resource ID.
    DisplayPopupText { text_id: i32 },
    /// Display the Sherwood production report.
    DisplaySherwoodReport,
    /// Fade screen to black and back over `speed` frames.
    FadeToBlack { speed: i32 },
    /// Set outline/hidden entity rendering mode.
    SetOutlineDisplay { display: bool },
    /// Teleport actor to a new position (called by SetActorLocation
    /// and RecordEnterGame).  When `dest_layer_sector` is `Some`, the
    /// engine-side handler will also reconcile the projection-area
    /// obstacle + footstep material for the actor's new floor/sector
    /// after the layer/sector update.  `None` leaves them untouched
    /// (computed locations don't carry the destination's layer/sector).
    ///
    /// `spawn_elevation_probe`: when set, the engine-side handler
    /// evaluates the destination sector's projection-area top plane at
    /// that `(x, y)` and places the actor at `(x, y + z, z)` in 3D.
    /// The probe point is the *inside* destination (`(dx, dy)` of the
    /// enter-game target), not the spawn point — the spawn sits outside
    /// the map and would never match a projection area on its own.
    SetActorLocation {
        actor_handle: i32,
        x: f32,
        y: f32,
        dest_layer_sector: Option<(u16, u16)>,
        spawn_elevation_probe: Option<(f32, f32)>,
    },
    /// Mission won.
    Win { show_window: bool },
    /// Update information bars (blazon display, etc.).
    UpdateInformationBars,
    /// Trigger a hero speech barked line on `pc_id`.  Used by script
    /// native helpers that need engine-owned `hero_speaking` state.
    HeroSpeak {
        pc_id: crate::element::EntityId,
        expression: u16,
    },
    /// Flash a one-frame full-alpha outline on the given actor.
    /// The engine resolves the actor handle and routes the EntityId
    /// into `pending_side_effects.pending_mark_pc_ids` for the host to
    /// pick up this frame.
    MarkPc { actor_handle: i32 },
    /// Fire a scripted `MakeNoise`: broadcast a one-shot noise from a
    /// location so nearby NPCs react.  `layer` and `sector` identify the
    /// source projection area used to recover its terrain elevation; volume
    /// is derived from the noise type using the `NOISE_VOLUME_*` table.
    MakeNoise {
        noise_type: crate::ai::NoiseType,
        x: f32,
        y: f32,
        layer: u16,
        sector: u16,
    },
    /// Finish a script scroll-status update after the native has already
    /// written the canonical status synchronously. The engine-side barrier
    /// refreshes the minimap dot and (on `Opened`) forces the `BONUS_THREE`
    /// animation. `scroll_handle` is the actor script handle; status is in
    /// `0..=3` (Invisible/Visible/Taken/Opened) — both pre-validated by the
    /// native.
    SetScrollStatus { scroll_handle: i32, status: i32 },
    /// Crouch a PC via the full sequence/animation rewrite path:
    /// rewrite an active movement sequence to its crouched variant, or
    /// launch a brand-new `RHCOMMAND_CROUCH_DOWN` so the actor plays
    /// the crouch-down animation.  The native arm runs in `ScriptEffects`
    /// without the `EngineInner` borrow, so it queues this command for
    /// the engine to drain via `actor_make_crouched`.
    ScriptMakePCCrouched { actor_handle: i32 },
    /// Propagate generic Activate/Deactivate from the script-visible mobile
    /// handle to its non-entity RHElementMobile master.
    SetMobileActive { mobile_index: u16, active: bool },
}

/// Whether an ordered engine command is a genuine host-facing effect or a
/// deterministic follow-up that still needs a wider engine mutation context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptCommandDomain {
    Presentation,
    SimulationBarrier,
}

impl EngineCommand {
    /// Compiler-exhaustive queue classification. Adding a command variant must
    /// make an explicit architectural choice here.
    pub const fn domain(&self) -> ScriptCommandDomain {
        use EngineCommand::*;
        match self {
            ScrollCameraTo { .. }
            | JumpCameraTo { .. }
            | SetZoomLevel { .. }
            | DisplayMap { .. }
            | DisplayConsole
            | DisplayPopupText { .. }
            | DisplaySherwoodReport
            | UpdateInformationBars
            | SetOutlineDisplay { .. }
            | MarkPc { .. } => ScriptCommandDomain::Presentation,

            StartDialog { .. }
            | ChooseVictoryDefeatText { .. }
            | FadeToBlack { .. }
            | HeroSpeak { .. }
            | CustomizeMinimapDisplay { .. }
            | DefineFlatTrajectoryZone { .. }
            | SetActorLocation { .. }
            | Win { .. }
            | MakeNoise { .. }
            | SetScrollStatus { .. }
            | ScriptMakePCCrouched { .. }
            | SetMobileActive { .. } => ScriptCommandDomain::SimulationBarrier,
        }
    }
}

/// Commands queued by script natives for the engine's sound system.
/// The engine drains these after each script execution step.
#[derive(
    Debug, Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum SoundCommand {
    SuspendAll,
    ResumeAll,
    Activate(i32),
    Deactivate(i32),
    Destroy(i32),
    PlayJingle(crate::sound::Jingle),
}

/// Commands queued by script natives for the engine to process after
/// script execution. Analogous to `SoundCommand` but for game logic.
#[derive(
    Debug, Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum DeferredCommand {
    /// Complete `RHArtificialIntelligence::AddPatrolMember` by running the
    /// chief's virtual `InitializePatrol` synchronously at the script-native
    /// boundary. The native has already appended the theoretical member.
    AddAsSubordinateInitialize { chief: i32 },
    /// Execute `RHArtificialIntelligence::ClearPatrol` at the engine-owned
    /// script barrier. Clearing the chief and each member pointer is
    /// synchronous, and every default-state member immediately runs
    /// `ForceReturnToDuty`; that nested AI work needs SimulationContext and
    /// LevelAssets and therefore cannot be completed inside NativeContext.
    RemoveAllSubordinates { actor: i32 },
    /// Finish SelectActorPC(actor, select) after the native has already
    /// updated the canonical selection synchronously. `actor == 0` means
    /// "all PCs". The engine-side barrier performs action/sequence and
    /// Sherwood-interface side effects.
    SelectPC { actor: i32, select: bool },
    /// Stop the actor's current and pending sequence elements
    /// (script-level priority).
    StopActor { actor: i32 },
    /// Set the engine-global freeze flag.
    FreezeAll { freeze: bool },
    /// Toggle PC playability via `MSG_ENABLE_CHARACTER` /
    /// `MSG_DISABLE_CHARACTER`. The engine should update the portrait
    /// bar when processing this command.
    SetPlayable { actor: i32, playable: bool },
    /// Quit any active swordfight for the actor. Used when teleporting
    /// an actor to "honolulu" (SetActorLocation with null location).
    QuitSwordfight { actor: i32 },
    /// Remove any unconscious-stars titbit for the actor (only fires
    /// when the actor is no longer unconscious — `is_still_unconscious`
    /// is checked in the handler).  Used when a human actor is sent to
    /// honolulu (null location).
    RemoveUnconsciousStars { actor: i32 },
    /// Process patch effects produced by ApplyPatch/ResetPatch script natives.
    /// The patch state was already mutated in the native; this deferred command
    /// lets the engine apply the side effects (swap objects, toggle animations,
    /// invalidate background, etc.) with full access to EngineInner state.
    ProcessPatchEffects {
        patch_index: crate::patch::PatchIndex,
        effects: Vec<crate::patch::PatchEffect>,
    },
    /// Reset the actor's sprite to frame 0 of its current row.  Called
    /// from the `ResetAnim` script native.
    ResetSpriteFrame { actor: i32 },
    /// Position an actor inside a building: SetActive(false), move to
    /// the building's special layer + sector, teleport onto the first
    /// gate's `point_in`, and (for PCs) DisableAllActionsTemp.
    PutActorInBuilding { actor: i32, building: i32 },
    /// Clear every quick-action memory slot on a PC: walk
    /// `NUMBER_OF_QA_MEMORY` slots, call `SetQuickActionSequence(0, 0,
    /// i, 0xFFFFFFFF)` on each (deletes sequence, titbits, QUICKITOS),
    /// and `RemoveQuickActionTitbitsFor`.  The per-slot logic lives in
    /// the engine command path; we iterate here so the native keeps to
    /// entity-state writes.
    ClearAllQuickActionSlots { actor: i32 },
}
