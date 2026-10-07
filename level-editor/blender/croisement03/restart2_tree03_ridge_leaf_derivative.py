"""Private single native leaf depth correction; frozen source remains immutable."""
import sys,math,json,hashlib,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
R=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from restart2_tree02_firsthit_v1 import rows_for,sample
from restart2_tree02_joint_context_v4 import hit
from render_slots import acquire,release
from evidence_io import sha,write_json
B=R/'level-editor/work/croisement03-refinement/restart2';S=math.sin(math.radians(35));C=math.cos(math.radians(35));RAY=Vector((0,-C,S))
def digest(values):return hashlib.sha256(np.asarray(values).tobytes()).hexdigest()
def surfaces(objects):
 result={}
 for o in objects:
  m=o.data;result[o.name]=dict(vertices=digest([tuple(v.co) for v in m.vertices]),polygons=digest([list(p.vertices) for p in m.polygons]) if len(set(len(p.vertices) for p in m.polygons))==1 else hashlib.sha256(str([list(p.vertices) for p in m.polygons]).encode()).hexdigest(),uv={u.name:digest([tuple(v.uv) for v in u.data]) for u in m.uv_layers},matrix=digest(o.matrix_world),materials=[m.name for m in m.materials])
 return result
def main():
 save='--save' in sys.argv;version='v2';assert shutil.disk_usage(R).free>8*1024**3+32*1024**2;out=B/(f'tree03-ridge-leaf-derivative-{version}' if save else f'tree03-ridge-leaf-cpu-{version}');out.mkdir(exist_ok=False);acquire()
 try:
  paths=[B/'tree03-crown-prototype-v2/worker.blend',B/'tree02-isolated-prototype-v7/worker.blend',B/'tree04-crown-prototype-v1/worker.blend'];pins={str(p):sha(p) for p in paths};bpy.ops.wm.open_mainfile(filepath=str(paths[0]));scene=bpy.data.scenes['Tree13 isolated wood'];bpy.context.window.scene=scene;own=[o for o in scene.objects if o.type=='MESH'];original_objects=set(bpy.data.objects);neighbors=[];original_images={im.name:digest(im.pixels[:]) for ob in own for mat in ob.data.materials for node in mat.node_tree.nodes if node.type=='TEX_IMAGE' for im in [node.image]}
  for path,groups in [(paths[1],{'croisement03-tree-02','croisement03-arbre08-fragment-tree02-provisional'}),(paths[2],{'croisement03-tree-04','croisement03-arbre07-fragment-tree04-provisional'})]:
   with bpy.data.libraries.load(str(path),link=False) as (a,b):b.objects=list(a.objects)
   selected=[o for o in b.objects if o and o.type=='MESH' and o.get('asset_group') in groups]
   assert selected
   for o in selected:scene.collection.objects.link(o)
   neighbors+=selected
  bpy.context.view_layer.update();before_surfaces=surfaces(own+neighbors);rows=rows_for(own,False)+rows_for(neighbors,True);domains={}
  for n in (2,3,4):
   d=np.array(Image.open(B/f'tree{n:02}-bark-proposal-v1/proposed-bark.png'))>0
   if n in (2,3):
    p=B/('tree02-isolated-prototype-v7/native-leaves.png' if n==2 else 'tree03-canopy-fragment-source-v1/000.png');ar=np.array(Image.open(p).convert('RGBA'));x=175 if n==2 else 225;d[:ar.shape[0],x:x+ar.shape[1]]|=ar[:,:,3]>0
   domains[f'tree{n:02}']=d
  baseline={k:[(int(x),int(y),sample(rows,int(x),int(y))) for y,x in zip(*np.nonzero(d))] for k,d in domains.items()}
  leaf=next(o for o in own if o.name=='Arbre08 fragment provisional crown context');face=leaf.data.polygons[2787];ids=list(face.vertices);assert len(ids)==4;assert all(sum(v in p.vertices for p in leaf.data.polygons)==1 for v in ids);old=[leaf.matrix_world@leaf.data.vertices[v].co for v in ids];move=13.0
  for v in ids:leaf.data.vertices[v].co+=leaf.matrix_world.to_3x3().inverted()@(RAY*move)
  leaf.data.update();bpy.context.view_layer.update();new=[leaf.matrix_world@leaf.data.vertices[v].co for v in ids];project=lambda p:(p.x,-p.y*S-p.z*C);errors=[max(abs(a-b) for a,b in zip(project(p),project(q))) for p,q in zip(old,new)];assert max(errors)<.0001
  after_surfaces=surfaces(own+neighbors);changed=[k for k in before_surfaces if before_surfaces[k]!=after_surfaces[k]];assert changed==[leaf.name];assert {k:v for k,v in before_surfaces[leaf.name].items() if k!='vertices'}=={k:v for k,v in after_surfaces[leaf.name].items() if k!='vertices'}
  rows=rows_for(own,False)+rows_for(neighbors,True);checks={}
  for key,values in baseline.items():
   changes=[]
   for x,y,expected in values:
    got=sample(rows,x,y)
    if got!=expected:changes.append([x,y,expected,got])
   checks[key]=dict(samples=len(values),changes=changes)
  profile=json.loads((B/'tree02-ridge-ray-guard-v4/receipt.json').read_text())['ridge_profile'];vs=[];faces=[]
  for x,y,my in profile:vs.extend([(x,-my/S,0),(x,-my/S,85/C),(x,-350/S,85/C),(x,-350/S,0)])
  for i in range(len(profile)-1):
   for j in range(4):faces.append((i*4+j,(i+1)*4+j,(i+1)*4+(j+1)%4,i*4+(j+1)%4))
  faces.extend([(0,3,2,1),tuple(range(len(vs)-4,len(vs)))]);m=bpy.data.meshes.new('Shared source ridge CPU');m.from_pydata(vs,[],faces);m.update();m.calc_loop_triangles();ts=list(m.loop_triangles);o=bpy.data.objects.new('Shared source ridge CPU',m);ridge=(o,BVHTree.FromPolygons([Vector(p) for p in vs],[list(t.vertices) for t in ts],all_triangles=True),[Vector(p) for p in vs],ts,None,None,False,False)
  ridge_checks={}
  # Only Tree02/03 source domains intersect the diagnosed receiver.
  for key in ('tree02','tree03'):
   changes=[]
   for x,y,_ in baseline[key]:
    before=hit(rows,x,y);after=hit(rows+[ridge],x,y)
    if before!=after:changes.append([x,y,before,after])
   ridge_checks[key]=dict(samples=len(baseline[key]),changes=changes)
  # Surface translation preserves cell shape; nearest wood distance documents support context.
  wood=rows_for([o for o in own if 'fragment' not in o.get('asset_group','')],False)
  support=[]
  for tag,points in [('before',old),('after',new)]:
   center=sum(points,Vector())/4;nearest=min((r[1].find_nearest(center)[3],r[0].name) for r in wood);support.append(dict(stage=tag,center=list(center),nearest_wood=nearest[1],distance=nearest[0]))
  image_after={im.name:digest(im.pixels[:]) for ob in own for mat in ob.data.materials for node in mat.node_tree.nodes if node.type=='TEX_IMAGE' for im in [node.image]};assert original_images==image_after
  assert not any(v['changes'] for v in checks.values());assert not any(v['changes'] for v in ridge_checks.values())
  result=dict(status='PASS CPU source/ridge guards; independent geometry/joint review pending',source_hashes=pins,image_rgba_sha256=original_images,image_rgba_unchanged=original_images==image_after,polygon=2787,vertex_ids=ids,ray_translation=move,world_delta=list(RAY*move),old_world=[list(p) for p in old],new_world=[list(p) for p in new],max_projection_error=max(errors),changed_objects=changed,unchanged_geometry_uv_material_matrix_proof=before_surfaces,after_surfaces=after_surfaces,checks=checks,ridge_checks=ridge_checks,support_context=support,limits=['One disconnected four-vertex native leaf cell translates rigidly along original source rays; no other geometry changes.','UVs and material assignments unchanged. Frozen source file hashes retain original image resources.','Wood support remains the original inferred network; complete joint morphology must be inspected before user approval.','Shared ridge diagnostic is not whole terrain completion; full animation ordering remains separate.'])
  if save:
   for ob in list(bpy.data.objects):
    if ob not in original_objects:bpy.data.objects.remove(ob,do_unlink=True)
   assert [ob for ob in scene.objects if ob.type=='MESH']==own
   bpy.ops.wm.save_as_mainfile(filepath=str(out/'worker.blend'),compress=True);result['model_sha256']=sha(out/'worker.blend')
  assert all(sha(Path(p))==h for p,h in pins.items());write_json(out/'receipt.json',result);print(result['status'],checks.keys(),support)
 finally:release()
if __name__=='__main__':main()
