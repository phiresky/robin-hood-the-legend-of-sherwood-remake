# DxWnd — launch hooks and GOG version differences

- Original title: DxWnd doesn't do anything
- Source: [Original publication](https://sourceforge.net/p/dxwnd/discussion/general/thread/f20e2b7e/)
- Author / publisher: David Camacho, Daniel, gho, and other DxWnd forum participants
- Language / date: English; 2016-02-04 through 2021-10-08
- Access: Indexed first-page discussion and full second page inspected.
- Checked: 2026-09-09

In 2016, gho explains that the actual game process is game.exe rather than its launcher and supplies a configuration using DLL injection and flip compensation. The discussion helps explain why apparently valid profiles can have no effect.

On [page two](https://sourceforge.net/p/dxwnd/discussion/general/thread/f20e2b7e/?page=1), Daniel’s 2021 launch crash disappears after changing the hook method from suspended-process injection to SetWindowsHook. gho attributes the difference to a GOG version with a bundled ddraw.dll, and discourages abandoning the working setup. The page also discusses minimizing and Alt-Tab behavior. These are version-specific historical findings, not universal setup instructions.
