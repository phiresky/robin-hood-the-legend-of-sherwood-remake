"""Read-only saved tree ray audit against a compact native-mask ridge section."""
import sys,math,json,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
R=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from restart2_tree02_firsthit_v1 import rows_for
from restart2_tree02_joint_context_v4 import hit
from render_slots import acquire,release
from evidence_io import sha,write_json
B=R/'level-editor/work/croisement03-refinement/restart2';S=math.sin(math.radians(35));C=math.cos(math.radians(35))
def main():
 version=sys.argv[sys.argv.index('--')+1] if '--' in sys.argv else 'v4';assert version in ('v2','v3','v4');right=260 if version=='v3' else 340
 assert shutil.disk_usage(R).free>10*1024**3+2*1024**2;out=B/f'tree02-ridge-ray-guard-{version}';out.mkdir(exist_ok=False);acquire()
 try:
  model=B/'tree02-isolated-prototype-v7/worker.blend';neighbor=B/'tree03-crown-prototype-v2/worker.blend';pins={str(p):sha(p) for p in (model,neighbor)};bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.data.scenes['Tree02 isolated'];bpy.context.window.scene=scene;own=[o for o in scene.objects if o.type=='MESH']
  with bpy.data.libraries.load(str(neighbor),link=False) as (a,b):b.objects=[n for n in a.objects if n.startswith('Tree03 private stem') or n.startswith('Arbre08 fragment provisional') or n.startswith('Inferred cluster support')]
  other=[o for o in b.objects if o and o.type=='MESH']
  for o in other:scene.collection.objects.link(o)
  bpy.context.view_layer.update();rows=rows_for(own,False)+rows_for(other,True)
  level=json.loads((B.parent/'baseline/Croisement03.rhp.json').read_text());mask=np.zeros((960,1408),bool);im=np.array(Image.open(B.parent/'baseline/masks/000096.png'))>0;x,y=level['masks'][96]['box_top_left'];mask[y:y+im.shape[0],x:x+im.shape[1]]=im
  # Native silhouette samples fix only the rear lip; top height remains authored85.
  vs=[];faces=[];profile=[];protected_bark=np.array(Image.open(B/'tree03-bark-proposal-v1/proposed-bark.png'))>0
  for x in range(175,right+1):
   sy=int(np.flatnonzero(mask[:,x])[0]);known=np.flatnonzero(protected_bark[:,max(0,x-1):x+2].any(axis=1));sy=max(sy,int(known.max())+1 if version=='v4' and len(known) else sy);my=sy+85;profile.append([x,sy,my]);vs.extend([(x,-my/S,0),(x,-my/S,85/C),(x,-350/S,85/C),(x,-350/S,0)])
  for i in range(len(profile)-1):
   for j in range(4):faces.append((i*4+j,(i+1)*4+j,(i+1)*4+(j+1)%4,i*4+(j+1)%4))
  faces.extend([(0,3,2,1),tuple(range(len(vs)-4,len(vs)))]);mesh=bpy.data.meshes.new('CPU ridge section');mesh.from_pydata(vs,[],faces);mesh.update();obj=bpy.data.objects.new('Native mask96 lip and authored obstacle52 height diagnostic',mesh);mesh.calc_loop_triangles();triangles=list(mesh.loop_triangles);bv=BVHTree.FromPolygons([Vector(p) for p in vs],[list(t.vertices) for t in triangles],all_triangles=True);ridge=(obj,bv,[Vector(p) for p in vs],triangles,None,None,False,False)
  domains={}
  for n in (2,3):
   d=np.array(Image.open(B/f'tree{n:02}-bark-proposal-v1/proposed-bark.png'))>0
   leaf=Image.open(B/('tree02-isolated-prototype-v7/native-leaves.png' if n==2 else 'tree03-canopy-fragment-source-v1/000.png')).convert('RGBA');ar=np.array(leaf);start=175 if n==2 else 225;d[:ar.shape[0],start:start+ar.shape[1]]|=ar[:,:,3]>0;domains[f'tree{n:02}']=d
  checks={}
  for name,domain in domains.items():
   changes=[]
   for y,x in zip(*np.nonzero(domain)):
    before=hit(rows,int(x),int(y));after=hit(rows+[ridge],int(x),int(y))
    if before!=after:changes.append([int(x),int(y),before,after])
   checks[name]=dict(samples=int(domain.sum()),changes=changes)
  write_json(out/'receipt.json',dict(status='Diagnostic only; source conflicts must be resolved before receiver construction',source_hashes=pins,ridge_profile=profile,section_bounds=[175,right,350],version=version,static_bark_crest_constraint=version=='v4',authored_height=85,checks=checks,lower_trunk_probes=[dict(x=211,y=y,before=hit(rows,211,y),after=hit(rows+[ridge],211,y)) for y in (166,167,180,200,220,237)],limits=['No model or images saved or altered. Lip follows first occupied mask96 row but mask coverage alone is not ownership authority.','Section is finite context geometry; side/front cuts are diagnostic crop boundaries, not finished terrain.']))
  assert all(sha(Path(p))==h for p,h in pins.items());print({k:len(v['changes']) for k,v in checks.items()})
 finally:release()
if __name__=='__main__':main()
