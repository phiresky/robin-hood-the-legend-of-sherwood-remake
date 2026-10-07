# Native character cutout candidate

Private renderer primitive; not imported into the editor. `node --test level-editor/blender/croisement02/native-character-masks/character-masks.test.mjs` uses the existing pinned Croisement02 source packet and checks all 142 masks against independently exported PNG pixel hashes.

The caller supplies current mask activity, character layer and map position, separately from the displayed frame origin. Source frames remain immutable. Character masks apply strictly behind their ascending polyline, with inclusive horizontal endpoints; mask bitmaps use MSB-first scanline runs. Non-character masks and other layers do not apply.

This implements ordinary character cutouts only. Hidden-character outlines are explicitly rejected. Projectile/flying policies, mutable grid layer membership, mask state changes from patches, actor state snapshots, and GPU scene ordering still require their own binding and evidence. No actor state, visibility command, movement, or AI is inferred here.
