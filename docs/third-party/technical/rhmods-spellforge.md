# RH Mods — Spellforge Editor

- Original source: [Spellforge Editor](https://rhmods.com/tools/spellforge-editor/)
- Author / publication: YetiWizard; RH Mods.
- Language / date: English; publication date not displayed.
- Access: Full page retrieved directly; linked software not downloaded
- Checked: 2026-09-09
- Retrieved: 2026-09-09
- Archived copy: lookup failed (archive.org rate limit); not checked
- Format: header notes, original summary, then complete factual notes (tool page, not transcribed)

The documentation describes a mission editor paired with a game extension, with compatibility stated for version 1.1 and Ready2Play. A same-named Lua file overrides the original mission script; otherwise the game uses its SCB script. LuaJIT 2.1 is specified, with API documentation and shared helpers included in the package.

Edited missions must be started afresh rather than resumed from an in-mission save. Some fields and script arguments remain unidentified. The included Blender add-on imports map geometry but does not export it back to the original format. These are documented capabilities, not independently tested behavior.

## Detailed notes

Page facts: rhmods.com tool page "Spellforge Editor"; author field: YetiWizard; no publication date, version, view count or comments displayed; footer "Copyright © 2026". Retrieved directly on 2026-09-09. No licence permitting reproduction is stated, so the description is summarised in the page's order.

Overview:

- Two parts: a mod for Robin Hood: Legend of Sherwood that adds extended modding capabilities, plus an accompanying editor program for editing missions.

File contents and installation:

- `dinput.dll` extends the game so it can load missions saved by the Spellforge editor, and enables scripting missions in Lua (LuaJIT 2.1). Place it in the game folder next to game.exe.
- `Spellforge.exe` also goes in the game folder. On first start it creates `spellforge.ini` in the same folder, used to modify game paths and some editor settings.
- Missions and maps live under `Data/Levels`. Recommended backups: all mission files (`.rhm`), map files (`.rhp`), and the config file `profile.cpf` under `Data/Configuration`.

Scripting:

- A mission is scripted by placing a `.lua` file with the same name as the `.rhm`. If no such Lua file exists, the game uses the original `.scb` script.
- The editor can generate a default Lua script for the open level via "Script" in the menu bar.
- Included Lua files go in the levels folder: `api.lua` documents the available Lua functions; `common.lua` and `enums.lua` hold helper functions and enum tables shareable across all scripts.
- Recommended editor: Visual Studio Code. Placing the included `.vscode` folder under `Data/Levels` yields a Lua-plugin recommendation and settings that improve the visibility of Lua files in the explorer view.

Save-game caveat:

- When starting a modded mission, do not load any save made inside that mission; it must be loaded from the start.
- For the first mission: delete the Restart and Continue saves of the selected profile.
- For other missions: make a save in Sherwood before starting the mission and always begin the modded level from that save.

Known gaps:

- Some editor input fields are labelled "Unknown", and many script functions and arguments are likewise unidentified; the author asks for help figuring them out.

Blender:

- A Blender add-on is included that imports the geometry of map files (`.rhp`). Exporting geometry back to the original format is not supported.

Compatibility and credits:

- Compatible with game version 1.1; supports the Ready2Play Launcher (linked).
- Thanks to the Robin Hood community; special thanks to Nescafe (promotion) and testers JimboKern, Molsga and Red Officer.
- Distribution: "Get it on ModDB" link to `moddb.com/mods/spellforge-editor-robin-hood-legend-of-sherwood`.

Site context: same navigation as the other rhmods tool pages (Tools: Asset Editor, Developer Console, Profile Tool, Ready2Play Launcher, Rhuce, Spellforge Editor; Media: Gallery, Videos, Music, Save game, Demos; links to Steam app 46560, GOG, ModDB, YouTube @RobinHoodCommunity, Discord; contact mail@rhmods.com).
