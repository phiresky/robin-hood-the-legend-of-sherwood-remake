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

Complete assets beyond the map boundary rather than terminating their geometry
at the source-image edge. Preserve the observed in-map silhouette and artwork;
mark extrapolated trunks, branches, crowns and materials as inferred. Review
the entire completed asset from all eight directions, using supplemental wider
cameras when the frozen packet clips the added geometry. This applies to other
edge-clipped scenery as well as trees.

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

Patch-controlled obstacle groups retain native metadata for state integration;
they must not become permanently visible buildings or fences without artwork
evidence. `state_inventory.py` audits the nine native patches and their old/new
mask and obstacle sets. `review_ownership.py` writes a separate revised catalog:
obstacle 144 leaves tree 03, rock 133 joins the northwest outcrop, and obstacle
132 is identified as northern wood mask 21. Frozen worker catalogs and approved
groups remain unchanged. The
complete 15 animation sequences, nine native patches and 129 mission patches
remain source evidence; a synchronized first frame does not implement animation
or prove every mission state. Mask-only undergrowth/grass, three unassigned wood
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

The second pass extends irregular crown ownership to unapproved trees with
`revise_crown_edges.py -- --masks <ids>` in Blender. It preserves native RGB,
records the inferred edge separately, and refuses to revise approved trees 01
and 25. Tree 42 already uses a whole canopy mask and needs no partition revision.
`haystack_geometry.py`, used by `revise_feedback.py -- --assets
south-field-haystack`, builds two closed halves of a continuous rounded mound.

For bark, first inspect each tree's own source crop and write
`inspection/bark-donor-selection.json` with `native_mask`, a map-image-coordinate
`source_box` (`left, top, right, bottom`), `source_sha256`, reviewer and notes.
`refresh_tree_bark.py -- --masks <ids>` checks that selection, preserves geometry
and accepted source RGB, and refreshes saved-material evidence. A brown sample
alone does not prove ownership: reject foreground kindling, soil and leaf pixels.
Run `python3 level-editor/blender/croisement02/self_review_packet.py <ids>` to
assemble current eight-view and source-comparison boards. This command never
grants readiness; manual observations belong in hash-bound visual-review records.

`revise_tree03.py` removes obstacle 144 in a new frozen worker and passed source
and eight-view self-review. `revise_relief.py` reconstructs closed shared-vertex
rock volumes, rounds exposed corners, and uses the revised ownership catalog.
`complete_boundary_crowns.py` adds inferred off-map continuations to 24/40;
`complete_north_tree23.py` replaces an incorrectly associated in-map crown with
an inferred crown above its northern trunks. These completions still require
full-crown visual review. `prepare_tree21.py` replaces the former bank with its
native wood silhouette and an inferred northern crown. Its bark excludes
foreground canopy 134. The revised tree 23 wood uses row-span sweeps to retain
the small visible root edges that a simplified skeleton missed.
`complete_north_tree20.py` separates its northern trunks from a formerly assigned
canopy domain already covered by other trees, recording the overlap evidence.
It adds an explicitly inferred northern crown and excludes foreground canopy
135 from bark projection. Northern crowns use irregular volumes and varying
leaf-card orientations; horizontal depth is at least their width. Boundary
24/40 completions join the observed half across the map edge rather than leaving
a straight gap. These changes still require current full-crown self-review.
Remaining self-review TODOs include inferred bark on 41.
Tree 23's former canopy
lay left of its map-edge trunks and is not evidence of their actual crown.
These candidates remain in progress even where silhouette and depth metrics pass.
Preserve frozen inventories and cameras; corrected ownership needs a new worker,
and extra inspection views must be explicitly recorded.

`audit_map_edges.py` inventories native foliage domains touching the image edge;
contact is a review cue, not an automatic failure. `complete_northern_caps.py`
creates fresh `forest-v4-round-3` workers for northern continuations, retaining
the earlier models and approvals as archived evidence. Added surfaces are
explicitly inferred. Existing crown vertices, faces, UVs and ownership are
checked for preservation, and the saved appearance is retained after projection.
New geometry requires its own review; it does not inherit the previous approval.
The cap recipe also handles tree 39's eastern continuation. Added leaf volumes
use world-aligned radii; stretching a narrow cap along the full prior depth
produced long tilted protrusions and failed self-review.
`verify_edge_completions.py` reopens both generations and verifies the retained
mesh, UVs, source ownership, transforms and packed appearance independently.
`partition_tree40_crown.py` separates the rear eastern crown from the foreground
tree 39, proving that the neighbour still owns every removed source pixel.
Run its boundary completion and `repair_bark_visibility.py -- 40 --redo` next;
the latter removes foreground canopy/shrub colour from bark. Joint scene
coverage remains a separate check from that source-domain proof.

`repair_tree_roots.py -- --masks <ids>` reconstructs the existing canonical wood
assignments, joins short trace gaps for single-part trees and tree 06, and unions overlapping
tubes before bending low roots toward ground along the source ray. Union must
precede the ground deformation: remeshing already flattened roots can erase
thin connections. Cross sections, gap bridges and ground contact are inferred;
review the source overlay and all eight views after baking. Approved models are
guarded against revision. Root repairs are candidates, not automatic approvals.
Root revisions for 06/15/30/35/39 passed the subsequent source-overlay and
eight-view self-review; inferred basal textures still need texture review.

`render_full_crown.py -- 17 19` adds eight solid-mesh and saved-material views with
at least 1.5 times the original orthographic scale. It recentres on the complete
visible geometry and enlarges further if required for 20 percent padding.
Original frozen views remain intact. Stale supplemental packets are archived.
The supplemental solid mode displays opaque foliage cards; use the actual
materials to judge leaf coverage. The gallery binds these supplemental images
and camera evidence to the saved model. These wider views address review-frame
clipping, not authored map-boundary cropping or missing out-of-map artwork.
Both supplemental packets passed visual review with the complete crowns visible.

`repair_bark_visibility.py -- 0 5 27 41 46` uses wood self-occlusion inside native
wood domains after excluding overlapping native shrub/canopy masks. Coarse
neighbor proxies do not prove visible bark ownership. Its tree 00 correction
assigns the two traces by their horizontal source positions to parts 044/045,
with each part's own depth. `--redo` archives the previous correction receipt.
Optional aperiodic vertical donor mapping reduces short repeated bands on
unobserved bark; it does not manufacture observed source ownership. Tree 41's
mostly hidden bark still failed visual review despite passing geometry metrics.

Run `prepare_tree18_domains.py` with the same dependencies as `trace_wood.py`,
then `revise_tree18.py` in Blender. The new `forest-v4-round-2` worker freezes
domain 301 (native wood 18 minus the reviewed kindling domain 300). Trunk geometry
is traced again from that domain, a residual isolated stick edge is omitted,
and the bark donor comes from a broad trunk strip above the bundle. The earlier
worker and approved kindling remain intact. `--redo` archives the correction
receipt. Source comparison explicitly uses corrected domain 301. The gallery
and self-review packet select this replacement only after its receipt exists.

Trees 00/05/18/27/46 passed the subsequent source and eight-view geometry review.
Texture completion and integrated ground contact remain separate work.

`stage_review_scene.py` combines current hash-validated workers into
`integration-review/scene.blend`, retaining geometry approvals separately from
unapproved candidates and hidden state metadata. It reconciles all 150 native
parts and renders eight oblique scene views. This private review assembly is
not a publication. The first integrated review remains on hold for foreground
removal, missing mask-only scenery, pronounced oblique foliage layering,
terrain completion and mission/animation state integration.
