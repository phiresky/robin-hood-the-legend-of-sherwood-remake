"""Build private source-preserving receiver caps for independent hole reveals."""
import sys,json,math,hashlib
from pathlib import Path
import bpy,numpy as np
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE));sys.path.insert(0,str(HERE.parents[1]/'refinement'))
from render_slots import acquire,release
from restart9_hole_contact import clip_halfplane
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35))
ROOT=HERE.parents[1]/'work/croisement02-refinement'
DEST=ROOT/'restart10-hole-aperture/packet-v1'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def area(poly):
 return abs(sum(a[0]*b[1]-b[0]*a[1] for a,b in zip(poly,poly[1:]+poly[:1])))*.5 if len(poly)>2 else 0.
def cut(poly,ellipse):
 current=poly;outside=[]
 for a,b in zip(ellipse,ellipse[1:]+ellipse[:1]):
  out=clip_halfplane(current,a,b,False)
  if area(out)>1e-8:outside.append(out)
  current=clip_halfplane(current,a,b)
  if area(current)<1e-8:return outside,[]
 return outside,current

def main():
 DEST.mkdir(parents=True,exist_ok=False)
 authority=ROOT/'restart9-hole-endpoints/receiver-audit-v2/report.json';audit=json.loads(authority.read_text())
 holes=[]
 for i,r in enumerate(audit['positions']):
  x,y=r['display_position'];ellipse=[[x+49+17*math.cos(t*math.tau/96),y+56+12*math.sin(t*math.tau/96)]for t in range(96)]
  holes.append(dict(id=f'hole-{i:02}',position=[x,y],triggers=r['instances'],owner=r['center']['owner'],support_z=r['center']['hit'][2] if r['center']['hit'][2]>20 else 0,ellipse=ellipse))
 for i,a in enumerate(holes):
  for b in holes[i+1:]:
   dx=(a['position'][0]-b['position'][0])/17;dy=(a['position'][1]-b['position'][1])/12
   assert dx*dx+dy*dy>4,'Overlapping aperture ellipses require joint partitioning'
 meshes=[];proof=[];cap_area={h['id']:0. for h in holes};max_area_error=0.;unchanged=changed=0
 for parent in audit['models']:
  if not any(r['name'] in {h['owner']for h in holes} for r in parent['objects']):continue
  assert sha(parent['path'])==parent['sha256']
  bpy.ops.wm.open_mainfile(filepath=parent['path']);bpy.context.view_layer.update()
  for record in parent['objects']:
   ob=bpy.data.objects[record['name']];assert np.max(np.abs(np.array(ob.matrix_world)-record['matrix']))<1e-7
   mesh=ob.data;mesh.calc_loop_triangles();groups={'outside':[]};source=[]
   candidates=[h for h in holes if h['owner']==ob.name]
   for ti,tri in enumerate(mesh.loop_triangles):
    xyz=np.array([list(ob.matrix_world@mesh.vertices[v].co)for v in tri.vertices]);screen=np.array([[p[0],-p[1]*SIN-p[2]*COS]for p in xyz]);poly=screen.tolist()
    uvs={l.name:[list(l.data[j].uv)for j in tri.loops]for l in mesh.uv_layers}
    original=dict(xyz=xyz.tolist(),uv=uvs,material=tri.material_index,source_triangle=ti)
    source.append(original)
    hits=[]
    for h in candidates:
     if max(abs(float(v[2])-h['support_z'])for v in xyz)>.02:continue
     e=np.array(h['ellipse'])
     if np.all(screen.max(0)>=e.min(0)) and np.all(screen.min(0)<=e.max(0)):
      outside,inside=cut(poly,h['ellipse'])
      if inside:hits.append(h)
    if not hits:
     groups['outside'].append(original);unchanged+=1;continue
    changed+=1;matrix=np.vstack([screen.T,np.ones(3)]);assert abs(np.linalg.det(matrix))>1e-8;inv=np.linalg.inv(matrix)
    pieces=[('outside',poly)]
    for h in hits:
     updated=[]
     for tag,p in pieces:
      if tag!='outside':updated.append((tag,p));continue
      outside,inside=cut(p,h['ellipse']);updated.extend(('outside',q)for q in outside)
      if inside:updated.append((h['id'],inside))
     pieces=updated
    total=sum(area(p)for _,p in pieces);err=abs(total-area(poly));max_area_error=max(max_area_error,err);assert err<max(1e-5,area(poly)*1e-10),(ob.name,ti,err)
    for tag,p in pieces:
     if tag!='outside':cap_area[tag]+=area(p)
     for j in range(1,len(p)-1):
      q=[p[0],p[j],p[j+1]]
      if area(q)<1e-8:continue
      weights=np.array([inv@np.array([*v,1.])for v in q]);assert weights.min()>-1e-7
      row=dict(xyz=(weights@xyz).tolist(),uv={k:(weights@np.array(v)).tolist()for k,v in uvs.items()},material=tri.material_index,source_triangle=ti,weights=weights.tolist())
      groups.setdefault(tag,[]).append(row)
   materials=[]
   for mat in mesh.materials:
    images=[]
    if mat and mat.use_nodes:
     for node in mat.node_tree.nodes:
      if node.type=='TEX_IMAGE' and node.image:
       im=node.image;images.append(dict(name=im.name,packed_sha256=hashlib.sha256(im.packed_file.data).hexdigest()if im.packed_file else None))
    materials.append(dict(name=mat.name if mat else None,images=images))
   meshes.append(dict(name=ob.name,parent=parent['sha256'],matrix=record['matrix'],materials=materials,source=source,groups=groups))
  assert sha(parent['path'])==parent['sha256'];proof.append(parent)
 endpoints={}
 for phase,rel,digest in [('initial','candidate-v2/initial','24c2c300bf9e940225676c2dbc4f5529865af1f6564bce6b0f1aa77ed299db4f'),('applied','candidate-v3/applied','b6528505a80f19e0b2aef610cfa4915603a001361a291e636ed9bf3b7c2e6f5d')]:
  f=ROOT/'restart9-hole-endpoints'/rel/'model.blend';assert sha(f)==digest;bpy.ops.wm.open_mainfile(filepath=str(f));bpy.context.view_layer.update();triangles=[]
  for ob in bpy.context.scene.objects:
   if ob.type!='MESH':continue
   ob.data.calc_loop_triangles()
   for tri in ob.data.loop_triangles:triangles.append([list(ob.matrix_world@ob.data.vertices[v].co)for v in tri.vertices])
  endpoints[phase]=dict(path=str(f),sha256=digest,triangles=triangles)
 expected=area(holes[0]['ellipse'])
 for key,value in cap_area.items():assert abs(value-expected)<.02,(key,value,expected)
 packet=dict(authority=dict(path=str(authority),sha256=sha(authority)),holes=holes,meshes=meshes,endpoints=endpoints,parents=proof,guards=dict(unchanged_triangles=unchanged,split_triangles=changed,max_projected_area_error=max_area_error,cap_projected_areas=cap_area,expected_cap_area=expected),scope='Private geometry representation only; no approved terrain, catalog, live state contract or model changed')
 f=DEST/'packet.json';f.write_text(json.dumps(packet,separators=(',',':'))+'\n');print('PACKET',f,sha(f),packet['guards'],flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
