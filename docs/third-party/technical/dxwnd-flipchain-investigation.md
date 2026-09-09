# DxWnd — cursor trails and flipchain investigation

- Original title: Overlay and Flipchains Emulation; Dxwnd old and obsolete export files?
- Source: [Original publication](https://sourceforge.net/p/dxwnd/discussion/general/thread/566ddb1947/?page=2)
- Author / publisher: gho, BEEN_Nath_58, dippy dipper, and other DxWnd contributors
- Language / date: English; March 2023
- Access: Robin Hood passages and surrounding developer discussion inspected; complete related short thread inspected.
- Checked: 2026-09-09

On 25 March, gho reports eliminating Robin Hood’s cursor trails by correcting surface order in a flipchain with two backbuffers, avoiding the costly compensation path. The discussion also records problems applying the change to other games.

A [31 March follow-up](https://sourceforge.net/p/dxwnd/discussion/general/thread/ef8d4c788c/) says the ordinary build still produces trails, while a rebuilt DLL using circular surface rotation renders correctly. This distinction prevents the experimental success from being mistaken for a completed release fix. The discussion is useful primary evidence of wrapper development; its binaries, images, and rendering claims were not tested locally.
