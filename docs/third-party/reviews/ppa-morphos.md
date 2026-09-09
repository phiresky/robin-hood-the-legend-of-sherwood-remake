# PPA — Polish review of the MorphOS port

- Original source: [PPA — Polish review of the MorphOS port](https://www.ppa.pl/gry/robin-hood-the-legend-of-sherwood.html)
- Author / publication: Grzegorz Murdzek / PPA.pl (Polski Portal Amigowy)
- Language / date: Polish; 2011-05-19
- Access: Full page retrieved directly
- Checked: 2026-09-09
- Retrieved: 2026-09-09
- Archived copy: [Wayback Machine, 2024-12-28](https://web.archive.org/web/20241228072124/https://www.ppa.pl/gry/robin-hood-the-legend-of-sherwood.html)
- Format: complete English translation followed by the preserved Polish article text

## English translation

### Robin Hood — The Legend of Sherwood

*19 May 2011, 18:37 — article author: Grzegorz Murdzek*

England, the year 1190. In the absence of King Richard the Lionheart, who is fighting in a crusade in the Holy Land, all power is brutally exercised by his brother, Prince John Lackland, who is plotting evil plans with the Sheriff of Nottingham. Only one man, together with a group of loyal companions, is ready to fight the oppressors of the poor English; they call him Robin Hood.

This simple and uncomplicated introduction promises some time-killing fun, spent taking the role of the “man in the hood” in the action-adventure game *Robin Hood: The Legend of Sherwood*—one of the few commercial game conversions released for MorphOS. At least several games, many films, and television series have been made about the legendary outlaw of Sherwood Forest and his band (including *Robin of Sherwood*, the BBC series with an extraordinary atmosphere created by music from the band Clannad). Among games released for Amiga are titles such as [*The Adventures of Robin Hood*](http://www.ppa.pl/artykul-the.adventures.of.robin.hood-7_15_1153.html) and *Conquests of the Longbow*.

### What is in the box?

When I received the package containing the game, I was surprised by what I saw: a German-language cover with “Linux CD-ROM” printed at the very top. I was already about to call to complain, but fortunately a modest requirements imprint calmed me down; against a white background with a blue butterfly it said “MorphOS CD-ROM.” The cover itself also turned out to be double-sided, with the other side printed in English. The same applies to the short instruction manual (expanded versions are available on the game disc as PDF files), written in English and German (there is no Polish version). The MorphOS game was released by Runesoft (formerly Epic Interactive); earlier, *Robin Hood* was available for platforms such as Windows, Mac, and Linux (the manual contains a few sentences about requirements and installation only for the latter two).

### Requirements

The game should run normally on a Pegasos with a PowerPC G3 processor, although a G4 is recommended. As for memory, the minimum is 128 MB of RAM, and the graphics cards recommended by the publisher are any supported Radeon, Voodoo, or Permedia. The game needs up to 1 GB of hard-drive space and a CD-ROM drive, since it is distributed on that medium.

### On the way to Sherwood Forest

Ready to fight in the name of good and justice, I began with something banal: installing the game on the hard drive. I would not mention this were it not for the problems I encountered, which occur in the MorphOS version. After clicking the English- or German-version installation icon, the installer first reports the previously mentioned free-space requirements and the recommended file system for the partition on which the game will be installed.

![Installer screenshots: Robin 1 and Robin 2](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_1.png) ![Installer screenshots: Robin 2](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_2.png)

Since I had everything needed (a Pegasos II G4, 1024 MB of RAM, a Radeon 9250, and plenty of room on the hard drive, where all my MorphOS partitions are SFS), I went one step further. After clicking “Continue,” I selected the place on the drive where the game was to be installed and… unfortunately saw what is shown in the following picture:

![Installer screenshots: Robin 3 and Robin 4](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_3.png) ![Installer screenshot: Robin 4](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_4.png)

![Installer screenshot: Robin 5](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_5.png) Thus it turned out that the player’s first task would not be fighting the Sheriff’s soldiers, but fighting the broken installation script supplied with the game. To deal with the problem:

- Copy the **untarka**, **Install_Robin_English**, and **Install_Robin_English.info** files (or the German version of the script) somewhere such as the RAM Disk.
- Click the “Executable” tick box for **untarka**.
- Click the “Writable” tick box for **Install_Robin_English**.
- Edit **Install_Robin_English**, for example with the system editor **Ed** (`SYS:Utilities/Ed`), changing `_RobinCD:untarka "RobinCD:Robin_main.tar.bz2"_` to `_RAM:untarka "RobinCD:Robin_main.tar.bz2"_`, and `_RobinCD:untarka "RobinCD:Robin_english.tar.bz2"_` to `_RAM:untarka "RobinCD:Robin_english.tar.bz2"_`.

We can then start the installer normally from its icon, but of course we launch the script copied to the RAM Disk.

The inquisitive will surely have noticed that installing the game is nothing more than unpacking two archives. This does not make the installation problem any less troublesome for someone who does not realize that it is not enough simply to click three times and that a little tinkering is also required.

After installation, the game directory contains—besides the standard files appropriate to the title—libraries specific to MorphOS. The game uses the multiplatform SDL library, so the MorphOS version requires `powersdl.library` to run. Version 11.11 is supplied with the game; it is very old, so anyone who already has a newer version of that library in the system can even remove the `Libs` directory from the game directory.

### Action!

![Robin 6](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_6.png) The game’s introduction is a short rendered 3D animation presenting the agility of the titular Robin Hood as he fights the Sheriff’s men. An introductory movie is standard in this kind of production. From the intro we move to the main game menu, accompanied by somewhat sad, slow, but atmospheric music. The main part of the start screen shows statistics for the game so far. As time passes and the game progresses, the information is updated. The most important statistics are money, game points, the percentage of enemies killed, time spent playing, and progress through the game. On the left side of the main menu there are various options, including player selection (the equivalent of a profile or account), so we can play under our own account while another household member plays on a different profile—an extra advantage on a single-user system such as MorphOS. The options also include loading a game (fortunately, a game can be saved at almost any time, with no mission-code system), sound and graphics settings, viewing movies that appeared during the game (such as the intro), viewing the credits (in case someone does not make it to the end credits), and the simple options to start the game or exit to… “Windows.”

![Robin 7](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_7.png) The sound and graphics settings give the user basic ways to set priorities and tune game performance. For sound, these include adjusting the volume of effects, music, dialogue, and commentary, and enabling music in stereo or 3D mode. In the graphics settings, one can choose among three resolutions (648×480, 800×600, and 1024×768). The higher the resolution, the larger the field of view and therefore the more convenient the game, although that may be a subjective impression. In addition to resolution, some extra eye-pleasing but somewhat hardware-intensive effects can be enabled or disabled, such as displaying enemies’ fields of vision, shadow transparency, animations for certain effects, and background animation. If the game feels sluggish, you can return and turn effects off and perhaps lower the resolution to speed it up. There is also an option to change the default keyboard shortcuts used in the game and restore their defaults.

![Robin 8](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_8.png) *Robin Hood: The Legend Of Sherwood* gives the player three difficulty levels (Easy, Medium, and Hard), differing mainly in the ratio of our characters’ strength and agility to those of the Sheriff’s soldiers. I do not recommend the lowest difficulty: the game is then trivially easy to complete, the opponents are very weak, and one can escape practically any trouble without major losses; energy also regenerates, and very quickly. The game is divided into missions, some mandatory and the others (mainly raids on the Sheriff’s soldiers and gold convoys) serving chiefly to replenish the band’s treasury. In the first two missions we learn the story of Robin’s return from Locksley and assemble the band. The gameplay is primarily action-oriented, interspersed with dialogue and comments/hints about newly discovered details needed to complete the game. There are no complicated puzzles or need to click every pixel to find an extremely well-hidden passage. That does not mean, of course, that completing a mission does not require a great deal of walking, running, climbing, and sneaking. You can conduct the game—especially missions in castles occupied by soldiers loyal to the Sheriff—carefully, mainly sneaking behind an enemy and knocking him out with a fist (or strangling him, depending on the particular character’s abilities), and only if discovered taking up a melee weapon or using a bow. Alternatively, you can go all in and try to kill everyone in your path; this is harder because the soldiers quickly call for support, and even if you manage the hand-to-hand fighting, archers or crossbowmen will certainly get you.

![Robin 9](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_9.png) The game is controlled with the mouse, with optional keyboard-shortcut support. The controls are comfortable and need no getting used to, but it is worth studying the manual carefully so as not to miss conveniences in operating the game. The authors boast of an innovative combat system. To be honest, I find it just as innovative as it is unintuitive, somewhat reminiscent of learning so-called mouse gestures. Fighting with a sword or any melee weapon sometimes looks comical; it lacks the action’s sharpness and excitement, and the characters’ combat movements seem anaemic and too simple. Ultimately, however, one can get used to it. The interface is supplemented by a terrain map (with characters marked in appropriate colours) and the ability to zoom the play area in and out. It is also worth mentioning the various emotion icons shown above the characters. Their meaning is easy to interpret, but anyone with doubts can consult the manual.

At first we can issue orders only to Robin, but as missions progress and the band is assembled, it becomes possible to control more characters simultaneously. Another interesting feature is so-called quick actions (up to five actions) that can be queued for execution. This provides additional possibilities such as distracting an enemy, carrying out a diversion, or making a combined attack. In total, we can control up to nine different characters, who may join us in successive missions. In the meantime we learn each one’s traits (the manual’s description of every character’s traits and abilities is an additional convenience). Everyone who joins the band specializes in fighting with different weapons, can use artefacts intended only for that character, and may even have different culinary preferences. For example, Robin Hood can use a sword (in castle missions), a staff, and a bow. He can also sneak, climb, and jump over roofs and various obstacles, knock out an opponent with a powerful blow when he approaches unnoticed, or scatter purses of money to distract attention. Many of these specializations become useful in particular missions if we choose a difficulty higher than the easiest. There is no point describing them all here; I suggest discovering their significance in the game yourself.

*Source: YouTube.*

![Robin 10](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_10.png) During the game, various scrolls will certainly attract our attention. Reading them provides many hints and assistance. A blue scroll tells us which character can use a given item scattered around the map. It also tells us what the item is for, as well as the purpose of particular locations in the game. Similarly, gameplay hints about mission details are contained in scrolls tied with a red ribbon. To make the gameplay even more varied, the authors introduced five skill-training levels for melee combat and bow use (for characters with that ability). Each successive level increases weapon proficiency and consequently damage. After completing the first two missions, our band returns to Sherwood Forest, where it has its headquarters. This is a special place to which you return after every subsequent mission and where you can perform various actions and issue orders, such as collecting medicinal herbs, training in combat or archery, and preparing food. Since not all companions will always participate in a mission, those left at headquarters can perform these tasks instead of sitting idle. You decide which mission comes next by selecting it on the map screen in the upper-right corner.

### Graphics

![Robin 11](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_11.png) Most importantly, the graphics do not tire the eyes with excessive contrast. That is a major plus given how precisely every detail is rendered and how pleasing it looks. One can object only to the controlled heroes blending into the background, which makes them hard to see when obscured by trees. The animation is lively, and details such as background animations give it additional life. Another advantage is that the game is entirely 2D—no 3D, rotating cubes, or similar gimmicks. This is a typical action game, and the eye can focus on the game and its details. We run, shoot, and fight; the game proceeds smoothly in accordance with our orders (unless someone has too weak a configuration), with no graphical defects or other shortcomings. Graphically the game is complete, and although this is not a production in which, as in the old days, everything was painted pixel by pixel (rather, we are dealing with elements prepared using 3D software but cleverly embedded in a 2D world), it leaves the impression that every detail was carefully painted by an artist. The characters, especially in dialogue views, look comical to me. They are like cartoon characters, pretty and well-groomed, led by Robin Hood, who looks as if he has just left a beauty salon.

### Music and sound

The background music is competently made, but quickly becomes boring and gives the whole game a monotonous atmosphere. Changing the theme during a fight with an enemy does not help. There is nothing to criticize in the sound effects; they perfectly reproduce everything one would want to hear. The only flaw for me was when frightened residents repeatedly shouted for help in exactly the same way—at that point I turned down the speaker. The game also contains spoken dialogue, which likewise cannot be faulted (apart from the hero’s facial animation not reflecting what we hear).

### Is it worth playing?

![Robin 12](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_12.png) *Robin Hood*’s playability depends on the player’s approach. If the lowest difficulty is chosen, the player will quickly become bored and finish the game in half a day without effort. At higher difficulty levels the game becomes more varied and demanding, and can therefore hold one’s interest longer. The creators tried to interest the player and vary the gameplay through a range of different artefacts and heroes’ special traits, but unfortunately less through mission variety. Despite containing new elements as one strives to complete the game, later missions become boring because tasks are performed much as before, while the curiosity of discovery slides downhill. A decent enemy intelligence is certainly an interesting element (although it is really what one might call insect intelligence): an enemy can not only flee in fear when seeing our advantage, but also call reinforcements from a distant part of the location and try to find us. We kill a lot in the game, but it is not excessively brutal (nobody performs a “fatality,” although two characters can finish off someone lying down), which may be a drawback for some and an advantage for others. It is certainly an easy and pleasant game, but unfortunately also one that people complete once and never return to. It is one-off entertainment; the only question is whether it is worth the price. It is certainly the best commercial game (or rather, game conversion) released specifically for MorphOS, so if you are a fan of the system, it is worth taking an interest in this title (even if only by trying the demo). Unfortunately, the game is very unstable, which harms its playability: it can freeze at almost any moment; the game screen freezes and only music can be heard, so it is worth saving the game often.

*Robin Hood: The Legend of Sherwood* was created by Spellbound Studios and released for leading platforms by Wanadoo in November 2002. The MorphOS version [was released in October 2006](http://www.ppa.pl/robin.hood.the.legend.of.sherwood,4764;aktualnosci.html) by RuneSoft. A demo version is available on the publisher’s website.

The game can be ordered from the Polish Wupra shop and the German Vesalia shop.

MorphOS game page: <http://www.rune-soft.com/product.php?product_id=20>  
Demo version: <http://www.rune-soft.com/downloads/robin_demo.lha>

Wupra shop: <http://wupra.com/morphos/80-robin-hood-the-legend-of-sherwood-morphos.html>  
Vesalia shop: <http://www.vesalia.de/e_robinhood.htm>

### Rating and comments metadata

| Rating item | Score | Details |
| --- | ---: | --- |
| Graphics | 100 | 1 CD |
| Sound | 80 |  |
| Gameplay | 75 |  |

The page records 7 comments; the latest is dated 28 May 2011, 18:15. The retrieved article page contains no comment bodies or author names, only this count and latest-date link.

## Original text

### Robin Hood - The Legend of Sherwood

*19.05.2011 18:37, autor artykułu: Grzegorz Murdzek.*

![Robin Hood - The Legend of Sherwood logo](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_logo.png)

![Robin Hood scene](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_13.png) Anglia, rok 1190. Pod nieobecność walczącego w krucjacie w Ziemi Świętej króla Ryszarda Lwie Serce całą władzę brutalnie sprawuje jego brat - książe John Lackland, z którym knuje niecne plany Szeryf z Nothingham. Tylko jeden człowiek wraz z grupą wiernych kompanów jest gotów stanąć do walki z ciemiężycielami biednych Anglików - zwą go Robin Hood. 

Taki oto prosty i nieskomplikowany wstęp zapowiada zabijanie czasu, który spędzić można wcielając się w "człowieka w kapturze" w zręcznościowo-przygodowej grze "Robin Hood: The Legend of Sherwood" - jednej z nielicznych komercyjnych konwersji gier wydanych dla systemu MorphOS. O przygodach legendarnego banity z lasu Sherwood oraz jego drużyny powstało przynajmniej kilka gier, wiele filmów i telewizyjnych seriali (w tym Robin of Sherwood, serial produkcji BBC o niesamowitym klimacie, który nadawała produkcji muzyka zespołu Clannad). Wśród gier wydanych na Amigę są to takie tytuły jak ["The Adventures of Robin Hood"](http://www.ppa.pl/artykul-the.adventures.of.robin.hood-7_15_1153.html) czy "Conquests of the Longbow". 

### Co w pudełku?

Po odbiorze przesyłki z grą byłem zdziwiony tym, co zobaczyłem. Okładka w języku niemieckim, z napisem na samej górze "Linux CD-ROM". Już miałem dzwonić po reklamację, ale na szczęście uspokoił mnie skromny nadruk z wymaganiami, gdzie na białym tle z niebieskim motylem widniał napis "MorphOS CD-ROM". Sama okładka okazała się być również dwustronna z zadrukowaną częścią w języku angielskim. Podobnie jest ze wstępną instrukcją obsługi (wersje rozszerzone są dostępne na płycie z grą w postaci plików PDF) napisaną w języku angielskim i niemieckim (wersji polskiej brak). Grę dla MorphOS-a wydała firma Runesoft (dawniej Epic Interactive), a wcześniej "Robin Hood" był dostępny na platformy takie jak Windows, Mac czy system Linux (w instrukcji znajdziemy kilka zdań o wymaganiach i instalacji tylko dla tych dwóch ostatnich). 

### Wymagania

Gra powinna działać normalnie na Pegasosie z procesorem PowerPC G3, choć zalecane jest G4. Jeśli chodzi o pamięć, minimum to 128 MB RAM, a karta graficzna, jaką poleca wydawca to dowolny obsługiwany Radeon, Voodoo bądź Permedia. Gra potrzebuje do 1 GB miejsca na twardym dysku oraz napędu CD-ROM (bo na takim nośniku jest rozprowadzana gra). 

### W drodze do lasu Sherwood

Gotowy do walki w imię dobra i sprawiedliwości rozpocząłem od rzeczy banalnej - instalacji na dysku twardym. Nie wspomniałbym tutaj o tym, gdyby nie problemy, jakie napotkałem a które występują w przypadku wersji gry dla MorphOS-a. Po kliknięciu w ikonkę instalacji wersji angielskiej lub niemieckiej skrypt instalacyjny najpierw informuje o wspomnianych wcześniej wymaganiach wolnej przestrzeni dyskowej, a także zalecanym systemie plików na partycji, na której zostanie zainstalowana gra. 

![Zrzut instalatora 1](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_1.png) ![Zrzut instalatora 2](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_2.png)

Ponieważ miałem wszystko, co było potrzebne (Pegasos II G4, 1024 MB RAM, Radeon 9250 oraz spory zapas miejsca na dysku, na którym wszystkie partycje dla MorphOS-a mam w SFS), poszedłem krok dalej. Po kliknięciu w "Continue" wybrałem miejsce na dysku, gdzie zainstalowana miała zostać gra i... niestety zobaczyłem to, co na poniższym obrazku: 

![Zrzut instalatora 3](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_3.png) ![Zrzut instalatora 4](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_4.png)

![Installer screenshot 5](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_5.png) Tym samym okazało się, że pierwszym zadaniem dla gracza będzie walka nie z żołnierzami Szeryfa z Nothingham, a z zepsutym skryptem instalacyjnym, jaki otrzymał wraz z grą. Aby poradzić sobie z tym problemem, należy:  
\- przekopiować pliki **untarka** oraz **Install_Robin_English** i **Install_Robin_English.info** (lub wersję niemiecką skryptu) np. do Ram Dysku,  
\- kliknąć w "ptaszka" przy "Uruchamialny" dla pliku **untarka** ,  
\- kliknąć w "ptaszka" "Zapisywalny" dla pliku **Install_Robin_English** ,  
\- zmodyfikować plik **Install_Robin_English** np. systemowym edytorem **Ed** (SYS:Utilities/Ed) i zamienić linijkę:  

_RobinCD:untarka "RobinCD:Robin_main.tar.bz2"_  

na:  

_RAM:untarka "RobinCD:Robin_main.tar.bz2"_  

oraz  

_RobinCD:untarka "RobinCD:Robin_english.tar.bz2"_  

na:  

_RAM:untarka "RobinCD:Robin_english.tar.bz2"_. 

Następnie możemy uruchomić skrypt instalacyjny normalnie z ikonki, ale uruchamiamy oczywiście ten skrypt, który skopiowany został do Ram Dysku. 

Dociekliwi zapewne zauważyli, że instalacja gry to nic innego jak rozpakowanie dwóch archiwów, co nie umniejsza problemów z instalacją gry w przypadku, gdyby ktoś nie zorientował się, że nie wystarczy tylko trzy razy kliknąć, a trzeba jeszcze trochę pogrzebać. 

Po zakończonej instalacji w katalogu gry - poza plikami standardowymi, właściwymi dla tytułu - są również dostarczone specyficzne biblioteki dla systemu MorphOS. Gra korzysta z dobrodziejstw multiplatformowej biblioteki SDL, a więc w przypadku wersji dla systemu MorphOS do działania wymaga powersdl.library. Razem z grą dostajemy wersję 11.11, która jest wersją bardzo starą, więc jeśli ktoś już ma w systemie nowszą wersję wspomnianej biblioteki, może nawet wyrzucić z podkatalogu z grą katalog Libs. 

### Akcja!

![Robin Hood intro](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_6.png) W klimat gry wprowadza introdukcja będąca krótką, wyrenderowaną animacją 3D, która przedstawia zwinność tytułowego Robin Hooda w walce z ludźmi Szeryfa. Filmik wprowadzający to standard w tego typu produkcjach. Z intra przenosimy się do menu głównego gry, w którym przygrywa nam nieco smutna, powolna, ale klimatyczna muzyka. W głównej części ekranu startowego podana jest statystyka dotychczasowej rozgrywki. W miarę upływu czasu i postępu w grze informacje są aktualizowane. W grze najważniejsze dane statystyczne to pieniądze, punkty gry oraz procent zabitych wrogów, czas spędzony na grze i postęp zaawansowania przejścia gry. W głównym menu po lewej stronie ekranu znajdziemy szereg opcji, a wśród nich możliwość wyboru gracza (odpowiednik profilu lub konta), dzięki czemu możemy grać na swój rachunek, a np. kto inny z domowników może grać na innym profilu, co przy jednoużytkownikowym systemie, jakim jest MorphOS, ma dodatkową zaletę. W opcjach znajdziemy również m. in. możliwość odczytu stanu gry (na szczęście stan gry można prawie w dowolnym momencie zapisać i nie ma tutaj żadnych rozwiązań typu kod do misji itp.), ustawienia dźwięku i grafiki, możliwość wyświetlenia filmików, które wystąpiły podczas gry (jak np. wspomniane intro), możliwość obejrzenia listy płac (na wypadek, gdyby ktoś nie dotrwał do napisów końcowych), a także banalne opcje rozpoczęcia gry i wyjścia z gry do... "Windows". 

![Robin Hood settings](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_7.png) Wspomniane ustawienia dźwięku i grafiki dają użytkownikowi podstawowe możliwości ustawienia priorytetów i dostrojenia wydajności gry. Dla dźwięku będzie to odpowiednio możliwość regulacji głośności efektów, muzyki, dialogów oraz komentarzy, możliwość włączenia muzyki w trybie stereo lub 3D. W ustawieniach graficznych można wybrać spośród trzech rozdzielczości (648x480, 800x600 oraz 1024x768). Im wyższa rozdzielczość tym większe pole widzenia, a co za tym idzie wygodniejsza gra, ale może to być wrażenie subiektywne. Poza rozdzielczością można również włączyć lub wyłączyć niektóre dodatkowe miłe dla oka, ale obciążające nieco sprzęt efekty graficzne takie jak wyświetlanie pola widzenia wrogów, przezroczystość cieni, animacje niektórych efektów oraz animacje tła. Jeśli poczujesz, że gra się ślimaczy zawsze możesz wrócić i powyłączać efekty i ewentualnie zmniejszyć rozdzielczość w celu przyspieszenia gry. Poza tym istnieje jeszcze możliwość zmiany domyślnych skrótów klawiaturowych występujących w grze oraz możliwość powrotu do ich ustawień domyślnych. 

![Robin Hood gameplay](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_8.png) "Robin Hood: The Legend Of Sherwood" daje graczowi do wyboru trzy poziomy trudności (Easy, Medium oraz Hard), które różnią się od siebie głównie stosunkiem siły i zręczności naszych postaci do żołnierzy Szeryfa z Nothingham. Nie polecam wyboru najniższego poziomu trudności - albowiem wtedy gra jest banalnie prosta do przejścia, przeciwnicy są bardzo słabi i praktycznie z każdych opałów można wyjść bez większych strat, a dodatkowo energia odrasta i to bardzo szybko. Gra podzielona jest na misje, których część jest obowiązkowa, a pozostałe (głównie napadanie na żołnierzy szeryfa i konwoje ze złotem) służą głównie do podreperowania skarbca drużyny. W pierwszych dwóch misjach poznajemy historię powrotu Robina z Locksley i kompletujemy drużynę. Gra ma przede wszystkim zręcznościowy charakter rozgrywki, który przeplatany jest dialogami i komentarzami/wskazówkami o kolejnych odkrytych szczegółach potrzebnych do ukończenia gry. Nie ma tutaj żadnych skomplikowanych zagadek czy konieczności klikania w każdy piksel ekranu, aby odnaleźć bardzo drobiazgowo ukryte przejście, co oczywiście nie oznacza, że nie trzeba się dużo nachodzić, nabiegać, wspinać i skradać, by przejść konkretną misję. Rozgrywkę, zwłaszcza w misjach na terenie zamków opanowanych przez żołnierzy wiernych Szeryfowi, możesz prowadzić ostrożnie - głównie skradając się za plecy wroga, ogłuszając go pięścią (lub np. dusząc, zależnie od możliwości konkretnej postaci), a tylko w razie zdemaskowania podjąć walkę bronią białą lub użyć łuku albo pójść na żywioł i spróbować zabić każdego, kto stanie nam na drodze (zadanie trudniejsze o tyle, że żołnierze szybko zwołują wsparcie i nawet jeśli dasz radę w zabawie z walce wręcz, to na pewno dopadną Cię łucznicy lub kusznicy). 

![Robin Hood combat](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_9.png) W grze sterujemy myszką przy opcjonalnym wsparciu skrótów klawiaturowych. Sterowanie jest wygodne i nie trzeba się do niego przyzwyczajać, ale warto zapoznać się szczegółowo z instrukcją, aby nie pominąć udogodnień w obsłudze gry. Autorzy gry chwalą się innowacyjnym systemem walki zastosowanym w grze. Przyznam szczerze, że jest on dla mnie innowacyjny tyle samo, co nieintuicyjny, przypominający nieco opanowanie tzw. gestów myszy. Sama walka mieczem lub jakąkolwiek bronią białą wygląda czasami komicznie i brak w niej zaciętości akcji i emocji, ruchy postaci w walce wydają się anemiczne i zbyt proste, ale ostatecznie można się do tego przyzwyczaić. Interfejs obsługi uzupełnia możliwość skorzystania z mapy terenu (z zaznaczonymi odpowiednim kolorem postaciami) oraz możliwość zbliżania lub oddalania pola gry. Dodatkowo warto wspomnieć, że w grze występują różne ikonki emocji, które znajdują się nad postaciami występującymi w grze. Ich znaczenie łatwo zinterpretować, ale gdyby ktoś miał wą tpliwości może zajrzeć do instrukcji. 

Rozkazy wydawać możemy na początku tylko Robinowi, ale wraz z kolejnymi misjami, w miarę skompletowania drużyny, pojawi się możliwość sterowania większą liczbą postaci jednocześnie. Dodatkową ciekawostką w grze są tzw. szybkie akcje (do 5 akcji), które można wyznaczyć do wykonania. Daje to dodatkowe możliwości, takie jak odciągnięcie uwagi wroga, dywersja lub kombinowany atak. Ogólnie w grze będziemy mogli sterować maksymalnie dziewięcioma różnymi postaciami, które będą mogły dołączyć do nas w kolejnych misjach. W międzyczasie poznamy cechy każdej z nich (dodatkowym ułatwieniem jest opis cech i możliwości każdej z postaci w instrukcji obsługi). Każdy, kto dołączy do drużyny, specjalizuje się w walce różną bronią, potrafi korzystać z sobie tylko przeznaczonych artefaktów, a nawet może mieć inne upodobania kulinarne. Przykładowo Robin Hood może posługiwać się mieczem (w misjach na zamkach), kosturem oraz łukiem. Potrafi również skradać się, wspinać i skakać po dachach i różnych przeszkodach czy też ogłuszać przeciwnika mocnym uderzeniem (gdy zajdzie go niepostrzeżenie) lub rozrzucać sakiewki z pieniędzmi w celu odciągnięcia uwagi. O ile wybierzemy wyższy poziom trudności niż najłatwiejszy, wiele z tych specjalizacji przyda się w konkretnych misjach. Nie ma sensu w tym miejscu opisywać ich wszystkich, proponuję samemu odkrywać ich znaczenie w grze. 

Źródło: YouTube 

![Robin Hood scrolls](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_10.png) W trakcie rozgrywki naszą uwagę na pewno przykują różne zwoje. Czytając je, uzyskamy wiele wskazówek i pomocy. I tak, zwój niebieski pozwala dowiedzieć się o tym przez jaką postać dany, rozrzucony po mapie przedmiot, może być wykorzystany. Dowiemy się również o jego przeznaczeniu, ale także o celowości istnienia konkretnych lokalizacji w grze. Analogicznie wskazówki w rozgrywce dotyczące szczegółów misji są zawarte w zwojach zawiązanych czerwoną kokardką. Aby rozgrywkę jeszcze bardziej urozmaicić, autorzy wprowadzili pięć poziomów wyszkolenia umiejętności w walce bronią białą oraz posługiwania się łukiem (dla tych postaci, które mają taką umiejętność). Kolejne poziomy zwiększają skuteczność posługiwania się bronią i co za tym idzie, siłę obrażeń. Po przejściu dwóch pierwszych misji nasza drużyna wraca do lasu Sherwood, w którym ma swoje centrum dowodzenia. Jest to miejsce szczególne, do którego będziesz wracać po każdej z kolejnych misji i w którym możesz wykonać szereg różnych akcji i rozkazów, jak np. polecenie zbierania ziół leczniczych, treningu w umiejętności walki bądź strzelania z łuku, przygotowywanie jedzenia itp. Z racji tego, że nie zawsze wszyscy nasi kompani będą uczestniczyć w misji, pozostawieni w kwaterze głównej mogą, zamiast siedzieć bezczynnie, wykonywać te zadania. O tym która misja będzie następna, decydujesz sam, dokonując wyboru z ekranu mapki w prawym górnym rogu. 

### Oprawa graficzna

![Robin Hood graphics](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_11.png) Co najważniejsze grafika nie męczy wzroku zbyt dużymi kontrastami. To duży plus biorąc pod uwagę to, że każdy szczegół jest odwzorowany dokładnie i wygląda mile dla oka. Można mieć jedynie zastrzeżenia co do wtapiania się w tło sterowanych bohaterów, przez co zasłonięci drzewami są słabo widoczni. Animacja w grze jest żywa, a życia dodatkowo nadają jej szczegóły, takie jak animacje w tle. Zaletą jest również to, że gra jest w całości w 2D, żadnego trójwymiaru, obracania sześcianów i tym podobnych cudów na kiju. To typowa gra akcji i wzrok może się skupić na grze i jej szczegółach. Biegamy, strzelamy, walczymy - gra toczy się płynnie, zgodnie z naszymi rozkazami (chyba że ktoś posiada zbyt słabą konfigurację), nie ma żadnych defektów graficznych czy innych niedoróbek. Gra jest kompletna, jeśli chodzi o grafikę i choć nie jest to produkcja, w której, jak za dawnych czasów, wszystko zostało namalowane piksel-w-piksel (raczej mamy tu do czynienia z elementami przygotowanymi za pomocą oprogramowania 3D, lecz sprytnie zatopionymi w świecie 2D), to pozostaje wrażenie, jakby każdy detal był dokładnie malowany przez grafika. Postaci w grze, zwłaszcza w widoku dialogów, wyglądają jak dla mnie komicznie. Są jak postacie z bajki, ładni i zadbani na czele z Robin Hoodem, który wygląda, jakby właśnie wyszedł z salonu piękności. 

### Muzyka i dźwięk

Muzyka w tle jest wykonana poprawnie, ale szybko się nudzi i nadaje klimatu monotonii całej grze. Nie pomaga zmiana motywu podczas walki z wrogiem. Do efektów dźwiękowych nie można się przyczepić, doskonale odwzorowują wszystko, co chciałoby się usłyszeć. Jedynym mankamentem dla mnie były sytuacje, gdy spłoszeni mieszkańcy krzyczeli po pomoc w kółko tak samo raz za razem - w tym momencie ściszałem głośnik. W grze występują również dialogi mówione, którym również nie można niczego zarzucić (poza tym, że animacja twarzy bohatera nie odzwierciedla tego, co słyszymy). 

### Czy warto grać?

![Robin Hood review](https://www.ppa.pl/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_12.png) Grywalność Robin Hooda zależy od podejścia gracza. Jeśli wybierze najniższy poziom trudności, szybko się znudzi i przejdzie grę w pół dnia bez wysiłku. W przypadku wyższych poziomów trudności gra staje się bardziej urozmaicona i wymagająca, przez co może wciągnąć na dłużej. Twórcy starali się zaciekawić gracza i urozmaicić rozgrywkę przez szereg różnych artefaktów, cech szczególnych bohaterów, ale niestety mniej przez różnorodność misji, które pomimo zawierania elementów nowych w dążeniu do przejścia gry w kolejnych odsłonach nudzą, bo zadania wykonuje się podobnie jak dotychczas, ale ciekawość odkrywania spada równią pochyłą w dół. Na pewno ciekawym elementem gry jest przyzwoita inteligencja przeciwnika (choć jest to raczej tzw. inteligencja owada) - wróg potrafi nie tylko uciec ze strachu, widząc naszą przewagę, ale też wezwać posiłki z odległej części danej lokalizacji i spróbować nas odnaleźć. W grze dużo zabijamy, ale nie jest przesadnie brutalna (nikt nikomu nie robi "fatality", ale za to dwie z postaci potrafią dobić leżącego), co może być dla jednych wadą, a dla innych zaletą. Na pewno jest to gra należąca do tych, które są łatwe i przyjemne, ale niestety również do takich, które przechodzi się raz i nie wraca już nigdy więcej. To rozrywka jednorazowa, pytanie tylko czy warta swojej ceny? Na pewno jest to najlepsza komercyjna gra (a właściwie konwersja gry) wydana specjalnie dla MorphOS-a, więc jeśli jesteś fanem tego systemu warto zainteresować się tym tytułem (choćby poprzez wypróbowanie wersji demonstracyjnej). Gra jest niestety bardzo niestabilna, co negatywnie wpływa na grywalność - gra potrafi się zawiesić niemal w dowolnym momencie, zamraża się ekran gry i słychać tylko muzykę, dlatego warto często zapisywać stan gry. 

"Robin Hood: The Legend of Sherwood" to gra stworzona przez Spellbound Studios, a wydana na wiodące platformy przez Wanadoo, w listopadzie 2002 roku. Wersja dla MorphOS-a [została wydana w październiku 2006 roku](http://www.ppa.pl/robin.hood.the.legend.of.sherwood,4764;aktualnosci.html) przez firmę RuneSoft. Na stronie wydawcy dostępna jest wersja demonstracyjna gry. 

Grę można zamówić w polskim sklepie Wupra, a także w niemieckim sklepie Vesalia. 

Strona gry w wersji dla MorphOS: <http://www.rune-soft.com/product.php?product_id=20>  
Wersja demo: <http://www.rune-soft.com/downloads/robin_demo.lha>

Sklep Wupra: <http://wupra.com/morphos/80-robin-hood-the-legend-of-sherwood-morphos.html>  
Sklep Vesalia: <http://www.vesalia.de/e_robinhood.htm>

| Element | Score | Details |
| --- | ---: | --- |
| Robin Hood - The Legend of Sherwood - Runesoft 2006 | 100 | 1 CD |
| Sound | 80 |  |
| Gameplay | 75 |  |

[komentarzy: 7](https://www.ppa.pl/forum/komentarze/23649/robin-hood-the-legend-of-sherwood#komentarze), [ostatni: 28.05.2011 18:15](https://www.ppa.pl/forum/komentarze/23649/robin-hood-the-legend-of-sherwood#m303807)
