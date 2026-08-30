# Feature 38 owner review: browser rollback multiplayer

Review date: 2026-08-30  
Implementation checkpoint: `5bd80ff2f` (`Keep content closure helpers native-only`)  
Primary final feature checkpoint: `7b0ecd856` (`Bind browser invitations to exact host content`)  
Branch: `codex/feature38-revised`

## Acceptance summary

The implementation is complete and locally verified. It adds an authenticated
browser peer to the protocol-28 predictive multiplayer session through
iroh's relay WebSocket transport, without adding a second gameplay protocol or
a browser-owned canonical history. It also binds every browser join to the
native host's exact Demo or Full content closure.

Production acceptance is not yet complete. The remaining work is deployment
and live-environment QA: publish the game shell, versioned WASM, and operator
staged data through the `robinhood.phiresky.xyz` Cloudflare Static Assets
Worker; deploy the isolated identity signer; provide credentials and a fresh
native-host ticket; and run the live relay/reconnect driver. No GitHub Pages or
binaries fallback is part of the accepted deployment design.

The owner can therefore review and accept the code independently of the
deployment prerequisites below, but the browser multiplayer feature must not
be called production-ready until those prerequisites and the live QA gate pass.

## Delivered scope and behavior

### Host publication and settings

- Browser-link publication is controlled by the persisted
  **Options -> Multiplayer / Privacy -> Publish Browser Join Links** setting.
  It defaults to on for existing and new profiles.
- `--mp-browser-join-links true|false` overrides that saved preference for one
  hosted launch. Disabling publication does not disable native iroh play.
- Publication fails loudly if the host is not relay-online, its content closure
  cannot be proven, an unsupported data overlay is active, or the requested
  mission/session fields cannot be represented canonically.
- The default public share origin is `https://robinhood.phiresky.xyz/`.

### Signed invitation

- The canonical public artifact is an `rhmp3-...` ticket with schema 3. The
  final integration uses gameplay wire protocol 28; invitation and content
  schemas are independently versioned.
- The native host's persistent iroh endpoint key signs the ticket in the
  `robinhood/browser-join-ticket/v3` domain. The public key must be the key that
  owns the advertised host endpoint.
- The signature binds the exact protocol, full engine commit, host endpoint,
  session, mission and optional mission profile, expected player count,
  Demo/Full edition, exact native content-closure SHA-256, and one disclosed
  canonical HTTPS relay URL.
- The invitation lifetime is exactly 30 minutes. A new durable browser owner
  must redeem it within that window; an owner already parked on the host may
  reconnect after expiry. Tickets issued more than two minutes in the future
  are rejected.
- Expected player count is constrained to 1 through 4. Malformed, oversized,
  non-canonical, unknown-field, wrong-build, wrong-protocol, wrong-session, or
  bad-signature tickets are rejected with an explicit error.
- The link carries the ticket in the URL fragment. The stable shell captures it
  and replaces browser history before loading the selected versioned artifact,
  so the ticket is not sent in HTTP requests or referrers.
- The ticket is public bootstrap data, not a bearer secret. Host endpoint
  authentication and the browser's seat proof remain mandatory.

### Browser identity and admission

- Durable browser seat ownership lives on the isolated
  `https://identity.robinhood.phiresky.xyz` origin. Its Ed25519 private
  `CryptoKey` is generated non-extractable and stored in IndexedDB; only the
  public key crosses origins.
- The signer iframe accepts only the exact
  `robinhood.multiplayer-identity.v1` typed operations: `status`,
  `was_redeemed`, `mark_redeemed`, and `sign_seat_proof`. It has no generic
  signing API.
- Requests require exact fields, canonical identifiers, unique request IDs,
  the configured game origin, and the actual parent window. The game-side
  client checks the exact signer origin and iframe source and times out instead
  of silently inventing identity state.
- A seat proof is domain-separated and binds the 32-byte session id, native
  host endpoint, and the browser page's ephemeral iroh transport endpoint.
  Copying a proof to another session, host, or endpoint does not authenticate.
- Redemption is recorded only after a valid host `Welcome`. The host assigns
  seats by authenticated durable owner, retains disconnected seat state, and
  uses monotonically increasing connection generations so an older connection
  cannot release a replacement connection's seat.
- The server stamps the authenticated seat onto inbound commands; the browser
  cannot claim another seat through the command payload.

### Exact Demo and Full content

- A signed invitation binds the native host's exact content closure, not merely
  a build-authorized transformed artifact. The closure hashes the entire
  primary `Data/` tree plus the exact fallback and selected locale trees used
  by the native loader, with canonical paths, lengths, and file bytes.
- Loose native installations are walked directly. Symlinks, unreadable or
  non-file entries, case-insensitive path collisions, ZIP overlays, and
  non-core directory overlays fail publication instead of producing partial
  identity data.
- Operators can compute a native closure explicitly with:

  ```text
  cargo run --example content_identity -- <installation>/Data
  ```

