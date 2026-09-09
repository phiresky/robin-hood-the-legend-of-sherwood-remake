# Missing original text

The collection contains 168 source Markdown files in its subdirectories. Every file has a matching capture in `originals/`, but the 11 captures below contain access errors or archive notices rather than the requested source text. Existing research notes in these files are labeled as editorial notes, not transcripts.

The mappings were checked against `fetch_originals.sh` and the saved HTML, text, PDF, and JSON files. A failed text conversion alone does not count as a missing original: readable HTML, compressed HTML, and alternate text captures were used where available.

## No original article, review, or discussion text

| Markdown file | What the saved original contains |
| --- | --- |
| [Jeuxvideo camp arrow shortage](guides/jeuxvideo-camp-arrow-shortage.md) | Wayback Machine interface/error page; no discussion text. |
| [Medieval soundspace paper](history/medieval-soundspace.md) | Repository bot challenge; no paper text. |
| [GameZone review](reviews/gamezone-de.md) | Cloudflare challenge; no review text. |
| [Grouvee player records](reviews/grouvee-player-records.md) | Wayback “not archived” notice; no player records. |
| [Neoseeker Christian Gamer review](reviews/neoseeker-christian-gamer.md) | Wayback “not archived” notice; no review text. |
| [Patient Gamers — SpiderousMenace](reviews/patientgamers-spiderousmenace.md) | Reddit security block in HTML/TXT; the `.json` file also contains blocked HTML. No post or comments. |
| [Patient Gamers — zehnpae](reviews/patientgamers-zehnpae.md) | Reddit security block in HTML/TXT; the `.json` file also contains blocked HTML. No post or comments. |
| [Toronto Computes — Talbot](reviews/toronto-computes-talbot.md) | Scribd client challenge; no magazine review text. |
| [CodeWeavers CrossOver](technical/codeweavers-crossover.md) | Access-block/archive notice; no compatibility record text. |
| [DxWnd flipchain investigation](technical/dxwnd-flipchain-investigation.md) | Wayback “not archived” notice; no discussion text. |
| [ModDB cinematic enhancement](technical/moddb-cinematic-enhancement.md) | Cloudflare challenge and empty text rendering; no mod description. |

## Available text with material coverage limits

These files have usable original text, so they are not included in the missing-original count above.

- [Gamez.ru walkthrough](guides/gamez-russian.md): page 1 of 3 is captured; pages 2–3 are absent.
- [Sina/Yicai walkthrough](guides/sina-yicai-chinese.md): the captured second page covers missions 9–23; the preceding page with missions 1–8 is absent.
- [GRYOnline walkthrough index](guides/gry-online-walkthrough.md) and [Jeuxvideo tips index](guides/jeuxvideo-french-tips.md): the captured pages contain indexes; linked chapter/tip bodies are not part of those originals. The separately captured GRYOnline chapters have their own Markdown files.
- [PC Games review](reviews/pcgames.md): only the first article page is captured.
- [DxWnd hooking discussion](technical/dxwnd-hooking-discussion.md): page 1 contains 25 posts; page 2 is absent.
- [Mod by Gravitr](technical/moddb-gravitr.md): ten of twelve comments are captured; the second comment page is absent.
- [Metacritic](reviews/metacritic.md): the capture exposes seven critic excerpts and seven user-review entries, not every review counted by the site; hidden spoiler text is unavailable.
- [GOG edition](history/gog-edition.md): includes the five user reviews exposed by the captured product page, not all reviews on GOG.
- [Magazine references](reference/magazine-indexes.md): the capture has issue references and a game/video record, not the magazine articles themselves.
- [Speedrun community](reference/speedrun-community.md): includes the captured leaderboard and community index, not unprovided tabs or linked forum discussions.
- [Video walkthrough](reference/video-walkthrough.md): title, metadata, and description are available; no transcript, captions, or viewer comments were captured.
- [Steam secret-ending guide](guides/steam-secret-ending.md): ten comment entries are captured although the page reports eleven; some instructions are supplied only as linked images.
- [Kisa Windows guide](technical/steam-kisa-windows.md): the full guide is captured, but only ten of 78 comments are present.
- [Language and fonts guide](technical/steam-language-fonts.md): the full guide is captured, but only ten of sixteen comments are present.
- [Naxyň FPS guide](technical/steam-naxyn-fps.md): the full guide is captured, but only ten of 71 comments are present.
- [Polish localization guide](technical/steam-polish-localization.md): the full guide is captured, but only ten of 55 comments are present.
- [ModDB performance fix](technical/moddb-performance-fix-history.md): captured comments include approval-status notices whose underlying comment text is unavailable.
- [WineHQ mouse-jitter bug](technical/wine-mouse-jitter-57031.md): the first message is captured, but reply bodies are absent from the supplied HTML/TXT. Earlier research notes about replies and Bugzilla Comment #1 are retained separately as unverified editorial notes.

Other per-file notes identify comments that were counted or linked by a site but not present in the saved page. No missing comments, linked articles, hidden text, or video transcripts were reconstructed from summaries.

Checked: 2026-09-09.
