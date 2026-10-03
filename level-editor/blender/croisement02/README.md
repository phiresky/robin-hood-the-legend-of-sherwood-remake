# Croisement02 refinement

Reproducible geometry candidates live in `level-editor/work/croisement02-refinement/`.
The published library is not changed by these recipes. The review gallery is
`work/croisement02-refinement/gallery/index.html` (relative to `level-editor/`).

## Tree constraints

Use only these two library assets as construction references:

- `leicester/leicester-southeast-cottage-tree/model.glb`
- `leicester/leicester-moat-bank-tree/model.glb`

Keep wood separate from fixed, crossed foliage surfaces. Do not copy the
references' shallow crowns or gray texture patches. Target horizontal depth at
least as large as visible width, and verify actual alpha-covered geometry rather
than transparent mesh bounds. Check all eight actual-material views. Gray in the
separate source-only sheets means unobserved surface, not a finished material.

This map supplies its own RGB, wood outlines and leaf coverage. Native canopy
masks 128–135 correspond to the eight canopy animation clusters; animation
frames alone contain only changing pixels and are not complete crown masks.
Rounded overlapping supports within a shared canopy are explicit ownership
hypotheses. Hidden leaf depth, branch cross sections and rear appearance are
inferred. Front and rear leaf faces have separate ownership and explicit culling.

`reference_trees.py` records the two reference hashes and orthographic views.
`source_coverage.py` independently renders saved tree geometry from the map camera
and compares it with its assigned masks. `opacity_bounds.py` samples the saved
triangles' actual UV alpha. Neither a silhouette score nor a bounds check replaces
visual review.

## Reproduction

Run from the repository root. Blender jobs use the shared render-slot pool.
A baseline is immutable; `setup_scene.py` and `group_scene.py` refuse to replace
existing baseline/grouped files. These commands assume the archived native volume
scene and extracted full-game data are already available.

```sh
node level-editor/pipeline/src/export-interior-layers.ts Croisement02 level-editor/work/croisement02-refinement/source-states
/usr/bin/blender --background --threads 2 --python-exit-code 1 --python level-editor/blender/croisement02/setup_scene.py
python3 level-editor/blender/croisement02/source_evidence.py
python3 level-editor/blender/croisement02/survey.py
python3 level-editor/blender/croisement02/catalog.py
uv run --with scikit-image --with pillow --with scipy python level-editor/blender/croisement02/trace_wood.py
/usr/bin/blender --background --threads 2 --python-exit-code 1 --python level-editor/blender/croisement02/group_scene.py
/usr/bin/blender --background --threads 2 --python-exit-code 1 --python level-editor/blender/croisement02/reference_trees.py
/usr/bin/blender --background --threads 2 --python-exit-code 1 --python level-editor/blender/croisement02/stage_forest.py
/usr/bin/blender --background --threads 2 --python-exit-code 1 --python level-editor/blender/croisement02/refine_forest.py
/usr/bin/blender --background --threads 2 --python-exit-code 1 --python level-editor/blender/croisement02/finalize_trees.py
/usr/bin/blender --background --threads 2 --python-exit-code 1 --python level-editor/blender/croisement02/refine_scenery.py
/usr/bin/blender --background --threads 2 --python-exit-code 1 --python level-editor/blender/croisement02/repair_scenery.py
/usr/bin/blender --background --threads 2 --python-exit-code 1 --python level-editor/blender/croisement02/render_candidates.py
/usr/bin/blender --background --threads 2 --python-exit-code 1 --python level-editor/blender/croisement02/render_candidates.py -- --scenery
/usr/bin/blender --background --threads 2 --python-exit-code 1 --python level-editor/blender/croisement02/audit_candidates.py
python3 level-editor/blender/croisement02/build_gallery.py
```

`refine_forest.py -- --masks 42` restricts work to selected wood-mask IDs.
Completed current workers are skipped. `--redo` explicitly rebuilds a worker.
`finalize_trees.py` migrates older leaf-patch candidates to correctly oriented,
separately owned front/back surfaces; fresh candidates already use this form.
It also preserves inferred rear material slots in the reprojection fallback
attribute, so refreshing source projection cannot relabel backs as observed.
`--render` resumes any interrupted inspection renders for corrected models.
Review images are archived when replaced, and readiness requires a visual-review
record bound to the saved model hash. Source-only sheets and actual-material
sheets are deliberately distinct. The shared worker validates outside objects,
source references, masks, transforms and frozen review cameras.

## Scope and remaining gates

The initial catalog accounts for all 150 native obstacle parts in 68 groups:
44 trees and 24 scenery/state groups. Part IDs 018/024 are masonry wall returns;
`croisement02-east-rail-fence` is a retained legacy candidate identifier, not an
accurate material description. The repair recipe uses masonry and the gallery
shows the corrected name. Foreground underbrush masks are excluded from affected
scenery receivers rather than projected onto masonry or wood.

Three state groups retain native geometry pending effective-state review. The
complete 15 animation sequences, nine native patches and 129 mission patches
remain source evidence; a synchronized first frame does not implement animation
or prove every mission state. Mask-only undergrowth/grass, four unassigned wood
masks, terrain foreground removal and full-scene gap checks remain integration
work. Their absence is shown in the gallery, not hidden by the native-part count.

User decisions are archived by `record_feedback.py` against the exact gallery
revision, model and image hashes in `user-reviews/`. Rebuilding the gallery keeps
approved geometry hidden and keeps feedback attached to its original revision.
A source-projection correction may retain geometry approval only when its saved
vertex/face/transform signature is unchanged. There is no texture approval, AI
texture synthesis or publication.
Follow the [shared review gate](../../refinement/PROCEDURE.md#7-review-and-approval-gate)
before synthesis or publication. Unknown bark completion here is a deterministic
same-tree donor applied only to unobserved samples; accepted source RGB is checked
unchanged by the shared baker and per-texel provenance is retained.

## Self-review before handoff

Compare each revised asset with its source crop and native masks, then inspect
all eight solid, source-only and actual-material views. Use `compare_source.py`
for exact-camera scenery comparisons. In particular, check wall crest profiles,
gate posts and braces, pile continuity and depth, visible-source texture
coverage, and artificial crown cuts. Passing hashes and bounds is insufficient.
Keep a candidate in progress until these visual checks pass; bind the review
to the saved model and actual sheet hashes. Gray source-visible faces are a
projection defect to investigate, not automatically unobserved surfaces.

The first feedback pass is reproducible with `record_feedback.py <pasted-text>`,
`prepare_kindling_revision.py`, `prepare_wattle_revision.py`, then
`revise_feedback.py -- --assets <slugs>` and `revise_tree25.py` in Blender.
`--redo` archives the preceding correction receipt before another correction.
The kindling and wattle workers live in `scenery-round-2`: corrected authored
source domains require a new frozen inventory. The native actor-occlusion mask
for the wattle fence omits some visibly painted weave, so its supplemental
coverage is explicitly traced from the source artwork. Tree 25 retains the
shared native canopy authority and a separately recorded inferred crown edge.

For each revised scenery worker, run `compare_source.py -- --assets <slugs>`
in Blender before self-review and rebuilding the gallery. Its three panels
show source artwork, saved geometry at the exact map camera, and an overlay.
No new user approval is implied by an assistant self-review.
