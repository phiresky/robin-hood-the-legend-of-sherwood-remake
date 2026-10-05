"""Sample source visibility within wheel caps rather than at their hidden hub."""
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
from restart2_build_south_cart_wreck import main

if __name__ == '__main__':
    main('v2', triangulate=True)
