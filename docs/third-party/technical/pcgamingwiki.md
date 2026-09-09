# PCGamingWiki — compatibility and technical reference

- Original source: [PCGamingWiki — compatibility and technical reference](https://www.pcgamingwiki.com/wiki/Robin_Hood%3A_The_Legend_of_Sherwood)
- Author / publication: PCGamingWiki contributors
- Language / date: English; dynamic page checked 2026-09-09
- Access: Full article text retrieved via the wiki parse API
- Checked: 2026-09-09
- Retrieved: 2026-09-09, revision 1763804
- Archived copy: [Wayback Machine, 2025-10-21](http://web.archive.org/web/20251021135736/https://www.pcgamingwiki.com/wiki/Robin_Hood:_The_Legend_of_Sherwood)
- Format: header notes, original summary, then the full licensed article text

The main troubleshooting reference collects patch links, display and input information, configuration locations, and performance workarounds.

It identifies Windows configuration under `DATA/Configuration/` and saves under `DATA/Savegame/` within the installation. Its technical table lists DirectDraw 7, FMOD 3.6 audio, and Bink Video 1.5L cinematics.

The page discusses DirectDraw wrappers, DxWnd, font replacement, and Ready2Play. Its font-performance explanation carries a citation-needed marker. Compatibility workarounds therefore have different levels of support and should not be treated as equally verified.

**Version caution:** Windows, original native Linux/Mac ports, and Proton installations are distinct environments. Historical workarounds may not apply to the currently patched store build.

**Research use:** technical leads and external issue reports. No listed fix or binary was installed or tested for this collection.

## Full text

Reproduced from PCGamingWiki revision 1763804 (2026-03-29T16:52:41Z, page id 21608), retrieved through the wiki's parse API on 2026-09-09. PCGamingWiki content is available under the [Creative Commons Attribution Non-Commercial Share Alike 3.0 License](https://creativecommons.org/licenses/by-nc-sa/3.0/). Icon-only table cells were replaced by their tooltip text (true/false/hackable/etc.) so the tables stay readable.

Robin Hood: The Legend of Sherwood [](https://www.pcgamingwiki.com/wiki/File:Robin_Hood_The_Legend_of_Sherwood_cover.jpg)  
---  
Developers  
| [Company:Spellbound Entertainment](https://www.pcgamingwiki.com/wiki/Company:Spellbound_Entertainment "Company:Spellbound Entertainment")[Spellbound Entertainment](https://www.pcgamingwiki.com/wiki/Company:Spellbound_Entertainment "Company:Spellbound Entertainment")  
macOS (OS X) | [Company:RuneSoft](https://www.pcgamingwiki.com/wiki/Company:RuneSoft "Company:RuneSoft")[RuneSoft](https://www.pcgamingwiki.com/wiki/Company:RuneSoft "Company:RuneSoft")  
Linux | [Company:RuneSoft](https://www.pcgamingwiki.com/wiki/Company:RuneSoft "Company:RuneSoft")[RuneSoft](https://www.pcgamingwiki.com/wiki/Company:RuneSoft "Company:RuneSoft")  
Publishers  
Retail, Europe | [Company:Wanadoo Edition](https://www.pcgamingwiki.com/wiki/Company:Wanadoo_Edition "Company:Wanadoo Edition")[Wanadoo Edition](https://www.pcgamingwiki.com/wiki/Company:Wanadoo_Edition "Company:Wanadoo Edition")  
Retail, North America | [Company:Strategy First](https://www.pcgamingwiki.com/wiki/Company:Strategy_First "Company:Strategy First")[Strategy First](https://www.pcgamingwiki.com/wiki/Company:Strategy_First "Company:Strategy First")  
Retail re-release | [Company:Sold Out Software](https://www.pcgamingwiki.com/wiki/Company:Sold_Out_Software "Company:Sold Out Software")[Sold Out Software](https://www.pcgamingwiki.com/wiki/Company:Sold_Out_Software "Company:Sold Out Software")  
OS X, North America | [Company:Freeverse Software](https://www.pcgamingwiki.com/wiki/Company:Freeverse_Software "Company:Freeverse Software")[Freeverse Software](https://www.pcgamingwiki.com/wiki/Company:Freeverse_Software "Company:Freeverse Software")  
Digital | [Company:Anuman Interactive](https://www.pcgamingwiki.com/wiki/Company:Anuman_Interactive "Company:Anuman Interactive")[Anuman Interactive](https://www.pcgamingwiki.com/wiki/Company:Anuman_Interactive "Company:Anuman Interactive")  
Release dates  
Windows | November 15, 2002  
macOS (OS X) | December 17, 2004  
Linux | January 12, 2005  
Reception  
Metacritic | [80](https://www.metacritic.com/game/robin-hood-the-legend-of-sherwood/critic-reviews/?platform=pc)  
Taxonomy  
Monetization | [One-time game purchase](https://www.pcgamingwiki.com/wiki/Category:One-time_game_purchase "Category:One-time game purchase")  
Modes | [Singleplayer](https://www.pcgamingwiki.com/wiki/Category:Singleplayer "Category:Singleplayer")  
Pacing | [Real-time](https://www.pcgamingwiki.com/wiki/Category:Real-time "Category:Real-time")  
Perspectives | [Bird's-eye view](https://www.pcgamingwiki.com/wiki/Category:Bird%27s-eye_view "Category:Bird's-eye view"), [Isometric](https://www.pcgamingwiki.com/wiki/Category:Isometric "Category:Isometric")  
Controls | [Point and select](https://www.pcgamingwiki.com/wiki/Category:Point_and_select "Category:Point and select"), [Multiple select](https://www.pcgamingwiki.com/wiki/Category:Multiple_select "Category:Multiple select")  
Genres | [Stealth](https://www.pcgamingwiki.com/wiki/Category:Stealth "Category:Stealth"), [Strategy](https://www.pcgamingwiki.com/wiki/Category:Strategy "Category:Strategy")  
Art styles | [Stylized](https://www.pcgamingwiki.com/wiki/Category:Stylized "Category:Stylized")  
Themes | [Medieval](https://www.pcgamingwiki.com/wiki/Category:Medieval "Category:Medieval")  
Series | [Robin Hood](https://www.pcgamingwiki.com/wiki/Series:Robin_Hood "Series:Robin Hood")  
Official siteRobin Hood: The Legend of Sherwood in GOG DatabaseRobin Hood: The Legend of Sherwood on HowLongToBeatRobin Hood: The Legend of Sherwood on IGDBRobin Hood: The Legend of Sherwood on IsThereAnyDealRobin Hood: The Legend of Sherwood on LutrisRobin Hood: The Legend of Sherwood on ProtonDBRobin Hood: The Legend of Sherwood on SteambaseRobin Hood: The Legend of Sherwood on SteamDBRobin Hood: The Legend of Sherwood on MobyGamesRobin Hood: The Legend of Sherwood on WikipediaRobin Hood: The Legend of Sherwood on WineHQ  
[Robin Hood](https://www.pcgamingwiki.com/wiki/Series:Robin_Hood "Series:Robin Hood")  
---  
[Conquests of the Longbow: The Legend of Robin Hood](https://www.pcgamingwiki.com/wiki/Conquests_of_the_Longbow:_The_Legend_of_Robin_Hood "Conquests of the Longbow: The Legend of Robin Hood") | 1991  
[Robin Hood's Games of Skill and Chance](https://www.pcgamingwiki.com/wiki/Robin_Hood%27s_Games_of_Skill_and_Chance "Robin Hood's Games of Skill and Chance") | 1992  
[The Adventures of Robin Hood](https://www.pcgamingwiki.com/w/index.php?title=The_Adventures_of_Robin_Hood&action=edit&redlink=1 "The Adventures of Robin Hood \(page does not exist\)") | 1993  
Robin Hood: The Legend of Sherwood | 2002  
[Robin Hood: Defender of the Crown](https://www.pcgamingwiki.com/wiki/Robin_Hood:_Defender_of_the_Crown "Robin Hood: Defender of the Crown") | 2003  
[Robin Hood's Quest](https://www.pcgamingwiki.com/wiki/Robin_Hood%27s_Quest "Robin Hood's Quest") | 2003  
[Nocked! True Tales of Robin Hood](https://www.pcgamingwiki.com/wiki/Nocked!_True_Tales_of_Robin_Hood "Nocked! True Tales of Robin Hood") | 2019  
[Robin Hood: Country Heroes](https://www.pcgamingwiki.com/wiki/Robin_Hood:_Country_Heroes "Robin Hood: Country Heroes") | 2019  
[Robin Hood Sherwood Builders](https://www.pcgamingwiki.com/wiki/Robin_Hood_Sherwood_Builders "Robin Hood Sherwood Builders") | 2024  
---  
  
**Warnings**

    

Disadvantage

The macOS (OS X) release of this game _does not work_ on macOS Catalina (version 10.15) or later due to the removal of support for 32-bit-only apps.

_**Robin Hood: The Legend of Sherwood**_ is a [singleplayer](https://www.pcgamingwiki.com/wiki/Category:Singleplayer "Category:Singleplayer") [bird's-eye view](https://www.pcgamingwiki.com/wiki/Category:Bird%27s-eye_view "Category:Bird's-eye view") and [isometric](https://www.pcgamingwiki.com/wiki/Category:Isometric "Category:Isometric") [stealth](https://www.pcgamingwiki.com/wiki/Category:Stealth "Category:Stealth") and [strategy](https://www.pcgamingwiki.com/wiki/Category:Strategy "Category:Strategy") game in the [Robin Hood](https://www.pcgamingwiki.com/wiki/Series:Robin_Hood "Series:Robin Hood") series. 

**General information**

    

More information

[Official website](https://web.archive.org/web/20070217140808/http://www.robinhood-game.com:80/) (archived)

    

More information

[GOG.com Community Discussions](https://www.gog.com/forum/robin_hood_legend_of_sherwood)
    

More information

[GOG.com Support Page](https://support.gog.com/hc//categories/201400969?game=1207659008)
    

More information

[Steam Community Discussions](https://steamcommunity.com/app/46560/discussions/)

## Availability

Source | DRM | Notes | Keys | OS  
---|---|---|---|---  
Retail  | Disc check |  Sysiphus [DRM](https://www.pcgamingwiki.com/wiki/Digital_rights_management_\(DRM\) "Digital rights management \(DRM\)") disc check (German release).  |  | [Windows](https://www.pcgamingwiki.com/wiki/Windows "Windows")  
Retail  | DRM-free |  Russian, Polish, US (English) releases  |  | [Windows](https://www.pcgamingwiki.com/wiki/Windows "Windows")  
Retail  | DRM-freeCD-key |  2011 year version needs a key required for startup.  |  | [macOS (OS X)](https://www.pcgamingwiki.com/wiki/OS_X "macOS \(OS X\)")  
[GOG.com](https://af.gog.com/game/robin_hood?as=1649876489) | DRM-free |  |  | [Windows](https://www.pcgamingwiki.com/wiki/Windows "Windows")  
[Steam](https://store.steampowered.com/app/46560/?utm_source=PCGamingWiki&utm_medium=PCGamingWiki&utm_campaign=PCGamingWiki) | [Steam](https://www.pcgamingwiki.com/wiki/Steam "Steam") |  |  | [Windows](https://www.pcgamingwiki.com/wiki/Windows "Windows")  
[ZOOM Platform](https://www.zoom-platform.com/product/robin-hood-the-legend-of-sherwood?affiliate=d13eea34-a694-4c6b-831e-0706cd728e86) | DRM-free |  |  | [Windows](https://www.pcgamingwiki.com/wiki/Windows "Windows")  
[GamersGate](https://www.dpbolvw.net/click-6723194-11554588?url=https://www.gamersgate.com/product/robin-hood-the-legend-of-sherwood?caff=5418682) | [Steam](https://www.pcgamingwiki.com/wiki/Steam "Steam") |  |  | [Windows](https://www.pcgamingwiki.com/wiki/Windows "Windows")  
[Green Man Gaming](https://greenmangaming.sjv.io/c/3659980/1281797/15105?u=https://www.greenmangaming.com/games/robin-hood-the-legend-of-sherwood-pc) | [Steam](https://www.pcgamingwiki.com/wiki/Steam "Steam") |  |  | [Windows](https://www.pcgamingwiki.com/wiki/Windows "Windows")  
[Mac App Store](https://apps.apple.com/app/id830562813) (_unavailable_) | [Mac App Store](https://www.pcgamingwiki.com/wiki/Mac_App_Store "Mac App Store") |  |  | [macOS (OS X)](https://www.pcgamingwiki.com/wiki/OS_X "macOS \(OS X\)")  
[](https://gamesplanet.com/game/43-43?ref=pcgwiki) (_unavailable_) | DRM-free |  [1] [2] |  | [Windows](https://www.pcgamingwiki.com/wiki/Windows "Windows")  
[GamersGate](https://www.dpbolvw.net/click-6723194-11554588?url=https://www.gamersgate.com/product/robin-hood-the-legend-of-sherwood?caff=5418682) (_unavailable_) | DRM-free |  [3][4] |  | [Windows](https://www.pcgamingwiki.com/wiki/Windows "Windows")[macOS (OS X)](https://www.pcgamingwiki.com/wiki/OS_X "macOS \(OS X\)")  
  
### Demo

    

Information

A free [demo](https://web.archive.org/web/20120908011353/http://www.imagineer.co.jp/pc/products/robinhood/download/RH_DEMO_EN.exe) version is available.

## Essential improvements

### Patches

    

Information

Patches are available ([US](https://www.patches-scrolls.de/patch/3466/7/49278/download), [European](https://www.patches-scrolls.de/patch/3465/7/49276/download), [Japan](https://www.patches-scrolls.de/patch/3465/7/49277/download)). Changelog can be found [here](https://web.archive.org/web/20070213052133/http://www.robinhood-game.com:80/web/en/robinhood.php?m0=_download&menu=2&id=1)

### [Ready2Play Launcher (Patch)](https://www.moddb.com/mods/robin-hood-legend-of-sherwood-ready2play-launcher/)

    

Information

Improves compatibility and performance on modern Windows (7-11) systems.
    

Advantage

Portable custom launcher
    

Advantage

Includes OpenGL/Direct3D9 renderer, with filter/shader support, windowed and borderless windowed modes, higher resolutions in graphic options (and UI fix for new higher resolutions)
    

Advantage

Alt+Tab issues fixed
    

Advantage

Can enable and disable intro

## Game data

### Configuration file(s) location

System | Location  
---|---  
Windows  | [<path-to-game>](https://www.pcgamingwiki.com/wiki/Glossary:Game_data#Installation_folder "Glossary:Game data")\DATA\Configuration\[Note 1]  
macOS (OS X)  |   
Linux  |   
Steam Play (Linux) | [<SteamLibrary-folder>](https://www.pcgamingwiki.com/wiki/Glossary:Game_data#Steam_client "Glossary:Game data")/steamapps/compatdata/46560/pfx/[Note 2]  
  
    

Information

It's unknown whether this game follows the [XDG Base Directory Specification](https://specifications.freedesktop.org/basedir/latest/) on Linux. Please fill in this information.

### Save game data location

System | Location  
---|---  
Windows  | [<path-to-game>](https://www.pcgamingwiki.com/wiki/Glossary:Game_data#Installation_folder "Glossary:Game data")\DATA\Savegame\[Note 1]  
macOS (OS X)  |   
Linux  |   
Steam Play (Linux) | [<SteamLibrary-folder>](https://www.pcgamingwiki.com/wiki/Glossary:Game_data#Steam_client "Glossary:Game data")/steamapps/compatdata/46560/pfx/[Note 2]  
  
### [Save game cloud syncing](https://www.pcgamingwiki.com/wiki/Glossary:Save_game_cloud_syncing "Glossary:Save game cloud syncing")

System | Native | Notes  
---|---|---  
[GOG Galaxy](https://www.pcgamingwiki.com/wiki/Store:GOG.com "Store:GOG.com") | Native support |   
[Steam Cloud](https://www.pcgamingwiki.com/wiki/Store:Steam#Steam_Cloud "Store:Steam") | No native support |   
  
## Video

[](https://www.pcgamingwiki.com/wiki/File:Robin_Hood_-_The_Legend_of_Sherwood_-_Graphics.png)

In-game video settings.

Graphics feature | State | Notes  
---|---|---  
[Widescreen resolution](https://www.pcgamingwiki.com/wiki/Glossary:Widescreen_resolution "Glossary:Widescreen resolution") | Hackable |  See Widescreen resolution.  
[Multi-monitor](https://www.pcgamingwiki.com/wiki/Glossary:Multi-monitor "Glossary:Multi-monitor") | No native support |   
[Ultra-widescreen](https://www.pcgamingwiki.com/wiki/Glossary:Ultra-widescreen "Glossary:Ultra-widescreen") | No native support |   
[4K Ultra HD](https://www.pcgamingwiki.com/wiki/Glossary:4K_Ultra_HD "Glossary:4K Ultra HD") | No native support |   
[Field of view (FOV)](https://www.pcgamingwiki.com/wiki/Glossary:Field_of_view_\(FOV\) "Glossary:Field of view \(FOV\)") | Not applicable |   
[Windowed](https://www.pcgamingwiki.com/wiki/Glossary:Windowed "Glossary:Windowed") | Hackable | See Windowed.  
[Borderless fullscreen windowed](https://www.pcgamingwiki.com/wiki/Glossary:Borderless_fullscreen_windowed "Glossary:Borderless fullscreen windowed") | No native support | _See the[glossary page](https://www.pcgamingwiki.com/wiki/Glossary:Borderless_fullscreen_windowed "Glossary:Borderless fullscreen windowed") for potential workarounds._  
[Anisotropic filtering (AF)](https://www.pcgamingwiki.com/wiki/Glossary:Anisotropic_filtering_\(AF\) "Glossary:Anisotropic filtering \(AF\)") | Not applicable |   
[Anti-aliasing (AA)](https://www.pcgamingwiki.com/wiki/Glossary:Anti-aliasing_\(AA\) "Glossary:Anti-aliasing \(AA\)") | Not applicable |   
[High-fidelity upscaling](https://www.pcgamingwiki.com/wiki/Glossary:High-fidelity_upscaling "Glossary:High-fidelity upscaling") | No native support | _See the[glossary page](https://www.pcgamingwiki.com/wiki/Glossary:High-fidelity_upscaling#Force_upscaling_in_unsupported_games "Glossary:High-fidelity upscaling") for potential workarounds._  
[Vertical sync (Vsync)](https://www.pcgamingwiki.com/wiki/Glossary:Vertical_sync_\(Vsync\) "Glossary:Vertical sync \(Vsync\)") | No native support | _See the[glossary page](https://www.pcgamingwiki.com/wiki/Glossary:Vertical_sync_\(Vsync\) "Glossary:Vertical sync \(Vsync\)") for potential workarounds._  
[60 FPS and 120+ FPS](https://www.pcgamingwiki.com/wiki/Glossary:Frame_rate_\(FPS\) "Glossary:Frame rate \(FPS\)") | No native support | Menus and cutscenes are capped at 60 FPS while the gameplay is capped at 20 FPS.  
[High dynamic range display (HDR)](https://www.pcgamingwiki.com/wiki/Glossary:High_dynamic_range_\(HDR\) "Glossary:High dynamic range \(HDR\)") | No native support |   
[Color blind mode](https://www.pcgamingwiki.com/wiki/Glossary:Color_blind_mode "Glossary:Color blind mode") | No native support | _See the[glossary page](https://www.pcgamingwiki.com/wiki/Glossary:Color_blind_mode "Glossary:Color blind mode") for potential alternatives._  
  
### [Widescreen resolution](https://www.pcgamingwiki.com/wiki/Glossary:Widescreen_resolution "Glossary:Widescreen resolution")

FixUse Ready2Play Launcher[_citation needed_]  
---  
FixModify configuration file[5]  
---  
  
  1. Launch the game at least once.
  2. Go to `[<path-to-game>](https://www.pcgamingwiki.com/wiki/Glossary:Game_data#Installation_folder "Glossary:Game data")\DATA\Savegame`
  3. Open the `Profiles` file with [wxMEdit](https://wxmedit.github.io/downloads.html) or other hex editor.
  4. Press `Ctrl`+`F` and fill the **Find Hex String** checkbox*.
  5. Find one of the following strings:

  * 20 44 00 00 F0 43
  * 48 44 00 00 16 44
  * 80 44 00 00 40 44

  6. Replace it with the value corresponding to the desired resolution:

  * 1024x576 - 80 44 00 00 10 44
  * 1280x720 - A0 44 00 00 34 44
  * 1360x768 - AA 44 00 00 40 44
  * 1600x900 - C8 44 00 00 61 44
  * 1920x1080 - F0 44 00 00 87 44
  * For more resolution values, [see here](https://www.gog.com/forum/robin_hood_legend_of_sherwood/widescreen/post19).

* Depends on the hex editor.   
  
### [Windowed](https://www.pcgamingwiki.com/wiki/Glossary:Windowed "Glossary:Windowed")

FixUse Ready2Play Launcher[_citation needed_]  
---  
FixUse DxWnd[_citation needed_]  
---  
  
  1. Download [DxWnd](https://sourceforge.net/projects/dxwnd/) and extract it.
  2. Launch DxWnd as administrator.
  3. Configure it.
  4. Choose **Edit** , and **Add**.
  5. Type in the name for it (e.g. Robin Hood).
  6. Set the path to `[<path-to-game>](https://www.pcgamingwiki.com/wiki/Glossary:Game_data#Installation_folder "Glossary:Game data")\Game.exe`.
  7. Under position specify the **X** , **Y** position of a window for the game and Width (**W**) and Height (**H**) of the window.
  8. Go to **Video** tab.
  9. Under **Window Handling** , check **Modal Style**.
  10. Under **Color management** check **Set 16BPP RGB565 encoding**.
  11. Go to **Input** tab.
  12. Set the **Cursor visibility** to **Hide**.
  13. Click **OK** to save the settings.
  14. Minimize DxWnd and launch the game.

  
  
## Input

[](https://www.pcgamingwiki.com/wiki/File:Robin_Hood_-_The_Legend_of_Sherwood_-_Key_Bindings.png)

In-game input settings.

Keyboard and mouse | State | Notes  
---|---|---  
[Remapping](https://www.pcgamingwiki.com/wiki/Glossary:Remapping "Glossary:Remapping") | Native support |   
[Mouse sensitivity](https://www.pcgamingwiki.com/wiki/Glossary:Mouse#Sensitivity "Glossary:Mouse") | No native support |   
[Mouse acceleration](https://www.pcgamingwiki.com/wiki/Glossary:Mouse_acceleration "Glossary:Mouse acceleration") | No native support |   
[Mouse input in menus](https://www.pcgamingwiki.com/wiki/Glossary:Mouse "Glossary:Mouse") | Native support |   
[Keyboard](https://www.pcgamingwiki.com/wiki/Keyboard "Keyboard") and [mouse](https://www.pcgamingwiki.com/wiki/Glossary:Mouse "Glossary:Mouse") prompts | Unknown |   
[Mouse Y-axis inversion](https://www.pcgamingwiki.com/wiki/Glossary:Invert_Y-axis "Glossary:Invert Y-axis") | No native support |   
Controller |  |   
[Controller support](https://www.pcgamingwiki.com/wiki/Glossary:Controller "Glossary:Controller") | No native support |   
  
## Audio

[](https://www.pcgamingwiki.com/wiki/File:Robin_Hood_-_The_Legend_of_Sherwood_-_Audio.png)

In-game audio settings.

Audio feature | State | Notes  
---|---|---  
Separate volume controls | Native support | FX, Dialogue, Music and Comments.  
[Surround sound](https://www.pcgamingwiki.com/wiki/Glossary:Surround_sound "Glossary:Surround sound") | Native support |   
Subtitles | No native support |   
Closed captions | No native support |   
Mute on focus lost | Always on (no native option) |   
Royalty free audio | Unknown |   
  
### Localizations

Language | UI | Audio | Sub | Notes  
---|---|---|---|---  
English | Native support | Native support | No native support |   
Italian | Native support | Native support | No native support | Retail version only.  
Czech | Native support | Native support | No native support | Retail - [📥](https://steamcommunity.com/sharedfiles/filedetails/?id=1349014146)  
French | Native support | Native support | No native support |   
German | Native support | Native support | No native support |   
Polish | Native support | Native support | No native support | GOG.com version and local retail release.  
Official translation, [download](https://community.pcgamingwiki.com/files/file/1184-robin-hood-the-legend-of-sherwood-polish-translation/)  
Brazilian Portuguese | Native support | No native support | Native support | Retail only— _Robin Hood: A Lenda de Sherwood_.[6] Download for the digital version of the translation is available [here](https://www.centraldetraducoes.net.br/2006/10/traducao-do-robin-hood-legend-of-sherwood-pc.html).  
Brazilian Portuguese | Hackable | No native support | No native support | Fan translation: [download (GameVício)](https://www.gamevicio.com/traducao/traducao-de-robin-hood-the-legend-of-sherwood-para-portugues-brasil/)  
Spanish | Native support | Native support | No native support |   
Russian | Native support | Native support | No native support | Was released under title **Робин Гуд: Легенда Шервуда**  
  
## Issues fixed

### Uneven/poor performance

FixInstall a wrapper[7]  
---  
  
  1. Download a [modified DDraw wrapper](https://community.pcgamingwiki.com/files/file/785-robin-hood-the-legend-of-sherwood-ddraw-wrapper/) and extract it.
  2. Copy **aqrit.cfg** and **ddraw.dll** to a folder where you have installed the game.
  3. If you're using **Windows 8 / 8.1 / 10** start the game with DxWnd (can be set to fullscreen). See Windowed.

  
FixReplace English fonts[_citation needed_]  
---  
  
  1. Download the [GOG German Fonts](https://files.gog.com/support/Fonts.zip) and extract it.
  2. Copy the files into the Data\Interface\Fonts folder contained where the game is installed.
  3. There is a bug inside the English font set which may hamper performance.
  4. Alternatively, you can use this fix with font files from any non-english localization.

  
FixEasy and Correct Wrapper and DxWnd Installer[8]  
---  
  
  1. Download the [Performance Fix v.1.0](https://www.gamepressure.com/download.asp?ID=59061) and extract it.
  2. Place the **Robin Hood - Performance Fix.exe** to the folder where you have installed the game.
  3. The Installer will install the correct working versions of the Ddraw Wrapper and DxWnd.
  4. Run the game via the **Robin Hood - Performance Fix.exe** shortcut on your desktop. (If missing, create a new shortcut)
  5. For more information read the 'Description' and 'How to Install' on the download-page.

  
  
## Other information

### API

Technical specs | Supported | Notes  
---|---|---  
DirectDraw | 7 |   
  
Executable| PPC | 32-bit | 64-bit | Notes  
---|---|---|---|---  
Windows| Not applicable | Native support | No native support | _32-bit executables may need the[LAA flag applied](https://www.pcgamingwiki.com/wiki/Windows#Set_older_32-bit_games_to_use_4_GB_RAM_instead_of_2 "Windows") to work properly on modern machines._  
macOS (OS X)| Native support | Native support | No native support | Two versions exist for OS X: the original 2004 PowerPC port, and the 2011 digital release on the App Store.  
  
The digital release is unknown if it made it to 64-bit, but did have a Lion patch the same year it released.  
Linux| Native support | Native support | Unknown |   
  
### Middleware

| Middleware | Notes  
---|---|---  
Audio | [FMOD](https://www.pcgamingwiki.com/wiki/FMOD "FMOD") | 3.6  
Cutscenes | [Bink Video](https://www.pcgamingwiki.com/wiki/Bink_Video "Bink Video") | 1.5L  
  
## System requirements

**Windows**| **macOS (OS X)**| **Linux**  
---|---|---  
  
[Windows](https://www.pcgamingwiki.com/wiki/Windows "Windows")  
---  
| Minimum[9]  
Operating system (OS) | 98, ME, 2000, XP  
Processor (CPU) | Intel Pentium II 233 MHz  
System memory (RAM) | 64 MB  
Storage drive (HDD/SSD) | 1 GB  
Video card (GPU) |  4 MB of VRAM  
DirectX 8.1 compatible  
  
[macOS (OS X)](https://www.pcgamingwiki.com/wiki/Mac_OS "Mac OS")  
---  
| Minimum  
Operating system (OS) | 10.6.6  
Processor (CPU) | Intel 1.8 GHz  
System memory (RAM) | 512 MB  
Storage drive (HDD/SSD) | 2 GB  
Video card (GPU) |  128 MB of VRAM  
  
[Linux](https://www.pcgamingwiki.com/wiki/Linux "Linux")  
---  
| Minimum  
Operating system (OS) |   
Processor (CPU) | 500 MHz x86  
500 MHz PowerPC  
System memory (RAM) | 128 MB  
Storage drive (HDD/SSD) | 1 GB  
Video card (GPU) |  8 MB of VRAM  
  
  

## Notes

  1. ↑ 1.0 1.1 When running this game without elevated privileges (**Run as administrator** option), write operations against a location below `[%PROGRAMFILES%](https://www.pcgamingwiki.com/wiki/Glossary:Game_data#Shared_applications "Glossary:Game data")`, `[%PROGRAMDATA%](https://www.pcgamingwiki.com/wiki/Glossary:Game_data#Shared_application_data "Glossary:Game data")`, or `[%WINDIR%](https://www.pcgamingwiki.com/wiki/Glossary:Game_data#Windows "Glossary:Game data")` might be redirected to `[%LOCALAPPDATA%](https://www.pcgamingwiki.com/wiki/Glossary:Game_data#User_application_data "Glossary:Game data")\VirtualStore` on Windows Vista and later ([more details](https://www.pcgamingwiki.com/wiki/Game_data#Installation_folder "Game data")).
  2. ↑ 2.0 2.1 **Notes regarding Steam Play (Linux) data:**
     * File/folder structure within this directory reflects the path(s) listed for Windows and/or Steam game data.
     * Use [Wine's registry editor](https://wiki.winehq.org/Regedit) to access any Windows registry paths.
     * The app ID (46560) may differ in some cases.
     * Treat backslashes as forward slashes.
     * See the [glossary page](https://www.pcgamingwiki.com/wiki/Glossary:Game_data#Windows_data_paths "Glossary:Game data") for details on Windows data paths.

## References

  1. ↑ [Gamesplanet](https://web.archive.org/web/20120706184532/http://uk.gamesplanet.com/buy-download-pc-games/Robin-Hood---Sherwood-43-43.html) \- last accessed on 2025-07-11
  2. ↑ [Gamesplanet](https://web.archive.org/web/20100717093533/http://uk.gamesplanet.com:80/buy-download-pc-games/Robin-Hood-Legend-of-Sherwood-43-17.html) \- last accessed on 2025-08-30
  3. ↑ [GamersGate](https://web.archive.org/web/20100204225025/http://www.gamersgate.com/DD-HOOD/robin-hood-the-legend-of-sherwood) \- last accessed on 2025-07-30
  4. ↑ [GamersGate](https://web.archive.org/web/20130204104234/http://www.gamersgate.com/DD-RHMAC/robin-hood-mac) \- last accessed on 2025-08-30
  5. ↑ [GOG.com - Forum - widescreen, page 1](https://www.gog.com/forum/robin_hood_legend_of_sherwood/widescreen) \- last accessed on May 2023
  6. ↑ [Big Box PC Games Brasil Wiki](https://big-box-pc-games-brasil.fandom.com/pt-br/wiki/Robin_Hood:_A_Lenda_de_Sherwood) \- last accessed on 2023-09-16
  7. ↑ Verified by [User:Suicide_machine](https://www.pcgamingwiki.com/wiki/User:Suicide_machine "User:Suicide machine") on 2016-10-21
  8. ↑ Verified by [User:Misorian](https://www.pcgamingwiki.com/w/index.php?title=User:Misorian&action=edit&redlink=1 "User:Misorian \(page does not exist\)") on 2021
  9. ↑ [mobygames.com - Unknown page title (retrieval failure)](https://www.mobygames.com/game/7907/robin-hood-the-legend-of-sherwood/cover/group-17564/cover-42063/) \- last accessed on 2025-09-19

## Converted text from the original HTML



### technical__pcgamingwiki.html

_Source: `originals/technical__pcgamingwiki.html`._

[ ](/wiki/Home)

  * Explore
    * [Lists](/wiki/List_of_lists)
    * [Games](/wiki/Category:Games)
    * [Categories](/wiki/Category:Top)
    * [Random page](/wiki/Special:RandomInCategory/Games "Load a random page \[x\]")
    * [Recent changes](/wiki/Special:RecentChanges "A list of recent changes in the wiki \[r\]")
    * [Troubleshooting guide](/wiki/Troubleshooting_guide)
  * Editing 
    * [Editing guide](/wiki/PCGamingWiki:Editing_guide)
    * [Sample article](/wiki/PCGamingWiki:Sample_article)
    * [Projects](/wiki/Category:Projects)
    * [Taxonomy](/wiki/Taxonomy)
    * [Wiki policy](/wiki/PCGamingWiki:Editing_guide/Wiki_policy)
    * [Maintenance](/wiki/PCGamingWiki:Maintenance)
    * [Changelog](/wiki/PCGamingWiki:Changelog)
  * Community
    * [Assignments](/wiki/PCGamingWiki:Assignments)
    * [Discord](/wiki/PCGamingWiki:Discord)
    * [Files](https://community.pcgamingwiki.com/files)
    * [Files policy](/wiki/PCGamingWiki:Editing_guide/Files)
    * [Forums](https://community.pcgamingwiki.com/)
    * [PCGW Account](/wiki/PCGamingWiki:Account)
    * [Other communities](/wiki/PC_gaming_online_communities)
  * About
    * [About](/wiki/PCGamingWiki:About)
    * [Conduct](/wiki/PCGamingWiki:Code_of_conduct)
    * [FAQ](/wiki/PCGamingWiki:FAQ)
    * [Staff](/wiki/PCGamingWiki:Staff)
    * [Donate](/wiki/PCGamingWiki:Donate)
  * Tools
    * [What links here](/wiki/Special:WhatLinksHere/Robin_Hood:_The_Legend_of_Sherwood "A list of all wiki pages that link here \[j\]")
    * [Related changes](/wiki/Special:RecentChangesLinked/Robin_Hood:_The_Legend_of_Sherwood "Recent changes in pages linked from this page \[k\]")
    * [Special pages](/wiki/Special:SpecialPages "A list of all special pages \[q\]")
    * [Printable version](javascript:print\(\); "Printable version of this page \[p\]")
    * [Permanent link](/w/index.php?title=Robin_Hood:_The_Legend_of_Sherwood&oldid=1763804 "Permanent link to this revision of this page")
    * [Page information](/w/index.php?title=Robin_Hood:_The_Legend_of_Sherwood&action=info "More information about this page")
    * [Cargo data](/w/index.php?title=Robin_Hood:_The_Legend_of_Sherwood&action=pagevalues)


  * [Log in](/w/index.php?title=Special:UserLogin&returnto=Robin+Hood%3A+The+Legend+of+Sherwood "You are encouraged to log in; however, it is not mandatory \[o\]")



  * [Log in](/w/index.php?title=Special:UserLogin&returnto=Robin+Hood%3A+The+Legend+of+Sherwood "You are encouraged to log in; however, it is not mandatory \[o\]")



PCGamingWiki has migrated to new servers, and the browsing experience should be improved overall! If you notice or experience any issues, please let us know on [Discord.](https://discord.gg/KDfrTZ8)

* * *

Anonymous edits have been disabled on the wiki. If you want to contribute please [login](https://auth.pcgamingwiki.com/oauth2/start?rd=https://www.pcgamingwiki.com/wiki/Home) or [create](https://sso.pcgamingwiki.com/auth/realms/PCGamingWiki/protocol/openid-connect/registrations?response_type=code&client_id=mediawiki&redirect_uri=https://www.pcgamingwiki.com/wiki/PCGamingWiki:Account/confirmed) an account. 

  * [Page](/wiki/Robin_Hood:_The_Legend_of_Sherwood "View the content page \[c\]")
  * [Discussion](/wiki/Talk:Robin_Hood:_The_Legend_of_Sherwood "Discussion about the content page \[t\]")


  * [Read](/wiki/Robin_Hood:_The_Legend_of_Sherwood)
  * [View source](/w/index.php?title=Robin_Hood:_The_Legend_of_Sherwood&action=edit "This page is protected.
You can view its source \[e\]")
  * [View history](/w/index.php?title=Robin_Hood:_The_Legend_of_Sherwood&action=history "Past revisions of this page \[h\]")



# Robin Hood: The Legend of Sherwood

From PCGamingWiki, the wiki about fixing PC games 

Robin Hood: The Legend of Sherwood [](/wiki/File:Robin_Hood_The_Legend_of_Sherwood_cover.jpg)  
---  
Developers  
| [](/wiki/Company:Spellbound_Entertainment "Company:Spellbound Entertainment")[Spellbound Entertainment](/wiki/Company:Spellbound_Entertainment "Company:Spellbound Entertainment")  
macOS (OS X) | [](/wiki/Company:RuneSoft "Company:RuneSoft")[RuneSoft](/wiki/Company:RuneSoft "Company:RuneSoft")  
Linux | [](/wiki/Company:RuneSoft "Company:RuneSoft")[RuneSoft](/wiki/Company:RuneSoft "Company:RuneSoft")  
Publishers  
Retail, Europe | [](/wiki/Company:Wanadoo_Edition "Company:Wanadoo Edition")[Wanadoo Edition](/wiki/Company:Wanadoo_Edition "Company:Wanadoo Edition")  
Retail, North America | [](/wiki/Company:Strategy_First "Company:Strategy First")[Strategy First](/wiki/Company:Strategy_First "Company:Strategy First")  
Retail re-release | [](/wiki/Company:Sold_Out_Software "Company:Sold Out Software")[Sold Out Software](/wiki/Company:Sold_Out_Software "Company:Sold Out Software")  
OS X, North America | [](/wiki/Company:Freeverse_Software "Company:Freeverse Software")[Freeverse Software](/wiki/Company:Freeverse_Software "Company:Freeverse Software")  
Digital | [](/wiki/Company:Anuman_Interactive "Company:Anuman Interactive")[Anuman Interactive](/wiki/Company:Anuman_Interactive "Company:Anuman Interactive")  
Release dates  
Windows | November 15, 2002  
macOS (OS X) | December 17, 2004  
Linux | January 12, 2005  
Reception  
Metacritic | [80](https://www.metacritic.com/game/robin-hood-the-legend-of-sherwood/critic-reviews/?platform=pc)  
Taxonomy  
Monetization | [One-time game purchase](/wiki/Category:One-time_game_purchase "Category:One-time game purchase")  
Modes | [Singleplayer](/wiki/Category:Singleplayer "Category:Singleplayer")  
Pacing | [Real-time](/wiki/Category:Real-time "Category:Real-time")  
Perspectives | [Bird's-eye view](/wiki/Category:Bird%27s-eye_view "Category:Bird's-eye view"), [Isometric](/wiki/Category:Isometric "Category:Isometric")  
Controls | [Point and select](/wiki/Category:Point_and_select "Category:Point and select"), [Multiple select](/wiki/Category:Multiple_select "Category:Multiple select")  
Genres | [Stealth](/wiki/Category:Stealth "Category:Stealth"), [Strategy](/wiki/Category:Strategy "Category:Strategy")  
Art styles | [Stylized](/wiki/Category:Stylized "Category:Stylized")  
Themes | [Medieval](/wiki/Category:Medieval "Category:Medieval")  
Series | [Robin Hood](/wiki/Series:Robin_Hood "Series:Robin Hood")  
[](https://web.archive.org/web/20071031084425/http://www.microids.com/en/catalogue/25/robin-hood-the-legend-of-sherwood.html)[](https://www.gogdb.org/product/1207659008)[](https://howlongtobeat.com/game?id=7877 "Robin Hood: The Legend of Sherwood on HowLongToBeat")[](https://www.igdb.com/games/robin-hood-the-legend-of-sherwood "Robin Hood: The Legend of Sherwood on IGDB")[](https://isthereanydeal.com/steam/app/46560/)[](https://lutris.net/games/robin-hood-the-legend-of-sherwood)[](https://www.protondb.com/app/46560/)[](https://steambase.io/apps/46560/)[](https://steamdb.info/app/46560/)[](https://www.mobygames.com/game/7907 "Robin Hood: The Legend of Sherwood on MobyGames")[](http://en.wikipedia.org/wiki/Robin_Hood:_The_Legend_of_Sherwood "Robin Hood: The Legend of Sherwood on Wikipedia")[](https://appdb.winehq.org/objectManager.php?sClass=application&iId=2585)  
[Robin Hood](/wiki/Series:Robin_Hood "Series:Robin Hood")  
---  
[Conquests of the Longbow: The Legend of Robin Hood](/wiki/Conquests_of_the_Longbow:_The_Legend_of_Robin_Hood "Conquests of the Longbow: The Legend of Robin Hood") | 1991  
[Robin Hood's Games of Skill and Chance](/wiki/Robin_Hood%27s_Games_of_Skill_and_Chance "Robin Hood's Games of Skill and Chance") | 1992  
[The Adventures of Robin Hood](/w/index.php?title=The_Adventures_of_Robin_Hood&action=edit&redlink=1 "The Adventures of Robin Hood \(page does not exist\)") | 1993  
Robin Hood: The Legend of Sherwood | 2002  
[Robin Hood: Defender of the Crown](/wiki/Robin_Hood:_Defender_of_the_Crown "Robin Hood: Defender of the Crown") | 2003  
[Robin Hood's Quest](/wiki/Robin_Hood%27s_Quest "Robin Hood's Quest") | 2003  
[Nocked! True Tales of Robin Hood](/wiki/Nocked!_True_Tales_of_Robin_Hood "Nocked! True Tales of Robin Hood") | 2019  
[Robin Hood: Country Heroes](/wiki/Robin_Hood:_Country_Heroes "Robin Hood: Country Heroes") | 2019  
[Robin Hood Sherwood Builders](/wiki/Robin_Hood_Sherwood_Builders "Robin Hood Sherwood Builders") | 2024  
  
## Contents

  * 1 Availability
    * 1.1 Demo
  * 2 Essential improvements
    * 2.1 Patches
    * 2.2 Ready2Play Launcher (Patch)
  * 3 Game data
    * 3.1 Configuration file(s) location
    * 3.2 Save game data location
    * 3.3 Save game cloud syncing
  * 4 Video
    * 4.1 Widescreen resolution
    * 4.2 Windowed
  * 5 Input
  * 6 Audio
    * 6.1 Localizations
  * 7 Issues fixed
    * 7.1 Uneven/poor performance
  * 8 Other information
    * 8.1 API
    * 8.2 Middleware
  * 9 System requirements
  * 10 Notes
  * 11 References

  
---  
  
**Warnings**

    

The macOS (OS X) release of this game _does not work_ on macOS Catalina (version 10.15) or later due to the removal of support for 32-bit-only apps.

_**Robin Hood: The Legend of Sherwood**_ is a [singleplayer](/wiki/Category:Singleplayer "Category:Singleplayer") [bird's-eye view](/wiki/Category:Bird%27s-eye_view "Category:Bird's-eye view") and [isometric](/wiki/Category:Isometric "Category:Isometric") [stealth](/wiki/Category:Stealth "Category:Stealth") and [strategy](/wiki/Category:Strategy "Category:Strategy") game in the [Robin Hood](/wiki/Series:Robin_Hood "Series:Robin Hood") series. 

**General information**

    

[Official website](https://web.archive.org/web/20070217140808/http://www.robinhood-game.com:80/) (archived)

    

[GOG.com Community Discussions](https://www.gog.com/forum/robin_hood_legend_of_sherwood)
    

[GOG.com Support Page](https://support.gog.com/hc//categories/201400969?game=1207659008)
    

[Steam Community Discussions](https://steamcommunity.com/app/46560/discussions/)

## Availability

Source | DRM | Notes | Keys | OS  
---|---|---|---|---  
Retail  | [](/wiki/Glossary:Disc_check "Disc check \(requires the CD/DVD in the drive to play\)") |  Sysiphus [DRM](/wiki/Digital_rights_management_\(DRM\) "Digital rights management \(DRM\)") disc check (German release).  |  | [](/wiki/Windows "Windows")  
Retail  | [](/wiki/Glossary:DRM-free "DRM-free") |  Russian, Polish, US (English) releases  |  | [](/wiki/Windows "Windows")  
Retail  | [](/wiki/Glossary:DRM-free "DRM-free")[](/wiki/Glossary:CD-key "CD key") |  2011 year version needs a key required for startup.  |  | [](/wiki/OS_X "macOS \(OS X\)")  
[GOG.com](https://af.gog.com/game/robin_hood?as=1649876489) | [](/wiki/Glossary:DRM-free "DRM-free") |  |  | [](/wiki/Windows "Windows")  
[Steam](https://store.steampowered.com/app/46560/?utm_source=PCGamingWiki&utm_medium=PCGamingWiki&utm_campaign=PCGamingWiki) | [](/wiki/Steam "Steam") |  |  | [](/wiki/Windows "Windows")  
[ZOOM Platform](https://www.zoom-platform.com/product/robin-hood-the-legend-of-sherwood?affiliate=d13eea34-a694-4c6b-831e-0706cd728e86) | [](/wiki/Glossary:DRM-free "DRM-free") |  |  | [](/wiki/Windows "Windows")  
[GamersGate](https://www.dpbolvw.net/click-6723194-11554588?url=https://www.gamersgate.com/product/robin-hood-the-legend-of-sherwood?caff=5418682) | [](/wiki/Steam "Steam") |  |  | [](/wiki/Windows "Windows")  
[Green Man Gaming](https://greenmangaming.sjv.io/c/3659980/1281797/15105?u=https://www.greenmangaming.com/games/robin-hood-the-legend-of-sherwood-pc) | [](/wiki/Steam "Steam") |  |  | [](/wiki/Windows "Windows")  
[Mac App Store](https://apps.apple.com/app/id830562813) (_unavailable_) | [](/wiki/Mac_App_Store "Mac App Store") |  |  | [](/wiki/OS_X "macOS \(OS X\)")  
[](https://gamesplanet.com/game/43-43?ref=pcgwiki) (_unavailable_) | [](/wiki/Glossary:DRM-free "DRM-free") |  [1] [2] |  | [](/wiki/Windows "Windows")  
[GamersGate](https://www.dpbolvw.net/click-6723194-11554588?url=https://www.gamersgate.com/product/robin-hood-the-legend-of-sherwood?caff=5418682) (_unavailable_) | [](/wiki/Store:GamersGate "DRM-free after installation \(requires an internet connection during installation\)") |  [3][4] |  | [](/wiki/Windows "Windows")[](/wiki/OS_X "macOS \(OS X\)")  
  
### Demo

    

A free [demo](https://web.archive.org/web/20120908011353/http://www.imagineer.co.jp/pc/products/robinhood/download/RH_DEMO_EN.exe) version is available.

## Essential improvements

### Patches

    

Patches are available ([US](https://www.patches-scrolls.de/patch/3466/7/49278/download), [European](https://www.patches-scrolls.de/patch/3465/7/49276/download), [Japan](https://www.patches-scrolls.de/patch/3465/7/49277/download)). Changelog can be found [here](https://web.archive.org/web/20070213052133/http://www.robinhood-game.com:80/web/en/robinhood.php?m0=_download&menu=2&id=1)

### [Ready2Play Launcher (Patch)](https://www.moddb.com/mods/robin-hood-legend-of-sherwood-ready2play-launcher/)

    

Improves compatibility and performance on modern Windows (7-11) systems.
    

Portable custom launcher
    

Includes OpenGL/Direct3D9 renderer, with filter/shader support, windowed and borderless windowed modes, higher resolutions in graphic options (and UI fix for new higher resolutions)
    

Alt+Tab issues fixed
    

Can enable and disable intro

## Game data

### Configuration file(s) location

System | Location  
---|---  
Windows  | [<path-to-game>](/wiki/Glossary:Game_data#Installation_folder "Glossary:Game data")\DATA\Configuration\[Note 1]  
macOS (OS X)  |   
Linux  |   
Steam Play (Linux) | [<SteamLibrary-folder>](/wiki/Glossary:Game_data#Steam_client "Glossary:Game data")/steamapps/compatdata/46560/pfx/[Note 2]  
  
    

It's unknown whether this game follows the [XDG Base Directory Specification](https://specifications.freedesktop.org/basedir/latest/) on Linux. Please fill in this information.

### Save game data location

System | Location  
---|---  
Windows  | [<path-to-game>](/wiki/Glossary:Game_data#Installation_folder "Glossary:Game data")\DATA\Savegame\[Note 1]  
macOS (OS X)  |   
Linux  |   
Steam Play (Linux) | [<SteamLibrary-folder>](/wiki/Glossary:Game_data#Steam_client "Glossary:Game data")/steamapps/compatdata/46560/pfx/[Note 2]  
  
### [Save game cloud syncing](/wiki/Glossary:Save_game_cloud_syncing "Glossary:Save game cloud syncing")

System | Native | Notes  
---|---|---  
[GOG Galaxy](/wiki/Store:GOG.com "Store:GOG.com") |  |   
[Steam Cloud](/wiki/Store:Steam#Steam_Cloud "Store:Steam") |  |   
  
## Video

[](/wiki/File:Robin_Hood_-_The_Legend_of_Sherwood_-_Graphics.png)

In-game video settings.

Graphics feature | State | Notes  
---|---|---  
[Widescreen resolution](/wiki/Glossary:Widescreen_resolution "Glossary:Widescreen resolution") |  |  See Widescreen resolution.  
[Multi-monitor](/wiki/Glossary:Multi-monitor "Glossary:Multi-monitor") |  |   
[Ultra-widescreen](/wiki/Glossary:Ultra-widescreen "Glossary:Ultra-widescreen") |  |   
[4K Ultra HD](/wiki/Glossary:4K_Ultra_HD "Glossary:4K Ultra HD") |  |   
[Field of view (FOV)](/wiki/Glossary:Field_of_view_\(FOV\) "Glossary:Field of view \(FOV\)") |  |   
[Windowed](/wiki/Glossary:Windowed "Glossary:Windowed") |  | See Windowed.  
[Borderless fullscreen windowed](/wiki/Glossary:Borderless_fullscreen_windowed "Glossary:Borderless fullscreen windowed") |  | _See the[glossary page](/wiki/Glossary:Borderless_fullscreen_windowed "Glossary:Borderless fullscreen windowed") for potential workarounds._  
[Anisotropic filtering (AF)](/wiki/Glossary:Anisotropic_filtering_\(AF\) "Glossary:Anisotropic filtering \(AF\)") |  |   
[Anti-aliasing (AA)](/wiki/Glossary:Anti-aliasing_\(AA\) "Glossary:Anti-aliasing \(AA\)") |  |   
[High-fidelity upscaling](/wiki/Glossary:High-fidelity_upscaling "Glossary:High-fidelity upscaling") |  | _See the[glossary page](/wiki/Glossary:High-fidelity_upscaling#Force_upscaling_in_unsupported_games "Glossary:High-fidelity upscaling") for potential workarounds._  
[Vertical sync (Vsync)](/wiki/Glossary:Vertical_sync_\(Vsync\) "Glossary:Vertical sync \(Vsync\)") |  | _See the[glossary page](/wiki/Glossary:Vertical_sync_\(Vsync\) "Glossary:Vertical sync \(Vsync\)") for potential workarounds._  
[60 FPS and 120+ FPS](/wiki/Glossary:Frame_rate_\(FPS\) "Glossary:Frame rate \(FPS\)") |  | Menus and cutscenes are capped at 60 FPS while the gameplay is capped at 20 FPS.  
[High dynamic range display (HDR)](/wiki/Glossary:High_dynamic_range_\(HDR\) "Glossary:High dynamic range \(HDR\)") |  |   
[Color blind mode](/wiki/Glossary:Color_blind_mode "Glossary:Color blind mode") |  | _See the[glossary page](/wiki/Glossary:Color_blind_mode "Glossary:Color blind mode") for potential alternatives._  
  
### [Widescreen resolution](/wiki/Glossary:Widescreen_resolution "Glossary:Widescreen resolution")

Use Ready2Play Launcher[_citation needed_]  
---  
Modify configuration file[5]  
---  
  
  1. Launch the game at least once.
  2. Go to `[<path-to-game>](/wiki/Glossary:Game_data#Installation_folder "Glossary:Game data")\DATA\Savegame`
  3. Open the `Profiles` file with [wxMEdit](https://wxmedit.github.io/downloads.html) or other hex editor.
  4. Press `Ctrl`+`F` and fill the **Find Hex String** checkbox*.
  5. Find one of the following strings:


  * 20 44 00 00 F0 43
  * 48 44 00 00 16 44
  * 80 44 00 00 40 44


  6. Replace it with the value corresponding to the desired resolution:


  * 1024x576 - 80 44 00 00 10 44
  * 1280x720 - A0 44 00 00 34 44
  * 1360x768 - AA 44 00 00 40 44
  * 1600x900 - C8 44 00 00 61 44
  * 1920x1080 - F0 44 00 00 87 44
  * For more resolution values, [see here](https://www.gog.com/forum/robin_hood_legend_of_sherwood/widescreen/post19).

* Depends on the hex editor.   
  
### [Windowed](/wiki/Glossary:Windowed "Glossary:Windowed")

Use Ready2Play Launcher[_citation needed_]  
---  
Use DxWnd[_citation needed_]  
---  
  
  1. Download [DxWnd](https://sourceforge.net/projects/dxwnd/) and extract it.
  2. Launch DxWnd as administrator.
  3. Configure it.
  4. Choose **Edit** , and **Add**.
  5. Type in the name for it (e.g. Robin Hood).
  6. Set the path to `[<path-to-game>](/wiki/Glossary:Game_data#Installation_folder "Glossary:Game data")\Game.exe`.
  7. Under position specify the **X** , **Y** position of a window for the game and Width (**W**) and Height (**H**) of the window.
  8. Go to **Video** tab.
  9. Under **Window Handling** , check **Modal Style**.
  10. Under **Color management** check **Set 16BPP RGB565 encoding**.
  11. Go to **Input** tab.
  12. Set the **Cursor visibility** to **Hide**.
  13. Click **OK** to save the settings.
  14. Minimize DxWnd and launch the game.

  
  
## Input

[](/wiki/File:Robin_Hood_-_The_Legend_of_Sherwood_-_Key_Bindings.png)

In-game input settings.

Keyboard and mouse | State | Notes  
---|---|---  
[Remapping](/wiki/Glossary:Remapping "Glossary:Remapping") |  |   
[Mouse sensitivity](/wiki/Glossary:Mouse#Sensitivity "Glossary:Mouse") |  |   
[Mouse acceleration](/wiki/Glossary:Mouse_acceleration "Glossary:Mouse acceleration") |  |   
[Mouse input in menus](/wiki/Glossary:Mouse "Glossary:Mouse") |  |   
[Keyboard](/wiki/Keyboard "Keyboard") and [mouse](/wiki/Glossary:Mouse "Glossary:Mouse") prompts |  |   
[Mouse Y-axis inversion](/wiki/Glossary:Invert_Y-axis "Glossary:Invert Y-axis") |  |   
Controller |  |   
[Controller support](/wiki/Glossary:Controller "Glossary:Controller") |  |   
  
## Audio

[](/wiki/File:Robin_Hood_-_The_Legend_of_Sherwood_-_Audio.png)

In-game audio settings.

Audio feature | State | Notes  
---|---|---  
Separate volume controls |  | FX, Dialogue, Music and Comments.  
[Surround sound](/wiki/Glossary:Surround_sound "Glossary:Surround sound") |  |   
Subtitles |  |   
Closed captions |  |   
Mute on focus lost |  |   
Royalty free audio |  |   
  
### Localizations

Language | UI | Audio | Sub | Notes  
---|---|---|---|---  
English |  |  |  |   
Italian |  |  |  | Retail version only.  
Czech |  |  |  | Retail - [📥](https://steamcommunity.com/sharedfiles/filedetails/?id=1349014146)  
French |  |  |  |   
German |  |  |  |   
Polish |  |  |  | GOG.com version and local retail release.  
Official translation, [download](https://community.pcgamingwiki.com/files/file/1184-robin-hood-the-legend-of-sherwood-polish-translation/)  
Brazilian Portuguese |  |  |  | Retail only— _Robin Hood: A Lenda de Sherwood_.[6] Download for the digital version of the translation is available [here](https://www.centraldetraducoes.net.br/2006/10/traducao-do-robin-hood-legend-of-sherwood-pc.html).  
Brazilian Portuguese |  |  |  | Fan translation: [download (GameVício)](https://www.gamevicio.com/traducao/traducao-de-robin-hood-the-legend-of-sherwood-para-portugues-brasil/)  
Spanish |  |  |  |   
Russian |  |  |  | Was released under title **Робин Гуд: Легенда Шервуда**  
  
## Issues fixed

### Uneven/poor performance

Install a wrapper[7]  
---  
  
  1. Download a [modified DDraw wrapper](https://community.pcgamingwiki.com/files/file/785-robin-hood-the-legend-of-sherwood-ddraw-wrapper/) and extract it.
  2. Copy **aqrit.cfg** and **ddraw.dll** to a folder where you have installed the game.
  3. If you're using **Windows 8 / 8.1 / 10** start the game with DxWnd (can be set to fullscreen). See Windowed.

  
Replace English fonts[_citation needed_]  
---  
  
  1. Download the [GOG German Fonts](https://files.gog.com/support/Fonts.zip) and extract it.
  2. Copy the files into the Data\Interface\Fonts folder contained where the game is installed.
  3. There is a bug inside the English font set which may hamper performance.
  4. Alternatively, you can use this fix with font files from any non-english localization.

  
Easy and Correct Wrapper and DxWnd Installer[8]  
---  
  
  1. Download the [Performance Fix v.1.0](https://www.gamepressure.com/download.asp?ID=59061) and extract it.
  2. Place the **Robin Hood - Performance Fix.exe** to the folder where you have installed the game.
  3. The Installer will install the correct working versions of the Ddraw Wrapper and DxWnd.
  4. Run the game via the **Robin Hood - Performance Fix.exe** shortcut on your desktop. (If missing, create a new shortcut)
  5. For more information read the 'Description' and 'How to Install' on the download-page.

  
  
## Other information

### API

Technical specs | Supported | Notes  
---|---|---  
DirectDraw | 7 |   
  
Executable| PPC | 32-bit | 64-bit | Notes  
---|---|---|---|---  
Windows|  |  |  | _32-bit executables may need the[LAA flag applied](/wiki/Windows#Set_older_32-bit_games_to_use_4_GB_RAM_instead_of_2 "Windows") to work properly on modern machines._  
macOS (OS X)|  |  |  | Two versions exist for OS X: the original 2004 PowerPC port, and the 2011 digital release on the App Store.  
  
The digital release is unknown if it made it to 64-bit, but did have a Lion patch the same year it released.  
Linux|  |  |  |   
  
### Middleware

| Middleware | Notes  
---|---|---  
Audio | [FMOD](/wiki/FMOD "FMOD") | 3.6  
Cutscenes | [Bink Video](/wiki/Bink_Video "Bink Video") | 1.5L  
  
## System requirements

**Windows**| **macOS (OS X)**| **Linux**  
---|---|---  
  
[Windows](/wiki/Windows "Windows")  
---  
| Minimum[9]  
Operating system (OS) | 98, ME, 2000, XP  
Processor (CPU) | Intel Pentium II 233 MHz  
System memory (RAM) | 64 MB  
Storage drive (HDD/SSD) | 1 GB  
Video card (GPU) |  4 MB of VRAM  
DirectX 8.1 compatible  
  
[macOS (OS X)](/wiki/Mac_OS "Mac OS")  
---  
| Minimum  
Operating system (OS) | 10.6.6  
Processor (CPU) | Intel 1.8 GHz  
System memory (RAM) | 512 MB  
Storage drive (HDD/SSD) | 2 GB  
Video card (GPU) |  128 MB of VRAM  
  
[Linux](/wiki/Linux "Linux")  
---  
| Minimum  
Operating system (OS) |   
Processor (CPU) | 500 MHz x86  
500 MHz PowerPC  
System memory (RAM) | 128 MB  
Storage drive (HDD/SSD) | 1 GB  
Video card (GPU) |  8 MB of VRAM  
  
  


## Notes

  1. ↑ 1.0 1.1 When running this game without elevated privileges (**Run as administrator** option), write operations against a location below `[%PROGRAMFILES%](/wiki/Glossary:Game_data#Shared_applications "Glossary:Game data")`, `[%PROGRAMDATA%](/wiki/Glossary:Game_data#Shared_application_data "Glossary:Game data")`, or `[%WINDIR%](/wiki/Glossary:Game_data#Windows "Glossary:Game data")` might be redirected to `[%LOCALAPPDATA%](/wiki/Glossary:Game_data#User_application_data "Glossary:Game data")\VirtualStore` on Windows Vista and later ([more details](/wiki/Game_data#Installation_folder "Game data")).
  2. ↑ 2.0 2.1 **Notes regarding Steam Play (Linux) data:**
     * File/folder structure within this directory reflects the path(s) listed for Windows and/or Steam game data.
     * Use [Wine's registry editor](https://wiki.winehq.org/Regedit) to access any Windows registry paths.
     * The app ID (46560) may differ in some cases.
     * Treat backslashes as forward slashes.
     * See the [glossary page](/wiki/Glossary:Game_data#Windows_data_paths "Glossary:Game data") for details on Windows data paths.



## References

  1. ↑ [Gamesplanet](https://web.archive.org/web/20120706184532/http://uk.gamesplanet.com/buy-download-pc-games/Robin-Hood---Sherwood-43-43.html) \- last accessed on 2025-07-11
  2. ↑ [Gamesplanet](https://web.archive.org/web/20100717093533/http://uk.gamesplanet.com:80/buy-download-pc-games/Robin-Hood-Legend-of-Sherwood-43-17.html) \- last accessed on 2025-08-30
  3. ↑ [GamersGate](https://web.archive.org/web/20100204225025/http://www.gamersgate.com/DD-HOOD/robin-hood-the-legend-of-sherwood) \- last accessed on 2025-07-30
  4. ↑ [GamersGate](https://web.archive.org/web/20130204104234/http://www.gamersgate.com/DD-RHMAC/robin-hood-mac) \- last accessed on 2025-08-30
  5. ↑ [GOG.com - Forum - widescreen, page 1](https://www.gog.com/forum/robin_hood_legend_of_sherwood/widescreen) \- last accessed on May 2023
  6. ↑ [Big Box PC Games Brasil Wiki](https://big-box-pc-games-brasil.fandom.com/pt-br/wiki/Robin_Hood:_A_Lenda_de_Sherwood) \- last accessed on 2023-09-16
  7. ↑ Verified by [User:Suicide_machine](/wiki/User:Suicide_machine "User:Suicide machine") on 2016-10-21
  8. ↑ Verified by [User:Misorian](/w/index.php?title=User:Misorian&action=edit&redlink=1 "User:Misorian \(page does not exist\)") on 2021
  9. ↑ [mobygames.com - Unknown page title (retrieval failure)](https://www.mobygames.com/game/7907/robin-hood-the-legend-of-sherwood/cover/group-17564/cover-42063/) \- last accessed on 2025-09-19



[Categories](/wiki/Special:Categories "Special:Categories"): 

  * [Windows](/wiki/Category:Windows "Category:Windows")
  * [OS X](/wiki/Category:OS_X "Category:OS X")
  * [Linux](/wiki/Category:Linux "Category:Linux")
  * [One-time game purchase](/wiki/Category:One-time_game_purchase "Category:One-time game purchase")
  * [Singleplayer](/wiki/Category:Singleplayer "Category:Singleplayer")
  * [Real-time](/wiki/Category:Real-time "Category:Real-time")
  * [Bird's-eye view](/wiki/Category:Bird%27s-eye_view "Category:Bird's-eye view")
  * [Isometric](/wiki/Category:Isometric "Category:Isometric")
  * [Point and select](/wiki/Category:Point_and_select "Category:Point and select")
  * [Multiple select](/wiki/Category:Multiple_select "Category:Multiple select")
  * [Stealth](/wiki/Category:Stealth "Category:Stealth")
  * [Strategy](/wiki/Category:Strategy "Category:Strategy")
  * [Stylized](/wiki/Category:Stylized "Category:Stylized")
  * [Medieval](/wiki/Category:Medieval "Category:Medieval")
  * [Games](/wiki/Category:Games "Category:Games")
  * [Invalid template usage (DRM)](/wiki/Category:Invalid_template_usage_\(DRM\) "Category:Invalid template usage \(DRM\)")
  * [Pages needing references](/wiki/Category:Pages_needing_references "Category:Pages needing references")



[ ](https://www.facebook.com/PCGamingWiki) [ ](https://www.twitter.com/PCGamingWiki) [ ](//www.youtube.com/user/PCGamingWikiTV) [ ](//steamcommunity.com/groups/pcgamingwiki) [ ](https://discord.gg/SU27ykMcsD)

  * PCGamingWiki 
  * [About us](//pcgamingwiki.com/wiki/PCGamingWiki:About)
  * [Contact us](//pcgamingwiki.com/wiki/PCGamingWiki:About#Contact)
  * [Advertising](//pcgamingwiki.com/wiki/PCGamingWiki:About#Advertising)
  * [Privacy policy](//pcgamingwiki.com/wiki/PCGamingWiki:Privacy_policy)
  * [General disclaimer](//pcgamingwiki.com/wiki/PCGamingWiki:General_disclaimer) 

  * Friends 
  * [Partnerships](//pcgamingwiki.com/wiki/PCGamingWiki:Partnerships)
  * [Extension](//pcgamingwiki.com/wiki/PCGamingWiki:Extension)
  * [API](//pcgamingwiki.com/wiki/PCGamingWiki:API)
  * [AppleGamingWiki](https://www.applegamingwiki.com)
  * [GOG.com](https://www.gog.com?pp=708a77db476d737e54b8bf4663fc79b346d696d2)
  * [Gamesplanet](https://gamesplanet.com?ref=pcgwiki)
  * [CheapShark](https://www.cheapshark.com) 

  * Powered by 
  * [MediaWiki](https://www.mediawiki.org/wiki/MediaWiki)
  * [Semantic MediaWiki](https://www.semantic-mediawiki.org/wiki/Semantic_MediaWiki)
  * [Cargo](https://www.mediawiki.org/wiki/Extension:Cargo)
  * [Open source](https://github.com/PCGamingWiki)
  * [Patrons](https://www.patreon.com/PCGamingWiki)
  * and You <3 


This page was last edited on 29 March 2026, at 16:52.

Content is available under [Creative Commons Attribution Non-Commercial Share Alike](//creativecommons.org/licenses/by-nc-sa/3.0) unless otherwise noted.

Some store links may include affiliate tags. Buying through these links helps support PCGamingWiki ([Learn more](/wiki/PCGamingWiki:About#Support_us)).
  *[One-time game purchase]: Games which requires an upfront purchase to access.
  *[Singleplayer]: The game supports solo play through a singleplayer mode.
  *[Real-time]: Real-time games present the game continuously, as opposed to in turns.
  *[Bird's-eye view]: Any view that is above a player character or is an overview of a larger world, often at a small angle.
  *[Isometric]: View using isometric 2D assets to create the impression of 3D space. Often incorporates a bird’s-eye view.
  *[Point and select]: Controls actions or movements of characters or objects through pointing and selecting. This can be done by mouse, controller or motion controls or other gestures.
  *[Multiple select]: Control or selects multiple characters or units at the same time.
  *[Stealth]: Stealth games require the player to avoid contact with enemies in the game and instead try to pass them by silently and hidden or using disguises. Goals can range from reaching a certain position, theft, sabotage, etc.
  *[Strategy]: Games that use strategy.
  *[Stylized]: Rather hard to define on its own, "stylized" refers to something with its own distinct visual style. However, it is more often than not also used for exaggerated realism or hyperrealism, such where the game's world or environment is rendered realistically but contains some exaggerations, ranging from the subtle (e.g. a highly idealized version of an otherwise realistic environment; think "Disneyfied" versions of the real world) to the obvious (e.g. buildings with architecture that's very difficult or otherwise impossible to pull off in real life).
  *[Medieval]: Takes place in Europe or the Middle East between roughly the years 900 and 1550, or equivalent settings.
  *[singleplayer]: The game supports solo play through a singleplayer mode.
  *[bird's-eye view]: Any view that is above a player character or is an overview of a larger world, often at a small angle.
  *[isometric]: View using isometric 2D assets to create the impression of 3D space. Often incorporates a bird’s-eye view.
  *[stealth]: Stealth games require the player to avoid contact with enemies in the game and instead try to pass them by silently and hidden or using disguises. Goals can range from reaching a certain position, theft, sabotage, etc.
  *[strategy]: Games that use strategy.
  *[DRM]: Digital rights management: Commonly used to refer to copy protection and/or technical protection measures employed by companies in an attempt to limit the manipulation and copying of game data and content by end-users after the purchase, download, and/or install of the product.
  *[Keys]: Optional product keys for other services
  *[OS]: Operating system(s)
  *[_unavailable_]: Although the product has been listed on this source, it is no longer available for purchase.
  *[<path-to-game>]: The base installation folder
  *[Steam Play (Linux)]: Windows version on Steam running through the Proton wrapper of Steam Play on Linux
  *[<SteamLibrary-folder>]: The SteamLibrary folder the user installed the game under; or the base Steam installation folder if no alternate location was used.
  *[46560]: app ID may differ in some cases
  *[Multi-monitor]: Game can run at a spanned resolution across multiple displays
  *[Ultra-widescreen]: Game can run at an ultra-widescreen (21:9) resolution
  *[4K Ultra HD]: Game can run at 4K (3840x2160) resolution
  *[Field of view (FOV)]: Game has an adjustable field of view
  *[Windowed]: Game can run in a regular windowed mode
  *[Borderless fullscreen windowed]: Game can run in a borderless fullscreen windowed mode
  *[60 FPS]: Game can run at 60 frames per second
  *[120+ FPS]: Game can run at 120 frames per second (or higher)
  *[High dynamic range display (HDR)]: Game supports expanded color space on HDR-compatible displays
  *[Remapping]: This game supports the option to customize the keybind layout.
  *[Mouse sensitivity]: This game supports the option to adjust the speed of in-game mouse movement.
  *[Mouse acceleration]: This game supports the option to modify the acceleration curve of the in-game mouse movement.
  *[Mouse input in menus]: This game allows you to navigate the menus using the mouse input.
  *[[Keyboard](/wiki/Keyboard "Keyboard") and [mouse](/wiki/Glossary:Mouse "Glossary:Mouse") prompts]: This game supports keyboard and mouse prompts.
  *[Mouse Y-axis inversion]: This game supports the option to invert the mouse's Y-axis.
  *[Controller support]: This game supports controllers.
  *[Royalty free audio]: Also known as 'streamer-friendly audio' as it pertains to the use of audio or music that streamers and content creators are unlikely to receive DMCA strikes for using.
  *[Language]: Please note that some languages may not be available from all storefronts or across all versions.
  *[UI]: User interface.
  *[Sub]: Subtitles or closed captions.
  *[DirectDraw]: Only supported on Windows
  *[PPC]: PowerPC
  *[32-bit]: Intel 32-bit (x86 / IA-32)
  *[64-bit]: Intel 64-bit (x64 / x86-64 / AMD64)
  *[VRAM]: Video RAM
  *[%PROGRAMFILES%]: Windows: copy this path into a folder address bar to go to this location
  *[%PROGRAMDATA%]: Windows: copy this path into a folder address bar to go to this location
  *[%WINDIR%]: Windows: copy this path into a folder address bar to go to this location
  *[%LOCALAPPDATA%]: Windows: copy this path into a folder address bar to go to this location
