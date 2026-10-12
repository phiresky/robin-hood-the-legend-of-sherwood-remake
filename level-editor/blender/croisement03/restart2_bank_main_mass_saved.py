"""Save the complete convex upper-bank experiment after independent source guards."""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import restart2_bank_lofts_saved as recipe

if __name__ == '__main__':
    recipe.CPU = recipe.B / 'bank-main-masses-cpu-v1'
    recipe.E = recipe.B / 'bank-convex-east-cpu-v1/geometry.json'
    recipe.OUT = recipe.B / 'bank-main-masses-saved-v1'
    recipe.main()
