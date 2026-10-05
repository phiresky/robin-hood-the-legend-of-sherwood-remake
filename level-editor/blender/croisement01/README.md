# Croisement01 refinement

Private candidates and evidence live in `work/croisement01-refinement/`.
Publication receipts identify the exact approved scopes already installed in the
library; a private candidate is not evidence of publication. All background
Blender work acquires the shared four-slot render pool before opening a scene.

The frozen source is 1408 × 960 pixels, with 85 sight-obstacle parts,
103 masks on layers 0 and 1, 13 animation records, six native patches and
108 mission patch records. The archived volume scene is only a starting
hypothesis; its grouped parts are not approval or complete source ownership.

`source_inventory.py` writes individual native mask cutouts, untouched contexts,
contact sheets and hashes. Both occlusion-depth layers must be inspected.
`survey.py` shows the existing volume footprints over artwork. `catalog.py`
accounts for every native part while explicitly retaining unresolved groups.
The first named assemblies are six stumps and one fallen branch; mask-only
foliage, full crowns, terrain and mission visuals remain separate work.

## Reproduction

Run from the repository root, creating the work directory first:

```sh
mkdir -p level-editor/work/croisement01-refinement
node level-editor/pipeline/src/export-interior-layers.ts Croisement01 level-editor/work/croisement01-refinement/source-states
python3 level-editor/blender/croisement01/source_inventory.py
python3 level-editor/blender/croisement01/survey.py
python3 level-editor/blender/croisement01/catalog.py
/usr/bin/blender --background --threads 2 --python-exit-code 1 --python level-editor/blender/croisement01/setup_scene.py
/usr/bin/blender --background --threads 2 --python-exit-code 1 --python level-editor/blender/croisement01/group_scene.py
```

Baseline setup refuses to replace existing evidence. Geometry, material and
texture review follow the [shared procedure](../../refinement/PROCEDURE.md).
Inspect saved actual materials from all eight directions and the original
camera before readiness. Pixel preservation is not source completeness.

Only the Leicester southeast cottage tree and Leicester moat bank tree may
serve as external tree construction references. Own-map artwork is valid.
Infer complete out-of-map volume rather than cutting at the image boundary;
keep inferred appearance distinct from observed pixels. Crown depth should
reach its width unless source evidence justifies otherwise. Static foliage
under checkerboard animation remains part of the displayed source.

AI texture generation uses the authorized Sunburst/OpenRouter workflow only
after explicit approval of the applicable geometry. Sparse source inputs must
include actual supplementary reference images. Geometry approval and texture
approval remain separate, and no approval carries over from another map.

## Private construction experiments

`plan_tree_groups.py` records a source association proposal for 27 tree groups;
this is not the final catalog or an approval. `prepare_branch.py` constructs a
connected bent branch and twig from the mask 70 source trace, then saves the
shared packet and actual material views. It refuses to overwrite a candidate.

The stump source split and silhouette fitting scripts are diagnostic experiments.
Their outputs remain HOLD: the tall grass interpretation and flattened grass
revision both failed actual material review, and the fitted silhouette narrowed
the stump base implausibly. Occupancy pixels alone do not establish a separate
plant. Do not promote these experiments, treat fit scores as validation, or use
them as approved texture inputs.

`audit_native_coverage.py` casts independent pixel-center rays through the saved
mesh against the native occupancy domain and reports archived terrain support.
Its percentage is diagnostic: foreground grass and shadows must be classified
from the artwork rather than claimed as wood to increase the score.
`render_terrain_contact.py` preserves saved asset materials and shows the asset
against neutral archived terrain at four frozen cameras. That terrain remains
provisional. For the fallen branch, bank 078 intersects a naive zero-height
placement; later private revisions preserve the native projection while raising
only the bank-facing end. Union operations reject loss of main wood volume.

The mask 68 stump has separate foreground grass in native mask 80. The latter
accounts for only part of the stump mask's non-wood artwork; static leafy growth
beside the bare stem still needs its own complete ownership and geometry.
`prepare_small_stump_grass.py` keeps all grass attempts private. Detached source
pixels, a flat continuous front and rapidly varying radial depth all failed
oblique review. `trace_grass_leaves.py` preserves native alpha and derives rooted
leaf-path assignments; its latest experiment improves continuity but remains
HOLD. Do not copy these experiments as an approved foliage recipe.

## Scoped publication and compact workers

New prop candidates retain only their target and required terrain or joint
context before saving. The approved source packets and earlier evidence remain
immutable. Saved material, native-camera and contact reviews are separate from
texture-sheet review; geometry and texture decisions remain separate.

The tree21/tree22 baseline parts share a group with unrelated parts61/77.
`restart2_partition_group061.py` prepares a private residual with those mesh
nodes, pivots, materials, buffer payloads and gameplay records unchanged.
Tree metadata is then rebased into standalone pivots with world-coordinate
checks. This is a private staging operation until the combined editor proof
and coordinated publication succeed.

The south stump has two native gameplay references even though its reviewed
wood is one continuous surface. The guarded exporter can separate its existing
cap face from its shaft without changing any surface, UV, material or corner
ownership value. Both references, walking/projection surfaces, jump zones,
paired edges and cross-asset segments must survive publication. Metadata
adapters reject unfamiliar fields rather than silently discarding them.

The private east-border leaning-tree recipe transports tube frames continuously
through changes in lean. Switching the ring basis at a tangent threshold caused
a pinched shaft in an earlier candidate. Native references62/63 partition the
unchanged outer surface; `restart2_tree71_surface_union.py` checks that their
combined surface is one closed, nondegenerate component. Per-part seam edges
are intentional, and no internal cap or separate floating wood joint is added.
This construction evidence does not replace native silhouette, terrain-contact
or grouped user review.
