"""Separate raw stipple mismatches from distance to rendered physical support."""
from pathlib import Path
import json,hashlib,sys
import numpy as np
from scipy.ndimage import distance_transform_edt
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement/restart14-canopy-animation'
def main():
 version=sys.argv[1];work=BASE/f'tree42-motion-{version}'/'native-phases-v1';report=json.loads((work/'report.json').read_text());source=json.loads((BASE/'source-reconciliation-v1/report.json').read_text())['groups'][1];rows=[]
 for i,f in enumerate(source['frames']):
  image=work/f'phase-{i:02}.png';a=np.array(Image.open(image))[:,:,3]>=128;expected=np.zeros(a.shape,bool);x,y,w,h=f['bbox'];expected[y-688:y-688+h,x-616:x-616+w]=np.array(Image.open(f['path']))[:,:,3]>0;distance=distance_transform_edt(~a);missing=expected&~a;rows.append({'phase':i,'raw_missing':int(missing.sum()),'missing_with_support_within_one_pixel':int((missing&(distance<=1)).sum()),'missing_with_support_within_sqrt2':int((missing&(distance<=2**.5)).sum()),'missing_farther_than_two_pixels':int((missing&(distance>2)).sum()),'max_distance_to_any_actual_alpha':float(distance[expected].max()),'render_sha256':hashlib.sha256(image.read_bytes()).hexdigest()})
 out={'status':'DISTANCE_DIAGNOSTIC_NOT_OWNERSHIP_OR_EXACT_PARITY','prototype_sha256':report['prototype_sha256'],'rows':rows,'limits':['Nearby alpha is measured, not added to the model or source mask.','Actual rendered crown includes inferred rear/interior foliage; near coverage does not establish observed-front correspondence.','Raw stipple mismatches remain preserved; no source pixels, textures, wood, neighbors or geometry edited.']};(work/'occupied-support.json').write_text(json.dumps(out,indent=2)+'\n');print(rows[7])
if __name__=='__main__':main()
