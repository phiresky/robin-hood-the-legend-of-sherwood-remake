"""Check proposed local motion anchors bidirectionally before a geometry trial."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.ndimage import gaussian_filter
from restart14_tree42_gap_classification import sample,BASE
DEST=BASE/'tree42-gap-classification-v2'
def main():
 source=json.loads((BASE/'source-reconciliation-v1/report.json').read_text())['groups'][1];features=[]
 for phase in(0,7):
  f=source['frames'][phase];x,y,w,h=f['bbox'];a=np.array(Image.open(f['path']).convert('RGBA'),dtype=float)/255;c=np.zeros((288,342,4));c[y-688:y-688+h,x-616:x-616+w]=a;c[:,:,:3]*=c[:,:,3:];features.append(gaussian_filter(c,(2,2,0)))
 report=json.loads((DEST/'report.json').read_text());gy,gx=np.mgrid[-5:6,-5:6];weight=np.exp(-(gx*gx+gy*gy)/18);weight/=weight.sum();rows=[]
 for r in report['rows']:
  if not r['proposal'].startswith('test'):continue
  p=np.array(r['native_pixel'],float)-[616,688];forward=np.array(r['independent_lowpass_patch_best_displacement']);q=p-forward;target=sample(features[0],q[0]+gx,q[1]+gy)
  def cost(dx,dy):return float(np.sum(np.mean((sample(features[1],q[0]+gx-dx,q[1]+gy-dy)-target)**2,axis=2)*weight))
  best=min((cost(dx,dy),dx,dy)for dy in np.arange(-4,4.01,.5)for dx in np.arange(-4,4.01,.5));back=np.array(best[1:]);roundtrip=float(np.linalg.norm(forward+back));rows.append({'native_target_pixel':r['native_pixel'],'source_phase0_center':(q+[616.5,688.5]).tolist(),'target_phase7_center':(p+[616.5,688.5]).tolist(),'forward_displacement':forward.tolist(),'reverse_displacement':back.tolist(),'roundtrip_error_pixels':roundtrip,'eligible_for_bounded_field_trial':roundtrip<=.75,'original_field_displacement':r['predicted_displacement'],'baseline_classification':r['classification'],'support_window_pixels':11,'feature_blur_sigma':2})
 out={'status':'PROPOSED_CORRESPONDENCE_CONSTRAINTS_NOT_APPLIED','classification_sha256':hashlib.sha256((DEST/'report.json').read_bytes()).hexdigest(),'anchors':rows,'next_trial':['Only bidirectionally consistent anchors may constrain a smooth local displacement field; do not snap isolated source pixels.','Fit all14 source phases, preserve exact phase0/loop, require nonfolding Jacobians and retain observed 4tick holds.','Compare against v4 without moving wood, textures, neighboring assets or adding alpha coverage.','Keep unsupported target862,775 separate: its solid receiver is absent already in the stationary basis; motion fitting cannot establish new geometry.'],'limits':['Two-frame low-pass constraints are candidate leaf-cluster correspondence, not exact observed material-point identities.','Do not use the source checker parity as a motion target.']};(DEST/'correction-plan.json').write_text(json.dumps(out,indent=2)+'\n');print([(r['native_target_pixel'],r['roundtrip_error_pixels'],r['eligible_for_bounded_field_trial'])for r in rows])
if __name__=='__main__':main()
