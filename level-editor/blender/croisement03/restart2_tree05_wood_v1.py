"""Private four-track Tree05 construction with explicit source/gameplay distinction."""
import sys,math,json,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from restart2_tree13_wood_v1 import mesh_for
from render_slots import acquire,release
from render_views import render_views
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2';OUT=B/'tree05-wood-prototype-v1';SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));RAY=Vector((0,-COS,SIN))
SPECS={11: {'ground': 221, 'polygon': 0, 'observed': (55, 135), 'points': [(511, 221, 8), (512, 170, 7), (512, 134, 6), (513, 115, 5), (514, 95, 5), (515, 75, 4.5), (517, 55, 4), (519, 25, 3.8), (515, -10, 2.7), (501, -40, 0.6)]}, 501: {'ground': 214, 'polygon': 1, 'observed': (89, 150), 'points': [(530, 214, 6), (531, 170, 4.7), (532, 149, 4), (533, 130, 3.5), (534, 110, 3.5), (535, 90, 3.4), (535, 55, 3), (538, 15, 2.4), (548, -35, 0.5)]}, 502: {'ground': 199, 'polygon': 2, 'observed': (75, 149), 'points': [(550, 199, 7), (552, 169, 5), (553, 147, 4.5), (554, 125, 4.2), (555, 105, 4), (555, 85, 4), (554, 74, 3.8), (552, 35, 3.3), (547, 0, 2.3), (538, -30, 0.6)]}, 14: {'ground': 191, 'polygon': 3, 'observed': (72, 157), 'points': [(565, 191, 7), (565, 170, 5.5), (565, 156, 5), (565, 140, 4.5), (565, 120, 4.5), (565, 100, 4.5), (565, 80, 4.2), (565, 72, 4), (563, 35, 3.2), (565, 0, 2.3), (575, -30, 0.5)]}}

