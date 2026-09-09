# DxWnd — Overlay and Flipchains Emulation

- Original title: “Overlay and Flipchains Emulation”
- Source: [DxWnd General Discussion, pages 1–5 of 5](https://sourceforge.net/p/dxwnd/discussion/general/thread/566ddb1947/)
- Recovered captures: pages 1, 2, 4, and 5 from originals/recovery/dxwnd-flipchain-p{1,2,4,5}.web.txt; manual capture for page 3: originals/technical__dxwnd-flipchain-investigation-manual.html (canonical ?page=2)
- Authors: BEEN_Nath_58, gho, huh, and dippy dipper
- Language / dates: English; 17 March–13 April 2023
- Availability: **Complete discussion text across all 5 pages.** All captured posts, quotes, code fragments, and attachment filenames are retained. Linked binary/media contents are not included; recovered captures did not expose direct attachment URLs for every filename, so no URLs are invented.
- Checked: 2026-09-09

## Original text

### Page 1

#### BEEN_Nath_58 — 2023-03-17

Made this thread since overlay emulation is new and many aspects can be covered here like like in the CD audio thread.

#### BEEN_Nath_58 — 2023-03-17

Although the setting is present, it is most broken than ever:
* overlaydemo.exe : doesn't even display
* mosquito.exe : it makes a big mosquito photo on the screen over which it plays. On Windows 11 the mosquito also starts bouncing up and down.
* Nascar Revolution : they have either the green screen over the video or vice versa. It is not consistent, it USED TO BE consistent.
* mosquito.exe of DX6 : although it is the same, it replaces the DX7 calls with older DX calls. You will need to add those methods so that overlay works for them too.
I would be happy if the first 3 BUGS are fixed before .94 release.

#### gho — 2023-03-17

I have some suspects that some problems could depend on the different DxWnd configuration. As you saw, I tend to use default windowed 800x600 configurations while you seem to prefer full-sized settings. Of course there's nothing bad in this, but if you share the exports of your tests it would be easier for me to replicate and fix the problems.

#### BEEN_Nath_58 — 2023-03-17

Actually the problem is consistent with any profile. For the Nascar game, I only added Force Windowing.

The simplest problematic problem is the default + Emulate Overlay.

#### BEEN_Nath_58 — 2023-03-17

NEW APP:

Apparently http: //www. geisswerks. com/drempels/

is an overlay app. With DxWnd I get an UpdateOverlay failed error (Run drempels.exe from C:\Windows)

#### gho — 2023-03-18

There are no errors about UpdateOverlay in the DxWnd log and, if I remember correctly, that could be because I didn't hook the UpdateOverlay call yet to IDirectDrawSurface objects with release less than 7. So, there's probably some more coding work to do here.
Good, it had to be done anyway, and it's good to have a sample for testing, BTW including full sources too!

#### gho — 2023-03-18

This one is better, I added the missing hooks and now drempel.exe can run with no errors, though there's still a big problem.
drempel renders using a YUVU codec (or, in alternative, a YUY2) and all I can see is a green window. I think that this depends on a missing HW codec support, while the SW support is provided by DxWnd but only in certain conditions. I'll have to investigate and understand how to add these codecs support in the right places.
Anyway, here is the updated dxwnd.dll, who knows if anyone could have a better luck.

#### BEEN_Nath_58 — 2023-03-18

It worked somewhat. I see the green screen. But there were traces of seconds, WHERE the actual overlay is playing. I tried it on my Intel machine where it works well.

The second issue is the demo is supposed to stop when mouse is mouved. It doesn't. It does natively.

Third it makes my desktop wallpaper pink.
Fourth, other demos were unaffected. Except mosquito which doesn't work properly, no other overlay runs. I want to go bck to the time when all overlays tested were woeking (except Godfather)

Fifth, maybe you missed it, but there is another overlay to test. ddoverlay.exe in the same folder as mosquito.exe

*Last edit: BEEN_Nath_58 2023-03-18*

#### BEEN_Nath_58 — 2023-03-18

On overlay supported systems, (VMware WinXP) the result is conflicting:
* Mosquito flies but not wings. YES I am on .93!
* Mosquito's massive background isn't there.
* Mosquito doesn't bounce up and down unlike Win11
* Flag - transparent does nothing.
* drempels.exe green screen has a slanted line across the screen (like a line of a broken glass)?
* OH, and a BSOD out of nowhere?
* Nascar Road Racing video with DxWnd even works without overlay emulation?

#### gho — 2023-03-18

> Nascar Road Racing video with DxWnd even works without overlay emulation?

Most games don't surrender if the overlay capability is not supported, they just blit on regular surfaces and the result is no worse than otherwise. The GodFather was probably one of the very few that didn't handle the capability this way (very silly thing for the game distribution!)

> OH, and a BSOD out of nowhere?
Can't be my fault. I wish I could do that out of a user program, I'd be a rich hacker threatening the whole world.

> drempels.exe green screen has a slanted line across the screen (like a line of a broken glass)?

My guess is that there could be a memory surface whose pitch is not an exact multiple of the line size. Usually this is not the case, so please keep it replicable because this could be a bug difficult to reproduce but worth fixing

> Flag - transparent does nothing.
Where? On mosquito or drempels or whatever?

#### gho — 2023-03-18

This screenshot of the surface dumps is interesting: it shows how the drempels program builds nicely colored patterns that gets flattened to a green surface probably only in the last step when blitting to the primary surface. So, it's something that suggests how I will have to fix it.
P.s. the drempels settings by default propose the program as a default screensaver, so it's not too unexpected that it may interfere with other screensavers, especially nowadays when screensavers are proposed and updated automatically by Windows.
To avoid this risk I configured the program to avoid being a screensaver, then I run it directly. To change the configuration you have to use the configuration link or add the "/c" flag to the argument list. The picture shows my configuration.

Attachment: config.png

#### BEEN_Nath_58 — 2023-03-18

Just noticed, if you run the demo with "Force Windowing", you will get an actual window with just 1 cross button. Pressing that, the demo will end and the wallpaper will return. Certainly better than just ending the task (but I would like it to end when mouse moves, whenever that comes in DxWnd)
Another thing: games that need overlay permanently keep their overlay surface with the game. The issue comes when you scale the window, that the overlay surface doesn't scale. It just happened that Nascar Rev kept the overlay on exit and I, by any means wasn't able to remove it and had to restart PC.

> Where? On mosquito or drempels or whatever?

Mosquito...

> so please keep it replicable because this could be a bug difficult to reproduce but worth fixing
Okay. Maybe this will be replicable in every XP VMware.

> Most games don't surrender if the overlay capability is not supported,

Both Nascar games blit the video without Overlay. However DxWnd isn't able to do that in Nascar Rev. In Road Racing, there is a pink square and the video covers the proper rectangle and blinks.

NOTE: Please bring the mosquito demo on top of everything, instead of the dialog. We already verified it in the other threads!!!

*Last edit: BEEN_Nath_58 2023-03-18*

#### BEEN_Nath_58 — 2023-03-18

Interestingly, look at the pop up corner at the lower right. In that section, there a Dr. icon and if you right click, it shows the desktop moving there!

*Last edit: BEEN_Nath_58 2023-03-18*

#### huh — 2023-03-18

I tried this drempels and dxwnd.drempel.rar here in Win7. I intentionally set a smaller resolution than I have through drempels.exe. It works strangely. Either I have a green screen or I see a suspended effect when I click on the Dr icon in tray. Something prevents animation here, but I don't know what.

#### gho — 2023-03-18

That's because the dxwnd logic is still to be fixed. I wrapped the UpdateOverlay method on all ddraw versions and this eliminates the error, but now I have to fix the Flip logic ... W.I.P. now.

#### gho — 2023-03-18

Oh, happiness! I'm not posting any screenshot here because you have to see this MOVING!!!
fixed dll here.

#### huh — 2023-03-18

Well, unfortunately I can't see him, nothing has changed,

#### BEEN_Nath_58 — 2023-03-18

ITS WORKING!

Howeber it Freeze if I move my mouse and avain unfreezes sometime later...

#### BEEN_Nath_58 — 2023-03-18

Hey hey. Whatever voodoo magic that you did with mosquiro, keep it like that. The Mosquito is perfect!!!

#### huh — 2023-03-18

OK, so it probably only works in Win10/11, seems...

#### BEEN_Nath_58 — 2023-03-18

Well it works weirdly. I have Force windowing enabled, when I move the window away from overlay, it works.

#### huh — 2023-03-18

Oh! I enabled logging and it started working. Maybe there is a little bug....

#### BEEN_Nath_58 — 2023-03-18

Just ran on Win7 with Win11 profile, works even better. There no pause or freeze. Have you tried as I told?

#### huh — 2023-03-18

I don't know what you mean, here it only works with Overwrite-Debug-DxWnd hacks flags. There must be some bug.
Even if I have None-Debug-DxWnd hacks flags set it works. Otherwise, no.

*Last edit: huh 2023-03-18*

#### BEEN_Nath_58 — 2023-03-18

I meant to enable Force windowing, and don't notify on task switxh...

Attachment: dxwnd.drempel.rar
Attachment: drempels.log
Attachment: mosquito.log
Attachment: nascarrev.log
Attachment: nascarroadracing.log
Attachment: config.png
Attachment: dump.png
Attachment: drempelsdesktop.png
Attachment: drempelsscr.png
Attachment: WIN11drempelsdesktop.png
Attachment: dxwnd.drempel2.rar

### Page 2

#### huh — 2023-03-18

No, that doesn't work here.
As I wrote, the only thing that works here is enabling the Logs-"Debug"+"DxWnd hacks" flags and it doesn't matter if the None or Overwrite flag is on.

*Last edit: huh 2023-03-18*

#### gho — 2023-03-18

@Huh: can you send some logs of the faulty behavior? I know that this may sound difficult because getting the logs seem to fix the problem, but maybe you can find a way ...

#### huh — 2023-03-18

Here are two logs. DxWnd hacks flags only and DxWnd hacks + Debug flag where it works.
Do you see any difference there?

Update:
Full log without debug flag.

*Last edit: huh 2023-03-18*

#### gho — 2023-03-18

It's quite strange. In the faulty log there is this error:
IDirectDrawSurface::Flip: StretchBlt ERROR err=0 285
but the problem is that err=0 means there is no error code. Another oddity is that, according to the log, this error happens only after a few frames. Does this correspond with what you see? There should be a short period of time when the program works, then after a while the image should stop.
Last consideration: the error refers to a StretchBlt operation and this should depend on the stretching ratio. You could try to run the program with different window sizes and see if this makes a difference. Perhaps, were you trying to stretch the window when the problem happened?
A little off-topic: did you know that you could add a 256x256 jpg image in the c:\Programs Files(x86)\Dreampels folder and have your custom savescreen image? In the following screenshot can you see myself while I knead the dough?

*Last edit: gho 2023-03-18*

#### huh — 2023-03-18

Not anything like that. I didn't do stretching window.
It works for half a second when I tap on Dr icon in the tray to exit the program.
With the flag Logs-"Debug"+"DxWnd hacks" it works fine all the time.

Update:
No difference with a 640x480 window.

> did you know that you could add a 256x256 jpg imag

Yes, it's in the description.

> In the following screenshot can you see myself while I knead the dough?

:-)

