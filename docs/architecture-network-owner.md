# Campaign-owned native multiplayer lifetime

The native client transport key and host mission-continuation slot are no
longer process globals. `MultiplayerCampaignSession`, owned by the existing
cross-mission `RustCallbacks` coordinator, is passed explicitly through both
interactive and headless bootstrap into multiplayer setup.

Each owner generates a distinct ephemeral native client key. Its replacement
mission clients reuse that key; durable ranked attestation still loads the
separate install identity. Standalone public transport constructors create
isolated one-mission owners. Callers needing continuation use the explicit
`start_server_in_campaign` / `connect_client_in_campaign` APIs.

The owner retains authenticated seat reservations and relay information across
the existing endpoint teardown boundary. A server-context lease rejects two
simultaneous host mission transports for one owner. Old contexts must be
dropped before replacement; an old context therefore cannot publish into a
replacement transport. Failed startup leaves the pending continuation intact;
successful endpoint startup consumes it. Endpoint identity and player-count
validation remain mandatory and do not consume a mismatched handoff.

Snapshot-commit publication and orderly preserved shutdown still merge seat
reservations for the same authenticated session. Fresh-session cleanup now
targets only the explicit owner and rejects cleanup while its transport is
active. Dropping another campaign never affects this state. The owner is not
restored by serde: decoding produces no live transport authority.

Tests cover independent owner identity/storage, mismatch and failed-preparation
retry, exclusive leases, repeated authenticated-seat publication, and opaque
serialization. Existing exact authenticated-seat reconnect tests are retained.

TODO: Record final focused native test and build results after the cold build
completes. Browser transport ownership is unchanged in this native-only track.
