# Native server session authority and protocol components

Base: `930eebbbc`. This change preserves the wire format and deterministic input
ordering while rejecting messages from superseded, released, or detached streams.

## Authority transaction

The decoded peer-reader message enters `dispatch_server_peer_message`, which
holds the session-dispatch gate through generation/writer validation, protocol
transitions, and host/writer channel publication. The gate never spans an await.
Checking a generation and subsequently releasing its protection is insufficient:
readiness may downgrade ranked eligibility before touching seat readiness, and a
ranked response can publish several downstream effects.

All top-level authority entry points acquire the same gate:

- Peer-message synchronous dispatch and reader-termination classification.
- Each host outgoing-pump message (including writer detachment and transition commit).
- `prepare_peer_session`: claim, Welcome/snapshot queueing, and initial admission effects.
- `release_peer_session`: owner/generation release through disconnect/admission effects.
- Each admission-monitor tick: generation/deadline inspection through timeout downgrade.
- `ServerHandle::install_ranked_session_setup`: explicit host ranked setup installation.

The gate is outermost. Existing ranked lifecycle → peer-registry lock ordering
is retained; helpers called from these entries do not reacquire the gate. Network
reads/writes remain outside it. Channel sends retain their existing nonblocking
queues; no new effects queue was introduced. Shutdown does not hold the gate
while joining runtime threads.

Obsolete reader teardown now returns immediately: it cannot progress admission
for its successor. Detached writers remain authenticated ownership records until
release, but cannot authorize new reader messages.

Inactive dispatch failures are typed (released, superseded, detached), not parsed
from error strings. The async peer I/O driver preserves the same pinned writer
future for every inactive outcome. Once the reader reports inactivity,
already-queued reconnect/commit frames may drain for at most 15 seconds.
Ordinary active-reader write behavior is unchanged. No new reader effects are allowed during
that drain. Continuing the original future avoids replaying a partially written
frame header. EOF/read errors after authority loss use the same drain path.

## Cohesive protocol state

`native/server_protocol.rs` contains transport-independent components:

- `ReadyBarrier`: computes connected/attached quorum and maximum release frame,
  commits once, and resets on full resynchronization. Per-seat readiness remains
  in its seat record; provisional ready frames retain their prior maximum policy.
- `SnapshotTransitions`: validates acknowledgements, retains disconnected peers
  in the barrier (even after an earlier acknowledgement), and commits once.
- `CoSignTracker`: bounded request/seen history, target binding, signer validation,
  cryptographic verification, and consumption only after successful verification.
- `RankedAdmissionTracker`: exclusive pending challenge, generation-bound
  cancellation, and explicit waiting/finished/expired deadline states.

Components use serde, not a new binary schema. They do not own transport handles.
Deserialized admission diagnostic deadlines expire immediately; a decoded seat
still has no writer and cannot authorize dispatch.

## Regression coverage and validation

The production-dispatch table exercises replacement, detachment, and release for
input, readiness, snapshot acknowledgement, co-sign response, ranked unavailable
and attestation responses, receipt selection, preflight signature, and modal
proposal. It verifies contextual rejection, unchanged successor state, no host or
writer publication, and no accidental ranked downgrade. Positive dispatch tests
exercise current-session input/readiness and snapshot commit with authority
detachment. The production async I/O-driver regression covers commit, reconnect,
replacement, and release followed by buffered input: one terminal frame must
drain, the original writer must run only once, and no unauthorized input may be
published. A stalled writer must fail within its drain budget. Separate tests
cover stale teardown, readiness policy, snapshot acknowledgement rejection, and
admission deadline/cancellation generation binding. Existing native lifecycle/co-sign tests
are retained and use the extracted state operations.

The async driver test preserves an already-started writer waiting on its queue;
it does not emulate partially emitted QUIC frame bytes. EOF/read-error authority
classification was independently code-reviewed, but is not directly exercised by
a dedicated regression test.

Initial checkpoint `8ccd2adfa`: 39 focused release-native tests passed, including
the existing real-iroh ranked/reconnect tests. Independent review subsequently
identified the terminal-writer cancellation race described above; the drain fix
and async regression tests were added before final acceptance.

Post-drain checkpoint `098f00021`: all 42 focused release-native tests passed,
including the async terminal-drain and timeout regressions, with no new warnings.
Command: `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 RUST_TEST_THREADS=2 cargo test --locked
-p robin_rs --lib --no-default-features --features release multiplayer::native::tests`.
`cargo fmt --all` and `git diff --check` also passed.

Final combined source `09a2438b1`: 1,673 release-feature library tests passed,
with six ignored. The separate release-feature binary passed both headless and
graphical live multiplayer scenarios (ten checks each, zero desyncs/missed hash
comparisons) and all four native lifecycle/replay phases. Default-client and
browser checks also passed; see [combined acceptance](REFACTOR_BOUNDARIES.md).
TODO: a future bounded actor mailbox could replace the synchronous gate if server
throughput measurements justify it; this change intentionally does not alter the
existing runtime scheduling or introduce queue backpressure policy.