*Last edit: huh 2023-03-18*

#### gho — 2023-03-18

I'm making blind guesses. Maybe it's a timing problem, the more logs can slow the program and make it work? You could try setting a small FPS delay ... though I'm not sure that the overlay code has the FPS control.

#### huh — 2023-03-18

I set the Timing Limit to 10 Hz. No difference. Logs are small it won't be this case.
The only difference is the Logs-Debug flag, but I don't know why.

I noticed that there is a missing line in both logs without the Debug flag
CreateWindowExA: ActiveMovie=0

Sorry I have to go to bed it's too late for me.

*Last edit: huh 2023-03-18*

#### BEEN_Nath_58 — 2023-03-18

VM runs the demo best dor some reason. Also his behaviour looks like the old one. @huh2 when you used Force windowing, did you get green screen or white DxWns overlay. It should be the latter.

*Last edit: BEEN_Nath_58 2023-03-18*

#### gho — 2023-03-18

Goodnight.
Don't bother too much about this experiment results. The program seems cursed because after I cleaned up some mess (just deleting some useless log instructions) the result changed dramatically, I got the green screen again and I can't fix it. There's something that doesn't tick ....
Tomorrow I'll try to understand.

#### huh — 2023-03-19

@BEEN_Nath_58
When I used Force windowing the screen was gray.
As I wrote, if I right-click the Dr icon in the tray, the scene moves for a moment.

