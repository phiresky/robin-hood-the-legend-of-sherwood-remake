# Derby patch coverage audit — 2026-09-27

The initial audit found three missing interior reveals. The subsequent
[interior repair](derby-interiors.md) restores West Tower, East Hall, and Upper
Gatehouse alongside the existing Main Hall control. Both mission drawbridge
switches and the mission winch state remain absent from scene appearance controls.

## Complete game-data inventory

Checked all seven `Derby.rhp.json` patches and every mission whose
`header.map_filename` is Derby: `H03_Der_MK`, `S04_Der_EC`, and `Str02_Der_MP`.
Each mission contains three additional patches: nine mission records in total.
All sixteen decoded patch records exactly match the retained reference manifest.
The audit also read all 42 published Derby asset descriptors and GLB node extras,
including the terrain and compatibility assets, and the scene's placement bindings.

| Base patch | Appearance/effect | Published refined scene |
| --- | --- | --- |
| `patch-000` / Patch01 | Main Hall interior | Bound through `appearance-1`; existing covered/revealed control |
| `patch-001` / Patch02 | West Tower room | Restored cutaway and `appearance-1` binding; curved crown retained |
| `patch-002` / Patch03 | East Hall interior | Restored partial facade cutaway and `appearance-1` binding |
| `patch-003` / Patch04 | Upper Gatehouse chamber | Restored removable facade state and `appearance-1` binding |
| `patch-004` / pixel_vert | Remove mask layer 6, index 19 | No building-cover artwork; mask change, not an additional room |
| `patch-005` / pixel_vert | Replace mask layer 0, index 112 with 113; doors 39–41 | No building-cover artwork; mask/door state |
| `patch-006` / pixel_vert | Reversible masking-sector change, final layer 2 | No building-cover artwork; masking state |

The following three records occur in **each** of the three missions, at runtime
patch indices 7–9. Their stable reference IDs are
`mission-<mission-name>-patch-000`, `-001`, and `-002`.

| Mission patch | Game artwork states | Published refined scene |
| --- | --- | --- |
| `000` / Pont_levis01 | Raised; 35 transition frames; last transition frame baked into background | Upper Gatehouse bridge exists, but no lowered appearance or switch is published |
| `001` / Pont_levis02 | Raised; 35 transition frames; lowered final frame | Second Drawbridge has initial/applied library scenes, but only initial is placed; no scene switch |
| `002` / Pont_levis01_mecanisme | Initial 1×1 frame; 35 transition frames; last frame baked into background | No dedicated mechanism state or patch binding |

Do not interpret a missing final animation as a missing lowered bridge: the first
bridge and mechanism integrate their last transition frame into the background.
Mask, sight-obstacle, door, and visual transitions are independent. In particular,
West Tower and Upper Gatehouse interior reveals have no sight-obstacle changes.
Hiding only the changed sight obstacles would miss both.

## Remaining repair work

- Connect both raised/lowered bridge endpoints to mission patch controls and
  verify their positions and material states. The second bridge's existing
  endpoint model can be reused. Continuous animation remains unvalidated.
- Represent and verify the separate winch/mechanism patch.
- Include every visual patch in publication verification. A Main Hall-only
  browser check does not establish complete Derby state coverage.

The bridge and mechanism entries remain confirmed gaps. This inventory
does not validate the game runtime's door, collision, or masking behavior.

Local evidence: `work/needs-review/derby-patch-audit-20260927/audit.json` records native input,
graphic-frame and published-model hashes, all patch states, and actual exported
bindings. `interiors.png` compares the four covered/revealed source crops.

## Upper bailey battlements preset correction

The spline preset previously extracted a section crossing a sharp texture
boundary near source X=58. Its repeat reproduced the dark, streaked region of
the source wall. This was visible even in the single-strip audit, before any
spline repetition.

The recipe now uses X=[-240, 40], snapping both ends inward to complete crenel
gaps. The resulting repeat is 274.712 units long. The focused browser audit
passed, and straight, curved, corner, and enlarged join renders were inspected
before installing the corrected strip. Five extraction regression tests pass.
The source Upper Bailey East Curtain model itself is unchanged; this correction
selects continuous masonry for the reusable preset.
