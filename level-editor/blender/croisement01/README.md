# Croisement01 refinement

Private candidates and evidence live in `work/croisement01-refinement/`.
The current library is untouched. All background Blender work acquires the
shared four-slot render pool before opening a scene.

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
