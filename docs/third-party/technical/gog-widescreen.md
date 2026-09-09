# GOG — historical profile-based widescreen experiments

- Original source: [widescreen](https://www.gog.com/forum/robin_hood_legend_of_sherwood/widescreen/page1)
- Author / publication: gmx, ZellSF, and respondents; GOG forum.
- Language / date: English; opened 2012-06-08; key experiment 2018-07-08; last post 2022-09-05.
- Access: Both pages retrieved directly (posts 1-15 and 17-30; no post 16 exists on either page)
- Checked: 2026-09-09
- Retrieved: 2026-09-09
- Archived copy: lookup failed (archive.org rate limit); not checked
- Format: header notes, original summary, then complete post-by-post notes (forum thread, not transcribed)

ZellSF reports obtaining unsupported resolutions by editing the Profiles file under DATA/Savegame. Their experiments distinguish 1024×576, which reportedly fits the existing interface well, from larger resolutions that shrink or disrupt interface elements. They explicitly limit their testing to basic functionality; a subsequent player reports success at 1920×1080.

The thread is historical evidence of profile-controlled rendering dimensions, not proof of complete widescreen support. Later replies discuss additional dimensions and increased visible map area. The posted byte sequences were not tested or reproduced here. These observations predate the GOG preservation update.

## Detailed notes

Page facts: GOG forum thread "widescreen" in the Robin Hood Legend of Sherwood subforum; 29 posts over 2 pages; opened by gmx on 2012-06-08. Page 1 holds posts 1 to 15 (2012-06-08 to 2018-12-20); page 2 (posts 17 to 30) is covered in a second table below. Post 8 carries a "high rated" badge. Retrieved directly from gog.com on 2026-09-09. Forum threads are not licensed for reproduction, so the posts are summarised post by post; the byte sequences are reproduced exactly because they are the technical payload.

| # | Poster (profile details shown) | Date | Content |
|---|---|---|---|
| 1 | gmx ("Evil geck0"; registered Sep 2008; Poland) | 2012-06-08 | Asks about widescreen support and max resolution, and whether the GOG version is better than others; it is on sale. |
| 2 | wtan1 (New User; registered Apr 2012; United States) | 2012-06-09 | Maximum in-game resolution is 1024x768. Asks for a way to run windowed or at 1920x1080. |
| 3 | Benne (New User; registered Nov 2009; Serbia) | 2012-06-11 | Could not run windowed out of the box. Workaround: run the game inside a VMware Player virtual machine to simulate a windowed setup. |
| 4 | JavyC89 (New User; registered Jul 2012; Costa Rica) | 2012-07-18 (edited) | Asks for an easy way to play windowed; on a 1080 monitor the game looks awful. |
| 5 | MrDOS (New User; registered Dec 2008; Canada) | 2012-09-08 (edited) | Had some success with D3DWindower (links a Neowin forum topic), though it needed a lot of fiddling. |
| 6 | Theruler (New User; registered Sep 2009; Italy) | 2017-01-09 | Asks whether the same widescreen patch could be made for Desperados. |
| 7 | Blinkin89 (Master Survivor, GOG Patron; registered Sep 2012; Netherlands) | 2017-01-09 | D3DWindower is a general application, so it can be tried with any Direct3D game. |
| 8 | ZellSF (New User; registered Apr 2010; Norway) | 2018-07-08 (edited) | The GOG version does not support widescreen, but the game accepts widescreen if specified in the profile. Open `Robin Hood\DATA\Savegame\Profiles` in a hex editor and search for one of the three sequences below (multiple entries may exist with multiple profiles; replace all). Does not know what the numbers mean but found working values by experiment. 1024x576: the game apparently uses width to pick which UI to load, so the UI is perfect at this resolution, and it is neither wider nor taller than any supported resolution; needs a custom resolution in the GPU driver with GPU (not monitor) scaling. 1280x720: preferred for pixel-scaled titles, optimal for 1440p monitors, but the UI looks bad. 1920x1080: everything way too tiny, though the game has a blocky zoom function. Only very basic functionality tested; links a YouTube video (PTXB807T7JA). Attaches 576.jpg, 720.jpg, 1080.jpg. |
| 9 | mbhtst (New User; registered Feb 2014; Russian Federation) | 2018-07-14 (edit note says "by snowdark") | Thanks; the 1920x1080 value really helped. |
| 10 | robip85 (New User; registered Mar 2013; Slovenia) | 2018-07-14 | Corrects the 1280x720 value to `A0 44 00 00 34 44`; agrees it suits 1440p monitors. 1024x576 does not work for them: even the main menu shows just tiles and no text. Asks for 1600x900 because 1920x1080 is borderline playable. Higher resolutions show more map but the HUD shrinks; usable up to 900 height. Advises aiming for at least 768 height, i.e. at least 1366x768, since 768 was already available in the original game. |
| 11 | chrix (New User; registered Dec 2010; Italy) | 2018-07-23 (edited 2018-07-24) | Praises the finding; also cannot find the logic in the numbers, but they work. Confirms the edit works on the Steam version too. |
| 12 | ZellSF | 2018-07-24 | 1024x576 works fine for them; could not fix it otherwise. The trick simply sets an unsupported resolution and hopes the game accepts it; Robin Hood tolerates it, whereas Chicago 1930 and Desperados crash when the same is done. Adds 1600x900 and 1360x768 values (below). Attaches two game_2018-07-24_.jpg screenshots. |
| 13 | Gamesiarz (New User; registered May 2011; Poland) | 2018-11-10 | After switching to 720p on a WQHD monitor, cannot scroll the camera to the bottom edge of the screen; top, left and right work. |
| 14 | Gamesiarz | 2018-12-19 (edited) | Notes the 1280x720 line in post 8 is two characters shorter than every other pair; asks for a check. |
| 15 | ZellSF | 2018-12-20 | Bottom-edge camera scrolling works fine for them. Confirms the typo: `A0 44 00 34 44` should be `A0 44 00 00 34 44`. |

### Every byte sequence posted

Search targets in the Profiles file (the three stock resolutions, as given in post 8):

| Sequence | Resolution |
|---|---|
| `20 44 00 00 F0 43` | 640x480 |
| `48 44 00 00 16 44` | 800x600 |
| `80 44 00 00 40 44` | 1024x768 |

Replacement values:

| Sequence | Resolution | Posted by | Notes |
|---|---|---|---|
| `80 44 00 00 10 44` | 1024x576 | ZellSF, post 8 | UI fits perfectly; robip85 reports broken menu at this size. |
| `A0 44 00 34 44` | 1280x720 | ZellSF, post 8 | Typo, one byte missing; corrected in posts 10 and 15. |
| `A0 44 00 00 34 44` | 1280x720 | robip85 post 10, confirmed ZellSF post 15 | Correct value. |
| `F0 44 00 00 87 44` | 1920x1080 | ZellSF, post 8 | Everything very small; confirmed working by mbhtst. |
| `C8 44 00 00 61 44` | 1600x900 | ZellSF, post 12 | Requested by robip85. |
| `AA 44 00 00 40 44` | 1360x768 | ZellSF, post 12 | |

The posters did not identify the encoding. The sequences are consistent with two little-endian 32-bit floats separated as width then height (for example `00 00 80 44` = 1024.0 and `00 00 40 44` = 768.0), with the search strings spanning the last two bytes of the width float and the whole height float; that reading is an observation made here, not a claim from the thread.

Other tools mentioned: VMware Player (windowing workaround), D3DWindower (windowing), GPU-driver custom resolutions with GPU scaling.

### Page 2 (posts 17 to 30)

Page facts: page 2 retrieved directly from gog.com on 2026-09-09. The page numbers its posts 17 to 30; there is no post 16 anchor on either page (the thread counter still says 29 posts, so one post was presumably deleted). Post 19 carries a "high rated" badge.

| # | Poster (profile details shown) | Date | Content |
|---|---|---|---|
| 17 | tonik2000 (New User; registered Jul 2017; Russian Federation) | 2019-11-11 (edited 2019-12-22) | Asks for a 16:10 code (1280x800 or 1440x900). Update: found 1280x800 themselves: `A0 44 00 00 48 44`. |
| 18 | ShiroOukami (New User; registered May 2017; Poland) | 2020-01-25 | At 1600x900 or 1920x1080 the ambush missions have a black background; asks why. |
| 19 | MrDOS | 2020-03-05 (edited 2020-06-16) | Wanted 960x600 for 2x integer scaling on a 1920x1200 monitor. The first three bytes control width, the second three height, but no consistent pattern found. For some heights the middle byte encodes 4-pixel steps: 0x34 - 0x16 = 30, and (720 - 600) / 30 = 4; 900 then gives 0x16 + 75 = 0x61, matching `00 61 44`; but 1080 would give 0x8E while the known value is `00 87 44`. Found `00 88 44` for 1200 by trial and error (later corrected, see post 21 and 26). Posts an omnibus list of known values (table below, as edited on 2020-06-16). |
| 20 | Irshansk (New User; registered Dec 2017; United States) | 2020-04-21 | Asks whether the game can run on a 4K screen with xBRZ upscaling or similar. |
| 21 | Lir1066 (New User; registered Apr 2020; Russian Federation) | 2020-04-26 | Corrects MrDOS: 1200 is `00 96 44`, not `00 88 44`. For resolutions above 1920 the third byte moves from block 44 to block 45. Gives `F0 44 00 00 B4 44` = 1920x1440 and `20 45 00 00 B4 44` = 2560x1440. Observes a cyclic relation "like a subnet mask": 2560/640 = 4 and both use code 20 (20 44 vs 20 45); 1920/480 = 4 and both use F0 (F0 43 vs F0 44). |
| 22 | Irshansk | 2020-04-27 (edited) | Asks for the 3840x2160 values. |
| 23 | Lir1066 | 2020-04-28 (edited 2020-04-29) | Plays at 2560x1440. Not all maps are wider than 2560 pixels: maps larger than the chosen resolution display fine, but once the resolution exceeds the map size graphic artifacts appear that make the game unplayable, because the engine cannot draw a "map edge" the way Age of Empires does. Switches to 1920x1440 for narrow maps. Recommends 1920x1080 on a 4K monitor because it is an exact 2x multiple. With the in-game zoom the map fits the screen and works normally, but you cannot zoom back out until the level is reloaded. Offers, untested because they have no 4K screen, `70 45 00 00 07 45` = 3840x2160, and `10 45 00 00 07 45` = 2304x2160 (run in DxWnd with "Run in Window" and "Hide desktop background", size 2304x2160). States the second level, Nottingham, is 2304 pixels wide and will look good at that setting; promises to record each level's width while replaying. |
| 24 | Irshansk | 2020-05-15 | Thanks (in Russian). 3840x2160 worked perfectly, even better than 2560x1440 or 2304x2160; the map-size limitation still applies, but at 4K with zoom-in the map fits well. |
| 25 | rtwonmac (New User; registered Nov 2013; Netherlands) | 2020-06-15 (edited) | The hex fixes did not work for them. Alternative with no editing: 1) compatibility settings as in the attached screenshot (Windows XP SP3, no widescreen optimisation); 2) set the monitor's own aspect ratio to 4:3; 3) use the highest in-game video setting. Not perfect but close to the original. Attaches compatibility_se.png. |
| 26 | MrDOS | 2020-06-16 | `00 96 44` works for them, but so does `00 88 44`; wonders whether a driver interaction explains it, since 1920x1200 is their monitor's maximum. Edited the omnibus post to use 96 and added Lir1066's higher resolutions. Says there is nearly enough information to build a resolution-patcher utility. |
| 27 | chimaco3 (New User; registered Jun 2018; Spain) | 2021-10-25 | Cannot find any of the codes with a hex search. |
| 28 | DranSetrius (New User; registered Nov 2014; Poland) | 2022-02-06 | Also cannot find the numbers; thinks theirs differ; attaches a HxD screenshot (robin_hxd_ss.jpg). |
| 29 | MrDOS | 2022-02-21 | The screenshot shows too little of the profile to help. In MrDOS's profile the resolution bytes start at offset 0x106. Search for a hex string, not a text string. Attaches sherwood-resolut.png. |
| 30 | smuggly (New User; registered Jun 2017; United States) | 2022-09-05 | Single word: "DXwnd". |

