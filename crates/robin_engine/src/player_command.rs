//! Serializable player commands for the replay / rollback pipeline.
//!
//! Every sim-affecting player action flows through [`PlayerCommand`].
//! The game session resolves raw platform input against read-only engine
//! state to produce fully-resolved commands, then feeds them to
//! [`EngineInner::apply_commands`].  The input system never holds `&mut EngineInner`.
//!
//! Commands are **resolved** — they carry entity IDs, map positions,
//! and command types determined at resolution time.  During replay,
//! the same commands are applied verbatim without re-resolving.

use crate::allied_control::{AlliedFormation, AlliedStance};
use crate::coordinates::{MapPoint, ScreenPoint, WorldPoint3D};
use crate::element::{Command, EntityId};
use crate::engine::EngineStateRequest;
use crate::profiles::Action;
use crate::sequence::Field;
use serde::{Deserialize, Serialize};

/// A single player-issued command that affects simulation state.
#[derive(Clone, Debug, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub enum PlayerCommand {
    /// No-op — signals that the input was consumed (e.g. an
    /// unrecognised swordfight gesture) without producing an action.
    Noop,

    // ── Movement ─────────────────────────────────────────────────
    /// Move a group of PCs to a destination.
    GroupMove {
        actors: Vec<EntityId>,
        destination: MapPoint,
        running: bool,
        /// Whether to show the click marker at the destination.
        /// Defaults true so older replay records keep mouse-click behaviour.
        #[serde(default = "default_true")]
        show_marker: bool,
        /// Optional explicit goal `(sector, layer)` that bypasses the
        /// spatial lookup at `destination`.  Used by host-side selected
        /// sector substitutions, including the patch-click flow that
        /// mirrors C++'s
        /// `pSectorGoal = mFastGrid.GetSector(patch.sector)` and
        /// `muwSelectedLayer = patch.layer` substitution at
        /// `RHengine.cpp:14201-14202`: when hovering a patch overlay,
        /// the move's goal sector and layer come from the patch's
        /// proto-loaded `(sector, final_layer)` rather than from a
        /// spatial query on the waypoint, which can pick the wrong
        /// layer (or no sector at all when the waypoint sits outside any
        /// registered geometry). Jump-sector hover also rewrites the
        /// selected sector to the underlying motion sector when no jump
        /// line is executable, matching `RHEngine::PerformMove`.
        goal_override: Option<(crate::sector::SectorNumber, u16)>,
        /// Replay-only exact arena identity for `goal_override`. Schema-16
        /// Original traces retain the sparse FastFindGrid slot; live input
        /// leaves this unset and uses the spatial hit's arena identity.
        #[serde(default)]
        goal_sector_index_override: Option<crate::fast_find_grid::SectorIndex>,
        /// Replay-only reconstruction of which Original route constructor was
        /// selected. `Some(false)` forces ordinary `AppendMoveToSequence`
        /// semantics even when Rust's spatial hit lands on a coincident door
        /// overlay; `Some(true)` forces the door-target constructor. Live
        /// input leaves this unset and uses the spatial selection normally.
        #[serde(default)]
        door_route_override: Option<bool>,
        /// Schema-16 replay-only ordinary gate routes, keyed by actor. Each
        /// gate is `(Original gate index, direct)`. Original has already run
        /// `FindPathGates` when it records this metadata; consuming that exact
        /// result avoids a second A* search choosing a different valid route
        /// and therefore authoring different building-exit waits/RNG draws.
        /// Live input leaves this empty.
        #[serde(default)]
        recorded_gate_routes: Vec<(EntityId, Vec<(u32, bool)>)>,
        /// Schema-16 replay-only actors whose authoritative Original
        /// `AppendMoveToSequence` gate search failed. Replaying that observed
        /// failure must not launch a second A* search against Rust topology.
        #[serde(default)]
        recorded_failed_gate_routes: Vec<EntityId>,
    },
    /// Stop a PC (clear path, set waiting).
    StopPc {
        pc_id: EntityId,
    },

    // ── Sequence-based interactions ──────────────────────────────
    /// Launch an interaction sequence (attack, heal, tie, search, etc.).
    ///
    /// `running`: seek with `RUNNING_UPRIGHT` instead of walking.
    /// Set true on double-click action paths.
    LaunchInteraction {
        actor: EntityId,
        target: EntityId,
        command: Command,
        running: bool,
    },
    /// Launch a ground-targeted ability (net, wasp nest, purse).
    ///
    /// `target_pos` is the full 3D point produced by the caller's
    /// 2D→3D projection lookup against the sight-obstacle area, so
    /// the titbit stamp and the `*_TARGET` sequence-field land on the
    /// same 3D coordinate the projectile itself will resolve to.
    ///
    /// `titbit_layer` is the layer argument threaded into the QA
    /// titbit — Purse/Wasp pass the currently selected layer, Net
    /// hard-codes `0`.  The caller resolves the correct value on the
    /// host side (where `selected_layer` is already tracked) so the
    /// engine handler never has to second-guess it.
    LaunchGroundTarget {
        actor: EntityId,
        target_pos: crate::coordinates::WorldPoint3D,
        command: Command,
        /// Which sequence property field to set the target position on.
        target_field: Field,
        /// Layer to stamp on the QA titbit (Purse/Wasp: currently
        /// selected layer; Net: `0`).
        titbit_layer: u16,
    },
    /// Launch a self-ability (whistle, eat, parry, drop corpse, etc.).
    LaunchSelfAbility {
        actor: EntityId,
        command: Command,
    },
    /// Click on a scroll-attached NPC — build the composite `LOCK_AI →
    /// turn-to-face (×2) → UNLOCK_AI → OPEN_SCROLL` sequence, prepend a
    /// Seek if the PC is out of range, and launch it.
    LaunchScrollRead {
        actor: EntityId,
        target: EntityId,
        /// Double-click bit — `true` makes the PC run to the NPC
        /// instead of walking.
        running: bool,
    },

    // ── Swordfight ───────────────────────────────────────────────
    /// Enter swordfight: engage an opponent.
    ///
    /// `running`: seek to the target with `RUNNING_UPRIGHT` animation
    /// instead of `WALKING_UPRIGHT` / `WALKING_CROUCHED`. Only set on
    /// the double-click-while-recording-macro branch; all other
    /// click paths want `running=false`.
    EnterSwordfight {
        actor: EntityId,
        target: EntityId,
        running: bool,
    },
    /// Sword strike on a specific target.
    SwordStrikeCmd {
        actor: EntityId,
        target: EntityId,
        command: Command,
        with_seek: bool,
        /// Exact tolerance resolved by the input source. Older commands did
        /// not carry it and retain the historical strike-specific fallback.
        #[serde(default)]
        seek_distance: Option<f32>,
    },
    /// Promote `opponent_id` to `actor`'s principal opponent (front of
    /// `human_data.opponents`).  Issued by the gamepad's swordfight
    /// A/B/C directional opponent cycle.
    SetPrincipalOpponent {
        actor: EntityId,
        opponent_id: EntityId,
    },

    // ── Action bar ───────────────────────────────────────────────
    /// Select an action from the portrait action bar.
    SelectAction {
        pc_id: EntityId,
        action_index: u32,
    },
    /// Select an already-resolved semantic action. Unlike `SelectAction`,
    /// this remains stable when a character's three portrait slots differ.
    SelectResolvedAction {
        pc_id: EntityId,
        action: Action,
    },
    /// Cancel the active action (set to NoAction).
    CancelAction {
        pc_id: EntityId,
    },
    /// Cancel action on all selected PCs (right-click unselect).
    UnselectAllActions,
    /// Right mouse button pushed down.  Sets
    /// `InputState.right_mouse_down` so any subsequent input that
    /// gates on a held right button (e.g. selection-drag extension)
    /// sees the correct state.  Issued by the gamepad CANCEL_PARADE
    /// press edge so gamepad parry-cancel goes through the same
    /// pipeline as the mouse.
    MouseRightDown,
    /// Right mouse button released.  Clears `InputState.right_mouse_down`.
    /// Paired with [`Self::MouseRightDown`] on the CANCEL_PARADE release
    /// edge so the held-state span brackets the press duration.
    MouseRightUp,
    /// Drain `pc_id`'s pending `Command::ShootBow` sequence elements.
    /// Right-clicking while Bow is armed drains any queued shots
    /// instead of cancelling the action — only an empty queue falls
    /// through to the action cancel.
    ClearShootList {
        pc_id: EntityId,
    },
    /// Drop ammo onto the ground.
    DropAmmo {
        pc_id: EntityId,
        action_id: u32,
        amount: u32,
    },
    /// Walk/run to `target_pos`, then drop a single ale bottle there.
    /// Resolves to a compound seek→drop-ale sequence engine-side.
    DropAleAt {
        actor: EntityId,
        target_pos: MapPoint,
        /// True selects `RUNNING_UPRIGHT` seek animation; comes from
        /// the double-click / record-QA matrix.
        running: bool,
        /// The parity recorder stores the center returned by Original's
        /// `FindAutorizedPosition`, while live input stores the raw cursor.
        /// Only an Original-trace replay sets this; older serialized commands
        /// retain the live/raw interpretation.
        #[serde(default)]
        already_authorized: bool,
        /// Authoritative route goal retained by a matching schema-16 route
        /// construction event. Spatially re-querying an authorized projected
        /// point can select an overlapping floor instead.
        #[serde(default)]
        goal_override: Option<(crate::sector::SectorNumber, u16)>,
        /// Replay-only exact sparse FastFindGrid identity for
        /// `goal_override`. Original compares sector pointers while public
        /// sector numbers are not unique, so retaining only the number can
        /// turn a valid cross-building route into no route at all.
        #[serde(default)]
        goal_sector_index_override: Option<crate::fast_find_grid::SectorIndex>,
    },
    /// Shield two-click protocol, first click: stash the focusable PC to
    /// protect in [`ShieldState::protected_pc`] and flip
    /// `is_protected = false` so the next click resolves the danger
    /// direction.  No sequence is launched.
    ShieldSelectProtected {
        /// The PC with the Shield action armed.  Captured for replay
        /// parity even though the engine reads the currently-selected
        /// PC from its own state.
        actor: EntityId,
        /// The focusable PC that the carrier will shield.
        protected_pc: EntityId,
    },
    /// Shield two-click protocol, second click (non-QA branch): set
    /// the danger point, launch the compound Seek(protected_pc, 50)
    /// → RaiseShield sequence, refresh the `DangerPoint` titbit, and
    /// deselect the Shield action.
    RaiseShieldWithDanger {
        /// The PC raising the shield.
        actor: EntityId,
        /// The PC whose defensive arc is being honoured — seeked to
        /// first.
        protected_pc: EntityId,
        /// The danger point toward which the shield is oriented.
        danger_point: MapPoint,
        /// Map layer the player was looking at when picking the danger
        /// point.  Differs from the carrier's own layer when the danger
        /// is across a chasm or off a balcony.  Stamped onto
        /// `Field::ShieldDangerPointLayer` so the danger-point titbit
        /// renders on the picked layer.
        danger_point_layer: u16,
    },

    // ── Posture ──────────────────────────────────────────────────
    /// Crouch down (applies to all selected PCs).
    CrouchDown,
    /// Stand up / leave disguise (applies to all selected PCs).
    StandUp,

    // ── Selection ────────────────────────────────────────────────
    SelectPc {
        pc_id: EntityId,
        append: bool,
    },
    TogglePcSelection {
        pc_id: EntityId,
    },
    /// Drop one specific PC from the selection.  Unlike
    /// [`Self::TogglePcSelection`] this never adds: a PC that is not
    /// currently selected is left alone and the selection-change
    /// follow-ups are skipped entirely.
    UnselectPc {
        pc_id: EntityId,
    },
    BoxSelect {
        pt1: MapPoint,
        pt2: MapPoint,
        shift: bool,
    },
    BoxUnselect {
        pt1: MapPoint,
        pt2: MapPoint,
    },
    SelectAllPcs,
    UnselectAllPcs,
    AssignQuickGroup {
        index: u8,
    },
    RecallQuickGroup {
        index: u8,
    },
    SelectByPortrait {
        portrait_index: u32,
        append: bool,
    },

    // ── Optional allied-soldier control ─────────────────────────
    SelectAlliedSoldiers {
        soldiers: Vec<EntityId>,
        append: bool,
    },
    BoxSelectAlliedSoldiers {
        pt1: MapPoint,
        pt2: MapPoint,
        shift: bool,
    },
    ClearAlliedSelection,
    PinAlliedSelection,
    UnpinAlliedGroup {
        group_id: u32,
    },
    SelectAlliedGroup {
        group_id: u32,
        append: bool,
    },
    PageAlliedPortraits {
        delta: i8,
    },
    MoveAlliedSoldiers {
        soldiers: Vec<EntityId>,
        destination: MapPoint,
        running: bool,
        formation: AlliedFormation,
    },
    SetAlliedStance {
        soldiers: Vec<EntityId>,
        stance: AlliedStance,
    },
    SetAlliedFormation {
        soldiers: Vec<EntityId>,
        formation: AlliedFormation,
    },
    SetAlliedPatrol {
        soldiers: Vec<EntityId>,
        destination: MapPoint,
        formation: AlliedFormation,
    },
    SetAlliedFollow {
        soldiers: Vec<EntityId>,
        hero: EntityId,
        formation: AlliedFormation,
    },
    ReleaseAlliedControl,

    // ── Special ──────────────────────────────────────────────────
    ResetComa {
        pc_id: EntityId,
    },
    /// Click on the trumpet indicator of a dead PC's portrait — dispatch
    /// `PcMessage::SendReinforcement` so the campaign spawns a replacement.
    SendReinforcement {
        pc_id: EntityId,
    },
    /// Double-click on a portrait while an action countdown is active:
    /// accelerate the targeted PC's active movement / sequence.  No
    /// separate QA-replay path is needed — recorded QA steps are
    /// dispatched as regular player commands, so the targeted PC's
    /// live sequence is already what `make_fast` acts on.
    MakePcFast {
        pc_id: EntityId,
    },
    /// Downgrade the targeted PC to walking. Counterpart to
    /// [`Self::MakePcFast`].
    MakePcSlow {
        pc_id: EntityId,
    },
    /// Stand the targeted PC up (rewrite queued crouched orders to
    /// upright variants).  When the PC has no active movement
    /// sequence, falls back to launching a `CROUCH_UP` element.
    MakePcUpright {
        pc_id: EntityId,
    },
    /// Crouch the targeted PC (rewrite queued upright orders to
    /// crouched variants).
    MakePcCrouched {
        pc_id: EntityId,
    },

    // ── Cutscene / engine state control ─────────────────────────
    /// Request an engine state change. Player viewport zoom/scroll is
    /// host-local; this remains for sim-visible state such as locker.
    /// Routed through `EngineInner::change_state` inside `apply_command`.
    ChangeState(EngineStateRequest),

    // ── Speed / pacing ───────────────────────────────────────────
    /// Enable fast-forward (slow-motion toggle).
    SetFastForward,

    // ── QA macro recording ──────────────────────────────────────
    /// Stop the currently-running macro recording, if any.
    StopRecordingMacro,
    /// Play back a recorded macro.  `pc = None` means "start the
    /// slot for every PC that has a macro there"; `pc = Some(id)`
    /// means "start just this PC's macro".  The engine clears each
    /// launched slot and — when *all* PCs finished slot N — runs the
    /// strip collapse pass.
    StartMacro {
        pc: Option<EntityId>,
        slot: u8,
    },
    /// Drop a recorded macro without replaying.  `pc = None` drops
    /// the slot for every PC that has one; `pc = Some(id)` drops
    /// just that PC's slot.  Unlike `StartMacro`, deletion does not
    /// run the strip collapse pass for a single-PC case.
    DeleteMacro {
        pc: Option<EntityId>,
        slot: u8,
    },
    /// Begin recording a quick-action macro.  `pc = None` arms
    /// recording on the first selected PC; `pc = Some(id)` targets
    /// that specific PC's portrait.  `slot` is the QA memory slot to
    /// record into (typically derived from the recording-place
    /// chooser).
    StartRecordingMacro {
        pc: Option<EntityId>,
        slot: u8,
    },
    /// Switch the active QA memory slot while a recording is in
    /// flight.  Ends the current recording, then arms a new one on
    /// the same selected PCs against the new slot.
    ChangeQaMemory {
        slot: u8,
    },
    /// Toggle the permanent alt-lock flag.  When true, the engine
    /// behaves as though alt is always held, enabling the view cone
    /// hover overlay without needing the key.  Host-driven from the
    /// sight HUD button.
    SetLockAlt(bool),

    /// Ctrl was pressed (the "move during action" modifier).  Saves
    /// the current action on every selected PC so the follow-on
    /// move command can run without the action overriding it, and
    /// ctrl-release can restore the saved action.  Routed through
    /// the command pipeline for replay determinism; the drain arm
    /// for `SimpleMessage::KeyControl` is the engine-internal mirror.
    KeyControl,

    /// Ctrl was released.  Restores each selected PC's action saved
    /// by the matching `KeyControl`.  No-op on macOS (ctrl is
    /// repurposed for stop-action there); honour the carve-out via
    /// `cfg` in the handler.
    KeyReleaseControl,

    // ── Per-frame aim orientation ──────────────────────────────
    /// Per-frame re-orientation of selected PCs toward the mouse
    /// map position while an aim/throw/help-climb/beggar action is
    /// active and the PC's animation hasn't committed to a throw
    /// yet.  Issued once per frame when the window has focus.
    /// Routed through the command pipeline so replay / rollback
    /// reproduce the same direction updates.
    PerformOrientation {
        mouse_map: MapPoint,
    },
    /// Apply a cursor-independent orientation operation already resolved by
    /// an authoritative input producer.
    PerformResolvedOrientation {
        pc_id: EntityId,
        action: Action,
        mouse_map: MapPoint,
        target: WorldPoint3D,
    },

    // ── Cheats ───────────────────────────────────────────────────
    /// `--goldeneye` CLI cheat: NPCs can't see PCs.  Issued once at
    /// startup so replay / rollback reproduces the same AI vision
    /// behaviour.
    SetGoldenEyeMode {
        on: bool,
    },

    // ── Host-driven sim mutations routed through the command pipeline
    //    so replay / rollback reproduces them deterministically. ──
    /// Toggle the engine-owned men-to-blazon conversion mode.
    SetMenToBlazonConversionMode {
        on: bool,
    },
    /// Register a generated peasant firstname+surname on the engine so
    /// subsequent peasant spawns don't reuse it.  Called from the UI
    /// panel when the civilian display-name generator picks a name.
    RegisterPeasantName {
        name: String,
    },
    /// Send a one-shot engine-level `ProcessMessage` to the global
    /// StartUp script.  Used e.g. by the Sherwood `GoToExit` button
    /// (msg=1000).
    DispatchStartupMessage {
        msg: i32,
        arg1: i32,
        arg2: i32,
    },
    /// `UNBLIP` console cheat — reveal all blipped entities.
    RevealAllBlips,
    /// Select (or clear with `None`) the next mission to play.
    /// Invoked from campaign-map / Sherwood HUD button handlers.
    CampaignSelectNextMission {
        mission_idx: Option<usize>,
    },
    /// Promote the campaign's pending missions to the accessible list.
    /// Runs when the player clicks the "show pending missions" choice
    /// on the campaign map.
    CampaignSwapPendingToAccessibleMissions,
    /// Harvest Sherwood's per-sector bonus counts + PC occupants back
    /// into the campaign before exiting Sherwood.  Invoked on the
    /// mission-start branch.
    CampaignHarvestProductionSectorState,
    /// Convert every peasant on the current mission team into blazons,
    /// removing their PC entities from the engine.  Dispatched from
    /// the Sherwood mission-start branch when the player committed
    /// the "men to blazon" mission-description choice.  Rolls each
    /// mission-team member's life-points against `2 * LIFEPOINTS_PC`
    /// on the deterministic sim RNG: successes move to reservists,
    /// failures are removed from the gang entirely.  The PC actor is
    /// removed from the engine in both cases.  After the loop the
    /// mission team is cleared and `BLAZON_VALUE` gains
    /// `num_to_convert / peasant_to_blazon_quotation`.  Routed
    /// through the command pipeline so the RNG-driven split replays
    /// deterministically.
    CampaignConvertSelectedPeasantsToBlazons,
    /// End-of-mission rollup: stat sync, coma reset, score bonuses,
    /// warcrime recruitment, blazon consumption.  Issued once after
    /// the engine tick returns a mission-end `GameCode`, before the
    /// debriefing is shown.
    ApplyQuitMissionUpdates {
        exit_code: crate::game_operation::GameCode,
        /// Difficulty selected by the application context that issued this
        /// deterministic command. Recruitment is the only quit-mission
        /// update affected by it.
        difficulty: crate::player_profile::DifficultyLevel,
    },
    /// Player confirmed the quit-mission popup.  Sets `quit_won`
    /// when the mission was already marked won (first-time-mission-
    /// won banner path) or `quit_interrupted` otherwise (normal
    /// abort path).  The tick's own flag-reading arms then convert
    /// these into the matching mission-end `GameCode`.
    QuitMissionRequested,
    /// F7 `TELEPORT` cheat — teleport every currently-selected PC to
    /// the mouse map position.  The first selected PC lands on
    /// `dest`; subsequent PCs keep their relative offset from it.
    /// Routed through the command pipeline so replay / rollback
    /// reproduces the teleport deterministically.
    TeleportSelectedToPoint {
        dest: MapPoint,
        /// Target layer (the currently selected layer).
        layer: u16,
        /// Target sector.  `None` leaves the sector unchanged — the
        /// teleport executor only overwrites `ElementData.sector`
        /// when this is `Some`.
        sector: Option<crate::position_interface::SectorHandle>,
    },

    // ── Minimap ─────────────────────────────────────────────────
    /// Window resize: reposition the minimap button / map boxes.
    MinimapResize {
        base: ScreenPoint,
        corner_size: crate::coordinates::ScreenSize,
    },
    /// Left mouse down on the minimap widget. Starts a drag when the
    /// map is deployed.
    MinimapMouseDown {
        click_pt: ScreenPoint,
        /// Whether host presentation had already started a drag before this
        /// edge. This fully resolves the UI-focus message decision so command
        /// application never derives engine output from host display scratch.
        continuing_drag: bool,
    },
    /// Mouse move — drives hover state, drag continuation, and the
    /// entered-nicely / capture flags.
    MinimapMouseMove {
        mouse_pt: ScreenPoint,
        left_mouse_down: bool,
        /// Whether the host-owned minimap drag is continuing on this move.
        /// Engine-visible focus output is derived from this recorded bit.
        continuing_drag: bool,
    },
    /// Left mouse up while interacting with the minimap (either the
    /// cursor is over the widget or a drag is in progress).  Handles
    /// the click / drag-end / center-on-map-click branch.
    MinimapMouseUp {
        on_minimap: bool,
    },
    /// Center the shared camera on a host-resolved map point. Minimap gesture
    /// presentation is deliberately represented by a separate command.
    CenterCameraOnPoint {
        point: MapPoint,
    },
    /// Right-click on the displayed minimap — close the map and clear
    /// pending highlights.
    MinimapRightClick,
    /// Accelerator key (bound via `SetAccelerator`) was pressed —
    /// toggle the map open or closed.
    MinimapToggle,

    // ── Display / UI setters ────────────────────────────────────
    /// Set the locker-mode follow target.  `None` clears the target
    /// and disables locker mode.
    SelectFollowElement {
        entity_id: Option<EntityId>,
    },
    /// Clear the one-shot `display_double_status_bar` flag on every
    /// NPC.  Issued after the bars have been rendered for the frame.
    ClearNpcDoubleStatusBarFlags,

    /// Change the authoritative hero-comment frequency immediately.
    /// The original `HeroSpeaking` reads the active profile setting on every
    /// call; carrying the edit as a command gives replay, rollback, and
    /// multiplayer the same application frame and RNG-flow gate.
    SetAmountOfSpeaking {
        /// Sound-menu slider value in the original 0..=9 range.
        amount: u16,
    },

    // ── Hero speech (side-effect feedback) ───────────────────────
    /// Trigger a hero speech barked line on `pc_id`.  Used by input
    /// handlers that need to emit a UX-feedback voice line when a
    /// click is discarded — e.g. plays `HERO_UNABLE_TO_DO_SOMETHING`
    /// when an AnonymousArcher tries to shoot an NPC.  The
    /// `expression` value is a `HERO_*` constant from `engine::melee`.
    HeroSpeak {
        pc_id: EntityId,
        expression: u16,
    },

    // ── Modal dismissal ─────────────────────────────────────────
    /// Result of a blocking modal that the host drained after the
    /// engine tick queued it (mission briefing `DisplayDialog`, the
    /// `DisplayPopupText` parchment scroll, etc.). Recorded so
    /// replays can auto-dismiss the modal instead of waiting for a
    /// human to click OK. Purely a host-side record —
    /// `EngineInner::apply_command` treats this as a no-op.
    ModalDismiss {
        kind: ModalKind,
        result: DialogResult,
    },

    // ── Seat lifecycle ───────────────────────────────────────────
    /// A peer (or the host on its behalf) announces that the
    /// referenced seat is now active.  Idempotent: re-running
    /// `ConnectSeat` for an existing seat updates the nickname and
    /// flips `connected = true` (drop-in/drop-out re-join).  The
    /// engine grows `seats` on demand via `ensure_seat`, so a
    /// `ConnectSeat` for an unseen `player_id` materialises a new
    /// `SeatState`.
    ///
    /// Recorded into the replay stream so seat-creation timing is
    /// data, not derived from transport state — that's what makes
    /// recordings byte-identical across machines.
    ConnectSeat {
        player_id: PlayerId,
        nickname: String,
    },
    /// A peer (or the host on its behalf) announces that the
    /// referenced seat has dropped out.  The seat's `SeatState`
    /// (selection, hotgroups) is preserved so the controlled PCs
    /// stay where they were left, on autopilot — drop-in/drop-out
    /// per `MULTIPLAYER.md` item 3.  A subsequent `ConnectSeat`
    /// with the same `player_id` re-arms the seat for that peer.
    DisconnectSeat {
        player_id: PlayerId,
    },
    /// Preserve `RHElementActorCivilian::MouseClicked`'s target-side
    /// cooldown stamp. A non-macro double-click discards the Pay interaction
    /// after recording only `make_pc_fast`, but this mutation still occurs.
    BeggarDontTalkStamp {
        beggar_id: EntityId,
    },
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::EntityIdKind;

    #[test]
    fn sword_seek_distance_roundtrips_and_missing_field_defaults_to_legacy_none() {
        let command = PlayerCommand::SwordStrikeCmd {
            actor: EntityId::new(3, EntityIdKind::Pc),
            target: EntityId::new(7, EntityIdKind::Soldier),
            command: Command::SwordstrikeThrustA,
            with_seek: true,
            seek_distance: Some(63.0),
        };
        let encoded = serde_json::to_value(&command).expect("serialize sword command");
        let decoded: PlayerCommand =
            serde_json::from_value(encoded.clone()).expect("roundtrip sword command");
        assert!(matches!(
            decoded,
            PlayerCommand::SwordStrikeCmd {
                seek_distance: Some(63.0),
                ..
            }
        ));

        let mut legacy = encoded;
        legacy
            .get_mut("SwordStrikeCmd")
            .and_then(serde_json::Value::as_object_mut)
            .expect("externally tagged sword command")
            .remove("seek_distance");
        let decoded: PlayerCommand =
            serde_json::from_value(legacy).expect("deserialize legacy sword command");
        assert!(matches!(
            decoded,
            PlayerCommand::SwordStrikeCmd {
                seek_distance: None,
                ..
            }
        ));
    }
}

