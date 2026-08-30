# Core overlay datadir

Always-on overlay datadir shipped with the engine (registered at startup
before the `mods/` overlays; overlays take precedence over the primary
datadir).

`core-overlay-manifest.json` is the canonical, strictly sorted inventory for
packaged targets. It pins this overlay to shipping-datadir schema 14 and records
the byte length and SHA-256 digest of every required file. Packaged desktop
startup validates the manifest and exact physical `Data/` tree before mounting
it. Android's Gradle build validates the same source directory, then Android
startup reads, validates, and mounts all 32 entries before any font or UI
construction. A missing, extra, symlinked, or corrupt entry aborts startup
instead of falling through to game data.

Browser packages deliberately do not copy this complete native overlay. Their
build-specific `preload-assets.json` contains only `arial.ttf` and the engine UI
PNGs reachable by that browser build; `wasm_preload_asset` installs those bytes
before `wasm_boot`. This smaller host-authored preload closure is a distinct
browser packaging boundary, not a partially validated copy of this manifest.

## Native bitmap fonts

`Data/Interface/Fonts/` restores the game's original bitmap fonts
(`*.bfn`, `dialog.fnt`) together with the `manager.cfg` that maps the UI
font roles to them.

The Steam release shipped only the international TrueType font set: its
`manager.cfg` leaves the native column empty for every role and the
`.bfn` files are missing from the depot entirely, so the original Steam
build renders all menus with the Windows system font SimSun instead of
the game's proper lettering. This overlay fixes that by supplying the
missing bitmap fonts (~280 KB, taken from the game demo release, where they
are identical to the CD data). For GOG/CD/demo installs the overlay is a
no-op — the same files already exist in the datadir.

Caveat: the bitmap fonts cover the Latin glyph set. Localized installs
that rely on TrueType for their script (e.g. CJK) can delete this
directory to fall back to their own font configuration.

`arial.ttf` backs the TrueType list-widget fonts (their `.tfn`
descriptors reference the Arial family, which the original game took
from Windows; the Linux port shipped this same file in its datadir).

## Engine UI assets

`Data/Interface/UI/` holds the engine's own UI additions: allied portrait,
pin, stance, patrol, and formation icons plus the generic and named villain
portraits. The complete inventory is 13 font/config files under
`Data/Interface/Fonts/` and 19 PNG files under `Data/Interface/UI/`; additions
to either directory must also update the manifest and the compiled
required-path list. They load through the virtual filesystem at engine-overlay
priority, ahead of shipping and mission bundles.
