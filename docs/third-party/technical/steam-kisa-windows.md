# Steam — Kisa's Windows 10/11 compatibility guide

- Original source: [Steam — Kisa's Windows 10/11 compatibility guide](https://steamcommunity.com/sharedfiles/filedetails/?id=640978579)
- Author / publication: Kisa ♥ / Steam Community
- Language / date: English; posted 2016-03-08; updated 2026-05-02
- Access: Full guide retrieved directly; 10 comments are present in the retrieved source, while the page reports 78 comments total
- Checked: 2026-09-09
- Retrieved: 2026-09-09
- Archived copy: lookup returned HTTP 000; not confirmed

## English translation

The source is primarily in English. The following is a complete translation of the non-English comment that appears in the preserved source:

### coscaexports — 27 Jan @ 11:45pm

“Hi @kisa, could you please renew the DSWIN8.zip link? Unfortunately, it does not work :(”

## Original text

### Guide metadata

**Robin Hood**

**How to run the game on Windows 10/11**

By Kisa ♥

How to start the game in Win10 and getting playable FPS

Category: [Gameplay Basics](https://steamcommunity.com/app/46560/guides/?browsesort=trend&filetype=11&requiredtags%5B%5D=Gameplay+Basics), [Modding or Configuration](https://steamcommunity.com/app/46560/guides/?browsesort=trend&filetype=11&requiredtags%5B%5D=Modding+or+Configuration)

Language: [English](https://steamcommunity.com/app/46560/guides/?browsesort=trend&filetype=11&requiredtags%5B%5D=English)

Posted: 8 Mar, 2016 @ 7:24am
Updated: 2 May @ 11:02pm

155 ratings
13,789 unique visitors
150 current favorites

### Guide index

- Overview
- How to run the game on Windows 10
- Automatic Setup for playable FPS
- Manual setup for playable FPS
- Comments

### How to run the game on Windows 10

Hello, i noticed many have problems in starting the game on Windows 10, including me. But i found an easy fix.

1. Go to your Robin Hood folder.
2. Right click on `Game.exe`.
3. Click on **Properties**.
4. Click on **Compatibility**.
5. Set compatibility mode to **Windows XP (Service Pack 3)** and click on **Apply**.

Done!

### Automatic Setup for playable FPS

Just visit <https://www.moddb.com/games/robin-hood-the-legend-of-sherwood/downloads/robin-hood-performance-fix1>.

Download it and extract `Robin Hood - Performance Fix.exe` to the game folder and start it everytime you want to play the game!

### Manual setup for playable FPS

The user ScottiePrimo from the GoG forums found a fix regarding the low FPS.

1. Download the following: [DSWin8.zip](https://drive.google.com/file/d/1Xw50wNnkMeC81fmxScCg6zBi8ZXq5_CH/view).
   1. Unrar the downloaded file (`DSWin8.zip`) and you should have two files in the extracted folder (`ddraw.dll` and `aqrit.cfg`).
   2. Place these two files into your `Steam\\SteamApps\\Common\\Robin Hood` directory (in with the `game.exe`).
2. Download `V2_02_90_build.rar` of DxWnd from [SourceForge](http://sourceforge.net/projects/dxwnd/files/Latest%20build/).

   > I've been advised by other users that the newer versions don't work for the purpose of running this game.

3. Unrar and inside the extracted foler run `dxwnd.exe` as administrator. Go to **Edit->Add**, in parameter **Name** set the name as Robin Hood.
4. In the **Path** parameter, choose the `.exe` file of the game (in my case it looks like `Steam\\SteamApps\\common\\Robin Hood\\game.exe`).
5. In the **Main** tab, under **Generic**, untick **Run in Window**.
   1. Also in the **Main** tab, under **Position**, set **Window initial position & size** to X=0 and Y=0. Set **W** and **H** to your native resolution (in my case that's W=1920 and H=1080).
6. In the **Video** tab, under **Windows handling**, tick **Modal Style** and under **Color management**, tick **Set 16BPP RGB565 encoding**.
7. Next, in the **Input** tab, tick **Hide Cursor**. If the cursor flickers on the main screen of the game, don't worry; it works fine in-game.
8. Lastly click **OK** in DxWnd, if all is good it will show a green circle before the name of the game in DxWnd. Launch the game in Steam (Not in DxWnd itself, DxWnd runs in the backround!). You may get an error/alert that says something like `SetHook: proc=GetAvailableVidMem(D) oldhook=26b3e0`. If this happens just press the Return key on your keyboard. If you can't select the error/alert window use Alt+Tab keys to cycle through the open windows. This part is just a bit of a trial and error, I don't know why it happens but you can get past it.
9. You will need to start DxWnd every time before you launch the game. When you close DxWnd, answer **Yes** to save the options that you changed. That's it...

I hope these fixes work for you aswell and if they do then have fun playing! :)

### Comments

#### Kisa ♥ (author) — 2 May @ 11:05pm

This comment is awaiting analysis by our automated content check system. It will be temporarily hidden until we verify that it does not contain harmful content (e.g. links to websites that attempt to steal information).

#### coscaexports — 27 Jan @ 11:45pm

Hi @kisa, kannst du bitte den DSWIN8.zip Link noch mal erneuern? Das klappt leider nicht :(

#### Excalibur — 19 Aug, 2024 @ 9:31am

Hey, i managed to get the game to work in windowed mode. I am playing on Mac Air M2, running it in Parallels virtual machine. The game starts but the cursor becomes invisible and stays invisible in game. Can anyone help with that?

#### DaWezel — 17 Aug, 2024 @ 2:40pm

it finaly worked after many many tries

#### Kisa ♥ (author) — 31 Mar, 2024 @ 12:15pm

Glad it's still working for people :) Enjoy the game!

#### BerserGER — 23 Mar, 2024 @ 9:59pm

Thanks ALOT

#### Hatzo — 28 May, 2023 @ 2:49am

thanks alot its working now

#### Knoblocha — 7 May, 2023 @ 3:01pm

Hi, everybody.

I have Windows 11 and I love this game. I also had a problem with FPS/Lag.

Fortunately, the procedure given here:

[https://www.pcgamingwiki.com/wiki/Robin_Hood:_The_Legend_of_Sherwood#Windowed](https://steamcommunity.com/linkfilter/?u=https%3A%2F%2Fwww.pcgamingwiki.com%2Fwiki%2FRobin_Hood%3A_The_Legend_of_Sherwood%23Windowed)
... works correctly with DXWnd v2_05_95 under Windows 11. I was using 1600x900 resolution.

But if you want, you can switch the game to run in Full Screen by adjusting the settings when you disable the "Run in Window" option.

Changing the resolution using link bellow, also works:
[https://www.pcgamingwiki.com/wiki/Robin_Hood:_The_Legend_of_Sherwood#Widescreen_resolution](https://steamcommunity.com/linkfilter/?u=https%3A%2F%2Fwww.pcgamingwiki.com%2Fwiki%2FRobin_Hood%3A_The_Legend_of_Sherwood%23Widescreen_resolution)

#### smtamrakar212 — 24 Dec, 2022 @ 1:33pm

game has a lagg can some one help me

#### Kisa ♥ (author) — 25 Feb, 2022 @ 12:41am

Updated the Download link for the DSWin8.zip file since it didn't seem to work anymore. I'm glad that this guide still helps you play the game :)

_Coverage note: the retrieved HTML and TXT each contain the complete guide and 10 comments; the page itself reports 78 comments, so 68 comments are not present in the supplied source snapshot._
