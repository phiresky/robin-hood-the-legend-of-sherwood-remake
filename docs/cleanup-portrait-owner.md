# Portrait cache upload ownership

PortraitCache retains non-Clone OwnedSurface tokens for every managed portrait,
action, decoration and requirements-table upload. Private cache slots and public
lookups expose borrowed SurfaceHandles, and HUD dimensions and draw submission
validate renderer provenance. The existing standalone pic_to_surface helper
remains the compatibility boundary for unrelated UI owners.

Loading builds a fresh candidate, preserving optional missing artwork and sparse
authored subframe indices. Required PNG and invalid upload errors retire the
candidate and retain the previous cache. Successful replacement retires every
previous managed upload, releases directly owned RGBA images, and preserves
localized/generated names. Retirement validates the whole bank before mutation,
clears all borrowed slots, and supports repeated calls. MissionPresentation Drop
retires portraits while its renderer is still alive, including early exits.

Direct RGBA images already own their GPU objects; they remain private and are
bound to the cache's renderer identity before panel drawing. Queued GPU bindings
can outlive explicit upload retirement. No layout, gameplay or draw-order changes
are intended; the former identity widget transform's integer rectangle rounding
is retained in the typed draw helper.

TODO: migrate remaining unrelated integer UI owners separately. A dropped cache
outside MissionPresentation still requires explicit retirement before renderer
teardown, consistent with the existing OwnedSurface contract.

Validation: initial focused default client UI suite passed all 28 tests after
8m24s cold compilation. Follow-up review combined paired dimension lookups,
kept panel identity checking constant-time, and reused the owned upload helper
for sparse requirements pictures so malformed RGB565 payloads fail consistently.

Validation pending on the combined integration checkpoint: named Vulkan GPU gate (including reload, sparse slots,
failed replacement, duplicate ownership, wrong renderer, repeated retirement),
successful public load with shipped PNGs, focused default client UI tests and
full default client tests/build. Cargo fmt and git diff --check passed.
The coordinator owns the final release binary runtime checks.
