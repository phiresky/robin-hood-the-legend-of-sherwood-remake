# Steam — Kisa's Windows 10/11 compatibility guide

- Original source: [Steam — Kisa's Windows 10/11 compatibility guide](https://steamcommunity.com/sharedfiles/filedetails/?id=640978579)
- Author / publication: Kisa ♥ / Steam Community
- Language / date: English; posted 2016-03-08; later revision date not fully verified
- Access: Guide text inspected
- Checked: 2026-09-09
- Format: original summary and research notes; not a transcript.

Kisa separates startup problems from low frame rates. The guide proposes Windows XP SP3 compatibility for startup, then describes an older combination of DirectDraw replacement files and DxWnd for performance.

Its manual recipe names DxWnd V2_02_90 and reports that some later builds do not behave the same way. It discusses RGB565 color handling, cursor behavior, and launch order.

These are historical instructions, not tested recommendations for the current Steam package. No linked executable was downloaded or run. Compare this guide with newer maintainer notes before attributing an old workaround to every edition.

## Converted text from the original HTML


### technical__steam-kisa-windows.html

_Source: `originals/technical__steam-kisa-windows.html`._

[ Sign in ](https://steamcommunity.com/login/home/?goto=sharedfiles%2Ffiledetails%2F%3Fid%3D640978579) [ Store ](https://store.steampowered.com/)

[ Home ](https://store.steampowered.com/) [ Discovery Queue ](https://store.steampowered.com/explore/) [ Wishlist ](https://store.steampowered.com/wishlist/) [ Points Shop ](https://store.steampowered.com/points/shop/) [ News ](https://store.steampowered.com/news/) [ Charts ](https://store.steampowered.com/stats/)

[ Community ](https://steamcommunity.com/)

[ Home ](https://steamcommunity.com/) [ Discussions ](https://steamcommunity.com/discussions/) [ Workshop ](https://steamcommunity.com/workshop/) [ Market ](https://steamcommunity.com/market/) [ Broadcasts ](https://steamcommunity.com/?subsection=broadcasts)

[ About ](https://store.steampowered.com/about/) [ Support ](https://help.steampowered.com/en/)

Change language 

[Get the Steam Mobile App](https://store.steampowered.com/mobile)

View desktop website 

© Valve Corporation. All rights reserved. All trademarks are property of their respective owners in the US and other countries.  [Privacy Policy](https://store.steampowered.com/privacy_agreement/)  |  [Legal](http://www.valvesoftware.com/legal.htm)  |  [Accessibility](https://help.steampowered.com/faqs/view/10BB-D27A-6378-4436)  |  [Steam Subscriber Agreement](https://store.steampowered.com/subscriber_agreement/)  |  [Refunds](https://store.steampowered.com/steam_refunds/)  |  [Cookies](https://store.steampowered.com/account/cookiepreferences/)

[ ](https://store.steampowered.com/)

[ ](https://store.steampowered.com/)

[ STORE ](https://store.steampowered.com/)

[ Home ](https://store.steampowered.com/) [ Discovery Queue ](https://store.steampowered.com/explore/) [ Wishlist ](https://store.steampowered.com/wishlist/) [ Points Shop ](https://store.steampowered.com/points/shop/) [ News ](https://store.steampowered.com/news/) [ Charts ](https://store.steampowered.com/stats/)

[ COMMUNITY ](https://steamcommunity.com/)

[ Home ](https://steamcommunity.com/) [ Discussions ](https://steamcommunity.com/discussions/) [ Workshop ](https://steamcommunity.com/workshop/) [ Market ](https://steamcommunity.com/market/) [ Broadcasts ](https://steamcommunity.com/?subsection=broadcasts)

[ About ](https://store.steampowered.com/about/) [ SUPPORT ](https://help.steampowered.com/en/)

[ Install Steam  ](https://store.steampowered.com/about/) [sign in](https://steamcommunity.com/login/home/?goto=sharedfiles%2Ffiledetails%2F%3Fid%3D640978579)  |  language

[ 简体中文 (Simplified Chinese) ](?l=schinese&id=640978579) [ 繁體中文 (Traditional Chinese) ](?l=tchinese&id=640978579) [ 日本語 (Japanese) ](?l=japanese&id=640978579) [ 한국어 (Korean) ](?l=koreana&id=640978579) [ ไทย (Thai) ](?l=thai&id=640978579) [ Bahasa Indonesia (Indonesian) ](?l=indonesian&id=640978579) [ Bahasa Melayu (Malay) BETA ](?l=malay&id=640978579) [ Български (Bulgarian) ](?l=bulgarian&id=640978579) [ Čeština (Czech) ](?l=czech&id=640978579) [ Dansk (Danish) ](?l=danish&id=640978579) [ Deutsch (German) ](?l=german&id=640978579) [ Español - España (Spanish - Spain) ](?l=spanish&id=640978579) [ Español - Latinoamérica (Spanish - Latin America) ](?l=latam&id=640978579) [ Ελληνικά (Greek) ](?l=greek&id=640978579) [ Français (French) ](?l=french&id=640978579) [ Italiano (Italian) ](?l=italian&id=640978579) [ Magyar (Hungarian) ](?l=hungarian&id=640978579) [ Nederlands (Dutch) ](?l=dutch&id=640978579) [ Norsk (Norwegian) ](?l=norwegian&id=640978579) [ Polski (Polish) ](?l=polish&id=640978579) [ Português (Portuguese - Portugal) ](?l=portuguese&id=640978579) [ Português - Brasil (Portuguese - Brazil) ](?l=brazilian&id=640978579) [ Română (Romanian) ](?l=romanian&id=640978579) [ Русский (Russian) ](?l=russian&id=640978579) [ Suomi (Finnish) ](?l=finnish&id=640978579) [ Svenska (Swedish) ](?l=swedish&id=640978579) [ Türkçe (Turkish) ](?l=turkish&id=640978579) [ Tiếng Việt (Vietnamese) ](?l=vietnamese&id=640978579) [ Українська (Ukrainian) ](?l=ukrainian&id=640978579) [Report a translation problem](https://www.valvesoftware.com/contact?contact-person=Translation%20Team%20Feedback)

[ Store Page ](https://store.steampowered.com/app/46560?snr=2_100100_100101_100106_apphubheader)

Robin Hood

[All](https://steamcommunity.com/app/46560) [Discussions](https://steamcommunity.com/app/46560/discussions/) [Screenshots](https://steamcommunity.com/app/46560/screenshots/) [Artwork](https://steamcommunity.com/app/46560/images/) [Broadcasts](https://steamcommunity.com/app/46560/broadcasts/) [Videos](https://steamcommunity.com/app/46560/videos/) [News](https://steamcommunity.com/app/46560/allnews/) [Guides](https://steamcommunity.com/app/46560/guides/) [Reviews](https://steamcommunity.com/app/46560/reviews/)

All  Discussions  Screenshots  Artwork  Broadcasts  Videos  News  Guides  Reviews 

### Robin Hood

[ Store Page ](https://store.steampowered.com/app/46560)

[Robin Hood](https://steamcommunity.com/app/46560) > [Guides](https://steamcommunity.com/app/46560/guides/) > [Kisa ♥'s Guides](https://steamcommunity.com/id/xNepnep/myworkshopfiles/?section=guides&appid=46560)

This item has been removed from the community because it violates Steam Community & Content Guidelines. It is only visible to you. If you believe your item has been removed by mistake, please contact [Steam Support](https://help.steampowered.com/en/wizard/HelpWithSteamIssue/?issueid=415). 

This item is incompatible with Robin Hood. Please see the [instructions page](https://steamcommunity.com) for reasons why this item might not work within Robin Hood. 

155 ratings

How to run the game on Windows 10/11

By Kisa ♥

How to start the game in Win10 and getting playable FPS

1

6

2

1

1

1

1

1

1

14

   

Award

Favorite

Favorited

Unfavorite

Share

This item has been added to your [Favorites](https://steamcommunity.com/my/myworkshopfiles/?section=guides&browsefilter=myfavorites).

Created by

[](https://steamcommunity.com/id/xNepnep)

Kisa ♥  
Online 

Category: [Gameplay Basics](https://steamcommunity.com/app/46560/guides/?browsesort=trend&filetype=11&requiredtags%5B%5D=Gameplay+Basics), [Modding or Configuration](https://steamcommunity.com/app/46560/guides/?browsesort=trend&filetype=11&requiredtags%5B%5D=Modding+or+Configuration)

Languages: [English](https://steamcommunity.com/app/46560/guides/?browsesort=trend&filetype=11&requiredtags%5B%5D=English)

Posted 

Updated 

8 Mar, 2016 @ 7:24am

2 May @ 11:02pm

13,789 | Unique Visitors  
---|---  
150 | Current Favorites  
  
Guide Index

Overview

How to run the game on Windows 10 

Automatic Setup for playable FPS 

Manual setup for playable FPS 

Comments

How to run the game on Windows 10 

Hello, i noticed many have problems in starting the game on Windows 10, including me. But i found an easy fix.  
  
  
1.) Go to your Robin Hood folder  
2.) Right click on Game.exe  
3:) Click on Properties  
4.) Click on Compatibility  
5.) Set compatibility mode to Windows XP (Service Pack 3) and click on Apply  
  
Done! 

Automatic Setup for playable FPS 

Just visit <https://www.moddb.com/games/robin-hood-the-legend-of-sherwood/downloads/robin-hood-performance-fix1>  
  
Download it and extract Robin Hood - Performance Fix.exe to the game folder and start it everytime you want to play the game! 

Manual setup for playable FPS 

The user ScottiePrimo from the GoG forums found a fix regarding the low FPS.  
  
1) Download the following: [https://drive.google.com/file/d/1Xw50wNnkMeC81fmxScCg6zBi8ZXq5_CH/view](https://steamcommunity.com/linkfilter/?u=https%3A%2F%2Fdrive.google.com%2Ffile%2Fd%2F1Xw50wNnkMeC81fmxScCg6zBi8ZXq5_CH%2Fview)  
1.1) Unrar the downloaded file (DSWin8.zip) and you should have two files in the extracted folder (ddraw.dll and aqrit.cfg).   
1.2) Place these two files into your "Steam\SteamApps\Common\Robin Hood" directory (in with the game.exe).   
  
2) Download V2_02_90_build.rar of DxWnd from... [http://sourceforge.net/projects/dxwnd/files/Latest%20build/](https://steamcommunity.com/linkfilter/?u=http%3A%2F%2Fsourceforge.net%2Fprojects%2Fdxwnd%2Ffiles%2FLatest%2520build%2F)   
(I've been advised by other users that the newer versions don't work for the purpose of running this game.)   
  
3) Unrar and inside the extracted foler and run "dxwnd.exe" as administrator. Go to "Edit->Add", in parameter "Name" set the name as Robin Hood.   
4) In parameter "Path" choose the .exe file of the game (in my case it looks like Steam\SteamApps\common\Robin Hood\game.exe).   
5) In the "Main" tab, under "Generic" untick "Run in Window".   
5.1) Also in "Main" tab, under "Position" set "Window initial position & size" to X=0 and Y=0. Set "W" and "H" to your native resolution (in my case that's W=1920 and H=1080).   
6) In the "Video" tab, under "Windows handling" tick "Modal Style" and under "Color management" tick "Set 16BPP RGB565 encoding".   
7) Next in "Input" tab tick "Hide Cursor". If the cursor flickers on the main screen of the game don't worry, it works fine in-game.   
8) Lastly click "OK" in dxwnd, if all is good it will show a green circle before the name of the game in DxWnd. Launch the game in Steam (Not in DxWnd itself, DxWnd runs in the backround!). You may get an error/alert that says something like "SetHook: proc=GetAvailableVidMem(D) oldhook=26b3e0". If this happens just press the Return key on your keyboard. If you can't select the error/alert window use Alt+Tab keys to cycle through the open windows. This part is just a bit of a trial and error, I don't know why it happens but you can get past it.   
9) You will need to start DxWnd every time before you launch the game. When you close dxwnd, answer "Yes" to save the options that you changed. That's it...   
  
  
I hope these fixes work for you aswell and if they do then have fun playing! :) 

78 Comments 

[<](javascript:void\(0\);) [>](javascript:void\(0\);)

[ ](https://steamcommunity.com/id/xNepnep)

[ Kisa ♥](https://steamcommunity.com/id/xNepnep)  [author]

2 May @ 11:05pm 

This comment is awaiting analysis by our automated content check system. It will be temporarily hidden until we verify that it does not contain harmful content (e.g. links to websites that attempt to steal information).

[ ](https://steamcommunity.com/id/coscaexports)

[ coscaexports](https://steamcommunity.com/id/coscaexports)

27 Jan @ 11:45pm 

Hi @kisa, kannst du bitte den DSWIN8.zip Link noch mal erneuern? Das klappt leider nicht :( 

[ ](https://steamcommunity.com/profiles/76561198095554085)

[ Excalibur](https://steamcommunity.com/profiles/76561198095554085)

19 Aug, 2024 @ 9:31am 

Hey, i managed to get the game to work in windowed mode. I am playing on Mac Air M2, running it in Parallels virtual machine. The game starts but the cursor becomes invisible and stays invisible in game. Can anyone help with that? 

[ ](https://steamcommunity.com/profiles/76561198063592047)

[ DaWezel](https://steamcommunity.com/profiles/76561198063592047)

17 Aug, 2024 @ 2:40pm 

it finaly worked after many many tries 

[ ](https://steamcommunity.com/id/xNepnep)

[ Kisa ♥](https://steamcommunity.com/id/xNepnep)  [author]

31 Mar, 2024 @ 12:15pm 

Glad it's still working for people :) Enjoy the game! 

[ ](https://steamcommunity.com/profiles/76561198020760755)

[ BerserGER](https://steamcommunity.com/profiles/76561198020760755)

23 Mar, 2024 @ 9:59pm 

Thanks ALOT

[ ](https://steamcommunity.com/id/omidtajik)

[ Hatzo](https://steamcommunity.com/id/omidtajik)

28 May, 2023 @ 2:49am 

thanks alot its working now 

[ ](https://steamcommunity.com/profiles/76561198355555416)

[ Knoblocha](https://steamcommunity.com/profiles/76561198355555416)

7 May, 2023 @ 3:01pm 

Hi, everybody.  
I have Windows 11 and I love this game. I also had a problem with FPS/Lag.  
  
Fortunately, the procedure given here:  
[https://www.pcgamingwiki.com/wiki/Robin_Hood:_The_Legend_of_Sherwood#Windowed](https://steamcommunity.com/linkfilter/?u=https%3A%2F%2Fwww.pcgamingwiki.com%2Fwiki%2FRobin_Hood%3A_The_Legend_of_Sherwood%23Windowed)   
... works correctly with DXWnd v2_05_95 under Windows 11. I was using 1600x900 resolution.  
  
But if you want, you can switch the game to run in Full Screen by adjusting the settings when you disable the "Run in Window" option.  
  
Changing the resolution using link bellow, also works:  
[https://www.pcgamingwiki.com/wiki/Robin_Hood:_The_Legend_of_Sherwood#Widescreen_resolution](https://steamcommunity.com/linkfilter/?u=https%3A%2F%2Fwww.pcgamingwiki.com%2Fwiki%2FRobin_Hood%3A_The_Legend_of_Sherwood%23Widescreen_resolution)

[ ](https://steamcommunity.com/profiles/76561199443369914)

[ smtamrakar212](https://steamcommunity.com/profiles/76561199443369914)

24 Dec, 2022 @ 1:33pm 

game has a lagg can some one help me 

[ ](https://steamcommunity.com/id/xNepnep)

[ Kisa ♥](https://steamcommunity.com/id/xNepnep)  [author]

25 Feb, 2022 @ 12:41am 

Updated the Download link for the DSWin8.zip file since it didn't seem to work anymore. I'm glad that this guide still helps you play the game :) 

[<](javascript:void\(0\);) [>](javascript:void\(0\);)

Share to your Steam activity feed

[]( "Share on Facebook")

[]( "Share on Twitter")

[]( "Share on Reddit")

Link: 

You need to sign in or create an account to do that.

[Sign In](https://steamcommunity.com/login/home/?goto=sharedfiles%2Ffiledetails%2F%3Fid%3D640978579%26insideModal%3D0%26requirelogin%3D1) [Create an Account](https://store.steampowered.com/join) Cancel

[Update](javascript:UpdateKVTagsSingle\(\);)


### technical__steam-kisa-windows.txt

_Source: `originals/technical__steam-kisa-windows.txt`._

[ Sign in ](https://steamcommunity.com/login/home/?goto=sharedfiles%2Ffiledetails%2F%3Fid%3D640978579) [ Store ](https://store.steampowered.com/)

[ Home ](https://store.steampowered.com/) [ Discovery Queue ](https://store.steampowered.com/explore/) [ Wishlist ](https://store.steampowered.com/wishlist/) [ Points Shop ](https://store.steampowered.com/points/shop/) [ News ](https://store.steampowered.com/news/) [ Charts ](https://store.steampowered.com/stats/)

[ Community ](https://steamcommunity.com/)

[ Home ](https://steamcommunity.com/) [ Discussions ](https://steamcommunity.com/discussions/) [ Workshop ](https://steamcommunity.com/workshop/) [ Market ](https://steamcommunity.com/market/) [ Broadcasts ](https://steamcommunity.com/?subsection=broadcasts)

[ About ](https://store.steampowered.com/about/) [ Support ](https://help.steampowered.com/en/)

Change language 

[Get the Steam Mobile App](https://store.steampowered.com/mobile)

View desktop website 

© Valve Corporation. All rights reserved. All trademarks are property of their respective owners in the US and other countries.  [Privacy Policy](https://store.steampowered.com/privacy_agreement/)  |  [Legal](http://www.valvesoftware.com/legal.htm)  |  [Accessibility](https://help.steampowered.com/faqs/view/10BB-D27A-6378-4436)  |  [Steam Subscriber Agreement](https://store.steampowered.com/subscriber_agreement/)  |  [Refunds](https://store.steampowered.com/steam_refunds/)  |  [Cookies](https://store.steampowered.com/account/cookiepreferences/)

[ ](https://store.steampowered.com/)

[ ](https://store.steampowered.com/)

[ STORE ](https://store.steampowered.com/)

[ Home ](https://store.steampowered.com/) [ Discovery Queue ](https://store.steampowered.com/explore/) [ Wishlist ](https://store.steampowered.com/wishlist/) [ Points Shop ](https://store.steampowered.com/points/shop/) [ News ](https://store.steampowered.com/news/) [ Charts ](https://store.steampowered.com/stats/)

[ COMMUNITY ](https://steamcommunity.com/)

[ Home ](https://steamcommunity.com/) [ Discussions ](https://steamcommunity.com/discussions/) [ Workshop ](https://steamcommunity.com/workshop/) [ Market ](https://steamcommunity.com/market/) [ Broadcasts ](https://steamcommunity.com/?subsection=broadcasts)

[ About ](https://store.steampowered.com/about/) [ SUPPORT ](https://help.steampowered.com/en/)

[ Install Steam  ](https://store.steampowered.com/about/) [sign in](https://steamcommunity.com/login/home/?goto=sharedfiles%2Ffiledetails%2F%3Fid%3D640978579)  |  language

[ 简体中文 (Simplified Chinese) ](?l=schinese&id=640978579) [ 繁體中文 (Traditional Chinese) ](?l=tchinese&id=640978579) [ 日本語 (Japanese) ](?l=japanese&id=640978579) [ 한국어 (Korean) ](?l=koreana&id=640978579) [ ไทย (Thai) ](?l=thai&id=640978579) [ Bahasa Indonesia (Indonesian) ](?l=indonesian&id=640978579) [ Bahasa Melayu (Malay) BETA ](?l=malay&id=640978579) [ Български (Bulgarian) ](?l=bulgarian&id=640978579) [ Čeština (Czech) ](?l=czech&id=640978579) [ Dansk (Danish) ](?l=danish&id=640978579) [ Deutsch (German) ](?l=german&id=640978579) [ Español - España (Spanish - Spain) ](?l=spanish&id=640978579) [ Español - Latinoamérica (Spanish - Latin America) ](?l=latam&id=640978579) [ Ελληνικά (Greek) ](?l=greek&id=640978579) [ Français (French) ](?l=french&id=640978579) [ Italiano (Italian) ](?l=italian&id=640978579) [ Magyar (Hungarian) ](?l=hungarian&id=640978579) [ Nederlands (Dutch) ](?l=dutch&id=640978579) [ Norsk (Norwegian) ](?l=norwegian&id=640978579) [ Polski (Polish) ](?l=polish&id=640978579) [ Português (Portuguese - Portugal) ](?l=portuguese&id=640978579) [ Português - Brasil (Portuguese - Brazil) ](?l=brazilian&id=640978579) [ Română (Romanian) ](?l=romanian&id=640978579) [ Русский (Russian) ](?l=russian&id=640978579) [ Suomi (Finnish) ](?l=finnish&id=640978579) [ Svenska (Swedish) ](?l=swedish&id=640978579) [ Türkçe (Turkish) ](?l=turkish&id=640978579) [ Tiếng Việt (Vietnamese) ](?l=vietnamese&id=640978579) [ Українська (Ukrainian) ](?l=ukrainian&id=640978579) [Report a translation problem](https://www.valvesoftware.com/contact?contact-person=Translation%20Team%20Feedback)

[ Store Page ](https://store.steampowered.com/app/46560?snr=2_100100_100101_100106_apphubheader)

Robin Hood

[All](https://steamcommunity.com/app/46560) [Discussions](https://steamcommunity.com/app/46560/discussions/) [Screenshots](https://steamcommunity.com/app/46560/screenshots/) [Artwork](https://steamcommunity.com/app/46560/images/) [Broadcasts](https://steamcommunity.com/app/46560/broadcasts/) [Videos](https://steamcommunity.com/app/46560/videos/) [News](https://steamcommunity.com/app/46560/allnews/) [Guides](https://steamcommunity.com/app/46560/guides/) [Reviews](https://steamcommunity.com/app/46560/reviews/)

All  Discussions  Screenshots  Artwork  Broadcasts  Videos  News  Guides  Reviews 

### Robin Hood

[ Store Page ](https://store.steampowered.com/app/46560)

[Robin Hood](https://steamcommunity.com/app/46560) > [Guides](https://steamcommunity.com/app/46560/guides/) > [Kisa ♥'s Guides](https://steamcommunity.com/id/xNepnep/myworkshopfiles/?section=guides&appid=46560)

This item has been removed from the community because it violates Steam Community & Content Guidelines. It is only visible to you. If you believe your item has been removed by mistake, please contact [Steam Support](https://help.steampowered.com/en/wizard/HelpWithSteamIssue/?issueid=415). 

This item is incompatible with Robin Hood. Please see the [instructions page](https://steamcommunity.com) for reasons why this item might not work within Robin Hood. 

155 ratings

How to run the game on Windows 10/11

By Kisa ♥

How to start the game in Win10 and getting playable FPS

1

6

2

1

1

1

1

1

1

14

   

Award

Favorite

Favorited

Unfavorite

Share

This item has been added to your [Favorites](https://steamcommunity.com/my/myworkshopfiles/?section=guides&browsefilter=myfavorites).

Created by

[](https://steamcommunity.com/id/xNepnep)

Kisa ♥  
Online 

Category: [Gameplay Basics](https://steamcommunity.com/app/46560/guides/?browsesort=trend&filetype=11&requiredtags%5B%5D=Gameplay+Basics), [Modding or Configuration](https://steamcommunity.com/app/46560/guides/?browsesort=trend&filetype=11&requiredtags%5B%5D=Modding+or+Configuration)

Languages: [English](https://steamcommunity.com/app/46560/guides/?browsesort=trend&filetype=11&requiredtags%5B%5D=English)

Posted 

Updated 

8 Mar, 2016 @ 7:24am

2 May @ 11:02pm

13,789 | Unique Visitors  
---|---  
150 | Current Favorites  
  
Guide Index

Overview

How to run the game on Windows 10 

Automatic Setup for playable FPS 

Manual setup for playable FPS 

Comments

How to run the game on Windows 10 

Hello, i noticed many have problems in starting the game on Windows 10, including me. But i found an easy fix.  
  
  
1.) Go to your Robin Hood folder  
2.) Right click on Game.exe  
3:) Click on Properties  
4.) Click on Compatibility  
5.) Set compatibility mode to Windows XP (Service Pack 3) and click on Apply  
  
Done! 

Automatic Setup for playable FPS 

Just visit <https://www.moddb.com/games/robin-hood-the-legend-of-sherwood/downloads/robin-hood-performance-fix1>  
  
Download it and extract Robin Hood - Performance Fix.exe to the game folder and start it everytime you want to play the game! 

Manual setup for playable FPS 

The user ScottiePrimo from the GoG forums found a fix regarding the low FPS.  
  
1) Download the following: [https://drive.google.com/file/d/1Xw50wNnkMeC81fmxScCg6zBi8ZXq5_CH/view](https://steamcommunity.com/linkfilter/?u=https%3A%2F%2Fdrive.google.com%2Ffile%2Fd%2F1Xw50wNnkMeC81fmxScCg6zBi8ZXq5_CH%2Fview)  
1.1) Unrar the downloaded file (DSWin8.zip) and you should have two files in the extracted folder (ddraw.dll and aqrit.cfg).   
1.2) Place these two files into your "Steam\SteamApps\Common\Robin Hood" directory (in with the game.exe).   
  
2) Download V2_02_90_build.rar of DxWnd from... [http://sourceforge.net/projects/dxwnd/files/Latest%20build/](https://steamcommunity.com/linkfilter/?u=http%3A%2F%2Fsourceforge.net%2Fprojects%2Fdxwnd%2Ffiles%2FLatest%2520build%2F)   
(I've been advised by other users that the newer versions don't work for the purpose of running this game.)   
  
3) Unrar and inside the extracted foler and run "dxwnd.exe" as administrator. Go to "Edit->Add", in parameter "Name" set the name as Robin Hood.   
4) In parameter "Path" choose the .exe file of the game (in my case it looks like Steam\SteamApps\common\Robin Hood\game.exe).   
5) In the "Main" tab, under "Generic" untick "Run in Window".   
5.1) Also in "Main" tab, under "Position" set "Window initial position & size" to X=0 and Y=0. Set "W" and "H" to your native resolution (in my case that's W=1920 and H=1080).   
6) In the "Video" tab, under "Windows handling" tick "Modal Style" and under "Color management" tick "Set 16BPP RGB565 encoding".   
7) Next in "Input" tab tick "Hide Cursor". If the cursor flickers on the main screen of the game don't worry, it works fine in-game.   
8) Lastly click "OK" in dxwnd, if all is good it will show a green circle before the name of the game in DxWnd. Launch the game in Steam (Not in DxWnd itself, DxWnd runs in the backround!). You may get an error/alert that says something like "SetHook: proc=GetAvailableVidMem(D) oldhook=26b3e0". If this happens just press the Return key on your keyboard. If you can't select the error/alert window use Alt+Tab keys to cycle through the open windows. This part is just a bit of a trial and error, I don't know why it happens but you can get past it.   
9) You will need to start DxWnd every time before you launch the game. When you close dxwnd, answer "Yes" to save the options that you changed. That's it...   
  
  
I hope these fixes work for you aswell and if they do then have fun playing! :) 

78 Comments 

[<](javascript:void\(0\);) [>](javascript:void\(0\);)

[ ](https://steamcommunity.com/id/xNepnep)

[ Kisa ♥](https://steamcommunity.com/id/xNepnep)  [author]

2 May @ 11:05pm 

This comment is awaiting analysis by our automated content check system. It will be temporarily hidden until we verify that it does not contain harmful content (e.g. links to websites that attempt to steal information).

[ ](https://steamcommunity.com/id/coscaexports)

[ coscaexports](https://steamcommunity.com/id/coscaexports)

27 Jan @ 11:45pm 

Hi @kisa, kannst du bitte den DSWIN8.zip Link noch mal erneuern? Das klappt leider nicht :( 

[ ](https://steamcommunity.com/profiles/76561198095554085)

[ Excalibur](https://steamcommunity.com/profiles/76561198095554085)

19 Aug, 2024 @ 9:31am 

Hey, i managed to get the game to work in windowed mode. I am playing on Mac Air M2, running it in Parallels virtual machine. The game starts but the cursor becomes invisible and stays invisible in game. Can anyone help with that? 

[ ](https://steamcommunity.com/profiles/76561198063592047)

[ DaWezel](https://steamcommunity.com/profiles/76561198063592047)

17 Aug, 2024 @ 2:40pm 

it finaly worked after many many tries 

[ ](https://steamcommunity.com/id/xNepnep)

[ Kisa ♥](https://steamcommunity.com/id/xNepnep)  [author]

31 Mar, 2024 @ 12:15pm 

Glad it's still working for people :) Enjoy the game! 

[ ](https://steamcommunity.com/profiles/76561198020760755)

[ BerserGER](https://steamcommunity.com/profiles/76561198020760755)

23 Mar, 2024 @ 9:59pm 

Thanks ALOT

[ ](https://steamcommunity.com/id/omidtajik)

[ Hatzo](https://steamcommunity.com/id/omidtajik)

28 May, 2023 @ 2:49am 

thanks alot its working now 

[ ](https://steamcommunity.com/profiles/76561198355555416)

[ Knoblocha](https://steamcommunity.com/profiles/76561198355555416)

7 May, 2023 @ 3:01pm 

Hi, everybody.  
I have Windows 11 and I love this game. I also had a problem with FPS/Lag.  
  
Fortunately, the procedure given here:  
[https://www.pcgamingwiki.com/wiki/Robin_Hood:_The_Legend_of_Sherwood#Windowed](https://steamcommunity.com/linkfilter/?u=https%3A%2F%2Fwww.pcgamingwiki.com%2Fwiki%2FRobin_Hood%3A_The_Legend_of_Sherwood%23Windowed)   
... works correctly with DXWnd v2_05_95 under Windows 11. I was using 1600x900 resolution.  
  
But if you want, you can switch the game to run in Full Screen by adjusting the settings when you disable the "Run in Window" option.  
  
Changing the resolution using link bellow, also works:  
[https://www.pcgamingwiki.com/wiki/Robin_Hood:_The_Legend_of_Sherwood#Widescreen_resolution](https://steamcommunity.com/linkfilter/?u=https%3A%2F%2Fwww.pcgamingwiki.com%2Fwiki%2FRobin_Hood%3A_The_Legend_of_Sherwood%23Widescreen_resolution)

[ ](https://steamcommunity.com/profiles/76561199443369914)

[ smtamrakar212](https://steamcommunity.com/profiles/76561199443369914)

24 Dec, 2022 @ 1:33pm 

game has a lagg can some one help me 

[ ](https://steamcommunity.com/id/xNepnep)

[ Kisa ♥](https://steamcommunity.com/id/xNepnep)  [author]

25 Feb, 2022 @ 12:41am 

Updated the Download link for the DSWin8.zip file since it didn't seem to work anymore. I'm glad that this guide still helps you play the game :) 

[<](javascript:void\(0\);) [>](javascript:void\(0\);)

Share to your Steam activity feed

[]( "Share on Facebook")

[]( "Share on Twitter")

[]( "Share on Reddit")

Link: 

You need to sign in or create an account to do that.

[Sign In](https://steamcommunity.com/login/home/?goto=sharedfiles%2Ffiledetails%2F%3Fid%3D640978579%26insideModal%3D0%26requirelogin%3D1) [Create an Account](https://store.steampowered.com/join) Cancel

[Update](javascript:UpdateKVTagsSingle\(\);)