Update:
The behavior of DxWnd hacks + Debug flags has not changed in dxwnd.drempel.rar version (it doesn't work).
The change came only in version dxwnd.drempel2.rar.

*Last edit: huh 2023-03-19*

#### BEEN_Nath_58 — 2023-03-19

Ok so the results are very different.

In fact I am having more problems today than tomorrow. It is interesting how it works the best on a VM.

#### BEEN_Nath_58 — 2023-03-19

I think I uncovered somethings. The reason why gho has been having inconsistent developments is beacuse overlay supported systems and unsupported systems are behaving differnetly with DxWnd.

Here I tested XP VM and mosquito demo can't remove that black square moving box. And the wings don't work either.

On my Intel gpu machine with Win10, the same phenomenon happened until I deleted the registry key required for overlay support on Win8+.

#### gho — 2023-03-19

I got a flaw in the overlay logic from my debug logs, but it's damned complex and I don't think I'll have time to fix it today. Please, stop the tests here, I have to make my mind with calm to avoid making a mess.

#### BEEN_Nath_58 — 2023-03-19

See my last post..

#### BEEN_Nath_58 — 2023-03-19

OFFTOPIC, for all members watching here (sighs)...

*Last edit: BEEN_Nath_58 2023-03-19*

#### gho — 2023-03-19

Oh my! I didn't know I had a lady Lulu Jane as a teammate! Please, post us a photo ... ;)

#### huh — 2023-03-19

@BEEN_Nath_58
How old is that post? Because UCyborg hasn't been here for a very very long time...
Lulu_Jane and Lowenz? They must be some undercover operatives that Gho hid from us, maybe they do all the dirty work for him hahaha :-)

#### BEEN_Nath_58 — 2023-03-19

Most mysteriously we have been hid from the majn developer: GH.
And I am not sure why they pulled data of Vogons' influential members here (Dege, lowenz and Ucyborg) . And who is Lulu_Jane, I dont even remember seeing them

(So basically ChatGPT told me how DxWnd can achieve 8-bit paletted texture ans overlay emulayion and it looked quite authentic...)

*Last edit: BEEN_Nath_58 2023-03-19*

#### gho — 2023-03-19

Some mild progress ... I rebuilt a dxwnd.dll version that works on my computer, but ready to do crazy things as soon as I add or cut some log lines. Not a satisfactory result, so far.
In addition I downloaded the DX6 mosquito.exe and in effect, though the result should be the same, it behaves in quite a different way. In particular, the mosquito is moved with SetOverlayPosition and the wings change position with a Flip call, but when you flip the mosquito becomes huge, I believe this is one of the reported errors.
I have also Nascar Revolution in my testbed, but I remember having seen the intro movie that now is not visible any more neither on overlay surface nor on plain surface. Odd.
It seems that there's still much work to do.

#### gho — 2023-03-21

I am sorry for my slow progress here, mainly due to a sudden peak of real work activity. Hopefully as soon as I will deliver my working program I'll be back on normal speed.
On this overlay topic, it is interesting to note the differences between the mosquitoes (the two versions for SDK6 and SDK7) and Drempels. In particular, there seems to be two set of calls that do similar things in different ways:
* SetOverlayPosition (and its reverse GetOverlayPosition) that use x,y coordinates only to determine the overlay position for a 1:1 flipping
* UpdateOverlay that accepts a RECT structure and therefore seems to indicate the possibility of scaling the flipped surface.
Of course, Microsoft seem particularly shy about all these methods because the documentation is progressively fading away from all web pages! Fortunately the SDK are still available. At the moment I'm trying to arrange the DxWnd code so that it could handle both overlay styles.

#### BEEN_Nath_58 — 2023-03-21

Don't worry, I have been busy as well, and will be till the end of March.

