# S2: exact prepared-load ownership

`SaveLoadRequest::Load` is now an unresolved local selection only.
`SaveLoadRequest::ApplyLoad` carries a private `PreparedLoad`, whose local
handle and `PreparedGameSave` can only be bound by manager preflight. There is
no constructor accepting an independent local handle and decoded payload, no
mutable payload accessor, and no request-level origin that can be transferred
to an unrelated later operation.

The owned variants distinguish local preflight, committed local snapshot and
committed remote snapshot. Quick-load confirmation, cold launch, multiplayer
snapshot proposal, route/application and mission handoff retain that exact
owner. Local generation/manager checks still run at consumption and mission
handoff; the payload is not reread. The prepared wrapper retains session-only
restart replay identity rather than reducing every load to a disk JSON payload.

`PendingLevelLoad` contains only the prepared owner. Mission identity is derived
from its payload; callers cannot independently replace the slot, mission ID,
payload or origin. Owner deserialization is rejected, including when nested in
a request or pending mission transition.

## Remote admission

A remote decoded payload remains an uncommitted snapshot until the transport
releases a `CommittedSnapshotTransition`. Its prepared payload, instance ID and
commit bit are private. Construction always starts uncommitted, and the narrow
commit method validates the exact instance and rejects repeated admission.

`commit_authenticated` trusts its existing authenticated network-event-drain
caller; it does not perform cryptographic verification itself. This is an
ownership boundary, not a claim that Rust visibility proves the complete network
authentication call graph. Neither the opaque committed token nor prepared
ownership can be recreated through serde data. Network/save/replay wire formats
remain unchanged; these wrappers are process-local bookkeeping.

## S1 caller integration

The executor no longer composes manual or diagnostic payload writes with a
separate `save_index`. It consumes successful manager-owned commit evidence from
the S1 publication transaction (`8449ac92b`). Special-slot wrappers keep their
existing behavior and return shape.

## Regressions and validation

Tests cover live selection after row movement; foreign, deleted, recreated and
decoded handles; rejection of serialized prepared authority; and a compile-fail
example showing that callers cannot replace the prepared payload.

The cross-mission test passes a committed local owner through the actual
executor, changes the on-disk save afterward, and verifies the original frame
and mission survive handoff. `MissionOutcome` retains the owner on successful
LevelLoad and discards it on error. Remote tests cover uncommitted take, wrong
instance, duplicate commit, one-time token consumption and latest-request
replacement without inherited authority. The existing CLI replaced-file test
now carries the prepared owner instead of reassembling its parts.

`cargo fmt` and `git diff --check` pass. No Cargo build ran in this lane; combined
client/default/multiplayer and browser validation belongs to the coordinator.
