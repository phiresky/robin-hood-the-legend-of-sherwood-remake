"""Run the private trio actual-Editor proof in the shared render pool."""
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from render_slots import acquire, release

if __name__ == '__main__':
    acquire()
    try:
        subprocess.run(['node', str(Path(__file__).with_suffix('.mjs'))], cwd=ROOT, check=True)
    finally:
        release()
