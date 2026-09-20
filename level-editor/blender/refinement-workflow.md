# Refinement workers

Start with a grouping agent reviewing the entire scene. Run
`refinement_inventory.inventory` against a copied scene and supply the complete
source artwork. The agent writes a catalog assigning every source part exactly
once to a descriptive logical asset: a building owns its roofs, walls and attached
parts. It also records terrain ownership and unresolved grouping questions.
Run `validate_catalog`, resolve ambiguities, and write `grouping-review.json` with
`status: "reviewed"`, a named `reviewer`, `catalog_sha256`, `inventory_sha256`, and
review notes. The hashes refer to the exact reviewed files. Preparation refuses
unreviewed or changed ownership evidence.
State the review scope explicitly: a complete static obstacle catalog does not
prove that animated mission patches such as drawbridges exist in the 3D scene.
Dispatch retains unresolved items, patch candidates and missing-asset records
alongside the reviewed static jobs. Audit and assign those additional assets and
terrain separately before claiming the entire scene is covered.

## One workspace per logical asset

Use `refinement_workspace.dispatch` to create `dispatch.json`, one job per asset,
including exact Blender preparation commands. This does not start agents or edit
the main scene. The coordinator runs the commands, starts one agent per prepared
asset within the configured concurrency limit, and supplies its INSTRUCTIONS.md.
Use separate background Blender processes, never concurrent edits to one session.

Each workspace contains:

```
INSTRUCTIONS.md          ownership, refinement and handoff instructions
workspace.json           stable part ownership and evidence hashes
baseline.blend           immutable full-scene baseline
model.blend              independent worker scene
reference/               immutable source artwork and reviewed grouping
input/
  context.png            raw bounding-box artwork crop, retaining background
  solid.png              eight solid views, tiled 4 by 2
  textured.png           eight source-only textured views, tiled 4 by 2
  views.json             frozen camera/crop/source provenance
  views/                 individual views and reliable-source masks
modified/                identical layout, regenerated from the edited model
projection/              fresh projection audit reports
history/                 previous successful modified packets
validation.json          latest successful handoff scope validation
review.md                worker-authored findings and remaining defects
```

All surrounding scene geometry stays in model.blend for occlusion, with selection
disabled. Only the assigned asset may change. Validation compares the geometry,
world transforms, parent relationships and stable identity of all other working
objects, as well as the immutable baseline, source evidence and input files. It
rejects missing or reassigned source parts. This is an auditable ownership guard,
not an operating-system sandbox; materials elsewhere are refreshed by the shared
projection pass and are not considered worker geometry changes.

The textured review never samples synthesized materials or an old baked atlas.
The editable asset's materials are also rebaked from original artwork during
preparation and modified generation, with hidden texels neutral gray. Generated
appearance in the source snapshot is not retained on the assigned worker asset.
It projects explicit original artwork onto current geometry with scene occlusion
and surface-angle checks. Unknown surfaces show neutral shaded geometry. The
original-view tile follows the same visibility rules: it is not force-filled with
source artwork. Background painted onto a roof still indicates a bad silhouette;
compare against the raw context crop and solid render before texture synthesis.

Authored scenery-occlusion PNGs provide additional source-camera silhouette
evidence. The hackable converter writes them under
`Data/Levels/<map>.rhp.d/masks/`, with placement and per-layer identities in
`manifest.json`. Review component membership and patch state before using a mask:
an actor occluder is not necessarily an entire logical asset, and a union can
contain overlapping foreground scenery. These masks constrain a 2D outline;
they do not determine roof heights or hidden geometry. Use the original context
crop alongside them. `occlusion_constraints.py` documents the explicit, reviewed
association schema accepted by the source projection baker, including source
hash and projection-layer guards. Merely exporting mask PNGs does not activate
constraints for a worker or replace its geometry review.

For maps with reveal patches, pass the audited projection manifest. The worker
copies covered and revealed artwork and the manifest into reference/, retaining
separate interior/exterior receivers. Interior receiver assignment is currently
implemented for Derby; other maps must add an explicit authored receiver review
before using layered projection. A single-image map can omit that manifest.

Supply a patch manifest to `refinement_inventory.inventory` to include base and
mission patch records in the grouping audit. Static obstacle coverage alone does
not establish complete scene coverage. Mission state references, including nested
animation frames, are copied and hashed in each workspace. A modeled mission
endpoint uses its explicitly named mission's initial/applied source composite for
context and exterior projection. Transition poses reject endpoint references;
they need a matching original animation frame. Interior layers remain separate.
All nested patch and animation-frame image references are also copied, retaining
their relative paths and immutable hashes. Missing referenced images fail
preparation rather than producing a packet with broken context references.

## Prepare and regenerate

Run from Blender Python, through the Blender MCP CLI process, or use the module's
CLI. All paths should be absolute when a worker might use a different directory.

```python
from refinement_workspace import prepare, modified, validate, dispatch

prepare("/work/map/workers/cottage", asset_id="map-cottage",
        scene_name="Map Refinement", collection_name="Map Working",
        source_path="/work/map/source.png",
        grouping_manifest="/work/map/catalog.json",
        inventory_path="/work/map/inventory.json",
        review_path="/work/map/grouping-review.json",
        projection_manifest="/work/map/layers.json")

# After saving edits, run in a process opened on the workspace's model.blend:
modified("/work/map/workers/cottage")
validate("/work/map/workers/cottage")
```

`modified` restores/reapplies source projection before rendering. It keeps all
eight input cameras and the raw crop fixed, so movement and silhouette changes
remain apparent. It stages output, validates ownership again, and only then saves
the projected model and replaces modified/. Failures retain the previous packet;
staging artifacts remain available for diagnosis. Input is never regenerated in
place. A deliberately changed baseline requires a new workspace.

Pass the same preparation options except asset_id to `dispatch`, plus
source_blend and max_concurrency. Jobs remain `planned` until the coordinator
actually prepares them; a command manifest is not evidence of completed work.
Two full-scene Blender files per workspace are intentional isolation overhead.

Review geometry first. The worker hands off a reproducible recipe, its model,
the paired input/modified evidence, and explicit unresolved issues. The coordinator
imports only that asset, reruns layered projection, and exports the map and asset
library together. Texture generation and its final atlas bake happen after shape
review, followed by another renderer/editor check.
