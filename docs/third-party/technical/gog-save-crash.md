# GOG — intermittent city-mission save crashes

- Original source: [Saving progress in a mission crashes the game](https://www.gog.com/forum/robin_hood_legend_of_sherwood/saving_progress_in_a_mission_crashes_the_game)
- Author / publication: janz5, ConjurerDragon, and Kvothe43; GOG forum.
- Language / date: English; opened 2016-10-15, last reply 2025-08-26.
- Access: Full thread (all 6 posts) retrieved directly
- Checked: 2026-09-09
- Retrieved: 2026-09-09
- Archived copy: lookup failed (archive.org rate limit); not checked
- Format: header notes, original summary, then complete post-by-post notes (forum thread, not transcribed)

janz5 reports intermittent saving crashes under Windows 10 with DxWnd, sometimes after incapacitating and hiding enemies. Follow-up details specify version 1.1 installed under C:\GOG Games, successful saving in Sherwood encounters, and failures during city missions. Both quick and normal saves can fail, with either new or existing filenames.

ConjurerDragon asks about permissions, compatibility settings, background programs, and disk space. None is established as the cause. A later respondent reports the same symptom. This is stronger reproduction context than a generic missing-save complaint, but the inspected discussion supplies no confirmed fix.

## Detailed notes

Page facts: GOG forum thread "Saving progress in a mission crashes the game" in the Robin Hood Legend of Sherwood subforum; 6 posts on one page; opened by janz5 on 2016-10-15, last post 2025-08-26; no post marked as the solution. Retrieved directly from gog.com on 2026-09-09. Forum threads are not licensed for reproduction, so the posts are summarised post by post.

| # | Poster (profile details shown) | Date | Content |
|---|---|---|---|
| 1 | janz5 (New User; registered Apr 2016; Czech Republic; signs "jan") | 2016-10-15 | Bought and installed the game on Windows 10 in the default install directory. Game was slow; fixed with the DxWnd method described elsewhere on the forum. Separate problem: sometimes saving crashes the game to desktop. It always crashes at certain points within any mission, for example after knocking out several enemies and hiding them in a nearby building; the crash happens while in the save-game dialog. Cannot predict when, but it recurs at the same point. Continuing without saving allows saving later, but a crash can recur at another later point, losing all progress. Asks whether others have it. |
| 2 | ConjurerDragon (Generalissimus; registered Sep 2011; Germany) | 2018-05-21 | On Windows 7 and newer, older games installed under C:\Programs need administrator rights to run properly, otherwise they may lack rights to alter their own files. Recommends installing elsewhere (own install is K:\Spiele\Robin Hood). Also suggests checking the game is version 1.1, shown in the upper right of the screen when starting a game. |
| 3 | janz5 | 2018-06-11 | Running version 1.1. Installed in C:\GOG Games\, the default GOG location, not C:\Program Files; the program still needs administrator rights to run. Clarifies the problem is intermittent, not total: saving in the Sherwood mini-games never fails; failures occur only within the city missions and are unpredictable. Asks whether ConjurerDragon uses compatibility settings. |
| 4 | ConjurerDragon | 2018-06-11 | Does not use Windows 10. Has "Windows 98/Me" compatibility mode enabled and "enhanced text services" disabled (German: "erweiterte Textdienste"). Notes city-mission maps are larger and asks: enough free disk space? Normal saves only, or quicksaves too? Other background programs such as the Windows Indexing Service? Does it fail only when overwriting an existing savegame name (rights issue) or also with a completely new name? |
| 5 | janz5 | 2018-06-13 | Asked about compatibility settings because it reportedly helped another problem; will try the Win98/Me setting, which had not been tried. About 10+ GB free. Both normal and quick saves fail, and it makes no difference whether the name is existing or new. Concludes it is probably his PC, maybe an old HDD. Thanks for the advice. |
| 6 | Kvothe43 (New User; registered Dec 2013; Spain) | 2025-08-26 | "Very late to the party", but says it was definitely not janz5's PC: has the exact same issue. Asks whether it was ever solved. |

Facts usable for parity work: crash reproduces at consistent game states within city missions; both quick save and dialog save paths crash; Sherwood mini-game saves never crash; version 1.1 GOG build under DxWnd on Windows 10; a second independent report seven years later with no fix.
