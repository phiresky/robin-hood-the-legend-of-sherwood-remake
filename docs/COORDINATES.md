# Coordinate Spaces

The Rust port should keep coordinate spaces explicit at gameplay
and rendering boundaries so projected map coordinates are not confused with
ground/world coordinates.

## Main Types

Use these domain types from `crates/robin_engine/src/coordinates.rs` in public
APIs and stored game state:

| Type | Meaning | Legacy pattern |
| --- | --- | --- |
| `WorldPoint3D` | Raw Spellbound `(x, y, z)` position. | `SBGeoPoint3D`, `RHposition`-style positions with height. |
| `MapPoint` | Projected map point `(world.x, world.y - world.z)`. | `GetPositionMap()`, `SBGeoPoint2D(pt.mX, pt.mY - pt.mZ)`. |
| `GroundPoint` | Ground/world-XY point `(world.x, world.y)`. | Sight and obstacle tests that drop Z, such as `ptEyesGround`. |
| `ScreenPoint` | Screen/UI point after viewport transform. | Mouse/widget/draw viewport coordinates. |
| `SpriteTopLeft` | Sprite draw origin before viewport transform. | `PositionSprite`. |
| `SpriteAnchor` | Offset from sprite top-left to map position. | `SpriteCenter`. |
| `SpriteFrameOffset` | Per-frame draw offset from sprite script data. | `spriteScript[row].avOffset[frame]`. |
| `MoveBox` | Actor-local movement bbox centered on actor position. | `GetMoveBox()`. |
| `MapBBox` | Absolute projected-map bbox. | `GetMoveBoxMap()`, path/fast-grid boxes. |
| `GroundBBox` | Absolute ground/world-XY bbox. | Sight obstacle ground extents. |
| `ScreenBBox` | Screen/UI bbox. | Widgets, HUD, refresh rectangles. |

The core projection rule is intentionally named:

```rust
let map = MapPoint::from_world_xyz(world.x, world.y, world.z);
// Legacy equivalent: SBGeoPoint2D(world.mX, world.mY - world.mZ)
```

When height is known and ground coordinates are needed:

```rust
let ground = GroundPoint::from_map_and_z(map, z);
```

AI snapshots intentionally carry two actor positions when door transit or a
carried PC can make them differ:

- `AiEntityView::position` is legacy `RHArtificialIntelligence::Position()`.
  It may snap a passing actor to the gate endpoint or substitute the carrier
  for a PC on shoulders, and is appropriate for destinations and AI planning.
- `AiEntityView::detection_position` is direct `GetPositionMap()` from the
  actor. Geometry that calls `ComputeDetectionPoint` or `ComputeEyesPoint`
  must start here instead of inheriting the AI-position substitution.

The evaluating NPC follows the same rule: `AiContext::position` is the
AI-facing position, while `self_upright_eye_world` is the direct unsnapped
`ComputeEyesPoint(..., UPRIGHT)` value used by the actor overload of
`IsDetecting360Degrees`.

## Arithmetic Rules

Points are positions. Vectors are deltas. Keep arithmetic in those terms:

```rust
let delta: MapVec = goal - origin;
let moved: MapPoint = origin + delta;
```

Do not add two points together. If legacy code appears to do that through
`SBGeoPoint2D` overloads, decide whether the second value is really a vector, an
anchor, an offset, or a local point, and name it with the matching Rust type.

## Accepted `geo2d` Boundaries

`geo2d` is now the low-level geometry adapter, not a general coordinate type.
It is acceptable in these places:

- `coordinates.rs`, where domain types convert to and from the legacy geometry
  storage and helper APIs.
- Computational geometry internals that call generic polygon, segment, or
  bbox algorithms, as long as the public function inputs and outputs are typed
  (`MapPoint`, `GroundPoint`, `ScreenPoint`, etc.).
- Binary/script compatibility where the file or save schema is named
  `GeoPoint2D` or `SBGeoBoundingBox2D`.
- Tests and fixtures that are explicitly exercising generic geometry helpers.

It is not acceptable to expose `GeoPoint2D`, `BBox2D`, or `geo2d::pt` from
gameplay/render APIs just because a helper currently needs generic geometry.
Convert at the boundary and keep the domain type in the caller.

## Current Stop Line

The goal is to prevent map/ground/screen/sprite coordinate mixups, not to remove
every internal `geo2d` call. Broad rewrites past typed public boundaries are
usually low-value unless they remove a real footgun.

Before converting more code, cross-check the legacy intent:

- `pt.mY - pt.mZ` means projected map space: use `MapPoint` or `MapVec`.
- `pt.mX, pt.mY` with Z ignored means ground/world-XY space: use `GroundPoint`.
- `PositionSprite + SpriteCenter = PositionMap` means sprite draw origin plus
  anchor reaches the projected map position.
- `GetMoveBox() + GetPositionMap()` means actor-local `MoveBox` translated into
  an absolute `MapBBox`.
