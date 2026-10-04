"""Check state-off preservation and scoped native log visibility after appearance gating."""
import json
import sys
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.ndimage import binary_dilation
from catalog import OUT
from native_log_foreground_reference import sha


def main():
    proof=OUT/(sys.argv[1] if len(sys.argv)>1 else 'log-state-appearance-proof-v4');manifest=json.loads((proof/'manifest.json').read_text());root=OUT/'state-target-evidence/log-trap';reference=root/'native-order-reference'
    box=manifest['camera']['bbox'];w=box[2]-box[0];h=box[3]-box[1];scale=manifest['camera']['ortho_scale'];yy,xx=np.mgrid[:512,:512];ix=np.floor(w/2+(xx+.5-256)*scale/512).astype(int);iy=np.floor(h/2+(yy+.5-256)*scale/512).astype(int);valid=(ix>=0)&(ix<w)&(iy>=0)&(iy<h)
    def project(mask):
        result=np.zeros((512,512),bool);result[valid]=mask[iy[valid],ix[valid]];return result
    def rgba(name):return np.array(Image.open(proof/f'{name}.png'))
    def emission(name):
        p=rgba(name);return(p[:,:,0]>240)&(p[:,:,1]<15)&(p[:,:,2]>240)
    native=np.array(Image.open(root/'tick-089.png'))[:,:,3]>0;footprint=project(native);expected=project(np.array(Image.open(reference/'visible-log-phase0.png'))>0)
    body=emission('logs-only-visibility');actual=emission('applied-phase-00-visibility')
    initial_delta=np.abs(rgba('initial-before').astype(int)-rgba('initial-after-state-off').astype(int));applied_delta=np.abs(rgba('applied-before').astype(int)-rgba('applied-phase-00').astype(int));outside=~binary_dilation(footprint,iterations=3)
    result=dict(status='diagnostic; inspect preservation and visibility before accepting prototype',manifest_sha256=sha(proof/'manifest.json'),initial_state_off=dict(changed_pixels=int(np.any(initial_delta>0,axis=2).sum()),maximum_channel_delta=int(initial_delta.max()),mean_channel_delta=float(initial_delta.mean())),outside_native_wood_footprint_with_3_render_pixel_antialias_margin=dict(pixels=int(outside.sum()),changed_pixels=int((np.any(applied_delta>0,axis=2)&outside).sum()),maximum_channel_delta=int(applied_delta[outside].max()),mean_channel_delta=float(applied_delta[outside].mean())),visibility=dict(expected_visible=int(expected.sum()),correctly_visible=int((expected&actual).sum()),missing_geometry=int((expected&~body&~actual).sum()),native_visible_still_hidden=int((expected&body&~actual).sum()),native_covered_wood_exposed=int((actual&footprint&~expected).sum()),unsupported_continuation_visible=int((actual&~footprint).sum())),limitations=['Outside-footprint image differences include renderer sampling changes; any significant difference must be resolved before a preservation claim.','Exact geometry and UV identity is separately bound by the renderer manifest; this image test does not replace it.'])
    rgb=np.zeros((512,512,3),np.uint8);rgb[expected&actual]=(50,200,80);rgb[expected&~body&~actual]=(180,20,230);rgb[expected&body&~actual]=(240,40,40);rgb[actual&footprint&~expected]=(240,160,20);rgb[actual&~footprint]=(30,100,240);Image.fromarray(rgb).save(proof/'native-visibility-comparison.png')
    (proof/'measurement.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))


if __name__=='__main__':main()
