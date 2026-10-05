# Mission state previews

Open **Crossroads 2**, select **Mission**, then choose **05 · Ambush**. The
**State preview** section offers the log trap and rock trap.

- **Original artwork** shows the selected transition from the game camera.
  Use **Play**, **Pause**, **Reset**, or the frame slider to inspect it.
- **3D initial** and **3D final** show the reviewed endpoint models at their
  map positions. The ordinary camera controls let you inspect their depth.
- Choose **Map only** or another mission to leave the preview. Switching editor
  panels pauses playback and restores the ordinary map view.

The moving presentation uses recorded artwork and timing. The 3D views are
independent endpoint reconstructions; they do not assert a continuous physical
trajectory between shapes. Previewing a state does not execute mission scripts,
change saved mission state, or activate other events. The surrounding published
map keeps its existing refinement status.

## Library contract

`mission-states/index.json` is optional. Each entry identifies a map and mission
and pins its state contract and source JSON by SHA-256. Contracts pin artwork,
models, timing, ordering, and explicit model placement. Reusable endpoint models
use a common family origin; their bindings restore the exact scene positions.

An endpoint can explicitly declare that no object is present. Replacing a placed
map object requires its exact object identity, asset hash, and transforms; an
edited or mismatched placement is preserved and the preview reports an error.
Leaving the preview restores the object's existing visibility, including any
independent patch visibility.

Entries declared as native loops offer artwork playback without initial/final
3D choices. Their frame slider follows the selected loop; nearby animations keep
their independent timing. These entries remain presentation previews, not mission
script execution.

Missing optional catalogs leave the editor unchanged. Declared resources that
are missing or fail validation report an error and retire the preview. Mission
switches and viewport disposal also retire pending loads and instances.
