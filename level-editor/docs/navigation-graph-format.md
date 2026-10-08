# Compiled navigation graph stream

Map export writes `asset_geometry.motion_data.graph_bytes` from placed motion
contours. The engine loads the prepared nodes and links into its existing
pathfinder. The extension below changes serialization at load time only.

All scalars are little-endian. Ordinary streams start with the `u16` actor-size
count. Extended streams start with `u16 65535`, `u16 1` (version), then the actual
`u16` actor-size count. Unknown versions are errors.

| Field | Ordinary stream | Extended version 1 |
| --- | --- | --- |
| Each node's outgoing link count | `u16` | `u32` |
| Each outgoing link index | `u16` | `u32` |
| Total link count | `u16` | `u32` |
| Other fields | Existing encoding | Unchanged |

The half-diagonal prepass understands the same header. Node addresses remain
four `u16` values (layer, area, obstacle, node); coordinates remain `i16` and
configuration indices remain `u16`. Export uses the extended stream only when
there are more than 65,535 links. Runtime link identities already use `u32`.

The compiler currently prepares the stock 6×3 actor half-diagonal. It computes
clearance and conditional-state requirements before writing links. The format
extension neither adds geometry solving during movement nor certifies the
correctness of a map's contours or routes.

## Animated passage conditions

Compiled lift doors may carry `passage_states`, an array of
`{ layer, area, allowed_states }`. Layer and area address the prepared motion
table. Each `allowed_states` entry is a required-state `u32` mask; a requirement
matches when any mask satisfies `(current_state & mask) == mask`. Every listed
requirement must match. An empty mask list blocks the entrance permanently;
omitting the requirements leaves normal gate permissions in control.

The compiler checks swept entrance segments against the placed obstacles and
retains area-local state identities. It warns about permanently blocked
entrances. The loader validates addresses and state bits, stores requirements
with immutable navigation assets and caches a blocked flag on the door at
startup and after obstacle-state changes. Permission queries read that flag;
they do not inspect geometry. Script activation and locks remain independent.
Replay schema 69 includes the cached flag. An executing passage checks that flag
before advancing its retained animation/order cursor. If blocked it pauses until
the prepared conditions permit passage again. This is a boolean lookup, not a
geometry query or route reconstruction. Thirty-six low-entry animation cases
verify closure and reopening; other phases still need broader validation.
