# 3D reconstruction

The level's 3D scene is rebuilt from the game's own sight-obstacle volumes,
textured by reverse projection from the painted map (`volumes.ts`, below).

The earlier SAM 3D track (fal.ai SAM 3 detection plus SAM 3D Objects meshes
per building, piloted on York) is retired and its tools (`detect`,
`reconstruct`, `merge-detections`, `scene`, `contact-sheet`, `pose-diag`) have
been removed; see git history before 4482f1f36 if it is ever needed again.

## Scene frame and camera (`shared/src/scene.ts`)

The original maps were rendered with a fixed oblique orthographic camera and
the game's world coordinates are projected units: world `(x, y, z)` lands on
map pixel `(x, y - z)`. Sight-obstacle footprints are true-ground rectangles
stored in those units, so the ground foreshortening `sin(elevation)` is
recovered by un-stretching y until adjacent footprint edges are
perpendicular (`map-camera.ts`). York: 694 quads, sin = 0.5725, mean |cos|
0.028, one grid step from the original game's own projection constant
`ASPECT_RATIO` 0.573576436 = cos 55° = sin 35° (the camera looks down 55°
from the vertical). The pipeline uses that constant, elevation 35.00°, and
keeps the fit as a sanity check; every map so far matches it.

Scene frame: right-handed, Z up, map-pixel units. `X = map x`,
`Y = -map y / sin θ` (away from the camera), `Z = z / cos θ`. GLB export
rotates the whole scene to glTF Y-up.

## Volumes track: the game's own geometry (`volumes.ts`)

```
pnpm volumes --map york --render [--fill synth|proc|smear|none] [--closeups x,y;x,y]
# -> library/scenes/york-volumes[-proc|-smear|-holes].scene.{glb,json},
#    work/active/york/york-scene/volumes[-proc|-smear|-holes]-{atlas,ground,compare,view-1,view-2,closeups}.*
```

Every sight obstacle in the level is a polygon with a `z_bottom`/`z_top`
per point, all heights absolute. Compact ones are the building volumes the
artists' scene was built from (sloped tops are roof planes, raised bottoms
roofs stacked on walls — York: 972 usable prisms). Polygons over 100 000 px²
are terraces (elevated ground): the original takes a unit's height from the
top plane of the obstacle it stands on (`mpPlane = mpObstacle->GetTopPlane()`),
the level's elevation lines run along their edges, and the York town polygon
wraps around the river-level tower house exactly along its walls. They are
built as solid plateaus (town at 90, church precinct at 160); houses inside
start at z 0 and extend below the surface, nothing is lifted. `--flat` drops
the terraces; obstacles under 2 px high are skipped.

