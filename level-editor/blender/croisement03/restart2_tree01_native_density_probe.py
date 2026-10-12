"""Isolate inferred shader brightness on the prior off-map-front crown geometry."""
import json,sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import numpy as np
from PIL import Image
import restart2_tree01_full_crown_saved as full
B=full.B

def scoped(fn,*args):
 old=(full.CELLS,full.OBJECT_NAME,full.GUARD_NAME)
 try:
  for cells,name,guard in [('tree01-native-cells-cpu-v3','Tree01 observed native frame0 provisional cells','native-source-guard.json'),('tree01-static-cells-cpu-v1','Tree01 observed native proposed static samples','static-source-guard.json')]:
   full.CELLS=B/cells/'geometry.json';full.OBJECT_NAME=name;full.GUARD_NAME=guard;fn(*args)
 finally:full.CELLS,full.OBJECT_NAME,full.GUARD_NAME=old

def main():
 static=np.array(Image.open(B/'tree01-static-leaf-proposal-v1/proposed-source-rgba.png'))[:,:,3]>0;union=np.array(Image.open(B/'tree01-native-fragment-v1/alpha-union.png'))>0;assert not (static&union).any();review=full.review;review.I=B/'tree01-canopy-volume-cpu-v4';review.O=B/'tree01-static-crown-emission-probe-v1';review.EXTRA_BUILD=lambda scene:scoped(full.build,scene);review.EXTRA_VERIFY=lambda scene,out:scoped(full.verify,scene,out);review.main()
if __name__=='__main__':main()
