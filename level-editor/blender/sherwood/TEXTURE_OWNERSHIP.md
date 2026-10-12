# Sherwood source-pixel ownership

Follow [PROCEDURE §4](../../refinement/PROCEDURE.md#4-use-masks-and-patches-as-ownership-authority)
and [the source-mask reprojection interface](../../refinement/reprojection.md).
Grouping approval establishes which meshes form an asset. It does not establish
which painted pixels belong to those meshes.

## Current preview limitation

The current `textures/reproject-v2` worker, and the grouping candidate derived
from it, are geometry/grouping previews. The Sherwood Day bake currently calls
`source_projection_bake.bake` **without `source_mask_manifest`**. Its NPZ texel
provenance records first-hit visibility, not reviewed native source ownership.
The ground cleanup mask and separate physical foliage opacity do not close that
gap. Do not treat those previews as mask-validated source textures or use their
protected-source flags as final texture authority.

The four already-generated OpenRouter candidates are bound to those earlier
packets. Their protected-pixel checks prove only that the old packet was preserved;
they do not prove that its original-art pixels belonged to the correct receivers.
After corrected reprojection, prepare fresh packets and invalidate stale generation
and bake bindings. No additional synthesis or texture publication should proceed
from the preview worker.

## Required sequence after grouping review

1. Freeze the accepted grouping and unchanged geometry. Inventory the native
   Sherwood mask PNGs, layer/patch metadata and exact original artwork for each
   applicable state. Keep Day artwork separate from animated-tree source layers.
2. Audit mask assignments against original art at source resolution. Assign masks
   explicitly to source nodes and, where needed, projection components. In
   particular, the ladder/tree split of source 24 and house/access split of source
   102 need component-level ownership. A merged group is not permission to use a
   broad union on every component. Bounding boxes, proximity, a render ID buffer,
   or mask indices resembling obstacle numbers are not ownership evidence.
3. Write a reviewed `occlusion_constraints.py` source-mask manifest. Bind each
   projection to the exact source PNG SHA-256 and state. Record accepted mask
   indices, reviewed foreground exclusions and any state-specific occluder
   constraints. Hash the manifest, inventory and every referenced bitmap.
   Explicitly check coverage: the shared bake leaves unassigned objects
   unconstrained, so merely passing a manifest is insufficient. Receivers with
   unresolved ownership must remain unknown and be reported, not silently
   accepted through the unconstrained fallback.
4. Reproject original RGB through **both** reviewed ownership masks and source
   camera visibility. Keep physical alpha, source ownership and generation/edit
   eligibility separate. Reject foreign art, hidden surfaces and out-of-image
   samples. Preserve world geometry and UVs unless a separately reviewed change
   explicitly requires otherwise. Grouping changes must not copy a neighbouring
   roof, branch, ladder or terrain texture onto the wrong component.
5. Validate accepted and rejected pixels, foreign and duplicate ownership,
   outside-object preservation, exact accepted source RGB, and geometry/UV/world
   transforms. Inspect adjacent assets together and show mask overlays on raw
   original artwork, including an independent source-visible domain check so an
   erroneous exclusion cannot hide itself. Record unresolved gaps explicitly.
6. Only then prepare fresh source-textured and solid Sunburst sheets. The first
   tile must be the original camera. Local editable masks protect accepted source
   texels; `--no-mask` refers only to omitting an API mask and never disables local
   ownership/protection. Synthesize genuinely unknown surfaces, bake using the
   frozen cameras and ownership, and require zero changes to accepted source RGB
   or physical opacity. Remaining uncovered texels stay explicit until completed.
7. Review the actual baked worker and exported materials before publication.
   Bind source artwork, native masks, assignments, worker geometry, camera sheets,
   generated output and validation receipts to the reviewed revision.

Keep the current grouping gallery and submitted decisions stable while the user
reviews it. The corrected mask-constrained texture preview will be a separate
revision; it must not overwrite or silently rebind existing grouping evidence.

## Executable audit and synthesis guard

`prepare_mask_audit.py` renders all 166 native masks beside the original Day
artwork. `source-mask-candidates.json` records explicit semantic associations,
inspected comparison-image hashes, referenced bitmap hashes and unresolved
boundaries. `build_mask_review.py` renders those associations and their proposed
foreground exclusions; it does not approve them.

`compile_source_masks.py --output <new-audit-directory>` compiles only inspected
rules. Every unresolved or absent assignment gets an explicit black bitmap;
the coverage check rejects any receiver that would reach the shared API's
unconstrained fallback. The compiler's `synthesis_ready` field remains false
while source nodes, canopy layers or the terrain domain are unresolved.

`reproject_masked_preview.py` is an **audit-only** Day pass. It opens the approved
grouped worker, resets RGB, projects through reviewed masks and first-hit depth
into the existing UV atlases, and retains physical opacity. It does not promote
unresolved regions to hidden surfaces or synthesize them. `verify_masked_preview.py`
reopens the saved worker and checks the packed RGB, source pixel coordinates,
mask membership, geometry/UV fingerprint and complete receiver coverage.
The `reproject-v3-audit` artifact uses the first 44 inspected Day assignments;
subsequent mask inspections are separate from that frozen bake.

`texture_packets.py` now requires a `source_ownership_validation` binding with
an immutable receipt path and SHA-256 before preparing, generating or baking.
The receipt must say `status: PASS`, bind the exact `worker_sha256`, report zero
`unresolved_source_nodes` and `unconstrained_receivers`, and include nonempty
hashed `evidence`. This final receipt also requires the visual and independent
domain checks in step 5 above; the partial audit verifier cannot produce it.
Previously prepared packets and the four old OpenRouter outputs cannot bypass
this guard. Rebuild them after the full source audit passes.

## Approved model baseline

The user approved all current models with `models: all approved`. The receipt in
`model-approval.json` binds all 80 current asset IDs, the exact worker, the grouping
receipt, and the reviewed northeast oak replacement. Per-card decisions retain
47 displayed revisions and their original preflight statuses; the blanket
approval does not claim that missing packets were rendered or inspected.
The original grouping worker and historical rejection records remain unchanged.

For subsequent mask passes, compile with
`--models work/needs-review/sherwood-refinement/models-approved/approval.json` and a new output
directory. The compiler validates the model receipt and freezes a recipe copy;
the Day audit opens the exact approved worker named in that coverage report.
This model approval does not resolve the remaining source masks or approve
future synthesized textures.

## Completed Day assignments and layered reprojection

`source-domain-partitions.json` records hand-traced polygons and ladder strokes in
original-art coordinates. `author_source_domains.py` renders their source cutouts;
`accept_source_domains.py <inspected-domain-ids>` freezes the inspected images and
bitmaps by content hash. Native silhouettes constrain most domains. Explicit
`clip_native: false` domains recover visible paint outside the native occlusion
mask, including the river bluff and driftwood. They require their own original-art
inspection; they are not inferred from a receiver's bounding box.

The complete Day compilation has 124 source-node rules and 56 exact component
overrides: 34 ladder-oak components and 22 central-house access components.
`verify_source_assignments.py <compiled-directory>` checks component selection,
source evidence hashes and zero ladder-paint overlap with the oak's bark domain.
Foreground foliage and ambiguous seams remain explicitly unknown. A completed
assignment list does not mean every visible source pixel fits the approved mesh.

The ground domain excludes native opaque props and foreground plants (14–165),
plus inspected authored extensions. It does **not** subtract animated crown masks
0–13 from Day: their original pixels live in separate sprite layers. Day contains
bare ground beneath the foreground Arbre05 oak. That oak's inferred wooden
skeleton therefore has an explicit empty Day domain, while its leaf meshes have
an original canopy source. Do not project grass onto that skeleton.

`prepare_canopy_sources.py --output <new-directory>` freezes all six original
frame-zero canvases, their alpha masks and the 36 explicit receiver names.
`reproject_canopy_layers.py --day <complete-Day-pass> --layers <canopy-directory>
--output <new-worker>` projects those layers through alpha-aware visibility in
each original tree family. It retains all approved geometry, UVs and physical
leaf opacity. Source eligibility and unknown-texel flags remain separate from
physical opacity.

`verify_full_source.py` reopens the combined worker and checks source RGB,
Day/alpha-mask membership, physical opacity, geometry/UVs and independent UV-to-
source coordinate samples. Scene renders and the source-visible gaps must still
be inspected before a final `source_ownership_validation` receipt authorizes
synthesis. Day assignment completion alone never opens that gate.

UV island padding is subordinate to every polygon interior. Reprojection clears
an earlier gutter sample when the real interior is encountered, even if the
interior has no valid source sample. Synthesis likewise excludes gutters that
overlap another polygon's interior. The independent UV-coordinate check caught
this case on terrain; do not loosen its pixel-coordinate tolerance to accept
padding sampled from the wrong surface.

After inspecting the exact combined worker in source/east/west scene renders,
record their hashes and the worker hash in `inspection.json`, then run
`authorize_source.py --root <combined-source-directory> --masks <compiled-masks>`.
It binds the independent verification, inspected images, original artwork,
native masks and canopy evidence. The texture gallery uses that same source
directory for scene comparisons and crops original art around each source view.

Fresh synthesis packets use `texture_packets.py --root <new-packet-directory>`;
the default is `textures/sunburst-masked`. The earlier unmasked packets and their
four generated candidates remain historical evidence and are not reused.

After the atlas-center bake, `screen_fill.py` projects actual first-hit fragments
from the same eight frozen cameras into still-unknown atlas texels. This covers
thin hoops and triangle edges whose texel centers miss the visible surface.
Only generated-sheet pixels inside the editable and solid domains can supply
color. The most facing view wins, with proximity to the atlas center breaking
ties; colors are never averaged. Original-source texels, previously filled
texels and physical alpha remain locked. Fragment transfer accepts any positive
facing angle because it transfers an actual visible fragment; the center pass
retains its stricter grazing-angle cutoff.

Render the baked worker again in all eight cameras and inspect those renders.
Report remaining unseen interior texels separately from visible unknown pixels:
fully occluded internal faces are not evidence of a visible hole, and a zero
visible count does not establish that every interior texel was synthesized.

Inspect both raw generations and the protected composite before baking. Reject
outputs that mistake neutral missing-texture gray for pale material. For terrain,
the removed-object silhouettes need continuous ground, not reconstructed props;
use the normal driver's prompt suffix to clarify a failed candidate and retain
the rejected output as evidence. User texture approval remains separate.

Large bakes can use disjoint `--asset` batches under the shared render-slot pool.
Keep all six terrain regions in one batch because they share receiver atlases.
Each normal bake exports hashed atlas updates. `merge_texture_candidates.py`
requires complete target coverage, rejects shared receivers, verifies source
pixels and physical alpha, and transfers texture bytes and their provenance onto
the source worker without importing geometry. Run `verify_texture_candidate.py`
on the saved combined candidate to independently compare every protected source
texel, physical alpha value, geometry and UV against the validated source worker.
The texture gallery requires this receipt as well as inspection of the actual
eight-view baked sheets.
