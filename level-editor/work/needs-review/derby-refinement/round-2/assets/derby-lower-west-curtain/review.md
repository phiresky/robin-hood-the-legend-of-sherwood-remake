# Lower Bailey West Curtain second-pass review

## Current partial candidate (after regression repair)

`inspection/complete-wall-candidate-v3/review.md` contains both complete closed
wall sections, reprojection,48° review sheets, and numbered source corner
closeups with local predicted-versus-observed errors. The northern crown is
lowered7.545 world units from a fit to four visually inspected source corners.
Notch counts are025:8,023:13,042:6. These are **not** merlon counts.
Each rebuilt wall passes manifold/degenerate/positive-volume checks. Approved
stairs/turret/supports are preserved. The first two visible gaps per section
have16 source-picked corners; their local analytic RMS errors are2.14px north
and1.64px south. Full ordered source traces and independent rendered-contour
verification remain unfinished, so this candidate is **not approval-ready**.
The earlier revision-v2 remains withdrawn, as explained below.

**WITHDRAWN MERLON REVISION:** The 2026-09-22 replacement below is invalid.
The meshes named “modeled battlements” also contain the complete wall bodies;
replacing them with disconnected boxes deleted those bodies. Ownership-only
validation failed to catch this. The three complete meshes have been restored
from baseline while preserving later approved support/roof changes. The repeat
counts and phase alignment below were not established by source tracing and
must not be used as evidence. Existing revision-v2 images are invalid historical
artifacts. See `merlon-revision-retraction.json`. Corrected merlon refinement
remains pending; do not integrate the rejected revision.

## Wall-walk merlon phase revision (2026-09-22)

The previous long wall-walk projection had two visible defects: the rendered
up/down rhythm was phase-shifted from the source artwork, and the modeled
repeat count was lower than the source.  I rebuilt only the authored
`/ modeled battlements` overlays on nodes 023, 025 and 042 from their measured
wall paths.  The wall/support mesh (045), approved access stair (038) and
approved southwest turret remain unchanged.  Source projection was reapplied
with the fixed framing and the same source mask manifest.

| section | source node | old repeats | revised repeats | path spacing |
| --- | --- | ---: | ---: | ---: |
| north walk | building-023 | 11 | **15** | 32.48 map units |
| north walk | building-025 | 6 | **9** | 28.31 map units |
| south walk | building-042 | 5 | **8** | 15.58 map units |

The revised repeat centers are sampled along the existing authored path, so
the phase begins and ends on the same source corners rather than translating
the walk.  `wallwalk_merlon_revision.py` is the reusable recipe.  Its output
is recorded in `merlon-revision.json`.

Review sheets (exactly the frozen eight-camera packet) are in
`input/` and `modified/`; the key full sheets are
`input/textured.png`, `modified/solid.png`, and `modified/textured.png`.
The modified sheet is source-projected, not synthesized.  Validation and
reprojection both pass (`validation.json`, `modified/views.json`).  Texture
generation remains held pending geometry approval.

Known limitation: the source artwork is low-resolution around the bends, so
the target counts are source-traced repeats rather than a claim about hidden
rear crenels.  Additional zoomed views should be reviewed before catalog or
map publication.

**Latest user-requested wall-walk split:** `wall-partition/review.md` contains the
new actual north/south models and frozen comparison. Approved access stair and
turret remain unchanged. New sections await user approval and root's explicit
component-routing catalog contract; lighting is provisional and AI held.

**Latest grouping handoff:** actual three-assembly split is implemented under
`logical-split/`, with separate component .blend files and fixed-sun/source-only
review sheets. See `logical-split/review.md`. Geometry remains ed9f7782a unchanged;
generation is HELD pending user approval. Shared catalog integration is root-owned.

Status: support follow-up complete; ready for technical integration and user
geometry review. The inherited support-panel cracks listed in the historical
section below are now repaired. No AI texture generation performed. Explicit
user approval is still required before Sunburst.

## Support follow-up: current handoff

New changes affect 037 (wall return), 038 (baseline stair support only), and 045
(continuous walk/support). Each was reconstructed from its authored closed plan
at authored bottom/top heights. This eliminates independently offset face panels
and open bottoms without blindly capping unknown boundary loops. The 30-point
walk remains a level, concave, connected solid through all its original turns.
All five previously accepted roof geometry hashes are unchanged; modeled treads
remain unchanged. `support-repair.json` records parts, volumes and roof hashes.

All 14 active asset meshes now have **zero nonmanifold edges and zero degenerate
faces**, with positive signed volume (`assembly-audit-followup.json`).
Workspace validation again passes with 803 protected outside objects unchanged,
stable IDs retained, and immutable reference/input checks passing.

Current approval views use the shared fixed world-space sun:
`inspection/support-approval-fixed-sun/{solid,textured}.png`. Extra close views:
`inspection/stair-support-fixed-sun/` and `inspection/walk-support-fixed-sun/`.
The frozen `modified/` pair retains input framing/lighting for historical geometry
comparison. Both types were visually inspected. Walk continuity, supported stair
treads, and consistent level heights are preserved. Dark bands in fixed-sun
views are actual battlement shadows, not holes.

### Authored mask and source ownership proof

