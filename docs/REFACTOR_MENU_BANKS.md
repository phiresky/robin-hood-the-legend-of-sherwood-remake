# Menu GPU resource ownership

Menu sprite packs now keep their sparse frame slots and maximum source dimensions
inside private `SpriteBank` records. Public state getters return renderer-provenance
`SurfaceHandle`s, with the original first-slot and cross-pack fallback order.
One private, non-cloneable owner bank retains retirement authority for static and
lazy uploads. Serialized sprite banks retain dimensions, never GPU authority.

Menu backgrounds, widget pictures, buttons, alpha masks, dialogue fades, sliders,
blazons, save/load fields, and main-menu/profile fields use typed renderer entry
points. Popup pictures no longer store menu upload IDs in the engine's legacy
`alternate_picture: u32` field. Unrelated generated runtime bitmap compatibility
paths retain their explicit raw-ID bridge; this does not change engine schemas.

`reload` preflights renderer ownership before loading a replacement and keeps the
old cache when DEFAULT.RES is unavailable. Successful replacement retires the old
uploads, including lazily loaded pictures. `retire` preflights the complete owner
bank before mutation, clears borrowed slots, and permits repeated retirement.
Lazy picture lookup checks one private owner in constant time, since all upload
insertions enforce a single renderer binding; individual draws still validate their
own handles. Full-bank validation is reserved for reload and retirement.
The integration owner installs mission teardown; renderer destruction still frees
uploads on assembly failures and standalone menu exits.

Validation added:

- Pure sparse-state/fallback and serialization-authority regression.
- Named GPU-gate helper exercising public `new`, successful/rejected `reload`,
  wrong-renderer retirement and draws, duplicate ownership/deletion rejection,
  lazy-cache reuse, and complete/idempotent retirement using an in-memory RES file.

`cargo fmt` and `git diff --check` run in this worktree. Compilation and the named
GPU gate run in the parent GPU integration lane to avoid duplicate cold builds.

Final combined source `09a2438b1` passed the full client suite and named Vulkan
gate, including this module's public lifecycle regressions. Graphical multiplayer
and ordinary/save-load replay acceptance also passed. Exact evidence and limits
are recorded in [combined acceptance](REFACTOR_BOUNDARIES.md).

Remaining boundary: legacy generated preview surfaces and engine alternate-picture
integers are intentionally outside this resource-bank migration. Cache lookup keys
for DEFAULT.RES subpictures retain their historical integer encoding; replacing that
encoding with a structured key is a separate correctness cleanup.
