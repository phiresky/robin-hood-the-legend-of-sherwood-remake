# DxWnd — cursor trails and flipchain investigation

- Original title: “Overlay and Flipchains Emulation; Dxwnd old and obsolete export files?”
- Source: [Original publication](https://sourceforge.net/p/dxwnd/discussion/general/thread/566ddb1947/?page=2)
- Author / publisher: gho, BEEN_Nath_58, dippy dipper, and other DxWnd contributors (as reported in the retained research notes)
- Language / date: English; March 2023 (date range inferred from the retained research notes)
- Checked: 2026-09-09
- Availability: **Original thread text unavailable.** The live SourceForge page returned HTTP 403 to the coordinating non-browser fetch, and the Wayback Machine reported no archived capture.

## Editorial research notes (not source text)

The retained notes report that, on 25 March, gho eliminated Robin Hood’s cursor trails by correcting surface order in a flipchain with two backbuffers, avoiding the costly compensation path. The notes also report problems applying the change to other games.

A [31 March follow-up](https://sourceforge.net/p/dxwnd/discussion/general/thread/ef8d4c788c/) is reported to say that the ordinary build still produces trails, while a rebuilt DLL using circular surface rotation renders correctly. This distinction prevents the experimental success from being mistaken for a completed release fix. The discussion is useful primary evidence of wrapper development; its binaries, images, and rendering claims were not tested locally.

The poster handles, exact dates, attachments, binaries, images, and the full wording of both discussions could not be verified from the retrieved files.

## Original text unavailable

The saved HTML/TXT contain only the following Wayback Machine availability notice, not the SourceForge discussion:

> The Wayback Machine has not archived that URL.

The notice links to [all archived pages under the thread URL](https://web.archive.org/web/*/https://sourceforge.net/p/dxwnd/discussion/general/thread/566ddb1947/*), but no capture of page 1 or page 2 was available in this pass.

The saved error file contains only a curl invocation error (`option : blank argument where content is expected`), not an HTTP response body.
