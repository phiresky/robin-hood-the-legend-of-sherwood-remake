"""Measure native cart-roof translation evidence, leaving breakup and actors unresolved."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from scipy.signal import correlate
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/croisement02-refinement'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    path=OUT/'state-target-evidence/north-cart/manifest.json';manifest=json.loads(path.read_text());part=manifest['parts'][0];frames=part['frames'];dest=OUT/'north-cart-roof-motion-v2';dest.mkdir(exist_ok=False)
    initial=np.array(Image.open(frames[0]['image']).convert('RGBA'));polygon=Image.new('L',(initial.shape[1],initial.shape[0]));ImageDraw.Draw(polygon).polygon([(145,3),(198,11),(157,51),(104,40)],fill=255);mask=(np.array(polygon,dtype=float)/255)*(initial[:,:,3]>0);ys,xs=np.nonzero(mask);x0,x1,y0,y1=xs.min(),xs.max()+1,ys.min(),ys.max()+1;mask=mask[y0:y1,x0:x1];template=(initial[:,:,:3]@np.array([.299,.587,.114]))[y0:y1,x0:x1];n=mask.sum();centered=(template-(template*mask).sum()/n)*mask;energy=(centered**2).sum();records=[]
    base_origin=np.array(part['position'])+np.array(frames[0]['offset']);base_anchor=base_origin+[x0,y0]
    for frame in frames:
        imagepath=Path(frame['image']);im=np.array(Image.open(imagepath).convert('RGBA'));gray=im[:,:,:3]@np.array([.299,.587,.114]);sums=correlate(gray,mask,mode='valid',method='fft');sq=correlate(gray**2,mask,mode='valid',method='fft');den=np.sqrt(np.maximum(sq-sums*sums/n,1e-9)*energy);scores=correlate(gray,centered,mode='valid',method='fft')/den;opaque=correlate((im[:,:,3]>0).astype(float),mask,mode='valid',method='fft');scores[opaque<n*.98]=-1;y,x=np.unravel_index(scores.argmax(),scores.shape);best=float(scores[y,x]);other=scores.copy();other[max(0,y-4):y+5,max(0,x-4):x+5]=-1;gap=best-float(other.max());global_anchor=np.array(part['position'])+np.array(frame['offset'])+[x,y];records.append(dict(frame=frame['index'],tick=frame['index'],native_image_sha256=sha(imagepath),source_anchor=global_anchor.tolist(),delta_from_initial=(global_anchor-base_anchor).tolist(),ncc=best,uniqueness_gap=gap,translation_supported=best>=.97 and gap>=.04))
    assert records[0]['ncc']>.999 and records[0]['delta_from_initial']==[0,0], 'Identity source match must recover itself'
    prefix=[]
    for record in records:
        if not record['translation_supported']:break
        prefix.append(record['tick'])
    result=dict(status='native roof-only translation diagnostic; no body or wheel motion approval',source_manifest_sha256=sha(path),template_polygon=[[145,3],[198,11],[157,51],[104,40]],template_pixels=int(n),parameters=dict(ncc_min=.97,uniqueness_min=.04,opaque_fraction=.98),consecutive_supported_ticks=prefix,records=records,limitations=['Roof texture agreement cannot prove unchanged wheels, shafts, horses or rigid cart body.','No world-height or terrain contact inferred from screen translation.','Breakup, wheel rotation, fabric deformation and actor movement require independent source correspondence.'])
    (dest/'manifest.json').write_text(json.dumps(result,indent=2)+'\n');print('supported prefix',prefix);print([(r['tick'],round(r['ncc'],3),r['delta_from_initial'])for r in records[::10]])
if __name__=='__main__':main()
