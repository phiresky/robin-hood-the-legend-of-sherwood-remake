"""Separate native evidence from inferred occlusion of complete log hypotheses."""
import hashlib,json,math
import numpy as np
from PIL import Image
from catalog import OUT

def main():
    root=OUT/'state-target-evidence/log-trap';m=json.loads((root/'manifest.json').read_text());left,top,right,bottom=m['bbox'];w=right-left;h=bottom-top;native=np.array(Image.open(root/'tick-089.png'))[:,:,3]>0;ever=np.zeros_like(native);counts=np.zeros_like(native,dtype=np.uint16);bindings=[]
    for part in m['parts']:
        for frame in part['frames']:
            x,y=[int(math.floor(a+b+.5))for a,b in zip(part['position'],frame['offset'])];a=np.array(Image.open(frame['image']))[:,:,3]>0;l=max(x,left);t=max(y,top);r=min(x+a.shape[1],right);b=min(y+a.shape[0],bottom)
            if r>l and b>t:ever[t-top:b-top,l-left:r-left]|=a[t-y:b-y,l-x:r-x];counts[t-top:b-top,l-left:r-left]+=a[t-y:b-y,l-x:r-x]
            bindings.append(frame['image_sha256'])
    candidate=OUT/'log-trap-state-candidate-v7';solid=np.array(Image.open(candidate/'applied-source-solid.png'))[:,:,3]>127;scale=max(w,h)*1.2;yy,xx=np.mgrid[:h,:w];rx=np.floor(256+((xx+.5)-w/2)*512/scale).astype(int);ry=np.floor(256+((yy+.5)-h/2)*512/scale).astype(int);body=solid[ry,rx];unknown=body&~native;potential=np.zeros_like(native);owners=[]
    masks=json.loads((OUT/'baseline/masks/manifest.json').read_text())['masks']
    for mask in masks:
        if mask['index']not in (5,6,29,30,62,63,130,133):continue
        x,y=mask['box_top_left'];mw,mh=mask['box_size'];l=max(x,left);t=max(y,top);r=min(x+mw,right);b=min(y+mh,bottom)
        if r<=l or b<=t:continue
        a=np.array(Image.open(OUT/'baseline/masks'/mask['png']).convert('L'))>0;canvas=np.zeros_like(native);canvas[t-top:b-top,l-left:r-left]=a[t-y:b-y,l-x:r-x];potential|=canvas;owners.append(dict(global_mask=mask['index'],native_mask_type=mask['mask_type'],character_mask=bool(mask['mask_type']&1),projectile_mask=bool(mask['mask_type']&2),unknown_overlap=int((unknown&canvas).sum()),unknown_never_native_wood=int((unknown&canvas&~ever).sum()),unknown_wood_in_other_frame=int((unknown&canvas&ever).sum())))
    rgb=np.zeros((h,w,3),np.uint8);rgb[native]=(75,170,75);rgb[unknown&potential&~ever]=(50,100,230);rgb[unknown&potential&ever]=(220,150,20);rgb[unknown&~potential]=(230,30,190);Image.fromarray(rgb).resize((w*3,h*3),Image.Resampling.NEAREST).save(candidate/'occlusion-evidence.png')
    report=dict(status='Evidence only; actual foreground depth review required',native_target_masking='These target FX do not request runtime sprite masking. Their transparent holes are already present in frame RGBA. Mask ownership overlap is not permission to apply another runtime mask.',model_sha256=hashlib.sha256((candidate/'worker.blend').read_bytes()).hexdigest(),native_frame_hashes=bindings,counts=dict(unknown_projection=int(unknown.sum()),potential_foreground_never_wood=int((unknown&potential&~ever).sum()),potential_foreground_wood_elsewhere=int((unknown&potential&ever).sum()),no_native_mask_support=int((unknown&~potential).sum())),mask_overlaps=owners,limitations=['Always-transparent pixels inside candidate source-owner domains support but do not prove hidden geometry.', 'Native canopy mask130 is projectile-only (type2); it does not define character/target foreground clipping. Wood mask30 is type7.','Pixels visible as wood at other times require temporal depth/ownership validation; a permanent generic alpha cut is not justified.','Masks overlap: individual owner counts are not exclusive and must not be added.','Current complete-body surveys are not motion identities or an approved count of distinct physical logs.']);(candidate/'occlusion-evidence.json').write_text(json.dumps(report,indent=2)+'\n');print(report['counts'])
if __name__=='__main__':main()
