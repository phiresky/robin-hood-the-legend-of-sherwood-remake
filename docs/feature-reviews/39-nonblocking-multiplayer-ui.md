# Non-blocking multiplayer UI and authoritative session transitions

## Decision summary

- **Owner decision:** Pending final review of this revised implementation.
- **Recommendation:** **ACCEPT AFTER THE SCHEMA RECONCILIATION DESCRIBED BELOW.**
- **Reviewed implementation:** exact source tip
  `7a61c72ba5e28e6ca18b94f5327f68a822d953b7` on
  `codex/feature39-revised`.
- **Implementation status:** The release blockers in the earlier Feature 39
  dossier are resolved. Pause-side and gameplay UI are frame-owned and do not
  pause multiplayer; modal results are session/occurrence/frame identified;
  Load, Restart, Quick Load, and campaign launch use an exact-byte
  Prepare/Ready/Commit transition; connected manual stepping is rejected
  unless the host explicitly requests synchronized automation; and shared
  settings/campaign commands have two authority checks.
- **Feature 11 status:** Incorporated. Feature 39 contains and completes the
  accepted cooperative-pause architecture, including its previously missing
  multiplayer Save/Load/Restart policy.
- **Verification confidence:** High for automated Rust, native, wasm, browser,
  protocol, authority, pagination, and transition coverage. Medium for manual
  two-machine play because the final branch was not exercised in a recorded
  live native/browser campaign session.
- **Direct-merge status:** The isolated tip is not itself safe to merge onto
  current `main`: Feature 39 and accepted Feature 45 independently assigned
  incompatible layouts the same network version, and Feature 39 did not assign
  a unique native-save version to its diagnostic marker. The exact required
  reconciliation is network protocol **31**, native save **63**, and replay
  **22** when combining only current `main` with this reviewed tip.

This is primarily multiplayer/session infrastructure rather than a configurable
gameplay or visual effect. There is no setting that may turn off authority,
exact-byte validation, readiness barriers, or the non-blocking frame model.
The normal Graphics, Sounds, Gameplay, Privacy, and Shortcuts values remain
configurable through the cooperative Options UI; host-owned gameplay settings
are read-only on clients.

## Review boundary and provenance

This dossier reviews the final isolated Feature 39 branch at `7a61c72ba`. Its
last rolling-main merge was `938d1f3d6`, which used native save 61, replay 21,
and network protocol 29. The final branch itself reports save 61, replay 21,
and protocol 30.

The implementation was built incrementally in these material checkpoints:

- `749693dea` identifies host-authoritative modal decisions;
- `2082eda84` converts Pause-side UI to one-frame tasks;
- `bfdfc5327` completes timeline, modal, terminal, and Sherwood follow-ups;
- `3765de9d8` applies typed HTTP policy to local UI tasks;
- `9eec341f0` routes host manual stepping through snapshot resynchronization;
- `5438cd493` adds coordinated multiplayer session transitions;
- `e37f6fd84` rejects peer-local multiplayer pause controls;
- `58e715231` preserves authenticated ownership across outer missions;
- `3fb97249f` adds typed bounded Options pagination;
- `6d37baddd` keeps the Shortcuts surface inside the logical viewport;
- `a94974a98` exposes the Rust compatibility protocol to wasm;
- `c3c51abee` adds transport and deterministic host-authority checks; and
- `7a61c72ba` aligns the final integrated authority fixtures.

This is not a second implementation of the already-main unified command
journal, rollback history, or rewind checkpoint tiers. Feature 39 consumes
those primitives to make UI, automation, reconnect, and outer-mission changes
safe. It does materially replace the previous blocking UI ownership and
uncoordinated session-transition behavior.

## Player-visible behavior

### Pause and side screens

Options, Graphics, Sounds, Gameplay, Privacy, Shortcuts, Save/Load,
overwrite/delete prompts, Quit confirmation, and Quick Load confirmation are
owned by `ActiveUiTask`. One UI tick consumes input, updates, renders, presents,
and returns to the mission driver. Campaign map/description/launch prompts,
pseudo-mission debriefs, lost-Sherwood flow, terminal debrief, and gameplay
modals follow the same outer-frame ownership rule.

In single-player, Pause retains the Original's paused-timeline behavior. In
multiplayer, Pause and modal presentation capture only local input: the shared
simulation, networking, replay recording, HTTP request drain, and window event
path continue. A peer opening Options cannot pause the host or advance a private
timeline.

