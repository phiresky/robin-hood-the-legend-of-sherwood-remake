# S6: authenticated submission ownership

`submission.rs` now returns a private-field `AuthenticatedSubmission` from exact
server-offer/signature verification. It borrows the immutable signed document and
owns its one storage projection. There is deliberately no serde implementation:
deserializable database records cannot recreate authentication authority.

`complete_upload` and finalization require that owner. Controller, genesis/session
identity, participants and envelope serialization are projected once and reused.
Finalization still parses and compares the exact reserved offer, validates the
admission campaign requirement, and delegates live challenge/lease/fencing checks
to the unchanged SQL transaction. HTTP expiry/shape checks, bounded replay
preflight before reservation, immediate readiness sampling, and acquired-only
ingestion remain in their original order. Constructor-local shape validation
protects future callers; shape errors remain BadRequest and invalid signatures
remain Unauthorized. Current callers already authenticated correctly; this does
not claim a previously reachable authentication bypass.

The projection is prepared after authentication, before transport preflight;
its ordinary serialization/digest operations use already validated immutable
data. Proposed submission UUID allocation consequently occurs earlier, but no
database reservation or artifact I/O moves before preflight. Serialization does
not grant authority, and the owner exposes no mutable document/projection access.

Tests added/extended:

- Compile-time negative trait inference rejects adding Deserialize to the owner.
- Real-router malformed signature list, invalid cryptographic signature and
  mismatched server offer retain distinct rejection classes before reservation.
- Interrupted-upload exact retries compare final and reserved envelope/controller/
  genesis projections; existing one-submission assertion remains.
- A simulated live competing reservation returns Busy without ingesting malformed
  campaign bytes; committed exact retries likewise do not ingest those bytes.

Formatting and diff checks run in this source-only lane. No Cargo compilation or
test success is claimed here; combined `robin_highscores` library/router acceptance
belongs to the warmed service lane. No schema, SQL, expiry or cleanup policy change.
