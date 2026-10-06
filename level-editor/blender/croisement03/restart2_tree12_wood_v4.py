"""Private three visible stem tracks; shared crown and topology remain separate."""
import sys,math,json,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from restart2_tree13_wood_v1 import mesh_for
from render_slots import acquire,release
from render_views import render_views
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2';OUT=B/'tree12-wood-v4';SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));RAY=Vector((0,-COS,SIN))
SPECS={25:dict(ground=179,points=[(986,179,4),(985,151,4),(985,136,4),(983.5,116,3.5),(982.5,96,3),(981,60,4),(983.5,25,4),(986,0,4),(988,-35,2.5),(985,-65,.5)]),24:dict(ground=187,points=[(995,187,4),(996,151,3),(997,132,3),(999,112,3),(1001,94,3),(1003,60,3),(1004,25,3),(1007,0,3),(1008,-35,2),(1005,-65,.4)]),47:dict(ground=187,points=[(1003,187,4),(1004,165,3.4),(1004.5,148,3.3),(1006,131,3.5),(1007,117,3.5),(1009.5,102,3.6),(1010.5,90,4),(1011,60,5),(1017,25,5.5),(1022,0,6),(1023,-40,3),(1019,-70,.5)])}
def fit_source_profiles():
 native=np.array(Image.open(B.parent/'baseline/masks/000012.png'))>0
 for node,spec in SPECS.items():
  old=spec['points'];new=[]
  for y in sorted({0,spec['ground'],*range(0,int(spec['ground']),3)},reverse=True):
   x=float(np.interp(y,[p[1] for p in reversed(old)],[p[0] for p in reversed(old)]));r=float(np.interp(y,[p[1] for p in reversed(old)],[p[2] for p in reversed(old)]));xs=np.where(native[min(native.shape[0]-1,int(y))])[0]+977;runs=[q for q in np.split(xs,np.where(np.diff(xs)>1)[0]+1) if len(q)]
   if len(runs)>=3 and y<55 or node==25 and len(runs)>=2 and y<145:
    run=runs[{25:0,24:1,47:2}[node]];x=(int(run[0])+int(run[-1])+1)/2;r=(int(run[-1])-int(run[0])+1)/2-.1
   elif runs:
    run=min(runs,key=lambda q:abs(float(q.mean())-x));lo,hi=int(run[0]),int(run[-1])+1
    if lo<x<hi:r=max(.7,min(r,x-lo+.15,hi-x+.15))
   if node==47:r+=.3*max(0,1-abs(y-158)/7)
   new.append((x,float(y),r))
  new.extend(p for p in old if p[1]<0);spec['points']=new