Window close exits from nested states. The Save/Load picker tracks a stable
filename rather than a row index, so list mutation cannot silently select a
different save. Overwrite/delete confirmation retains the underlying picker.
Ordinary screenshots capture the topmost presented UI. Held and hover input are
reset when returning to Pause so the activating click does not remain stuck.

The native data-folder chooser remains a synchronous OS dialog. This is the
explicitly accepted exception; it can temporarily stop that local process from
polling while the dialog is open.

### Host-owned actions

Load, Restart, Quick Load, and Sherwood campaign launch are host-only while
connected. Client controls are disabled, and central execution checks reject
the same operation if UI filtering is bypassed. Transition controls remain
disabled while another transition or reconnect is active.

Multiplayer Save and Quick Save are permitted as **local diagnostics**. The
payload and save-index row are tagged `multiplayer_diagnostic`. Connected Load
pickers hide those rows, and the central authoritative transition rejects a
tagged save even if it is supplied by another path. A diagnostic save is not a
future host snapshot and cannot become the source of a multiplayer Load.

Quit remains a local disconnect/exit operation. Shared campaign mutation,
modal decisions, seat lifecycle, and gameplay-rule changes remain host-owned.

## Exact transition protocol

### Prepare/Ready/Commit

A host-authored Load, Restart, Quick Load, or campaign launch performs this
sequence:

1. The host resolves and validates the source once. Save transitions serialize
   the complete `GameSaveFile`; campaign transitions encode the complete native
   Engine snapshot and target `GameCode`.
2. The host creates `SnapshotTransitionId { session_id, sequence }` and sends
   `PrepareSnapshotTransition { id, payload }` to every currently connected
   peer.
3. Each peer checks the session ID, decodes the exact bytes, validates the save
   header and mission or adopts the Engine snapshot against current assets,
   re-encodes it, and requires byte-for-byte equality. It retains those bytes
   before sending `SnapshotTransitionReady { id }`.
4. The server accepts one acknowledgement only from an expected authenticated
   seat. A wrong session, wrong ID, duplicate acknowledgement, client-authored
   Prepare/Commit, or host self-acknowledgement is an error.
5. A peer that disconnects during the barrier remains expected and must reclaim
   its seat and acknowledge again. The host commits only after the readiness
   set is empty.
6. `CommitSnapshotTransition { id }` releases every participant. The old
   mission transport is then torn down, and all participants enter the normal
   replacement-session snapshot/ReadyToSim/BeginSim barrier.

The host also takes the empty-peer case through the same state machine; it does
not invent a second local load path. Failed authoritative modal publication is
reported as a fatal session event instead of letting the host mutate alone.

### Cross-mission authenticated continuation

The outer campaign loop normally destroys and recreates the iroh endpoint.
Feature 39 carries a one-shot, process-local `HostSessionContinuation` across
that implementation boundary. It preserves:

- the host endpoint identity;
- the unpredictable 32-byte `MultiplayerSessionId`;
- the expected player count;
- the exact authenticated `PeerOwner -> seat` mapping; and
- the previously disclosed relay URL.

The replacement server starts with those owners parked as disconnected seats.
A returning native peer proves the same process-held iroh key. A browser peer
re-proves the durable signer stored by the browser. Only the exact owner may
reclaim its old seat; nicknames are presentation and grant no authority. The
host pins the same relay route, so an already redeemed browser invitation can
reach the replacement session without silently creating a new invitation
lifetime.

The continuation is one-shot and is published only for an authorized mission
transition. Normal shutdown or abandoning the session clears it. Host identity
or expected-player mismatch fails rather than consuming another session's
continuation. A native client identity is process-held rather than persisted
across application restarts; a full application restart therefore requires a
new admission rather than pretending to resume the old owner.

### Ready and reconnect barriers

A joining or reconnecting client installs the exact host snapshot before
announcing `ReadyToSim`. It never announces readiness after decode/adoption
failure. `BeginSim` before snapshot adoption is fatal. Reconnect may adopt a
snapshot older than the abandoned prediction future; adoption then clears
future inputs, state hashes, rewind anchors, and dense rollback history before
seeding the new exact timeline anchor.

An input older than retained rollback history causes a complete session
reconnect. It is never applied at the current frame as plausible fake history.
The host can drop one stale peer or all predicting peers and provide the latest
authoritative snapshot through the same readiness lifecycle.

## Modal identity and authority

Every synchronized gameplay modal has a
`ModalInstanceId { session_id, opened_frame, occurrence }`. The occurrence
counter distinguishes repeated dialogs of the same kind; session identity
rejects delayed traffic from an earlier outer session; and the host decision
carries its authoritative `decision_frame`.

