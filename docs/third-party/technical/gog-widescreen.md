# GOG forum: widescreen

- Source: [GOG forum — widescreen, page 1](https://www.gog.com/forum/robin_hood_legend_of_sherwood/widescreen/page1) and [page 2](https://www.gog.com/forum/robin_hood_legend_of_sherwood/widescreen/page2)
- Forum: Robin Hood Legend of Sherwood
- Language: English (with one brief non-English aside)
- Thread opened June 8, 2012; last post September 5, 2022
- Retrieved September 9, 2026

The original pages contain 29 posts numbered 1–15 and 17–30; no post 16 is present. This is the complete substantive post text, with repeated quoted copies omitted where the quoted post is transcribed separately. Technical values, links, and attribution are retained. Site navigation, account controls, ads, and other GOG chrome are removed.

## English translation

### Post 1 — gmx (June 8, 2012)

Widescreen support in game / max resolution - is gog version better then others? it's on sale now, nostalgic buy or not to buy problem

### Post 2 — wtan1 (June 9, 2012)

Maximum in game: 1024 x 768. If anyone knows how to get it to display in windowed mode or even better 1920 x 1080, please let me know...

### Post 3 — Benne (June 11, 2012)

Have not been able to run it windowed out of the box. However, FYI - by downloading free VMware Player and running it in a VM, I am able to simulate a windowed set-up. Just a thought.

### Post 4 — JavyC89 (July 18, 2012)

Hey.. Is there an easy way to play the game windowed?

I have a 1080 monitor and ir looks awful :(

### Post 5 — MrDOS (September 8, 2012)

I had some amount of success with [D3DWindower](http://www.neowin.net/forum/topic/603613-d3dwindower/), although I had to play with it a lot to make it work properly.

### Post 6 — Theruler (January 9, 2017)

Any chance to have the same widescreen patch made for desperados?

### Post 7 — Blinkin89 (January 9, 2017)

As far as I know D3DWindower is a general application, so you could try it with any (Direct3D) game you want.

### Post 8 — ZellSF (July 8, 2018; edited)

So I was very bored today and tried to figure this out. No the GoG version does not support widescreen. But the game does seem to accept widescreen if you specify it in the profile. This is a bit tricky though, you need to open `Robin Hood\DATA\Savegame\Profiles` with a hex editor and search for one of these:

```
20 44 00 00 F0 43
48 44 00 00 16 44
80 44 00 00 40 44
```

There might be multiple entries if you have multiple profiles, replace all of them. They are the three resolutions you can choose ingame (top is 640x480, middle is 800x600, bottom is 1024x768). I'm too stupid to figure out what the numbers mean but I've experimented and found some resolutions to try.

If you replace with `80 44 00 00 10 44` you get 1024x576. This is nice because the game apparently uses width to determine which UI to load and so you get a perfect UI in this resolution. Plus it's a resolution the game was "designed for", it isn't smaller or larger in width or height than any of the supported game resolutions. Obviously you need to add this as a custom resolution in your GPU driver and set scaling to GPU and not monitor as few monitors will accept this resolution.

If you replace with `A0 44 00 34 44` you get 1280x720 which I prefer for pixel-based scaling titles. Nothing is too tiny and it's just a nice resolution to play titles like this. Also it's optimal for scaling to 1440p monitors. This and other resolutions however makes the UI look bad.

If you replace it with `F0 44 00 00 87 44` you get 1920x1080. I think everything is way too tiny here, though the game does offer a blocky looking zoom function.

I don't know if widescreen breaks anything, I've only tested very basic functionality. This was the extent of my testing: [YouTube test video](https://www.youtube.com/watch?v=PTXB807T7JA)

### Post 9 — mbhtst (July 14, 2018; edit note “by snowdark”)

Thanks, it really helped!

### Post 10 — robip85 (July 14, 2018)

1280x720 is `A0 44 00 00 34 44`. As you said, it's best for 1440p monitor.

1024x576 doesn't work for me, even main menu is not shown correctly, just tiles and no text.

Can you please test if you can get 1600x900? I think 1600x900 would be nice to try, as 1920x1080 is borderline playable.

With higher resolutions you see more of map, but the HUD gets smaller, it is usable up to 900 height.

If you want widescreen, first thing is to aim to at least 768 height, as you could get that in original game 15 years ago, so no point in getting smaller. That means you need at least 1366x768.

### Post 11 — chrix (July 23, 2018; edited July 24, 2018)

You're not stupid at all.. you did a great finding.

I can't figure out myself what those numbers are.. can't find a logic in them (and in how they change at the different resolutions)... but.. hey: they work as you said!

Thank you very much for sharing this amazing finding with everyone..

(I have the game on Steam and it works there too! ).!!!

### Post 12 — ZellSF (July 24, 2018)

1024x576 works fine here. Even if it didn't I wouldn't have a clue how to fix it. This is really just altering the resolution to an entirely unsupported one and just hoping the game accepts it. For some reason Robin Hood doesn't mind too much. Chicago 1930 and Desperados just crash when doing the same.

At any rate:

```
C8 44 00 00 61 44 = 1600x900
AA 44 00 00 40 44 = 1360x768
```

### Post 13 — Gamesiarz (November 10, 2018)

Hello I have a little problem when I switch to 720p on my wqhd I can't move camera to the bottom of the screen, other sides top, left, right work fine. Any ideas?

### Post 14 — Gamesiarz (December 19, 2018; edited)

I think that line for 1280x720 is incomplete here because every other pair of strings have 2 more characters. Could you check this?

### Post 15 — ZellSF (December 20, 2018)

Camera moving to bottom works just fine for me, and yeah a typo:

```
A0 44 00 34 44
```

should be

```
A0 44 00 00 34 44
```

### Post 17 — tonik2000 (November 11, 2019; edited December 22, 2019)

Please 16:10 resolution code. Just one Please. 1280:800 maby or 1440:900. ThankU

Update: found myself - 1280 * 800 `A0 44 00 00 48 44`

### Post 18 — ShiroOukami (January 25, 2020)

When i change it to 1600x900 or 1920x1080 ambush missions have black background.

Do you know why is that maybe?

### Post 19 — MrDOS (March 5, 2020; edited June 16, 2020)

I tried to take another look at the values. My goal was to enable 960x600, so that I could use 2x integer scaling on a 1920x1200 monitor. The first half/three bytes clearly control width, and the second half the height. However, I still can't figure out a consistent pattern. For some vertical resolutions, the value of the middle byte appears to encode the difference between resolutions in 4-pixel steps. E.g., the difference between a vertical resolution of 720 and 600 is `0x34 - 0x16 = 0x1E = 30`. Dividing the difference in heights by that number shows that each change by 1 is worth 4 pixels: `(720 - 600) / 30 = 120 / 30 = 4`.

Based on that, we can re-derive some other heights; e.g., for 900 pixels: `900 - 600 = 300`; `300 / 4 = 75`; `0x16 + 75 = 0x16 + 0x4b = 0x61`, which matches the known-good pattern `00 61 44`. But this doesn't hold for higher resolutions: `1080 - 600 = 480`; `480 / 4 = 120`; `0x16 + 120 = 0x16 + 0x78 + 8e`, but the expected pattern is `00 87 44`. In fact, through trial and error, I found `00 88 44` to be a 1200-pixel height. Horizontal resolutions don't make any more sense to me, either.

Anyway, I did figure out 960x600, and for the benefit of anyone else who wants to dig into this, here's a summary of known values to date:

**Horizontal:**

```
640: 20 44 00
800: 48 44 00
960: 70 44 00
1024: 80 44 00
1280: a0 44 00
1360: aa 44 00
1600: c8 44 00
1920: f0 44 00
2304: 10 45 00
2560: 20 45 00
3840: 70 45 00
```

**Vertical:**

```
480: 00 f0 43
576: 00 10 44
600: 00 16 44
720: 00 34 44
768: 00 40 44
800: 00 48 44
900: 00 61 44
1080: 00 87 44
1200: 00 96 44
1440: 00 b4 44
2160: 00 07 45
```

### Post 20 — Irshansk (April 21, 2020)

Is there any way to run it on 4k screen with xbrz upscaling or something similar?

### Post 21 — Lir1066 (April 26, 2020; edited)

Hello Friend! You made a mistake with `00 88 44` = 1200. I checked, 1200 = `00 96 44`

I found a connection between blocks 43 and 44 - for resolutions above 1920, you must take block 45

```
F0 44 00 00 B4 44 = 1920x1440
20 45 00 00 B4 44 = 2560x1440
```

there is a cyclic connection between them, it resembles a subnet mask

2560/640=4, and both use code "20" (20-44 20-45)

1920/480=4, and both use code "F0" (F0-43 F0-44)

sry for my eng

### Post 22 — Irshansk (April 27, 2020; edited)

Could you please tell me the values for 3840x2160?

Thanks!

### Post 23 — Lir1066 (April 28, 2020; edited April 29, 2020)

Hello! I play in 2560x1440 resolution and not all maps in the game have a full size in width greater than 2560 pixels. Maps that are "wider" and "higher" than the resolution you use are displayed without problems. But as soon as you exceed this size, graphic artifacts will appear that make the game unplayable, because the engine does not provide for displaying the "edge of the map", as in strategies like the Age of Empires.

So I have to switch to a resolution of 1920x1440 for "narrow" maps, and then everything works fine.

You can play in the resolution of 1920x1080, because it is a multiple of the resolution of your monitor (one graphic pixel fits exactly into the square of the four pixels of your monitor, everything is clear and without blurring), or you will have to play around a bit with the resolutions, as you can read below.

If you use the game zoom, then the map will "fit" within your screen and everything will become normal (you cannot zoom out again until you reload the level). Therefore, if it’s convenient for you to play with zoom, here are the values for 4k (please answer if I calculated correctly, because I have nothing to check for this) -

```
70 45 00 00 07 45 = 3840x2160
```

You can also try to run the game in 2304x2160 (do not forget to select "Run in Window" in DXWnd, "Hide desktop background" and enter 2304x2160 size)

```
10 45 00 00 07 45 = 2304x2160
```

The second level (Nottingham) will definitely look good with these settings. (It has a width of 2304p). I myself just started to replay, as I progress, I will add the width of each level.

### Post 24 — Irshansk (May 15, 2020)

Thank you! :)

The 3840x2160 worked perfectly fine, in fact even better than 2560x1440 or 2304x2160. The map size limitation still limits the resolution, but at 4k when I zoom-in the map fits perfectly fine while still being better than under the same conditions at 2560x1440.

### Post 25 — rtwonmac (June 15, 2020; edited)

I tried the fixes above, but it doesn't work for me.

Easy solution (no work required) I found to fix the aspect ratio and the game not starting:

1. Use the compatibility settings in the attached image (win xp S3, no widescreen optimisation)
2. Manually change the aspect ratio on your monitor UI to 4:3
3. Use the highest video setting in game

Not perfect, but very close to the original experience.

### Post 26 — MrDOS (June 16, 2020)

Interesting.

`00 96 44` works for me, but so does `00 88 44`. I wonder if some interaction between my graphics driver and the game interprets something differently, because 1920x1200 is the highest resolution my monitor supports. Regardless, I've edited my post to reflect the more-correct value. Thank you for checking it!

That's fascinating. I've updated my omnibus listing to include the other common resolutions you've identified. Because of the limitations of my monitor, I hadn't hypothesized any higher, so thank you for expanding.

I think we nearly have enough information here to make a resolution patcher utility...

### Post 27 — chimaco3 (October 25, 2021)

I can't find with HEX searcher any of the codes, they are not inside. Some help?

### Post 28 — DranSetrius (February 6, 2022)

Hi, I tried to find to find those numbers, but I think that I have different ones. Can someone check if I f up?

### Post 29 — MrDOS (February 21, 2022)

Your screenshot doesn't include enough of your profile data for us to be able to help you find it, sorry.

In my profile, the resolution bytes start at `0x106`. Whatever hex editor you use, when you search for the current value, be sure to search for a hex string, not a text string.

### Post 30 — smuggly (September 5, 2022)

DXwnd

## Original text

### Post 1 — gmx (June 8, 2012)

Widescreen support in game / max resolution - is gog version better then others? it's on sale now, nostalgic buy or not to buy problem

### Post 2 — wtan1 (June 9, 2012)

Maximum in game: 1024 x 768. If anyone knows how to get it to display in windowed mode or even better 1920 x 1080, please let me know...

### Post 3 — Benne (June 11, 2012)

Have not been able to run it windowed out of the box. However, FYI - by downloading free VMware Player and running it in a VM, I am able to simulate a windowed set-up. Just a thought.

### Post 4 — JavyC89 (July 18, 2012)

Hey.. Is there an easy way to play the game windowed?

I have a 1080 monitor and ir looks awful :(

### Post 5 — MrDOS (September 8, 2012)

I had some amount of success with [D3DWindower](http://www.neowin.net/forum/topic/603613-d3dwindower/), although I had to play with it a lot to make it work properly.

### Post 6 — Theruler (January 9, 2017)

Any chance to have the same widescreen patch made for desperados?

### Post 7 — Blinkin89 (January 9, 2017)

As far as I know D3DWindower is a general application, so you could try it with any (Direct3D) game you want.

### Post 8 — ZellSF (July 8, 2018; edited)

So I was very bored today and tried to figure this out. No the GoG version does not support widescreen. But the game does seem to accept widescreen if you specify it in the profile. This is a bit tricky though, you need to open `Robin Hood\DATA\Savegame\Profiles` with a hex editor and search for one of these:

```
20 44 00 00 F0 43
48 44 00 00 16 44
80 44 00 00 40 44
```

There might be multiple entries if you have multiple profiles, replace all of them. They are the three resolutions you can choose ingame (top is 640x480, middle is 800x600, bottom is 1024x768). I'm too stupid to figure out what the numbers mean but I've experimented and found some resolutions to try.

If you replace with `80 44 00 00 10 44` you get 1024x576. This is nice because the game apparently uses width to determine which UI to load and so you get a perfect UI in this resolution. Plus it's a resolution the game was "designed for", it isn't smaller or larger in width or height than any of the supported game resolutions. Obviously you need to add this as a custom resolution in your GPU driver and set scaling to GPU and not monitor as few monitors will accept this resolution.

If you replace with `A0 44 00 34 44` you get 1280x720 which I prefer for pixel-based scaling titles. Nothing is too tiny and it's just a nice resolution to play titles like this. Also it's optimal for scaling to 1440p monitors. This and other resolutions however makes the UI look bad.

If you replace it with `F0 44 00 00 87 44` you get 1920x1080. I think everything is way too tiny here, though the game does offer a blocky looking zoom function.

I don't know if widescreen breaks anything, I've only tested very basic functionality. This was the extent of my testing: [YouTube test video](https://www.youtube.com/watch?v=PTXB807T7JA)

### Post 9 — mbhtst (July 14, 2018; edit note “by snowdark”)

Thanks, it really helped!

### Post 10 — robip85 (July 14, 2018)

1280x720 is `A0 44 00 00 34 44`. As you said, it's best for 1440p monitor.

1024x576 doesn't work for me, even main menu is not shown correctly, just tiles and no text.

Can you please test if you can get 1600x900? I think 1600x900 would be nice to try, as 1920x1080 is borderline playable.

With higher resolutions you see more of map, but the HUD gets smaller, it is usable up to 900 height.

If you want widescreen, first thing is to aim to at least 768 height, as you could get that in original game 15 years ago, so no point in getting smaller. That means you need at least 1366x768.

### Post 11 — chrix (July 23, 2018; edited July 24, 2018)

You're not stupid at all.. you did a great finding.

I can't figure out myself what those numbers are.. can't find a logic in them (and in how they change at the different resolutions)... but.. hey: they work as you said!

Thank you very much for sharing this amazing finding with everyone..

(I have the game on Steam and it works there too! ).!!!

### Post 12 — ZellSF (July 24, 2018)

1024x576 works fine here. Even if it didn't I wouldn't have a clue how to fix it. This is really just altering the resolution to an entirely unsupported one and just hoping the game accepts it. For some reason Robin Hood doesn't mind too much. Chicago 1930 and Desperados just crash when doing the same.

At any rate:

```
C8 44 00 00 61 44 = 1600x900
AA 44 00 00 40 44 = 1360x768
```

### Post 13 — Gamesiarz (November 10, 2018)

Hello I have a little problem when I switch to 720p on my wqhd I can't move camera to the bottom of the screen, other sides top, left, right work fine. Any ideas?

### Post 14 — Gamesiarz (December 19, 2018; edited)

I think that line for 1280x720 is incomplete here because every other pair of strings have 2 more characters. Could you check this?

### Post 15 — ZellSF (December 20, 2018)

Camera moving to bottom works just fine for me, and yeah a typo:

```
A0 44 00 34 44
```

should be

```
A0 44 00 00 34 44
```

### Post 17 — tonik2000 (November 11, 2019; edited December 22, 2019)

Please 16:10 resolution code. Just one Please. 1280:800 maby or 1440:900. ThankU

Update: found myself - 1280 * 800 `A0 44 00 00 48 44`

### Post 18 — ShiroOukami (January 25, 2020)

When i change it to 1600x900 or 1920x1080 ambush missions have black background.

Do you know why is that maybe?

### Post 19 — MrDOS (March 5, 2020; edited June 16, 2020)

I tried to take another look at the values. My goal was to enable 960x600, so that I could use 2x integer scaling on a 1920x1200 monitor. The first half/three bytes clearly control width, and the second half the height. However, I still can't figure out a consistent pattern. For some vertical resolutions, the value of the middle byte appears to encode the difference between resolutions in 4-pixel steps. E.g., the difference between a vertical resolution of 720 and 600 is `0x34 - 0x16 = 0x1E = 30`. Dividing the difference in heights by that number shows that each change by 1 is worth 4 pixels: `(720 - 600) / 30 = 120 / 30 = 4`.

Based on that, we can re-derive some other heights; e.g., for 900 pixels: `900 - 600 = 300`; `300 / 4 = 75`; `0x16 + 75 = 0x16 + 0x4b = 0x61`, which matches the known-good pattern `00 61 44`. But this doesn't hold for higher resolutions: `1080 - 600 = 480`; `480 / 4 = 120`; `0x16 + 120 = 0x16 + 0x78 + 8e`, but the expected pattern is `00 87 44`. In fact, through trial and error, I found `00 88 44` to be a 1200-pixel height. Horizontal resolutions don't make any more sense to me, either.

Anyway, I did figure out 960x600, and for the benefit of anyone else who wants to dig into this, here's a summary of known values to date:

**Horizontal:**

```
640: 20 44 00
800: 48 44 00
960: 70 44 00
1024: 80 44 00
1280: a0 44 00
1360: aa 44 00
1600: c8 44 00
1920: f0 44 00
2304: 10 45 00
2560: 20 45 00
3840: 70 45 00
```

**Vertical:**

```
480: 00 f0 43
576: 00 10 44
600: 00 16 44
720: 00 34 44
768: 00 40 44
800: 00 48 44
900: 00 61 44
1080: 00 87 44
1200: 00 96 44
1440: 00 b4 44
2160: 00 07 45
```

### Post 20 — Irshansk (April 21, 2020)

Is there any way to run it on 4k screen with xbrz upscaling or something similar?

### Post 21 — Lir1066 (April 26, 2020; edited)

Hello Friend! You made a mistake with `00 88 44` = 1200. I checked, 1200 = `00 96 44`

I found a connection between blocks 43 and 44 - for resolutions above 1920, you must take block 45

```
F0 44 00 00 B4 44 = 1920x1440
20 45 00 00 B4 44 = 2560x1440
```

there is a cyclic connection between them, it resembles a subnet mask

2560/640=4, and both use code "20" (20-44 20-45)

1920/480=4, and both use code "F0" (F0-43 F0-44)

sry for my eng

### Post 22 — Irshansk (April 27, 2020; edited)

Could you please tell me the values for 3840x2160?

Thanks!

### Post 23 — Lir1066 (April 28, 2020; edited April 29, 2020)

Hello! I play in 2560x1440 resolution and not all maps in the game have a full size in width greater than 2560 pixels. Maps that are "wider" and "higher" than the resolution you use are displayed without problems. But as soon as you exceed this size, graphic artifacts will appear that make the game unplayable, because the engine does not provide for displaying the "edge of the map", as in strategies like the Age of Empires.

So I have to switch to a resolution of 1920x1440 for "narrow" maps, and then everything works fine.

You can play in the resolution of 1920x1080, because it is a multiple of the resolution of your monitor (one graphic pixel fits exactly into the square of the four pixels of your monitor, everything is clear and without blurring), or you will have to play around a bit with the resolutions, as you can read below.

If you use the game zoom, then the map will "fit" within your screen and everything will become normal (you cannot zoom out again until you reload the level). Therefore, if it’s convenient for you to play with zoom, here are the values for 4k (please answer if I calculated correctly, because I have nothing to check for this) -

```
70 45 00 00 07 45 = 3840x2160
```

You can also try to run the game in 2304x2160 (do not forget to select "Run in Window" in DXWnd, "Hide desktop background" and enter 2304x2160 size)

```
10 45 00 00 07 45 = 2304x2160
```

The second level (Nottingham) will definitely look good with these settings. (It has a width of 2304p). I myself just started to replay, as I progress, I will add the width of each level.

### Post 24 — Irshansk (May 15, 2020)

Spasibo! :)

The 3840x2160 worked perfectly fine, in fact even better than 2560x1440 or 2304x2160. The map size limitation still limits the resolution, but at 4k when I zoom-in the map fits perfectly fine while still being better than under the same conditions at 2560x1440.

### Post 25 — rtwonmac (June 15, 2020; edited)

I tried the fixes above, but it doesn't work for me.

Easy solution (no work required) I found to fix the aspect ratio and the game not starting:

1. Use the compatibility settings in the attached image (win xp S3, no widescreen optimisation)
2. Manually change the aspect ratio on your monitor UI to 4:3
3. Use the highest video setting in game

Not perfect, but very close to the original experience.

### Post 26 — MrDOS (June 16, 2020)

Interesting.

`00 96 44` works for me, but so does `00 88 44`. I wonder if some interaction between my graphics driver and the game interprets something differently, because 1920x1200 is the highest resolution my monitor supports. Regardless, I've edited my post to reflect the more-correct value. Thank you for checking it!

That's fascinating. I've updated my omnibus listing to include the other common resolutions you've identified. Because of the limitations of my monitor, I hadn't hypothesized any higher, so thank you for expanding.

I think we nearly have enough information here to make a resolution patcher utility...

### Post 27 — chimaco3 (October 25, 2021)

I can't find with HEX searcher any of the codes, they are not inside. Some help?

### Post 28 — DranSetrius (February 6, 2022)

Hi, I tried to find to find those numbers, but I think that I have different ones. Can someone check if I f up?

### Post 29 — MrDOS (February 21, 2022)

Your screenshot doesn't include enough of your profile data for us to be able to help you find it, sorry.

In my profile, the resolution bytes start at `0x106`. Whatever hex editor you use, when you search for the current value, be sure to search for a hex string, not a text string.

### Post 30 — smuggly (September 5, 2022)

DXwnd