- Demo browser content is one exact compressed datadir. Before boot, the shell
  requires its catalog URL, SHA-256, byte length, and
  `nativeContentSha256` to match both the downloaded bytes and the signed host
  closure.
- Full browser content is intentionally owner-local. The owner selects a
  converted folder containing canonical schema-2
  `robinhood-web-content.json`. The shell verifies the manifest digest,
  engine commit, native closure, `datadir.bin`, every split mission/RHS/audio
  member, every length and digest, and the absence of missing or extra files.
  Retail assets are not uploaded.
- `scripts/build_web_shipping_datadir.sh <source-install> <output>` is the
  canonical Full producer. It writes the package under `<output>/Data/` and
  embeds the source identity in `Data/robinhood-web-content.json`.
- A converted shipping host must have and pass the same manifest verification.
  It cannot publish a browser invitation from an unverifiable old package.

### Transport and native/WASM parity

- The browser uses iroh 1.1's relay-only WebSocket path, with the exact HTTPS
  relay disclosed in the signed invitation. It does not substitute a fixed
  project relay or a direct UDP path.
- Native peers retain native iroh QUIC, direct connectivity, and relay
  fallback. Both paths use the same ALPN, protocol-28 messages, class-tagged
  framing, directional message policy, allocation limits, authoritative host
  ordering, snapshots, hashes, and start barrier.
- The browser refuses client publication of host-only messages and fails
  loudly on malformed frames, incompatible `Welcome` state, failed snapshot
  adoption, wrong mission/seed/config/session, or an explicit host `Reject`.
- `Welcome` is authoritative. The WASM peer constructs the exact announced
  mission, seed, simulation configuration, session, and assigned local seat; it
  does not fall back to local defaults after an error.

### Prediction, rollback, and reconnect

- Inputs use the existing server-authoritative ordering and two-frame
  scheduling delay. Peers simulate predictively and use retained dense and
  long-horizon snapshots to replay late commands.
- On browser or native transport loss, the peer immediately returns to
  snapshot admission and simulation remains held. Browser reconnect uses
  exponential backoff, the same ephemeral iroh endpoint, and a fresh proof
  from the same durable owner key.
- Reconnect accepts only the same seat, mission, seed, `SimConfig`, and session.
  A replacement authoritative snapshot may be older than the abandoned local
  prediction future; ordinary stale snapshots outside reconnect remain
  ignored.
- Before adopting a replacement snapshot, the client clears future input,
  pending hashes, dense rollback history, long-horizon rewind anchors, and
  abandoned clock/prediction state. It then seeds one exact timeline anchor and
  waits for the matching `BeginSim` before advancing.
- Host-only generations prevent the teardown of a superseded connection from
  disconnecting the newly reconnected owner.

### Replay semantics

- The native host alone records the canonical server-ordered multiplayer
  replay.
- A connecting native peer cannot combine `--connect` with `--record`.
- The browser peer's replay RPC returns `no active replay recording`; it does
  not publish a competing local prediction history.
- The local assigned seat is installed before gameplay/replay initialization,
  keeping recorded command ownership consistent with the authoritative host
  stream.

## Security and privacy properties

- Host ticket signatures, iroh endpoint authentication, and browser seat
  proofs cover separate trust boundaries; none is treated as a substitute for
  the others.
- Invitation parsing is canonical and bounded. Relay URLs require HTTPS and
  reject credentials, query strings, fragments, alternate encodings, and
  non-canonical serialization.
- Frame bodies are direction-classified and length-limited before decode,
  including the browser relay path. Opening failures use bounded typed
  `Reject` reasons.
- The isolated signer private key is non-extractable, and its API cannot be
  repurposed to sign arbitrary application data.
- Gameplay remains end-to-end encrypted through iroh. The disclosed relay can
  still observe participant IP addresses, connection timing, and traffic
  volume; the UI and documentation do not promise relay anonymity.
- The fragment scrub reduces accidental HTTP/referrer disclosure, but anyone
  who receives the public invitation can attempt a new admission during its
  validity window. Seat capacity, the signed exact session, durable proof, and
  host admission checks still apply.

## Verification evidence

The final clean implementation checkpoint was validated as follows:

| Gate | Result |
| --- | --- |
| `RUSTC_WRAPPER= cargo build -j3 --bin robin` | Pass |
| `RUSTC_WRAPPER= cargo test -j3 -p robin_rs --lib` | Pass: 1,068 passed, 0 failed |
| Targeted join-ticket suite | Pass: 5 passed |
| Targeted exact-content suite | Pass: 2 passed |
| Reconnect/timeline/rewind tests | Pass |
| `RUSTC_WRAPPER= cargo check -j3 -p robin_rs --lib --target wasm32-unknown-unknown --no-default-features` | Pass |
| `RUSTC_WRAPPER= cargo check -j3 --bin convert_datadir` | Pass |
| `RUSTC_WRAPPER= cargo check -j3 --example content_identity` | Pass |
| `node --experimental-strip-types --test src/*.test.ts` in `wasm-www` | Pass: 8 passed, 0 failed |
| `node_modules/.bin/tsc --noEmit` | Pass |
| `node_modules/.bin/vite build` | Pass; existing non-module `coi-serviceworker.js` warning only |
| `cargo fmt --check` and `git diff --check` | Pass |

