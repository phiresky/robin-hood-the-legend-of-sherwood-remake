"""Read-only local source/geometry evidence for two bounded prop omissions."""
import json,sys
from pathlib import Path
import bpy
from PIL import Image,ImageDraw
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY
from refinement_review import _tree
from refinement_workspace import _geometry

def main():
 out=OUT/'restart4-source-gaps/diagnostic-v1';out.mkdir(parents=True,exist_ok=False)
 specs=[('croisement02-logging-clearing-stumps',OUT/'texture-fill-round-1/croisement02-logging-clearing-stumps/experiment/bake-v1/worker.blend','b73e09d157db4104ec3a9a2c847e86cfde5fd689d1fdc55b91b24ded25448f0c',[1280,275,1400,370],[13]),('croisement02-woodcutters-shed',OUT/'restart2-textures/approved-prop-repairs-fill-v1/croisement02-woodcutters-shed/stored-preparation-v3/experiment/native-front-retained-v1/worker.blend','58a3cbc5183af47d652dff172ec015dc89bd64fc1004c56776367348076c5b24',[1710,100,1792,300],[12,20])]
 prior=json.loads((OUT/'restart3-scene-audit/coherent-batch-v3-v1/first-hit/audit.json').read_text());source=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGB');reports=[]
 for asset,path,digest,box,regions in specs:
  assert sha(path)==digest;bpy.ops.wm.open_mainfile(filepath=str(path));scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene;bpy.context.view_layer.update()
  objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==asset];assert objects
  records=[];canvas=source.crop(box).resize(((box[2]-box[0])*4,(box[3]-box[1])*4),Image.Resampling.NEAREST);draw=ImageDraw.Draw(canvas)
  for obj in objects:
   points=[obj.matrix_world@v.co for v in obj.data.vertices];projected=[(p.x,-p.y*SIN-p.z*COS)for p in points]
   for edge in obj.data.edges:
    a,b=[projected[i]for i in edge.vertices]
    if max(a[0],b[0])<box[0]or min(a[0],b[0])>box[2]or max(a[1],b[1])<box[1]or min(a[1],b[1])>box[3]:continue
    draw.line([((a[0]-box[0])*4,(a[1]-box[1])*4),((b[0]-box[0])*4,(b[1]-box[1])*4)],fill='cyan',width=1)
   records.append(dict(name=obj.name,source_node=obj.get('source_node'),signature=_geometry(obj,protect_appearance=True),matrix=[list(r)for r in obj.matrix_world],vertices=[list(p)for p in points],projected=projected,faces=[list(p.vertices)for p in obj.data.polygons],materials=[m.name if m else None for m in obj.data.materials]))
  canvas.save(out/(asset+'-source-edges.png'));tree,owners,_=_tree(objects);hits=[]
  for region in regions:
   for sample in prior['components'][region-1]['samples']:
    x,y=sample['pixel'];hit,normal,index,dist=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000,-RAY)
    hits.append(dict(region=region,pixel=[x,y],object=owners[index].name if hit is not None else None,point=list(hit)if hit is not None else None))
  reports.append(dict(asset=asset,model=str(path),model_sha256=digest,source_crop=box,objects=records,hits=hits));assert sha(path)==digest
 write_json(out/'report.json',dict(status='Read-only actual saved mesh diagnostic',reports=reports))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
