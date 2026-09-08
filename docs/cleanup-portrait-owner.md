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

Validation pending: named Vulkan GPU gate (including reload, sparse slots,
failed replacement, duplicate ownership, wrong renderer, repeated retirement),
focused default client UI tests, cargo fmt and git diff --check. The coordinator
owns the final combined binary build.
