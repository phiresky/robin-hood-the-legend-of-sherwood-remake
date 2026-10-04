"""Verify preserved target/mobile frames and measured endpoint source coverage."""
import hashlib,json
from pathlib import Path
import numpy as np
from PIL import Image
from catalog import OUT

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    root=OUT/'state-target-evidence';data=json.loads((root/'manifest.json').read_text());mobile=json.loads((root/'visual-completeness.json').read_text());frames=[]
    for profile in data['profiles']:
        for row in profile['rows']:
            for f in row['frames']:frames.append((Path(f['raw']),f['raw_sha256'],Path(f['image']),f['image_sha256']))
    for profile in mobile['mobile_profiles'].values():
        for row in profile['rows']:
            for f in row['frames']:frames.append((Path(f['raw']),f['raw_sha256'],Path(f['decoded']),f['decoded_sha256']))
    for raw,rawsha,decoded,decodedsha in frames:
        assert sha(raw)==rawsha and sha(decoded)==decodedsha
        expected=np.array(Image.open(raw).convert('RGBA'));expected[np.all(expected[:,:,:3]==[0,251,0],axis=2),3]=0
        assert np.array_equal(expected,np.array(Image.open(decoded).convert('RGBA')))
    proof=dict(status='PASS',target_profiles=len(data['profiles']),mobile_profiles=len(mobile['mobile_profiles']),exact_frames=len(frames),target_instances=len(data['instances']),mobile_instances=len(mobile['mobile_instances']),scope='RGBA decoding removes only native green color key. All raw frame bytes, source RGB, offsets, delays and sounds remain bound.')
    (root/'source-verification.json').write_text(json.dumps(proof,indent=2)+'\n');print(proof)
    dest=OUT/'log-trap-state-candidate-v4'
    if not (dest/'worker.blend').exists():return
    source=root/'log-trap';m=json.loads((source/'manifest.json').read_text());left,top,right,bottom=m['bbox'];scale=max(right-left,bottom-top)*1.2;yy,xx=np.mgrid[:512,:512];ix=np.floor((left+right)/2+(xx+.5-256)*scale/512-left).astype(int);iy=np.floor((top+bottom)/2+(yy+.5-256)*scale/512-top).astype(int);coverage=[]
    for state,tick in [('covered',-1),('applied',89)]:
        alpha=np.array(Image.open(source/f'tick-{tick:03d}.png'))[:,:,3]>127;expected=np.zeros((512,512),bool);valid=(ix>=0)&(ix<alpha.shape[1])&(iy>=0)&(iy<alpha.shape[0]);expected[valid]=alpha[iy[valid],ix[valid]];actual=np.array(Image.open(dest/f'{state}-source-solid.png'))[:,:,3]>127
        image=np.zeros((512,512,3),np.uint8);image[expected&actual]=[50,150,70];image[expected&~actual]=[255,0,255];image[actual&~expected]=[30,120,255];Image.fromarray(image).save(dest/f'{state}-source-coverage.png')
        coverage.append(dict(state=state,native_coverage=float((expected&actual).sum()/expected.sum()),iou=float((expected&actual).sum()/(expected|actual).sum()),native_pixels=int(expected.sum()),actual_pixels=int(actual.sum())))
    (dest/'source-coverage.json').write_text(json.dumps(dict(model_sha256=sha(dest/'worker.blend'),results=coverage,measurement='Native alpha projected into exact source review camera; no silhouette clipping or alpha material applied to geometry'),indent=2)+'\n');print(coverage)
if __name__=='__main__':main()