The broad workspace `cargo test -j3` gate reaches an unrelated inherited
example compile failure at
`crates/robin_rs/examples/original_parity_replay.rs:7769`: that old branch-base
example constructs `Mission` without the newer required `attempt_history`
field. Feature-owned libraries and tests pass; integration onto the current
root branch must retain the root-side fix for that stale example.

The live driver is `wasm-www/scripts/live-relay-e2e.mjs`. Its assertions cover
relay-WebSocket `Welcome`, authoritative browser input, forced transport loss,
a progressed replacement snapshot, post-reconnect input, host replay
availability, and browser replay refusal. It has not been run successfully
against production because the prerequisites below are absent.

## Deployment prerequisites and release blockers

These are external prerequisites, not missing silent fallbacks in Feature 38:

1. Deploy the stable game shell, versioned `/wasm/<12-char-commit>/` artifact,
   and `/wasm/latest.json` through the public Static Assets Worker at
   `https://robinhood.phiresky.xyz`. Its build manifest must advertise protocol
   28, ticket schema 3, and multiplayer content schema 2.
2. Deploy the isolated signer at
   `https://identity.robinhood.phiresky.xyz/identity-signer/`, with the exact
   game origin in its policy and the matching game-shell `frame-src`. There
   must be no generic signing route.
3. Operator-stage the exact Demo monolith at
   `/datadirs/demo-leicester/v8-web-opus-q80.rhdata.zst`. Supply
   `demo_content_identity_sha256` from the native source-closure command, not
   from the compressed artifact hash. The build catalog separately inventories
   the monolith's actual SHA-256 and length.
4. Before Cloudflare publication, fail loudly unless the staged Demo monolith
   is at most 26,214,400 bytes. This worktree has no operator data fixture, so
   compliance with Cloudflare Static Assets' 25 MiB per-file limit is unproven.
   Do not silently introduce R2 or an unreviewed chunking format.
5. If a Full package is authorized for a build, provide the exact canonical
   schema-2 manifest digest in the build catalog. Full content remains selected
   locally by its owner unless a separate remote-distribution design is
   reviewed.
6. Provide deployment credentials, one relay-online native host, a fresh
   `rhmp3` ticket, and Chrome with a remote-debugging port, then run the live
   driver through welcome, input, forced loss, replacement snapshot, resumed
   input, and replay checks.

Final environment observations on 2026-08-30 were:

- `https://robinhood.phiresky.xyz/` returned HTTP 200;
- the identity signer hostname did not resolve;
- `/wasm/latest.json` returned HTTP 404;
- `/datadirs/demo-leicester/v8-web-opus-q80.rhdata.zst` returned HTTP 404.

Those observations explain why live production QA is still blocked; they are
not converted into fake local success.

## Known limitations and non-goals

- Browser transport is relay-WebSocket only. Browser direct UDP/hole punching
  is not implemented.
- Builds must match the full engine commit and protocol 28. There is no legacy
  protocol, snapshot, ticket, or content-manifest migration.
- Browser multiplayer requires WebCrypto Ed25519 and durable IndexedDB. If the
  browser deletes the isolated origin's storage, it loses the durable owner and
  cannot reclaim that parked seat as the same identity.
- Full retail content is not hosted by this feature. The exact owner-local
  folder flow is deliberate.
- ZIP and non-core mod overlays cannot publish browser invitations because an
  exact corresponding browser package closure is not defined.
- Spellforge Lua remains rejected in multiplayer until its VM/state and event
  surface are serializable and versioned.
- Longer-running network loss, reorder, and modal-lane fault campaigns remain
  useful future coverage beyond the deterministic and live-driver assertions.
- A true-headless server still owns seat 0. Dedicated-server ownership and a
  non-local replay/split-screen viewport policy are separate future designs.
- Relay metadata privacy is limited as described above; the feature provides
  encrypted gameplay, not anonymity.

## Owner acceptance checklist

- [x] Accept implementation and security behavior through `5bd80ff2f`.
- [x] Reconcile the stale parity example fix while integrating onto current
      root.
- [ ] Confirm the Worker and isolated signer contain no GitHub Pages fallback.
- [ ] Prove and enforce the Demo artifact's 25 MiB Static Assets limit.
- [ ] Deploy schema-3 ticket/schema-2 content metadata and exact artifacts.
- [ ] Complete the live relay, reconnect, content, and replay QA run.
- [ ] Mark production acceptance only after every deployment item above passes.
