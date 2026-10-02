# Compile a map into a game mod

Open a map in the level editor and click **Export mod ZIP**. No local compiler,
server-side bake service or filesystem grant is needed. The button compiles the
current committed document revision, including unsaved edits. Finish or cancel
an in-progress spline before exporting. Save and Download still save the editable
map document; compilation does not mark edits as saved.

Place the downloaded ZIP in the game's configured mods directory (normally
`datadirs/mods`, or the directory selected with `ROBINHOOD_MODS_DIR`), then launch
the map from Custom Missions. The installed base datadir supplies characters and
shared resources. The ZIP is an overlay, not a standalone copy of the game.

Map gameplay is compiled independently of mission content. The Mission tab can
author PC spawn points and NPC soldiers, which are included separately in the
exported descriptor. A map with no Mission characters exports no invented spawns.

The output contains:

```text
details.json
editor/editor-<map>.rhlos-map.json
Data/Levels/editor-<map>.level.json
Data/Levels/Day/editor-<map>.map.png
Data/Levels/Day/editor-<map>.min.png
Data/Levels/Day/editor-<map>.occlusion-depth.png
compile-report.json
README.txt
```

Names are normalized and prefixed with `editor-` to avoid replacing base maps.
Two maps whose names normalize to the same name must not be installed together.
The runtime descriptor remains editable JSON. `editor/editor-<map>.rhlos-map.json`
is a separate snapshot of the editable scene, including unsaved changes. Extract
that file and open it in the editor with the referenced pinned asset library;
the ZIP does not duplicate library models and textures.

## Rendering and coordinates

Compilation renders the actual placed assets and spline geometry from the map
camera at one pixel per map unit. It excludes selection outlines, grids,
population/mission previews and other editing guides. Hidden objects/groups are
excluded. Patch visuals always use their initial state, independent of preview
switches. Synthesized texture fill is included regardless of the display switch.
The document's sunlight settings are included.

An explicit export frame wins over the source map size. Otherwise a bounded map
keeps its dimensions and an unbounded map fits its visible mesh vertices.
Fractional crop edges round outward. Geometry and the camera are rebased by the
same crop origin. Exports above 16,384 pixels per side or 64 megapixels fail before
allocating the output buffers. Rendering uses 1024-pixel tiles to avoid depending
on the viewport size or creating one enormous GPU render target.

Depth is a grayscale 16-bit PNG aligned with the map. Its values are
`round(clamp((surface_ground_y - crop_y) / height, 0, 1) * 65535)`, with zero for
uncovered pixels. Physical texture alpha clips the depth pass; alpha used only
for texture provenance does not. The engine compares this field with character
ground Y to hide sprites behind the baked scene.

## Asset-only gameplay compilation

The editor export button reads the placed assets' pinned definitions. It does
not load a proto-level, mission, source-map baseline or precomputed navigation
graph. Asset geometry and features use local coordinates and follow the same
part/group transforms as the rendered models. Source obstacle numbers and
source map names are provenance, not runtime references.

A descriptor can contain a `gameplay` definition (see
`shared/src/asset-gameplay.ts`). Version 1 supports:

- Explicit planar walkable surface polygons, including slopes and holes, attached to asset nodes.
- Part-local collision and sight shapes, preserving per-vertex sight heights.
- Passage/gate endpoints and lockpick flags, resolved against assembled surfaces.
- Lift surfaces, high/low entrances, traversal type and local direction.
- Empty building interiors with shared entrances and per-actor door locks.
- Map geometry without player spawns or NPCs; those belong to missions.

A walkable surface may opt into generated long-jump connections with a `jump`
property, for example:

```json
"jump": {
  "inset": 8,
  "landingDepth": 12,
  "maxGap": 100,
  "maxRise": 40,
  "maxDrop": 60,
  "minOverlap": 16,
  "clearance": { "radius": 4, "height": 60 }
}
```

These are author-selected map-unit limits, not character profile statistics.
Add `edges: [1, 3]` to restrict generation to selected polygon edges; otherwise
every outer edge is considered. Only level takeoff lines are generated. Export
constructs landing bands inside the surface, avoiding holes, and matches current
neighbours within both assets' limits. No neighbour IDs, original placements or
saved jump zones are needed. One edge can connect to multiple destinations.
Touching surface boundaries do not create unnecessary jumps. Flight checks cover
takeoff, both travel directions and the configured body envelope; blocked spans
are removed with warnings. Missing neighbours are normal for generated ledges.
Explicit jump pairs and exact sockets remain available for authored special cases.

For example, a ground-only asset can declare:

```json
{
  "gameplay": {
    "version": 1,
    "collision": "none",
    "surfaces": [{
      "id": "ground", "node": "$root", "height": 0,
      "polygon": [[0, 0], [511, 0], [511, 511], [0, 511]]
    }],
    "doors": []
  }
}
```

