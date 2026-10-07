"""Run the private physical sign browser proof in the shared render pool."""
import subprocess
import sys
from pathlib import Path

HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE.parents[1]/'refinement'))
from render_slots import acquire,release

if __name__=='__main__':
    acquire()
    try:
        subprocess.run(['node',str(HERE/'restart10_verify_sign_occlusion.mjs')],check=True)
    finally:
        release()
