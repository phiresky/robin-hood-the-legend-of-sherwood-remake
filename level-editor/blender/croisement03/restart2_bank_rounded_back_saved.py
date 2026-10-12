"""Save and review source-guarded rounded bank backing in a private bounded model."""
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import restart2_bank_lofts_saved as recipe
if __name__=='__main__':
 recipe.CPU=recipe.B/'bank-rounded-backing-cpu-v1';recipe.E=recipe.B/'bank-rounded-east-cpu-v1/geometry.json';recipe.OUT=recipe.B/'bank-rounded-backing-saved-v1';recipe.main()