/// Which blocking modal was dismissed, plus whatever identifier the
/// host needs to pair the dismissal with the specific script-queued
/// entry it came from.
///
/// Only modals that actually block the main loop and need replay
/// auto-dismiss are enumerated. The remaining drain sites
/// (short-briefings pane, other mission-state popups) share the same
/// pattern — add new variants as they get wired up.
#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum DebriefingTextId {
    Lose { index: usize },
    Win { index: usize },
}

impl DebriefingTextId {
    pub fn from_outcome(won: bool, index: usize) -> Self {
        if won {
            Self::Win { index }
        } else {
            Self::Lose { index }
        }
    }
}

#[derive(
    Clone, Debug, Eq, PartialEq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum ModalKind {
    /// Two-button dialogue window (`show_dialogue`), keyed by the
    /// script-assigned dialog id.
    Dialog { dialog_id: i32 },
    /// Single-OK popup scroll (`show_popup_scroll` via
    /// `DisplayPopupText`), keyed by the script text id.
    PopupText { text_id: i32 },
    /// Sherwood campaign stats report (`show_popup_scroll` variant
    /// dispatched from `DisplaySherwoodReport`). Unkeyed — only one
    /// in flight at a time.
    SherwoodReport,
    /// Mission debriefing page queued by `DisplayDebriefing`.
    Debriefing { text_id: DebriefingTextId },
    /// Final mission debriefing shown after the engine returns a
    /// mission exit code. Distinct from `Debriefing` because the final
    /// flow can also resolve to Restart or Load.
    FinalDebriefing { text_id: DebriefingTextId },
    /// Mission-state confirmation popup.
    MissionState { kind: MissionStateModalKind },
}

