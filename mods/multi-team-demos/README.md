# Multi-team demos

Ten custom missions sharing battlefield assets. Select a mission from the custom-mission menu, or launch with `--mission <mission>`.

| Mission | Description |
| --- | --- |
| `MultiTeamAllPcsCircle` | Every playable-character profile begins in an autonomous ten-way free-for-all around a compact circle. |
| `MultiTeamAllVariantsWheel` | Every soldier profile arranged in a large circle, with each NPC assigned a unique allegiance. |
| `MultiTeamArrowCrossfire` | Four mutually hostile archer companies converge on the same exposed patch of ground, each protected by a small guard screen. |
| `MultiTeamChampionsRetinues` | Four autonomous outlaws command compact retinues in a four-way battle, combining active hero combat with faction-matched soldiers. |
| `MultiTeamFourArmies` | Four mutually hostile allegiances field twelve soldiers each on a spacious obstacle-free battlefield, with three soldiers of each of four profiles per faction. |
| `MultiTeamFourGrades` | Four mutually hostile armies field the same four soldier types, but each army uses its own increasingly powerful soldier grade from 01 through 04. |
| `MultiTeamRobinVsLittleJohn` | Robin and Little John are non-playable autonomous PCs on different allegiances and begin a swordfight automatically. |
| `MultiTeamTenWay` | Ten mutually hostile allegiances, one soldier in each team. |
| `MultiTeamThreeWay` | Diplomacy test arena: allegiances 2 and 3 are allied, 3 and 4 are neutral, and all unspecified pairs remain hostile. |
| `MultiTeamTwentyRobins` | Ten forest Robins and ten town-disguise Robins meet twenty advanced Black Knights in two compact, inward-facing formations on open ground. |

The shared terrain is stored as JPEG XL in `Data/Levels/Day/OpenBattlefield.map`
(the terrain loader detects the format from its signature). It was converted
from the original 2508 × 2508 PNG with `cjxl -q 90 -e 7`, reducing the map
from 12.1 MB to 1.7 MB. The minimap remains PNG. To edit the terrain, decode
the map with `djxl OpenBattlefield.map OpenBattlefield.map.png`; a sibling
`.map.png` takes precedence during loading. Remove that editing copy before
packaging so it does not ship alongside the compressed map.
