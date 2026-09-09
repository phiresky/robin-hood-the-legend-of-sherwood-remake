#!/usr/bin/env bash
# Downloads the original page of every source in this collection into ./originals/
# (raw HTML plus a plain-text rendering) for local reading. The folder is gitignored.
# Pages that block plain downloads (GameFAQs, GameSpot, MobyGames) are fetched from the
# Wayback Machine; iDNES needs a crawler user agent to get past its consent wall.
set -u
cd "$(dirname "$0")"
mkdir -p originals
UA='Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0 Safari/537.36'
BOT='Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)'
WB='https://web.archive.org/web/2id_/'
fetch() { # name url [user-agent]
  local name=$1 url=$2 ua=${3:-$UA}
  local code
  code=$(curl -sS -L -A "$ua" --max-time 90 -o "originals/$name.html" -w '%{http_code}' "$url")
  printf '%-40s %s\n' "$name" "$code"
  if command -v html2text >/dev/null; then html2text --body-width=0 --ignore-images "originals/$name.html" > "originals/$name.txt" 2>/dev/null; fi
  sleep 2
}
fetch guides__gamefaqs-radu-bebe        "${WB}https://gamefaqs.gamespot.com/pc/562021-robin-hood-the-legend-of-sherwood/faqs/23711"
fetch guides__gamefaqs-swcarter         "${WB}https://gamefaqs.gamespot.com/pc/562021-robin-hood-the-legend-of-sherwood/faqs/23124"
fetch guides__gry-online-the-escape     "https://www.gry-online.pl/poradniki/robin-hood-legenda-sherwood/ucieczka/z2f57"
fetch guides__gry-online-the-letter     "https://www.gry-online.pl/poradniki/robin-hood-legenda-sherwood/list/z3f58"
fetch guides__gry-online-walkthrough    "https://www.gry-online.pl/poradniki/robin-hood-legenda-sherwood/"
fetch guides__steam-combat              "https://steamcommunity.com/sharedfiles/filedetails/?id=2930272620"
fetch guides__steam-scoring             "https://steamcommunity.com/sharedfiles/filedetails/?id=2829972690"
fetch guides__steam-secret-ending       "https://steamcommunity.com/sharedfiles/filedetails/?id=792309796"
fetch history__gamespot-preview         "${WB}http://www.gamespot.com/articles/robin-hood-the-legend-of-sherwood-preview/1100-2895665/"
fetch history__gogdb                    "https://www.gogdb.org/product/1207659008"
fetch history__gog-api                  "https://api.gog.com/products/1207659008?expand=description,changelog,screenshots,videos"
fetch history__runesoft                 "https://www.rune-soft.com/Games/Released/Game-239/game=Robin_Hood_The_Legend_of_Sherwood-13"
fetch history__wikipedia                "https://en.wikipedia.org/wiki/Robin_Hood:_The_Legend_of_Sherwood"
fetch reference__gamersglobal-p1        "https://www.gamersglobal.de/user-artikel/robin-hood-die-legende-von-sherwood?page=0,0"
fetch reference__gamersglobal-p2        "https://www.gamersglobal.de/user-artikel/robin-hood-die-legende-von-sherwood?page=0,1"
fetch reference__idnes-walkthrough      "https://www.idnes.cz/hry/robin-hood-1-cast-pruvodce-hrou.A021124_robinhoodlosnavod1_bw" "$BOT"
fetch reference__ign-review             "https://www.ign.com/articles/2002/11/21/robin-hood-the-legend-of-sherwood"
fetch reference__magazine-indexes       "https://www.pcgamesdatabase.de/gameinfo.php?id=37738&sort=2"
fetch reference__steam-guide-index      "https://steamcommunity.com/app/46560/guides/"
fetch reference__video-walkthrough      "https://www.youtube.com/watch?v=zK7oTCW4SJM"
fetch related-games__mogelpower-longbow "https://www.mogelpower.de/cheats/loesung.php?id=17262"
fetch reviews__game-over                "https://www.game-over.com/reviews/pc/Robin_Hood%3A_The_Legend_of_Sherwood.html"
fetch reviews__gamespot                 "${WB}https://www.gamespot.com/reviews/robin-hood-the-legend-of-sherwood-review/1900-2897317/"
fetch reviews__gamezone-de              "https://www.gamezone.de/Robin-Hood-Die-Legende-von-Sherwood-Spiel-30523/Tests/Robin-Hood-Die-Legende-von-Sherwood-im-Gamezone-Test-988941/"
fetch reviews__jeuxvideo-com            "https://www.jeuxvideo.com/articles/0000/00002646_test.htm"
fetch reviews__metacritic               "https://www.metacritic.com/game/robin-hood-the-legend-of-sherwood/"
fetch reviews__mobygames-reception      "${WB}https://www.mobygames.com/game/7907/robin-hood-the-legend-of-sherwood/reviews/"
fetch reviews__worthplaying             "https://www.worthplaying.com/article/2002/12/16/reviews/7395-pc-review-robin-hood/"
fetch technical__pcgamingwiki           "https://www.pcgamingwiki.com/wiki/Robin_Hood%3A_The_Legend_of_Sherwood"
fetch technical__steam-language-fonts   "https://steamcommunity.com/sharedfiles/filedetails/?id=1349014146"
fetch technical__steam-ready2play       "https://steamcommunity.com/sharedfiles/filedetails/?id=3290654040"
fetch technical__ubuntuusers-native-linux "https://wiki.ubuntuusers.de/Archiv/Spiele/Robin_Hood_-_Die_Legende_von_Sherwood/"