While you are working on mosquito, note that DDOVERLAY. EXE also has a dx6 and dx7 version. Although it is a simple overlay, it can. lay some of the basics for other overlay features.

And lastly, if you are interested there's another overlay named DMOVIE.EXE that plays avi files on desktop screen. This can either be very easy, or a long way to go, but it is worth mentioning!

#### gho — 2023-03-21

Wonderful!
Finally this release can manage all two mosquitoes and drempels. And, more than this, it showed some flaws that probably could affect other flipchain logics as well. It has to be polished and optimized, but this one works! I'll put it in the .rc section.

#### BEEN_Nath_58 — 2023-03-21

That works so much better!!!

Probably you can fix the current issues:

* Mosquito wings are a LITTLE FASTER.
* Nascar Revolution is back! However, the video blits between actual video and a green screen.
* Nascar Road Racing video is still not there (it was there in earlier DxWnd).

*Last edit: BEEN_Nath_58 2023-03-21*

#### huh — 2023-03-21

I can confirm that Drempels and overlaydemo is now also working here. Perfect!
Mosquito hasn't changed (known transparency issues in Win7).

#### BEEN_Nath_58 — 2023-03-21

overlaydemo? weid it still doesn't launch here
Attachment: drempel.zip
Attachment: full.7z
Attachment: custom.png
Attachment: drempels.jpg
Attachment: chatgptdxwnd.png

### Page 3

#### gho — 22 March 2023

Here is a precious and rare picture telling what should happen in the flipchain after a `Flip(0)` operation. Still, I have some doubts about what should happen when flipping with a specific surface, like this supposed case:

```text
a=CreateSurface(OVERLAY, backbuffercount=2);
b=GetAttachedSurface(a);
c=GetAttachedSurface(b);
a->Flip(c);
```