### Byte values posted on page 2

MrDOS's omnibus list (post 19, as edited 2020-06-16; widths are the first three bytes, heights the last three):

| Width | Bytes | | Height | Bytes |
|---|---|---|---|---|
| 640 | `20 44 00` | | 480 | `00 F0 43` |
| 800 | `48 44 00` | | 576 | `00 10 44` |
| 960 | `70 44 00` | | 600 | `00 16 44` |
| 1024 | `80 44 00` | | 720 | `00 34 44` |
| 1280 | `A0 44 00` | | 768 | `00 40 44` |
| 1360 | `AA 44 00` | | 800 | `00 48 44` |
| 1600 | `C8 44 00` | | 900 | `00 61 44` |
| 1920 | `F0 44 00` | | 1080 | `00 87 44` |
| 2304 | `10 45 00` | | 1200 | `00 96 44` |
| 2560 | `20 45 00` | | 1440 | `00 B4 44` |
| 3840 | `70 45 00` | | 2160 | `00 07 45` |

Full six-byte values posted on page 2:

| Sequence | Resolution | Posted by | Notes |
|---|---|---|---|
| `A0 44 00 00 48 44` | 1280x800 | tonik2000, post 17 | |
| `00 88 44` (height only) | 1200 | MrDOS, post 19 | Disputed; MrDOS says both 88 and 96 work on their setup. |
| `00 96 44` (height only) | 1200 | Lir1066, post 21 | Adopted in the omnibus list. |
| `F0 44 00 00 B4 44` | 1920x1440 | Lir1066, post 21 | |
| `20 45 00 00 B4 44` | 2560x1440 | Lir1066, post 21 | |
| `70 45 00 00 07 45` | 3840x2160 | Lir1066, post 23 | Confirmed working by Irshansk, post 24. |
| `10 45 00 00 07 45` | 2304x2160 | Lir1066, post 23 | Intended for DxWnd windowed mode. |

Other page-2 facts: resolutions larger than a map's pixel size produce artifacts because the engine does not render beyond the map edge; the Nottingham level is stated to be 2304 pixels wide; ambush missions show a black background at 1600x900 and 1920x1080 for one poster; the profile's resolution bytes sit at offset 0x106 in one poster's file; the observed `00 88 44` versus `00 96 44` discrepancy is consistent with the float reading (0x4496 = 1200.0 exactly, 0x4488 = 1088.0), which supports the float interpretation noted above.
