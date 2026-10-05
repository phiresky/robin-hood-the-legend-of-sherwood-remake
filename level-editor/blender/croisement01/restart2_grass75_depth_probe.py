"""Measure final generic blade assignments against the saved branch before editing."""
import json,math,sys
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_slots import acquire
from evidence_io import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2'
W=R/'grass75-volume-v13/assets/croisement01-grass-75'
def main():
 acquire();bpy.ops.wm.open_mainfile(filepath=str(W/'model.blend'))
 c=bpy.data.collections['Croisement01 Working'];grass=next(o for o in c.all_objects if o.type=='MESH' and o.get('asset_group')=='croisement01-grass-75');wood=next(o for o in c.all_objects if o.type=='MESH' and o.get('asset_group')=='croisement01-east-fallen-branch')
 tree=BVHTree.FromPolygons([wood.matrix_world@v.co for v in wood.data.vertices],[tuple(p.vertices) for p in wood.data.polygons]);sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35));ray=Vector((0,-cosine,sine))
 source=W.parent.parent/'source';rgba=np.asarray(Image.open(source/'native.png').convert('RGBA'));yy,xx=np.nonzero(rgba[:,:,3]>127);guide=json.loads((R/'grass75-source-v1/grass-075-leaf-paths-v3/paths.json').read_text());x0,y0,w,h=guide['native_bbox'];ground=json.loads((source/'generic-construction.json').read_text())['ground_z'];width=float(xx.max()-xx.min()+1);height=float(yy.max()-yy.min()+1);root=np.array([x0+(xx.min()+xx.max()+1)/2,-(y0+yy.max()+1-height*.18+ground*cosine)/sine,ground+.05]);rng=np.random.default_rng(80175);skeletons=[]
 for b in range(85):
  angle=math.tau*(b/85)+rng.uniform(-.15,.15);outward=np.array([math.cos(angle),math.sin(angle),0]);length=width*rng.uniform(.25,.53);rise=height*rng.uniform(.3,.85);base=root+outward*rng.uniform(0,width*.07);skeletons.append([base+outward*(length*t**1.35)+np.array([0,0,rise*math.sin(t*math.pi*.72)]) for t in np.linspace(0,1,7)])
 samples=np.asarray(skeletons).reshape(-1,3);projected=np.column_stack([samples[:,0],-samples[:,1]*sine-samples[:,2]*cosine]);rows=[];offsets=[0.]*85;root_conflicts=[]
 for n,(y,x) in enumerate(zip(yy,xx)):
  px,py=x0+x+.5,y0+y+.5;nearest=int(np.argmin(np.sum((projected-[px,py])**2,axis=1)));blade,step=divmod(nearest,7);center=sum((grass.matrix_world@grass.data.vertices[n*18+i].co for i in range(3)),Vector())/3
  # The six front triangle vertices average to the pixel center despite their shared diagonal.
  front_indices=[0,1,2,9,10,11];center=sum((grass.matrix_world@grass.data.vertices[n*18+i].co for i in front_indices),Vector())/6
  hit=tree.ray_cast(Vector((px,-py/sine,0))+ray*5000,-ray,10000)[0];required=max(0.,hit.z+.75-center.z) if hit else 0.;phase=step/6;weight=math.sin(phase*math.pi/2)
  if required>0 and weight<1e-8:root_conflicts.append(dict(pixel=[int(x),int(y)],blade=blade,required=required))
  elif required>0:offsets[blade]=max(offsets[blade],required/weight)
  rows.append(dict(pixel=[int(x),int(y)],blade=blade,step=step,z=center.z,required=required))
 out=R/'grass75-final-blade-depth-probe-v1.json';out.write_text(json.dumps(dict(status='measurement only',model_sha256=sha(W/'model.blend'),vertices=len(grass.data.vertices),expected_vertices=len(xx)*18+85*72,pixels=len(xx),root_world=root.tolist(),root_conflicts=root_conflicts,offsets=offsets,maximum_offset=max(offsets),conflicting_pixels=sum(row['required']>0 for row in rows),assignments=rows),indent=2)+'\n');print(json.dumps(dict(path=str(out),conflicts=sum(row['required']>0 for row in rows),root_conflicts=len(root_conflicts),maximum_offset=max(offsets))))
if __name__=='__main__':main()
