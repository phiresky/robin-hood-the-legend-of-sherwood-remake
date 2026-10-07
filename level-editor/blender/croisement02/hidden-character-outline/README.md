# Private hidden-character outline candidate

Prepare the small local source fixture and run CPU tests from the repository root:

```sh
python3 level-editor/blender/croisement02/hidden-character-outline/prepare_fixture.py
node --test level-editor/blender/croisement02/hidden-character-outline/outline.test.mjs
```

No runtime imports this candidate. It reuses the private character-mask decoder
and applicability query without changing either module or the scene planner's
explicit rejection. The runtime owner must integrate and verify it before that
rejection can be removed.

`decodeLegacy(source, {depth, shadowKey})` accepts unfiltered, binary-alpha legacy
RGBA and produces a packed 15/16-bit surface. The caller explicitly supplies the
decompressed ambient shadow key. Reserved blue maps to that key; reserved green
and zero alpha map to transparency. Do not feed rendered, blended, filtered or
ordinary RGBA-authored artwork through this decoder.

`outlineCharacter(surface, screenOrigin, actor, orderedMasks, options)` matches
the existing masker's positional inputs, but consumes packed pixels and needs a
packed `outlineColor`. It preserves current mask order and returns an isolated
surface plus per-mask counts. The caller supplies actual active membership,
current layer and actor map point separately from the integer screen origin.
`drawHidden: false` performs ordinary removal.

For each set mask bit, compare the current pixel to its right neighbor. Treat
the ambient shadow key as transparent for that comparison. If exactly one is
transparent, put the outline color at the current pixel; otherwise clear it.
The final column of each clipped mask/sprite intersection always clears. Only
horizontal transitions participate; this is not a full contour dilation or a
general color-edge detector. The left edge can therefore appear on a formerly
transparent pixel. Each mask reads the surface left by preceding masks. Unioning
the masks, reading all masks from an immutable original, or applying an outline
after the combined mask would change the result.

Only after all masks, `splitPacked` separates body RGBA from surviving shadow
coverage. Destination darkening remains the composition backend's job; ordinary
body color includes the outline. This avoids reintroducing shadows that a mask
removed. Conversion to display RGB expands packed channels by bit replication.

The runtime chooses the hidden-outline toggle and current actor color. This
candidate does not assume every actor is an enemy, enable the global toggle,
derive script IDs, recompute ordering or change actor activity. It does not
implement selection/hulk outlines, special projectile/flying-human mask queries,
building silhouettes, alternative pixel formats, destination shadow blending or
GPU filtering. Existing scene ownership and clock rules remain unchanged.

The real-data test freezes an Archer frame and mask 14, with the actor image
deliberately translated to exercise masking. Its 206 output outline pixels
match an independent Python packed-pixel calculation. This is source-art
algorithm evidence, not an authored mission overlap or full-scene parity proof.
Synthetic tests isolate mask order, right-border clearing, shadow exclusion,
15-bit conversion, inactive masks and ordinary cutout behavior.

TODO: retain the renderer rejection until the runtime owner supplies actual
ordered mask selection, explicit hidden color, raw packed/key-preserving sprite
input, and native-camera browser proof. Verify all masks in current query order,
including layered/current-state changes; then compare display-format conversion
and GPU output with a captured reference. Ordinary oblique rendering remains a
separate physical-depth policy.