`$root` is reserved for a map-background asset. Placeable assets name one of
their descriptor parts. Coordinates are game-world `[x,y,z]`; runtime motion
uses projected `[x,y-z]`. A door's `middle`, `inside` and `outside` are 3D
points; its 2D polygon is at the outside endpoint's height. Type `0` is a
passage and `3` is a gate. Map compilation neither requires nor generates a
player spawn. Surface `height` can be a constant or one value per polygon
vertex; all vertices must lie on a plane. Optional `holes` use the same local XY
frame and height plane. Coplanar surfaces are joined; connections between
different planes still require authored traversal features. Sector and layer
references are assigned after placement; missing or ambiguous endpoints fail.
The engine builds the actual fast-find grid, collision lines, door links and
visibility-route graph from the generated descriptor.

Walkable surfaces may include `projectionMaterials` with `defaultMaterial` and
ordered asset-local material IDs in `regions`. Its optional local 3D `footprint`
extends material coverage across blocked portions omitted from the walking contour.
`priorityHeight` supplies the local bounding height used for overlap selection;
`priority` resolves equal-height overlaps between placements, with higher values
winning. Equal priorities within one asset follow surface order. Conflicting
equal-priority definitions from different placements fail compilation. These fields
contain no runtime sector, obstacle or material-table references. Receiving faces
are partitioned without splitting the navigation area; ground material registration
remains controlled by each material region's `ground` flag.

Optional `movementClearances` use the same plane/polygon schema as surfaces.
They remove openings only from the owning instance's collision derived from
sight solids on the matching plane. They do not erase sight shapes, other assets'
collision, explicit movement blockers or surface holes. Both the collision and
its openings follow asset placement. Offline recovery clips static walkable
regions to each solid's footprint to restore these local openings, reporting
failed geometry operations as unresolved authoring records.

Movement coordinates use the engine's integer grid. Boolean cuts can produce
subpixel fragments that collapse when rounded; these generated regions or holes
are omitted with a compilation warning. Authored surfaces that collapse still
fail validation. Offline recovery also records the area changed by rounding in
its coverage report; successful quantization alone does not establish parity.

Optional `lifts` name an asset-local `surface`, a `node`, a traversal `type`
(`1` stairs, `2` ladder, `3` wall), a local XY `direction` vector and `doors`.
Lift door types are `4` high, `5` low and `6` high crenellation; their `inside`
endpoints resolve on that lift, and `outside` endpoints on ordinary surfaces.
Lift doors may have an empty clickable polygon. The compiler assigns the
reserved lift layer and rotates the direction with the owning part.

Optional `interiors` contain `id`, `node` and `doors` of type `1` building or
`2` building trap. Their inside endpoints belong to a generated virtual room;
only outside endpoints require a walkable surface. All entrances in one
definition share the room, while duplicated assets get separate rooms.
Interiors currently start empty. Doors optionally declare `active`,
`lockedVillains` and `lockedCivilians`, in addition to `locked` and `unlockable`.

The public asset index retains `gameplay`. Publish descriptor changes and update
saved asset pins through the normal asset-revision workflow. No map-level copy
of the asset's gameplay definitions is needed.

**This is not yet complete gameplay coverage.** The editor performs best-effort
export and reports missing definitions or unsupported features as omissions;
strict compiler checks remain available for authoring and regression tests.
Supported navigation joins, jumps, state transitions, mission markers and spline
geometry are compiled from their current definitions. Interior occupants, complete
visual state coverage and connections not described by asset metadata remain gaps.
See [the map-data checklist](map-data-checklist.md) for current verification and limits.
The low-level sandbox helper remains for the small renderer contract fixture;
the editor button always requests asset gameplay compilation.

### One-time metadata recovery

Source levels may be used by an offline authoring migration to restore missing
asset metadata. This is separate from compilation. The current recovery tool
writes reviewed-owner candidates in local 3D coordinates and an explicit gap
report; it does not publish incomplete metadata as valid gameplay:

```sh
pnpm --filter pipeline exec node src/recover-asset-gameplay.ts --map ../library/scenes/derby.rhlos-map.json --source ../../datadirs/fullgame_gog_hackable/Data/Levels/Derby.rhp.json --out ../work/map-compile/derby-asset-recovery
```

The resulting `*.gameplay-authoring.json` files are drafts requiring review and
support for the remaining feature types. They contain localized surface regions
and connection endpoints, without runtime source sector/layer references or
copied graph bytes. Terrain boundaries that include other assets' cutouts,
ambiguous door ownership, masks and patch behavior remain explicit gaps.

For static ground sectors containing raised surfaces, recovery assigns the
projected ground exclusions to the owning assets and fills those exclusions in
the terrain draft. Moving an asset therefore moves its exclusion too. Recovery
keeps the authored terrain boundary and measures reconstruction error. Stateful
ground still requires state ownership. Overlapping projections are checked using
height priority and source-order ties; unresolved overlaps are reported rather
than permanently cutting another asset's footprint into a surface.

When a source surface has been split into several editor assets, recovery uses
their asset-local collision footprints to assign disjoint pieces. It reports
uncovered and overlapping portions for review, retaining separate ownership
even when several parts belong to the same asset. Connection errors include the
endpoint position and nearby containing surfaces to distinguish missing coverage,
height mismatches and blocked geometry.

