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
//! - `DeferredCommand` — game-logic actions that need sequence manager
//!   or global engine state (SendMessage, SelectPC, StopActor, FreezeAll).

/// Ordered engine-bound commands queued by native functions for processing
/// after script execution. [`EngineCommand::domain`] distinguishes genuine
/// presentation from deterministic follow-up barriers.
#[derive(
    Debug, Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum EngineCommand {
    /// Smooth-scroll camera to a location entity's position.
    /// Speed 2.0 for normal, custom for SlowlyTo variant.
    ScrollCameraTo { location_handle: i32, speed: f32 },
    /// Instantly jump camera to a location entity's position.
    JumpCameraTo { location_handle: i32 },
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
    /// location so nearby NPCs react.  `layer` is the noise's world
    /// layer; volume is derived from the noise type using the
    /// `NOISE_VOLUME_*` table.
    MakeNoise {
        noise_type: crate::ai::NoiseType,
        x: f32,
        y: f32,
        layer: u16,
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
    /// SendMessage / SendMessageWithArguments. The engine must turn this
    /// into a one-element `Command::SendMessage` sequence; it is not a
    /// direct `ProcessMessage` callback request.
    SendMessage {
        actor: i32,
        message: i32,
        arg1: i32,
        arg2: i32,
    },
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
    /// Handle death for an actor whose life points reached 0.
    /// EngineInner should set death posture, quit swordfight, play dying animation.
    HandleDeath { actor: i32 },
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
    /// engine/commands.rs; we iterate here so the native keeps to
    /// entity-state writes.
    ClearAllQuickActionSlots { actor: i32 },
    /// Launch a low-priority `RHCOMMAND_WAIT` sequence element on the
    /// actor: build a fresh wait at `RHPRIORITY_WAIT` and feed it into
    /// the sequence manager so the instruct arbitration kicks the
    /// actor out of any already-running sequence at lower-or-equal
    /// priority.  Used by every `Set*` posture/action-state script
    /// native (which calls `Wait()` after stamping the new state) so
    /// the actor doesn't continue executing whatever command was
    /// running before the script poked at it.
    LaunchWait { actor: i32 },
    /// Apply a scripted life-points write through the full
    /// `combat::set_life_points` pipeline.  Clamps negative values to
    /// zero, ignores already-dead actors, blocks Sherwood-PC damage,
    /// stores max life for invulnerable actors, and runs the death
    /// pipeline (`Kill`) when the actor reaches zero.  The PC override
    /// fires HERO_DIE / HERO_HURT cues on a drop.  No damage titbit is
    /// emitted on the script call site.
    SetScriptedLifePoints { actor: i32, amount: i32 },
    /// Apply a scripted concussion write through the full
    /// `EngineInner::apply_concussion` pipeline.  Clamps to
    /// `[0, CONCUSSION_MAX]`, honours invulnerability/Sherwood guards,
    /// preserves wakeup threshold for tied/carried (script-locked is
    /// bypassed because `force_value` is `true`), toggles unconscious
    /// state, quits swordfight on KO, adds unconscious-stars titbit,
    /// and dispatches `EVENT_FITAGAIN` on wakeup.
    SetScriptedConcussion {
        actor: i32,
        amount: i32,
        force_value: bool,
    },
    /// Stop the actor's current and pending sequence elements at a
    /// caller-specified priority, used outside the script-level
    /// `StopActor` flow — currently used by `SetActorPosture`'s `ID_KO`
    /// arm which calls `Stop(RHPRIORITY_INJURY)` before stamping the
    /// lying posture so any in-flight preference/normal-priority
    /// sequence is torn down at the correct level.
    StopActorAtPriority {
        actor: i32,
        priority: crate::sequence::SequencePriority,
    },
    /// NPC-only AI broadcast that fires when an NPC is forced into KO
    /// or tied posture from script.  Queues a
    /// `StimulusType::EventLoseConsciousness` on the NPC's own AI
    /// brain (`pending_stimuli`) and broadcasts the body as a
    /// DETECTABLE_BODY to every other NPC via
    /// `broadcast_body_detectable`.  Handler is a no-op for non-NPC
    /// actors so the native can enqueue without re-checking.
    BroadcastLoseConsciousness { actor: i32 },
    /// NPC-only AI broadcast that fires when an NPC is brought back
    /// to upright from a LYING posture via script.  Walks every other
    /// NPC and removes the resurrected NPC from their
    /// `DETECTABLE_BODY` list so allies stop reacting to a "downed"
    /// friend.
    BroadcastResurrection { actor: i32 },
    /// Add a HIDDEN titbit attached to the given actor.  The
    /// script-level posture stamp on the `ID_ANONYMOUS_ARCHER` arm
    /// does not go through the stealth-command transition that
    /// normally adds the HIDDEN titbit, so this restores the visual
    /// "disguise" indicator above the actor.
    AddHiddenTitbitForActor { actor: i32 },
    /// Re-issue the in-flight `GoTo` for a patrolling NPC so a
    /// just-changed `default_path_walking_flags` (e.g. RUN ↔ WALK from
    /// `SetPathWalkingStyle`) takes effect mid-segment instead of
    /// waiting for the next waypoint to be reached.  The engine
    /// handler must build the per-tick `AiContext`, look up the
    /// current hiking-path waypoint, compute `WillStopAtNextWaypoint`,
    /// and call `ai.go_to(pos, default_flags | DONT_STOP if !will_stop, ctx)`.
    RelaunchPathAtNewSpeed { actor: i32 },
}
