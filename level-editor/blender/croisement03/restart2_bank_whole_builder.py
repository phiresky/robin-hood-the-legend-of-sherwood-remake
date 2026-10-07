"""Closed whole-bank surfaces with source-constrained strata and fixed crest."""
import argparse,hashlib,json,math,sys
from collections import Counter
from pathlib import Path
import numpy as np
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement';P=B/'restart2/bank-whole-source-plan-v1/plan.json';O=B/'restart2/bank-whole-geometry-plan-v4'
S=math.sin(math.radians(35));C=math.cos(math.radians(35))

def inside(p,ring):
 x,y=p[:2];hit=False
 for a,b in zip(ring,ring[1:]+ring[:1]):
  if (a[1]>y)!=(b[1]>y) and x<(b[0]-a[0])*(y-a[1])/(b[1]-a[1])+a[0]:hit=not hit
 return hit

def mesh(boundary,lines):
 # Coordinates here are authored x,mapY,height, before Blender axis scaling.
 from scipy.spatial import Delaunay
 verts=[list(p) for p in boundary];edges=[(i,(i+1)%len(verts)) for i in range(len(verts))];outer=set(tuple(sorted(e)) for e in edges);traces=[]
 def add(p):
  for i,q in enumerate(verts):
   if math.dist(p[:2],q[:2])<1e-7:
    assert abs(p[2]-q[2])<1e-5,(p,q)
    return i
  verts.append(list(p));return len(verts)-1
 for name,line in lines:
  ids=[]
  for p in line:
   assert inside(p,boundary) or any(math.dist(p[:2],q[:2])<1e-7 for q in boundary),(name,'outside',p)
   ids.append(add(p))
  edges.extend(zip(ids,ids[1:]));traces.append(dict(id=name,vertices=ids))
 # Recover every boundary and traced crease as a triangulation edge by midpoint
 # splitting. Midpoints interpolate the constraint height, never source RGB.
 splits=0
 for iteration in range(18):
  tri=Delaunay(np.array(verts)[:,:2]);present={tuple(sorted((int(a),int(b)))) for t in tri.simplices for a,b in zip(t,np.roll(t,-1))};missing=[e for e in edges if tuple(sorted(e)) not in present]
  if not missing:break
  new=[]
  for a,b in edges:
   key=tuple(sorted((a,b)))
   if key in present:new.append((a,b));continue
   mid=add([(verts[a][j]+verts[b][j])/2 for j in range(3)]);assert mid not in (a,b)
   new.extend([(a,mid),(mid,b)]);splits+=1
   if key in outer:outer.remove(key);outer.update([tuple(sorted((a,mid))),tuple(sorted((mid,b)))])
  edges=new
 else:raise AssertionError('Constraint recovery did not converge')
 faces=[]
 for t in tri.simplices:
  if inside(np.mean(np.array(verts)[t,:2],axis=0),boundary):
   a,b,c=map(int,t);pa,pb,pc=[verts[i] for i in [a,b,c]]
   if (pb[0]-pa[0])*(pc[1]-pa[1])-(pb[1]-pa[1])*(pc[0]-pa[0])<0:b,c=c,b
   faces.append([a,b,c])
 # A heightfield cannot self-intersect internally. Boundary walls and the flat
 # bottom close its volume; explicit edge counts certify consistent joins.
 edgecounts=Counter(tuple(sorted((a,b))) for f in faces for a,b in zip(f,f[1:]+f[:1]));actualboundary={e for e,n in edgecounts.items() if n==1};assert actualboundary==outer,(len(actualboundary),len(outer))
 n=len(verts);verts+= [[x,y,0.] for x,y,z in verts];faces += [[i+n for i in f[::-1]] for f in faces.copy()]
 for a,b in edges:
  if tuple(sorted((a,b))) in outer:faces.extend([[a,a+n,b+n],[a,b+n,b]])
 # Authored y goes negative in world, so reverse to retain outward orientation.
 world=[[x,-y/S,z/C] for x,y,z in verts];faces=[f[::-1] for f in faces]
 ec=Counter(tuple(sorted((a,b))) for f in faces for a,b in zip(f,f[1:]+f[:1]));assert set(ec.values())=={2}
 volume=0.;areas=[]
 for f in faces:
  a,b,c=np.array([world[i] for i in f]);normal=np.cross(b-a,c-a);areas.append(float(np.linalg.norm(normal)/2));volume+=float(np.dot(a,normal)/6)
 assert volume>0 and min(areas)>1e-8
 return dict(vertices=world,faces=faces,traces=traces,boundary_top_indices=sorted({v for e in outer for v in e}),constraint_splits=splits,source_vertex_count=n,signed_volume=volume,min_area=min(areas),edge_count=len(ec),euler=len(world)-len(ec)+len(faces),topology='Closed manifold heightfield and bottom; no internal surface intersections by construction')

