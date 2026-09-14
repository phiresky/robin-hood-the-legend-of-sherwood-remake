//! Native operations that require engine ownership beyond the VM's short borrow.
//!
//! These values belong to the current native invocation. The driver completes
//! them before resuming the VM; only actual backend output remains buffered.

/// Engine operation requested by one suspended native invocation.
#[derive(Debug, Clone, PartialEq)]
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
    /// obstacle for an ordinary actor-location change. The original game's obstacle update
    /// deliberately preserves the existing footstep material.
    /// RecordEnterGame (`spawn_elevation_probe.is_some()`) preserves the
    /// actor's existing obstacle, plane, and material, matching Original's
    /// outside-spawn path. A `None` destination topology also leaves them
    /// untouched (computed locations don't carry layer/sector metadata).
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
        dest_layer_sector: Option<(u16, crate::position_interface::SectorHandle)>,
        spawn_elevation_probe: Option<(f32, f32)>,
    },
    /// Mission won.
    Win { show_window: bool },
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
    /// location so nearby NPCs react. `layer` and the exact retained `sector`
    /// identity select the source projection area used to recover its terrain
    /// elevation; volume is derived from the noise type using the
    /// `NOISE_VOLUME_*` table.
    MakeNoise {
        noise_type: crate::ai::NoiseType,
        x: f32,
        y: f32,
        layer: u16,
        sector: crate::position_interface::SectorHandle,
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
    /// launch a brand-new crouch-down command so the actor plays
    /// the crouch-down animation before the script continues.
    ScriptMakePCCrouched { actor_handle: i32 },
    /// Propagate generic Activate/Deactivate from the script-visible mobile
    /// handle to its non-entity mobile-element master.
    SetMobileActive { mobile_index: u16, active: bool },
}

/// One native invocation's operation, completed before its VM resumes.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeCommand {
    Engine(EngineCommand),
    Sound(SoundCommand),
    World(WorldNativeCommand),
}

/// Sound-state operation completed before the native returns to its VM.
#[derive(Debug, Clone, PartialEq)]
pub enum SoundCommand {
    SuspendAll,
    ResumeAll,
    Activate(i32),
    Deactivate(i32),
    Destroy(i32),
    PlayJingle(crate::sound::Jingle),
}

/// World operation completed before the native returns to its VM.
#[derive(Debug, Clone, PartialEq)]
pub enum WorldNativeCommand {
    /// Complete original-game patrol-member addition by running the
    /// chief's patrol initialization synchronously at the script-native
    /// boundary. The native has already appended the theoretical member;
    /// `member_count` is the chief's theoretical-patrol length right after
    /// that append, so the barrier initializes over the same prefix the
    /// append saw rather than over the roster's final state.
    AddAsSubordinateInitialize { chief: i32, member_count: usize },
    /// Select or unselect through the canonical player-selection path,
    /// including portrait and action changes. `actor == 0` means all PCs.
    SelectPC { actor: i32, select: bool },
    /// Set the engine-global freeze flag.
    FreezeAll { freeze: bool },
    /// Toggle PC playability via `MSG_ENABLE_CHARACTER` /
    /// `MSG_DISABLE_CHARACTER`. The engine should update the portrait
    /// bar when processing this command.
    SetPlayable { actor: i32, playable: bool },
    /// Apply or reset a patch and complete its world effects before returning.
    ApplyPatch {
        patch_index: crate::patch::PatchIndex,
        reset: bool,
    },
    /// Reset the actor's sprite to frame 0 of its current row.  Called
    /// from the `ResetAnim` script native.
    ResetSpriteFrame { actor: i32 },
    /// Position an actor inside a building: deactivate, move to
    /// the building's special layer + sector, teleport onto the first
    /// gate's `point_in`, and (for PCs) temporarily disable all actions.
    PutActorInBuilding { actor: i32, building: i32 },
}
