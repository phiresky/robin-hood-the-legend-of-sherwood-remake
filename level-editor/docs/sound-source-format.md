# Disconnected environmental sound shapes

An asset sound's `spatial.polylineBreaks` optionally lists point indices that
start disconnected fragments within `spatial.polyline`. Export writes the same
indices as `polyline_breaks` on the compiled sound source.

For example, points `[[0,0],[10,0],[100,0],[110,0]]` with breaks `[2]` describe
segments 0→1 and 2→3, with no segment 1→2. Indices must be strictly increasing
integers greater than zero and smaller than the point count. Single-point
fragments are valid. Global emitters cannot have fragments.

Spline export clips each authored fragment independently, inserts breaks between
surviving pieces and transforms the result through the placed spline. It emits
one sound source per surviving repetition, preserving the source's sample,
delay, activation, ambience, falloff and noise-covering settings. Disconnected
pieces share that emitter's existing playback clock and channel identity.

The loader validates and copies the prepared indices. Existing sound-distance,
panning and noise-covering calculations skip the absent segments using a sorted
index cursor. They do not reconstruct shapes or create an extra source for each
fragment. Sources without breaks keep their existing contiguous shape. Snapshot
and replay schema 70 includes the fragment indices.

Editor/native fixture equality verifies the cropped source count and geometry;
native loading, delay settings, distance/noise gap queries, invalid indices,
curved/rising placement and asset-local recovery are tested. Audible playback
and full-scene acoustic ownership still need review.
