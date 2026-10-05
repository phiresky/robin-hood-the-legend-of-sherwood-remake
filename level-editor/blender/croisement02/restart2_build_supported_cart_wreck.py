"""Build the source-constrained cart with shared canopy/wheel support plane."""
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
from restart2_build_south_cart_wreck import main

if __name__ == '__main__':
    main('v3', triangulate=True, fit_variant='v3', supported=True)
