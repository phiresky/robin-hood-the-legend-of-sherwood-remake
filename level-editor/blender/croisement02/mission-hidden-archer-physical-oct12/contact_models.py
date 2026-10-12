"""Intersect screen pixel volumes with alpha-aware installed receiver triangles."""
import os
os.environ['OPENBLAS_NUM_THREADS']='2'
import sys,json,math,shutil,collections
from pathlib import Path
import numpy as np
HERE=Path(__file__).resolve().parent;sys.path[:0]=[str(HERE.parent),str(HERE.parents[2]/'refinement')]
import qualify as q
import restart14_butterfly_canopy22_audit as reader
from restart14_butterfly07_prism_contacts import alpha_maximum
from restart14_butterfly07_anatomy_contacts import clip_planes
from render_slots import acquire,release
S,C=reader.SIN,reader.COS
DEST=q.OUT/'geometry-v1-contacts'
def main():
 assert not DEST.exists();assert shutil.disk_usage(q.ROOT).free>=10*1024**3+16*1024**2
 report=json.loads((q.OUT/'qualification.json').read_text());anchors=json.loads((q.OUT/'depth-proposal-v1.json').read_text());units=[]
 for n,row in enumerate(report['rows'],1):
  a=anchors[str(n)];xlo,xhi=row['source_union_box'][0],row['source_union_box'][2]
  def depth(x,y):
   u=(x-xlo)/(xhi-xlo);wy=a['world_y_left']*(1-u)+a['world_y_right']*u+a['depth_bulge']*math.sin(math.pi*u);z=(-y-wy*S)/C;return -C*wy+S*z
  for phase,f in [('initial',row['art']['states']['initial']['frames'][0]),('applied',row['art']['states']['transition']['frames'][-1])]:
   mask=np.array(q.rgba(f))[:,:,3]>0;pixels={(int(f['top_left'][0])+int(x),int(f['top_left'][1])+int(y))for y,x in zip(*np.where(mask))}
   if n==2:
    xx=[x for x,y in pixels if y==0]
    if xx:
     for y in range(-10,0):
      inset=int((-y-1)/2)
      for x in range(min(xx)+inset,max(xx)-inset+1):pixels.add((x,y))
   xy=np.array(list(pixels));dep=np.array([depth(x+dx,y+dy)for x,y in pixels for dx,dy in [(0,0),(1,0),(1,1),(0,1)]]);bounds=[float(xy[:,0].min()),float(xy[:,1].min()),float(dep.min()-a['thickness']-2),float(xy[:,0].max()+1),float(xy[:,1].max()+1),float(dep.max()+2)];units.append(dict(profile=n,phase=phase,pixels=pixels,depth=depth,anchor=dict(a),xlo=xlo,xhi=xhi,bounds=bounds,hits=[],counts=collections.Counter()))
 # Bind each closure's parameters explicitly after the construction loop.
 for unit in units:
  def depth(x,y,u=unit):
   a=u['anchor'];t=(x-u['xlo'])/(u['xhi']-u['xlo']);wy=a['world_y_left']*(1-t)+a['world_y_right']*t+a['depth_bulge']*math.sin(math.pi*t);return -wy/C-y*S/C
  unit['depth']=depth
 def inspect(placed,node,ni,pi,tri,uvs,mat,alpha,texture_record):
  # Camera coordinates are native x/y and distance toward the camera.
  xyz=np.stack([tri[:,:,0],S*tri[:,:,2]-C*tri[:,:,1],C*tri[:,:,2]+S*tri[:,:,1]],axis=2);uv=np.zeros((*tri.shape[:2],2))if uvs is None else uvs;attrs=np.concatenate([xyz,uv],axis=2);low=xyz.min(1);high=xyz.max(1);opaque=mat.get('alphaMode','OPAQUE')=='OPAQUE';image,sampler,factor=(None,{},1.)if opaque else texture_record(mat);cut=mat.get('alphaCutoff',.5)if mat.get('alphaMode')=='MASK'else .01
  for unit in units:
   bounds=unit['bounds'];selected=np.flatnonzero(np.all(high>=bounds[:3],axis=1)&np.all(low<=bounds[3:],axis=1))
   for ti in selected:
    xs=range(max(int(math.floor(low[ti,0])),int(bounds[0])),min(int(math.floor(high[ti,0]))+1,int(bounds[3])));ys=range(max(int(math.floor(low[ti,1])),int(bounds[1])),min(int(math.floor(high[ti,1]))+1,int(bounds[4])))
    for x in xs:
     for y in ys:
      if(x,y)not in unit['pixels']:continue
      d=unit['depth'](x,y);dx=unit['depth'](x+1,y)-d;dy=-S/C;constant=d-dx*x-dy*y;thick=unit['anchor']['thickness']
      planes=np.array([[-1,0,0,x],[1,0,0,-x-1],[0,-1,0,y],[0,1,0,-y-1],[-dx,-dy,1,-constant],[dx,dy,-1,constant-thick]])
      clipped=clip_planes(attrs[ti],planes)
      if not len(clipped):continue
      maximum,witness,cells=alpha_maximum(clipped,image,sampler,factor)
      if maximum+1e-8>=cut:
       unit['counts'][placed['id']]+=1
       if len(unit['hits'])<100:unit['hits'].append(dict(asset=placed['id'],node=node.get('name'),triangle=int(ti),source_cell=[x,y],alpha_maximum=maximum,camera_space_witness=witness[:3].tolist(),front_depth=d,back_depth=d-thick))
 def finish(rays,records,mp):
  DEST.mkdir();rows=[dict(profile=u['profile'],phase=u['phase'],triangle_cell_contacts=dict(u['counts']),witnesses=u['hits'])for u in units];(DEST/'report.json').write_text(json.dumps(dict(status='HOLD_CONTACTS_FOUND'if any(u['counts']for u in units)else 'PASS_ENDPOINT_VOLUMES_STATIC_RECEIVER_ALPHA',map=q.pin(mp),depth_proposal=q.pin(q.OUT/'depth-proposal-v1.json'),records=records,rows=rows,limits=['Endpoint volumes only; three native transition poses and continuous motion not tested.','Physical bilinear level0 alpha at clipped contact polygons; runtime mips can differ.','Contacts require ownership/support review; none are silently excused as foliage overlap.']),indent=2)+'\n')
 rays=[dict(screen=[(r['source_union_box'][0]+r['source_union_box'][2])/2,(r['source_union_box'][1]+r['source_union_box'][3])/2],hits=[])for r in report['rows']]
 reader.main(ray_records=rays,triangle_callback=inspect,postprocess=finish,output=DEST,query_margin=200,minimum_free_bytes=10*1024**3,output_limit_bytes=16*1024**2,expected_map_sha256=report['map']['sha256'])
if __name__=='__main__':
 acquire(slots=2)
 try:main()
 finally:release()
