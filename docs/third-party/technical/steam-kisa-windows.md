# Steam — Kisa's Windows 10/11 compatibility guide

- Original source: [Steam — Kisa's Windows 10/11 compatibility guide](https://steamcommunity.com/sharedfiles/filedetails/?id=640978579)
- Author / publication: Kisa ♥ / Steam Community
- Language / date: English; posted 2016-03-08; updated 2026-05-02
- Access: Full guide and all 78 comments retrieved directly across the base capture and captures for pages 2–8
- Checked: 2026-09-09
- Retrieved: 2026-09-09
- Captures: `steam-kisa.browser.html`, `steam-kisa-p2.browser.html` through `steam-kisa-p8.browser.html`
- Coverage: 78 unique comment IDs preserved below; no comment-count gap remains in the supplied captures

## English translation

The source is primarily in English. The following translations cover the non-English comments in the preserved source:

### coscaexports — 27 Jan @ 11:45pm

“Hi @kisa, could you please renew the DSWIN8.zip link? Unfortunately, it does not work :(”

### michaldziura92 — 28 Nov, 2021 @ 5:05pm

“Thank you for the help. The game runs without the slightest problem.”

### .:Lore*****:. — 12 Nov, 2020 @ 11:02am

“Thanks, bro.”

### Hagatio — 1 Feb, 2020 @ 10:58am

“You know, I do everything you say and it still does not work for me; the game runs at the same FPS. If anyone has another possible solution, please share :C”

### Kisa ♥ — 12 Nov, 2018 @ 10:52am

“You’re welcome! x3”

### 8vonoben — 12 Nov, 2018 @ 8:46am

“Very good, thank you. For the Germans, it is worth fighting through it because it works very well. Thanks @ Kisa | Remilia.”

### anemoneus — 28 Aug, 2016 @ 7:25pm

“Thanks a million, ¤ Nep Nep — found it!” (The Japanese phrase ネプネプ is a name/transliteration and is left as “Nep Nep.”)

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

#### Señor Gallina — 24 Feb, 2022 @ 7:12pm

Thank you very much! The second solution worked just fine for me :).

#### michaldziura92 — 28 Nov, 2021 @ 5:06pm

Thx for help.

#### michaldziura92 — 28 Nov, 2021 @ 5:05pm

Dziękuje za pomoc. Gra chodzi bez najmniejszego problemu.

#### Cauterize — 29 Mar, 2021 @ 10:06am

thanks. my fps problem solved!

#### cristian.3791 — 6 Jan, 2021 @ 9:49am

I can't run on my Win 10, can you help me?

#### .:Lore*****:. — 12 Nov, 2020 @ 11:02am

Grazie bro :praisesun:

#### Naxyň — 20 Sep, 2020 @ 4:24am

This methods are good but i have one another method with original font.

Link: [Click here](https://steamcommunity.com/sharedfiles/filedetails/?id=2227664168)

#### islandboyforreal — 7 Mar, 2020 @ 7:26pm

can't get the game to start and run on my lab top with windows 10

#### Hagatio — 1 Feb, 2020 @ 10:58am

sabes que hago todo lo que dices y aun así no me funciona y el juego corre a los mismo fps, si alguien tiene otra posible solución por favor compartir :C  ///   you know I do everything you say and it still doesn't work for me, the game runs at the same fps, if anyone has another possible solution please share :C

#### wcctacuda — 26 Dec, 2019 @ 6:15am

Fixed here. Thank you so much. This game is soo good

#### Shadowkaisa — 31 May, 2019 @ 8:30pm

followed the guide and after 10 seconds lagg comes back

#### Kisa ♥ — 14 Feb, 2019 @ 2:13pm

I don't know the exact build i used anymore but on the website it shows the date of the versions and i made this guide on march 8th of 2016 so maybe if you pick a version from that time it should work better? Good luck!

#### MPG Un1cOrnOfL0v3 — 14 Feb, 2019 @ 11:01am

I followed all the steps but I used the latest version of the program and its still laggy af, which version is the one that makes it work fine  ?

#### Kisa ♥ — 26 Nov, 2018 @ 6:56am

Hmm i don't really know i'd probably unplug the mouse and then back in and see if that loads the cursor

#### ning90407 — 26 Nov, 2018 @ 5:28am

Excuse me, I followed all the steps but there is no cursor(from the game) when I run it, is there anyway I fix it?

#### Kisa ♥ — 12 Nov, 2018 @ 10:52am

Bitte sehr! x3

#### 8vonoben — 12 Nov, 2018 @ 8:46am

mega gut, vielen Dank

für die Deutschen, es lohnt sich da durch zu kämpfen denn es funktioniert sehr gut.

Thanks @ Kisa | レミリア

#### Kisa ♥ — 11 Aug, 2018 @ 2:30pm

Enjoy! c:

#### The Поц — 11 Aug, 2018 @ 2:19pm

Tnx man. It's work. Good work! :zombiethumbsup:

#### Kisa ♥ — 22 Jul, 2018 @ 9:26am

I'm sorry that it didn't work for you :c

#### tury — 22 Jul, 2018 @ 6:45am

Works for about ten seconds, FPS than drops again :(

#### Kisa ♥ — 16 Jul, 2018 @ 8:04am

Maybe try updating/downloading directX? Robin Hood uses 8.1 i think~

#### Kisa ♥ — 15 Jul, 2018 @ 12:21pm

Have fun! It was my childhood game aswell :3

#### CreaTimagi — 14 Jul, 2018 @ 2:30pm

Just... thank you, thank you, thank you. This game was my childhood game and I'm so glad to have it back   a n d    it finally works.

#### Kisa ♥ — 26 May, 2018 @ 12:42pm

Glad it worked for you, enjoy! c:

#### kejmil16 — 26 May, 2018 @ 7:07am

Thx :D Finally I can play this game :)

#### UrbanRobin — 21 Jan, 2018 @ 10:34am

Okay, I missed a step so I fixed it's size, but the cursor is still leaving marks everywhere it moves, I double checked all the steps and I don't think I missed anything

#### UrbanRobin — 21 Jan, 2018 @ 10:20am

I followed the instructions and now the game is in top left corner the size of a quarter of m screen and the mouse cursor is leaving marks after moving. Any ideas how to fix it?

#### Kisa ♥ — 15 Dec, 2017 @ 5:17pm

I'm glad you are able to play now, have fun!

#### Thorg — 15 Dec, 2017 @ 7:49am

Thank you so much it worked I have no idea what I did with all these things but what it's important that I can finally play with it.I am in your debt. :steamhappy:

#### inuNd8 — 27 Nov, 2017 @ 9:54am

windows 7 doesn't need the first step! Thanks!

#### telekmatelek — 24 Nov, 2017 @ 5:08am

I did both methods but game is still stuttering and unplayable, does anyone know another methods?

#### Kisa ♥ — 17 Jul, 2017 @ 3:20am

Have fun! :nepnep:

#### Joa — 17 Jul, 2017 @ 2:55am

it worked! :D thx!! :steamhappy:

#### Kisa ♥ — 6 Jul, 2017 @ 4:44am

I'm glad it did, enjoy!

#### SWOBB — 5 Jul, 2017 @ 3:33pm

Thx a lot, it worked 4 me,

#### Kisa ♥ — 28 Mar, 2017 @ 3:33pm

Sadly that's not possible :I

#### NASCAR — 28 Mar, 2017 @ 1:26pm

Thank You , my game ist running in normal speed.

But i have not 1920x1080 , i have only nomal game size 1024*768

ist posibel to get 1920x1080?

greetings

#### Lupo — 10 Mar, 2017 @ 2:06pm

Still isn't working... :/

#### Kisa ♥ — 31 Dec, 2016 @ 5:28am

Yes it still doesnt run at full speed but its playable~Sadly i dont know any other way to get more fps~

#### GUBBO — 31 Dec, 2016 @ 4:38am

Hello, the game still really slow FPS, another solution? i tried everything,

#### Kisa ♥ — 31 Oct, 2016 @ 8:00pm

Im glad you found a fix! Have fun playing!

#### Zabu — 31 Oct, 2016 @ 5:36pm

Never mind!

This solution fixes the problem completely for me:

[https://www.gog.com/forum/robin_hood_legend_of_sherwood/the_game_running_slow/post25](https://www.gog.com/forum/robin_hood_legend_of_sherwood/the_game_running_slow/post25)

#### Zabu — 31 Oct, 2016 @ 5:21pm

Yea, that's correct. Every restart gives me acceptable framerates for about a minute, but it quickly deteriorates.

The cursor also causes visual glitches, especially during the main screen and when opening notes, I can try some more tinkereing with dxwnd but I'm not too sure what I'm doing there tbh..

#### Kisa ♥ — 28 Oct, 2016 @ 9:19pm

Sorry no idea~ Is it back at full speed if you restart the game?

#### Zabu — 28 Oct, 2016 @ 4:37pm

Hey! Thanks a ton, this really seemed to help!

However, I've noticed that as I'm playing through the first and second mission the game progressively slows down. The frames drop and the game itself runs less and less smoothly.

Do you have an idea what could cause this?

#### Kisa ♥ — 28 Aug, 2016 @ 8:41pm

Im glad, you're welcome!

#### anemoneus — 28 Aug, 2016 @ 7:25pm

Thanks a million, ¤ ネプネプ - found it!

#### Kisa ♥ — 28 Aug, 2016 @ 11:08am

Hello,

In your Steam library right click on Robin Hood and select "Properties". Then click on Local files and finally on the button "Browse local files..." :)

#### anemoneus — 28 Aug, 2016 @ 4:56am

Where is my "Steam\\SteamApps\\Common\\Robin Hood" folder please? I can open the game Steam, but I can't find any SteamApps or Robin Hood paths on my hard drive. I am trying to do the FPS fix. The Steam folder under my Programs only contains two files (Steam itself and Support Center), with no other folders. Thank you for any help you might be able to provide.

#### TheRevenantSkull — 10 Jul, 2016 @ 2:07am

Thank you but I have a question uhmm, after 5-10 minutes my game gets some lagg. Do you have any advice how I can improve my fps? Thank you for taking the time and reading. :)

#### ursus maritimus — 9 Jul, 2016 @ 12:52pm

Well, thanks for your time. I will try it.

#### Kisa ♥ — 9 Jul, 2016 @ 12:48pm

Hmm.. I would reinstall the game (and delete left over files from game folder if there are any) and try again from step one. Maybe you made something wrong and didnt notice. It doesnt hurt to try again and sadly i don't know of any other solution to that, i never had a crash~

#### ursus maritimus — 9 Jul, 2016 @ 12:40pm

Yes I did. Didn't help..

#### Kisa ♥ — 9 Jul, 2016 @ 12:34pm

Did you try to verify integry of game cache? Sometimes it's that simple to solve such issues

#### ursus maritimus — 9 Jul, 2016 @ 11:42am

Please help me. I followed every step and the game just crashes. It doesn't even load the start menu.

#### Kisa ♥ — 4 Jul, 2016 @ 3:40pm

Youre welcome, enjoy!

#### TheRevenantSkull — 4 Jul, 2016 @ 11:50am

It works, Thank you!!!!!!!

#### Rodan — 1 Jul, 2016 @ 11:44pm

it works, thanks mate :2016weiner:

#### grosman.alfa — 10 May, 2016 @ 4:05pm

Great guide, I owe you that one, man!

#### cavalera.vb — 1 May, 2016 @ 11:17pm

Win 10 works
Thank you!

#### Kisa ♥ — 29 Apr, 2016 @ 9:15pm

Thanks!

#### HeLLa — 29 Apr, 2016 @ 3:17pm

Of course, consider it done!

#### Kisa ♥ — 29 Apr, 2016 @ 3:08pm

Youre welcome, I'm happy if i can help! And i would love if you rate my guide :)

#### HeLLa — 29 Apr, 2016 @ 2:56pm

Really awesome. Most thorough and descriptive guide so far. Helped me fix the issues. Thanks man! :steamhappy:

#### Katiniukas — 18 Apr, 2016 @ 10:59pm

BEST WIN 10WORKS!!! :sundrive:

#### Kisa ♥ — 9 Mar, 2016 @ 11:47pm

Im glad it helped you and good to know :)

#### Zairev — 9 Mar, 2016 @ 7:09pm

This helped a lot, thanks mate ! (PS: Works in 8.1 too)
