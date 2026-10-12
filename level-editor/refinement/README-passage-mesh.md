# Passage mesh review

Run `node refinement/review-drawbridge-passage-mesh.mjs <placement-directory>`
after the drawbridge placement exporter. It reads the pinned models and exported
ordinary passage endpoints, transforms each model by its editor placement, and
samples both halves of each approach at 21 positions. It writes
`passage-mesh-report.json` before returning failure for uncovered samples.

This is a continuous centerline-support diagnostic, not a gameplay pass/fail
criterion. An explicit ordinary passage can traverse a small gap between two
receiving floors. Native actor traversal and lock tests must establish whether
that passage works. The report records both vertical mismatch and distance to
nearby mesh at the intended height so a deep void below a short seam is not
mistaken for a long unsupported approach.

It does not verify the actor's complete footprint, animated compositing, or
the support of every point in a walkable surface. Near-floor comparisons use
a 0.5-game-unit height tolerance, and distances are measured in game XY.
