"""Compare physical joint visibility against native draw-order visibility."""
import hashlib,json
from pathlib import Path
import numpy as np
from PIL import Image
from catalog import OUT

def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def main():
    joint=OUT/'log-state-foreground-joint-v2';m=json.loads((joint/'manifest.json').read_text());root=OUT/'state-target-evidence/log-trap';reference=root/'native-order-reference';box=m['camera']['bbox'];w=box[2]-box[0];h=box[3]-box[1];scale=m['camera']['ortho_scale'];yy,xx=np.mgrid[:512,:512];ix=np.floor(w/2+(xx+.5-256)*scale/512).astype(int);iy=np.floor(h/2+(yy+.5-256)*scale/512).astype(int);valid=(ix>=0)&(ix<w)&(iy>=0)&(iy<h)
    def project(alpha):
        result=np.zeros((512,512),bool);result[valid]=alpha[iy[valid],ix[valid]];return result
    expected=project(np.array(Image.open(reference/'visible-log-phase0.png'))>0);raw=project(np.array(Image.open(root/'tick-089.png'))[:,:,3]>0)
    def read(name):
        rgb=np.array(Image.open(joint/name));return(rgb[:,:,0]>240)&(rgb[:,:,1]<15)&(rgb[:,:,2]>240)
    body=read('logs-only-visibility.png');actual=read('joint-visibility.png');correct=expected&actual;missing_geometry=expected&~body&~actual;overhidden=expected&body&~actual;underhidden=actual&raw&~expected;unsupported=actual&~raw;rgb=np.zeros((512,512,3),np.uint8);rgb[correct]=(50,200,80);rgb[missing_geometry]=(180,20,230);rgb[overhidden]=(240,40,40);rgb[underhidden]=(240,160,20);rgb[unsupported]=(30,100,240);Image.fromarray(rgb).save(joint/'native-order-comparison.png')
    assert int(correct.sum()+missing_geometry.sum()+overhidden.sum())==int(expected.sum())
    report=dict(status='HOLD: measured physical/source visibility discrepancy',reference_manifest_sha256=sha(reference/'manifest.json'),joint_manifest_sha256=sha(joint/'manifest.json'),measurements=dict(expected_visible_native_pixels=int(expected.sum()),stochastic_coverage_disagreement=int((actual&~body).sum()),correctly_visible=int(correct.sum()),missing_geometry=int(missing_geometry.sum()),native_visible_wood_hidden_by_model_trees=int(overhidden.sum()),native_covered_wood_exposed_by_model=int(underhidden.sum()),unsupported_continuation_visible=int(unsupported.sum())),limitations=['Comparison holds foliage atphase0, matching workers; event-start foliage phase remains independent.','Over-occlusion can come from inferred crown depth, current log support/depth, overlapping owner geometry or inaccurate static appearance support; counts alone do not assign cause.','No approved tree geometry/materials changed; candidate logs remain unapproved.']);(joint/'native-order-comparison.json').write_text(json.dumps(report,indent=2)+'\n');print(report['measurements'])
if __name__=='__main__':main()
