"""Measure local receiver clearance without treating canopy faces as supports."""
import os
os.environ['OPENBLAS_NUM_THREADS']='2'
import sys,json,math,shutil
from pathlib import Path
import numpy as np
from scipy.ndimage import maximum_filter,gaussian_filter
HERE=Path(__file__).resolve().parent;sys.path[:0]=[str(HERE.parent),str(HERE.parents[2]/'refinement')]
import qualify as q
import restart14_butterfly_canopy22_audit as reader
from render_slots import acquire,release
DEST=q.OUT/'conformation-v1'
SCOPES={1:([153,110,210,177],{'croisement02-tree-00'}),3:([1558,148,1572,166],{'croisement02-tree-16'}),4:([2,866,143,953],{'croisement02-tree-24','croisement02-tree-25','croisement02-shrub-80'})}
def main():
 assert not DEST.exists();report=json.loads((q.OUT/'qualification.json').read_text());anchors=json.loads((q.OUT/'depth-proposal-v1.json').read_text());rays=[]
 for n,(box,assets)in SCOPES.items():
  for y in range(box[1],box[3]+1):
   for x in range(box[0],box[2]+1):rays.append(dict(profile=n,screen=[x,y],hits=[]))
 def finish(rays,records,mp):
  rows={};near_count=0
  for n,(box,assets)in SCOPES.items():
   a=anchors[str(n)];b=report['rows'][n-1]['source_union_box'];required=np.zeros((box[3]-box[1]+1,box[2]-box[0]+1));witness=[];excluded_far=0
   for ray in rays:
    if ray['profile']!=n:continue
    x,y=ray['screen'];t=(x-b[0])/(b[2]-b[0]);wy=a['world_y_left']*(1-t)+a['world_y_right']*t+a['depth_bulge']*math.sin(math.pi*t);front=-wy/reader.COS-y*reader.SIN/reader.COS
    hits=[h for h in ray['hits']if h['asset']in assets and h['passes_alpha_and_culling']]
    excluded_far+=sum(h['camera_depth']>front+8 for h in hits)
    near=[h for h in hits if front-a['thickness']-1<=h['camera_depth']<=front+8]
    if near:
     h=max(near,key=lambda h:h['camera_depth']);delta=max(0,h['camera_depth']+a['thickness']+.4-front);required[y-box[1],x-box[0]]=delta;near_count+=1
     if len(witness)<150:witness.append(dict(screen=[x,y],asset=h['asset'],node=h['node'],receiver_depth=h['camera_depth'],base_front_depth=front,required_camera_ray_advance=delta))
   field=np.maximum(required,gaussian_filter(maximum_filter(required,size=5),1.0));rows[str(n)]=dict(box=box,camera_ray_advance=field.tolist(),maximum_advance=float(field.max()),required_nonzero_corners=int((required>0).sum()),excluded_far_foreground_hits=excluded_far,witnesses=witness,classification='Local clearance from existing surfaces; Crown nodes are not structural attachment evidence. Sparse sampling requires whole-volume recheck.')
  DEST.mkdir();(DEST/'depth-field.json').write_text(json.dumps(dict(map=q.pin(mp),source_qualification=q.pin(q.OUT/'qualification.json'),base_depth=q.pin(q.OUT/'depth-proposal-v1.json'),receivers=records,rows=rows,limits=['Only near intersecting receiver surfaces, within eight ray units ahead of the initial front, guide conformation.','More distant foreground crown occlusion remains unresolved, not fitted by moving screens into canopy height.','Source and camera projection fixed. Grid-corner clearance is not an intersection proof; recheck triangulated volumes.']),indent=2)+'\n');print('MEASURED',near_count,flush=True)
 reader.main(ray_records=rays,postprocess=finish,output=DEST,asset_ids=set().union(*(a for _,a in SCOPES.values())),minimum_free_bytes=10*1024**3,output_limit_bytes=16*1024**2,expected_map_sha256=report['map']['sha256'])
if __name__=='__main__':
 acquire(slots=2)
 try:main()
 finally:release()
