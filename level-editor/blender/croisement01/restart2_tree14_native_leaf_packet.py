"""Partition exact native leaf RGBA with spatial support for the entire animation."""
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image

ROOT=Path(__file__).resolve().parents[3]
R=ROOT/'level-editor/work/active/croisement01/restart2'
out=R/'tree14-native-crown-animation-source-v3';out.mkdir(exist_ok=False)
source=json.loads((R/'tree14-native-crown-animation-source-v1/source.json').read_text())
left=min(f['bbox'][0] for f in source['frames']);top=min(f['bbox'][1] for f in source['frames'])
right=max(f['bbox'][0]+f['bbox'][2] for f in source['frames']);bottom=max(f['bbox'][1]+f['bbox'][3] for f in source['frames'])
width,height=right-left,bottom-top
frames=[];union=np.zeros((height,width),bool)
for frame in source['frames']:
 path=ROOT/frame['path'];assert hashlib.sha256(path.read_bytes()).hexdigest()==frame['sha256']
 raw=np.asarray(Image.open(path).convert('RGBA'));x,y,w,h=frame['bbox'];assert raw.shape==(h,w,4)
 rgba=np.zeros((height,width,4),np.uint8);rgba[y-top:y-top+h,x-left:x-left+w]=raw
 frames.append(rgba);union|=rgba[:,:,3]>0
first=frames[0];yy,xx=np.mgrid[:height,:width];sx,sy=xx+left,yy+top
seeds=np.asarray([(780,90),(825,130),(884,155),(725,165),(795,205),(915,215),(720,235),(850,285)])
distance=(sx[:,:,None]-seeds[:,0])**2+(sy[:,:,None]-seeds[:,1])**2
labels=np.argmin(distance,axis=2);covered=np.zeros(union.shape,bool);lobes=[]
for index,(cx,cy) in enumerate(seeds):
 nearest=union&(labels==index);assert np.any(nearest)
 radius=float(np.sqrt(distance[:,:,index][nearest].max())*1.25)
 angle=np.arctan2(sy-cy,sx-cx)
 support=np.sqrt(distance[:,:,index])<radius*(.94+.035*np.sin(angle*11+index)+.025*np.cos(angle*17-index))
 owned=union&support;covered|=owned;ys,xs=np.nonzero(owned)
 x0,x1=max(0,int(xs.min())-2),min(width,int(xs.max())+3)
 y0,y1=max(0,int(ys.min())-2),min(height,int(ys.max())+3)
 animated=[]
 for number,rgba in enumerate(frames):
  leaf=rgba[y0:y1,x0:x1].copy();leaf[:,:,3]*=support[y0:y1,x0:x1]
  front=out/f'lobe-{index:02}-frame-{number:02}.png';Image.fromarray(leaf).save(front)
  active=leaf[:,:,3]>0;assert np.array_equal(leaf[active],rgba[y0:y1,x0:x1][active])
  animated.append(dict(frame=number,path=str(front),sha256=hashlib.sha256(front.read_bytes()).hexdigest(),duration_ticks=source['frames'][number]['duration_ticks']))
 hidden=np.full((y1-y0,x1-x0,4),105,np.uint8);hidden[:,:,3]=owned[y0:y1,x0:x1]*255
 back=out/f'lobe-{index:02}-unknown.png';Image.fromarray(hidden).save(back)
 lobes.append(dict(index=index,bbox_source=[left+x0,top+y0,left+x1,top+y1],source=animated[0]['path'],unknown=str(back),
  source_sha256=animated[0]['sha256'],unknown_sha256=hashlib.sha256(back.read_bytes()).hexdigest(),
  native_pixels=int(np.count_nonzero((first[:,:,3]>0)&support)),animated_union_pixels=int(owned.sum()),depth_radius=210.,frames=animated))
assert np.array_equal(covered,union)
for number,rgba in enumerate(frames):
 reconstructed=np.zeros((height,width),bool)
 for lobe in lobes:
  x0,y0,x1,y1=lobe['bbox_source'];leaf=np.asarray(Image.open(lobe['frames'][number]['path']))
  reconstructed[y0-top:y1-top,x0-left:x1-left]|=leaf[:,:,3]>0
 assert np.array_equal(reconstructed,rgba[:,:,3]>0)
report=dict(status='NATIVE_LEAF_SOURCE_PACKET_NO_GEOMETRY_APPROVAL',native_animation=1,profile=source['profile'],native_frame=0,
 source_rgb_sha256=source['frames'][0]['sha256'],native_alpha_sha256=hashlib.sha256(first[:,:,3].tobytes()).hexdigest(),
 canopy_pixels=int(np.count_nonzero(first[:,:,3])),animated_union_pixels=int(union.sum()),bbox_source=[left,top,right,bottom],
 lobes=lobes,frame_records=source['frames'],loop_ticks=source['total_ticks'],all_fourteen_frame_rgba_unions_exact=True,
 source_ownership='Exact isolated leaf sprite RGBA. Moving leaves may overlay observed background wood; no background crop or occupancy-overlap subtraction.',
 native94_role='Occlusion extent only; it is not isolated material artwork.',
 limitations=['Geometry, hidden depth, unknown reverse texture and native animation integration remain unreviewed.'])
(out/'source-partition.json').write_text(json.dumps(report,indent=2)+'\n')
print({'native_pixels':report['canopy_pixels'],'animated_union_pixels':int(union.sum()),'lobes':len(lobes),'all_fourteen_frame_rgba_unions_exact':True})