A client may send a `ModalProposal`. The server authenticates and stamps the
source seat, and the host may display it as an advisory request. A proposal is
not a vote and never closes the client's or host's modal. Only the host emits a
`ModalDecision`; only the matching exact instance and kind may consume it.
Unmatched traffic remains deferred rather than resolving the nearest same-kind
surface.

The normal deterministic command stream records the accepted typed dismissal.
Replay playback retains unmatched commands until the corresponding modal and
fails if a supplied modal result is unused or invalid. Clients cannot inject
host settings, campaign operations, modal decisions, or seat connect/disconnect
commands through ordinary `Input` frames.

Host authority is checked twice:

1. the native server calls `validate_peer_command_authority` before it stamps
   and broadcasts a peer command; and
2. deterministic Engine admission calls
   `PlayerCommand::requires_host_authority`, so a forged or incorrectly routed
   client command cannot mutate shared state even after transport admission.

The predicate covers campaign selection/trading, quit updates, shared gameplay
rules, modal dismissals, tactical release, startup messages, seat lifecycle,
and the current configurable item/achievement/noise rules. Seat-authored actor
commands remain available to the owning peer.

## HTTP and keyboard stepping policy

HTTP automation keeps the accepted Feature 11 defaults:

- `n` defaults to 1;
- `auto_dismiss` defaults to `true`;
- typed modal results are validated against their exact `ModalKind`;
- default dismissal conservatively cancels local, uncommitted Pause-side UI;
- `auto_dismiss=false` preserves that UI and returns a descriptive blocker;
- a multiplayer client may propose a modal result but cannot close it; and
- the host publishes its authoritative decision before local dismissal.

Keyboard forward/backward stepping is disabled while connected. HTTP
`/set-paused` is also rejected in multiplayer. Ordinary HTTP forward, backward,
or absolute movement is rejected on clients and on a host unless the request
explicitly sets `synchronized_multiplayer=true`.

An explicit synchronized host step updates the host snapshot at the adopted
frame, disconnects every peer from its prediction future, and requires the
complete snapshot/ReadyToSim/BeginSim barrier before multiplayer resumes. The
host also enters reconnecting state, so a second transition cannot overlap the
barrier. This policy preserves automation without allowing a local debugger
request to desynchronize a live game.

HTTP and replay services continue reaching the outer mission boundary while a
local UI task is open. This includes terminal and campaign presentation, not
only the six-button Pause screen.

## Feature 11 incorporation

Feature 11's accepted cooperative-pause scope is fully present here; it does
not need a second competing implementation. Specifically, Feature 39 retains:

- the one-frame `ActiveUiTask` boundary for Options, Save/Load, Quit, Quick
  Load, and nested confirmations;
- single-player pause and multiplayer local-overlay semantics;
- continued network, replay, HTTP, screenshot, and close-event service;
- stable save selection and correct nested-picker restoration;
- default HTTP auto-dismiss with strict opt-out;
- local client presentation settings plus read-only host gameplay rules; and
- the accepted synchronous native folder picker exception.

It closes Feature 11's prior hold by giving Save an explicit diagnostic-only
meaning and by implementing host-only exact-byte Load/Restart/Quick Load rather
than allowing a peer-local mutation. The final owner decision for Feature 39
therefore also decides the completed Feature 11 multiplayer authority follow-up.

## Feature 00 settings/pagination seam

`crates/robin_rs/src/game_session/ui_task_state.rs` now supplies typed
descriptors instead of deriving button behavior from a rendered row index:

- `OptionRowAction` identifies Enter, Adjust, Rebind, preset, page navigation,
  Accept, Cancel, data-directory, and Finish actions;
- `OptionRow` owns its action, label, optional help, and enabled state; and
- `OptionsPager` computes bounded, non-wrapping visible ranges.

The cooperative page shows at most 12 settings (six per column), followed by
fixed Previous/Next and Accept/Cancel controls. Disabled actions reject mouse
and keyboard activation. The selected row has a visible keyboard focus state,
and the menu transform refreshes on window resize. The integrated 33-setting
Gameplay page is therefore 12/12/9 instead of overflowing the 640x480 logical
viewport. A synthetic 45-row test proves that four pages expose every setting
exactly once. The Shortcuts page retains its deliberately compact 27-pixel row
layout.

These types are currently `pub(super)` within `game_session`. Feature 00 may
reuse them directly from sibling session modules, or promote the descriptors
to a shared settings module when it routes the standalone main-menu Options
screen through the same pager. The seam exists, but Feature 39 deliberately
does not rewrite the separate main-menu screen.

## Implementation map

