"""Measure complete projected-cell foreground bounds before choosing foliage depth."""
import os
os.environ['OPENBLAS_NUM_THREADS']='2'
import sys,json,math
from pathlib import Path
import numpy as np
from scipy.ndimage import maximum_filter,gaussian_filter
HERE=Path(__file__).resolve().parent;sys.path[:0]=[str(HERE),str(HERE.parent),str(HERE.parents[2]/'refinement')]
import qualify as q
import restart14_butterfly_canopy22_audit as reader
from restart14_butterfly07_anatomy_contacts import clip_planes
from restart406_alpha_contact_predicate import threshold_contact
from resource_guard import guard_reader
from render_slots import acquire,release
DEST=q.OUT/'conformation-v7-frontmost-cell-bounds'
SCOPES={1:('croisement02-tree-00','Northwest Tree 00 / Crown'),4:('croisement02-shrub-80','Understory Shrub 80')}
def main():
 assert not DEST.exists();guard_reader(reader,q.ROOT,DEST)
 qualification=json.loads((q.OUT/'qualification.json').read_text());baselinepath=q.OUT/'conformation-v5-local-cell-bounds/depth-field.json';result=json.loads(baselinepath.read_text());anchors=json.loads((q.OUT/'depth-proposal-v1.json').read_text());units={}
 for n in SCOPES:
  row=qualification['rows'][n-1];pixels=set()
  for f in row['art']['states']['initial']['frames']+row['art']['states']['transition']['frames']:
   yy,xx=np.where(np.array(q.rgba(f))[:,:,3]>0);x0,y0=map(int,f['top_left']);pixels.update((int(x+x0),int(y+y0))for x,y in zip(xx,yy))
  old=result['rows'][str(n)];box=[min(old['box'][0],min(x for x,y in pixels)-4),min(old['box'][1],min(y for x,y in pixels)-4),max(old['box'][2],max(x for x,y in pixels)+5),max(old['box'][3],max(y for x,y in pixels)+5)];grid=np.zeros((box[3]-box[1]+1,box[2]-box[0]+1));a,b,c,d=old['box'];grid[b-box[1]:d-box[1]+1,a-box[0]:c-box[0]+1]=old['camera_ray_advance'];units[n]=dict(pixels=pixels,box=box,baseline=grid,frontmost={},counts=0)
 def base(n,x,y):
  a=anchors[str(n)];b=qualification['rows'][n-1]['source_union_box'];t=(x-b[0])/(b[2]-b[0]);wy=a['world_y_left']*(1-t)+a['world_y_right']*t+a['depth_bulge']*math.sin(math.pi*t);return -wy/reader.COS-y*reader.SIN/reader.COS
 def inspect(placed,node,ni,pi,tri,uvs,mat,alpha,texture_record):
  eligible=[n for n,scope in SCOPES.items()if scope==(placed['id'],node.get('name'))]
  if not eligible:return
  xyz=np.stack([tri[:,:,0],reader.SIN*tri[:,:,2]-reader.COS*tri[:,:,1],reader.COS*tri[:,:,2]+reader.SIN*tri[:,:,1]],axis=2);uv=np.zeros((*tri.shape[:2],2))if uvs is None else uvs;attrs=np.concatenate([xyz,uv],axis=2);lo=xyz.min(1);hi=xyz.max(1);opaque=mat.get('alphaMode','OPAQUE')=='OPAQUE';image,sampler,factor=(None,{},1.)if opaque else texture_record(mat);cut=mat.get('alphaCutoff',.5)if mat.get('alphaMode')=='MASK'else .01
  for n in eligible:
   unit=units[n];box=unit['box'];indices=np.flatnonzero((hi[:,0]>=box[0])&(lo[:,0]<=box[2])&(hi[:,1]>=box[1])&(lo[:,1]<=box[3]))
   for ti in indices:
    for x in range(max(box[0],math.floor(lo[ti,0])),min(box[2],math.floor(hi[ti,0])+1)):
     for y in range(max(box[1],math.floor(lo[ti,1])),min(box[3],math.floor(hi[ti,1])+1)):
      if(x,y)not in unit['pixels']:continue
      poly=clip_planes(attrs[ti],np.array([[-1,0,0,x],[1,0,0,-x-1],[0,-1,0,y],[0,1,0,-y-1]],float))
      if not len(poly):continue
      decision=threshold_contact(poly,image,sampler,factor,max(1e-12,cut-1e-8))
      if not decision['intersects']:continue
      unit['counts']+=1;bound=float(poly[:,2].max());old=unit['frontmost'].get((x,y))
      if old is None or bound>old['depth']:unit['frontmost'][(x,y)]=dict(depth=bound,asset=placed['id'],node=node.get('name'),node_index=int(ni),primitive_index=int(pi),triangle=int(ti),alpha_witness_value=decision['value'],alpha_mode=decision['mode'])
 def finish(rays,records,mp):
  evidence=[]
  for n,u in units.items():
   box=u['box'];seed=np.zeros_like(u['baseline'])
   for (x,y),cell in u['frontmost'].items():
    corners=[(x,y),(x+1,y),(x+1,y+1),(x,y+1)];advance=max(0,cell['depth']+anchors[str(n)]['thickness']+.4-min(base(n,*p)for p in corners))
    for xx,yy in corners:seed[yy-box[1],xx-box[0]]=max(seed[yy-box[1],xx-box[0]],advance)
    evidence.append(dict(profile=n,cell=[x,y],advance=advance,**cell))
   field=np.maximum(u['baseline'],np.maximum(seed,gaussian_filter(maximum_filter(seed,size=5),1.0)));result['rows'][str(n)].update(box=box,camera_ray_advance=field.tolist(),maximum_advance=float(field.max()),maximum_added_advance=float((field-u['baseline']).max()),measured_foreground_cells=len(u['frontmost']),triangle_cell_pairs=u['counts'])
  result.update(status='MEASUREMENT_ONLY_REVIEW_DEPTH_BEFORE_CONSTRUCTION',foreground_cell_bounds=evidence,input_field=q.pin(baselinepath),foreground_receivers=records,alpha_predicate=q.pin(HERE.parent/'restart406_alpha_contact_predicate.py'))
  result['limits'].append('Complete projected-cell bounds include receiver surfaces in front of the old screen, not only intersecting surfaces. Bounds conservatively include transparent parts of a triangle whenever any part is opaque. Large depth changes are hypotheses requiring context/support review, not automatic construction approval.')
  DEST.mkdir();(DEST/'depth-field.json').write_text(json.dumps(result,indent=2)+'\n');print({n:{k:r[k]for k in ['maximum_advance','maximum_added_advance','measured_foreground_cells']}for n,r in result['rows'].items()if int(n)in SCOPES},flush=True)
 rays=[dict(screen=[(r['source_union_box'][0]+r['source_union_box'][2])/2,(r['source_union_box'][1]+r['source_union_box'][3])/2],hits=[])for r in qualification['rows']]
 reader.main(ray_records=rays,triangle_callback=inspect,postprocess=finish,output=DEST,asset_ids={x[0]for x in SCOPES.values()},query_margin=200,minimum_free_bytes=10*1024**3,output_limit_bytes=16*1024**2,expected_map_sha256=qualification['map']['sha256'])
if __name__=='__main__':
 acquire(slots=2)
 try:main()
 finally:release()
