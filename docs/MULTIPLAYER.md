# Multiplayer architecture

Updated 2026-08-30. Multiplayer uses a server-ordered input stream with a
small scheduling delay, predictive simulation, rollback for late inputs,
periodic state-hash verification, and authoritative snapshots for joins. The
wire protocol is version 29; older protocol compatibility is unsupported.

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

## Transport

All game-session traffic runs over iroh (peer-to-peer QUIC with relay
fallback and hole punching). Peers are addressed by iroh endpoint id — the
public half of a persistent per-install key stored next to the save data
(`multiplayer_identity.key`). Because the id is derived from the stored key,
a host's connect address is known before its endpoint is even bound, which is
how matchmaking can advertise a game ahead of mission launch. There is no
bind address, port forwarding, or NAT configuration anywhere. Endpoint ids
resolve through two layered address-lookup systems: the n0 DNS/pkarr default
and publish/resolve on the BitTorrent Mainline DHT
(`iroh-mainline-address-lookup`), so lookups keep working even without any
hosted discovery infrastructure. Each session uses one bidirectional QUIC
stream per peer carrying length-prefixed frames.

## Matchmaking

Matchmaking is fully serverless. Every player who opens the multiplayer menu
joins one well-known iroh-gossip topic; peers for the topic are found through
the Mainline DHT (`distributed-topic-tracker`), so there is no broker, no
master server, and nothing to configure. Hosts periodically announce their
game as soft state (listings expire when the announcements stop); joiners
broadcast their join intent and the host counts them; Start is broadcast with
the synchronized `start_at_epoch_ms` (and repeated, with the started-state
announce as a fallback path, since gossip is fire-and-forget). The game id
doubles as the host's game endpoint id, which joiners then pass to the normal
`--connect` path. Announcements are currently unauthenticated soft state —
signing them with the game identity key is a known TODO.

## Ownership and platform split

`robin_engine::multiplayer` contains platform-neutral protocol types,
`NetChannels`, the shared frame cursor and the snapshot handoff. The native
iroh transport and identity live in
`robin_rs::multiplayer::{native, identity}`; `robin_rs::multiplayer::wasm` is
currently a stub (browser multiplayer is pending iroh wasm support).
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

## Protocol 29

Messages are bitcode-encoded binary frames, length-prefixed on a single
bidirectional QUIC stream per peer. The handshake rejects a different
protocol version.

| Direction | Message | Purpose |
| --- | --- | --- |
| client → server | `Hello { protocol_version, nickname }` | open or resume a session |
| server → client | `Welcome { your_seat, session_id, mission_id, mission_seed, sim_config, speech_timing_locale, host_nickname }` | authoritative session identity, mission construction, speech timing, and seat assignment |
| client → server | `Input { origin_frame, command }` | propose a local command |
| server → peers | `BroadcastInput { server_frame, origin_frame, target_frame, input }` | globally ordered, scheduled input |
| server → peers | `StateHash { frame, hash, clock_frame, ms_until_next_frame }` | desync detection and pacing sample |
| server → joining peer | `InitialSnapshot { frame, engine_bytes }` | authoritative mid-mission state |
| client → server | `ReadyToSim { frame }` | peer loaded and adopted the snapshot |
| server → peers | `BeginSim { frame, start_epoch_ms }` | release the start barrier |
| client → server | `ModalProposal { instance, kind, result, requested_frame }` | present a non-authoritative client request to the host |
| server → clients | `ModalDecision { instance, kind, result, decision_frame }` | commit the host's sole authoritative result for one exact modal occurrence |
| server → client | `ReconnectRequired { reason }` | discard the prediction future and perform a complete handshake/snapshot admission |
| server → clients | `PrepareSnapshotTransition { id, payload }` | distribute exact host-authored save or campaign-exit bytes for validation and retention |
| client → server | `SnapshotTransitionReady { id }` | acknowledge the exact prepared transition bytes |
| server → clients | `CommitSnapshotTransition { id }` | release the transition only after every current peer is ready |

`Welcome` is authoritative. A peer must not substitute a local mission, seed,
or `SimConfig` after decode failure. The snapshot payload uses the current
Engine schema and is rejected rather than migrated when incompatible.
`speech_timing_locale: Some(locale)` likewise requires that exact validated
voice pack on every peer. `None` is an explicit selection of the installation's
base `Data/Sounds`, not a missing field or permission to auto-select a local
presentation language. Browser connection state tracks "Welcome pending"
separately from the received `None` value.

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
- Browser clients are currently unsupported: the transport is iroh-only and
  iroh's wasm (relay-over-WebSocket) support has not been wired in yet.

## Modal authority and mission transitions

Pause-side screens and scripted modals are frame-owned states: they poll and
render once, then return to the mission driver so transport, HTTP, replay, and
the multiplayer simulation continue. A client may propose a result for a
session-bound modal instance, but only the host's `ModalDecision` closes it.
The normal replay record captures that decision at the mission-frame boundary.

Host-only save, load, restart, QuickLoad, and campaign-exit operations use an
exact `Prepare`/`Ready`/`Commit` barrier. Clients first decode, validate, and
retain the exact host bytes. The host commits only after every currently
connected peer acknowledges the same session-bound transition id; reconnecting
peers must acknowledge again. Participants then leave the old mission and
perform a complete handshake/readiness admission for the replacement state.
Local multiplayer saves remain explicitly diagnostic and are never accepted as
authoritative load input.

## CLI

- `--server` hosts a game transport on the install's persistent iroh
  identity; the endpoint id to share is logged at startup.
- `--connect ENDPOINT_ID` joins a host by its endpoint id.
- `--mp-expected-players N` configures the start barrier for direct launches.
- `--mp-nickname NAME` supplies the stable reconnect identity/overlay label.

The in-game multiplayer menu needs no flags or environment: it joins the
serverless matchmaking swarm automatically (see **Matchmaking** above).

## Deterministic smoke test

Build first, then run host and client separately (the host logs
`multiplayer: hosting on iroh endpoint <ID>` — pass that id to the client):

```bash
cargo build --bin robin

target/debug/robin --server --mp-expected-players 2 \
  --mp-nickname host --start-paused --fast-forward --http-server 7780

target/debug/robin --connect <HOST_ENDPOINT_ID> --mp-nickname alice \
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
