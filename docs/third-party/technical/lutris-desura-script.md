# Lutris — native Desura installer and language selection

- Original source: [Lutris — native Desura installer and language selection](https://lutris.net/games/install/7091/view)
- Author / publication: Lutris installer contributors
- Language / date: English; exact revision date not verified
- Access: Installer source and maintainer notes inspected; script not executed
- Checked: 2026-09-09
- Format: original summary and research notes; not a transcript.

The installer requests an existing Linux installer from Desura and launches data/robin. Its language-selection workaround reflects the maintainer's description of directory precedence: German data/1031, then French data/1036, with English data/2047 as fallback. The script renames unwanted language directories.

The notes recommend disabling the Lutris Runtime for the game to select its resolution correctly. This is historical maintainer guidance, not a verified requirement for current Lutris releases.

The description calls Desura the only available native edition, but that should not erase earlier CD and PowerPC releases documented elsewhere. No files were renamed or software installed during this research.