#[derive(
    Clone, Debug, Eq, PartialEq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum MissionStateModalKind {
    /// First-time mission-won prompt asking whether to leave now.
    LeaveMissionNow,
    /// End-state mission popup shown before the final debriefing.
    EndState { won: bool },
}

/// Outcome of a blocking modal.
///
/// Popup-scroll / single-button modals always record `Completed`;
/// `Aborted` is only meaningful for modals that distinguish a
/// play-through-all-sentences OK from an early Stop / Escape
/// (currently just `show_dialogue`).
#[derive(
    Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum DialogResult {
    /// Player saw every sentence / pressed OK / pressed Return.
    Completed,
    /// Player aborted (Stop / Escape / window close).
    Aborted,
    /// Final mission debriefing requested restart.
    Restart,
    /// Final mission debriefing requested loading a save slot.
    Load { slot: u32 },
}

/// Identifies which player issued a command. `LOCAL` (= 0) is the
/// implicit single-player default; multiplayer assigns distinct ids
/// per connected seat. The id is plumbed through the input stream,
/// replay file, and engine command-dispatch boundary so handlers can
/// authorise / route per-player commands. Viewport state is deliberately
/// host-local; engine seats carry deterministic selection/hotgroup state.
#[derive(
    Copy,
    Clone,
    Debug,
    Eq,
    PartialEq,
    Hash,
    Ord,
    PartialOrd,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub struct PlayerId(pub u8);

