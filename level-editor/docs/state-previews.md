# Mission state previews

Open **Crossroads 2**, select **Mission**, then choose **05 · Ambush**. The
**State preview** section offers the log trap and rock trap.

The log trap is also available in **07 · Ambush**, **09 · Ambush**, and
**02**, **06**, **19**, and **21 · Tactical**. The rock trap is also available
in **09 · Ambush** and those four Tactical missions. Each preview uses its
selected mission's artwork, placement, and timing.

**Net 01** offers empty and occupied outcomes in **05**, **07**, and **09 · Ambush**,
and **02** and **21 · Tactical**. **Net 03** offers both outcomes in the same Ambush
missions and **02** and **19 · Tactical**. Each outcome has original artwork playback
and separate initial/final 3D views. Final net animation keeps looping until paused
or reset; choosing an outcome does not infer which character triggered the trap.

Choose **06 · Pillaging** for **Signposts and ambient animation**. This preview
plays the five signposts with the surrounding animated artwork. Its 64-tick
slider follows the sign loop while nearby animations keep their own timing.
**Play** and **Pause** control the whole preview; seeking or resetting the sign
loop leaves nearby animation phases independent. Switching **View** preserves
playback and the current phases.

In that preview, choose a **Character** and enable **Show character in artwork**
to include it at its current edited position and facing. **Show hidden outline**
adds its outline where the artwork obscures it. The character choice stays the
same when you edit its position.

For a character positioned over **North Woodland Bank**, select that placement
under **Shadow surface**, then choose **3D scene**. The selected character's
shadow follows the bank's actual surface. Other surfaces are not yet offered.
Choose **None** to clear this binding. This control applies to the selected
character only; it does not reconstruct the character's body or make the mission
run. The full scene can naturally obscure the character behind foliage.

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

Entries declared as native loops offer artwork playback and a return to the
editable 3D scene, rather than initial/final endpoint choices. Their frame slider
follows the selected loop; nearby animations keep their independent timing.
These entries remain presentation previews, not mission script execution.

Contracts can also declare transient foreground or ground artwork with separate
initial, transition, and final phases. A declared final loop continues until
paused or reset; a nonlooping transition stops at its final frame. Transient
artwork does not permanently paint the background. Source profile hashes and
placement metadata must agree before the preview loads.

Missing optional catalogs leave the editor unchanged. Declared resources that
are missing or fail validation report an error and retire the preview. Mission
switches and viewport disposal also retire pending loads and instances.
