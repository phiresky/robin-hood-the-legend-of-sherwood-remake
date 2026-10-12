"""Saved review of full crown with sparse uncut off-map front continuation."""
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import restart2_tree01_full_crown_saved as full
I=full.B/'tree01-native-transition-cpu-v1'
def scoped(fn,*args):
 old=(full.CELLS,full.OBJECT_NAME,full.GUARD_NAME)
 try:
  for kind,name,guard in [('native','Tree01 observed native frame0 provisional cells','native-source-guard.json'),('static','Tree01 observed native proposed static samples','static-source-guard.json')]:full.CELLS=I/f'cells-{kind}.json';full.OBJECT_NAME=name;full.GUARD_NAME=guard;fn(*args)
 finally:full.CELLS,full.OBJECT_NAME,full.GUARD_NAME=old
if __name__=='__main__':
 full.review.I=I;full.review.O=full.B/'tree01-native-transition-saved-v1';full.review.EXTRA_BUILD=lambda scene:scoped(full.build,scene);full.review.EXTRA_VERIFY=lambda scene,out:scoped(full.verify,scene,out);full.review.main()
