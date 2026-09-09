# Missing original text

The collection contains 168 source Markdown files in its subdirectories. **No entry is now wholly missing original text.** The eleven previously unavailable sources were recovered from the manual uploads in `originals/tosort` on 2026-09-09. Some captures still have the specific coverage limits listed below.

The mappings were checked against `fetch_originals.sh`, canonical URLs in the manual HTML captures, and the contents of the saved HTML, text, PDF, JSON, and image files. Manual captures have byte-identical stable copies named `originals/<directory>__<document>-manual.<extension>`; the uploads and earlier failed captures are preserved. `originals/manual-captures.json` records the upload-to-document mapping, also incorporated into `originals/conversion-mapping.json`.

## Recovered from manual uploads

| Markdown file | Recovered source |
| --- | --- |
| [Jeuxvideo camp arrow shortage](guides/jeuxvideo-camp-arrow-shortage.md) | French forum thread, with English translation. |
| [Medieval soundspace paper](history/medieval-soundspace.md) | Complete 21-page PDF, printed pages 307–327. |
| [GameZone review](reviews/gamezone-de.md) | German article, verdict and score, with English translation. |
| [Grouvee player records](reviews/grouvee-player-records.md) | Game page, ratings, player records and two review cards; expansion/comment gaps below. |
| [Neoseeker Christian Gamer review](reviews/neoseeker-christian-gamer.md) | Complete review and score; page reports no comments. |
| [Patient Gamers — SpiderousMenace](reviews/patientgamers-spiderousmenace.md) | Post and captured comments; comment-count gap below. |
| [Patient Gamers — zehnpae](reviews/patientgamers-zehnpae.md) | Post and thirteen comment nodes; one removed body is unavailable. |
| [Toronto Computes — Talbot](reviews/toronto-computes-talbot.md) | Article screenshot, including both text columns and information box. |
| [CodeWeavers CrossOver](technical/codeweavers-crossover.md) | Compatibility page, version ratings, metadata and installation text. |
| [DxWnd flipchain investigation](technical/dxwnd-flipchain-investigation.md) | Page 3 of 5, with 25 posts and attachment links. |
| [ModDB cinematic enhancement](technical/moddb-cinematic-enhancement.md) | Mod description, installation instructions, file listing and metadata. |
| [Gamez.ru walkthrough](guides/gamez-russian.md) | Additional manual combined-page capture, with all eleven captured sections translated. The saved article still ends mid-sentence; see the remaining gap below. |

## Recovered by web fetch and Playwright — converted

A recovery pass on 2026-09-09 saved the following additional originals in `originals/recovery/`. **The recovered text has been incorporated into the linked Markdown files, with English translations above non-English originals.** It is no longer missing source text. Browser result logs retain requested/final URLs and status; web extracts retain source URLs and crawl metadata. Archived and cached pages may reflect different dates from the initial captures.

