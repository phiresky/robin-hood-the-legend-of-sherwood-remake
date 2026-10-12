# Cached great-keep texture revision

`keep_texture_revision.py` rebuilds the rejected back-view textures without a new
prediction or geometry edit. Run through Blender with `--background --threads 2
--python level-editor/blender/leicester/keep_texture_revision.py --` followed by
one of the commands below. Output directories must be fresh.

The frozen experiments under `level-editor/work/needs-review/leicester-refinement/round-1/textures`
are `leicester-great-keep-approved-wave2-v1` (A) and
`leicester-great-keep-revealed-approved-wave2-v1` (B). Their cached generation is
`generation-short-no-mask-with-lighting/generated-preserved.png`.

Run the four `bake EXP SOURCE OUTPUT MANIFEST` passes in order:

1. A, A/bake-gaps-v3/worker.blend, new covered exterior directory,
   `views-single-exterior-v4.json`.
2. A, preceding worker.blend, new covered final directory,
   `views-covered-visible-interior-v5.json`.
3. B, preceding covered worker.blend, new revealed interior directory,
   `views-single-interior-v4.json`.
4. B, preceding revealed worker.blend, new revealed exterior directory,
   `views-single-exterior-v4.json`.

Then run `finalize A COVERED_FINAL_NAME B REVEALED_EXTERIOR_NAME OUTPUT`.
The final covered candidate is the second pass; the final revealed candidate is
this finalization output. The recorded reference outputs are A/bake-covered-state-v5
and B/bake-state-v7.

The manifests select `best-facing-single` with coherent preferred cameras on
rear masonry and stone caps. Colors come directly from the preserved predictor;
the optional raw-reference tone adjustment is deliberately omitted. Source pixels
remain protected, so the coarse original roof raster still differs from the
cleaner generated roof. Compare whole surfaces, not only missing-gray counts.

`covered-visible-interior-diagnosis.json` records the 68 faces needing covered
prediction. These are **not** all invariant between states: some extend into
roof-occluded floors or furniture. `shared-exterior-face-scope.json` records only
the proven outside gable, component 294 face 3, identified at view 4 pixel
(130, 280). Finalization applies its covered masonry in both states, preserving
revealed mapping on the other 67 faces. Broadly assigning all 68 creates gray
interior floors and is invalid.

Each bake guards other face materials, polygon vertices, existing UVs and packed
atlases. Finalization composes the two revealed passes' complete material maps,
checks the model hash chain, preserves all but the explicitly scoped face, renders
eight views of each state, requires every covered render to match the primary
candidate exactly, and verifies the revealed assignments round-trip. Publication
must use `paired-material-variants.json` when switching state; visibility alone is
insufficient. Inspect all sixteen final views against the preserved generation
and require independent review before changing review markers. This recipe does
not publish assets or modify the canonical gallery.
