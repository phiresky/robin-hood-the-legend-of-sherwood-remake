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
  the same authenticated native endpoint or durable browser owner reclaims the
  parked seat; nicknames are presentation only.

Local input must use `PlayerInput::new(local_seat, command)`. The host-only
constructor is for single-player and explicitly host-owned paths.

## Transport

All game-session traffic runs over iroh. Native peers use QUIC with relay
fallback and hole punching; browsers use iroh's relay-over-WebSocket path and
only the HTTPS relay disclosed in the signed invitation. Peers are addressed
by iroh endpoint id — the
public half of a persistent per-install key stored next to the save data
(`multiplayer_identity.key`). Because the id is derived from the stored key,
a host's connect address is known before its endpoint is even bound, which is
how matchmaking can advertise a game ahead of mission launch. There is no
bind address, port forwarding, or NAT configuration anywhere. Endpoint ids
resolve through two layered address-lookup systems: the n0 DNS/pkarr default
and publish/resolve on the BitTorrent Mainline DHT
(`iroh-mainline-address-lookup`), so lookups keep working even without any
hosted discovery infrastructure. Each session uses one bidirectional stream
per peer carrying class-tagged, length-prefixed frames. The browser applies
the same directional allocation limits as native before decoding a body.

### Browser invitations, identity, and content

- Browser-link publication is a persisted Multiplayer/Privacy setting. It is
  on by default and can be overridden per host launch. Disabling it leaves
  native iroh play available without requiring relay readiness.
- A canonical `rhmp3` invitation is signed by the host endpoint key, valid for
  first redemption for exactly 30 minutes, and binds protocol 34, the full
  engine commit, mission/session, expected seats, one disclosed canonical
  HTTPS relay, Demo/Full edition, and the native host's exact content-closure
  SHA-256. The URL stores the public ticket in its fragment; the stable shell
  captures and erases it before its first request.
- One authenticated multiplayer session spans its campaign's mission
  transports. At load/restart/campaign boundaries the host retains the exact
  session id, authenticated owner-to-seat roster, expected player count, and
  disclosed relay route for the one replacement transport. The next host
  endpoint consumes that state once; leaving the session discards it.
- The browser's durable seat key is an IndexedDB-held, non-extractable Ed25519
  private key on the isolated `identity.robinhood.phiresky.xyz` origin. That
  origin exposes only typed status, redemption, and seat-proof operations.
  The proof binds session, host endpoint, and the page's ephemeral iroh
  endpoint; there is no generic signing operation.
- Demo bytes must match their exact length, SHA-256, and native source-closure
  identity. Full owners select a local package whose canonical schema-2
  manifest binds the source Data/locale closure and every transformed
  datadir, split mission, and audio byte. Missing, extra, stale, or mismatched
  content stops before game boot. Retail assets are never uploaded.
- Operators compute the catalog/ticket closure with
  `cargo run --example content_identity -- <installation>/Data`. The Full web
  conversion recipe embeds the same value automatically.
- Relays can observe participant IP addresses, connection timing, and byte
  counts. Gameplay remains end-to-end encrypted; the invitation/log/UI states
  this rather than implying relay anonymity.

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
`robin_rs::multiplayer::{native, identity}`; the relay-only browser client
lives in `robin_rs::multiplayer::wasm`. Canonical ticket and content-closure
code is shared across the platform boundary.
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

## Protocol 32

Messages are bitcode-encoded binary frames, class-tagged and length-prefixed
on a single bidirectional iroh stream per peer. The handshake rejects a
different protocol version.

| Direction | Message | Purpose |
| --- | --- | --- |
| client → server | `Hello { protocol_version, nickname, browser_auth }` | open or resume a session; browser auth carries the signed ticket and durable seat proof |
| server → client | `Welcome { your_seat, session_id, mission_id, mission_seed, sim_config, speech_timing_locale, host_nickname }` | authoritative session identity, mission construction, speech timing, and seat assignment |
| server → client | `Reject { reason }` | typed fail-loud admission rejection |
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

Client `Input` frames can contain only seat-authored gameplay commands. Shared
settings, campaign mutations, modal decisions, and seat connect/disconnect
events require host authority and are rejected before server broadcast; the
engine enforces the same predicate again at deterministic command admission.

`Welcome` is authoritative. A peer must not substitute a local mission, seed,
or `SimConfig` after decode failure. The snapshot payload uses the current
Engine schema and is rejected rather than migrated when incompatible.
`speech_timing_locale: Some(locale)` likewise requires that exact validated
voice pack on every peer. `None` is an explicit selection of the installation's
base `Data/Sounds`, not a missing field or permission to auto-select a local
presentation language. Browser connection state tracks "Welcome pending"
separately from the received `None` value.

The session id is stable across an authorized outer-mission rebuild. A native
client reuses its process-held iroh owner key, while a browser re-proves its
durable signer key. The replacement server seeds those authenticated owners as
disconnected seats and accepts only their exact reclamation; nicknames carry
no authority. The host pins the previously disclosed relay route for the
replacement endpoint, so a redeemed browser invitation remains reachable
without silently minting a new invitation lifetime.

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
- Native clients reconnect with exponential backoff and reclaim their parked
  owner generation. Browser clients reconnect with the same ephemeral iroh
  endpoint and durable session/host/transport proof. Disconnect immediately
  returns admission to the snapshot-wait
  state; simulation and headless HTTP timeline steps remain held until the
  replacement snapshot and `BeginSim` are accepted. A replacement snapshot
  may be older than the abandoned prediction future; adoption clears queued
  inputs, hashes, rewind anchors, and dense history before seeding the new
  exact timeline anchor. Ordinary stale snapshots remain ignored.

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
- `--mp-browser-join-links true|false` overrides the saved browser-link
  publication preference for this hosted game.
- `--join RHMP3_TICKET` consumes a canonical host-signed invitation. The web
  shell installs it internally after authenticating and scrubbing it.

Only the host records the canonical server-ordered multiplayer replay. A
connecting peer cannot select `--record`, and the browser peer's replay RPC
fails with `no active replay recording` instead of publishing a competing
history.

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

The live browser driver is `wasm-www/scripts/live-relay-e2e.mjs`. It requires
a native host with its loopback HTTP API, a fresh `rhmp3` ticket, and Chrome
already listening on a remote-debugging port. It asserts relay-WebSocket
welcome, authoritative input delivery, a forced transport loss, a progressed
replacement snapshot, post-reconnect input, host replay availability, and
browser-peer replay refusal. Production QA additionally requires the isolated
signer origin and exact ticket-selected `/wasm/<commit>` and content catalog
to be deployed.

## Remaining work

- add longer network fault/reorder tests around reconnect and modal lanes;
- define a host viewport policy for split-screen or replay viewing from a
  non-local seat;
- define dedicated-server seat ownership; a true-headless `--server` still
  owns and bootstraps seat 0 like an interactive host;
- keep Spellforge Lua rejected in multiplayer until its VM/state is
  serializable and its event surface is versioned.
