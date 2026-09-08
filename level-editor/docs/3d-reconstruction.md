# 3D reconstruction track

Goal: a full 3D reconstruction of a map with every building as a separate
library asset, built from the pre-rendered map images with SAM 3 (instance
masks) and SAM 3D Objects (mesh + texture per mask), both via fal.ai.
York is the pilot: mostly free-standing, simple buildings.

## Pipeline (all in `pipeline/`)

```
pnpm detect      --map york                       # SAM 3 sweep -> work/york-scene/detections.json + overlay.png
pnpm reconstruct --detections ../work/york-scene/detections.json [--only id,id] [--limit N] [--skip-existing] [--parallel 4]
pnpm merge-detections --detections ../work/york-scene/detections.json --auto [--groups 012+013,040+041]  # -> detections-merged.json (blocks)
pnpm reconstruct --detections ../work/york-scene/detections-merged.json
pnpm reconstruct --map york --bbox x,y,w,h --prompt "..." --name "..."   # one ad-hoc building
pnpm reconstruct --asset <id>                     # re-fit / re-run an existing asset (cached)
pnpm scene       --map york --render              # library/scenes/york.scene.{json,glb} + work/york-scene/{compare,view-1,view-2}.png
pnpm contact-sheet --map york                     # work/york-scene/contact.png: every fit, weakest first
pnpm pose-diag   --map york                       # tilt table of all 24 pose readings over the map's assets
```

Every fal response and downloaded artifact is cached under `work/sam-cache/`
(SAM 3) and `work/sam3d-cache/<hash>/` (SAM 3D: response.json, input.png,
mask-N.png, object-N.glb/.ply), keyed by image + masks + params. Re-running
any step with the same inputs is free.

Costs: SAM 3 image-rle $0.005/request, SAM 3D objects $0.02/request. A
York sweep is 24 tiles × 4 prompts ≈ $0.50; ~100 buildings ≈ $2.

### 1. Detection (`detect.ts`)

The Day map (roof-closer patches composited) is swept in 1024px tiles with
512px overlap; each concept prompt (`house, tower, church, building`) runs on
each tile with `max_masks: 32` (endpoint maximum). Masks are merged across
tiles and prompts: complete instances beat ones truncated at a tile edge,
higher score wins among duplicates (mask IoU > 0.5), masks ≥80% inside an
already kept mask are dropped as fragments. `overlay.png` shows every kept
mask numbered (suffix `T` = truncated at a tile edge) for review; edit
`detections.json` by hand to drop or rename entries before reconstructing.

### 1b. Blocks (`merge-detections.ts`)

Houses built against each other are reconstructed better as one model than
as separately masked halves. `--auto` unions masks whose outlines touch
(≥ `--min-contact` px in a `--gap` px band, strongest contacts first, at most
`--max-members` per block, bbox ≤ `--max-side`); `--groups` adds explicit
blocks. Touching in the projection does not prove two buildings are
attached (a house in front overlaps the one behind), so on York almost
everything groups; the decision is left to the fit: a block asset carries
`merged_from`, and scene assembly uses the block instead of its members only
when its fit score (IoU + colour agreement) is at least the members' average.

### 2. Reconstruction (`reconstruct.ts`)

For each detection the 2D asset is written first (same cut-outs and clipped
level metadata as the 2D track, via `asset-writer.ts`), then the map is
cropped around the mask with ~35% context and sent with the mask to
`fal-ai/sam-3/3d-objects` (`export_textured_glb: true`, seed 42). The
returned local-frame GLB is stored as `library/<id>/model.glb` and its
placement is fitted into the map's scene frame (`asset.json.model`).

### 3. Scene assembly (`scene.ts`)

