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

All six top-level authority entry points acquire the same gate:

- Peer-message synchronous dispatch.
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
detachment. Separate tests cover stale teardown, readiness policy, and admission
deadline/cancellation generation binding. Existing native lifecycle/co-sign tests
are retained and use the extracted state operations.

TODO: record focused and combined test results after their frozen-source runs.
TODO: a future bounded actor mailbox could replace the synchronous gate if server
throughput measurements justify it; this change intentionally does not alter the
existing runtime scheduling or introduce queue backpressure policy.
