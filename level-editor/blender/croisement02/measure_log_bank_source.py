"""Measure saved actual cylinder projection against immutable native wood pixels."""
import hashlib
import json
import sys
from pathlib import Path
import numpy as np
from PIL import Image
from catalog import OUT


def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    base=OUT/(sys.argv[1]if len(sys.argv)>1 else 'log-trap-state-candidate-v12')
    review=base/'bank-review';manifest=json.loads((review/'manifest.json').read_text())
    assert sha(base/'worker.blend')==manifest['log_model_sha256']
    source=OUT/'state-target-evidence/log-trap';evidence=json.loads((source/'manifest.json').read_text());left,top,right,bottom=evidence['bbox'];w=right-left;h=bottom-top
    yy,xx=np.mgrid[:512,:512];scale=max(w,h)*1.2;ix=np.floor(w/2+(xx+.5-256)*scale/512).astype(int);iy=np.floor(h/2+(yy+.5-256)*scale/512).astype(int);valid=(ix>=0)&(ix<w)&(iy>=0)&(iy<h)
    alpha=np.array(Image.open(source/'tick-089.png'))[:,:,3]>127;expected=np.zeros((512,512),bool);expected[valid]=alpha[iy[valid],ix[valid]]
    actual=np.array(Image.open(review/'logs-source-actual.png'))[:,:,3]>127
    rgb=np.zeros((512,512,3),np.uint8);rgb[expected&actual]=(65,180,85);rgb[expected&~actual]=(255,35,160);rgb[actual&~expected]=(40,85,140);Image.fromarray(rgb).save(review/'raw-source-coverage.png')
    result=dict(status='raw source geometry diagnostic, not visibility approval',model_sha256=manifest['log_model_sha256'],source_rgba_sha256=sha(source/'tick-089.png'),actual_render_sha256=sha(review/'logs-source-actual.png'),source_manifest_sha256=sha(source/'manifest.json'),native_pixels=int(expected.sum()),covered_native_pixels=int((expected&actual).sum()),native_coverage=float((expected&actual).sum()/expected.sum()),iou=float((expected&actual).sum()/(expected|actual).sum()),limitations=['Native draw-order static-plus-animation composite and physical foreground owners remain separate visibility requirements.','Inferred body pixels outside source alpha may be hidden continuation; they are not automatically valid or invalid.'])
    (review/'raw-source-coverage.json').write_text(json.dumps(result,indent=2)+'\n');print(result)


if __name__=='__main__':main()
