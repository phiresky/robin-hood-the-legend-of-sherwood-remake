#!/usr/bin/env bash
set -euo pipefail

cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
output_dir="${1:-mission-maps}"
frame="${2:-0}"
data_dir="${3:-${ROBINHOOD_DATA_DIR:-datadirs/fullgame_gog}}"
mkdir -p -- "$output_dir"

cargo build --example render_mission_map --release
renderer="${CARGO_TARGET_DIR:-target}/release/examples/render_mission_map"

while IFS='|' read -r mission name; do
    if ! "$renderer" "$mission" --frame "$frame" --reveal-all --headless \
        --data-dir "$data_dir" --output "$output_dir/$name.png"; then
        printf 'warning: skipping %s after renderer failure\n' "$name" >&2
    fi
done <<'MISSIONS'
Emb01_FoA_EC|Ambush 1
Sherwood|Sherwood Forest
Emb02_FoC_MK|Ambush 2
Emb03_FoC_MP|Ambush 3
Emb04_FoA_MP|Ambush 4
Emb05_FoB_MP|Ambush 5
Emb06_FoC_EC|Ambush 6
Emb07_FoB_JMS|Ambush 7
Emb08_FoA_JMS|Ambush 8
Emb09_FoB_JMS|Ambush 9
EmbTut_FoC_EC|Ambush Tutorial
H01_Lin_VL|Robin's Godfather
H02_Not_EC|Contact Marian
H03_Der_MK|The Outlaw and the Prince
H04_Lei_VL|Contact Ranulph
H05_Lin_EC|Free Godwin
H07_Not_MK|The Silver Arrow
H09_Not_VL|Free Robin
H10_Yor_VL|Lackland's Plan
H12_Not_MP|The Sheriff of Nottingham
S01_Not_VL|Save Stuteley
S02_Lei_MP|Save Scarlett
S03_FoB_MP|Save Little John
S04_Der_EC|Save Tuck
S05_Yrk_EC|Save Marian
Str01_Lin_EC|Attack Lincoln
Str02_Der_MP|Attack Derby
Str03_Yor_MK|Attack York
Tac01_FoA_MP|Tactical mission 1
Tac02_FoB_EC|Tactical mission 2
Tac03_FoC_MP|Tactical mission 3
Tac04_FoA_EC|Tactical mission 4
Tac05_FoC_MP|Tactical mission 5
Tac06_FoB_EC|Tactical mission 6
Tac17_FoC_EC|Tactical mission 17
Tac18_FoA_EC|Tactical mission 18
Tac19_FoB_EC|Tactical mission 19
Tac21_FoB_EC|Tactical mission 21
SherwoodOutro|SherwoodOutro
MISSIONS
