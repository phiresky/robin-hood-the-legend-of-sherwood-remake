# Central turret and connecting gallery refinement

Technical handoff, awaiting coordinator and user visual review. No AI textures
were generated or published. Recipe commit: `6d7af6093`.

## Follow-up requested by review

Executed correction is now in `trace-v2/model.blend`, with original source
projection reapplied and eight-view renders at the explicitly selected 48° sun.
See [the illustrated correction report](trace-v2/review.md). Nodes 153 and 158
have corrected gap phase/width and front notch depth. Visible counts were
already correct. All thirteen active meshes are closed with positive volume;
817 outside objects are unchanged. The bridge is measured at 7.705° physical
slope, with 24.382 world units rise. Source deck endpoints are parapet-occluded,
so flattening remains unsupported and the bridge is unchanged.

The old `central_alignment_audit.py` and `source-bridge-landmarks.png` are
**withdrawn**: their unverified long cyan lines crossed the courtyard rather
than following the bridge. The script now refuses execution. The new report
uses actual projected deck vertices and explicitly separates them from manual
source corner observations.

The central turret/gallery parapet zigzag is still visibly out of phase with
the source artwork in both the covered and revealed views.  The connecting
gallery/bridge also appears slanted in the camera view.  A camera diagonal is
not, by itself, evidence that the deck is physically sloped, so the follow-up
must compare its world-space deck fit against the two exact source edge pairs
before changing it.  `central_alignment_audit.py` writes that measurement and
the complete top-profile samples for crenellation nodes 153 and 158.  This is
an audit-only step; no texture or catalog publication should use a revised
central component until the profile and bridge comparisons are reviewed.

The isolated `model.blend` contains only changes to canonical IDs152–162,170,181.
All817 outside objects retain exact geometry, world transforms, parent and IDs.
The immutable source is `../../component-followup-source.blend`, SHA256
`cd80b8cd57fe5cbe46b54bc8045a7c4e5637b6586294b592005f2aa967fee364`.
The baseline link points to that immutable source. Original asset-group tags are
preserved; temporary review selection tags are restored before saving.

## Changes

- Closed all13 active central assembly meshes. Inherited separate face islands
  had subunit gaps and missing underside/attachment caps. The13 retained meshes
  now have zero boundary/nonmanifold edges and positive signed volume.
- Rebuilt157 walkway slab,159 connecting wall and160 gallery deck from their
  existing upper-surface triangulations, with coherent matching sides and
  undersides. Walkway/wall lower elevations remain518.83075/354.0261. The gallery
  underside follows its existing end thicknesses, rather than one nonplanar
  polygon crossing its open ends.
- Lowered the161 front-gallery parapet base9 units so it extends below the
  sloping deck; its visible upper edge and footprint are retained. A remaining
  lateral seam is0.1596 units at the closest sampled vertex, below one source
  pixel. This is reported rather than presented as a fully fused assembly.
- Replaced156's intersecting sloped ramp and modeled-tread pair with one closed
  stepped support. Every tread elevation is retained; its underside extends to
  the previous support base518.83075. The old `modeled treads` child is hidden.
  Integration must reconcile that obsolete child, not import both visible.
- Closed153/158 crenellated walls using separate horizontal underside and
  vertical attachment caps. Recalculated outward normals restore correct
  illumination and source acceptance on their previously dark back battlements.
  No crenel profile was filled or replaced with a full-height parapet.

## Source and patch authority

Inspected the unmasked component context and whole Keep covered/revealed crops,
patch000/001 graphics and their alpha footprints, and original-state manifest.
The central walk remains an exterior receiver; none of its reviewed texture
authority overlaps positive patch000 or001 alpha. Thus these surfaces receive
the H03_Der_MK initial covered artwork, never Hall furniture or West landing
pixels. Full attached Hall cutaways remain the Hall owner's separate scope.

Both the actual source bake and all eight preview views bind `source-masks.json`.
The derived authority is the reviewed union of native168/130/136/135/139,
subtracting foreground West bartizan125, North roof/tower140/141, and the
**covered** Hall roof169 (layer6 local15). Mask178 is an alternate revealed-state
mask and is not used in the final binding. A source-y<645 regional restriction
removes the broad containing facade envelope below the covered walk, where the
Main Hall hides this module. These are containing masks plus a reviewed region,
not exact semantic segmentation of each stone surface. The source image,
native inventory, all contributing mask PNGs and patch alpha hashes are recorded
in `authority-provenance.json`.

Final bake:80024 accepted known texels;343311 mask-rejected texels. Final
eight-view audit:140635 accepted source pixels, zero outside-authority samples,
zero ray misses and zero RGB mismatches against the covered source. Native
foreground exclusions are part of the audited authority bitmap. This proves
the stated constraint and pixel provenance, not exact independent per-part
segmentation. `ownership-audit.json` also checks recipe idempotence and contacts.

The first original view retains34468 known pixels versus36771 in the immutable
unconstrained input; the removed regions include the occluded gallery attachment
and Hall-facing surfaces. Freshly corrected outward battlements gain reliable
source coverage. Unknown backs and adjacent-building-covered interfaces stay
shaded gray. Both final sheets were visually inspected across all eight views.

## Reproduce and inspect

Run the unique recipe's `refine()` on the immutable split source, or execute
`run.py` with background Blender and `--threads 2`. `source_authority.py` rebuilds
the source constraints. `run.py` reprojects, renders the exact input cameras and
fixed world sun, checks outside scope, and saves `model.blend`. Existing modified
packets are archived before replacement. `audit.py` verifies the saved result.

- Before: `input/solid.png`, `input/textured.png`, `input/context.png`.
- After: `modified/solid.png`, `modified/textured.png`, `modified/context.png`.
- Scope/topology: `geometry-validation.json`.
- Accepted-pixel/state proof: `ownership-audit.json`, `authority-provenance.json`.
- Native mask inspection: `native-contact.png`, `source-grid.png`.

This remains an attached architectural module supported by neighboring Hall
geometry, not a complete detached building. Source-painted small arches/windows
on the central facade remain mostly surface detail. Hidden wall profiles and
exact gallery end bearing cannot be recovered fully from one source view.
