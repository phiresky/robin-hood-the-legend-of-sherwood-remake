# Editor-preview integration plan

Runtime edits remain gated on completion and acceptance of the installed-context
browser proof. This plan adds presentation of explicit editor-preview state; it
does not add AI, mission startup, script execution or gameplay parity.

1. Extend the existing mission loader with stable source identities: mission and
   level content hashes, source family/index and retirement epoch. Preserve the
   selected frame, direction, authored placement and projection support. Rescue
   characters already load and must retain their existing identity. Keep editor
   spawn markers and unassigned campaign placeholders explicitly editor-only.
2. Build a separate construction inventory from preserved chunk/group order and
   verified allocation rules. Include invisible constructions where they consume
   ranks; do not count only image-bearing patches. Keep script handles separate.
   Nested mobiles, runtime-created actors and unknown ordering overrides remain
   unresolved until their explicit inventory/state is supplied. Never substitute
   mesh-child order or loader family order.
3. Capture one immutable preview snapshot after the existing authoritative clock
   advance. Each supported entity includes identity/epoch, activity, exact current
   frame and direction, placement, float32 display order, construction rank and
   masking policy. Character mask inputs additionally include current layer,
   actor map point, integer sprite screen origin, ordered active mask membership,
   hidden-outline toggle/color and legacy pixel format/shadow key. Raw mask data
   has no standalone active flag: initial or changed membership needs a verified
   state provider, not an unconditional array-to-active conversion. Unresolved
   state must be reported and excluded from parity claims, not silently guessed.
4. For native-camera actors, decode the untouched legacy frame, apply masks in
   current query order, then separate body and surviving shadow keys. The private
   hidden-outline candidate supplies both ordinary removal and the horizontal
   transition rule; it must run before body/shadow splitting. Merge actor draws
   with ordinary effects by current display order and construction rank, keeping
   background restoration in its separate pass. Isolated object rendering and
   destination-keyed composition must not mutate shared materials or clear the
   full scene depth. Unsupported flying/projectile policies stay explicit.
5. For physical views, update body/shadow/offsets together. Extract actual receiver
   top triangles in native map coordinates and clip them with `receiver-shadow`;
   do not reuse the four-corner prototype on uneven support. Reject absent or
   ambiguous receiver coverage. Keep raised regions disconnected and prove their
   visibility/contact without invented side/ramp faces. Preserve ordinary world
   depth, source resources and exact upper-level ownership/disposal boundaries.
6. Test actual decoded human directions and a marker phase/hide/reset, ordinary
   and hidden masks, and two actors crossing a sign in native order. Include a
   real bank or terrain breakline in physical view. Compare native output bytes
   against independently composed source pixels and inspect oblique contacts.
   Exercise global pause, representation switches, mission retirement and failed
   preparation cleanup. Repeated snapshots must neither advance clocks nor emit
   audio/events. No publication or texture generation is part of this work.

Private prerequisites are CPU-tested frame binding/order, mask decoding, keyed
GPU pixels, hidden outlines and clipped receiver geometry. Their isolated tests
do not constitute a full actor/terrain integration proof. Resource preparation,
source identity and state coverage must be bound together before enabling the
native actor backend in the editor.
