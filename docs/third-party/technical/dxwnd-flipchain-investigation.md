# DxWnd — Overlay and Flipchains Emulation

- Original title: “Overlay and Flipchains Emulation”
- Source: [DxWnd General Discussion, page 3 of 5](https://sourceforge.net/p/dxwnd/discussion/general/thread/566ddb1947/?page=2)
- Manual capture: `originals/technical__dxwnd-flipchain-investigation-manual.html` (canonical `?page=2`; the page identifies itself as Page 3 of 5)
- Authors: gho, huh, and BEEN_Nath_58
- Language / dates: English; 22–25 March 2023
- Availability: **Partial.** This capture preserves all 25 posts on page 3. The other four thread pages were not captured here, so this is not the complete thread.
- Checked: 2026-09-09

## Original text

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

### Capture limitations

This is a faithful conversion of the 25 captured posts on the mapped manual page. The capture does not include pages 1, 2, 4, or 5 of the thread, nor does it include the binary/media contents of the linked attachments. Attachment filenames and links are retained. Site navigation, login controls, reactions UI, and unrelated footer/recommendation content were removed.
