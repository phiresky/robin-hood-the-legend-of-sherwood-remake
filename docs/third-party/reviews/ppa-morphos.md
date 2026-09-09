# PPA — Polish review of the MorphOS port

- Original source: [PPA — Polish review of the MorphOS port](https://www.ppa.pl/gry/robin-hood-the-legend-of-sherwood.html)
- Author / publication: PPA.pl; author not verified
- Language / date: Polish; exact article date not verified
- Access: Substantial article text inspected
- Checked: 2026-09-09
- Format: original summary and research notes; not a transcript.

An unusually useful account of physical packaging and native-port behavior. The reviewer received a box with Linux branding and a MorphOS label; the reversible cover and short manual were in English/German, with extended PDF manuals on disc.

Testing on a Pegasos II G4 exposed an installer-script problem. The article explains copying the extraction tool and script to writable storage, adjusting permissions and executable paths, then installing the two data archives. It identifies SDL and the bundled powersdl.library 11.11.

Gameplay discussion covers profiles, configurable controls, display settings, difficulty, and Quick Actions. The reviewer finds mouse-gesture combat less intuitive than its marketing suggests.

The article states that RuneSoft released the MorphOS version in October 2006.

**Research use:** port-specific packaging, installation, library dependencies, and interface observations. These findings should not be generalized to Windows or later rereleases; no fix was executed here.

## Converted text from the original HTML


### reviews__ppa-morphos.html

_Source: `originals/reviews__ppa-morphos.html`._

[93mWarning:[0m Use the [92m--decode-errors=ignore[0m flag.


### reviews__ppa-morphos.txt

_Source: `originals/reviews__ppa-morphos.txt`._

[ Zaloguj się](/profile/login/index)

[Utwórz konto](/profile/register/index/)

[Wyszukiwarka](/szukaj/)

≡

[](https://www.ppa.pl)

[Aktualności](/) [Forum](/forum/) [Giełda](/gielda/) [Graffiti](/graffiti/) [Programy](/programy/) [Publicystyka](/publicystyka/) [Sprzęt](/sprzet/) [Strefa Gier](/gry/)

  * [PPA.pl](/)
  * » **[Strefa Gier](/gry/)**
  * » [Recenzje, opisy](/gry/recenzje-opisy/)

  *   * ### Robin Hood - The Legend of Sherwood

  * 

19.05.2011 18:37, autor artykułu: Grzegorz Murdzek 

odsłon: 16832, oryginalny rozmiar obrazków,  powiększ obrazki,  wersja do wydruku, 

[ ](/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_logo.png)

[](/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_13.png) Anglia, rok 1190. Pod nieobecność walczącego w krucjacie w Ziemi Świętej króla Ryszarda Lwie Serce całą władzę brutalnie sprawuje jego brat - książe John Lackland, z którym knuje niecne plany Szeryf z Nothingham. Tylko jeden człowiek wraz z grupą wiernych kompanów jest gotów stanąć do walki z ciemiężycielami biednych Anglików - zwą go Robin Hood. 

Taki oto prosty i nieskomplikowany wstęp zapowiada zabijanie czasu, który spędzić można wcielając się w "człowieka w kapturze" w zręcznościowo-przygodowej grze "Robin Hood: The Legend of Sherwood" - jednej z nielicznych komercyjnych konwersji gier wydanych dla systemu MorphOS. O przygodach legendarnego banity z lasu Sherwood oraz jego drużyny powstało przynajmniej kilka gier, wiele filmów i telewizyjnych seriali (w tym Robin of Sherwood, serial produkcji BBC o niesamowitym klimacie, który nadawała produkcji muzyka zespołu Clannad). Wśród gier wydanych na Amigę są to takie tytuły jak ["The Adventures of Robin Hood"](http://www.ppa.pl/artykul-the.adventures.of.robin.hood-7_15_1153.html) czy "Conquests of the Longbow". 

**Co w pudełku?**

Po odbiorze przesyłki z grą byłem zdziwiony tym, co zobaczyłem. Okładka w języku niemieckim, z napisem na samej górze "Linux CD-ROM". Już miałem dzwonić po reklamację, ale na szczęście uspokoił mnie skromny nadruk z wymaganiami, gdzie na białym tle z niebieskim motylem widniał napis "MorphOS CD-ROM". Sama okładka okazała się być również dwustronna z zadrukowaną częścią w języku angielskim. Podobnie jest ze wstępną instrukcją obsługi (wersje rozszerzone są dostępne na płycie z grą w postaci plików PDF) napisaną w języku angielskim i niemieckim (wersji polskiej brak). Grę dla MorphOS-a wydała firma Runesoft (dawniej Epic Interactive), a wcześniej "Robin Hood" był dostępny na platformy takie jak Windows, Mac czy system Linux (w instrukcji znajdziemy kilka zdań o wymaganiach i instalacji tylko dla tych dwóch ostatnich). 

**Wymagania**

Gra powinna działać normalnie na Pegasosie z procesorem PowerPC G3, choć zalecane jest G4. Jeśli chodzi o pamięć, minimum to 128 MB RAM, a karta graficzna, jaką poleca wydawca to dowolny obsługiwany Radeon, Voodoo bądź Permedia. Gra potrzebuje do 1 GB miejsca na twardym dysku oraz napędu CD-ROM (bo na takim nośniku jest rozprowadzana gra). 

**W drodze do lasu Sherwood**

Gotowy do walki w imię dobra i sprawiedliwości rozpocząłem od rzeczy banalnej - instalacji na dysku twardym. Nie wspomniałbym tutaj o tym, gdyby nie problemy, jakie napotkałem a które występują w przypadku wersji gry dla MorphOS-a. Po kliknięciu w ikonkę instalacji wersji angielskiej lub niemieckiej skrypt instalacyjny najpierw informuje o wspomnianych wcześniej wymaganiach wolnej przestrzeni dyskowej, a także zalecanym systemie plików na partycji, na której zostanie zainstalowana gra. 

[](/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_1.png) [](/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_2.png)

Ponieważ miałem wszystko, co było potrzebne (Pegasos II G4, 1024 MB RAM, Radeon 9250 oraz spory zapas miejsca na dysku, na którym wszystkie partycje dla MorphOS-a mam w SFS), poszedłem krok dalej. Po kliknięciu w "Continue" wybrałem miejsce na dysku, gdzie zainstalowana miała zostać gra i... niestety zobaczyłem to, co na poniższym obrazku: 

[](/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_3.png) [](/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_4.png)

[](/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_5.png) Tym samym okazało się, że pierwszym zadaniem dla gracza będzie walka nie z żołnierzami Szeryfa z Nothingham, a z zepsutym skryptem instalacyjnym, jaki otrzymał wraz z grą. Aby poradzić sobie z tym problemem, należy:  
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

**Akcja!**

[](/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_6.png) W klimat gry wprowadza introdukcja będąca krótką, wyrenderowaną animacją 3D, która przedstawia zwinność tytułowego Robin Hooda w walce z ludźmi Szeryfa. Filmik wprowadzający to standard w tego typu produkcjach. Z intra przenosimy się do menu głównego gry, w którym przygrywa nam nieco smutna, powolna, ale klimatyczna muzyka. W głównej części ekranu startowego podana jest statystyka dotychczasowej rozgrywki. W miarę upływu czasu i postępu w grze informacje są aktualizowane. W grze najważniejsze dane statystyczne to pieniądze, punkty gry oraz procent zabitych wrogów, czas spędzony na grze i postęp zaawansowania przejścia gry. W głównym menu po lewej stronie ekranu znajdziemy szereg opcji, a wśród nich możliwość wyboru gracza (odpowiednik profilu lub konta), dzięki czemu możemy grać na swój rachunek, a np. kto inny z domowników może grać na innym profilu, co przy jednoużytkownikowym systemie, jakim jest MorphOS, ma dodatkową zaletę. W opcjach znajdziemy również m. in. możliwość odczytu stanu gry (na szczęście stan gry można prawie w dowolnym momencie zapisać i nie ma tutaj żadnych rozwiązań typu kod do misji itp.), ustawienia dźwięku i grafiki, możliwość wyświetlenia filmików, które wystąpiły podczas gry (jak np. wspomniane intro), możliwość obejrzenia listy płac (na wypadek, gdyby ktoś nie dotrwał do napisów końcowych), a także banalne opcje rozpoczęcia gry i wyjścia z gry do... "Windows". 

[](/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_7.png) Wspomniane ustawienia dźwięku i grafiki dają użytkownikowi podstawowe możliwości ustawienia priorytetów i dostrojenia wydajności gry. Dla dźwięku będzie to odpowiednio możliwość regulacji głośności efektów, muzyki, dialogów oraz komentarzy, możliwość włączenia muzyki w trybie stereo lub 3D. W ustawieniach graficznych można wybrać spośród trzech rozdzielczości (648x480, 800x600 oraz 1024x768). Im wyższa rozdzielczość tym większe pole widzenia, a co za tym idzie wygodniejsza gra, ale może to być wrażenie subiektywne. Poza rozdzielczością można również włączyć lub wyłączyć niektóre dodatkowe miłe dla oka, ale obciążające nieco sprzęt efekty graficzne takie jak wyświetlanie pola widzenia wrogów, przezroczystość cieni, animacje niektórych efektów oraz animacje tła. Jeśli poczujesz, że gra się ślimaczy zawsze możesz wrócić i powyłączać efekty i ewentualnie zmniejszyć rozdzielczość w celu przyspieszenia gry. Poza tym istnieje jeszcze możliwość zmiany domyślnych skrótów klawiaturowych występujących w grze oraz możliwość powrotu do ich ustawień domyślnych. 

[](/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_8.png) "Robin Hood: The Legend Of Sherwood" daje graczowi do wyboru trzy poziomy trudności (Easy, Medium oraz Hard), które różnią się od siebie głównie stosunkiem siły i zręczności naszych postaci do żołnierzy Szeryfa z Nothingham. Nie polecam wyboru najniższego poziomu trudności - albowiem wtedy gra jest banalnie prosta do przejścia, przeciwnicy są bardzo słabi i praktycznie z każdych opałów można wyjść bez większych strat, a dodatkowo energia odrasta i to bardzo szybko. Gra podzielona jest na misje, których część jest obowiązkowa, a pozostałe (głównie napadanie na żołnierzy szeryfa i konwoje ze złotem) służą głównie do podreperowania skarbca drużyny. W pierwszych dwóch misjach poznajemy historię powrotu Robina z Locksley i kompletujemy drużynę. Gra ma przede wszystkim zręcznościowy charakter rozgrywki, który przeplatany jest dialogami i komentarzami/wskazówkami o kolejnych odkrytych szczegółach potrzebnych do ukończenia gry. Nie ma tutaj żadnych skomplikowanych zagadek czy konieczności klikania w każdy piksel ekranu, aby odnaleźć bardzo drobiazgowo ukryte przejście, co oczywiście nie oznacza, że nie trzeba się dużo nachodzić, nabiegać, wspinać i skradać, by przejść konkretną misję. Rozgrywkę, zwłaszcza w misjach na terenie zamków opanowanych przez żołnierzy wiernych Szeryfowi, możesz prowadzić ostrożnie - głównie skradając się za plecy wroga, ogłuszając go pięścią (lub np. dusząc, zależnie od możliwości konkretnej postaci), a tylko w razie zdemaskowania podjąć walkę bronią białą lub użyć łuku albo pójść na żywioł i spróbować zabić każdego, kto stanie nam na drodze (zadanie trudniejsze o tyle, że żołnierze szybko zwołują wsparcie i nawet jeśli dasz radę w zabawie z walce wręcz, to na pewno dopadną Cię łucznicy lub kusznicy). 

[](/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_9.png) W grze sterujemy myszką przy opcjonalnym wsparciu skrótów klawiaturowych. Sterowanie jest wygodne i nie trzeba się do niego przyzwyczajać, ale warto zapoznać się szczegółowo z instrukcją, aby nie pominąć udogodnień w obsłudze gry. Autorzy gry chwalą się innowacyjnym systemem walki zastosowanym w grze. Przyznam szczerze, że jest on dla mnie innowacyjny tyle samo, co nieintuicyjny, przypominający nieco opanowanie tzw. gestów myszy. Sama walka mieczem lub jakąkolwiek bronią białą wygląda czasami komicznie i brak w niej zaciętości akcji i emocji, ruchy postaci w walce wydają się anemiczne i zbyt proste, ale ostatecznie można się do tego przyzwyczaić. Interfejs obsługi uzupełnia możliwość skorzystania z mapy terenu (z zaznaczonymi odpowiednim kolorem postaciami) oraz możliwość zbliżania lub oddalania pola gry. Dodatkowo warto wspomnieć, że w grze występują różne ikonki emocji, które znajdują się nad postaciami występującymi w grze. Ich znaczenie łatwo zinterpretować, ale gdyby ktoś miał wą tpliwości może zajrzeć do instrukcji. 

Rozkazy wydawać możemy na początku tylko Robinowi, ale wraz z kolejnymi misjami, w miarę skompletowania drużyny, pojawi się możliwość sterowania większą liczbą postaci jednocześnie. Dodatkową ciekawostką w grze są tzw. szybkie akcje (do 5 akcji), które można wyznaczyć do wykonania. Daje to dodatkowe możliwości, takie jak odciągnięcie uwagi wroga, dywersja lub kombinowany atak. Ogólnie w grze będziemy mogli sterować maksymalnie dziewięcioma różnymi postaciami, które będą mogły dołączyć do nas w kolejnych misjach. W międzyczasie poznamy cechy każdej z nich (dodatkowym ułatwieniem jest opis cech i możliwości każdej z postaci w instrukcji obsługi). Każdy, kto dołączy do drużyny, specjalizuje się w walce różną bronią, potrafi korzystać z sobie tylko przeznaczonych artefaktów, a nawet może mieć inne upodobania kulinarne. Przykładowo Robin Hood może posługiwać się mieczem (w misjach na zamkach), kosturem oraz łukiem. Potrafi również skradać się, wspinać i skakać po dachach i różnych przeszkodach czy też ogłuszać przeciwnika mocnym uderzeniem (gdy zajdzie go niepostrzeżenie) lub rozrzucać sakiewki z pieniędzmi w celu odciągnięcia uwagi. O ile wybierzemy wyższy poziom trudności niż najłatwiejszy, wiele z tych specjalizacji przyda się w konkretnych misjach. Nie ma sensu w tym miejscu opisywać ich wszystkich, proponuję samemu odkrywać ich znaczenie w grze. 

  
Źródło: YouTube 

[](/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_10.png) W trakcie rozgrywki naszą uwagę na pewno przykują różne zwoje. Czytając je, uzyskamy wiele wskazówek i pomocy. I tak, zwój niebieski pozwala dowiedzieć się o tym przez jaką postać dany, rozrzucony po mapie przedmiot, może być wykorzystany. Dowiemy się również o jego przeznaczeniu, ale także o celowości istnienia konkretnych lokalizacji w grze. Analogicznie wskazówki w rozgrywce dotyczące szczegółów misji są zawarte w zwojach zawiązanych czerwoną kokardką. Aby rozgrywkę jeszcze bardziej urozmaicić, autorzy wprowadzili pięć poziomów wyszkolenia umiejętności w walce bronią białą oraz posługiwania się łukiem (dla tych postaci, które mają taką umiejętność). Kolejne poziomy zwiększają skuteczność posługiwania się bronią i co za tym idzie, siłę obrażeń. Po przejściu dwóch pierwszych misji nasza drużyna wraca do lasu Sherwood, w którym ma swoje centrum dowodzenia. Jest to miejsce szczególne, do którego będziesz wracać po każdej z kolejnych misji i w którym możesz wykonać szereg różnych akcji i rozkazów, jak np. polecenie zbierania ziół leczniczych, treningu w umiejętności walki bądź strzelania z łuku, przygotowywanie jedzenia itp. Z racji tego, że nie zawsze wszyscy nasi kompani będą uczestniczyć w misji, pozostawieni w kwaterze głównej mogą, zamiast siedzieć bezczynnie, wykonywać te zadania. O tym która misja będzie następna, decydujesz sam, dokonując wyboru z ekranu mapki w prawym górnym rogu. 

**Oprawa graficzna**

[](/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_11.png) Co najważniejsze grafika nie męczy wzroku zbyt dużymi kontrastami. To duży plus biorąc pod uwagę to, że każdy szczegół jest odwzorowany dokładnie i wygląda mile dla oka. Można mieć jedynie zastrzeżenia co do wtapiania się w tło sterowanych bohaterów, przez co zasłonięci drzewami są słabo widoczni. Animacja w grze jest żywa, a życia dodatkowo nadają jej szczegóły, takie jak animacje w tle. Zaletą jest również to, że gra jest w całości w 2D, żadnego trójwymiaru, obracania sześcianów i tym podobnych cudów na kiju. To typowa gra akcji i wzrok może się skupić na grze i jej szczegółach. Biegamy, strzelamy, walczymy - gra toczy się płynnie, zgodnie z naszymi rozkazami (chyba że ktoś posiada zbyt słabą konfigurację), nie ma żadnych defektów graficznych czy innych niedoróbek. Gra jest kompletna, jeśli chodzi o grafikę i choć nie jest to produkcja, w której, jak za dawnych czasów, wszystko zostało namalowane piksel-w-piksel (raczej mamy tu do czynienia z elementami przygotowanymi za pomocą oprogramowania 3D, lecz sprytnie zatopionymi w świecie 2D), to pozostaje wrażenie, jakby każdy detal był dokładnie malowany przez grafika. Postaci w grze, zwłaszcza w widoku dialogów, wyglądają jak dla mnie komicznie. Są jak postacie z bajki, ładni i zadbani na czele z Robin Hoodem, który wygląda, jakby właśnie wyszedł z salonu piękności. 

**Muzyka i dźwięk**

Muzyka w tle jest wykonana poprawnie, ale szybko się nudzi i nadaje klimatu monotonii całej grze. Nie pomaga zmiana motywu podczas walki z wrogiem. Do efektów dźwiękowych nie można się przyczepić, doskonale odwzorowują wszystko, co chciałoby się usłyszeć. Jedynym mankamentem dla mnie były sytuacje, gdy spłoszeni mieszkańcy krzyczeli po pomoc w kółko tak samo raz za razem - w tym momencie ściszałem głośnik. W grze występują również dialogi mówione, którym również nie można niczego zarzucić (poza tym, że animacja twarzy bohatera nie odzwierciedla tego, co słyszymy). 

**Czy warto grać?**

[](/artykuly/pics/sekcje/strefa2/robinhoodthelegendofsherwood/robin_12.png) Grywalność Robin Hooda zależy od podejścia gracza. Jeśli wybierze najniższy poziom trudności, szybko się znudzi i przejdzie grę w pół dnia bez wysiłku. W przypadku wyższych poziomów trudności gra staje się bardziej urozmaicona i wymagająca, przez co może wciągnąć na dłużej. Twórcy starali się zaciekawić gracza i urozmaicić rozgrywkę przez szereg różnych artefaktów, cech szczególnych bohaterów, ale niestety mniej przez różnorodność misji, które pomimo zawierania elementów nowych w dążeniu do przejścia gry w kolejnych odsłonach nudzą, bo zadania wykonuje się podobnie jak dotychczas, ale ciekawość odkrywania spada równią pochyłą w dół. Na pewno ciekawym elementem gry jest przyzwoita inteligencja przeciwnika (choć jest to raczej tzw. inteligencja owada) - wróg potrafi nie tylko uciec ze strachu, widząc naszą przewagę, ale też wezwać posiłki z odległej części danej lokalizacji i spróbować nas odnaleźć. W grze dużo zabijamy, ale nie jest przesadnie brutalna (nikt nikomu nie robi "fatality", ale za to dwie z postaci potrafią dobić leżącego), co może być dla jednych wadą, a dla innych zaletą. Na pewno jest to gra należąca do tych, które są łatwe i przyjemne, ale niestety również do takich, które przechodzi się raz i nie wraca już nigdy więcej. To rozrywka jednorazowa, pytanie tylko czy warta swojej ceny? Na pewno jest to najlepsza komercyjna gra (a właściwie konwersja gry) wydana specjalnie dla MorphOS-a, więc jeśli jesteś fanem tego systemu warto zainteresować się tym tytułem (choćby poprzez wypróbowanie wersji demonstracyjnej). Gra jest niestety bardzo niestabilna, co negatywnie wpływa na grywalność - gra potrafi się zawiesić niemal w dowolnym momencie, zamraża się ekran gry i słychać tylko muzykę, dlatego warto często zapisywać stan gry. 

"Robin Hood: The Legend of Sherwood" to gra stworzona przez Spellbound Studios, a wydana na wiodące platformy przez Wanadoo, w listopadzie 2002 roku. Wersja dla MorphOS-a [została wydana w październiku 2006 roku](http://www.ppa.pl/robin.hood.the.legend.of.sherwood,4764;aktualnosci.html) przez firmę RuneSoft. Na stronie wydawcy dostępna jest wersja demonstracyjna gry. 

Grę można zamówić w polskim sklepie Wupra, a także w niemieckim sklepie Vesalia. 

Strona gry w wersji dla MorphOS: <http://www.rune-soft.com/product.php?product_id=20>  
Wersja demo: <http://www.rune-soft.com/downloads/robin_demo.lha>

Sklep Wupra: <http://wupra.com/morphos/80-robin-hood-the-legend-of-sherwood-morphos.html>  
Sklep Vesalia: <http://www.vesalia.de/e_robinhood.htm>

  
Robin Hood - The Legend of Sherwood - Runesoft 2006   
---  
| 100 |  | 1 CD |   
| 80 |  |   
| 75 |  |   
|   
  
[komentarzy: 7](/forum/komentarze/23649/robin-hood-the-legend-of-sherwood#komentarze), [ostatni: 28.05.2011 18:15](/forum/komentarze/23649/robin-hood-the-legend-of-sherwood#m303807)

[Baldies - wersja demonstracyjna](/gry/baldies-wersja-demonstracyjna.html) [](/gry/baldies-wersja-demonstracyjna.html)

[](/gry/pang.html) [Pang](/gry/pang.html)

  * [Amiga FAQ](/faq/)
  * [Baza użytkowników](/baza-uzytkownikow/)
  * [Giełda](/gielda/)

  * [Aktualności](/)
  * [Programy](/programy/)
  * [Publicystyka](/publicystyka/)
  * [Rodzynki](/rodzynki/)
  * [Sprzęt](/sprzet/)
  * [Strefa Gier](/gry/)

  * [Forum](/forum/)
  * [Graffiti](/graffiti/)
  * [Archiwum ankiet](/ankiety/archiwum/)
  * [Emulki](/emulki/)
  * [Teleport](/teleport/)
  * [Wyszukiwarka](/szukaj/)

  * [O stronie](/o-stronie.html)
  * [Polityka Prywatności](/polityka-prywatnosci.html)
  * [Kontakt](/kontakt/)
  * [Komu Piwo](/komu-piwo.html)
  * [Statystyki](/statystyki/)
  * [Regulamin](/regulamin.html)

  * [RSS](//www.ppa.pl/aktualnosci/rss/)
  * [Discord](//discord.com/invite/rCQ8etjute)
  * [Facebook](//www.facebook.com/polski.portal.amigowy)
  * [Twitter](//twitter.com/portalamigowy)
  * [IRC](/irc.html)
  * [Dobry hosting w PPA](/hosting/oferta/ "twoje konto WWW/FTP/e-mail na serwerze PPA")
  * [Reklama w PPA](/reklama.html)

Copyright © 2000-2026 **Polski Portal Amigowy**. All rights reserved. Amiga and its logos are registered trademarks of their respective owners. 

Na stronie **www.PPA.pl** , podobnie jak na wielu innych stronach internetowych, wykorzystywane są tzw. **cookies** (ciasteczka). Służą ona m.in. do tego, aby zalogować się na swoje konto, czy brać udział w ankietach. Ze względu na nowe regulacje prawne jesteśmy zobowiązani do poinformowania Cię o tym w wyraźniejszy niż dotychczas sposób. Dalsze korzystanie z naszej strony bez zmiany ustawień przeglądarki internetowej będzie oznaczać, że zgadzasz się na ich wykorzystywanie. 

OK, rozumiem
