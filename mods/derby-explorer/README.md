# Derby Explorer

A small exploration mission for the Rust port, using the installed full-game
Derby map. Robin is the only character. There are no soldiers, civilians,
reinforcements, objectives, or time limit. The startup script opens both
drawbridges, then does nothing. Robin starts in the main courtyard in daylight.

Choose **Custom Missions → Derby Explorer → Robin only**, or run from the
repository root:

```sh
ROBINHOOD_DATA_DIR=datadirs/fullgame_gog target/debug/robin --mission DerbyExplorer --proto Derby
```

The archive can also be mounted explicitly:

```sh
ROBINHOOD_DATA_DIR=datadirs/fullgame_gog target/debug/robin --custom-mission mods/derby-explorer/derby-explorer.zip --mission DerbyExplorer --proto Derby
```

Requires full-game assets; the Leicester demo does not contain Derby.
Use the pause menu to leave when finished exploring.

`DerbyExplorer.level.patch.json` removes the mission's actors and events before
runtime construction and retains a single Robin spawn. The terrain, buildings,
and navigation are supplied by the installed Derby map. `package.py` generates
the startup bytecode and rebuilds the ZIP after edits to the mission patch.

Validation: custom-mission discovery accepts the archive; a headless launch
reports one PC, zero soldiers, zero civilians, and zero targets.