Attachment: [ddfig9.png](https://sourceforge.net/p/dxwnd/discussion/general/thread/566ddb1947/dc96/attachment/ddfig9.png)

#### gho — 22 March 2023

This is a custom-made version of SDK7 `mosquito,exe` where the 3 wings positions were marked by a progressive number (1, 2, 3) and a red circle in 3 different positions.

According to the Flip schema I would expect to see a sequence of 1, 2, 3, 1, 2, 3 ... but there seems to be a prevalence of 1, like 1, 2, 1, 3, 1, 3, 1, 3 ... and in some DxWnd versions the sequence seems broken like 1, 2, 1, 2, 1, 2 ...

It would be interesting to see how this program behaves on a real condition, maybe on a XP or Win7, but maybe before I'll have to fix the FPS delay setting on overlay surfaces.

Attachment: [mosquito.rar](https://sourceforge.net/p/dxwnd/discussion/general/thread/566ddb1947/da72/attachment/mosquito.rar)

#### gho — 22 March 2023

And this is another dxwnd.dll signed as .rc7 (though it may be more experimental than the previous one) where there is a preliminary FPS control on overlay operations.

I use this one to slow the mosquito programs and try to see if the wings sequence makes sense.

It is untested on the nascar games, so maybe it broke something ...

oops, I forgot the flip tracing on, this release will be more verbose, but maybe that's not too bad.

*Last edit: gho 2023-03-22*

Attachment: [dxwnd.rc7.rar](https://sourceforge.net/p/dxwnd/discussion/general/thread/566ddb1947/5e38/attachment/dxwnd.rc7.rar)

#### huh — 22 March 2023

> It would be interesting to see how this program behaves on a real condition, maybe on a XP or Win7

OK, I recorded this on one Win7 computer via Fraps.
I don't know if that's enough.

Update:
WinXP.

*Last edit: huh 2023-03-22*

Attachments:

- [mosquito 2023-03-22 10-41-16-32.avi](https://sourceforge.net/p/dxwnd/discussion/general/thread/566ddb1947/5e38/4f5f/attachment/mosquito%202023-03-22%2010-41-16-32.avi)
- [mosquitoxp.avi](https://sourceforge.net/p/dxwnd/discussion/general/thread/566ddb1947/5e38/4f5f/attachment/mosquitoxp.avi)

#### gho — 22 March 2023

Thank you very much. As I feared, it's clearly visible that the sequence is 1,2,3,1,2,3 as I supposed from the dxwnd log and some guess on the program purpose.

This means that something still doesn't tick in my code! Damn ...

#### gho — 22 March 2023

`.rc8:`

Fly, mosquito, fly!!

Attachment: [dxwnd.rc8.rar](https://sourceforge.net/p/dxwnd/discussion/general/thread/566ddb1947/fead/attachment/dxwnd.rc8.rar)

#### BEEN_Nath_58 — 22 March 2023

Great, it works!

Back to the issues:

- For Nascar Revolution, real fullscreen game: **game is stuck in a black screen.** For some idea on what is happening, in windowed mode, after the video ends, **the game creates the game main surface on the DxWnd overlay surface as well as the other main surface,** It can be verified by the fact that in windowed mode, once the video ends, you can move the game window and there will be 2 windows. In certain cases one will say “DxWnd overlay” (if you moved the window at a perfect time).
- Nascar Road Racing, windowed mode: Video **screen is pink.**
- Nascar Road Racing, real fullscreen mode: After video, **game screen is black.** Luckily I caught an instance where it said “DxWnd overlay”.
- Mosquito is **missing in XP IF “transparent” is enabled (why have black square?)**

Edit: I verified problem-4 of XP, and now the mosquito wings don't fly again, and black border doesn't go with transparent.

Edit-2: Restarted XP, the mosquito goes missing again as told. Something is werid.

*Last edit: BEEN_Nath_58 2023-03-22*

#### huh — 22 March 2023

@gho

Ok, I confirm that mosquito works the same here in Win7 as in the previous version, so I can say that this version didn't break anything here :-)

@BEEN_Nath_58

Transparency doesn't work in Win7 either here, but it's not a new issue, gho is aware of this situation.

#### gho — 23 March 2023

Back from the release candidate thread:

The results on XP and Win7 are somehow expected with one single exception: on my portable, Win7 with 32bit desktop the mosquito was perfect. I will repeat the test, you never know, maybe the computer was using actual overlays instead of emulation.

One prayer: whenever you do reports on Win7 or XP please specify the desktop color depth, it makes a difference.

Anyway, the 16bit bug was a nasty one (bad pointer usage, I was overwriting the stack so that it was impossible to understand anything later) and we're getting closer.

Just one note about Nascar Road Racing: the logs tell that the operations are done correctly, I have the suspect that we have a variation of this Nascar oddity: nothing is visible until you move or stretch the window, like in Nascar Revolution, but with the difference that as soon as you trigger the visibility then the game immediatelyy stops the movie and shows the main menu screen. If this is true, the porblem is not in overlay handling but it is somewhere else. In effect, the intro movie is not visible also turning overlay emulation off.

#### BEEN_Nath_58 — 23 March 2023

for mosquito i did the same thing on xp and win7.

32 bit desktop: no mosquito on both OS
16 bit desktop: no mosquito on BOTH OS.

Nascar Road racing overlay, I think I mentioned the version where it worked correctly, probably in the DXSDK thread or GodFather thread. Maybe it's a recession or something new because of overlay change.

Another thing, I saw drempels run significantly slow now

#### gho — 23 March 2023

> Another thing, I saw drempels run significantly slow now

That could depend on the insertion of vSync and FPS controls. There are 3 operations that could be used to change the overlay content:

- Flip
- SetOverlayPosition
- UpdateOverlay

Unfortunately, Flip is the only method where you can specify a vSync option, so it is arguable if the screen updates made by the other two methods should follow any vSync synchronization or not. I inserted in the wrapper the vSync and FPS emulation, so you can control the overlay speed by forcing the vSync options ON or OFF or increase the delay with the FPS limit delay. Probably, setting vSync OFF should bring Drempel to the initial speed, but I don't know which one is the correct one.

#### gho — 23 March 2023

Wow, I got something about Nascar Road racing: the surface pixel format is wrong, it has no color specification, so it's not strange it renders in black. Later I'll try a fix, but I still have to make my mind about how ...

#### gho — 23 March 2023

Here is the surface dumps of the Nascar Road Racing intro sequence. Despite the black screen, it is possible to clearly see the EA logo. This means that the overlay logic is correct, there must be something else ...

Attachment: [nrr.png](https://sourceforge.net/p/dxwnd/discussion/general/thread/566ddb1947/f4ad/attachment/nrr.png)

#### gho — 23 March 2023

An interesting discovery: look at what happens in Nascar Road Racing when setting the clipper option to ON (or, pretty similar result, if you use the GDI renderer). The intro movie becomes partially visible and the overlay window is not cleared. I don't know why, but it seems likely that the secret to fix the game is fiddling with some clipper option.

#### BEEN_Nath_58 — 24 March 2023

I recall if you disable the overlay emulation, ans let the videi run without it, there will. be a flicker between pink and ea video.

on real win7 this didn't happen, at least for me

#### BEEN_Nath_58 — 24 March 2023

Ok the SW renderer is intensive. I wonder if you can make it use the CPU better.

Btw, how do you make the overlay stay on top.

*Last edit: BEEN_Nath_58 2023-03-24*

#### BEEN_Nath_58 — 24 March 2023

Unreported problem: Since you are already on Nascar RRacing, there is another issue. When you start a Race, a SuperMike window opens. It becomes black and window needs to be resized, repalced to have it visible

#### gho — 25 March 2023

**HURRAY!!**

After so may errors and fixes in the mosquito handling, the fix that updates the wings pictures in the proper order produced the desired effect: applying the very same logic to the primary flipchain of a game with double backbuffer, the sequence is now correct ... what does that mean? As I hoped from the very beginning, now I can play “Robin Hood - The Legend of Sherwood” turning the “Compensate Flip emulation” off and with a perfect rendering, no more cursor trails on the screen!

This not only dramatically improves the quality, but also bypasses the CPU consuming logic for the Flip error compensation.

For me, recalling how much effort and frustration I got from these cursor trails, this is a wonderful result!

Attachment: [robinhood.png](https://sourceforge.net/p/dxwnd/discussion/general/thread/566ddb1947/e450/attachment/robinhood.png)

#### huh — 25 March 2023

Seems, that it also fixed mouse tracks in Rainbow Six - Rogue Spear Demo...

#### BEEN_Nath_58 — 25 March 2023

I am not at home now, can anybody test if Motorhead video changed?

#### gho — 25 March 2023

Also “Blupi at Home” was fixed with the new flipchain handling. I know it's not a fundamental game, but it was the first one that came available and already installed.

The ExpFinder +NOFLIPEMULATION command should provide a list of all games currently configured with that flag.

I got instead problems with Rainbow Six, I don't know if because of the flipchain or some other weird experiment, but we'll have to be cautious before releasing this: maybe the flipchain has some trouble with D3D integration?

#### BEEN_Nath_58 — 25 March 2023

Forget Motorhead video, Rogue Spear D3D became the same as Rogue Spear DDraw

#### gho — 25 March 2023

Well, I think I know why ...

#### gho — 25 March 2023

Ok, the problem is now the overlay has a correct flipchain handling, while the normal surfaces don't, and aligning the remaining code to the new policy will require some delicate interventions. It will be necessary to change the Blt and BltFast wrapper and the sBlt common routine, so it will take a while and some calm.

I am sorry for the delay, but the dramatic positive effect on some simpler games (Robin Hood, Blupi) make me certain that it is worth the trouble. 👍

#### gho — 25 March 2023

The problem is serious. I think that in short this is the reason: the flipchain handling made by DxWnd implies that the surfaces in the flipchain are replaced in a cyrcular way. But when you build a D3D device (mind you: only from D3D1 to D3D7!) you have to pass a surface handle that will match the surface where you drop some 2D stuff. If the surface doesn't match with the shifted surface in the flipchain, the toy breaks! I have to find some kind of magic to fix that!

### Page 4

#### gho — 2023-03-27

In the attempt to consolidate the flipchain handling, at least for overlay surfaces, I wanted to test more, so I modified the mosquito SDK7 just a little to handle a variable number of backbuffers in the flipchain, from 1 to 6.
If you want to play with that, it takes this rebuilt mosquito and, for a good emulation, the attached dxwnd.dll. I numbered all backbuffers, so it is easy to check if the emulation shows the correct sequence.
To set a custom number of backbuffers, add a numeric argument, like "mosquito.exe 4".

*Last edit: gho 2023-03-27*

#### BEEN_Nath_58 — 2023-03-27

Umm so what are we onto now... what do I check?

#### huh — 2023-03-27

@gho
If it is correct that the numbers on the mosquito's chest are consecutive (1-2-3-4-5-6) it is OK here.
Update:
Legal Crime very blinking with this version. To be fair, it also flashes with version 2.05.94.rc12.
With 2.05.93 it doesn't.

*Last edit: huh 2023-03-27*

#### gho — 2023-03-28

Finally I got a dxwnd release that works as I wanted to, cycling the flipchain surfaces to provide a smooth and more efficient page flipping. But now I pointed out THE problem:
emulating the flipchain means that each time you reference a surface in the flipchain you get the reference to another surface, according to the flipped operations. If the flip compensation is made in both the write and read operations (or blit to and blit from, if you prefer) everything works perfectly.
The trouble comes when you open a Direct3DDevice using one surface in the flipchain. For instance, "Rogue Spear" uses the backbuffer.
If the surface is also flipped, it may happen that the CreateDevice receives one surface reference while next operations will receive the reference of another flipped surface.Since Direct3D1-7 doesn't let you know whether you are blitting to a surface that will be used by Direct3D or DirectDraw, you can't know whether it is necessary to remap that surface in the flipchain or not.
In that case, the old schema that had only one backbuffer to be copied to the primary surface was less efficient, but safer.
So, basically, it seems that we have now two flipchain schemas:
1) the original one, less efficient but good with Direct3D1-7, though unable to handle flipchains with multiple backbuffers (see "Robin Hood")
2) this new one, more efficient and handling perfectly the multi-backbuffer situations (see also "Mosquito"), but not good for Direct3D1-7 games.
Life is complicated ....

#### dippy dipper — 2023-03-28

Well with this dxwnd.wip14.rar version RogueSpear almost got back its D3D rendering but the screen is flickering wildly between normal rendering and a black screen. Also now RogueSpear mouse trails can not be removed even with the Compensate Flip emulation flag.
Edit:
I mentioned that the mouse stutter was still present but testing more it does not seem so afterall. The screen was just flashing so fast that there was an optical illusion.

*Last edit: dippy dipper 2023-03-28*

#### gho — 2023-03-28

I fear it's not so simple. Rogue Spear opens a primary surface + 1 backbuffer, then uses these two surfaces for the intro movies, but also connects the backbuffer to the Direct3DDevice. When the movies are over, the game draws the cursor by blitting (with Blt) the cursor sprite to the backbuffer. Since the backbuffer is connected to the Direct3DDevice, the backbuffer should be sent to screen when the 3D frame is completed.
But the problem is that during the movies the primary/backbuffer surfaces are swapped a certain number of times. If that number is even or odd means that the surface for the Diredt3DDevice is right or wrong, and this can't be controlled since pressing the ESC key you interrupt the movie at a certain number of frames.
So, what I would expect (and somehow I saw) is that on repeated runs the game cursor may show correctly or not at all, depending on a 50% of chances. Awful!

#### dippy dipper — 2023-03-28

Warhammer: Rites of War is broken again (see screenshot).

#### dippy dipper — 2023-03-28

Driver also flickers like crazy now but I guess you already know that.

#### BEEN_Nath_58 — 2023-03-28

Let me make a guess. Things didn't break in the "no need For Compensafe Flip" situation, but rather the time when gho probably said he fixed a problem in flip chain (.rc10?)

Why not undo the changes made there? I never saw an error in practice, or in any app or game.

#### gho — 2023-03-28

Undoing everything is certainly a solution, but I hate the idea of surrender too early.
You should not judge the path by the current situation, certainly making a radical change is expected to cause instability for a while, but the final result may be worth the trouble.
By the way, I am also testing a third approach, this one again with benefits and limitations, but who knows? The idea is this one:
Simulate the flip operation by getting and swapping the pointers to the surface buffers. After all, this is exactly what the Swap operation does, but you can do this also using GetSurfaceDesc + SetSurfaceDesc.
It seems to work, but SetSurfaceDesc is available only from DirectDraw version 3 and greater.

#### BEEN_Nath_58 — 2023-03-29

I didn't mean everything. It doesn't look like "everything" broke "everything". It's just 1 thing that you thought should have been fixed, that broke it.

SetSurfaceDesc is an option, but I have a lot of games (I opened thread but your Intel driver didn't agree) that would want DxWnd to fix them, in DDraw1

#### gho — 2023-03-30

Maybe I'm a little stubborn, but I'm still exploring these Flip options.
Yesterday I wrote some complex routine (I mean, complex enough to give me a headache) that flips a flipchain by making a loop of Lock/copy content/unlock of the surface buffers.
This way is not as efficient as exchanging the pointers with SetSurfaceDesc, but it is absolutely portable from ddraw1 to ddraw7, so it could be a basic option, maybe still perfectible.
Now the big problem is to get rid of the incredible mess that I made everywhere in the code and maybe restart from scratch. Yesterday night I tried this experiment by applying this new flip schema to the .rc11 release (the older, the better?) and it worked far better than the last ones, but with a few surprises:
1) Rogue Spear shows the intro movies but doesn't show the cursor sprite moving into the screen. But since now I'm not flipping te surface handles, the reason can't be the one I supposed, so there must be something else. Of course, if the reason is another, it is also possible that once fixed also the handle flipping method could work, who knows?
2) Robin Hood now works again with no cursor trails, but unexpectedly the logs show that the game uses a single backbuffer, so the trails reason is not the missing handling of a 3 backbuffer surfaces!
So, please be a little more patient, I can't drop all this now!

#### dippy dipper — 2023-03-30

> Robin Hood now works again with no cursor trails, but unexpectedly the logs show that the game uses a single backbuffer, so the trails reason is not the missing handling of a 3 backbuffer surfaces!
I think the mouse trails got fixed with .rc8.
So you could compare .rc6 changes to .rc8 in order to pinpoint what did the trick:

#### gho — 2023-03-30

I dropped source and dll in the .rc section as .rc16.
That one is in reality a .rc9 with some modifications made in the later releases and some final fix to test the surface rotation. In the end, there seems to be three possible strategies to handle the page flipping:
1) rotate the surface handles (no good for D3D)
2) rotate the buffer memory pointers (unsupported for ddraw1 and 2 and not working so far)
3) rotate the memory buffers content (in this release).
It could be noted that rotating the buffer contents (with Lock/memcpy/Unlock) is not so different from the old schema that used Lock/Blt/Unlock, but in this release it is generalized for a number of backbuffers also greater than 1.
But the interesting thing is this (see the picture): it seems that the overlay and primary flipchains should be handled differently. The mosquito testcase demonstrates that the contents should be rotated cycling all surface contents, and this was somehow expected.
The news instead is that the normal flipchain (that I call primary flipchain for clarity) should not be handled this way, after a Flip operation the content of the video surface is lost and the last backbuffer is copied but also remains unaltered. At least, this was the only way to make the video show something, because applying the circular schema the window remained black.
It is odd, but in effect there are some comments in the ddraw web pages that suggest that this could be true.

#### gho — 2023-04-01

work still in progress, have faith ...

#### gho — 2023-04-01

Posted in .rc thread:
> This one works like a charm, but it is still perfectible for better performances.
> Note: the "Compensate Flip emulation" flag is still valid and should be set for more accurate behavior (in practice, it works like before ...).
> Though I tested it on a restricted testbed, it works very well and it is the first release ever that cancels the mouse trails in "Rainbow Six" the 1998 original game!!!

#### gho — 2023-04-02

There are two problems (at least):
"Warhammer 40.000 Rites of War demo" has a nasty one: the exe creates two flipchains, one for the intro panels and one for the game itself, and they are overlapped (like create 1; create 2; use 2; use 1) so when drawing the game screens DxWnd considers the wrong flipchain.
The problem is severe because in the current schema there is only one primary flipchain, so the data that should be used are overwritten and no longer valid! The only solution would be to make the flipchain descriptor dynamic and link each one to its relative primary surface. It can be done, but damn, just when I thought the work was done ...
BTW I'm not even sure that the full game has the same problem.
"Driver" is a puzzling one: the logs tell that everything is ok, but the 3D screens are striped ...

#### gho — 2023-04-02

I got a solution for the Warhammer 40K problem. The second primary surface was created with no FLIP or BACKBUFFERS capability, so the fix is to condition the creation of a flipchain to the effective need. In effect, two primary flipchains should not exist at the same time, or not?
The fix is posted as .wip21 in the now crowded .rc thread, at the moment there remains only the problem on "Driver", hopefully I will catch that one as well.

My current testbed for flip operation is this one:
* Tomb Raider III the lost artifact
* Driver
* Mosquito
* Warhammer 40.000 Rites of War demo
* Silver
* Rainbow Six Rogue Spear
* Rainbow Six (1998 edition)
* Robin Hood the Legend of Sherwood
* Dungeon Keeper Gold
* Luftwaffe Commander

Feel free to add some more ...

*Last edit: gho 2023-04-02*

#### gho — 2023-04-02

I think I now understood what's wrong with "Driver".
Unlike most other games, Driver doesn't build a primary surface with n backbuffers, but it builds a naked primary surface and after that it builds a backbuffer surface to be attached to the primary.
So, the pseudo-coding is not this:
lpPrim=lpDD->CreateSurface(DDSCAPS_PRIMARY, BackBuffers=1);
but rather this one:
lpPrim=lpDD->CreateSurface(DDSCAPS_PRIMARY);
lpBack=lpDD->CreateSurface(DDSCAPS_BACKBUFFER);
lpPrim_>AttachSurface(lpBack);
Unfortunately, the idea of swapping the buffer pointers or contents works only if the surfaces have the same characteristics, like the same pitch and so forth. If you manage COMPLAX surfaces, likely the pitch don't match and instead of copying the whole buffer you should copy one line at a time. Or, instead, use a Blt operation to do that automatically, that is pretty much what DxWnd was doing before!
So, now the next step will be to create a generic n-backbuffer flipchain management based on the old schema, then It will be possible to select the optimal schema according to the conditions.

#### BEEN_Nath_58 — 2023-04-02

Rogue Spear is facing what's the common "slanted text" in Midtown Madness, but its more severe. (And no, the setting that fixes thing in MM doesn't do that here, instead makes thing worse)

*Last edit: BEEN_Nath_58 2023-04-02*

#### gho — 2023-04-05

Ok, I think I can now close this experimental phase.
Unfortunately, the results were much below my expectations, but at least now I know DxWnd is doing its best.
To recap, I tried several ways to reimplement the Flip operation in a flipchain. Here is a summary:
1) Blit surface contents with the Blt method. It was the original method and still the best one. It works.
2) copy surface contents by getting the dwSurface pointer and copying the buffers with memcpy: it works unless the surfaces pixel formats or pitch are different, which may seldom happen. It caused the slanted picture in Rogue Spear. It is arguable more efficient than making a Blt operations, so it doesn't seem worth taking the risk.
3) swap the dwSurface pointers with SetSurfaceDesc: here <https://learn.microsoft.com/en-us/windows/win32/api/ddraw/nf-ddraw-idirectdrawsurface7-setsurfacedesc> is explained that ddraw would free the overwritten buffer, which would make it unavailable for the next circular swaps. It can't work. In addition, SetSurfaceDesc is not available on ddraw version 1 and 2.
4) use the Flip operation between surface couples: it doesn't work if the surfaces are not in a real flipchain.
5) swap surface handles: it may work with limitations (for instance, when the surface in the flipchain us used as a reference surface in a D3D device) and it utterly complex requiring delicate changes all over the places. Not worth the risk.
So, in conclusion, what did we get?

