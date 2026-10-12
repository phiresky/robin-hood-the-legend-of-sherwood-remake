"""Partition isolated native leaf RGBA, never background sampled through an occlusion mask."""
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image

ROOT=Path(__file__).resolve().parents[3]
R=ROOT/'level-editor/work/croisement01-refinement/restart2'
out=R/'tree14-native-crown-animation-source-v2';out.mkdir(exist_ok=False)
source=json.loads((R/'tree14-native-crown-animation-source-v1/source.json').read_text())
frame=source['frames'][0];path=ROOT/frame['path'];raw=path.read_bytes()
assert hashlib.sha256(raw).hexdigest()==frame['sha256']
rgba=np.asarray(Image.open(path).convert('RGBA'));left,top,width,height=frame['bbox']
assert rgba.shape==(height,width,4)
alpha=rgba[:,:,3]>0;yy,xx=np.mgrid[:height,:width];sx,sy=xx+left,yy+top
seeds=np.asarray([(780,90),(825,130),(884,155),(725,165),(795,205),(915,215),(720,235),(850,285)])
distance=(sx[:,:,None]-seeds[:,0])**2+(sy[:,:,None]-seeds[:,1])**2
labels=np.argmin(distance,axis=2);covered=np.zeros(alpha.shape,bool);lobes=[]
for index,(cx,cy) in enumerate(seeds):
    nearest=alpha&(labels==index);assert np.any(nearest)
    radius=float(np.sqrt(distance[:,:,index][nearest].max())*1.25)
    angle=np.arctan2(sy-cy,sx-cx)
    support=np.sqrt(distance[:,:,index])<radius*(.94+.035*np.sin(angle*11+index)+.025*np.cos(angle*17-index))
    owned=alpha&support;covered|=owned;ys,xs=np.nonzero(owned)
    x0,x1=max(0,int(xs.min())-2),min(width,int(xs.max())+3)
    y0,y1=max(0,int(ys.min())-2),min(height,int(ys.max())+3)
    leaf=rgba[y0:y1,x0:x1].copy();leaf[:,:,3]*=support[y0:y1,x0:x1]
    front=out/f'lobe-{index:02}-native.png';Image.fromarray(leaf).save(front)
    hidden=leaf.copy();hidden[:,:,:3]=105
    back=out/f'lobe-{index:02}-unknown.png';Image.fromarray(hidden).save(back)
    lobes.append(dict(index=index,bbox_source=[left+x0,top+y0,left+x1,top+y1],source=str(front),unknown=str(back),
        source_sha256=hashlib.sha256(front.read_bytes()).hexdigest(),unknown_sha256=hashlib.sha256(back.read_bytes()).hexdigest(),
        native_pixels=int(owned.sum()),depth_radius=210.))
assert np.array_equal(covered,alpha)
report=dict(status='NATIVE_LEAF_SOURCE_PACKET_NO_GEOMETRY_APPROVAL',native_animation=1,profile=source['profile'],native_frame=0,
    source_rgb_sha256=frame['sha256'],native_alpha_sha256=hashlib.sha256(rgba[:,:,3].tobytes()).hexdigest(),
    canopy_pixels=int(alpha.sum()),lobes=lobes,frame_records=source['frames'],loop_ticks=source['total_ticks'],
    source_ownership='Exact isolated leaf sprite RGBA. Moving leaves may overlay observed background wood; no background crop or occupancy-overlap subtraction.',
    native94_role='Occlusion extent only; it is not isolated material artwork.',
    limitations=['Geometry, hidden depth, unknown reverse texture and native animation integration remain unreviewed.'])
(out/'source-partition.json').write_text(json.dumps(report,indent=2)+'\n')
bg=Image.new('RGBA',(width,height),(65,65,65,255));bg.alpha_composite(Image.fromarray(rgba))
bg.resize((width*3,height*3),Image.Resampling.NEAREST).save(out/'native-leaf-source.png')
print({'native_pixels':int(alpha.sum()),'lobes':len(lobes),'exact_alpha_union':True})
