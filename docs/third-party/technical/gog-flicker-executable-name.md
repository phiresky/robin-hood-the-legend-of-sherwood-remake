# Movies and menu and all animated things in-game flickering? Here's a solution! (Win7)

- **Source:** [GOG forum thread](https://www.gog.com/forum/robin_hood_legend_of_sherwood/movies_and_menu_and_all_animated_things_ingame_flickering_heres_a_solution_win7)
- **Forum:** Robin Hood Legend of Sherwood
- **Posts:** 5
- **Date range:** June 30, 2014 – December 31, 2014
- **Language:** English

## Post 1

**Tarthur** — New User; registered October 2012; from Finland. Posted June 30, 2014. [Permalink](https://www.gog.com/forum/robin_hood_legend_of_sherwood/movies_and_menu_and_all_animated_things_ingame_flickering_heres_a_solution_win7/post1)

My problem was, that the intro movie was flickering, and also all the menus in the starting screen. Didn't bother to go ingame, because I had this exact same problem with Desperados - Wanted Dead or Alive, which is another game from the company that made this game. I KNOW the terrain would have been OK, but all the characters (animated things) flicker. Guess this is their signature bug/error in their games, heh?

The solution is: **RENAME YOUR GAME.EXE.** I renamed it `Robinhood_game.exe` (and the desperados game `desperados_game.exe`), and everything works like a charm.

Running a Lenovo Thinkpad Edge with Windows 7 and Intel i7 and 8gbram + nvidia geforce 630m.

I hope this helps someone with their game. :)

## Post 2

**Korell** — registered June 2009; from the United Kingdom. Posted July 1, 2014. [Permalink](https://www.gog.com/forum/robin_hood_legend_of_sherwood/movies_and_menu_and_all_animated_things_ingame_flickering_heres_a_solution_win7/post2)

> **Tarthur:** My problem was, that the intro movie was flickering, and also all the menus in the starting screen. Didn't bother to go ingame, because I had this exact same problem with Desperados - Wanted Dead or Alive, which is another game from the company that made this game. I KNOW the terrain would have been OK, but all the characters (animated things) flicker. Guess this is their signature bug/error in their games, heh?
>
> The solution is: RENAME YOUR GAME.EXE. I renamed it Robinhood_game.exe (and the desperados game desperados_game.exe), and everything works like a charm.
>
> Running a Lenovo Thinkpad Edge with Windows 7 and Intel i7 and 8gbram + nvidia geforce 630m.
>
> I hope this helps someone with their game. :)

Out of interest, does it have to be `<name>_game.exe` or can you rename it to anything different? Only I'm wondering if this issue is due to the nvidia game profiles in the driver software, and by renaming the executable it doesn't use the profile that the drivers have set up for them but the general settings instead.

## Post 3

**Tarthur** — New User; registered October 2012; from Finland. Posted August 16, 2014. [Permalink](https://www.gog.com/forum/robin_hood_legend_of_sherwood/movies_and_menu_and_all_animated_things_ingame_flickering_heres_a_solution_win7/post3)

> **Tarthur:** My problem was, that the intro movie was flickering, and also all the menus in the starting screen. Didn't bother to go ingame, because I had this exact same problem with Desperados - Wanted Dead or Alive, which is another game from the company that made this game. I KNOW the terrain would have been OK, but all the characters (animated things) flicker. Guess this is their signature bug/error in their games, heh?
>
> The solution is: RENAME YOUR GAME.EXE. I renamed it Robinhood_game.exe (and the desperados game desperados_game.exe), and everything works like a charm.
>
> Running a Lenovo Thinkpad Edge with Windows 7 and Intel i7 and 8gbram + nvidia geforce 630m.
>
> I hope this helps someone with their game. :)

> **Korell:** Out of interest, does it have to be `<name>_game.exe` or can you rename it to anything different? Only I'm wondering if this issue is due to the nvidia game profiles in the driver software, and by renaming the executable it doesn't use the profile that the drivers have set up for them but the general settings instead.

Hello

Seems like you can rename it in any ways, tried `platypushood_game`, worked like a charm. I was about to test reverting the file back to just `game.exe`, but there already was a `game.exe` with 0 bit data. DIdn't want to go there deleting the extra file, but seemed a bit strange. You might want to check if you have two `game.exe` files in the folder. I'd guess that the two get mixed up in the PC, when it tries to contact the real `game.exe`.

**EDIT:**

Whoa boy, am I a big idiot or what?

Yeah, you can name it anything, doesn't have to be `[name]_game.exe`, just `[name].exe` works fine.

*Post edited August 16, 2014 by Tarthur.*

## Post 4

**redacity** — New User; registered September 2011; from the United States. Posted December 30, 2014. [Permalink](https://www.gog.com/forum/robin_hood_legend_of_sherwood/movies_and_menu_and_all_animated_things_ingame_flickering_heres_a_solution_win7/post4)

> **Tarthur:** My problem was, that the intro movie was flickering, and also all the menus in the starting screen. Didn't bother to go ingame, because I had this exact same problem with Desperados - Wanted Dead or Alive, which is another game from the company that made this game. I KNOW the terrain would have been OK, but all the characters (animated things) flicker. Guess this is their signature bug/error in their games, heh?
>
> The solution is: RENAME YOUR GAME.EXE. I renamed it Robinhood_game.exe (and the desperados game desperados_game.exe), and everything works like a charm.
>
> Running a Lenovo Thinkpad Edge with Windows 7 and Intel i7 and 8gbram + nvidia geforce 630m.
>
> I hope this helps someone with their game. :)

I am a computer scientist and I CANNOT figure out why this should work. But it worked for me. Thanks!

## Post 5

**Korell** — registered June 2009; from the United Kingdom. Posted December 31, 2014. [Permalink](https://www.gog.com/forum/robin_hood_legend_of_sherwood/movies_and_menu_and_all_animated_things_ingame_flickering_heres_a_solution_win7/post5)

> **redacity:** I am a computer scientist and I CANNOT figure out why this should work. But it worked for me. Thanks!

From the way it is described, and from what little I know of NVIDIA GeForce drivers, I'd say it is the game profile in the graphics drivers. Using NVIDIA Inspector I can see that there is a built in profile for `Game.exe` so this profile's settings will be used when playing with the original exe name. But by renaming it to `Robinhood_game.exe` there is no matching profile in the drivers and so it uses the global profile instead. And there are some differences between these two profiles.
