"""Save and review the corrected complementary native/static leaf layer separation."""
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import restart2_tree01_native_transition_saved as recipe
if __name__=='__main__':
 recipe.I=recipe.full.B/'tree01-native-transition-cpu-v2';recipe.full.review.I=recipe.I;recipe.full.review.O=recipe.full.B/'tree01-native-transition-saved-v2';recipe.full.review.EXTRA_BUILD=lambda scene:recipe.scoped(recipe.full.build,scene);recipe.full.review.EXTRA_VERIFY=lambda scene,out:recipe.scoped(recipe.full.verify,scene,out);recipe.full.review.main()