def extend_stripes(boundary,lines):
 # Continue observed front breaks around inferred side shoulders, sharing the
 # exact endpoint with the perimeter instead of making isolated surface dents.
 for name,line in lines:
  for side in (0,-1):
   p=line[side];hits=[]
   for i,(a,b) in enumerate(zip(boundary,boundary[1:]+boundary[:1])):
    if min(a[1],b[1])<p[1]<max(a[1],b[1]):
     t=(p[1]-a[1])/(b[1]-a[1]);x=a[0]+t*(b[0]-a[0]);hits.append((x,i))
   x,index=min(hits) if side==0 else max(hits)
   q=[x,p[1],p[2]];boundary.insert(index+1,q)
   if side==0:line.insert(0,q)
   else:line.append(q)
 return boundary,lines

def prepare():
 O.mkdir(exist_ok=True);assert not (O/'geometry.json').exists();plan=json.loads(P.read_text());level=json.loads((B/'baseline/Croisement03.rhp.json').read_text());profile=json.loads((B/'restart2/tree02-ridge-ray-guard-v4/receipt.json').read_text())['ridge_profile'];raw=level['sight_obstacles'][52]['points'];points=[raw[0]]+[dict(x=x,y=my,z_top=85) for x,sy,my in profile]+raw[3:]
 boundary52=[[p['x'],p['y'],p['z_top']] for p in points]
 # Front heights are inferred under vegetation; the source-supported rear crest
 # and the 52/54 shared upper contact remain exact.
 heights={7:20.,8:30.,9:40.,12:38.,13:43.,14:60.,15:55.}
 for index,height in heights.items():boundary52[len(profile)+index-2][2]=height
 boundary52.insert(len(profile)+5,[615.,312.,25.])
 lines52=[];old=plan['previous_main_breaks'];upper=old[0]['points'];lower=old[1]['points'];lx=[p[0] for p in lower];ly=[p[1] for p in lower]
 a=[[x,y+85,85] for x,y in upper];b=[]
 for x in sorted(set(lx+[p[0] for p in upper if lx[0]<p[0]<lx[-1]])):
  y=float(np.interp(x,lx,ly))
  # A shallow five-unit band below the fixed85 top, with the source
  # screen slope carried by depth instead of an excessively deep vertical drop.
  my=y+80.
  b.append([x,my,my-y])
 lines52.extend([('west-upper',a),('west-lower',b)])
 lines54=[]
 for t in plan['new_traces']:
  lines=lines52 if t['owner']==52 else lines54;up=[];low=[]
  for pair in t['paired_world']:
   x,y,z=pair['upper_world'];height=z*C
   if t['owner']==52:height={'east-upper-block':75.,'east-middle-block':65.,'east-low-block':45.}[t['id']]
   sy=pair['source_upper'][1];lower_y=pair['source_lower'][1];my=sy+height
   up.append([x,my,height]);low.append([x,my+1.5,my+1.5-lower_y])
  lines.extend([(t['id']+'-upper',up),(t['id']+'-lower',low)])
 raw54=level['sight_obstacles'][54]['points'];boundary54=[[p['x'],p['y'],p['z_top']] for p in raw54]
 # Align visual contact to exact bank52 endpoints, retaining gameplay separately.
 boundary54[0]=[raw[11]['x'],raw[11]['y'],raw[11]['z_top']];boundary54[1]=[raw[10]['x'],raw[10]['y'],raw[10]['z_top']]
 boundary54[2][0]+=2.;boundary54[2][2]=34.
 boundary54,lines54=extend_stripes(boundary54,lines54)
 out={'52':mesh(boundary52,lines52),'54':mesh(boundary54,lines54)}
 for id,m in out.items():
  m['native_obstacle']=int(id);m['gameplay_metadata']=level['sight_obstacles'][int(id)]
 fixed=[]
 for x,sy,my in profile:
  target=[x,-my/S,85/C];error=min(max(abs(a-b) for a,b in zip(v,target)) for v in out['52']['vertices']);fixed.append(error)
 assert max(fixed)<1e-9
 out['guards']=dict(fixed_crest_points=len(profile),max_crest_world_error=max(fixed),plan_sha256=hashlib.sha256(P.read_bytes()).hexdigest(),baseline53_sha256=hashlib.sha256((B/'restart2/bank-continuous-strata-v1/worker.blend').read_bytes()).hexdigest(),interface52_54=[boundary54[0],boundary54[1]],
  interface52_53='At runtime subtract retained53 solid from52 with exact solver, keeping53 mesh/transform/materials unchanged. Check resulting52 closed topology and shared contact; no source/gap approval implied.',
  source='Original3446 seeds only until new source ownership reviewed; new proposals remain reserved.',
  required=['Saved topology and source firsthits after interface Boolean','Zero lost native seeds and zero tested neighbor ray changes','Native plus actual8/solid8 west/east/south contact inspection','53 geometry exactly retained and fixed crest unchanged'])
 (O/'geometry.json').write_text(json.dumps(out,indent=2)+'\n');print({k:{j:v[j] for j in ['source_vertex_count','constraint_splits','signed_volume','min_area','euler']} for k,v in out.items() if k!='guards'})