- `crates/robin_engine/src/multiplayer.rs`
  - protocol messages, session/modal/transition IDs, exact encoded initial
    snapshots, deferred modal routing, reconnect requests, and transition APIs.
- `crates/robin_engine/src/player_command.rs` and
  `crates/robin_engine/src/engine/commands.rs`
  - shared-command authority classification and deterministic admission check.
- `crates/robin_rs/src/game_session/ui_task_state.rs`
  - one-frame side tasks, typed Options rows, pagination, stable Save/Load
    selection, and HTTP auto-dismiss policy.
- `crates/robin_rs/src/game_session/{frame_prepare,frame_simulate,tick}.rs`
  - non-pausing multiplayer scheduling, task composition, typed HTTP outcomes,
    synchronized host stepping, and reconnect gating.
- `crates/robin_rs/src/game_session/{modal_state,multiplayer}.rs` and
  `crates/robin_rs/src/ingame_menu/modal_net.rs`
  - exact modal occurrence handling, replay consumption, snapshot adoption,
    Prepare validation, and Commit application.
- `crates/robin_rs/src/game_session/{sherwood_flow,terminal_debriefing}.rs`
  - frame-owned campaign/terminal UI instead of nested blocking loops.
- `crates/robin_rs/src/main_entry/callbacks.rs`
  - central diagnostic-save behavior and host-authoritative transition entry.
- `crates/robin_rs/src/{save_file,savegame}.rs`
  - diagnostic marker, index filtering, and fail-closed write/load helpers.
- `crates/robin_rs/src/multiplayer/native.rs`
  - transport-side authority, peer readiness, exact transition barrier,
    authenticated seat continuation, stable native owner, and relay pinning.
- `crates/robin_rs/src/multiplayer/wasm.rs`
  - browser transition events, reconnect validation, and durable owner proof.
- `wasm-www/src/join_ticket.ts`
  - browser invitation's exact Rust network-protocol binding.

## Verification evidence

The final isolated tip was tested after its last merge and fixture correction:

- `cargo test -p robin_rs --lib`: **1,204 passed, 0 failed**.
- `cargo build --bin robin`: passed for the native game binary.
- wasm32 `wasm-dev` build with `--no-default-features --features audio`:
  passed.
- browser multiplayer tests: **8 passed, 0 failed**.
- `pnpm typecheck`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.

Focused coverage includes:

- exact modal proposal/decision wire round trips and session-bound occurrence
  identity;
- exact snapshot-transition bytes and IDs;
- snapshot adoption before Ready and rejection of premature Begin;
- stale-input full reconnect and discard of abandoned outbound commands;
- a replacement transport seeding only exact authenticated owners/seats;
- owner-only seat reclaim and replacement;
- every current peer being required by Prepare/Ready/Commit;
- a transition peer disconnecting and being required to acknowledge again;
- reconnect rejection for wrong session, mission configuration, or speech
  timing locale;
- peer commands being rejected before broadcast when host authority is needed;
- deterministic Engine rejection of client-authored shared settings and seat
  lifecycle commands;
- local multiplayer Pause not stopping the shared timeline;
- client HTTP proposals not dismissing modals and host publication preceding
  dismissal;
- multiplayer pause/step rejection and explicit host synchronized stepping;
- diagnostic markers being written to both payload and index;
- 45-row and integrated 33-row pagination coverage; and
- strict/default HTTP policy across every Pause-side task kind.

The final build emitted existing non-fatal unused/deprecation warnings. The
local `sccache` server repeatedly stopped, so some compilation fell back to
local rustc. Neither condition changed the successful results.

## Current limitations and review risks

- **No recorded live two-machine campaign pass.** Automated transport and
  session tests are extensive, but the final revision was not manually driven
  through native host + native client or native host + browser client across a
  real mission Load/Restart/campaign transition.
- **The synchronous folder picker blocks locally.** This is an accepted desktop
  exception. It should remain an administration action rather than become a
  model for other UI.
- **Native owner continuity is process-local.** Browser ownership is durable;
  native continuation deliberately does not survive a full application
  restart. A new process performs new admission.
- **The Prepare barrier can wait indefinitely for a required peer.** This is
  safer than committing without it, but there is no host-facing timeout/kick UI
  in this feature. A disconnected expected player must return before the
  replacement `BeginSim` barrier completes.
- **No legacy network compatibility.** Protocol, current native snapshots, and
  current Rust save/replay formats are exact-match boundaries. Legacy C++ save
  import is separate and remains supported by `legacy_save`.
