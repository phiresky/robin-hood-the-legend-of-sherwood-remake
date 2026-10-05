"""Locate unobserved wall-cap pixels and expose overlapping native owners."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement03-refinement'
def main():
    w=OUT/'restart2/stone-wall-v1/assets/croisement03-southeast-stone-wall';r=json.loads((w/'inspection/source-comparison/report.json').read_text());x,y,x1,y1=r['crop'];a=np.asarray(Image.open(w/'inspection/source-comparison/render.png').convert('RGBA'))
    assert hashlib.sha256((w/'model.blend').read_bytes()).hexdigest()==r['model_sha256']
    grey=(a[:,:,3]>127)&(np.ptp(a[:,:,:3].astype(int),axis=2)<2)&(a[:,:,0]>30)&(a[:,:,0]<220);yy,xx=np.indices(grey.shape);grey&=(xx+x>1220)&(xx+x<1280)&(yy+y<805)
    coords=[[int(px+x),int(py+y)] for py,px in zip(*np.nonzero(grey))];level=json.loads((OUT/'baseline/Croisement03.rhp.json').read_text());owners={}
    for i,row in enumerate(level['masks']):
        ox,oy=row['box_top_left'];width,height=row['box_size'];im=np.asarray(Image.open(OUT/f'baseline/masks/{i:06}.png'))
        count=sum(0<=px-ox<width and 0<=py-oy<height and im[py-oy,px-ox]>0 for px,py in coords)
        if count:owners[i]=int(count)
    source=Image.open(OUT/'baseline/covered.png').convert('RGB');arr=np.asarray(source).copy()
    for px,py in coords:arr[py,px]=[255,0,255]
    target=OUT/'restart2/southeast-wall-source';Image.fromarray(arr).crop((1220,765,1285,812)).resize((650,470),Image.Resampling.NEAREST).save(target/'gray-patch-source.png')
    record=dict(model_sha256=r['model_sha256'],pixels=coords,native_mask_membership=owners,interpretation='Source close-up visually identifies bark in wood25. Coarse building046 does not supply refined source-shaped occlusion. Keep these wall faces unknown until refined tree integration; do not paint bark onto stone.',scope='Diagnostic mask overlap is not automatic reassignment or proof of a completed geometric joint.')
    (target/'gray-owner-audit.json').write_text(json.dumps(record,indent=2)+'\n');print(len(coords),owners)
if __name__=='__main__':main()
