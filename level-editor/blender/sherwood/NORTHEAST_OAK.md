# Northeast woodland oak model revision

The rejected model had a broad, flat-capped trunk below a narrow canopy support.
The revision replaces its wooden structure with a continuous branching volume
following native Sherwood mask 11. The existing leaf meshes are unchanged.

`refine_northeast_oak.py --prepare` constructs front and rear surfaces from the
largest connected native silhouette. Rounded sections, branch depth and rear
shape are inferred along the original camera rays. Eight ambiguous pixel-corner
contacts are bridged by one pixel to avoid nonmanifold edges. Those inferred
pixels cannot acquire original RGB outside the unchanged texture-ownership mask.
Disconnected native specks are omitted from the wooden volume.

The Blender stage replaces the meshes of these existing objects:

- `Tree 042 - tapered trunk.001`
- `Sherwood - Arbre03 tree 042 - limbs forks and terminal twigs.001`
- `building-043.002`

Their source identities and approved asset grouping remain intact. The source
parts partition one wooden surface without duplicate caps at their interfaces.
The stage checks all other geometry, UVs and material bindings against the frozen
approved grouping worker. Only the revised wood receives new atlas UVs and a
native-mask-constrained Day bake. This remains a model review, not a final texture
approval or a published replacement.

The isolated artifacts are in
`work/needs-review/sherwood-refinement/northeast-oak-review/`:

- `candidate.blend` and `stage.json`: revised worker and exact baseline binding.
- `geometry-source.json`: original mask hash and inferred geometry details.
- `saved-worker-verification.json`: reopened mesh/metadata comparison, including
  a combined surface edge check across all three parts.
- `packet/`: four solid and four texture views, original camera first, plus both
  composite original artwork and the bare Day artwork.
- `inspection.json`: visual inspection bound to the packet hash.

Run `render_model_candidate.py`, then inspect its actual images.
`build_model_candidate_review.py` enables approval only after that inspection and
saved-worker verification match the current revision. It publishes to the
existing review server's `/revisions/` path, preserving the completed grouping
gallery and earlier model decisions. `verify_model_candidate_gallery.mjs` checks
images, enabled approval and feedback export in an isolated browser profile.

The remaining source-mask work is tracked in
[TEXTURE_OWNERSHIP.md](TEXTURE_OWNERSHIP.md). The revised oak must be incorporated
into the next full source worker after model review; it must not inherit the
old worker's final source-validation binding.
