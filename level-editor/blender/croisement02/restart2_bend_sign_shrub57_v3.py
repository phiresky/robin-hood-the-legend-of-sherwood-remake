"""Reproduce the private piecewise depth candidate; no selected model changes."""
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
from restart2_bend_sign_shrub57 import main
from render_slots import acquire, release

if __name__ == '__main__':
    acquire()
    try:
        main(mode='piecewise', version=3)
    finally:
        release()
