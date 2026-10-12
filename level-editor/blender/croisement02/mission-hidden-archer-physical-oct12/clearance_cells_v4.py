"""Bound complete intersecting triangle portions instead of isolated alpha witnesses."""
import os
os.environ['OPENBLAS_NUM_THREADS']='2'
import sys,json,math,collections
from pathlib import Path
import numpy as np
from scipy.ndimage import maximum_filter,gaussian_filter
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parent),str(HERE.parents[2]/'refinement')]
import qualify as q
import restart14_butterfly_canopy22_audit as reader
from restart14_butterfly07_anatomy_contacts import clip_planes
from resource_guard import guard_reader
from render_slots import acquire,release
DEST=q.OUT/'conformation-v4-cell-bounds'
def main():
 assert not DEST.exists()
 guard_reader(reader,q.ROOT,DEST)
 oldpath=q.OUT/'conformation-v2/depth-field.json';contactpath=q.OUT/'geometry-v3-contacts/report.json'
 data=json.loads(oldpath.read_text());contacts=json.loads(contactpath.read_text());qualification=json.loads((q.OUT/'qualification.json').read_text());anchors=json.loads((q.OUT/'depth-proposal-v1.json').read_text())
 assert data['map']==contacts['map']==qualification['map']
 requests=collections.defaultdict(set)
 for row in contacts['rows']:
  assert sum(row['triangle_cell_contacts'].values())==len(row['witnesses']), 'Capped witnesses cannot define complete requested cells'
  for w in row['witnesses']:requests[(w['asset'],w['node'],w['triangle'])].add((row['profile'],*w['source_cell']))
 seen=collections.Counter();seeds={k:np.zeros_like(v['camera_ray_advance'],dtype=float) for k,v in data['rows'].items()};evidence=[]
 def base(n,x,y):
  a=anchors[str(n)];box=qualification['rows'][n-1]['source_union_box'];t=(x-box[0])/(box[2]-box[0]);wy=a['world_y_left']*(1-t)+a['world_y_right']*t+a['depth_bulge']*math.sin(math.pi*t)
  return -wy/reader.COS-y*reader.SIN/reader.COS
 def inspect(placed,node,ni,pi,tri,uvs,mat,alpha,texture_record):
  matching=[k for k in requests if k[:2]==(placed['id'],node.get('name'))]
  xyz=np.stack([tri[:,:,0],reader.SIN*tri[:,:,2]-reader.COS*tri[:,:,1],reader.COS*tri[:,:,2]+reader.SIN*tri[:,:,1]],axis=2)
  for key in matching:
   assert key[2]<len(tri),key
   seen[key]+=1
   for n,x,y in sorted(requests[key]):
    clipped=clip_planes(xyz[key[2]],np.array([[-1,0,0,x],[1,0,0,-x-1],[0,-1,0,y],[0,1,0,-y-1]],dtype=float))
    assert len(clipped),key
    corners=[(x,y),(x+1,y),(x+1,y+1),(x,y+1)];row=data['rows'][str(n)];box=row['box'];a=anchors[str(n)]
    assert all(box[0]<=xx<=box[2] and box[1]<=yy<=box[3] for xx,yy in corners)
    # Linear receiver depth reaches its maximum at a clipped polygon vertex.
    # Assigning the bound at all four corners bounds both saved screen triangles.
    bound=max(0,float(clipped[:,2].max())+a['thickness']+.4-min(base(n,*p) for p in corners))
    for xx,yy in corners:seeds[str(n)][yy-box[1],xx-box[0]]=max(seeds[str(n)][yy-box[1],xx-box[0]],bound)
    evidence.append(dict(asset=key[0],node=key[1],triangle=key[2],profile=n,cell=[x,y],receiver_depth_max=float(clipped[:,2].max()),advance=bound))
 def finish(rays,records,mp):
  assert all(seen[k]==1 for k in requests),dict(seen)
  for n,row in data['rows'].items():
   old=np.array(row['camera_ray_advance']);required=np.maximum(old,seeds[n]);field=np.maximum(required,gaussian_filter(maximum_filter(required,size=5),1.0));row.update(camera_ray_advance=field.tolist(),maximum_advance=float(field.max()),maximum_added_advance=float((field-old).max()))
  data.update(status='PRIVATE_CLEARANCE_HYPOTHESIS_REQUIRES_MODEL_AND_FULL_RECHECK',input_field=q.pin(oldpath),contact_source=q.pin(contactpath),receiver_records=records,cell_bounds=evidence)
  data['limits'].append('The entire contacting receiver triangle inside each cell is bounded, including transparent parts. This deliberately conservative hidden-depth hypothesis preserves projected source coordinates; it is not a complete collision proof or support claim.')
  DEST.mkdir();(DEST/'depth-field.json').write_text(json.dumps(data,indent=2)+'\n');print({n:{'maximum':r['maximum_advance'],'added':r['maximum_added_advance']}for n,r in data['rows'].items()},flush=True)
 rays=[dict(screen=[x+.5,y+.5],hits=[]) for x,y in sorted({(x,y) for cells in requests.values() for n,x,y in cells})]
 reader.main(ray_records=rays,triangle_callback=inspect,postprocess=finish,output=DEST,asset_ids={k[0]for k in requests},minimum_free_bytes=10*1024**3,output_limit_bytes=16*1024**2,expected_map_sha256=qualification['map']['sha256'])
if __name__=='__main__':
 acquire(slots=2)
 try:main()
 finally:release()
