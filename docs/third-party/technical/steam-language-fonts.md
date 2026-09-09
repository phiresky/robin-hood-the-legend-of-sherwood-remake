# Robin Hood Language & Texts

- Original source: [Psojed — languages and fonts](https://steamcommunity.com/sharedfiles/filedetails/?id=1349014146)
- Author: Psojed
- Published: 1 April 2018, 3:58 a.m.
- Source language: English and Czech
- Retrieval: Full guide body and all comments retrieved from `originals/recovery/steam-fonts-p1.browser.html` and `originals/recovery/steam-fonts-p2.browser.html` on 9 September 2026
- Comments: 16 unique comments captured across the two paginated sources (16 comments reported).

## English translation

### Robin Hood Language & Texts

By Psojed

A short guide about changing the game language and fonts.

### Selecting language

Welcome to my guide!

Robin Hood: Legend of Sherwood is an old game, and as with many old games, it needs some configuration before you can play properly. As the Store page promises, Robin Hood comes in four different languages: English, German, French and Spanish.

To select your preferred language, simply **Right-click** with your mouse on the game in your Steam Library and select **Properties**.

![Steam game Properties menu](https://images.steamusercontent.com/ugc/925927998461118339/6EED6E60B092DC5637904D400F083FC537984A13/)

Then, switch to the **Languages** tab and there, select your preferred language from the drop-down menu:

![Steam Languages tab](https://images.steamusercontent.com/ugc/925927998461121465/72D785D0D907E55A4A45C738CCC1F4381068E981/)

That's it. When you launch the game now, it will have the language you selected.

### Other languages

Robin Hood: Legend of Sherwood was also translated into other languages, but sadly neither Steam nor GOG offer these languages, therefore you need modding. Changing Robin Hood's language is simple: basically, you need the language files you want, then you have to place them into the game directory. Finally, you need to delete or rename the English folder, named `2047`, to anything else, for example `2047-1`. Then the game will load any other language files present in the game's directory.

There are some guides dealing with the Polish language, so you can check those in the guides. As for my fellow Czech players, I have created a simple installer which will do all the work for them. The next section will be in Czech.

### Czech

For other Czech players, I created a simple installer that will do all the work for you. First, make sure that you have the game in English; see the illustrated instructions at the beginning. Then simply run the installer, which:

- switches all text and dubbing to Czech;
- changes the appearance of the text to the original, better version;
- can easily be uninstalled at any time.

Download link (about 150 MB):

[Google Drive download](https://drive.google.com/open?id=1DTzmUqLyzhmNAwfnk22BBcQD21xTrqys)

### Proper Text Font

There is also a different text font in the Steam version of the game. No idea why the Steam version uses this one, but the original font is much better.

There is already a guide dealing with changing the font, and you can also check the screenshots posted in this guide (you don't need Russian; the images are in English):

<http://steamcommunity.com/sharedfiles/filedetails/?id=432645606>

The process is very simple: you download the fonts from the provided link in that guide, then copy the `Fonts` folder into the game folder.

Alternatively, you can also download my Czech language installer (see the section above), which installs the fonts too, but then you have to navigate into the game folder and rename the folder `2047_EN` to `2047`; otherwise your game would load in Czech. Renaming the folder will cause the game to load in English.

### Tips

The game's native resolution is 1024x768. The game might look weird on today's 1080p (or higher) displays. If your display offers the option to change resolution from 16:9 to 4:3, use it while you play; it will make your game look better.

You can also play the game in a window, or try using emulating software, but your results may vary.

However, when the game is installed, it defaults to only 800x600 resolution, so the first thing you should do is go into the in-game **Settings → Graphics** and change the resolution to 1024x768.

![Robin Hood graphics settings](https://images.steamusercontent.com/ugc/925927998461232846/707F7B37395C963479B6AF93D62EFF5E2C4BA208/)

That's all, enjoy!

### Comments (16 captured of 16 reported)

#### I am Cornholio !!! — 29 January 2026, 11:00 p.m.

“Thank you, it works.”

#### sewca7 — 7 December 2025, 7:57 p.m.

“Hi, is it possible that the intro does not work because of the Czech installation?”

#### General_Targus — 1 September 2025, 4:30 p.m.

“It works great for the GOG version too! You are a legend, sir!”

#### Kafkyns — 19 August 2025, 11:23 p.m.

“Does Czech still exist for the game?”

#### Wala — 26 November 2024, 11:19 p.m.

“Hi, hopefully you will read this someday, but I have a problem with your Czech installer.

Even after installing it, my game runs in English... could you take a look at it?”

#### TahniDoPekla — 22 June 2024, 11:30 p.m.

“Hi, everything was done according to the instructions and Czech has no diacritics... the two folders Fonts and Fonts_EN are there ... I somehow don't know where to look for the hidden problem..”

#### Macchester92 — 25 May 2023, 3:31 p.m.

“I'm trying to run the game on MacOS Mojave (10.14), and all seems good but the language is stuck to Deutsch. Steam settings show the language is set to English. I tried to find some config in the files to change it manually, however was unable to find anything. Does anybody know any trick to make it run in English?”

#### Emotikon — 29 May 2020, 8:13 p.m.

“In the end I found a solution for implementing both Czech and the performance fix.

1. Install Czech according to the instructions.
2. Google, download, and extract dgVoodoo 2 into the `steamapps/common/Robin Hood` directory.
3. Copy the files from the `MS` folder and paste them into the main Robin Hood directory.
4. Run `dgVoodooCpl.exe` and, in the DirectX tab, check ‘Fast video memory access’.
5. Confirm and close the program.
6. Start the game normally from then on — everything is in Czech, including diacritics, and there are no FPS drops.”

#### Emotikon — 29 May 2020, 6:44 p.m.

“Thanks a lot for the guide! However, diacritics do not work for me either. Is it possible that the performance fix is causing trouble? Without it the game is unplayable for me.”

#### Psojed — 13 April 2020, 1:41 p.m. (author)

“King slayer. Diacritics require you to use a font that supports Czech characters. My installer includes them. I just tried installing the game on Steam and then installing the contents of my installer, and I have the game in Czech with diacritics and the correct font, so the error must be somewhere on your end.

Look in `Steam\\steamapps\\common\\Robin Hood\\DATA\\Interface`; the fonts are stored there. After applying my installer, there should be two folders there, `Fonts` and `Fonts_EN`.”

#### Psojed — 13 April 2020, 1:25 p.m. (author)

“Fabry911 and sid.sethi91. So you install the game on Steam, start the game, and it is in German? Because I installed the game now, but my game installed in English by default.”

#### královrah — 13 April 2020, 10:44 a.m.

“I installed Czech according to the instructions, but all text is missing diacritical characters, which is quite confusing when reading the scrolls. Can anything be done about it?”

#### thenytfox — 2 April 2020, 10:59 p.m.

“I have the same issue as Fabry911. Any help?”

#### Fabry911 — 24 May 2019, 10:36 p.m.

“Hi, I followed your guide; on Steam, English appears selected as the language, but when I play the game, it is still German... how is that possible?”

#### Psojed — 7 February 2019, 10:32 p.m. (author)

#### dzafi — 6 February 2019, 1:54 a.m.

“Thanks for the Czech translation! I was just looking for whether it existed; otherwise I would have created it.”

## Original text

### Robin Hood Language & Texts

By Psojed

A short guide about changing the game language and fonts

### Selecting language

Welcome to my guide!

Robin Hood: Legend of Sherwood is an old game, and as with many old games, it needs some configuration before you can play properly. As the Store page promises, Robin Hood comes in four different languages: English, German, French and Spanish.

To select your preferred language, simply **Right-click** with your mouse on the game in your Steam Library and select **Properties**.

![Steam game Properties menu](https://images.steamusercontent.com/ugc/925927998461118339/6EED6E60B092DC5637904D400F083FC537984A13/)

Then, switch to **Languages** tab and there, select your preferred language from the drop-down menu:

![Steam Languages tab](https://images.steamusercontent.com/ugc/925927998461121465/72D785D0D907E55A4A45C738CCC1F4381068E981/)

That's it. When you launch the game now, it will have the language you selected.

### Other languages

Robin Hood: Legend of Sherwood was also translated into other languages, but sadly neither Steam nor GOG offer these languages, therefore you need modding. Changing Robin Hood's language is simple, basically you need the language files you want, then you have to place them into the game directory. Finally, you need to delete or rename the english folder, named "2047" to anything else, for example "2047-1". Then the game will load any other language files present in the game's directory.

There are some guides dealing with the Polish language, so you can check those in the guides. As for my fellow Czech players, I have created a simple installer which will do all the work for them. The next section will be in Czech.

### Čeština

Pro ostatní České hráče jsem vytvořil jednoduchý instalátor, který udělá veškerou práci za Vás. Nejdřív se ujistěte, že máte hru v angličtině, viz. obrázkový návod na začátku. Poté stačí spustit instalátor, který:

- Veškeré texty a dabing přepne do češtiny
- Změní vzhled textu na původní, lepší
- Lze kdykoliv snadno odinstalovat

DL link (~150MB)

[https://drive.google.com/open?id=1DTzmUqLyzhmNAwfnk22BBcQD21xTrqys](https://steamcommunity.com/linkfilter/?u=https%3A%2F%2Fdrive.google.com%2Fopen%3Fid%3D1DTzmUqLyzhmNAwfnk22BBcQD21xTrqys)

### Proper Text Font

There is also a different text font in the Steam version of the game. No idea why the Steam version uses this one, but the original Font is much better.

There is already a guide dealing with changing the Font, and you can also check the screenshots posted in this guide (you don't need russian, the images are in english):

<http://steamcommunity.com/sharedfiles/filedetails/?id=432645606>

The process is very simple, you download the Fonts from the provided link in that guide, then you copy the Fonts folder into the game folder.

Alternatively, you can also download my Czech language installer (see section above), which installs the fonts too, but then you have to navigate into the game folder and rename the folder "2047_EN" into "2047", otherwise your game would load in Czech. Renaming the folder will cause the game to load in English.

### Tips

The game's native resolution is 1024x768. The game might look weird on today's 1080p (or higher) displays. If your display offers to change resolution from 16:9 to 4:3, use it while you play, it will make your game look better.

You can also play the game in a window, or try using emulating software, but your results may vary.

However, when the game is installed, it defaults to only 800x600 resolution, so the first thing you should do is to go into the ingame Settings -> Graphics, and change the resolution to 1024x768.

![Robin Hood graphics settings](https://images.steamusercontent.com/ugc/925927998461232846/707F7B37395C963479B6AF93D62EFF5E2C4BA208/)

That's all, enjoy!

### Comments

#### I am Cornholio !!! — 29 Jan @ 11:00pm

Děkuji, funguje

#### sewca7 — 7 Dec, 2025 @ 7:57pm

ahoj je možný že kvůli instalaci češtiny nejede intro ?

#### General_Targus — 1 Sep, 2025 @ 4:30pm

Funguje to skvěle i pro GOG verzi! Jsi legenda, pane!

#### Kafkyns — 19 Aug, 2025 @ 11:23pm

Existuje ještě čeština do hry ?

#### Wala — 26 Nov, 2024 @ 11:19pm

Ahoj, snad si to ještě někdy přečteš, ale mám problém s tvým instalátorem češtiny.

I po jeho instalaci mi hra běží v angličtině...mohl by jsi se na něj podívat?

#### TahniDoPekla — 22 Jun, 2024 @ 11:30pm

Ahoj, vše uděláno dle návodu a čeština je bez diakritiky... tyto dvě složky tam jsou Fonts a Fonts_EN ... nějak nevím kde hledat zakopaného psa..

#### Macchester92 — 25 May, 2023 @ 3:31pm

I'm trying to run the game on MacOS Mojave (10.14), and all seems good but the language is stuck to Deutsch. Steam settings show the language is set to English. I tried to find some config in the files to change it manually, however was unable to find anything. Does anybody know any trick to make it run in English?

#### Emotikon — 29 May, 2020 @ 8:13pm

Tak nakonec jsem nalezl řešení, jak implementovat češtinu i performance fix.

1) nainstalovat češtinu podle návodu
2) vygooglit, stáhnout a rozbalit program dgVoodoo 2 do adresáře steamapps/common/Robin Hood
3) ze složky "MS" zkopírovat soubory a vložit do hlavního adresáře Robin Hood
4) spustit dgVoodooCpl.exe a v záložce DirextX zaškrtnout 'Fast video memory access'
5) potvrdit a zavřít program
6) spouštět hru už normálním způsobem - vše v češtině včetně diakritiky a bez FPS dropů

#### Emotikon — 29 May, 2020 @ 6:44pm

Díky moc za návod! Nicméně mi taktéž nefunguje diakritika. Je možné, že dělá neplechu performance fix? bez něj je pro mne hra nespustitelná.

#### Psojed — 13 Apr, 2020 @ 1:41pm (author)

královrah. Diakritika potřebuje abys používal font který umí české znaky. Můj instalátor je obsahuje. Zkusil jsem teď instalovat hru na Steamu a pak instalovat obsah mého instalátoru, a mám hru česky i s diakritikou a správným fontem, takže chyba bude někde u tebe.

Mrkni se do Steam\\steamapps\\common\\Robin Hood\\DATA\\Interface, tam jsou uložené fonty. Po aplkaci mého instalátoru by tam měly být dvě složky, Fonts a Fonts_EN.

#### Psojed — 13 Apr, 2020 @ 1:25pm (author)

Fabry911 and sid.sethi91. So you install the game on Steam, start the game, it is in German? Beucase I installed the game now, but my game installed in English by default.

#### královrah — 13 Apr, 2020 @ 10:44am

Češtinu jsem si dle návodu nainstaloval, ale veškeré texty postrádají znaky s diakritikou, což je při čtení pergamenů docela matoucí. Dá se s tím něco dělat?

#### thenytfox — 2 Apr, 2020 @ 10:59pm

I have the same issue as Fabry911. Any help?

#### Fabry911 — 24 May, 2019 @ 10:36pm

hii, i followed your guide, on steam english result like the language selected, but when i play the game, it's still german...how it is possible?

#### Psojed — 7 Feb, 2019 @ 10:32pm (author)

#### dzafi — 6 Feb, 2019 @ 1:54am

Díky za tu češtinu! Zrovna jsem hledal jestli to existuje jinak bych ji vytvořil.
