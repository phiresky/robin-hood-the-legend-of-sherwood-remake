# Psojed — languages and fonts

- Original source: [Psojed — languages and fonts](https://steamcommunity.com/sharedfiles/filedetails/?id=1349014146)
- Author / publication: Psojed / Steam Community
- Language / date: English; 2018-04-01
- Access: Full guide page retrieved directly
- Checked: 2026-09-09
- Retrieved: 2026-09-09
- Archived copy: none found in the Wayback Machine (checked 2026-09-09)
- Format: header notes, original summary, then complete factual notes (no redistribution grant on Steam guides, so not transcribed)

Explains selecting Steam's supplied language and adding other localizations through replacement files. The guide describes changing the language in the game's Steam properties and discusses renaming the English `2047` directory so another installed localization is loaded.

It also has sections on Czech localization, appropriate fonts, and additional tips. The article is useful evidence that language assets and font choices have been a recurring community configuration issue.

**Historical scope:** its account of languages available on Steam and GOG is from 2018. It should not be presented as the current storefront language list; the GOG page inspected in this collection now includes Polish. Steam's properties interface has also changed since publication.

No localization files were downloaded or installed.

## Detailed notes

Guide metadata at retrieval: posted 1 Apr 2018 3:58am; categories Gameplay Basics, Modding or Configuration; 2,158 unique visitors, 20 favourites, 16 comments. Sections: Selecting language, Other languages, Čeština, Proper Text Font, Tips. No redistribution grant, so paraphrased.

- Selecting language: the Steam store promised four languages (English, German, French, Spanish); right-click the game in the library, Properties, Languages tab, pick from the drop-down.
- Other languages: the game was translated into more languages than Steam or GOG offer; place the wanted language files in the game directory and delete or rename the English folder `2047` (for example to `2047-1`) so the game loads the other language. Polish guides exist separately.
- Czech: the author provides an installer (about 150 MB, Google Drive) that switches text and dubbing to Czech, restores the original better-looking font, and can be uninstalled; the game must be set to English first.
- Fonts: the Steam build uses a different text font from the original; a Russian-language guide (Steam id 432645606) supplies the original fonts, which are copied as a `Fonts` folder into the game folder; the Czech installer also installs them, after which renaming `2047_EN` to `2047` restores English text.
- Tips: native resolution 1024×768; on 16:9 displays switch the monitor to 4:3 if possible, or run windowed or through emulation; the game installs defaulting to 800×600, so set 1024×768 in Settings → Graphics.
- Comments: fonts live in `DATA\Interface` and after the installer both `Fonts` and `Fonts_EN` folders exist (author, 13 Apr 2020); one reader combined the Czech pack with dgVoodoo 2 (copy the files from its MS folder into the game directory, enable "Fast video memory access" in the DirectX tab) to get diacritics and no FPS drops (29 May 2020); a macOS Mojave user reported the game stuck in German (2023); a reader confirms the installer works with the GOG version (Sep 2025); others report missing diacritics or a missing intro after installing.
