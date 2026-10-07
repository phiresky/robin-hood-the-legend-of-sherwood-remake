# Current actor frame binding candidate

Private CPU-tested physical-preview preparation. No application runtime imports
this module. Run `node --test level-editor/blender/croisement02/native-actor-rendering/frame-binding.test.mjs`.

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
