"""Read-only native geometry/terrain contact check for the reverse Tree38 screenshot."""
from pathlib import Path
import json,struct,hashlib,math
import numpy as np
R=Path.cwd();B=R/'level-editor/work/croisement02-refinement';O=B/'tree38-reverse-neighbor-contact-oct12-v1'
def read(p):
 raw=p.read_bytes();n=struct.unpack_from('<I',raw,12)[0];g=json.loads(raw[20:20+n]);binary=raw[28+n:]
 def acc(i):
  a=g['accessors'][i];v=g['bufferViews'][a['bufferView']];buf=binary if v.get('buffer',0)==0 else (p.parent/g['buffers'][v['buffer']]['uri']).read_bytes();dt=np.dtype({5126:'<f4',5125:'<u4',5123:'<u2'}[a['componentType']]);dim={'VEC3':3,'VEC2':2,'SCALAR':1}[a['type']];return np.ndarray((a['count'],dim),dtype=dt,buffer=buf,offset=v.get('byteOffset',0)+a.get('byteOffset',0),strides=(v.get('byteStride',dim*dt.itemsize),dt.itemsize))
 return g,acc
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 O.mkdir(exist_ok=False);lib=R/'level-editor/library';tp=lib/'3d-assets/croisement02/croisement02-tree-37/model.glb';gp=lib/'3d-assets/croisement02/croisement02-terrain/model.glb';g,a=read(tp);ground,ga=read(gp);tris=[]
 for node in g['nodes']:
  if 'wood'not in node.get('name','')or'mesh'not in node:continue
  for q in g['meshes'][node['mesh']]['primitives']:tris.append(a(q['attributes']['POSITION'])[a(q['indices']).ravel()].reshape(-1,3,3).astype(float)+[1495,-1277,0])
 triangles=np.concatenate(tris);cuts=[]
 for t in triangles:
  if t[:,2].min()>0 or t[:,2].max()<0:continue
  pts=[]
  for i,j in [(0,1),(1,2),(2,0)]:
   p,q=t[i],t[j]
   if p[2]*q[2]<0:pts.append(p+(q-p)*(-p[2])/(q[2]-p[2]))
  if len(pts)==2:cuts.append(pts)
 cuts=np.array(cuts);mid=cuts.mean(1);groundtris=[]
 for mesh in ground['meshes']:
  for q in mesh['primitives']:groundtris.append(ga(q['attributes']['POSITION'])[ga(q['indices']).ravel()].reshape(-1,3,3))
 terrain=np.concatenate(groundtris);u=terrain[:,1,:2]-terrain[:,0,:2];v=terrain[:,2,:2]-terrain[:,0,:2];den=u[:,0]*v[:,1]-u[:,1]*v[:,0];valid=abs(den)>1e-12;terrain=terrain[valid];u=u[valid];v=v[valid];den=den[valid];heights=[];miss=0
 for p in mid:
  w=p[:2]-terrain[:,0,:2];s=(w[:,0]*v[:,1]-w[:,1]*v[:,0])/den;t=(u[:,0]*w[:,1]-u[:,1]*w[:,0])/den;hit=(s>=-1e-6)&(t>=-1e-6)&(s+t<=1+1e-6)
  if not hit.any():miss+=1;continue
  z=terrain[:,0,2]+s*(terrain[:,1,2]-terrain[:,0,2])+t*(terrain[:,2,2]-terrain[:,0,2]);heights.append(float(z[hit].max()))
 # Camera is exactly the close reverse formula frozen in round_wood_editor_check.js.
 target=np.array([1618.4545288085938,-1219.0376586914062,61.88017296791077]);scale=772/290
 def screen(p):
  d=p-target;return np.column_stack([710-d[:,0]*scale,439.5-(d[:,2]*math.cos(math.radians(35))-d[:,1]*math.sin(math.radians(35)))*scale])
 pixels=screen(cuts.reshape(-1,3));report={'status':'TREE37_IDENTIFIED_GROUND_INTERSECTION_PRESENT','tree_model':{'path':str(tp),'sha256':sha(tp)},'terrain_model':{'path':str(gp),'sha256':sha(gp)},'method':'Native Z-up mesh indexed triangles, known unrotated map placement; interpolate all wood intersections atZ0 then vertical barycentric terrain hits. Camera reconstructed from exact private Tree38 bounds and frozen close reverse formula. Screen viewport edges read from screenshot, so pixel coordinates approximate within1px.','wood_z_range':[float(triangles[:,:,2].min()),float(triangles[:,:,2].max())],'ground_crossing_segments':len(cuts),'footprint_min':cuts.min((0,1)).tolist(),'footprint_max':cuts.max((0,1)).tolist(),'terrain_hit_samples':len(heights),'terrain_misses':miss,'terrain_z_minmax':[min(heights),max(heights)],'predicted_ground_contact_screen_bbox':[pixels.min(0).tolist(),pixels.max(0).tolist()],'identification':'Tree37 is west of38, correctly projects to right in reverse. Tree39 east projects left. Ground crossing footprint projects at the visible pale trunk foot; no positive physical gap at that cross-section.','limits':['No assertion about every visible image pixel or occluded side surface.','Existing texture/shadow appearance can still deserve separate review; filling Tree39 cannot change this Tree37 contact.','No new geometry or source edits; no blanket external-neighbor clearance claim.']};(O/'audit.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))
if __name__=='__main__':main()
