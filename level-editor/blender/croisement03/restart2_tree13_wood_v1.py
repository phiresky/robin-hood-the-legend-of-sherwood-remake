"""Private three-stem continuation with narrowly established fern receiver bark."""
import json,math,sys,shutil
from pathlib import Path
import bpy,bmesh
from mathutils import Vector
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from render_slots import acquire,release
from refinement_workspace import prepare,modified
from evidence_io import sha,write_json
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));ASSET='croisement03-tree-13'
# Source-plane centre/radius hypotheses preserve separate stems. The unseen
# upper continuation is deliberately explicit; no north-edge cap is accepted.
SPECS={31:dict(ground=87,points=[(1065,87,7.2),(1065,60,6.6),(1062,25,5.9),(1059,0,5.4),(1057,-45,4.8),(1060,-95,3.8),(1056,-150,2.0)]),32:dict(ground=95,points=[(1088,95,8.0),(1090,65,7.4),(1092,30,6.6),(1095,0,6.0),(1099,-50,5.2),(1102,-110,4.0),(1110,-175,2.0)]),48:dict(ground=101,points=[(1043,101,6.2),(1045,78,5.5),(1046,45,5.0),(1047,0,4.5),(1048,-55,4.0),(1044,-115,3.0),(1045,-160,1.7)])}
def mesh_for(name,spec):
 vertices=[];faces=[];sides=16
 for j,(x,y,r) in enumerate(spec['points']):
  c=Vector((x,-spec['ground']/SIN,(spec['ground']-y)/COS))
  for k in range(sides):
   a=k*math.tau/sides;rad=r*(1+.035*math.cos(5*a+j*.35));vertices.append(tuple(c+Vector((math.cos(a)*rad,math.sin(a)*rad,0))))
 faces.append(tuple(reversed(range(sides))))
 for j in range(len(spec['points'])-1):
  for k in range(sides):faces.append((j*sides+k,j*sides+(k+1)%sides,(j+1)*sides+(k+1)%sides,(j+1)*sides+k))
 faces.append(tuple((len(spec['points'])-1)*sides+k for k in range(sides)))
 m=bpy.data.meshes.new(name);m.from_pydata(vertices,[],faces);m.update();bm=bmesh.new();bm.from_mesh(m);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges);bm.to_mesh(m);bm.free();m.uv_layers.new(name='UVMap');mat=bpy.data.materials.new(name+' inferred bark');mat.diffuse_color=(.42,.42,.42,1);m.materials.append(mat);return m

def main():
 assert shutil.disk_usage(ROOT).free>25*1024**3
 root=OUT/'restart2/tree13-wood-v1';root.mkdir(exist_ok=False);worker=root/'assets'/ASSET
 bark=OUT/'restart2/fern-wood-proposal-v1/35-proposed-wood.png';assert sum(v>0 for v in Image.open(bark).getdata())==9
 masks=json.loads((OUT/'baseline/masks/manifest.json').read_text())
 for row in masks['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
 masks['masks'].append(dict(index=131,layer=0,layer_index=131,png=str(bark),box_top_left=[0,0],box_size=list(Image.open(bark).size),authored=True,mask_type=0,obstacle_indices=[48]));write_json(root/'mask-inventory.json',masks)
 write_json(root/'source-masks.json',dict(version=1,mask_inventory=str(root/'mask-inventory.json'),projections={'exterior':dict(state='Private wood only; nine established receiver bark pixels, all other mixed domain unresolved',source_sha256=sha(OUT/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=ASSET,mask_indices=[131])])}))
 write_json(root/'construction-plan.json',dict(specs=SPECS,known_bark_pixels=9,reference_assets=['leicester-southeast-cottage-tree','leicester-moat-bank-tree'],status='PRIVATE HOLD: three trunks only, inferred continuation, mixed leaf domain and crowns unresolved'))
 acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement03-grouped.blend'));bpy.context.preferences.filepaths.save_version=0
  objects=[o for o in bpy.data.collections['Croisement03 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==ASSET];assert len(objects)==3
  built={}
  for o in objects:
   node=o.get('source_node',o.name).split('.')[0];index=int(node.rsplit('-',1)[1]);assert index in SPECS;m=mesh_for('Tree13 '+node+' closed inferred stem',SPECS[index]);o.data=m;o.matrix_world.identity();o['construction_proxy']='Three distinct source stems with explicit north continuation; crown pending';built[o.name]=m
  prepare(worker,asset_id=ASSET,scene_name='Croisement03 Refinement',collection_name='Croisement03 Working',source_path=OUT/'baseline/covered.png',grouping_manifest=OUT/'catalog.json',inventory_path=OUT/'inventory/inventory.json',review_path=OUT/'grouping-review.json',source_mask_manifest=root/'source-masks.json',width=384,height=384,framing_padding=1.25,lighting=dict(toward_sun=[-.45,-.55,.70],ambient=.22,diffuse=.78,shadow_epsilon=.05))
  modified(worker);write_json(root/'receipt.json',dict(status='PRIVATE HOLD; wood-only construction requires native/contact review and complete independent mixed pixel ownership',model_sha256=sha(worker/'model.blend'),known_bark_pixels=9,source_nodes=[31,32,48],limitations=['Three separate stems, not one merged trunk.','Upper extents and circular cross sections inferred from clipped source context.','All leaf pixels and unknown bark outside established nine-pixel receiver domain remain unassigned.','No geometry, texture, canopy or complete asset approval implied.']))
  print(worker)
 finally:release()
if __name__=='__main__':main()
