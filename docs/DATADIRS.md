This document lists ALL KNOWN versions of this game and how they differ.

TLDR: either get the Leicester demo from
https://www.moddb.com/games/robin-hood-the-legend-of-sherwood/downloads/robin-hood-the-legend-of-sherwood-demo-robin-hood-the-legend-of-sherwood

or buy the full game from GOG:
https://www.gog.com/en/game/robin_hood


I DO NOT recommend buying the [steam version](https://store.steampowered.com/app/46560/Robin_Hood_The_Legend_of_Sherwood/) of this game. It does not properly work on modern Windows and the publishers do not seem to care about fixing it. Buy the GOG version.

# Language Codes

```
1028 - Chinese (Taiwan)
1029 - Czech +
1031 - German (Germany) +
1033 - English (US)
1036 - French (France) +
1040 - Italian (Italy) +
1041 - Japanese +
1042 - Korean
1045 - Polish +
1046 - Portuguese (Brazil)
1049 - Russian +
1054 - Thai
2047 - English (GB) +
2052 - Chinese (PRC)
2070 - Portuguese (Portugal)
3082 - Spanish (Spain Modern Sort) +
```

# Leicester Demo — "The Scarlet Night"

Mission: Rescue Will Scarlet from the Leicester jail. All known builds play the same
mission but differ in tutorials, Level.res size, and other details.

## Windows — ECoste build (2002-09-18, with tutorials)

Compiled by developer `ECoste` on their Windows machine. PE timestamp: 2002-09-18.

English (1033):

    filename: setup_demo_ecoste.exe (originally setup_us_demo.exe / Setup_us.exe / Robin_Hood_LS_Demo_1_an.exe)
    format:   Wise Installer (80.8 MB)
    source:   https://www.moddb.com/games/robin-hood-the-legend-of-sherwood/downloads/robin-hood-the-legend-of-sherwood-demo-robin-hood-the-legend-of-sherwood
    source:   https://archive.org/details/RobinHoodTheLegendOfSherwoodDemo
    source:   https://archive.org/details/Robin_Hood_demo (as "RH demo.exe" in zip)
    source:   http://quedzasvideogames.free.fr/robin_hood/telechargements_rh.php
    MD5:      a8c4df5cbf009f3381ba582e6fe6c5f2
    SHA256:   68cca8fd84ac87bac22f7092fd69282986f25107f43110c80726f34e3dc8c9ec

French (1036):

    filename: Robin_Hood_LS_Demo_1_fr.exe
    format:   Wise Installer (84.2 MB)
    source:   http://quedzasvideogames.free.fr/robin_hood/telechargements_rh.php
    MD5:      04e6e5e4f5edec9d2baccd909f3db8bf
    SHA256:   69ade643e7c5f700c1f4af1858dccd4f79dd51a87f1c742f573b59f86971a9f8

German (1031):

    filename: Robin_Hood_LS_Demo_1_al.exe (also Setup_de.exe on GameStar disc)
    format:   Wise Installer (81.0 MB)
    source:   http://quedzasvideogames.free.fr/robin_hood/telechargements_rh.php
    source:   https://archive.org/details/gscda2002 (GameStar 12/2002 disc, Demos/Robin/Setup_de.exe)

Also on PC Games 01/2003 DVD (`PCGDVD0103`) as `Demos/RobinHood/Setup_PCGames.exe` (112.8 MB —
larger than other demo builds, possibly a combined or extended demo). ISO incomplete on
archive.org, file could not be extracted to verify.
    MD5:      d6ca71596f36d686c7ff00ad5c9c24e1
    SHA256:   ab7bd1312fca4b1dfcea0e1ead48cceccbe03bdd360624ab9deaff13f9917c5d

- Locale: 1033 (English US) / 1036 (French) / 1031 (German)
- SCB script: 52 classes, includes 10 `Tutorial_*` classes for in-game hints
  - Character tutorials: Robin, Marianne, Tuck, Little John (Petit_jean), beggar (mendiant)
  - Mechanic tutorials: drawbridge (PontLevis 1/2/3), lockpicking (Crocheter), herbs (Trefle)
  - Each tutorial triggers `DisplayPopupText` when a PC actor interacts with a parchment object
- Level.res: 41 resources — only Leicester mission text + tutorial strings + mission pictures
- UI string table (343 strings) includes "3D sound" option (inserted at index 53)
- `ParchmentPrison` script shows 4 popup texts (IDs 10, 11, 23, 25)

This is the version our existing `datadirs/demo` matches (identical file hashes).

## Windows — Pariso build (2002-08-19, no tutorials)

Same mission, but an earlier build without the tutorial system.
Compiled by developer `PARISO~1.ROO` (Pariso?) on their Windows machine. PE timestamp: 2002-08-19.

    filename: setup_demo_pariso.exe (originally setup_english.exe / Setup_Demo_English.exe)
    format:   Wise Installer (83 MB)
    source:   https://www.moddb.com/games/robin-hood-the-legend-of-sherwood/downloads/robin-hood-the-legend-of-sherwood-demo
    MD5:      27b181a7ad61748447834e7deecdae94
    SHA256:   52b69cabad7757612283c5b3df5a4e79eb11d88538424837ca6dc0a8704dbf8d

- Locale: 1033 (English US)
- SCB script: 42 classes — no tutorial classes at all
- Level.res: 508 resources — accidentally includes text/pictures for ALL full game missions
  (mission titles, briefings, popup texts, wave paths for every level), not just Leicester
- UI string table (343 strings) lacks "3D sound" option, shifted by -1 relative to ECoste build
- `ParchmentPrison` script shows only 2 popup texts (IDs 10, 11)
- Title.bfn is 82 KB vs ECoste build's 56 KB (more glyphs)

## Windows — Differences between the two builds

Both play the same Leicester mission and produce identical file trees (same filenames),
but 17 files differ:

| File | ECoste build | Pariso build |
|------|----------|---------------|
| `Robin Hood.exe` | 2,891,776 bytes | 2,842,624 bytes |
| `Dem_Lei_MP.scb` | 64,632 bytes (52 classes) | 59,640 bytes (42 classes) |
| `Dem_Lei_MP.rhm` | 15,669 bytes | 15,149 bytes |
| `leicester.rhp` | same size, different content | same size, different content |
| `Level.res` | 927 KB (41 resources) | 3.5 MB (508 resources) |
| `Slideshow_in.pak` | 122 KB (3 images: Wanadoo, Strategy First, Spellbound) | 82 KB (2 images: Wanadoo, Spellbound — no Strategy First logo) |
| `DEFAULT.RES` | has extra cursor | slightly smaller |
| `Title.bfn` | 56 KB | 82 KB |
| 3 other `.bfn` fonts | slightly different sizes | slightly different sizes |
| `keyset1/2.cfg` | fewer bindings | more bindings |
| 3 `.wav` files | slightly different voice lines | slightly different voice lines |

Both include `Fmod.dll` (identical), `WiseUpdt.exe`, and DirectX 8.1 redistributables in `TEMP/`.

## Linux — Runesoft port

Same game data as the ECoste build above. Ported by Runesoft.

    filename: rh-linux-demo-x86.run
    format:   Makeself 2.1.3 self-extracting archive (~62 MiB)
    source:   https://archive.org/details/rh-linux-demo-x86
    MD5:      b98a9b4abe44787b2c05443fb350dd2e
    SHA256:   05a63fe131741d351ece9040d0fd3e0a452d18232ebaab183b0a68030e4fcb5e

Extract without executing:

    OFFSET=$(head -n 361 rh-linux-demo-x86.run | wc -c)
    tail -c +$((OFFSET + 1)) rh-linux-demo-x86.run | gunzip | tar x

Produces a `robinhood_demo/` directory with `Data/`, `1033/`, `arial.ttf`, and `robin_demo` (32-bit x86 ELF binary).

## AmigaOS 4 / MorphOS — PowerPC port

Same game data as the ECoste build above. Port for AmigaOS 4 / MorphOS (PowerPC).
Dated November 2006. Uses PowerSDL libraries. Includes `rh.png` icon not present in other versions.

    filename: RobinHood-demo.lha
    format:   LHA archive (63.9 MB)
    source:   https://www.morphos-storage.net/?id=1901771
    MD5:      02ed5fdaac6565ff3d71002699a2b745
    SHA256:   b3ad95b7255d1dce7f0b823df87503646d475cf2c5694577ad6bb4986b4eae4d

Binary: `robin_demo` — ELF 32-bit MSB relocatable, PowerPC.
Bundled libs: `powersdl.library`, `powersdl_gfx.library`, `powersdl_image.library`,
`powersdl_mixer.library`, `powersdl_net.library`, `powersdl_sound.library`, `powersdl_ttf.library`.

## Redump — German demo disc

| Field | Value |
|-------|-------|
| Redump | http://redump.org/disc/132050/ |
| Title | Robin Hood: Die Legende von Sherwood |
| Region | Germany |
| Language | German |
| Version | Demo v1.00 |
| Edition | Demo |
| Media | CD, Mode 1, 1 track |
| Size | 146,889,456 bytes |
| CRC-32 | `a90eeedc` |
| MD5 | `14dc71f72ca946d4b7707a71b6d25e55` |
| SHA-1 | `948dff7219681d2ee1d06cb745a3d29dea904bbd` |
| Mastering Code | ROBIN HOOD |
| Mastering SID | IFPI LTZ1 |

# Lincoln Demo — "Free Lincoln" (DEMO II)

Mission: Help Godwin recapture Lincoln castle.

English (1033):

    filename: Setup_demo_en_xp.exe
    format:   Wise Installer (92 MB)
    source:   https://archive.org/details/winxp-magazine-dvd-2003-06-issue-19-d
    MD5:      9e8edf0a4578d4a250f59abe010eda8e
    SHA256:   d058be90faaebdf06a7ff318927762c5e5ad19a53db0e01327972da4af5c9679

- Locale: 1033 (English US)
- EULA: English, Wanadoo Edition
- PE timestamp: 2002-11-19
- SCB script: `Demo_Lin.scb` (68,263 bytes, 58 classes, compiled by ECoste)
- Map: `lincoln.rhp` + `Lincoln.map`/`.min` (night)
- Includes `binkw32.dll`, `Loading.pak`; no `Slideshow_in.pak`
- 348 sound files, 6 music tracks (Lincoln-specific: `Lincoln_D.wav`, `Lincoln_NF.wav`)
- 46 character .rhs files

German (1031):

    filename: Robin_Hood_LS_Demo_2_al.exe (originally Setup_de.exe)
    format:   Wise Installer (74.6 MB)
    source:   https://web.archive.org/web/20110820155810/http://www.spellbound.de/files/demos/Robin_Hood/Demo2/Setup_de.exe
    source:   http://quedzasvideogames.free.fr/robin_hood/telechargements_rh.php
    MD5:      294e4e496d67a46077b7d8c7327efedb
    SHA256:   517704ff0e6e10e070cb1931d8f4303ce6c3bfa7a0683b5cfb917b3947381508

The fan site notes: *"Je n'ai jamais trouvé cette deuxième démonstration en une autre langue!"*
("I never found this second demo in any other language!") — however, an English build does exist (see above).

A developer data dump with both demo level sets also exists:

    filename: Levels.rar
    format:   RAR5 archive (12 MB)
    MD5:      508360551d0fa6003a694d78fbb9854d
    SHA256:   a56b7943c55068c9d4dc7206632edf74106cd76e0c5217d9f3e9174e86551a26

Contains `Demo_Lin.scb` (58 classes, compiled by ECoste, 2002-10-14), `Demo_Lin.rhm`,
`lincoln.rhp` (map geometry), `Lincoln.map`/`.min` (night map), plus the Leicester demo
files (`Dem_Lei_MP.*`, `leicester.rhp`, `Leicester.map`/`.min` — matching the ECoste build).

A YouTube video confirms this demo existed and was publicly released:
https://www.youtube.com/watch?v=LCXfCY0jx94
> "Two demo levels were released before the official game launch.
> This level was included as 'Free Lincoln' in the final release with some minor changes."

Reference notes:

- Version string: `DEMO II v1.00`
- Registry key: `Software/Spellbound Software/Robin Hood Demo II 1.0`
- Mission file: `Demo_Lin` (vs `Dem_Lei_MP` for Leicester)
- Proto-level/map: `Lincoln` (vs `Leicester`)
- Playable characters: `RSABC` — Robin, Scarlet, and 3 others (vs `RJMT` — Robin, Little John, Marian, Tuck for Leicester)
- Menu text table: resource ID `1000034` (vs `1000040` for Leicester)
- Uses `` (vs ``)
- Loading screen text: `"DEMO II v1.00"` (vs `"DEMO v1.00"`)

The Lincoln demo would have required different data files than the Leicester demo:
- A `Demo_Lin.scb` script (not `Dem_Lei_MP.scb`)
- A `Lincoln.rhp` map (not `leicester.rhp`)
- A `Level.res` with menu text at resource ID `1000034`

The English installer above contains all these files. The Pariso build's Level.res includes
full-game mission text for Lincoln (at resource `1000000`: "Free Lincoln" / "Godwin wants
to recapture his castle"), but it ships with Leicester map data and Leicester scripts — it
was never actually a Lincoln demo.

# Patch

    filename: patch_robin_hood.exe
    format:   Windows executable (2.9 MB)
    source:   http://quedzasvideogames.free.fr/robin_hood/telechargements_rh.php
    MD5:      94cd8ebeafdc6b324d938e291430bdef
    SHA256:   2aa97adf89383de880eccfb680bcda9b7aecfef38607e89b3b4bf02d838a485a

# Full Release

## Windows — Japanese retail (2003-03-14, v1.1)

Published by Imagineer Co., Ltd. (イマジニア株式会社), distributed by Capcom (株式会社カプコン).
Developer: Spellbound Entertainment AG. Flip-top big box, ¥7,980. Version 1.1.

    filename: RBNH203801.iso
    format:   CD image (637 MB)
    source:   https://archive.org/details/robin-hood-the-legend-of-sherwood-jp-20030314-win
    MD5:      a19342b1b22e0de7f29fb02bdedbaeb7
    SHA256:   c9a431a8862bad906529df3eb2e71f2c600b2dcaef74db3a72068674c634a080

- Locale: 1041 (Japanese)
- A v1.15 update patch is also available at the same archive.org item (Update_1.15.7z, 5.4 MB)

## Windows — USA retail (The Pirate Bay)

MDF/MDS image. The MDF matches the Redump USA disc image below by size and MD5.

    filename: Robin_Hood.mdf + Robin_Hood.mds
    local:    datadirs/installers/Robin_Hood_tpb/
    format:   MDF/MDS disc image (713 MB + 486 bytes), Mode 2 / 2352-byte sectors
    source:   unknown BitTorrent source (local directory name: Robin_Hood_tpb)
    BTIH:     F93C10CE424E449A5EAD12F7C759095146492D6B
    MD5:      5d0e420693f5ed48c89246f05ad65d94 (Robin_Hood.mdf)
    MD5:      55fd00942f4b05556cccd2e24f02724a (Robin_Hood.mds)
    SHA256:   6b724ca164a962f80ce8360ff655cd0de3508e87c2654445e60e02fdc06948a1 (Robin_Hood.mdf)
    SHA256:   fcbf06117410f724121b635f03a040bd7d0740fc124dcb65d8052d5d6cd8c0a4 (Robin_Hood.mds)

- Locale: 1033 (English US)
- Matches Redump disc 40703 (`747,557,328` bytes, MD5 `5d0e420693f5ed48c89246f05ad65d94`)

## Windows — Italian retail

Internet Archive CD image. Volume ID: `Robin_Hood`.

    filename: robin_hood_it.iso
    local:    datadirs/installers/robin_hood_it.iso
    format:   ISO 9660 CD image (664 MB)
    source:   https://archive.org/details/robin_hood_202207
    MD5:      c446dc5b1fbc1ec2de770cf7e8d9f6f7
    SHA256:   6bcd5bbb5b2e4f1c86905a27f97c4da0338e09f5874845fc450b3181f830e1f1

- Locale: 1040 (Italian)
- Includes Italian manual at `Manuale/RHManuale.pdf`

## Windows — German retail

Internet Archive Alcohol 120% disc image. The archive.org metadata notes that the CD is
copy-protected and does not run unmodified.

    filename: Robin_Hood.mdf + Robin_Hood.mds
    local:    datadirs/installers/german-ia/
    format:   MDF/MDS disc image (840 MB + 24 KB)
    source:   https://archive.org/details/robin-hood-die-legende-von-sherwood-german
    MD5:      86beed578e56ad9f250259b971606976 (Robin_Hood.mdf)
    MD5:      a373f05fa333d0c1a8a8e1b8c4a31ff7 (Robin_Hood.mds)
    SHA256:   cada9bb9557e5b987e1d0a5bd0cf6eebfba3881b03a2cfaa644993f0748dd646 (Robin_Hood.mdf)
    SHA256:   ef1ea4d6597c592a8e4f1241287ec79b865c791aa698fad062f78513db1afca4 (Robin_Hood.mds)

- Locale: 1031 (German)
- Internet Archive item title: `Robin Hood - Die Legende von Sherwood (German)`
- Internet Archive metadata date: 2003-07-09

## Windows — Russian retail

RuTracker ISO image. Volume ID: `ROBINHOOD`.

    filename: RobinHood-ru1.iso
    local:    datadirs/installers/RobinHood-ru1.iso
    format:   ISO 9660 CD image (594 MB)
    source:   https://rutracker.org/forum/viewtopic.php?t=3144799
    BTIH:     0853169003410CFB6E04BA501305029BFF6EE694
    MD5:      5cb02915a1f8beeb54cd7b0870b629c9
    SHA256:   75fb1dbd0b81cca47d1fc2a128190b21f5d9f31103d590f35239ee46fa02a147

- Locale: 1049 (Russian)
- Includes `NOTINC/1033/Data/Text/Level.res` and font files under `NOTINC/Data/Interfac/Fonts`

## Windows — Russian retail (InstallShield)

RuTracker MDF/MDS image. Volume ID: `ROBIN`.

    filename: ROBIN.mdf + ROBIN.mds
    local:    datadirs/installers/Робин гуд. Легенда Шервуда/
    format:   MDF/MDS disc image (696 MB + 688 bytes)
    source:   https://rutracker.org/forum/viewtopic.php?t=5595965
    BTIH:     13A1D8DB01E839E87354CE0ADDD4C751E2FAC62C
    MD5:      15815b7f7d9271a5add56a16a0b0e5da (ROBIN.mdf)
    MD5:      aa4373152e465d051eff22da0ed24e68 (ROBIN.mds)
    SHA256:   f9369c31dc51ecd16c18312f09524477f287994639b4304cc5afefc98de2c175 (ROBIN.mdf)
    SHA256:   22f453135f71344f9cf92bfcf61da25cb117c3716451502d15e41a1a38cc6ab6 (ROBIN.mds)

- Locale: 1049 (Russian)
- InstallShield-style installer with `DATA1.CAB`, `DATA1.HDR`, `SETUP.EXE`, and DirectX redistributables

## ~~Windows — Czech magazine coverdisc (Score #152, October 2006)~~ NOT THIS GAME

Despite the archive.org title listing "Robin Hood", this disc contains
**Robin Hood: Defender of the Crown** (volume ID: RHDOTC2), not Legend of Sherwood.

    source:   https://archive.org/details/score152dvd

## Windows — GOG/romsfun repackage (v2.0.0.12)

GOG-style installer with bonus content (artworks, avatars, manual, soundtrack, wallpapers).

Official GOG store page:

    source:   https://www.gog.com/en/game/robin_hood

    filename: robin-hood-romsfun.zip (contains setup_robin_hood_2.0.0.12.exe, 577 MB)
    format:   Zip archive (641 MB total)
    source:   https://romsfun.com/download/robin-hood-the-legend-of-sherwood-196025
    MD5:      d755e50c1a1a5c292c277f2808c92c2f
    SHA256:   479673d404d3bf697417b9809ce10dc7bf2ad8ef054982adae3eded1fb250de1

## Windows — GOG v1.1 hotfix (gogunlocked repackage)

GOG installer with v1.1 hotfix (build 24778). InnoSetup exe + bin. Same extras as romsfun.

    filename: robinhood-gogunlocked.zip (contains setup_robin_hood_-_the_legend_of_sherwood_1.1_hotfix_(24778).exe + .bin, 636 MB)
    format:   Zip archive (701 MB total)
    source:   https://gogunlocked.com/robin-hood-the-legend-of-sherwood-free-download/
    MD5:      00292c14650583a9b350ce23decda5b4
    SHA256:   7451b367a9713f43fad9a3f2b9c6a1d9af382a9a97809f676bd0aa9cd9a0c028

## Windows — GOG v1.1 hotfix v3 (RuTracker repackage)

GOG installer with v1.1 hotfix, GOG v3 build 80557. InnoSetup exe + bin, with bonus
content in separate zip files.

    filename: setup_robin_hood_-_the_legend_of_sherwood_1.1_gog_v3_(80557).exe + setup_robin_hood_-_the_legend_of_sherwood_1.1_gog_v3_(80557)-1.bin
    local:    datadirs/installers/Robin_Hood_-_The_Legend_of_Sherwood_1.1_hotfix_gog_v3_(80557)_win_gog/
    format:   InnoSetup installer (1.1 MB exe + 635 MB bin, 701 MB directory total)
    source:   https://rutracker.org/forum/viewtopic.php?t=5606819
    BTIH:     7B2C37C36876AFA8C186E5AECA65F6DD09247B73
    MD5:      8ab629e662919ba32c9f1094cd7bcb4a (setup_robin_hood_-_the_legend_of_sherwood_1.1_gog_v3_(80557).exe)
    MD5:      fe407e06182b924375bcd1e4bce034af (setup_robin_hood_-_the_legend_of_sherwood_1.1_gog_v3_(80557)-1.bin)
    SHA256:   43fe38caa9ffe35e868d19bd358a0f9e4e4ba0e9582ee5d4f1a97cf79a2ff3d6 (setup_robin_hood_-_the_legend_of_sherwood_1.1_gog_v3_(80557).exe)
    SHA256:   e280b9f2eab56cfa584dade364b6058f933a29f32a5b426382f5a519e7f86718 (setup_robin_hood_-_the_legend_of_sherwood_1.1_gog_v3_(80557)-1.bin)

- Bonus zips: soundtrack, wallpapers, artworks, avatars, manual

### German retail vs. GOG game executable

The German retail and GOG executables are related but are not the same build. This
comparison excludes the retail launch/CD-check code and compares the actual game
payload from `robin hood.icd` with `datadirs/fullgame_gog/Game.exe`.

    German retail payload:
        build timestamp:  2002-10-17 12:48:01
        file size:        2,891,776 bytes
        SHA256:           0dfffe2e9cd502cb11ab2373427277d1d1b8752da77ad9c04c30a26de7fb0365
        .text size:       2,570,713 bytes

    GOG Game.exe:
        build timestamp:  2003-01-15 16:21:56
        file size:        2,899,968 bytes
        SHA256:           b0963242470d96cdd8021e2cb81ccbc5e26b4fb2cb122febdec50ad30774f08e
        .text size:       2,580,025 bytes

The GOG build therefore contains 9,312 additional bytes of executable code. A
normalized x86 comparison found 4,809 of 5,790 direct-call-delimited regions
(83.1%) with exactly the same instruction and operand shapes after removing
relocated addresses. These matches cover at least 55.5% of the retail `.text`
section. This is a conservative lower bound: one changed instruction causes its
whole call-delimited region to be counted as different.

Confirmed code changes include:

- **Wide-character font rendering.** The retail code measures and draws narrow
  strings with `GetTextExtentPoint32A`, `DrawTextA`, and `TextOutA`. The
  corresponding GOG path accepts a 16-bit character, masks it with `0xffff`, and
  calls `GetTextExtentPoint32W`, `DrawTextW`, and `TextOutW` with a character
  count of one.
- **Additional international fonts.** GOG adds load/unload code and paths for
  `0051EU__.ttf`, `1141EU__.ttf`, `CloisterBlackEU.ttf`, `MERLINN.TTF`,
  `MSGOTHIC.TTF`, `MSMINCHO.TTF`, `KAIU.TTF`, and `MINGLIU.TTF`. It imports
  `AddFontResourceA` and `RemoveFontResourceA`; the retail payload does not.
- **Cinematic positioning.** GOG calls `_BinkGetSummary@8` after opening a
  cinematic and computes `(480 - video_height) / 2`, apparently to center the
  video vertically. The retail build has neither the call nor this calculation.
- **Developer console additions.** GOG contains the `OPTIMIZE` frame-cache
  allocator command and the `CHROMA` sprite hue/saturation/value command. Their
  implementation and diagnostic strings are absent from the retail payload.

The retail-only imports `GetDriveTypeA`, `GetVolumeInformationA`,
`GetFileVersionInfoA`, `GetFileVersionInfoSizeA`, `VerQueryValueA`,
`SetErrorMode`, and `lstrcpyA` belong to launch/version/CD-specific code and were
not treated as game-logic differences.

Both payloads are optimized 32-bit release builds compiled predominantly with
Microsoft Visual C++ 6.0:

- PE linker version `6.0`
- VC6 C and C++ compiler build `8168`
- Microsoft `Linker600` build `8447`

Their decoded Microsoft Rich headers also contain a few statically linked
objects produced by Visual Studio .NET 2002-era `Utc13` tools. The retail build
contains builds `8830`/`9466`, while GOG contains builds `9178`/`9210`. Thus the
main game compiler and linker are the same generation, but some linked
components and the game source differ.

Despite the executable differences, `DATA/robinhood.bks` and
`DATA/robinhood.dic` are byte-identical between these two installations:

    robinhood.bks SHA256: 0d836524f7fcd01a04efb7b8e10deef1395132b1e944094306e236bf085d0e8f
    robinhood.dic SHA256: 85ab438ed158c6f842f9d1e4ea4309d405e23f71f30c868f8a970b64128a3bc4

## Windows — Steam release

Steam store release published by Microids. The store page lists English, French,
German, and Spanish interface languages.

    source:   https://store.steampowered.com/app/46560/Robin_Hood_The_Legend_of_Sherwood/

## Mac OS X — Macintosh Garden DMG

HFS+ disk images from Macintosh Garden. The CD image contains `Install Robin Hood.pkg`
and English manuals; the v1.1 image contains a ready-to-run `Robin Hood.app` bundle.

    filename: Robin_Hood_CD_0.dmg
    local:    datadirs/installers/Robin_Hood_CD_0.dmg
    format:   Apple DMG / HFS+ disk image (532 MB compressed, 648 MB unpacked)
    source:   https://macintoshgarden.org/games/robin-hood-the-legend-of-sherwood
    MD5:      82824a1621831fee3c0f00d7dac0eb3b
    SHA256:   0f538c7e581f4ff6d02914848450f085bce1b553307728055cab51d7d69b3733

    filename: Robin_Hood_1.1.dmg
    local:    datadirs/installers/Robin_Hood_1.1.dmg
    format:   Apple DMG / HFS+ disk image (1.25 GB compressed, 1.36 GB unpacked)
    source:   https://macintoshgarden.org/games/robin-hood-the-legend-of-sherwood
    MD5:      501af9a2b8e84a5513cfd32cf976f142
    SHA256:   065c4f6f8431b803c3177ef0f92ba4091d852826c9f924ebf336a835bbcbaa61

    filename: Robin Hood Legend of Sherwood.dmg
    local:    datadirs/installers/Robin Hood Legend of Sherwood.dmg
    format:   Apple DMG / HFS+ disk image (550 MB compressed, 1000 MB unpacked)
    source:   https://rutracker.org/forum/viewtopic.php?t=140907
    BTIH:     545425FCCE54E5DC5F7BC16F80808784A3CEE066
    MD5:      6f166c4884d25ffa9b4afc9f5ea7a0da
    SHA256:   9ff90c4140778bb14f04527bc75df702cf6689dba0706885b9682fcaec576894

- Volume root: `Robin Hood CD`
- Installer package: `Install Robin Hood.pkg`
- Package payload: `Install Robin Hood.pkg/Contents/Archive.pax.gz` (524 MB)
- Manuals: `Manual/Robin Hood Manual.pdf`, `Manual/Robin Hood Manual for Print.pdf`
- v1.1 volume root: `Robin Hood 1.1`
- v1.1 app bundle: `Robin Hood.app`
- v1.1 game data path: `Robin Hood.app/Contents/Resources/Data`
- RuTracker volume root: `Robin Hood`
- RuTracker app bundle: `Robin Hood/RH.app`
- RuTracker game data paths: `Robin Hood/Data` and `Robin Hood/2047/Data`

## ZETA OS — BeOS/ZETA port

Port for ZETA OS (a BeOS derivative). BFS filesystem disc image containing `RobinHood.zpkg`,
`Manual.pdf`, `Handbuch.pdf` (German manual), and `ReadME.txt`.

    filename: Robin Hood The Legend of Sherwood ZETA.iso
    format:   BFS disc image (557 MB)
    source:   https://archive.org/download/robin-hood-the-legend-of-sherwood-zeta
    MD5:      7e4fbec45cd6d42c69bab32fbabd6903
    SHA256:   e96c332b61178ea4d03423c60ddc0086e4bc4589f316c4e3240625a1443b187d

## Redump — full release disc images

| Redump | Title | Region | Lang | Version | Edition | Serial | Mode | Size | CRC-32 | MD5 | SHA-1 | Mastering Code | Mastering SID | Mould SID |
|--------|-------|--------|------|---------|---------|--------|------|------|--------|-----|-------|----------------|---------------|-----------|
| [40703](http://redump.org/disc/40703/) | Robin Hood: The Legend of Sherwood | USA | English | 1.0 | Original | 24423CD | Mode 2 | 747,557,328 | `7d4e4cae` | `5d0e420693f5ed48c89246f05ad65d94` | `77ddff6fbe515388c04874a0a978f9cc9b5dcfb1` | 1AYM5\<9265\>24423 | IFPI L485 | IFPI 81C1, IFPI 818C |
| [35111](http://redump.org/disc/35111/) | Robin Hood: Legenda Sherwood | Poland | Polish | 1.1 | Nowa eXtra Klasyka | — | Mode 1 | 824,213,712 | `d66312b9` | `3305bb13fbcad234f0b17326b8de3b32` | `157830fe7b7ea9f00aba8ab1ef6229e3718c3e0c` | CDPROJEKT 1900697503 RBINHB Robin Hood GM Records | IFPI LT57 | IFPI Z901 |
| [113363](http://redump.org/disc/113363/) | Robin Hood: Die Legende von Sherwood | Germany | German | 1.0 | Back to Games | CD 60812 | Mode 1 | 812,286,720 | `528e254b` | `80950711ca5b4e995b750e3992493b66` | `e3b8ce9b2786051d50bb80c6c4d64ba76e018a8f` | MPO CR 60812ROBINALPHA @ 11/19/06 | IFPI L039 | IFPI 1263 |

# Linux Full Release — Runesoft

The license for releasing a Linux version of this game was at some point sold to Runesoft — a German company specializing in porting Windows games to Linux.

They released at least three different versions:

## Linux version v1.0

## Linux version v1.1

## Linux version v1.2

Multilingual, CD-ROM edition. 32-bit x86 ELF executable (MojoSetup installer).

    filename: robin.hood_1.2-multilingual_x86-20121114.mojo.run
    format:   MojoSetup ELF installer (832 MB)
    MD5:      a691372b2894d85ad66f22ceb750795f
    SHA256:   a9beda295cd1a34b4ede838caefdd0db93729aee3139bf9e4e42562c56107f62

Known bugs (from Runesoft's issue tracker): French/German language selection is swapped,
cutscene dialogues fail with "File!Not!Found" errors.

## Unsorted Runesoft notes

> https://bitbucket.org/runesoftdev/robinhood_public/issues

> Using robin.hood_1.2-multilingual.cdrom_x86-20121114.mojo.run the game is installed in the wrong language when selecting either French or German as the version on CD (English not tested), i.e. when selecting French the ingame texts are in German and vice versa. I should probably mention that the cutscene dialogues also aren't working any more - whenever a cutscene should play an error message similar to "File!Not!Found Data/Text/Dialogues/DLG_..." (written from my memory) appears.

> When using robin.hood_1.2-multilingual.cdrom_x86-20121108.mojo.run to create a system wide installation of the German version of the game, the following files are created with wrong permissions: * 1031/Data/Interface/Slideshow_in.pak * 1031/Data/Text/Level.res

> SDL_VIDEO_X11_XRANDR is enabled by default now. Get the 20121105 update, either from Desura or if you own the Linux retail cdrom from https://bitbucket.org/runesoftdev/robinhood_public/downloads
> https://bitbucket.org/runesoftdev/robinhood_public/issues/1/no-sound-and-game-hangs-on-quit

> The game is only designed to work with 640x480, 800x600 and 1024x768. In the screen-shot you can see one of the many drawbacks of supporting other and even wide screen resolutions. The whole GUI only works for these fixed resolutions and it is not easy to change.

> Sorry, this looks like a Unity/Ubuntu specific problem and we do not know how to fix it with SDL 1.2. We will have a look at this as soon as we migrate Robin Hood to SDL 2.0.

> Multi language support will be available with the next update. Currently we are porting Robin Hood from SDL 1.2 to SDL 2.0. We are planning to release an update after that. (2013-05-30)

(i don't think this ever happened?)
