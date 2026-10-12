# Sherwood grouping revision

The first migration kept too many historical obstacle boundaries. The September
2026 grouping audit instead follows complete huts, furniture sets, platform and
ladder assemblies, trees with their branches and roots, and contiguous outcrops.
The candidate now has 80 selectable assets, down from 107. Following the first
grouping review, the west and west-border platform assemblies merge into their
respective houses. Twenty-six unchanged grouping approvals remain applicable;
the two merged assemblies get fresh review packets. Fifty-two other assets retain
their original membership and names.

`grouping-candidate.json` is a review candidate, not the published catalog. The
live library and `refinement/catalogs/sherwood.json` remain the initial publication
until the revised grouping is reviewed and handed off through normal publication.
The candidate worker is `work/needs-review/sherwood-refinement/grouping-review/grouped-source-only.blend`.
Its geometry, UVs, material assignments, transforms and mesh names match the
corrected source-only worker exactly; only ownership metadata changes.

The central oak contains sources 32, 33, 48, 49 and 52. Its house and platform
remain separate. Source 24 explicitly splits tree meshes from attached ladder
meshes; source 102 explicitly splits house walls from platform rails, braces and
ladders. Version 2 component selectors retain canonical source/gameplay ownership.
Source 48 was already merged into the native trunk mesh and remains a canonical
part rather than a duplicated visible mesh.

The audit inspected world geometry against the original Day art and the composite
with original animated-tree first frames. It corrected misleading east/west and
central-treehouse names, including the southwest shelter and north woodland
shelter. Independent props and separated rocks remain independent. Existing
geometry defects, including the northeast woodland oak feedback, remain separate
refinement work; grouping approval does not approve geometry or generated textures.

To reproduce from the valid source-only worker, run from `level-editor`:

```sh
/usr/bin/blender --background --threads 2 --python blender/sherwood/inspect_grouping.py -- --worker work/needs-review/sherwood-refinement/textures/reproject-v2/source-only.blend --output work/needs-review/sherwood-refinement/grouping-review/geometry.json
python3 blender/sherwood/regroup_models.py --geometry work/needs-review/sherwood-refinement/grouping-review/geometry.json --output work/needs-review/sherwood-refinement/grouping-review
/usr/bin/blender --background --threads 2 --python blender/sherwood/stage_grouping_review.py -- --root work/needs-review/sherwood-refinement/grouping-review
/usr/bin/blender --background --threads 2 --python blender/sherwood/verify_grouping_worker.py -- work/needs-review/sherwood-refinement/grouping-review
```

Inspect all four solid and source-textured views for each changed group. Record
its `packet.json` SHA-256 in `grouping-review/inspections.json`, keyed by asset ID,
only after that visual check. This enables grouping approval without fabricating
user approval. Rebuild the parent gallery before the grouping gallery:

```sh
python3 blender/sherwood/build_model_review.py --packets work/needs-review/sherwood-refinement/textures/sunburst --output work/needs-review/sherwood-refinement/model-review
python3 blender/sherwood/build_grouping_review.py --root work/needs-review/sherwood-refinement/grouping-review
node blender/sherwood/verify_grouping_gallery.mjs
python3 -m unittest discover -s blender/sherwood -p 'test_regroup_models.py'
```

The existing server on port 5190 serves the grouping gallery at `/grouping/`.
Every sheet starts with the original camera, then west, east and back. Each card
includes raw original-art context. Model decisions remain in `model-decisions.json`;
superseded component cards are omitted from the pending model gallery. A grouping
review is a separate revision and does not overwrite previous component decisions.

## Texture ownership after grouping

The source-textured sheets in this grouping gallery are previews, not proof of
exact source-pixel ownership. The Day-art preview bake has not yet been constrained
by reviewed native source-mask assignments. Follow
[TEXTURE_OWNERSHIP.md](TEXTURE_OWNERSHIP.md) before preparing final synthesis
packets or publishing completed textures. Grouping review can proceed unchanged.

## Recording grouping decisions

`grouping_decisions.py` verifies submitted 16-character review IDs against the
current gallery and hashes every archived image and report before recording the
exact user line. Run it with `--gallery`, `--root`, `--feedback` (a text file),
and `--decisions blender/sherwood/grouping-decisions.json`.

Decisions also bind asset ID/name, exact mesh membership, source/component parts,
canonical source owners, and the geometry/UV/material fingerprint. This allows an
approved group to remain approved when unrelated groups change metadata. Any
change to its membership, ownership or geometry requires a new grouping review.
The original review revision and evidence hashes are retained permanently.

The renderer accepts repeated `--asset` arguments to regenerate only changed
packets while always saving and verifying the complete regrouped worker.
