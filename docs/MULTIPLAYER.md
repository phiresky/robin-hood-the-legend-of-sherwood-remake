multiplayer plan: 

1. the engine needs to be modified to handle multiple simultaneous players.

- each player has their own camera, selected character / action, etc. 
- each input action in the input stream has an assigned player id.

## seat-id model (canon)

- `PlayerId(u8)` is a sim-side seat identifier, assigned by **join
  order**: the host (or first-joined peer in headless setups) is
  always `PlayerId::HOST` (= 0); subsequent peers get `PlayerId(1)`,
  `PlayerId(2)`, ….  This numbering is identical on every machine in
  the session, so a recording produced on peer-2 is byte-identical to
  one produced on the host.
- `Host::local_seat: PlayerId` is the per-process answer to "which
  seat does this machine drive?".  Lives on the host side, never
  serialized, varies per machine.  Live input pipelines stamp every
  outgoing `PlayerCommand` with `host.local_seat` to build a
  `PlayerInput`.
- `PlayerInput::host(cmd)` is the constructor used by single-player /
  v1-replay-upgrade paths — it's correct precisely because in those
  cases the only seat *is* the host seat.  For live multiplayer use
  `PlayerInput::new(host.local_seat, cmd)` instead so the stamping is
  data-driven.

## smoke test methodology

How to verify host/client gameplay sync without a human at each end.
Used to confirm the EngineManager + frame-0-skip + chokepoint refactor
(2026-04-29).  Reproduced 2150 hash-OK events across frames 0-300 with
zero DESYNCs.

**Driver flags:**
- `--headless` — skip rendering. Both processes run pure simulation.
- `--fast-forward` — uncaps the frame rate so `/step-forward` returns
  immediately instead of waiting for wall-clock pacing.
- `--start-paused` — sets `manual_pause` at boot.  Per-frame loop tick
  block is gated until cleared, so the sim doesn't free-run.  Without
  this both processes would race.
- `--http-server PORT` — exposes the HTTP RPC.

**Why `/step-forward`:** the per-frame loop pause gate blocks the tick.
Enter clears it, but headless processes have no keyboard.  The
`/step-forward` HTTP RPC bypasses the gate and advances `sim_frame` by
N deterministically — independent of pause state.

**Recipe:**
```bash
# Server
cargo run --bin robin -- --server 127.0.0.1:7878 --start-paused \
  --headless --fast-forward --http-server 7780 --mp-nickname host &

# Wait for server's first loop iter (snapshot capture).  Sleeping
# too short means the client adopts an empty initial_snapshot and
# never triggers the frame-0 hash-compare path.
sleep 3

# Client
cargo run --bin robin -- --connect 127.0.0.1:7878 --start-paused \
  --headless --fast-forward --http-server 7781 --mp-nickname alice &

# Wait for the client log:
#   "skipping frame-0 snapshot adopt; local engine already matches host hash=…"
# That's the proof both sides constructed identical engines from the
# same seed.

# Drive frames in lockstep.  Both processes must step the same N or
# they'll desync (host broadcasts state-hash every 25 frames; the
# trailing peer fails the compare).
for _ in $(seq 1 12); do
  curl -X POST http://127.0.0.1:7780/step-forward -d '{"n": 25}'
  curl -X POST http://127.0.0.1:7781/step-forward -d '{"n": 25}'
done
```

**Pass condition:** zero `multiplayer DESYNC` errors, zero panics,
matching `state_hash` log entries on both sides at every
`STATE_HASH_INTERVAL` boundary (frames 0, 25, 50, …).

**What this proves:** clocks stay in sync and snapshot adopt is a
no-op when both sides built from the same seed.  **What it doesn't
prove:** command application stays in sync under load — both sides
just tick empty frames.  For that, replay-feed a recorded gameplay
session into both processes (`--replay <path>`) so PlayerInputs flow
through the wire and both engines apply them deterministically.

## phase 3e (done): client auto-reconnect

The native client now drives reconnection internally.  Replaced
the dup'd-stream reader+writer pair with a single-thread session
loop that uses a 20ms `set_read_timeout` to interleave reads and
outgoing-channel drains.

When a session ends with a network error, the I/O thread:
1. Emits `NetEvent::Disconnected` (the game loop logs but keeps
   playing — late inputs will roll into the past once the wire
   resumes).