fit_source_profiles()
def main():
 assert shutil.disk_usage(ROOT).free>25*1024**3;OUT.mkdir(exist_ok=False);acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(B/'tree13-wood-v6/worker.blend'));scene=bpy.data.scenes['Tree13 isolated wood'];bpy.context.window.scene=scene
  for o in list(scene.objects):
   if o.type=='MESH':bpy.data.objects.remove(o,do_unlink=True)
  image=Image.open(B.parent/'baseline/covered.png').convert('RGBA');mask=Image.open(B/'tree12-bark-proposal-v1/proposed-bark.png').convert('L');box=mask.getbbox();crop=image.crop(box);crop.putalpha(mask.crop(box));crop.save(OUT/'provisional-bark.png');im=bpy.data.images.load(str(OUT/'provisional-bark.png'));im.pack();left,top,right,bottom=box
  material=bpy.data.materials.new('Tree12 provisional bark cores and unknown wood');material.use_nodes=True;n=material.node_tree.nodes;n.clear();links=material.node_tree.links;tex=n.new('ShaderNodeTexImage');tex.image=im;tex.interpolation='Closest';tex.extension='CLIP';e=n.new('ShaderNodeEmission');unknown=n.new('ShaderNodeBsdfPrincipled');unknown.inputs['Base Color'].default_value=(.18,.18,.18,1);mix=n.new('ShaderNodeMixShader');out=n.new('ShaderNodeOutputMaterial');g=n.new('ShaderNodeNewGeometry');dot=n.new('ShaderNodeVectorMath');dot.operation='DOT_PRODUCT';dot.inputs[1].default_value=RAY;front=n.new('ShaderNodeMath');front.operation='GREATER_THAN';front.inputs[1].default_value=0;alpha=n.new('ShaderNodeMath');alpha.operation='MULTIPLY';links.new(g.outputs['Normal'],dot.inputs[0]);links.new(dot.outputs['Value'],front.inputs[0]);links.new(front.outputs[0],alpha.inputs[0]);links.new(tex.outputs['Alpha'],alpha.inputs[1]);links.new(tex.outputs['Color'],e.inputs[0]);links.new(alpha.outputs[0],mix.inputs[0]);links.new(unknown.outputs[0],mix.inputs[1]);links.new(e.outputs[0],mix.inputs[2]);links.new(mix.outputs[0],out.inputs[0]);objects=[]
  for node,spec in SPECS.items():
   mesh=mesh_for(f'Tree12 stem{node}',spec);mesh.materials.clear();mesh.materials.append(material);uv=mesh.uv_layers['UVMap']
   for f in mesh.polygons:
    f.use_smooth=len(f.vertices)==4
    for li in f.loop_indices:
     p=mesh.vertices[mesh.loops[li].vertex_index].co;uv.data[li].uv=((p.x-left)/(right-left),1-((-p.y*SIN-p.z*COS)-top)/(bottom-top))
   o=bpy.data.objects.new(f'Tree12 private stem{node}',mesh);scene.collection.objects.link(o);o['asset_group']='croisement03-tree-12';o['source_node']=f'building-{node:03}';objects.append(o)
  trees=[(o,BVHTree.FromPolygons([v.co for v in o.data.vertices],[list(f.vertices) for f in o.data.polygons])) for o in objects];a=np.array(mask)>0;miss=[]
  for y,x in zip(*np.nonzero(a)):
   origin=Vector((x+.5,-(y+.5)/SIN,0))+RAY*10000
   if not any(t.ray_cast(origin,-RAY)[0] is not None for o,t in trees):miss.append([int(x),int(y)])
  target=Vector((1000,-325,140))
  for i in range(8):
   cam=bpy.data.objects[f'Tree13 view{i}'];angle=i*math.tau/8;direction=Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN));cam.location=target+direction*1000;cam.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();cam.data.ortho_scale=365
  bpy.data.libraries.write(str(OUT/'worker.blend'),{scene},fake_user=True,compress=True);render_views(scene.name,{f'view-{i}':f'Tree13 view{i}' for i in range(8)},OUT/'actual',modes=('textured','solid'),width=384)
  for mode in ('textured','solid'):
   sheet=Image.new('RGB',(1536,768),'#333333')
   for i in range(8):
    p=Image.open(OUT/'actual'/f'view-{i}-{mode}.png').convert('RGBA');bg=Image.new('RGBA',p.size,'#333333');bg.alpha_composite(p);sheet.paste(bg.convert('RGB'),((i%4)*384,(i//4)*384))
   sheet.save(OUT/'actual'/f'{mode}.png')
  write_json(OUT/'receipt.json',dict(status='PRIVATE incomplete wood draft; source505 proposal/root review and full crown pending',model_sha256=sha(OUT/'worker.blend'),proposed_pixels=int(a.sum()),source_ray_misses=miss,observed_prior_fern_receiver_pixels=64,source_spec=SPECS,limits=['Separate visible stem tracks; obscured joining not invented.','Coarse node labels are provenance, not exclusive source first-hit identity.','Upper caps are provisional support extents until shared canopy association resolved.','No texture API or geometry approval implied.']));print('MISSES',len(miss))
 finally:release()
if __name__=='__main__':main()
