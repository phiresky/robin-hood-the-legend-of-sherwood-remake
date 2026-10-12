# Needs-review storage audit — 2026-10-12

Reviewed 192 top-level entries containing 704,073 regular-file paths and
384.43 GiB of allocated file data. Allocation is counted once per device/inode;
symlink targets are not traversed. This is a live-directory inventory, not a
filesystem snapshot. No files were moved, consolidated, or deleted by this review.

## Where the space goes

| Entry | GiB | Finding |
| --- | ---: | --- |
| `lincoln-refinement` | 169.50 | 141.45 GiB of editable Blender models; trees, completion work and texture revisions dominate. |
| `derby-refinement` | 66.77 | 55.90 GiB of editable Blender models, mostly round-2 work. |
| `map-compile` | 59.74 | 53.17 GiB of metadata/diagnostic files; includes active-map inputs and publication history. |
| `nottingham-refinement` | 28.20 | Already underwent a prior revision cleanup; verified raw retention records exist. |
| `leicester-refinement` | 23.51 | Mixed editable sources and texture evidence, some consumed by active maps. |
| `sherwood-refinement` | 11.86 | Mixed models, texture runs, publications and derivative work. |
| `sam3d-cache` | 7.73 | Generated PLY/GLB models, input images and provider responses; not merely temporary files. |
| `library-square-atlas-v6` | 3.91 | Mostly exported models; compare selected outputs and provenance before pruning. |
| `canonical-library` | 2.51 | Historical library material; no whole-directory deletion clearance. |
| `scene-backups` | 2.37 | Explicit dependency of all four active-map setup scripts. |

Across the tree, editable Blender models use 235.44 GiB. All `index.json`
files together use 43.41 GiB, largely repeated library snapshots. Generated
`*.level.json` and `*.level.json.scene.json` diagnostic files use another
4.81 GiB. These are candidate classes, not an approved deletion set, and these
figures overlap the directory totals above. Browser profiles and Blender
autosaves together account for only about 0.36 GiB.

## Verified duplicate contents

Library-index snapshots contain **151 exact SHA-256 duplicate groups**, with
**22.41 GiB** of redundant allocation after retaining one copy per group. These
are distinct inodes, with no extra hardlinks recorded. They are historical
snapshots: preserve each unique version and fix consumers before removing copies.

Model/export files contain **771 exact duplicate groups** across 2,185 file
paths. After accounting for recorded hardlinks, their duplicate allocation
potential is **66.03 GiB**, making **88.44 GiB combined** with the index copies.
This assumes consumers can be redirected to retained content and link counts
remain unchanged. Actual filesystem reclaim can differ with reflinks/compression.
This is consolidation potential, not a deletion manifest.

## Inputs that must remain available

- Crossings01 and Crossings03 texture scripts read Leicester approved texture
  evidence as material references. Crossings03 also reads Leicester audit images.
- Crossings02 reconstruction reads masks from
  `map-compile/occlusion-depth-all-layers/`.
- York and all three Crossings setup scripts read
  `scene-backups/map-manifest-migration/scenes/`.
- A Crossings02 admission script still names Lincoln's render-slot pool; preserve
  synchronization dependencies until their current cutover state is resolved.

The repository reference scan found references to 22 top-level entries. It covers
repository text files up to 5 MiB, excluding work history, library data and
node_modules. References can be outputs or documentation as well as inputs;
absence from this scan does not prove a file is unused. Exact examples are in
`active-map-dependencies.json` and `repository-references.json` in the audit folder.

## Raw synthesis and final source provenance

All **429** files in Nottingham's prior retention list passed SHA-256 verification:
218 PNGs (212 API outputs plus six conservative matches) and 211 generation
metadata files. They remain protected. That list intentionally preserves more
than a proven final-only selection. Nottingham's original master still has the
previously recorded missing external image; raw retention validation does not
repair or validate that master.

The prior migration's zero byte matches between raw/composite images and packed
master textures is **not evidence that final raw provenance is unavailable**.
Baking transforms the images. The correct chain is the publication import's
model hash, its recorded approved texture decision, and the decision's raw and
composite paths/hashes. This review checks those links directly. A verified
historical publication input is protected evidence, but must still be reconciled
with the current live asset/state before declaring the final-only set complete.

The publication-input sample checked 361 records, including repeated plans and
smoke stages. **101 distinct raw/composite/model chains passed** all checks
(89 Lincoln, 12 Nottingham). **260 records need reconciliation**: 233 model-path
checks failed and 146 raw plus 146 composite checks failed (categories overlap).
The failing model paths are absent; the failing image paths exist with different
bytes. This does not establish that the currently published assets are broken.
Derby plans with no embedded texture-decision chain and Sherwood's different
publication format are not covered by these 101 verified chains.

The differing images correspond to 72 distinct Leicester raw hashes and 72
composite hashes across overlapping publication plans. **69 of those 72 original
raw hashes were found elsewhere in gallery history**. Those exact originals must
be retained before pruning galleries, even when a same-named raw file still
exists in its generation directory. The same search located 69 of the 72 original composite hashes and three of
the 173 distinct missing model hashes (using already-hashed duplicate models).
`retain-original-texture-content.json` records the image content to protect.
Three raw hashes remain unresolved within
the named-image search: `leicester-north-church-tree`,
`leicester-northwest-forest-tree`, and `leicester-northwest-tower-tree`.
This is not an exhaustive search of embedded image data.
See `historical-path-issues.json` for the exact paths and matches. Do not simply
refresh an approval hash to match the changed file.

## Cleanup order

1. Consolidate exact duplicate model/export and library-index contents while
   retaining a verified keeper and updating every path/hash consumer. Do not
   replace mutable working files with hardlinks indiscriminately. Identical Blender
   bytes can still resolve relative external textures differently in different
   folders; verify those dependencies before choosing a shared keeper.
2. Retain active-map masks and material references, selected editable asset/state
   sources, exact final raw synthesis outputs, generation inputs/settings, and
   the small approval/publication records that bind them together.
3. Review obsolete unique model revisions and reproducible diagnostics after
   those dependencies have explicit retention records. Preserve current review
   results and recipes; a folder named `scratch`, `cache`, or `approved` is not
   sufficient evidence in either direction.
4. Present a concrete deletion manifest and physical-space estimate before any
   deletion, as requested by the user.

Detailed local evidence and audit scripts are in
`work/storage-maintenance/needs-review-audit-20261012/`: full inventory,
classification of every top-level entry, reference findings, raw verification,
diagnostic candidates, exact duplicate hashes, and publication provenance checks.