## Verification

From `level-editor/`:

```sh
pnpm --filter app exec node --test src/map-compile.test.ts src/editor-viewport.test.ts
pnpm --filter app typecheck
pnpm --filter app dev --host 127.0.0.1 --port 5182 --strictPort
```

With the dev server running, in another terminal:

```sh
CHROME=chromium TEST_PAGE=map-bake.html node app/tests/run-lifecycle.mjs http://127.0.0.1:5182
```

The browser fixture tests crop alignment, tile boundaries, hidden geometry,
sRGB output, texture alpha semantics and lossless depth encoding. To regenerate
the small archive used by the Rust loader contract test, add
`TEST_BAKE_ZIP=../crates/robin_rs/tests/fixtures/editor-bake-contract.zip` to that
command. From the repository root run:

```sh
cargo test -p robin_rs --test editor_mod_export
```

This test discovers and mounts the real browser-produced archive, expands the
descriptor into runtime sight/motion data and reads the map, minimap and depth
through the engine's terrain loaders.

The asset compiler is also tested independently of the renderer:

```sh
pnpm --filter pipeline exec node --test ../shared/src/compile-asset-gameplay.test.ts ../app/src/map-compile.test.ts
```

These checks cover placement translation, rotation, duplication, elevation,
surface joins, slopes, holes, unresolved metadata and reopening the archived editor JSON.
The synthetic descriptor in `crates/robin_engine/tests/fixtures` is generated
from `shared/test-fixtures/asset-gameplay.ts`; a cross-language contract test
asserts it matches the compiler output. The Rust test mounts no game datadir:

```sh
cargo test -p robin_engine --test asset_map_compilation
```

It constructs an actual engine and checks grid, sight, door links and blocked
versus clear movement queries. Passing this fixture is not a claim of complete
functional parity for the extracted game maps.


## Spline walls

Spline sources use asset-local physical definitions plus `gameplay.spline` model
calibration (bounds, part transforms and model hash). Export repeats and clips
volumes and walkable polygons, bends them along the current path, and reconstructs
navigation and sight geometry. Corner towers use the same run breaks, rotation,
anchor and scale as the renderer. Raised spans preserve the space underneath.

Prepared strips initially use conservative continuous barriers measured from their
own models. Recipes declare material and sight opacity; wood fences allow sight
through. Openings and walkable wall tops must be authored explicitly. Export warns
about this approximation. It also warns and omits unsupported uncalibrated sources,
cross-section straightening, source rotation and stateful wall geometry. Local
doors, lifts, masks, lights, sounds and material regions are not deformed with a
spline; use ordinary asset placements for those features.

Re-author existing prepared strips and calibrate selectable corners without
changing models:

```sh
python3 refinement/walls/author_gameplay.py --output work/spline-gameplay-edits.json \
  --corners lincoln-east-gate-south-tower nottingham-south-gate-west-tower
node pipeline/src/configure-spline-gameplay.ts library work/spline-gameplay-edits.json \
  work/spline-gameplay-review
```

After reviewing the staged edits, use a new backup directory and `--apply` to
install definitions and update saved-scene pins. The tool rejects changed
descriptors/models, preserves scene formatting, and restores files if installation
fails. New strips built by `refinement/walls/build_segments.py` include these
definitions automatically.

Five curtain-wall recipes additionally declare `walkwayHeight` in scene units.
`pipeline/src/author-wall-walkway.ts` dissolves coplanar mesh caps into compact
solids, selects that explicit deck, and subtracts higher geometry such as parapets.
It stores the resulting polygons and support clearances in the asset. Export then
retains one navigation region through a rising or curved deck while preserving
each receiving height plane. Coplanar tiles keep ordinary shared-edge joining.
Clearances apply only to their owning placement, so another wall or building can
still obstruct the walkway. Undercut openings require separately authored volumes;
the cap authoring tool extends solids to the model base and reports this limitation.
Use `--ids` with the Python command above to stage only selected assets.

## Whole-library checks

Check every saved scene, including custom scenes which reuse assets from other
maps, against its pinned asset definitions:

```sh
pnpm --filter pipeline exec node src/audit-map-compilation.ts --out ../work/map-compile/all-maps-audit.json
```

The check calls the actual gameplay compiler, reports each map separately, and
returns a failure status if any map fails (or the scene directory is empty).
It reads only the saved editor maps and asset library. Passing this check covers
geometry compilation; it does not certify rendered output or gameplay parity.

The separately authorized one-time recovery can process all source-map scenes:

```sh
pnpm --filter pipeline exec node src/recover-library-gameplay.ts --sources ../../datadirs/fullgame_gog_hackable/Data/Levels --out ../work/map-compile/all-map-recovery
```

This writes per-asset authoring drafts and a library-wide recovery report.
Custom scenes without a `sourceMap` use shared asset definitions; their own
terrain and scene-specific map features still need authoring. Recovery drafts
are not automatically installed as complete gameplay definitions.
