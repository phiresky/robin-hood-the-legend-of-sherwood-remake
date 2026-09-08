# Native server seat registry

`ServerPeers` now owns one `ServerSeat` per authenticated stream generation.
The record holds its writer, presentation name, authenticated owner, optional
durable ranked identity, claim classification, generation, readiness frame and
deterministic simulation membership. The eight parallel active-seat maps/sets
are gone; sender and simulation iterators project the records directly.

Writer detachment remains distinct from authenticated release. Snapshot commits
and forced reconnects take only writer handles, retaining ownership for campaign
publication and reader teardown. Active replacement preserves simulation
membership, changes the generation and clears readiness. Only matching owner
and generation may release the complete record into a disconnected reservation.
That reservation stores no active stream metadata. Continuation publication
combines the two disjoint ownership sources and asserts unique owners and seats.
Runtime writers are skipped by serde and cannot be restored by decoding.

Seat allocation remains monotonic, nicknames confer no authority, provisional
connections retain sorted publication, and ranked/readiness/snapshot barriers
keep their existing policies. Wire messages and the explicit campaign owner and
lease are unchanged. Generation overflow is now checked before consuming any
reservation or advancing allocation, so a rejected claim is transactional.

Four focused registry tests cover detached and replaced streams, stale and
wrong-owner release, readiness reset and admission, retained reconnect claims,
overflow rejection for active/disconnected/fresh owners, and writer-free serde.
Existing snapshot and co-sign tests now claim actual authenticated records
instead of constructing impossible partial map state.

TODO: The peer reader still has independently maintained per-message generation
policy; a broader stale-reader authorization audit belongs in a separate change.

## Validation

`cargo fmt --all` and `git diff --check` passed. Native library release-feature
tests are pending; the parent integration lane owns the combined binary build
and browser/runtime acceptance.
