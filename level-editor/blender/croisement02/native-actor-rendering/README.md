# Current actor frame binding candidate

Private CPU-tested physical-preview preparation. No application runtime imports
these modules. Run `node --test level-editor/blender/croisement02/native-actor-rendering/*.test.mjs`.

The caller supplies a stable source identity and one synchronous presentation
snapshot: validated body geometry/texture, corresponding shadow texture and
common canvas bounds, current anchor/activity/orientation, projection elevation,
support-height query and explicit shadow style. Bounds use the preview's upward
`top` coordinate, not a raw downward sprite offset. Already-composed frame
textures must not receive the original source offset a second time.

Both body and projected shadow change in one synchronous application. Fallible
support sampling completes before visible resources change. The binding owns
only its generated shadow geometry/material; frame textures and body geometry
remain borrowed from the loader. It starts with a body without a pre-existing
shadow; migrating an existing loader must retire that loader-owned shadow using
its own disposal registry. Recompute on placement/support changes as well as on
direction/frame changes. This prototype deliberately allocates each sample;
production caching needs explicit support revisions and frame identities.

There is no action chooser, clock, script-handle mapping or AI. Inactive and hidden
snapshots retain source identity but remove the shadow and hide the body. Missing
body resources throw; a null frame explicitly means hidden artwork. Destination
keyed shadows, native current-layer masks and ordered scene composition remain
separate backend work: this physical preview shadow is not a claim of pixel
parity. CPU tests use synthetic geometry and resource objects, not browser image
decoding. Existing marker-source tests separately bind the actual exported art.

TODO: integrate only after the shared-runtime publication freeze is released,
using current identity/state snapshots and the loader's cleanup registry. Verify
direction changes, moving support, hides, native/physical switches and scene
retirement in the browser with real actors. Preserve construction order separately
from script registration; do not substitute scene child order for either.

`planCurrentScene` merges already-resolved actors/effects from one epoch/tick
snapshot. Background restoration uses construction rank independently of Y;
ordinary draws use float32 display order then construction rank. Inactive entries
remain accounted for but do not draw. The caller must resolve actual creation
ranks across all families, including invisible constructions, before calling it.
It intentionally cannot derive ranks from loader batches or script indices.
Display order is supplied current state, including any relative-order override;
the planner does not recalculate it from an initial mission position.

Character rows require current map position, layer and hidden-outline policy.
The current private bitmap masker supports ordinary alpha removal only; active
hidden-outline requests fail explicitly. Masks must come from current active
membership, not an unconditional static inventory. Backend integration still
needs an isolated per-object target, mask application in native pixel coordinates,
and destination-keyed composition of body/shadow before the next ordered draw.
Render the static scenery without these bound dynamic identities first; keep
background restoration separate. Ordinary oblique rendering uses world depth.
Do not globally clear scene depth or mutate shared materials to simulate ordering.

Release gate: prove real moving actor/sign overlap, masks crossing native layer
boundaries, directional shadow changes, pause/reset and native/oblique switching
with a single externally advanced clock. The existing source-art GPU fixtures do
not provide that evidence. Current actor AI/script state and hidden-outline
support remain explicit blockers, not inferred defaults.
