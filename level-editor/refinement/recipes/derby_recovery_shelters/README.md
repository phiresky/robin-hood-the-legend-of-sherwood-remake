# Derby shelter texture recovery

These map-specific recipes use the shared renderer, ownership constraints,
OpenRouter texture driver and guarded texture baker. Run from repository root.
Outputs live under `level-editor/work/needs-review/derby-refinement/round-2/recovery-shelters-20260923`.

The three approved groups are East Wall Lean-to (`building-069`), South Curtain
Lean-to (`building-067`, `building-068`) and Northwest Cottage Barrel (`building-062`).
Historical generations had no native-mask contract and used the previous sun
elevation. Recovery renders preserve approved geometry and camera framing, apply
reviewed native masks and the selected 48-degree sun, then regenerate through
OpenRouter with two images, high quality and no API mask. Protected source pixels
are restored locally. Raw generations, including the South chimney retry, remain
archived.

- `compare_geometry.py`: initial loaded-library diagnostic. Use the subsequent
  world-space comparison for actual differences; loaded parent transforms and
  vertex reordering can make raw hashes/indexwise deltas misleading.
- `prepare.py`: run in the historical approved worker using Blender `--python`,
  passing its old Sunburst experiment basename after `--`. Saves unchanged model,
  native mask constraints and fresh source/solid packets. `modified-masked-v2`
  avoids overwriting an earlier failed render. East Wall's completed packet is
  in `modified` from the initial successful run.
- `packet.py <asset-id>`: freezes the refreshed render for generation, retaining
  the original explicit geometry approval and recording the authorized mask and
  lighting refresh. It does not imply new user texture approval.
- `compare_world.py`: run in publication11 to compare saved approved geometry
  against live geometry, reporting both indexed and nearest vertex distances.
- `handoff.py`: rechecks bake evidence, outside-object guards and protected pixels,
  then emits hashed scoped import fragments. It does not publish assets.

Use the common `generate-textures.ts --provider openrouter --prompt-variant short
--no-mask --lighting-reference .../solid.png` and `bake_reviewed_asset.py` commands
documented in the main procedure. South's selected result is `baked-chimney-retry`;
the first result left the chimney sides gray and remains archived. The barrel's
front is coarse because its native source artwork is approximately 19×21 pixels;
source preservation intentionally retains it. Hidden bottoms and wall-contact
surfaces may have no visible sample in the eight cameras. Counts including atlas
padding therefore must not be interpreted as visible texture coverage percentages.
