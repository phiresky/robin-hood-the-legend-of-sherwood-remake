"""Worktree-local outputs; only source game data may come from the main repo."""
import os
from pathlib import Path

ROOT=Path(__file__).resolve().parents[3]
GAME_ROOT=ROOT.parents[1] if ROOT.parent.name=='.worktrees' else ROOT
DATA=Path(os.environ.get('SHERWOOD_DATA_DIR',str(GAME_ROOT/'datadirs/fullgame_gog_hackable/Data')))
OUT=ROOT/'level-editor/work/sherwood-refinement/pass2'