2. Sleeps with exponential backoff (500 ms → 10 s cap).
3. Re-runs the handshake (`Hello` → `Welcome`).  Server reuses
   the same seat by nickname (phase 3c machinery), so the
   reconnecting peer takes back ownership of their PCs without
   any state loss.
4. Emits `NetEvent::Reconnected` then `AssignedLocalSeat` and
   `MissionSeed` so the game loop picks up the (re-confirmed)
   seat.

Outgoing inputs queued during the disconnect are dropped — the
existing rollback-on-late-input path is the recovery mechanism
once the wire is back.

`drain_net_inputs` no longer drops `host.net` on `Disconnected` —
the transport handles its own retry, so single-player fallback
only happens if the I/O thread itself dies (game-loop drops the
channel).

## phase 3f (done): wasm seed sync

New `NetEvent::MissionSeed(u64)` variant.  Both native and wasm
emit it on every `Welcome` (initial handshake + each successful
reconnect).  `drain_net_inputs` folds the value into
`host.mp_mission_seed` so the wasm path catches up
asynchronously after engine init.

## phase 3g (done): mid-mission state snapshot

New `NetMsg::InitialSnapshot { frame, engine_bytes }` (bitcode-
serialized `Engine`).  `NetChannels::set_initial_snapshot(frame,
bytes)` lets the host cache one snapshot in a shared `Arc<Mutex<>>`
that the server's handshake handler reads after `Welcome` and
sends to each new peer.

The host captures its post-init snapshot right after
`restore_rng_from_seed` in the mission flow.  Receiving peers
fold the snapshot via a new `NetEvent::InitialSnapshot`:
`drain_net_inputs` deserializes via `bitcode::deserialize`,
replaces the live `Engine`, resets the rewind buffer, and trims
`pending_inputs` / `peer_hashes` of anything older than the
snapshot frame.

Mid-mission joiners now adopt the host's exact engine state
instead of trying to reproduce it from seed alone — important
for missions whose init has script-driven side effects beyond
deterministic RNG.

## phase 3d (done): wasm websocket client

Browser-side `connect_client` ported to `web_sys::WebSocket`.
Wasm clients can connect to a native `--server` running on a
desktop / dedicated host.  Server-side hosting is not supported in
the browser (no listening sockets) — the wasm `start_server` stub
returns an `io::Error` so callers fail loudly if they try.

Native and wasm transports share the same [`super::NetMsg`] +
[`NetEvent`] + [`NetOutbound`] surface.  Differences:

- **Outgoing pump**: wasm uses `setInterval(40ms)` to poll the
  outgoing channel, since browsers don't have background threads
  by default.  At 25 Hz this matches the sim tick rate.
- **Handshake**: native blocks on `recv_timeout` waiting for
  `AssignedLocalSeat`; wasm can't (would freeze the browser
  event loop), so the seat lands later via the per-frame
  `drain_net_inputs`.
- **Mission seed at connect time**: native captures the seed from
  `Welcome` synchronously and stamps `host.mp_mission_seed`
  before mission init; wasm starts engine init with seed=0 (same
  as single-player) and the seed arrives asynchronously — usable
  once the host carries a `MissionSeed` event variant
  (TODO).

## phase 3c (done): seat rejoin on reconnect

