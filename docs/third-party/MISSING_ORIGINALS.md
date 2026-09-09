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

## Available text with material coverage limits

These files have usable original text, but further captures would fill the following gaps.

- [Grouvee player records](reviews/grouvee-player-records.md): Luitenant_Gruber’s review is truncated behind “Read more”; neither review card’s linked comment body is captured.
- [Patient Gamers — SpiderousMenace](reviews/patientgamers-spiderousmenace.md): the post reports 25 comments, but only 23 comment nodes are captured.
- [Patient Gamers — zehnpae](reviews/patientgamers-zehnpae.md): all thirteen comment nodes are captured, but one deleted/moderator-removed comment has no recoverable body.
- [CodeWeavers CrossOver](technical/codeweavers-crossover.md): per-version submitted-rank details and additional “Show More” entries are dynamically loaded and absent from the capture; the linked tutorial is not included.
- [DxWnd flipchain investigation](technical/dxwnd-flipchain-investigation.md): only page 3 of 5 is captured (`?page=2` is zero-indexed); pages 1–2 and 4–5 are absent. Attachment links are preserved, not the binary/media contents.
- [ModDB cinematic enhancement](technical/moddb-cinematic-enhancement.md): the captured file-list excerpt is truncated; the linked download page and video contents are not included.
- [Toronto Computes — Talbot](reviews/toronto-computes-talbot.md): the article is legible, but the screenshot does not show the issue date or page number; the existing May 2003/page 51 citation is retained as editorial provenance.

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