- **The isolated diagnostic flag is too permissive.** At reviewed tip
  `7a61c72ba`, `SaveHeader::multiplayer_diagnostic` uses `#[serde(default)]` and
  save version remains 61. That silently accepts a pre-feature Rust header as
  non-diagnostic. This contradicts the owner's no-old-Rust-compatibility policy
  and must be removed during the Feature 45 schema reconciliation below.
- **The standalone main-menu settings screen is not paginated here.** Feature
  39 provides and tests the typed cooperative pager seam; Feature 00 owns
  routing the separate presentation through it.
- **No in-world walkable campaign-space work is included.** Campaign UI is
  frame-owned and network-safe, but the separately deferred walkable-space idea
  remains out of scope.

## Exact current-main and Feature 45 reconciliation

Do not merge code as part of this review. After owner acceptance, integrate
Feature 39 with exact current `main` (observed here at `3bc6f4ac1`) and preserve
accepted Feature 45 `ea4b9fd00` plus the `robin_parity` move in `0c8acd407`.

The important ancestry is:

| Boundary | Shared base `938d1f3d6` | Feature 39 `7a61c72ba` | Current main after Feature 45 | Required union |
| --- | ---: | ---: | ---: | ---: |
| Native save | 61 | 61 plus a new diagnostic field | 62 | **63** |
| Replay | 21 | 21, no new replay layout | 22 | **22** |
| Multiplayer protocol | 29 | 30 | 30 | **31** |

The numbers 30 are incompatible development layouts. Feature 45 uses protocol
30 for its typed Engine/snapshot representation. Feature 39 independently uses
protocol 30 for session IDs, modal instances, exact encoded snapshots,
reconnect messages, and Prepare/Ready/Commit. Leaving the combined build at 30
would allow a standalone Feature 45 peer to pass the handshake and then decode
a different wire/snapshot layout. The union must therefore be protocol 31.

Likewise, Feature 39's diagnostic marker changes current save JSON but the
isolated branch left the version at 61 and supplied a serde default. Feature 45
already assigns save 62 to a different typed runtime layout. The union must:

1. remove `#[serde(default)]` from `SaveHeader::multiplayer_diagnostic` so the
   field is required in current Rust saves;
2. assign `SAVE_FORMAT_VERSION = 63` and update its version-history/exact-value
   tests;
3. retain Feature 45's `REPLAY_SCHEMA_VERSION = 22`, because Feature 39 adds no
   serialized replay field or command variant;
4. assign `NET_PROTOCOL_VERSION = 31` in
   `crates/robin_engine/src/multiplayer.rs` and retain both Feature 45 typed
   snapshot state and every Feature 39 message/authority rule;
5. update the protocol assertion/comment in that Rust module;
6. update `wasm-www/src/join_ticket.ts`, `join_ticket.test.ts`, and
   `multiplayer_content.test.ts` to 31;
7. update the protocol heading/invitation text in `docs/MULTIPLAYER.md`, the
   protocol reference in `docs/NEW_FEATURES.md`, and the Feature 38 review's
   compatibility references; and
8. rerun the full Engine/robin_rs save, replay, native, wasm, browser, and
   TypeScript gates on the reconciled tree.

Current main also moves the Original parity example from
`crates/robin_rs/examples/original_parity_replay.rs` to
`crates/robin_parity/examples/original_parity_replay.rs`. Preserve Feature 39's
new `StepKind::{Forward, GoToFrame}` patterns with `..` in the moved file rather
than resurrecting the deleted example.

If another accepted protocol-changing feature lands before reconciliation, 31
must not be reused. In that case use the next globally unallocated protocol
after that integrated tip and update every Rust/browser/documentation binding
atomically. The exact union of only current `3bc6f4ac1` and reviewed Feature 39
is 31/63/22 as specified above.

## Owner acceptance checklist

The final decision is whether to accept these policies as implemented:

- multiplayer UI presentation never pauses the shared simulation;
- the native folder picker remains the sole accepted synchronous UI exception;
- clients may propose modal results but the host is the only authority;
- Save/Quick Save are explicitly local diagnostics in multiplayer;
- Load, Restart, Quick Load, and campaign launch are host-only exact-byte
  transitions with readiness and replacement-session barriers;
- HTTP stepping defaults to automation-friendly dismissal but requires an
  explicit synchronized host opt-in while connected;
- authenticated key ownership, never nickname, preserves a seat across an
  outer mission; and
- post-acceptance integration performs the mandatory 31/63/22 schema
  reconciliation rather than merging either side's version constants as-is.

With those policies accepted and that reconciliation performed, Feature 39 is
ready for integration. No remaining source-level blocker was found in the
isolated implementation.