The server's `ServerPeers` now tracks a `disconnected_seats:
HashMap<String, u8>` keyed by nickname.  When a peer disconnects
its nickname → seat mapping moves into this map (instead of being
forgotten).  When a fresh `Hello` arrives with a nickname that
matches a parked entry, the server reassigns the **same seat
number** instead of allocating a new one.

The sim's drop-in/drop-out support
(`PlayerCommand::DisconnectSeat` preserves selection + hotgroups;
`ConnectSeat` re-arms without resetting selection) means the
rejoining peer takes back control of the PCs they were driving —
no state loss across the reconnect.

**Client-side reconnect** (auto-reconnect on a transient drop) is
a follow-up: today the I/O thread emits `Disconnected` and the
host falls back to single-player.  The user can manually re-launch
the client with the same `--mp-nickname` to claim back their seat.

## phase 3b (done): seed sync + state-hash desync detection

**Mission seed:**
- Server picks a random `mission_seed` at session start and sends
  it via `Welcome`.  Both server and client adopt it
  (`Host::mp_mission_seed`) and call
  `Engine::restore_rng_from_seed(seed)` immediately after
  `Engine::new`, so both machines simulate the same RNG sequence.
  Single-player keeps the engine's hardcoded seed.

**State hash broadcast / verify:**
- New wire variant `NetMsg::StateHash { frame, hash }` and
  `NetEvent::PeerStateHash { frame, hash }`.
- Outgoing channel becomes `Sender<NetOutbound>` so the same wire
  carries inputs and authoritative hashes.  `NetChannels` exposes
  `send_input(cmd)` and `send_state_hash(frame, hash)`.
- Every `STATE_HASH_INTERVAL` (= 25) frames the host samples
  `engine.state_hash()` at the same pre-tick point the replay
  recorder uses and broadcasts.  Clients compute their own hash at
  the matching frame and compare; mismatches log a `multiplayer
  DESYNC` error.  Reuses the existing `crate::replay::state_hash`
  helper.

## phase 3a (done): lockstep + rollback for late inputs

`BroadcastInput` now carries `server_frame`, `origin_frame`, and
`target_frame`.  Locally produced inputs are tagged with the sender's
current frame; the server stamps the shared target as
`max(server_frame, origin_frame) + INPUT_DELAY_FRAMES` (2 = ~80ms at
25Hz).  This keeps input latency low without letting a client
receive its own input in the past if its clock is slightly ahead.

Before the first tick, multiplayer sessions now use a ready barrier:
the lobby passes the expected player count to the host, every client
loads the mission and adopts the host `InitialSnapshot`, then sends
`ReadyToSim`.  The host waits until every expected player is connected
and ready, then broadcasts `BeginSim { frame, start_epoch_ms }`.
Clients stay paused until that barrier opens.  After start, pacing
keeps clients within a one-frame cushion of the host clock; the
two-frame input delay is only an input scheduling horizon, not an
allowed sim-clock drift window.

Game-loop integration:

- `pending_inputs: BTreeMap<u32, Vec<PlayerInput>>` — future-frame
  queue, drained when each tick reaches the matching frame.
- `drain_net_inputs(host, engine, game, assets, dev, rewind_buffer,
  pending_inputs, sim_frame)` triages each incoming wire input:
  - `target >= sim_frame` → queue in `pending_inputs[target]`.
  - `target < sim_frame` → splice into `rewind_buffer`'s command
    log at `target`, restore the dense recent `EngineManager`
    engine snapshot for `target`, and roll forward to `sim_frame`.
    If the target is outside the dense two-second history, fall back
    to `rewind_buffer.rewind_to(sim_frame)`.
  - `target` below the rewind horizon → log a desync error and
    apply at `sim_frame` as a degraded fallback.
- New `RewindBuffer::splice_late_input(frame, input)` and
  `oldest_cmd_frame()` accessor.
- The host publishes `sim_frame` through `host.net.publish_frame()`
  at the top of every tick, so the server's broadcast pump always
  stamps from a fresh cursor.

This is the rollback netcode pattern: a single rewind handles a
batch of late inputs because `rewind_to` replays the whole command
log forward — every spliced input lands in the order the server
saw it.  `EngineManager` keeps dense two-second engine history for
the common short rollback path; existing rewind machinery
(`SNAPSHOT_INTERVAL`-spaced snapshots, exponential pruning) remains
the long-horizon fallback.

## phase 3 (MVP): websocket transport

Server / client transport over WebSockets, sync `tungstenite`
crate.  Wire format is bitcode-serialized [`NetMsg`] inside binary
WebSocket frames.

### CLI

- `--server [HOST:PORT]` — host a session.  This process drives
  seat 0 (`PlayerId::HOST`); peers receive `PlayerId(1+)` in join
  order.
- `--connect HOST:PORT` — join as a client.  The server assigns
  the seat in the welcome handshake, which lands on
  `Host::local_seat`.
- `--mp-nickname NAME` — nickname for the portrait overlay.

### Protocol

| Direction | Message | Purpose |
| --- | --- | --- |
| C → S | `Hello { protocol_version, nickname }` | Opening handshake. |
| S → C | `Welcome { your_seat, mission_seed, host_nickname }` | Seat assignment. |
| C → S | `Input(PlayerCommand)` | Local input the client wants applied. |
| S → all | `BroadcastInput(PlayerInput)` | Server-stamped input fanout. |
| S → all | `SeatJoined { player_id, nickname }` | Folded into `ConnectSeat` on each peer. |
| S → all | `SeatLeft { player_id }` | Folded into `DisconnectSeat` on each peer. |

The server is authoritative on seat numbering and input ordering.
Every machine receives the same `BroadcastInput` sequence in the
same order (WebSocket per-connection FIFO + server's single
broadcast loop), so the input stream — and therefore the sim —
evolves identically.

### Game-loop integration

- `dispatch_local_commands(host, engine, assets, &cmds)` replaces
  `engine.apply_local_commands(...)` at the four input-handler
  callsites.  Single-player branch applies locally as before;
  multiplayer branch sends to `host.net.outgoing`, which the
  transport tags + broadcasts (server) or forwards over the wire
  (client).  On the server the broadcast pump immediately echoes
  the input back into the local incoming queue, so the host has
  zero apply-lag for its own actions.
- `drain_net_inputs(host, engine, assets)` runs at the top of
  every frame.  It empties `host.net.incoming`, applies received
  inputs to the engine, and returns them so the recorder / rewind
  buffer captures them alongside any local-input echoes.
- `Host::net: Option<NetChannels>` carries the wire halves; `None`
  is single-player.

### What this MVP doesn't yet do

- **Strict lockstep** — clients are paced to stay within one frame of
  the host clock, but the main loop is still predictive with rollback
  rather than a hard "do not tick frame N until every input for N is
  known" lockstep.
- **Reconnect / resilience** — a dropped wire ends the session
  for that peer.  The sim's drop-in/drop-out is supported but
  the host doesn't yet auto-emit `DisconnectSeat` on socket
  errors.
- **Wasm transport** — std-net + sync tungstenite are native-only.
  The browser path is single-player until wasm websocket glue is
  added.

## phase 2c-tick (done): per-seat tick_display_state

`tick_display_state` now loops over every active seat each tick:

```text
perform_director_work()             # cinematic, host seat (script-driven)
fast_forward gate                   # frame-level
for seat in active seats:           # NEW: per-seat integration
    decelerate_scrolling(seat)
    dispatch on seats[seat].display_op
    perform_zoom_step(seat) when zooming
    reset seats[seat].display_op for next frame