def main():
 assert shutil.disk_usage(ROOT).free>25*1024**3;OUT.mkdir(exist_ok=False);acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(B/'tree13-wood-v6/worker.blend'));scene=bpy.data.scenes['Tree13 isolated wood'];bpy.context.window.scene=scene
  for o in list(scene.objects):
   if o.type=='MESH':
    for collection in list(o.users_collection):collection.objects.unlink(o)
  image=Image.open(B.parent/'baseline/covered.png').convert('RGBA');mask=Image.open(B/'tree05-bark-proposal-v2/proposed-bark.png').convert('L');box=mask.getbbox();crop=image.crop(box);crop.putalpha(mask.crop(box));crop.save(OUT/'provisional-bark.png');im=bpy.data.images.load(str(OUT/'provisional-bark.png'));im.pack();left,top,right,bottom=box
  material=bpy.data.materials.new('Tree05 provisional bark cores and unknown wood');material.use_nodes=True;n=material.node_tree.nodes;n.clear();links=material.node_tree.links;tex=n.new('ShaderNodeTexImage');tex.image=im;tex.interpolation='Closest';tex.extension='CLIP';e=n.new('ShaderNodeEmission');unknown=n.new('ShaderNodeBsdfPrincipled');unknown.inputs['Base Color'].default_value=(.18,.18,.18,1);mix=n.new('ShaderNodeMixShader');out=n.new('ShaderNodeOutputMaterial');g=n.new('ShaderNodeNewGeometry');dot=n.new('ShaderNodeVectorMath');dot.operation='DOT_PRODUCT';dot.inputs[1].default_value=RAY;front=n.new('ShaderNodeMath');front.operation='GREATER_THAN';front.inputs[1].default_value=0;alpha=n.new('ShaderNodeMath');alpha.operation='MULTIPLY';links.new(g.outputs['Normal'],dot.inputs[0]);links.new(dot.outputs['Value'],front.inputs[0]);links.new(front.outputs[0],alpha.inputs[0]);links.new(tex.outputs['Alpha'],alpha.inputs[1]);links.new(tex.outputs['Color'],e.inputs[0]);links.new(alpha.outputs[0],mix.inputs[0]);links.new(unknown.outputs[0],mix.inputs[1]);links.new(e.outputs[0],mix.inputs[2]);links.new(mix.outputs[0],out.inputs[0]);objects=[]
  source_mask=np.array(mask)>0
  for node,spec in SPECS.items():
   fitted=[];region=Image.new('L',mask.size);ImageDraw.Draw(region).polygon(json.loads((B/'tree05-bark-proposal-v2/proposal.json').read_text())['polygons'][spec['polygon']],fill=255);stem_mask=source_mask&(np.array(region)>0)
   for x,y,r in spec['points']:
    if spec['observed'][0]<=y<=spec['observed'][1]:
     yy,xx=np.nonzero(stem_mask[max(0,int(y)-7):min(960,int(y)+8)])
     assert len(xx);x=(float(xx.min())+float(xx.max())+1)/2;r=max(r,(float(xx.max()-xx.min())+1)/2+.6)
    fitted.append((x,y,r))
   spec['points']=fitted
   mesh=mesh_for(f'Tree05 stem{node}',spec);mesh.materials.clear();mesh.materials.append(material);uv=mesh.uv_layers['UVMap']
   for f in mesh.polygons:
    f.use_smooth=len(f.vertices)==4
    for li in f.loop_indices:
     p=mesh.vertices[mesh.loops[li].vertex_index].co;uv.data[li].uv=((p.x-left)/(right-left),1-((-p.y*SIN-p.z*COS)-top)/(bottom-top))
   o=bpy.data.objects.new(f'Tree05 private stem{node}',mesh);scene.collection.objects.link(o);o['asset_group']='croisement03-tree-06';o['source_node']=('building-011' if node==11 else 'building-014' if node==14 else 'authored-tree05-observed-stem');o['construction_mapping']='tree05-bark-proposal-v2/construction-mapping.json';o['stem_component']=node;objects.append(o)
  trees=[(o,BVHTree.FromPolygons([v.co for v in o.data.vertices],[list(f.vertices) for f in o.data.polygons])) for o in objects];a=np.array(mask)>0;miss=[]
  for y,x in zip(*np.nonzero(a)):
   origin=Vector((x+.5,-(y+.5)/SIN,0))+RAY*10000
   if not any(t.ray_cast(origin,-RAY)[0] is not None for o,t in trees):miss.append([int(x),int(y)])
  target=Vector((541,-205/SIN,145))
  for i in range(8):
   cam=bpy.data.objects[f'Tree13 view{i}'];angle=i*math.tau/8;direction=Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN));cam.location=target+direction*1000;cam.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();cam.data.ortho_scale=410
  bpy.data.libraries.write(str(OUT/'worker.blend'),{scene},fake_user=True,compress=True);render_views(scene.name,{f'view-{i}':f'Tree13 view{i}' for i in range(8)},OUT/'actual',modes=('textured','solid'),width=384)
  for mode in ('textured','solid'):
   sheet=Image.new('RGB',(1536,768),'#333333')
   for i in range(8):
    p=Image.open(OUT/'actual'/f'view-{i}-{mode}.png').convert('RGBA');bg=Image.new('RGBA',p.size,'#333333');bg.alpha_composite(p);sheet.paste(bg.convert('RGB'),((i%4)*384,(i//4)*384))
   sheet.save(OUT/'actual'/f'{mode}.png')
  write_json(OUT/'receipt.json',dict(status='PRIVATE forked wood prototype; provisional bark proposal/root review and full crown pending',model_sha256=sha(OUT/'worker.blend'),proposed_pixels=int(a.sum()),source_ray_misses=miss,observed_prior_fern_receiver_pixels=0,source_spec=SPECS,limits=['Four observed stem tracks; central and pale tracks have authored scenery association only. Neighbor Tree06 bark is excluded; hidden roots remain inferred behind rock/ivy.','Coarse node labels are provenance, not exclusive source first-hit identity.','Upper caps are provisional support extents until shared canopy association resolved.','No texture API or geometry approval implied.']));print('MISSES',len(miss))
 finally:release()
if __name__=='__main__':main()