All assets of the map with a `model` are placed by their fitted placement
into `library/scenes/<map>.scene.json` (`SceneDoc`, our format) and merged
into one Y-up `<map>.scene.glb` together with a ground quad textured with
the downscaled map. `--render` rasterizes the assembled scene from the
map's own camera next to the original (`work/<map>-scene/compare.png`) — the
litmus test for placement — plus two orbit views (`view-1/2.png`). The app's
3D mode views the GLB with an orthographic camera; "Map view" reproduces the
original camera from the scene document.

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

## Fitting a SAM 3D model (`reconstruct.ts: fitPlacement`)

SAM 3D returns the mesh in a canonical local frame (Y up, upright) plus a
local→camera pose (quaternion, translation, uniform scale). The endpoint
documents the quaternion as `[x, y, z, w]` and says nothing about the camera
frame; the model code is pytorch3d-based. The reading that leaves buildings
upright is hardcoded as `CANONICAL_INTERPRETATION`: quaternion `[w, x, y, z]`,
pytorch3d camera frame (x left, y up, z forward), and the model's local frame
Z-up relative to the exported GLB (the reference code's `(x, y, z) → (-x, z, y)`
remap). Verified on the York thatched hut: 4° residual tilt, every other of
the 24 readings ≥ 24°. `--diag` tabulates all readings by pre-snap tilt and
silhouette IoU for checking a new map or model version.

The model assumes a perspective camera the painted map does not have, so
translation and absolute scale are discarded and the pose only supplies the
orientation:

1. Gravity snap: the residual tilt of the model's local up axis is removed
   (buildings stand upright); tilts > 45° are warned about.
2. Yaw search: the pose's yaw is wrong by 90°/180° on about a third of the
   buildings, so yaw offsets over a full turn (10° steps, then ±6° refinement)
   are each placed as below and scored by silhouette IoU plus colour
   agreement between an unlit map-camera render of the textured model and
   the map crop (`appearance`, 0..1). The texture is baked from the crop, so
   only the right yaw reproduces it.
3. Uniform scale so the projected bbox area matches the mask bbox
   (`anisotropy` in the log is how far the x/y ratios disagree).
4. Lowest point on the ground plane (Z = 0), projected bbox aligned to the
   mask bbox → scene position.
5. Silhouette IoU of the placed model against the mask (`fit_iou`);
   < 0.5 is flagged and `scene.ts --min-iou` drops such assets. Low IoU
   with a good-looking model almost always means a partial or merged
   detection mask, not a bad placement.

`work/<id>/fit.png` shows crop | model over the crop from the map camera |
four orbit views.

## York status (2026-09-02)

120 detections; 42 reconstructed before the fal balance ran out (HTTP 403
"Exhausted balance"), 78 pending — `pnpm reconstruct --detections
../work/york-scene/detections.json --skip-existing` resumes after a top-up.
Of the 42, 28 pass the IoU 0.5 gate; the weak fits are mostly masks that
include neighbours, truncated map-edge buildings, and the two castle-wall
segments the sweep picked up (b024, b032).

## Volumes track: the game's own geometry (`volumes.ts`)

```
pnpm volumes --map york --render [--fill synth|proc|smear|none] [--closeups x,y;x,y]
# -> library/scenes/york-volumes[-proc|-smear|-holes].scene.{glb,json},
#    work/york-scene/volumes[-proc|-smear|-holes]-{atlas,ground,compare,view-1,view-2,closeups}.*
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

## Known gaps / next

- Lean-tos and annexes are often outside the SAM 3 mask and thus missing
  from the mesh; box/point prompts or a second mask per building would fix
  that.
- Translation and absolute scale from SAM 3D are discarded; relative scale
  between buildings comes purely from the mask bboxes, so a building
  partially hidden behind another is fitted too small.
- Elevated buildings (on walls, platforms) are placed with their lowest
  point on Z = 0; the sight-obstacle `z_bottom` could lift them.
- Walls, bridges and the river are not reconstructed; the ground quad shows
  the flat map artwork instead.
- The app's 3D mode (`Scene3D.tsx`) only views `library/scenes/*.scene.glb`;
  no per-building editing yet.