update_sound_listener_position()    # once per frame, host seat
```

The frame_scrolled reset in `perform_hourglass` also loops over
every seat now, so peer-2's held-scroll edge state is cleared
each frame just like the host's.

This is what unblocks actual peer-2 camera motion: peer-2 can
scroll/zoom and `seats[2].camera.view_position` evolves
deterministically on every machine.  Switching the renderer to
display peer-2's view (rather than host's) is the next step
(see "phase 2c-renderer" below).

Per-seat helper signatures: `decelerate_scrolling(seat)`,
`perform_check_scroll(seat)`, `perform_zoom_step(seat)`,
`set_view_position_for_seat(seat, pos)`,
`is_zoom_*_for_seat(seat)`.

### phase 2c-renderer (done)

Renderer migrated from `engine.camera()` (host-seat-only) to
`engine.local_camera(host.local_seat)` for every viewport-projection
read in `game_render.rs` and `level_loading_host.rs` (~55
call sites).  Host code that drives the local viewport now respects
`Host::local_seat`, so peer-2's process draws peer-2's view.

`engine.camera()` is preserved for sim-state queries / HUD readouts
that are canonical-host-seat-relative regardless of which seat this
process drives.

Six helper functions gained a `host: &Host` parameter so they can
read `host.local_seat`: `render_view_cone_overlay`,
`render_all_view_cones`, `draw_status_bar`, `render_debug_doors`,
`render_debug_motion_graph`, `render_debug_whatsup_overlay`.

`local_camera(local_seat)` falls back to seat 0 when the requested
seat doesn't exist yet (e.g. during the first frames after level
load, before the join-order `ConnectSeat` arrives), so single-player
and the bootstrapping window keep pointing at a valid camera.

## phase 2c (partial): viewport pipeline lifted + dispatch threaded

`BackgroundTransform` (smooth-scroll integration buffer),
`display_op` (state machine), and `frame_scrolled` (per-tick
dedupe) move from `EngineInner` into `SeatState`.  Each seat now
owns its own scroll/zoom integration; peer-2's input no longer
collides with the host's pipeline.

Threaded `seat` through:

- `process_scroll(seat, dir)` and `is_engine_ready_to_scroll(seat, dir)`
- `set_operation(seat, op)`
- `center_on_point(seat, point)` and `set_view_position_for_seat(seat, pos)`
- `change_state(seat, EngineStateRequest)` for `LockerOn/Off` and `ZoomingUp/Down`
- New `_for_seat` zoom helpers: `is_zoom_possible_for_seat`,
  `is_zoom_up_possible_for_seat`, `is_zoom_down_possible_for_seat`.
  The non-`_for_seat` versions remain for host UI gating; they
  read seat 0.

Dispatch handlers in `apply_command_for_seat`:

- `ScrollViewport(dir)` → mutates issuing seat's pipeline
- `CenterCameraOn { point }` → mutates issuing seat's view
- `ChangeState(LockerOn/LockerOff/ZoomingUp/ZoomingDown)` → mutates issuing seat

Non-dispatch callers (tick messenger drains, scripted scenes,
level-load auto-select) pass `0` (host seat) explicitly.

**Still on host seat (single-viewport mode):** `tick_display_state`
advances only the host seat's pipeline each frame.  The renderer
draws one viewport — the local player's — so the per-frame
integration of all peers' bg_transforms is deferred.  When
multi-seat replay rewind lands ("step through a recording from
peer-2's viewpoint"), `tick_display_state` will need to loop over
active seats; until then, peer scroll/zoom inputs are recorded
deterministically in the input stream but only the host's are
visualised live.

## phase 2b (partial): thread seat through camera dispatch

Threaded the `seat` parameter into the camera-mutating dispatch
handlers so the issuing seat is the one mutated:

- `SetLockAlt(on)` — fully per-seat (`seats[seat].is_lock_alt`).
- `SelectFollowElement { entity_id }` — fully per-seat
  (`seats[seat].follow_element` / `locker_active` /
  `camera.position_saved`).  The off-screen-recenter still calls
  `center_on_point`, which targets the host viewport — see TODO
  below.
- `ChangeState(LockerOn/LockerOff)` — fully per-seat.

**Deferred to phase 2c (`viewport pipeline duplication`):** the
visual viewport (BackgroundTransform smooth-scroll buffer,
`display_op` state machine, `pending_zoom_mouse_screen`,
`center_on_point` writes to `view_position` / `camera_slide`,
`process_scroll`, zoom up/down) is global engine state shared by
every seat.  Threading the issuing seat through these helpers
requires:

1. Per-seat `BackgroundTransform` (the smooth-scroll integration
   buffer) so peer 2's scroll doesn't disturb the host's render.
2. Per-seat `display_op` (the InitZoom / InZoom / Scroll / Redraw
   state machine).
3. Renderer changes to draw N viewports (one per active seat).

Until then, `ScrollViewport` / `CenterCameraOn` /
`ChangeState(ZoomingUp/Down)` / `SetPendingZoomMouse` still
target seat 0's camera.  Marked with `TODO(phase-2c)` comments
inline.

## phase 2a (done): lift camera + locker + alt-lock into SeatState

- `CameraState` (entire struct, including `screen_size` /
  `level_size` / locker pose / pending zoom mouse), `locker_active`,
  `follow_element`, and `is_lock_alt` move from `EngineInner` into
  [`SeatState`].  Each seat owns its own viewport, follow target,
  and alt-lock toggle.
- The mechanical lift redirects every `self.camera.foo` /
  `self.locker_active` / `self.follow_element` / `self.is_lock_alt`
  reference to `self.seats[0].…`.  Semantics are identical: the
  host seat is the only one anything mutates today.  Phase 2b will
  thread the issuing `seat` through scroll / zoom / locker dispatch
  so non-host peers can move their own cameras.
- New accessor: `EngineInner::seat_camera(PlayerId)` for renderers
  that want to walk per-seat viewports.  The legacy
  `EngineInner::camera()` still returns the host seat's camera and
  is what single-player rendering reads.

## phase 1d (done): seat lifecycle commands

- `PlayerCommand::ConnectSeat { player_id, nickname }` and
  `DisconnectSeat { player_id }` model join/leave as ordinary
  entries in the input stream.  Recording the lifecycle into the
  replay (instead of deriving it from transport state) is what
  keeps recordings byte-identical across machines.
- `SeatState` gains `connected: bool` and `nickname: String`.
  `is_active(idx)` returns `true` for the host seat (always) and
  for any non-host seat with `connected = true`.
- The dispatch handler reads the **target** from the command
  payload, not from the issuing seat — the host can issue these on
  behalf of a peer that hasn't materialised yet, and a peer
  self-announcing its own join works too (since `ensure_seat`
  lazy-creates).
- Disconnect preserves `selection` + `quick_select_groups` so the
  controlled PCs stay where they were left, on autopilot
  (`MULTIPLAYER.md` item 3).  A subsequent `ConnectSeat` for the
  same `player_id` re-arms without resetting selection (and
  optionally updates the nickname).
- New accessors: `EngineInner::seat(PlayerId)`,
  `EngineInner::seats()`, `EngineInner::active_seats()` —
  consumed by the upcoming portrait "controlled by" overlay.

## phase 1c (done): per-seat dispatch

- `EngineInner::ensure_seat(PlayerId) -> usize` grows `self.seats`
  on demand, returning the index. Drop-in/drop-out hook: a peer that
  joins mid-mission gets a fresh empty seat; a peer that leaves keeps
  its slot (their PCs stay where left).
- `EngineInner::seat_selection(PlayerId)` is the multi-seat read path.
  `selected_pc_ids()` stays as the `PlayerId::HOST` shortcut for host code.
- `apply_commands(&[PlayerInput])` now ensures a seat per input and
  dispatches via `apply_command_for_seat(seat, ...)`. The legacy
  single-arg `apply_command` is the host-seat convenience wrapper.
- Selection-mutating handlers (`SelectPc` / `TogglePcSelection` /
  `BoxSelect` / `BoxUnselect` / `SelectAllPcs` / `UnselectAllPcs` /
  `AssignQuickGroup` / `RecallQuickGroup` / `SelectByPortrait` /
  `SelectAction` / `CancelAction` / `UnselectAllActions` / `KeyControl` /
  `KeyReleaseControl` / `CrouchDown` / `StandUp` / `StartRecordingMacro` /
  `ChangeQaMemory`) now mutate the issuing seat instead of always seat 0.
- Threaded helpers: `select_pc`, `toggle_pc_selection`, `select_all_pcs`,
  `unselect_all_pcs`, `assign_quick_group`, `recall_quick_group`,
  `select_by_portrait_index`, `perform_multi_selection`,
  `perform_multi_unselection`, `apply_post_select_action_fanout`,
  `save_action_for_selected_pcs`, `set_pc_action`,
  `manage_input_pre_action_bow`, `select_pc_action_by_index`,
  `apply_disable_all_actions_temp`, `select_highest_priority_pc`,
  `apply_box_select`, `apply_box_unselect`, `apply_crouch_down`,
  `apply_stand_up`, `record_macro_step_for(_pc)`,
  `apply_start_recording_macro`, `apply_change_qa_memory`.
- Non-dispatch callers (tick messenger drains, scripted scenes, level
  loading) pass `0` (host seat) explicitly.
- Read-side helpers (`is_selected_pc_swordfighting`, `retrieve_stature`,
  `are_selected_pc_in_mission_team`, `refresh_pc_selection_hulk`,
  `any_selected_pc_drawing_selection_mark`, host-side input.rs reads)
  still observe seat 0. They drive single-player UI and need to grow
  multi-seat awareness when the renderer learns about per-player
  cameras / portraits.

## phase 1b (done): per-seat sim state container

- `engine::SeatState` (in `engine/seat.rs`) holds `selection` and
  `quick_select_groups` — the per-seat sim-tracked fields.
- `EngineInner.seats: Vec<SeatState>` replaces the flat
  `selected_pc_ids` and `quick_select_groups` fields. Always at least
  one entry (the host seat, `PlayerId::HOST`).
- Every internal access (`self.selected_pc_ids` / `self.quick_select_groups`)
  now goes through `self.seats[0]`. The public `selected_pc_ids()` accessor
  keeps its signature so the host crate is unchanged.
- Camera / locker / alt-lock are still on `EngineInner` — they're tangled
  with rendering and sim-side state and warrant a dedicated pass. Once
  lifted they'll join `SeatState`.
- Phase 1c will thread `PlayerInput.player_id` into the dispatch path
  so selection commands mutate the *issuing* seat instead of always
  seat 0.

## phase 1a (done): tag the input stream

- `PlayerId(u8)` newtype with `HOST = 0` (the join-order seat 0;
  in single-player it's the only seat).
- `PlayerInput { player_id, command: PlayerCommand }` is the new dispatch / queue / replay unit.
- `FrameCommands.commands`, `RewindBuffer.commands`, `RollbackChecker.cmds` all hold `Vec<PlayerInput>`.
- `EngineInner::apply_commands(&[PlayerInput])` is the multiplayer-aware batch entry point;
  `EngineInner::apply_local_commands(&[PlayerCommand])` is the single-player shortcut for live host pipelines.
- `apply_command` (singular, `&PlayerCommand`) is unchanged — used by host code that's still implicitly local.
- On-disk replay JSONL is now schema v2: each `c[i]` is a tagged `PlayerInput` (`{"player_id": …, "command": …}`). v1 recordings (untagged `Vec<PlayerCommand>`) still load — the reader untags via a per-element `untagged` enum and stamps `PlayerId::HOST` (correct because v1 was single-player-only). (Originally a TODO from phase 1a, completed alongside phase 1c.)
- handlers don't yet read `player_id` — that's phase 1b (lift selection/camera state into per-seat containers).

2. the renderer needs to be modified
- each player has a selected character they are currently playing as
    - generally, multiple players can simultaneously control a character though we might forbid this later
    - in the character portraits, underneath or above the character name the current player handle/nickname is shown

3. drop in drop out
- a player can drop out and rejoin later
- this should not cause issues since the PC characters are switchable and stay mostly the same - if a player leaves, the character they controlled simply stays in game on autopilot (not doing much)

3. network latency handling
    - pre-programmed latency: the game negotiates latency betwen the peers. all inputs are then delayed by that latency locally, such that in most cases, no rollback is required on the remote side. up to some maximum (i guess?)
    - rollbacking - when a player input is received the local engine state is rolled back and local inputs + remote inputs are added together retroactively and fast-forwarded
    - will this feel laggy? not sure. but engine rollbacks for us are also pretty cheap so maybe it's not too necessary
    - TODO: blocking modal dismissal should become an all-player ready barrier. When one player presses OK/Stop, keep their modal open in a "waiting for X, Y" state until every connected player confirms, then dismiss it for everyone simultaneously. The current MVP may dismiss a modal for all players from the first received dismissal.
    - TODO: flatten blocking modal loops into main-loop modal state machines. Dialogues, popup scrolls, briefings, and similar UI should tick/render from the normal per-frame pipeline instead of running nested event loops, so multiplayer events, replay commands, frame pacing, and modal dismissal all share one synchronization path.
    - TODO: merge the rollback checker, rewind buffer, EngineManager history, and multiplayer rollback into one shared timeline/history subsystem. They all need pre-tick snapshots, per-frame command logs, replay-forward reconstruction, and timeline invalidation; keeping separate rings makes performance and correctness harder to reason about.

4. server / client infra
    - server is basically the same as the clients - but does not necessarily have a player. - headless mode
    - on connect, the server creates a dump of the current state, then starts streaming events starting from that state.
    - one thread per client - engine clone can happen in main thread if needed, then broadcast queue and similar standard rust tools should work perfectly.

5. lobby system? integration with steamworks api? https://docs.rs/steamworks/latest/steamworks/struct.Matchmaking.html
    - for now, just use direct tcp socket or websockets to server
    - TODO: lobby game leave/destroy events are missing. Peers should notify the lobby when they leave a listed game, and if the host closes/leaves the lobby should broadcast a destroy/remove event so other clients stop showing or joining the stale game.
