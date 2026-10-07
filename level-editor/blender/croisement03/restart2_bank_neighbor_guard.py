"""Read-only source-hit guard for the full bank against its reviewed tree neighbors."""
import sys,math,json
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
R=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from restart2_tree02_firsthit_v1 import rows_for
from restart2_tree02_shared_ridge_joint_v5 import hit
from render_slots import acquire,release
from evidence_io import sha,write_json
B=R/'level-editor/work/croisement03-refinement/restart2';O=Path(sys.argv[sys.argv.index('--')+1]).resolve() if '--' in sys.argv else B/'bank-full-prototype-v2'

def main():
 assert not (O/'neighbor-firsthits.json').exists();acquire()
 try:
  model=O/'worker.blend';digest=sha(model);bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;bank=[]
  for o in scene.objects:
   if o.type!='MESH' or not o.name.startswith('Candidate bank'):continue
   m=o.data;m.calc_loop_triangles();vs=[o.matrix_world@v.co for v in m.vertices];ts=list(m.loop_triangles);bank.append((o,BVHTree.FromPolygons(vs,[list(t.vertices) for t in ts],all_triangles=True),vs,ts,None,None,False,False))
  checks={};pins={str(model):digest};paths={2:B/'tree02-isolated-prototype-v7/worker.blend',3:B/'tree03-ridge-leaf-derivative-v2/worker.blend'}
  for n in range(4,8):
   manifests=list(B.glob(f'geometry-round*-tree{n:02}*/review-candidates.json'));assert len(manifests)==1;paths[n]=Path(json.loads(manifests[0].read_text())['items'][0]['model'])
  for n,path in paths.items():
   pins[str(path)]=sha(path)
   with bpy.data.libraries.load(str(path),link=False) as (a,b):
    name='Tree02 isolated' if n==2 else 'Tree13 isolated wood';assert name in a.scenes;b.scenes=[name]
   tree_scene=b.scenes[0];bpy.context.window.scene=tree_scene;bpy.context.view_layer.update();objects=[o for o in tree_scene.objects if o.type=='MESH'];rows=rows_for(objects,True)
   domain=np.array(Image.open(B/f'tree{n:02}-bark-proposal-v1/proposed-bark.png'))>0
   if n in (2,3):
    leafpath=B/('tree02-isolated-prototype-v7/native-leaves.png' if n==2 else 'tree03-canopy-fragment-source-v1/000.png');leaf=np.array(Image.open(leafpath));start=175 if n==2 else 225;domain[:leaf.shape[0],start:start+leaf.shape[1]]|=leaf[:,:,3]>0
   changes=[];holes=0
   for y,x in zip(*np.nonzero(domain)):
    before=hit(rows,int(x),int(y));after=hit(rows+bank,int(x),int(y))
    if before is None:holes+=1;continue
    if before!=after:changes.append([int(x),int(y),before,after])
   checks[f'tree{n:02}']=dict(samples=int(domain.sum()),baseline_holes=holes,changes=changes,scope='Bark plus native leaves' if n<4 else 'Accepted bark; native canopy ends above bank skyline')
   print(n,len(changes),flush=True)
  assert all(sha(Path(p))==h for p,h in pins.items());write_json(O/'neighbor-firsthits.json',dict(status='PASS source hits preserved' if all(not r['changes'] for r in checks.values()) else 'HOLD new bank blocks accepted tree source',source_hashes=pins,checks=checks,limits=['Trees loaded read-only and never saved.','Tree01 coarse and full canopy04–07 integration remain separate checks.','Identical object/face/distance proves this bank introduces no new block for the tested rays; no runtime animation claim.']))
 finally:release()
if __name__=='__main__':main()
