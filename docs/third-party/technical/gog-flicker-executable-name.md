# GOG — executable naming and flickering report

- Original source: [Movies and menu and all animated things in-game flickering? Here's a solution! (Win7)](https://www.gog.com/forum/robin_hood_legend_of_sherwood/movies_and_menu_and_all_animated_things_ingame_flickering_heres_a_solution_win7)
- Author / publication: Tarthur, Korell, and redacity; GOG forum.
- Language / date: English; 2014-06-30 to 2014-12-31.
- Access: Full thread (all 5 posts) retrieved directly
- Checked: 2026-09-09
- Retrieved: 2026-09-09
- Archived copy: lookup failed (archive.org rate limit); not checked
- Format: header notes, original summary, then complete post-by-post notes (forum thread, not transcribed)

Tarthur reports that renaming Game.exe stopped intro and menu flickering on a Windows 7 laptop with a GeForce 630M. Further testing says the replacement name need not follow a particular pattern. redacity independently reports success.

Korell proposes that the new filename avoids an NVIDIA driver profile associated with Game.exe, and reports inspecting such a profile with NVIDIA Inspector. That mechanism remains a participant’s explanation rather than a proven diagnosis. The opening’s prediction about in-game character flicker was inferred from Desperados before entering a mission; it should not be counted as an observed Robin Hood symptom.

## Detailed notes

Page facts: GOG forum thread "Movies and menu and all animated things in-game flickering? Here's a solution! (Win7)" in the Robin Hood Legend of Sherwood subforum; 5 posts on a single page; opened by Tarthur on 2014-06-30, last post 2014-12-31; no post marked as the solution. Retrieved directly from gog.com on 2026-09-09. Forum threads are not licensed for reproduction, so the posts are summarised post by post.

| # | Poster (profile details shown) | Date | Content |
|---|---|---|---|
| 1 | Tarthur (New User; registered Oct 2012; Finland) | 2014-06-30 | Intro movie and all start-screen menus flickered. Did not go in-game because the same problem occurred with Desperados: Wanted Dead or Alive (same developer); assumes terrain would be fine but all animated things (characters) would flicker, calling it the developer's "signature bug". Solution: rename Game.exe. Renamed it Robinhood_game.exe (and Desperados to desperados_game.exe); "everything works like a charm". Hardware: Lenovo ThinkPad Edge, Windows 7, Intel i7, 8 GB RAM, NVIDIA GeForce 630M. |
| 2 | Korell (registered Jun 2009; United Kingdom) | 2014-07-01 | Asks whether the name must be `<name>_game.exe` or can be anything. Hypothesis: the issue is caused by NVIDIA game profiles in the driver software; renaming the executable stops the driver applying the per-game profile, so the general settings are used instead. |
| 3 | Tarthur | 2014-08-16 (edited same day) | Any name works; tried `platypushood_game` successfully. Wanted to test reverting to game.exe but found a second game.exe with 0 bytes already in the folder and did not delete it; suggests others check whether they have two game.exe files. Guesses the two get mixed up. Edit: calls himself an idiot and confirms the name can be anything, `[name].exe` works, no `_game` suffix needed. |
| 4 | redacity (New User; registered Sep 2011; United States) | 2014-12-30 | Quotes post 1. Says he is a computer scientist and cannot figure out why this should work, but it worked for him. |
| 5 | Korell | 2014-12-31 | From what he knows of NVIDIA GeForce drivers, it is the game profile in the graphics drivers. Using NVIDIA Inspector he can see a built-in profile for Game.exe, whose settings apply when the original exe name is used; with Robinhood_game.exe there is no matching profile so the global profile is used, and there are differences between the two profiles. |

Points to keep in mind:

- Only two players report the fix working (Tarthur, redacity); both on unspecified or NVIDIA hardware. Nobody in the thread tested on AMD or Intel graphics.
- The Windows 7 label comes from the thread title and Tarthur's system; no other OS is discussed.
- The "0 byte game.exe" observation in post 3 is unexplained and retracted in tone by the edit, but it is the only mention of file-system state.
- Korell's NVIDIA-profile explanation is a hypothesis backed by an NVIDIA Inspector observation, not by a before/after comparison of specific profile settings.
