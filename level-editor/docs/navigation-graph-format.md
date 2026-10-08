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
