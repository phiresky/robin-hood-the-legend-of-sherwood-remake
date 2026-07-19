# Multiplayer architecture

Updated 2026-07-19. Multiplayer uses a server-ordered input stream with a
small scheduling delay, predictive simulation, rollback for late inputs,
periodic state-hash verification, and authoritative snapshots for joins. The
wire protocol is version 11; older protocol compatibility is unsupported.

## Seat model

- `PlayerId(u8)` is the deterministic seat identifier. `PlayerId::HOST` is
  seat 0; later peers receive seats in server join order.
- `PlayerInput` carries the issuing seat with every deterministic command.
- `Host::transport.local_seat` is process-local and says which seat this
  machine drives. It is never serialized.
- `SeatState` stores deterministic per-player selection, quick groups,
  connection/nickname state, follow target, locker state and alt-lock.
- Disconnect preserves a seat's selection and quick groups. Reconnecting with
  the same nickname reclaims the parked seat.

Local input must use `PlayerInput::new(local_seat, command)`. The host-only
constructor is for single-player and explicitly host-owned paths.

## Ownership and platform split

`robin_engine::multiplayer` contains platform-neutral protocol types,
`NetChannels`, the shared frame cursor and the snapshot handoff. Native and
browser WebSocket transports live in `robin_rs::multiplayer::{native, wasm}`.
Mission-loop admission, input scheduling, rollback, hash comparison and modal
synchronization live in `robin_rs::game_session::multiplayer`. The graphical
and true-headless drivers use the same network drain and timeline admission
state; the headless path does not construct renderer, native-input, UI, or
audio substitutes.

The Engine owns deterministic seats and the shared script/director camera. The
interactive host owns its one local `ViewportState`; rendering projects the
world through that host viewport while HUD/input query the Engine selection for
`host.transport.local_seat`. Do not recreate the obsolete per-seat Engine
camera design: split-screen or replay-from-another-seat would need an explicit
host viewport policy.

## Protocol 11

Messages are bitcode-encoded binary WebSocket frames. The handshake rejects a
different protocol version.

| Direction | Message | Purpose |
| --- | --- | --- |
| client → server | `Hello { protocol_version, nickname }` | open or resume a session |
| server → client | `Welcome { your_seat, mission_id, mission_seed, sim_config, host_nickname }` | authoritative mission construction and seat assignment |
| client → server | `Input { origin_frame, command }` | propose a local command |
| server → peers | `BroadcastInput { server_frame, origin_frame, target_frame, input }` | globally ordered, scheduled input |
| server → peers | `StateHash { frame, hash, clock_frame, ms_until_next_frame }` | desync detection and pacing sample |
| server → joining peer | `InitialSnapshot { frame, engine_bytes }` | authoritative mid-mission state |
| client → server | `ReadyToSim { frame }` | peer loaded and adopted the snapshot |
| server → peers | `BeginSim { frame, start_epoch_ms }` | release the start barrier |
| either direction | `ModalDismiss { kind, result }` | synchronize a blocking modal outside the normal frame drain |

`Welcome` is authoritative. A peer must not substitute a local mission, seed,
or `SimConfig` after decode failure. The snapshot payload uses the current
Engine schema and is rejected rather than migrated when incompatible.

## Input scheduling and rollback

The server stamps every input for:

```text
max(server_frame, origin_frame) + INPUT_DELAY_FRAMES
```

`INPUT_DELAY_FRAMES` is currently 2 (about 80 ms at 25 Hz). Each mission keeps
future inputs grouped by target frame. When a late input arrives:

1. splice it into the shared command log at its target frame;
2. restore the dense recent Engine snapshot when available;
3. replay deterministically to the current frame;
4. use the longer-horizon rewind history when outside the dense window;
5. report a desync and apply at the current frame only when the target is older
   than every retained snapshot.

This is predictive rollback, not strict lockstep. Clients are paced close to
the host clock but do not wait for every possible input before each tick.

The host broadcasts a pre-tick state hash every 25 frames. Peers compare at the
same frame boundary used by replay recording. Hash mismatch is an error and
must not be repaired by silently adopting a new default Engine.

## Join, start and reconnect

- The host publishes a current Engine snapshot for peers that join after
  initialization.
- A joining peer installs the exact snapshot and trims older pending input/hash
  state before announcing `ReadyToSim`.
- Both interactive and true-headless missions wait until the configured
  expected players are ready, then the server broadcasts `BeginSim`.
- Admission is an explicit timeline state machine: a host waits for
  `BeginSim`; a peer waits for successful snapshot adoption, then `BeginSim`;
  both wait for `start_epoch_ms` before simulation advances. A peer never
  announces `ReadyToSim` after a decode/adoption failure, and `BeginSim` before
  its snapshot is a fatal ordering error.
- True-headless hosts perform the same local-seat bootstrap and frame-zero
  snapshot publication as interactive hosts, then keep the reconnect snapshot
  and host clock samples current while running.
- Native clients reconnect with exponential backoff and reclaim a seat by
  nickname. Disconnect immediately returns admission to the snapshot-wait
  state; simulation and headless HTTP timeline steps remain held until the
  replacement snapshot and `BeginSim` are accepted. Inputs queued only in the
  disconnected process are not guaranteed to survive transport loss.
- Browser clients use `web_sys::WebSocket`; browsers can join a native server
  but cannot host a listening server.

## Blocking modals

Blocking dialogs stop the ordinary per-frame command drain, so their dismissal
uses `ModalDismiss` rather than a scheduled `PlayerCommand`. The normal replay
record still captures the dismissal at the mission-frame boundary. New modal
types must define their network/replay ordering explicitly.

## CLI

- `--server HOST:PORT` hosts a game transport.
- `--connect HOST:PORT` joins one.
- `--mp-expected-players N` configures the start barrier for direct launches.
- `--mp-nickname NAME` supplies the stable reconnect identity/overlay label.
- `--lobby-server HOST:PORT` runs only the lobby service.

## Deterministic smoke test

Build first, then run host and client separately:

```bash
cargo build --bin robin

target/debug/robin --server 127.0.0.1:7878 --mp-expected-players 2 \
  --mp-nickname host --start-paused --fast-forward --http-server 7780

target/debug/robin --connect 127.0.0.1:7878 --mp-nickname alice \
  --start-paused --fast-forward --http-server 7781
```

After both peers report `BeginSim`, advance both through the HTTP server in
matching increments and verify:

- no `multiplayer DESYNC` or fatal transport errors;
- matching state hashes at frames 0, 25, 50, ...;
- commands from both seats appear in the same server order;
- a deliberately late input rolls back and reconstructs the same hash;
- reconnect reuses the same seat and preserves its selection/quick groups.

For meaningful gameplay coverage, replay the same recorded command stream on
both peers; stepping empty frames proves clock/snapshot agreement but not the
full command surface.

## Remaining work

- add longer network fault/reorder tests around reconnect and modal lanes;
- define a host viewport policy for split-screen or replay viewing from a
  non-local seat;
- define dedicated-server seat ownership; a true-headless `--server` still
  owns and bootstraps seat 0 like an interactive host;
- keep Spellforge Lua rejected in multiplayer until its VM/state is
  serializable and its event surface is versioned.
