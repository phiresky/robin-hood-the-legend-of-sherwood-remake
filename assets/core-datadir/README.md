# Core overlay datadir

Always-on overlay datadir shipped with the engine (registered at startup
before the `mods/` overlays; overlays take precedence over the primary
datadir).

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
