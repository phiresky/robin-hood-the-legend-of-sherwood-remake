# Spellforge Editor

- Original source: [Spellforge Editor — RH Mods](https://rhmods.com/tools/spellforge-editor/)
- Author: YetiWizard
- Publication date: Not displayed
- Language: English
- Retrieved: 2026-09-09
- Archived copy: [Wayback Machine, 2026-06-07](https://web.archive.org/web/20260607071836/https://rhmods.com/tools/spellforge-editor/)

## Original text

**Author:**

YetiWizard

**Description:**

The Spellforge Editor is a combination of a mod for Robin Hood: Legend of Sherwood that enables extended modding capabilities, and an accompanying editor program to edit missions.

**File contents**

The `dinput.dll` file extends the game and allows it to load missions saved by the Spellforge editor. It also allows scripting the missions using [Lua](https://www.lua.org/docs.html) (LuaJIT 2.1). The `.dll` file should be placed in the game folder (where the `game.exe` is).

The `Spellforge.exe` should also be placed in the game folder. Upon starting the editor a config file is created in the same folder (`spellforge.ini`). It can be used to modify the game paths and change some editor settings.

The missions and maps are located under “Data/Levels”. It is recommended that you back up all mission files (`.rhm`), map files (`.rhp`), as well as the config file (`profile.cpf`) under “Data/Configuration”.

Missions are scripted by including a `.lua` file with the same name as the `.rhm`. If such a Lua file is not present, the game will use the original script file (`.scb`). You can create a default Lua script for the open level by clicking on “Script” in the menu bar in the editor.

The included `.lua` files are to be placed in the levels folder. `api.lua` documents the available Lua functions while `common.lua` and `enums.lua` have some helpful functions and enum tables that can be shared across all scripts.

For editing the Lua scripts, the “Visual Studio Code” editor is recommended. By placing the “.vscode” folder under “Data/Levels” you will get a recommendation which Lua plugin to use as well as some settings to increase the visibility of Lua files in the explorer view.

When starting a modded mission, it is imperative that you **DO NOT** load any save files that take place in that mission. The mission has to be loaded from the start. For the first mission you can delete the Restart and Continue saves for the selected profile. For the other missions make a save in Sherwood before starting it and always begin the modded level from that save.

You may notice that there are some input fields labelled “Unknown”. The same applies for many script functions. Help us by figuring out what these inputs and script functions/arguments do.

Finally, there is a Blender add-on included that allows importing the geometry of the map files (`.rhp`). Exporting the geometry in the original format is currently not supported.

The mod is compatible with the game version 1.1 and supports using the excellent [Ready2Play Launcher](https://rhmods.com/tools/ready2play-launcher/).

Many thanks to the Robin Hood community for their commitment and support!

Special thanks to **Nescafe** for the promotion and to the hard-working testers **JimboKern**, **Molsga** and **Red Officer**!

Have fun using the editor!

### Screenshots

![Spellforge Editor screenshot 1](https://rhmods.com/content/uploads/Screenshot_1.1.png)

![Spellforge Editor screenshot 2](https://rhmods.com/content/uploads/Screenshot_2.1.png)

![Spellforge Editor screenshot 3](https://rhmods.com/content/uploads/Screenshot_3.1.png)

![Spellforge Editor screenshot 4](https://rhmods.com/content/uploads/Screenshot_4.1.png)

![Spellforge Editor screenshot 5](https://rhmods.com/content/uploads/Screenshot_5.1.png)

[Get it on ModDB](https://www.moddb.com/mods/spellforge-editor-robin-hood-legend-of-sherwood)