| Markdown file(s) | Newly available source coverage | Capture names in `originals/recovery/` |
| --- | --- | --- |
| [Kisa Windows](technical/steam-kisa-windows.md), [fonts](technical/steam-language-fonts.md), [Naxyň FPS](technical/steam-naxyn-fps.md), [Polish localization](technical/steam-polish-localization.md), [secret ending](guides/steam-secret-ending.md) | All reported comment sets: 78, 16, 71, 55 and 11 respectively; 231 unique comment IDs across the five guides, including existing first-page captures. | `steam-kisa*`, `steam-fonts*`, `steam-naxyn*`; Polish pages 1–2 plus `steam-polish-retry-p3` through `p6`; existing secret-ending original plus `steam-secret-retry-p2.browser.html`. |
| [GOG edition](history/gog-edition.md) | 36 review pages, 178 distinct review cards. | `gog-reviews.browser.html` and `gog-reviews-p2` through `p36.browser.html`. |
| [WineHQ mouse-jitter bug](technical/wine-mouse-jitter-57031.md) | Initial message and eight replies/events, including Bugzilla Comment #1. | `wine-thread.browser.html`. |
| [Sina/Yicai walkthrough](guides/sina-yicai-chinese.md) | Previously missing first article page. | `sina-p1.browser.html`. |
| [GRYOnline walkthrough](guides/gry-online-walkthrough.md) | Fifteen additional chapter bodies, their map links, and 21 captured guide comments; the other two chapters already have separate Markdown files. | Fifteen `gry-<chapter>.browser.html` files, excluding `gry-index.browser.html`. |
| [DxWnd flipchain](technical/dxwnd-flipchain-investigation.md) | Pages 1, 2, 4 and 5, complementing the manual page 3. | `dxwnd-flipchain-p1.web.txt`, `p2.web.txt`, `p4.web.txt`, `p5.web.txt`. |
| [DxWnd hooking](technical/dxwnd-hooking-discussion.md) | Missing second discussion page. | `dxwnd-hooking-p2.web.txt`. |
| [PC Games review](reviews/pcgames.md) | Second article page, including verdict and rating, from Wayback. The apparent third page redirects to an image gallery. | `pcgames-archive-p2.browser.html`; `pcgames-archive-p3.browser.html` is a gallery capture, not another article page. |
| [Metacritic](reviews/metacritic.md) | Integrated 19 user-review records with full quotes, including two spoiler-hidden reviews, and ten recovered critic excerpt records (12 retained after merging earlier excerpts). This does not establish coverage of every review counted by the site. | `metacritic-expanded.browser.html`, `metacritic-critics.browser.html`; extract the `__NUXT_DATA__` records, not only visible text. |
| [Jeuxvideo tips](guides/jeuxvideo-french-tips.md) | Three of eleven linked tips: Les chevaliers, Sherwood, and Cheat codes. | `jv-tip-6.web.txt`, `jv-tip-11.web.txt`, `jv-tip-12.web.txt`. |
| [Speedrun community](reference/speedrun-community.md) | Resources list and three linked forum discussions: missing secret Attack on Lincoln level, blocking hotkeys, and missing an ambush. | `speedrun-10.web.txt`, `speedrun-87.web.txt`, `speedrun-88.web.txt`, `speedrun-90.web.txt`. |

Failed captures are preserved for diagnostics, **not usable originals**. In particular, the non-retry Polish pages 3–6, non-retry secret-ending pages, direct PC Games pages, browser DxWnd pages, and all Jeuxvideo tip extracts except 6/11/12 are error responses. The recovery manifest lists only selected usable captures.

The archived second [Mod by Gravitr](technical/moddb-gravitr.md) comment page was also checked: its four visible comment bodies are already in the existing capture and Markdown. The prior “two missing comments” claim was incorrect; the remaining gap is an approval-hidden comment body.

## Still unavailable or incomplete after the recovery pass

- [Grouvee player records](reviews/grouvee-player-records.md): full Luitenant_Gruber review and linked comment bodies remain blocked by site verification. A Metacritic LT_Gruber review has matching opening text and date and supplies a possible complete cross-post, but is not verified as the exact Grouvee continuation.
- [Gamez.ru walkthrough](guides/gamez-russian.md): the newly supplied archived `4233_full.htm` capture is integrated in full, but its article text ends mid-sentence at “Самое время найти” (“It is time to find”), followed by the author/date and footer. The continuation remains unavailable; the filename does not establish completeness.
- [Magazine references](reference/magazine-indexes.md): issue references are available, not the magazine articles themselves.
- [Speedrun community](reference/speedrun-community.md): additional leaderboard tabs, run metadata and other linked discussions remain uncaptured; some web-fetch requests failed.
- [Video walkthrough](reference/video-walkthrough.md): browser capture and description expansion did not expose a transcript, captions or viewer comments.
- [Steam secret-ending guide](guides/steam-secret-ending.md): comment pagination is recovered, but image-only instructions remain linked rather than transcribed.
- [Mod by Gravitr](technical/moddb-gravitr.md) and [ModDB performance fix](technical/moddb-performance-fix-history.md): approval-hidden comment bodies are not publicly available in the captures.
- Discussion attachments, linked videos and remaining PC Games gallery images have not been downloaded or transcribed.

No missing text was reconstructed from summaries. Converted Markdown removes site chrome, retains relevant comments and formatting, and places English translations above non-English originals. Source mappings and per-file conversion status are recorded in `originals/conversion-mapping.json` and `originals/recovery/manifest.json`.

Checked: 2026-09-09.
