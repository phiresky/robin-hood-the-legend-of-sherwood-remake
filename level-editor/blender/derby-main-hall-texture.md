# Main Hall revealed shell texture repair

The Main Hall's revealed shell retained neutral placeholders on four materials:
interior floor/gallery segments 02, 03 (two materials), and 05. The earlier reveal
control repair did not texture these faces. The covered facade hid some of the
same surfaces, so testing the toggle alone did not detect their missing texture.

`derby_main_hall_texture.py` projects `reference/revealed.png` onto neutral shell
texels through native masks 172 and 174. First-hit visibility includes all placed
Derby buildings, preventing their artwork from being projected onto hidden Hall
faces. Remaining neutral faces use a source masonry
swatch at [650,1050,680,1100], mapped in surface coordinates and darkened to fit
the shell. Furniture and posts use the wood swatch [653,867,671,879]. These
fallbacks are inferred texture, not recovered hidden artwork.

The repair preserves existing nonneutral pixels, geometry, UVs, and original
binary payloads. New materials are used only while revealed. Segments 02 and 05
and the smaller interior meshes receive reveal-only copies so their covered
resources remain intact. The existing revealed materials get private texture
copies; shared covered materials are not modified.

```sh
python3 level-editor/blender/derby_main_hall_texture.py \
  level-editor/library/3d-assets/derby REFERENCE MASKS STATES
python3 level-editor/blender/derby_publish_interiors.py stage \
  level-editor/library STATES PUBLICATION
```

Use pristine inputs for reproduction; the installation backup retains them.
`REFERENCE` contains the full revealed source image. `MASKS` contains the native
mask PNGs and manifest. Refresh derivatives and run the full publication audit
as described in [the interior repair notes](derby-interiors.md), then apply the
publication. The publisher supports a selected subset of the four Derby rooms.

Local evidence: `work/needs-review/derby-main-hall-texture/`. `states-v7/integration.json`
records input/output hashes, 295,894 source-projected texels, inferred fills,
and zero remaining used neutral texels in the repaired materials.

The remaining small interior placeholders are repaired by the same process,
for 23 materials total. Existing projection stretching at oblique angles remains;
this change repairs missing texture, without reshaping the authored geometry.

The repair is installed in the canonical Derby library. The full editor audit
passed all four reveal controls, 40 groups / 271 parts, 40 standalone insertions,
duplicate/undo/redo, save, and full-page reload. The browser-sized model is about
2 MB. Covered source-camera and oblique comparisons differ by at most one
channel level in one pixel; original geometry and covered resource bytes match.
`publication-v7/installed.json` pins the installed files and browser result.