**Native mask constraints are applied to both the current bake and previews.**
The manifest is `source-masks.json`, SHA256
`388796990bb89557842ddcc5e3d7a7d3b1308268438283284c4b01086753d919`.
Reviewed native IDs: 36,40,43,45,46,48,50,55,56,59,60. IDs43,45,50,55,56 are layer0;
36,40,46,48,59,60 are layer2. These conditional character layers are not assumed
to be 3D depth. `inspection/mask-source-associations.png` shows the reviewed
source cutouts. Node-specific assignments retain the source architecture only;
045 excludes foreground parapet/turret masks36,40,46,48,56,59,60.

Projection source is H03_Der_MK initial mission composite, SHA256
`767343496dfdca645368ea64480a5eac38a27e9951881046464cfcc53f14b469`.
`reference/layers.json` contains no owned part in patch before/after/candidate
sets. Covered and revealed crops (230,1450)-(585,2470) are pixel-identical: no
interior/reveal state is assigned to this asset. No cover geometry was hidden.

The current bake report is `projection/25ab29059702/exterior-ownership.json`:
298,538 known texels, 1,377,078 unknown, 224,556 candidate texels rejected by
reviewed masks. Every one of the 14 active receivers is constrained.
`known-pixel-native-audit.json` independently reraycasts **96,108 accepted known
preview pixels** and reads native PNGs with PIL: **0 outside reviewed membership
or inside foreground exclusions, 0 missing geometry rays**. It records inventory
and all referenced bitmap hashes, also captured in preview `views.json`.
These counts validate the reviewed mask contract, not semantic ownership of
every tiny prop painted within an authored scenery mask.

### Remaining limitations for user review

Hidden lower stair spacing remains an inference; the visible upper steps and
support joins are coherent. Arrow slits, masonry corbels and leaning wall props
remain source texture details. Native collision plans are a coherent structural
guide rather than proof of every artist-modeled surface. The roof's hand-painted
eave/finial silhouette remains approximate at pixel scale; no independent
pixel-perfect silhouette claim is made. Unknown rear surfaces stay gray and
have not been synthesized. No substantial open-seam issue remains in this pass.

For this follow-up, import only changed037/038 baseline support/045 geometry,
preserving the separately accepted028–032 roof and existing038 treads. Retain or
merge `source-masks.json` in global reprojection; otherwise its ownership fixes
will not survive a later global rebake.

## Changed geometry

Only active roof source parts 028–032 changed. All five identities, grouping,
object transforms and parenting are preserved. Straight roof wedges were replaced
with a curved, continuous shell and narrow finial. The earlier apex ended around
map y1963, approximately 38 source pixels below the authored tip. The missing
132-degree rear wedge is now closed through existing parts 030/031. This concealed
continuation assumes rotational symmetry; it does not claim unseen ornament.
Roof underside overlaps the shaft top instead of leaving a hairline gap.

The roof source crop and native mask 59 upper component support the new profile.
Mask 59 is layer 2, bbox (335,1925,73,359); it is conditional and was used as
silhouette evidence, not assigned as an unconditional whole-asset mask. Native
mask 60 covers another shaft component. Other parapet masks were reviewed as
context only, without inventing semantic ownership.

`roof-before/views/view-0-textured.png` and
`roof-after/views/view-0-textured.png` use matching cameras. The old roof samples
wall/background on its right edge and lacks its tip. The replacement restores the
curved source silhouette. Unknown rear surfaces remain gray. The restored finial
is thin in the original; its coarse source pixels remain visible at close zoom.

## Verification

- `geometry-repair.json`: each of five replacement sectors is closed, has zero
  nonmanifold edges and degenerate faces, and positive volume.
- `validation.json`: PASS; 803 protected outside objects unchanged; all owned
  stable source parts preserved; immutable input/reference checks pass.
- `modified/solid.png` and `modified/textured.png`: regenerated with frozen input
  cameras and source projection reapplied, preserving exact packet layout.
- `projection/7b015dce0594/exterior-ownership.json`: source ownership bake rerun
  for all owned nodes with full scene occluders and initial mission-state source.
- `roof-after/solid.png`: eight close angles show continuous roof, no open rear
  wedge. `stair-detail/solid.png` reviews 18 treads, base support and upper landing.
- Full assembly before/after sheets inspected: existing wall runs and battlement
  spacing retained; no arbitrary alterations to hidden stair spacing or masonry.

## Historical issues before the support follow-up (now repaired)

`assembly-audit.json` documents inherited open panel seams in source nodes 037,
038 baseline support, and 045 walk/support. These are not defects introduced by
the roof repair and have not been silently marked clean:

- Part 037: 40 boundary edges. Read-only trial welding within one world unit
  leaves eight edges entirely at ground; closing the bottom yields a manifold
  volume without degenerate faces. Needs a verified repair/reprojection pass.
- Part 038 baseline support: 21 boundary edges. Same trial leaves five ground
  edges; bottom closure is feasible. The separate 18-step mesh itself is closed.
- Part 045: 142 boundary edges. Trial welding still leaves 38 edges spanning
  ground to walk height. This needs explicit loop/assembly inspection before
  filling, rather than automatically spanning an uncertain opening.
- Arrow slits and corbels remain texture details; no recovery of unseen shape.
- Concealed lower stair spacing remains inferred. The upper landing and treads
  look coherent in the reviewed eight angles, but this is not proof of hidden
  game geometry.

Reusable recipe: `level-editor/blender/derby_round2_lower_west_curtain.py`.
The historical first handoff imported five roof meshes. Current support follow-up
integration instructions and validation are at the top of this document.
