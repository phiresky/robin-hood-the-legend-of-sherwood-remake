# DxWnd doesn't do anything

- **Source:** [SourceForge — DxWnd General Discussion](https://sourceforge.net/p/dxwnd/discussion/general/thread/f20e2b7e/)
- **Project:** DxWnd (“Window hooker to run fullscreen programs in window and much more...”)
- **Creator:** David Camacho
- **Created:** 2016-02-04
- **Updated:** 2021-10-08
- **Language:** English
- **Capture:** Pages 1 and 2 of 2; page 2 recovered from `originals/recovery/dxwnd-hooking-p2.web.txt`

## Original text

#### David Camacho — 2016-02-04

Hi,

I am trying to play Robin Hood: The Legend of Sherwood in windowed mode, but it doesn't do anything at all. I have put the path and the launch into the `game.exe` and if i try to run it from the dxwnd it executes it but with no change. I have checked the window mode, put the resolution at 1366x768 and executed it as administrator. I would really appreciate if you could somehow help me.

Thank you.

EDIT: I have also tried to check many other options to see if it did something, even crashing the game, but it always does the same thing and nothing changes.

*Last edit: David Camacho 2016-02-04*

#### gho — 2016-02-04

Other possible reasons for DxWnd doing nothing are the following:

1. you hooked what is in reality a little frontend executable, but the game logic in in another file (for instance, but it's just an example, `robin.exe` that loads and starts `robin.bin`)
2. the hooked task has compatibility settings for Win95 or Win98 platform. In this case, it's not really executed, but just emulated by some Windows virtual machine.
3. It's a GOG or Steam version that has another hooker that conflicts with DxWnd

In any case I remember having tested successfully that game, so don't despair: as soon as I'll be able to do some testing I'll give you a basic setting. Just tell me what type of game is it (CD/DVD, sometimes they are released in different versions, or GOG ? Steam or whatever). For the hooking ligic that could make a big difference.

And, please, try not to be so dramatic in your post titles: “DxWnd doesn't do anything” sounds a little unfair .... ;)

*Last edit: gho 2016-02-04*

#### David Camacho — 2016-02-04

Sorry for the dramatic title! What I meant is that it was not doing anything in my particular case with Robin Hood. Gonna change it now! (EDIT: I haven't been able to, sorry :/)

I got the Steam version. I've tried many configurations posted in their forum and GOG in order to increase the FPS of the game and none of them worked. Then I realised that DXwnd was not doing, i think, anything at all, because I tried to check multiple things to see if something happened and it did not. There was this [patch](http://188.138.113.72/SHARE/rpollice/DSWin8.zip), which actually improved the FPS in the game, but it is still unplayable. Changing the fonts hasn't worked either, although it seemed to work to a lot of people.

As you may see, I have done quite research but nothing helped, so I was a bit desesperated, thus my post title was this (Sorry again).

I think I'm hooking the correct file because it worked for many people, but I am not sure at all. I have recently updated to Windows 10, but I tried all the things mentioned above in Windows 8 and didn't work either.

I don't know if it will help, but I attach you the dxdiag of my computer.

Thank you for your time, you have done a great job! And sorry if I have seemed to be ungrateful!

Attachment: [DxDiag.txt](https://sourceforge.net/p/dxwnd/discussion/general/thread/f20e2b7e/c5aa/attachment/DxDiag.txt)

#### gho — 2016-02-04

Yes, I already experienced that in the SF board the title is the only thing you can't update.

Don't worry, it's not a problem. The game suffered from reason 1), that is the real game is not `Robin Hood.exe` but `game.exe`. In addition, it requires the **“Use DLL injection”** flag to avoid setting a different screen resolution and **“Compensate flip emulation”**. DxWnd also needs administrator privileges.

You can use the posted configuration file (just “Import” it), update the game patch and let me know it everything works ok!

Attachments: [Robin Hood - The Legend of Sherwood (GOG).dxw](https://sourceforge.net/p/dxwnd/discussion/general/thread/f20e2b7e/c5aa/7988/attachment/Robin%20Hood%20-%20The%20Legend%20of%20Sherwood%20%28GOG%29.dxw), [ingame.png](https://sourceforge.net/p/dxwnd/discussion/general/thread/f20e2b7e/c5aa/7988/attachment/ingame.png)

#### David Camacho — 2016-02-05

Hi,

I have tried to import the configuration but it has not worked, the circle appears red. Furthermore, I have tried to manually configurate it and the circle is green until I check “use DLL injection”, when it becomes red.

I don't know if it might affect, but I have the Steam version. I think the GOG one is in the patch 1.0, while the version of Steam is 1.1.

Thank you again for your time!

#### gho — 2016-02-05

Red is OK, it just means that you can't run the game however you like, but you NEED to run it from the DxWnd interface, because DxWnd needs to inject its code in the game in an early stage, and then it has to have control of the game startup.

Just double-click on the DxWnd entry (or right-click and select the “Run” command, and it should work.

If it is a GOG release, that is not a problem. For Steam games, though, if they need to be started from the Steam interface, the red games won't work correctly.

*Last edit: gho 2016-02-05*

#### David Camacho — 2016-02-05

It's not working.. :(

It just does always the same, despite the configuration I put in dxwnd. The game is actually executed even if I run it from DxWnd but it always does the same, there is no variation. Furthermore, if I execute the game while DXWnd is not opened, there is no difference.

*Last edit: David Camacho 2016-02-05*

#### gho — 2016-02-05

My guess is that DxWnd can't, for some reason, hook the game. You may try to get some logs (check all the log flags in the log tab), but likely you won't find any `dxwnd.log` log file in the game folder. If there's one, that should explain something.

#### David Camacho — 2016-02-06

Nevermind, I give up. Despite of it, thank you for your time, you have done a great job!

#### Anonymous — 2016-02-06

hey man, let's me tell you that i never see some one said the word “give up” with gho! Give him some time and i'm sure he will fix it for you :p

Would you mind try the game again on another PC? I see this guy had success with D3DWindower [here](http://www.gog.com/forum/robin_hood_legend_of_sherwood/widescreen)

*The later reply identifies this poster as cloudstr.*

#### gho — 2016-02-06

Thank you for your support, cloudstr. Anyway I know another user that had success with this game, and he's myself (see screenshot above), Robin Hood perfectly working on my Win10. That's why I would consider the game as supporte already: likely there's something odd or wrong in David's configuration, but I can understand him if he doesn't case so much to spend time in trying to windowize a game.

In this case, D3DWindower could be a helpful option: much easier to configure, though it didn't work for me.

#### Daniel — 2021-10-06

Hi,

i use your “Robin Hood - The Legend of Sherwood (GOG).dxw” for this game and it works fine for me with Win10 (with game version 1.1). But i have one thing i am missing:

I want to leave with my mouse cursor the game-window if i just move outside the game-Window (to and want to work something on my desktop). And i want to continue playing when i go back into the game window with my mouse.

Can you tell me what i have to change in the options to get this to work?

#### gho — 2021-10-07

Does Alt-Tab work? If it works but the program crashes add **“Main / Do not notify on task switch”** flag, the game shouldn't notice the switch and should continue to go on waiting for you to Alt-Tab in again.

Please, let me know if it works.

#### Daniel — 2021-10-07

Alt-Tab works, but i can not do the window to the task bar (minimize to hide). The game does not crash with alt-tab, but i need to hide it. I tried “Windows style: thick frame” but i still can not minimize it to the taskbar.

#### gho — 2021-10-07

please, follow at the end of the thread ...

#### suckmysock — 2016-09-21

hello, my (forum) buddy tell that he can't make Robin Hood working on some recent version of Dxwnd. He is running windows 7 and use the oldie v2.02.90 which recomended by most of the guide floating out there: [YouTube](https://www.youtube.com/watch?v=cykBJw_gwOI)

So i did the test on my machine and the latest version still works, but maybe XP is more compatible, who know? I would be very grateful if you can do the test on your win7 PC and export the stable configuration. That also will shred some light on this odd problem.

*Last edit: suckmysock 2016-09-21*

#### gho — 2016-09-21

Ok, I'll do it, but one possibility is that the trouble depends on the Hook / **“No hook update”** flag: strangely, some games requires it set and others unset (luckily the vast majority just don't care ...) so that I had to make it selectable, but I agree it is a pain in the neck. Maybe changing this flag the game may start working? It's an easy try, otherwise il will take a little while....

#### gho — 2016-09-21

I just tested the game on Win10, and it works perfectly with the suggested configuration. The only thing that prevent the game to run is DxWnd missing the administrator capabilities. I don't know if this is the only problem, I should test it on Win7 (I whish I had the time!...).

Definitely, WinXP is much more compatible with games, and possibly will work better with old dxwnd releases. Unfortunately, Win7/8/10 made a mess with the hooked pointers so that the latest dxwnd releases contain much more code to manage this new situation, but this is not necessarily useful on WinXP.

#### Riitaoja — 2016-09-22

The game works fine on my Win7 64bit machine. One does need to make sure **“optimize for AERO mode”** options is enabled. And to eliminate the mouse cursor trails use **“Compensate Flip emulation”**.

Here is what I wrote about the game before:

> One thing I did notice is that v2_02_91 introduced the AERO friendly mode. Now if one runs the game with the option “optimize for AERO mode” disabled on Win7 then part of the user interface is missing and the mouse cursor is not visible. This could happen if someone updated from 2.02.90 (or earlier) to 2.02.91 and did not update their game configuration. Or if someone is using an old .dxw export file created with version 2.02.90 or earlier.

#### suckmysock — 2016-09-22

Hey, thanks. My friend did not expect that the old `.dxw` export file will turn off “optimize for AERO mode” option, so I tell him turn that flag on, or better use the new export file in the latest release. Things are sorted out, the game now works fine for him!

*Last edit: suckmysock 2016-09-22*

#### sam — 2018-05-18

i downloaded dxwnd v2_02_90 but its not running what should i do ????

#### sam — 2018-05-18 (duplicate post)

i downloaded dxwnd v2_02_90 but its not running what should i do ????

#### Anonymous — 2018-05-19

*Empty post.*

*Last edit: Anonymous 2018-10-21*

#### gho — 2018-05-19

Uhm... if I can't take anything for granted, I would suggest you to download / install a `.rar` archive decompressor (WinRAR, WinZIP etc.) and unpack the `v2_02_90.rar` file in a folder of your choice. Then, You'll see a `DxWnd.exe` file inside that runs and will make you my worst nightmare, unless you read some DxWnd tutorial, the DxWnd help and learn by yourself.

Better yet, follow suckmysock advice and get yourself an up-to-date release.

OMG!

#### gho — 2021-10-07

Daniel wrote:

> Alt-Tab works, but i can not do the window to the task bar (minimize to hide). The game does not crash with alt-tab, but i need to hide it. I tried “Windows style: thick frame” but i still can not minimize it to the taskbar.

Ok, I have to test the game, I'll do later when at home. See you later ...

#### Daniel — 2021-10-08

OK, looking forward for an solution. Thank you in advance!

#### gho — 2021-10-08

I tested the game and in effect it behaves in a strange way, but can be managed without changing the suggested configuration. When Alt-Tabbing the game window does not minimize, but the game freezes showing a black window and when you Alt-Tab in again the game resumes from where you left it. The black window can be overlapped by other windows, so the game will stay quiet until you resume it, without interfering with any program that could take control of the desktop.

But the game configuration was pretty old, you could try this one in attach that seems to behave much better, also allowing to minimize the task in the Windows taskbar icons.

p.s. you may need or not need the flags in the Input section. If you have troubles with mouse control you can try to clear these flags.

*Last edit: gho 2021-10-08*

Attachment: `Robin Hood - The Legend of Sherwood (GOG).dxw`

#### Daniel — 2021-10-08

Thank you for the new configuration. This one does crash after the dxwnd logo.

#### gho — 2021-10-08

This is strange and unfortunate. If not done already you may try to upgrade DxWnd to latest release v2.05.75 (maybe in a separate folder, so that you'll have both versions available).
I think I tested the configuration with an old GOG game version. Maybe I can find a more recent one ...

#### Daniel — 2021-10-08

I tried the new version and have the same problem - it crashes. But one thing is weird:
I created an new configuration myself and did all the options like in your latest .dxw i imported. Now with "my" configuration the game runs and the "minimize" works (most times). Sometimes i have to alt+tab twice. But that is no problem.

Thank you for your help!

#### gho — 2021-10-08

Uhm... if you didn't set the "Expert mode" there's much more configuration that you don't see. If you want to clarify the mystery you can do a very simple thing: export the bad configuration to one file, then export the good configuration to another file and post them both here. I'll compare them and I will spot the difference.

#### Daniel — 2021-10-08

OK, i missed that. But know i found what is the option which let the game crashes:

In your settings the "Hook" Tab says "Injects suspended process" and the game does not start.
When i change this to "SetWIndowsHook" the game starts.

#### gho — 2021-10-08

The mystery is revealed: you must have the v1.1 release that is patched in a completely different way from 1.0. The culprit is a retailed ddraw.dll file that is dropped in the game folder and makes the game work, but it is not compatible with some DxWnd use case. Renaming that file makes the game to load the standard ddraw.dll, the game starts but soon it switches to a badly handled 16 bit video mode that shows bad colors and half width. In conclusion, I would stick to your configuration that works!

### Recovery provenance

Page 1 contains 25 posts; the recovered page-2 text contains 8 posts, all dated 2021-10-08. The full discussion now includes all 33 posts in chronological page order. Recovery source: `originals/recovery/dxwnd-hooking-p2.web.txt`, whose source header identifies `https://sourceforge.net/p/dxwnd/discussion/general/thread/f20e2b7e/?page=1`.