def run():
 import bpy,bmesh
 sys.path.insert(0,str(Path(__file__).parent));import restart2_bank_full_v1 as base
 proof=json.loads((O/'cpu-crease-firsthits.json').read_text());assert proof['blocked']==0 and proof['misses']==0 and proof['geometry_sha256']==hashlib.sha256((O/'geometry.json').read_bytes()).hexdigest()
 data=json.loads((O/'geometry.json').read_text());west=json.loads((B/'restart2/bank-continuous-strata-plan-v1/geometry.json').read_text());data['53']=west
 assert data['guards']['plan_sha256']==hashlib.sha256(P.read_bytes()).hexdigest()
 baseline=B/'restart2/bank-continuous-strata-v1/worker.blend'
 assert data['guards']['baseline53_sha256']==hashlib.sha256(baseline.read_bytes()).hexdigest()
 base.O=B/'restart2/bank-whole-prototype-v2';interface={'geometry_plan':str(O/'geometry.json'),'geometry_plan_sha256':hashlib.sha256((O/'geometry.json').read_bytes()).hexdigest()}
 def signature(obj):return ([list(v.co) for v in obj.data.vertices],sorted(tuple(sorted(f.vertices)) for f in obj.data.polygons))
 def build(name,points,scene):
  index=name.rsplit(' ',1)[1];d=data[index];mesh=bpy.data.meshes.new(name+' continuous surface');mesh.from_pydata(d['vertices'],[],d['faces']);mesh.update();obj=bpy.data.objects.new(name,mesh);scene.collection.objects.link(obj)
  bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges) and all(f.calc_area()>1e-8 for f in bm.faces);assert bm.calc_volume()>0;bm.to_mesh(mesh);bm.free()
  if index=='53':
   with bpy.data.libraries.load(str(baseline),link=False) as (a,b):b.objects=['Candidate bank 53']
   reference=b.objects[0];assert signature(reference)==signature(obj),'Retained53 geometry differs';bpy.data.objects.remove(reference,do_unlink=True)
  if index=='54':
   owner=bpy.data.objects['Candidate bank 52'];cutter=bpy.data.objects['Candidate bank 53'];before=signature(cutter)
   bpy.context.view_layer.objects.active=owner;mod=owner.modifiers.new('Shared western solid interface','BOOLEAN');mod.operation='DIFFERENCE';mod.solver='EXACT';mod.object=cutter;bpy.ops.object.modifier_apply(modifier=mod.name)
   assert signature(cutter)==before;unknown=owner.data.materials[0];owner.data.materials.clear();owner.data.materials.append(unknown)
   for face in owner.data.polygons:face.material_index=0
   bm=bmesh.new();bm.from_mesh(owner.data);assert all(e.is_manifold for e in bm.edges) and all(f.calc_area()>1e-8 for f in bm.faces);assert bm.calc_volume()>0;bm.free()
   # The cutter is far from the fixed rear crest; every original crest point
   # must survive within the saved float-coordinate precision.
   profile=json.loads((B/'restart2/tree02-ridge-ray-guard-v4/receipt.json').read_text())['ridge_profile'];errors=[]
   for x,sy,my in profile:
    p=(x,-my/S,85/C);errors.append(min(max(abs(a-b) for a,b in zip(v.co,p)) for v in owner.data.vertices))
   assert max(errors)<1e-4
   from mathutils import Vector
   from mathutils.bvhtree import BVHTree
   ray=Vector((0,-C,S));trees={}
   for key in ('52','53','54'):
    m=bpy.data.objects['Candidate bank '+key].data;m.calc_loop_triangles();trees[key]=BVHTree.FromPolygons([v.co for v in m.vertices],[list(t.vertices) for t in m.loop_triangles],all_triangles=True)
   failures=[];sample_count=0;max_surface_error=0.
   for key in ('52','54'):
    for trace in data[key]['traces']:
     pts=[Vector(data[key]['vertices'][i]) for i in trace['vertices']]
     for a,b in zip(pts,pts[1:]):
      distance=math.hypot(b.x-a.x,(-b.y*S-b.z*C)-(-a.y*S-a.z*C));count=max(2,int(distance*2)+1)
      for i in range(count):
       point=a.lerp(b,i/(count-1));sample_count+=1;surface_error=trees[key].find_nearest(point)[3];max_surface_error=max(max_surface_error,surface_error);hits=[]
       for other,tree in trees.items():
        hit,normal,face,dist=tree.ray_cast(point+ray*5000,-ray)
        if hit is not None:hits.append((dist,other))
       nearest=min(hits) if hits else None
       if surface_error>1e-4 or nearest is None or nearest[0]<4999.99:failures.append(dict(owner=key,trace=trace['id'],source=[point.x,-point.y*S-point.z*C],surface_error=surface_error,firsthit=nearest))
   interface['full_crease_guard']=dict(samples=sample_count,failures=failures,max_surface_error=max_surface_error)
   (base.O/'construction-crease-guard.json').write_text(json.dumps(interface,indent=2)+'\n')
   assert not failures,'Post-Boolean full-crease guard failed; no saved model or render'
   interface.update(dict(retained53_exact=True,bank52_after_difference_vertices=len(owner.data.vertices),bank52_after_difference_faces=len(owner.data.polygons),fixed_crest_max_world_error=max(errors),status='PASS construction checks; saved native and oblique review pending'))
  return obj
 base.mesh_object=build;base.main();(base.O/'interface-construction.json').write_text(json.dumps(interface,indent=2)+'\n')
 assert sum(p.stat().st_size for p in base.O.rglob('*') if p.is_file())<32*1024**2

if __name__=='__main__':
 args=sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else sys.argv[1:]
 parser=argparse.ArgumentParser();parser.add_argument('--run',action='store_true');options=parser.parse_args(args)
 run() if options.run else prepare()