Some parts are stored displaced along the view ray. A point moved by
(y − Δ, z − Δ) projects to the same map pixel, so the 2D tool showed such a
roof slope in place and nothing in the game draws it, but in 3D it floats Δ
above and Δ south of its body (Lincoln's turret roof, Δ 36). The detector
(`snapFloatingParts`, `shared/src/level3d.ts`) lists raised parts without a
support under their footprint that land on an obstacle when slid by
Δ = z_bottom − top. It cannot tell those from overhanging roofs or cornice
gaps (York #572 rests 45 % on its body, #586 sits 4 units above a full
support), so nothing is moved automatically: `volumes --snap` moves the
non-opaque ones, and the editor tags suspects ("float?") with a per-part
"snap down Δ" button that applies the shift as an ordinary transform.

Coincident faces are clipped away before texturing. Buildings are stacks
of boxes whose planes coincide exactly (an opaque box to the eave plus
non-opaque boxes for jettied floors and roof slopes with the same front
plane; a house standing on a terrace edge; neighbours sharing a wall; two
boxes ending at the same roof height), which z-fights in the viewer and
splits the map pixels of that wall between two half-empty tiles. Every
face is a polygon in the canonical 2D frame of its plane (planes within
0.5° and 0.25 px are the same); faces on one plane facing the same way keep
only the highest-priority one over the overlap (terrace > opaque box >
non-opaque box, then larger area; `polygon-clipping` difference), faces
facing each other lose the overlap on both sides since it is inside the
joined block, slivers thinner than 1 px are dropped and the rest is
re-triangulated with earcut (holes included). York: 183 faces trimmed
behind a same-facing face, 147 shared interior overlaps removed.

The GLB is a scene hierarchy: `map` → `ground` (quad), `buildings` and
`terraces` groups with one node and mesh per obstacle (`building-042`,
`terrace-086`), all sharing the atlas material. `volumes.ts` also exports
`reconstruct(map, opts)` (volumes, id buffer, textures, no files) for the
bake (`bake.ts`, see `3d-editor.md`).

Texturing is a reverse projection with a per-face atlas. The camera is
orthographic, so every surface point maps to one map pixel, but a map pixel
belongs to exactly one surface: a full-resolution id buffer assigns each
pixel to the nearest camera-facing face. Each face gets a tile holding only
its own pixels (projected bbox, 1 px pad, may extend 512 px past the map
edge); everything else — occluded parts, whole back faces — is unknown.
Walls running away from the camera project to a sliver (base spanning less
than 0.4 of its length in map x) and get their tile in their own frame
instead (column = distance along the base, row = height) so they keep
resolution. The ground is a quad over the whole map treated the same way
(pixels owned by a face are unknown). The unknown pixels are then

- `--fill synth` (default when `~/.cargo/bin/texture-synthesis` exists,
  `cargo install --locked texture-synthesis-cli`, or `TEXTURE_SYNTHESIS=`):
  every face with ≥ 100 own pixels and at least 16 px on each side, plus
  the ground and terrace tops, is inpainted from its own pixels by the
  EmbarkStudios texture-synthesis CLI (example-based pixel synthesis,
  ~12 parallel processes); hidden and thin faces take the `proc` path
  below with the synthesized tiles as donors. York: 1807 tiles in 236 s +
  ground 79 s. Chosen over G'MIC after a side-by-side on real tiles, see
  `texture-synthesis-survey.md`.
- `--fill proc` (no external tool) filled procedurally. Roof tops are split into
  planar parts (a gable's two slopes are separate faces) and every wall and
  roof part has a local 2D frame in scene units (u along the base or ridge,
  v = height or distance down the slope), so any two faces line up at the
  same scale. Per face, in order:
  - a faithful 2D reflection of its own pixels across the visibility
    boundary (each unknown pixel takes the pixel mirrored through its
    nearest known pixel; no repetition);
  - then a donor copied at 1:1 scale from the largest rectangle inside the
    donor's polygon, mirror-repeated where the recipient is larger: walls
    take the opposite wall of the same building (mirrored), else its best
    visible wall, else the nearest visible wall within 600 px facing the
    same or the opposite way; roofs take the other slope across the ridge,
    else another part of the same roof, else the overlapping neighbour's
    roof, else the nearest roof. Donors need ≥ 0.3 of their polygon visible
    and must already be filled; only own, reflected and donor pixels are
    ever copied further (never repeated reflection or smear);
  - faces seen at a grazing angle (projection < 0.35 of their true area)
    get a tile in their own frame so they keep resolution; below 0.2 the
    map holds only a sliver that would stretch into lines, so they count as
    hidden. Hidden faces get a half-resolution local tile from the donor;
  - ground and terrace tops copy coherent patches of nearby visible ground:
    per connected hidden region, the shift (16 directions × 8 magnitudes
    relative to the region's extent) that brings the most visible ground
    onto it and matches the colours along its border best, repeated for
    what stays uncovered; seams between patches are feathered over 3 px;
  - what nothing reaches is reflected with repetition, then smeared. York:
    2209 faces with own pixels (1889 completed from a donor), 3089 hidden
    walls and 243 hidden roofs on donors, 27 flat-colour faces (obstacles
    nobody sees at all); 8192² atlas, 46 Mpx of tiles, 29 MB GLB, ~12 s.
  `--closeups x,y;x,y` adds a contact sheet of orbit close-ups (4 yaws per
  point); `--debug-fill` renders it again with every face in its fill
  category colour, `VOLUMES_ID_ATLAS=1` with face ids instead, and
  `VOLUMES_DUMP=dir:f,f` writes those faces' tiles and masks.
- `--fill smear`: the old fallback — iterative neighbour averaging from the
  face's own pixels, donors as above.
- `--fill none`: unknown pixels stay transparent (alpha 0, PNG atlas,
  materials in MASK alpha mode) so an inpainting model can fill them
  later; the software renders show them bright green. York: 74 MB GLB,
  28.8 M transparent atlas pixels.

The same-view render reproduces the map exactly in every mode, and from any
other angle no face wears pixels that belong to something in front of it.

Limits: geometry is only as fine as the sight volumes (no chimneys,
dormers, overhangs beyond what the artists blocked out); the base ground is
flat; terrace cliffs have no visible cliff donor in York and wear house
walls, and the painted river banks are slopes where the volumes have
vertical cliffs; the procedural fill repeats and mirrors, it does not
invent — see `texture-synthesis-survey.md` (quilting, graph cut, PatchMatch,
Wang tiles) and `ai-texture-completion.md` (inpainting models, view-based
texturing) for the next steps.
