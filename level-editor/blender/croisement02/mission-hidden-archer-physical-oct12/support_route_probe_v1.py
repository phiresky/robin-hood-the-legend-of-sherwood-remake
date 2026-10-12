"""Measure static occlusion along possible hidden stems; never treat it as support."""
import os
os.environ['OPENBLAS_NUM_THREADS']='2'
import sys,json,math
import numpy as np
from scipy.ndimage import label,distance_transform_edt
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parent),str(HERE.parents[2]/'refinement')]
import qualify as q
import restart14_butterfly_canopy22_audit as reader
from resource_guard import guard_reader
from render_slots import acquire,release
DEST=q.OUT/'support-route-probe-v1'
def main():
 assert not DEST.exists();guard_reader(reader,q.ROOT,DEST)
 source=q.OUT/'support-island-diagnosis-v1.json';diagnosis=json.loads(source.read_text());qualification=json.loads((q.OUT/'qualification.json').read_text());rays=[]
 for row in diagnosis['rows']:
  if row['profile']!=qualification['rows'][3]['profile']:continue
  for i,c in enumerate(row['detached_components']):
   if c['pixels']<20:continue
   p0,p1=c['candidate_link_local_pixels'];steps=max(2,math.ceil(c['nearest_main_distance']*2))
   for j in range(steps+1):
    t=j/steps;x=row['origin'][0]+p0[0]*(1-t)+p1[0]*t;y=row['origin'][1]+p0[1]*(1-t)+p1[1]*t
    rays.append(dict(profile=4,state=row['state'],frame=row['frame'],component=i,fraction=t,screen=[x,y],hits=[]))
 # Sample opaque interiors of major detached clusters for independent trunk attachments.
 for state in ['initial','transition']:
  for f in qualification['rows'][3]['art']['states'][state]['frames']:
   a=np.array(q.rgba(f));mask=a[:,:,3]>0;labels,n=label(mask,np.ones((3,3)));sizes=np.bincount(labels.ravel());sizes[0]=0;largest=int(sizes.argmax())
   for k in range(1,n+1):
    if k==largest or sizes[k]<20:continue
    cm=labels==k;distance=distance_transform_edt(cm);yy,xx=np.where(cm);chosen={(int(x),int(y)) for x,y in zip(xx,yy) if x%3==0 and y%3==0};best=np.unravel_index(int(distance.argmax()),distance.shape);chosen.add((int(best[1]),int(best[0])))
    for x,y in sorted(chosen):rays.append(dict(profile=4,state=state,frame=f['index'],component_label=k,kind='opaque_component_attachment',component_pixels=int(sizes[k]),source_interior_radius=float(distance[y,x]),screen=[f['top_left'][0]+x+.5,f['top_left'][1]+y+.5],hits=[]))
 def finish(rays,records,mp):
  DEST.mkdir();(DEST/'report.json').write_text(json.dumps(dict(status='SOURCE_RAY_OCCLUSION_DIAGNOSIS_NOT_SUPPORT_GEOMETRY',source=q.pin(source),qualification=q.pin(q.OUT/'qualification.json'),map=q.pin(mp),receivers=records,rays=rays,limits=['Straight screen-space links are candidate routes only.','Foreground receiver depth may conceal a branch, but neither proves attachment nor permits crossing that receiver surface.','Full saved geometry plus source-camera coverage is required for any subsequently constructed support.']),indent=2)+'\n')
 assets={c['id']for c in qualification['rows'][3]['bounds_candidates']}
 reader.main(ray_records=rays,postprocess=finish,output=DEST,asset_ids=assets,minimum_free_bytes=10*1024**3,output_limit_bytes=16*1024**2,expected_map_sha256=qualification['map']['sha256'])
if __name__=='__main__':
 acquire(slots=2)
 try:main()
 finally:release()