1) A generic flip emulation schema working for both primary and overlay flipchains with n elements, where n can be greater than 2
2) The need to rename the "Compensate Flip emulation" with a better fitting name, like "Complete Flip emulation". What this flag did and is still doing is to complete the swap cycle by replacing the last element with the first in the chain.

I'll try to reorder all things and make a good .rc release (or maybe a final release) later.

#### huh — 2023-04-05

It's painful that after all that work you found out that only two models are functional.
Well, at least now we know which roads are dead ends.

#### gho — 2023-04-06

New interface here (and first step to v2.05.95): a radio button for Flip emulation, much easier to understand. I set the default as FULL, while before it was equivalent to PARTIAL (that for brevity in the flags has been renamed as HALF). Anyway, I'll repeat the supposed equivalence:
none = no flags
partial = Flip emulation
full = Flip emulation + Compensate Flip emulation
So, no need to update the export files, but of course, I'll have to update the help pages as well ...

*Last edit: gho 2023-04-06*

#### gho — 2023-04-06

Flipping pain is not over yet. I was testing "Braveheart" that has some invisible cursor, but also another problem: the full flipping emulation is terribly flickering. The partial (HALF) mode is much better. Evidently there is still something to fix for this case. Fortunately in Braveheart the partial Flip is perfectly fine, so there is no hurry.