impl PlayerId {
    /// The host / first-joined seat.  Assigned by **join order**, not
    /// by which machine a recording was captured on:
    ///
    /// - In single-player, the only seat is `HOST`.
    /// - In multiplayer with a headful host, the host always gets `HOST`;
    ///   peers receive `PlayerId(1)`, `PlayerId(2)`, … in the order they
    ///   join.
    /// - In multiplayer with a headless host, the first peer to join
    ///   gets `HOST` and subsequent peers get `PlayerId(1+)`.
    ///
    /// This is **not** "the seat this process drives" — that's
    /// `Host::local_seat`, which can be any `PlayerId` and varies per
    /// machine.  Recordings serialize the join-order seat so a replay
    /// produced on peer-2 is byte-identical to one produced on the host.
    pub const HOST: Self = Self(0);
}

impl Default for PlayerId {
    fn default() -> Self {
        Self::HOST
    }
}

/// A [`PlayerCommand`] tagged with the [`PlayerId`] that issued it.
///
/// This is the wire-level / replay-level / batch-dispatch unit. The
/// inner [`PlayerCommand`] enum stays focused on *what* action was
/// requested; the wrapper records *who* requested it so replay,
/// rollback, network sync, and (eventually) per-seat state mutation
/// all see the same authoritative tag.
#[derive(Clone, Debug, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct PlayerInput {
    pub player_id: PlayerId,
    pub command: PlayerCommand,
}

impl PlayerInput {
    /// Tag a command as issued by the host seat ([`PlayerId::HOST`]).
    /// Used for the single-player input pipeline and for v1 replay
    /// upgrade (legacy untagged recordings have a single seat by
    /// definition).  Live multiplayer pipelines should use
    /// [`PlayerInput::new`] with `Host::local_seat` instead, so the
    /// stamping is data-driven.
    pub fn host(command: PlayerCommand) -> Self {
        Self {
            player_id: PlayerId::HOST,
            command,
        }
    }

    pub fn new(player_id: PlayerId, command: PlayerCommand) -> Self {
        Self { player_id, command }
    }
}

impl From<PlayerCommand> for PlayerInput {
    fn from(command: PlayerCommand) -> Self {
        Self::host(command)
    }
}

/// All player commands for a single frame.
#[derive(Clone, Debug, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct FrameCommands {
    pub commands: Vec<PlayerInput>,
}

impl FrameCommands {
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    /// Append a command. Accepts either a bare [`PlayerCommand`] (tagged
    /// [`PlayerId::HOST`] via `From`) or a pre-tagged [`PlayerInput`].
    pub fn push(&mut self, cmd: impl Into<PlayerInput>) {
        self.commands.push(cmd.into());
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}