#### BEEN_Nath_58 — 2023-04-06

If you tested dgVoodoo you'll see that dgVoodoo2 s Flp/Blt (whatever it uses) behaves similar to Compensate flip emulation. I assume we are going towards betterment in general

Attachment: dxwnd.2.05.94.wip13.rar
Attachment: mosquito.rar
Attachment: dxwnd.wip14.rar
Attachment: WH40K_ROW.png
Attachment: flipchain.png
Attachment: wip1.rar
Attachment: menu.png

### Page 5

#### huh — 2023-04-07

@gho
I happened to find this page with source codes, we can use something from this or learn something? It's just a blind shot.

*Last edit: huh 2023-04-07*

#### gho — 2023-04-07

Sounds really interesting. It seems limited to D3D9 programs only, but also in this case it could be interesting. At a first glance it seems applicable more for diagnostic overlays (like the DxWnd FPS counter, just to make an example) but even it that case it could be useful.
Thanks.

#### BEEN_Nath_58 — 2023-04-11

Query: We have general flip emulation and thr full flip emulation enabled by Compensate Flip emulation. Where is the original compensate flip emulation now?

*Last edit: BEEN_Nath_58 2023-04-11*

#### gho — 2023-04-11

partial = former "Flip emulation"
full = former "Flip emulation" + "Compensate flip emulation"

#### BEEN_Nath_58 — 2023-04-13

Probably I missed some patch. DxWnd About says v2.05.94. I don't get the settings.

#### BEEN_Nath_58 — 2023-04-13

... (problem regarding flipping fixed because the game used GDI+DDraw)

*Last edit: BEEN_Nath_58 2023-04-13*

Attachment: dxwndset.png

### Capture limitations

All five pages are represented in chronological/page order. The web captures preserve the post text and attachment filenames; the page-3 manual capture supplies its direct attachment URLs. Binary/media contents are not included. Site navigation, login controls, reactions UI, and unrelated footer/recommendation content were removed.
