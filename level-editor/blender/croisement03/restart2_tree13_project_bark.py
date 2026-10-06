"""Project only the reviewed 153 native bark pixels onto unchanged isolated stems."""
import json,math,sys,shutil,hashlib
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from render_views import render_views
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2';OUT=B/'tree13-wood-v3';SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));RAY=Vector((0,-COS,SIN))
def geom(o):return {'matrix':[list(r) for r in o.matrix_world],'vertices':[list(v.co) for v in o.data.vertices],'faces':[list(f.vertices) for f in o.data.polygons]}
def main():
 assert shutil.disk_usage(ROOT).free>25*1024**3;OUT.mkdir(exist_ok=False);source=B/'tree13-wood-v2/worker.blend';assert sha(source)=='520c66162f3014654592ea8d865c0c5531c8baf02106777ba89f03554d5b7ca2';source_rgb=Image.open(B.parent/'baseline/covered.png').convert('RGB');proposal=json.loads((B/'tree13-bark-proposal-v1/proposal.json').read_text());assert proposal['total_proposed_pixels']==153;acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.data.scenes['Tree13 isolated wood'];bpy.context.window.scene=scene;objects=[o for o in scene.objects if o.type=='MESH'];assert len(objects)==3;before={o.name:geom(o) for o in objects};records=[]
  for obj in objects:
   node=int(obj['source_node'].rsplit('-',1)[1]);mask_path=B/f'tree13-bark-proposal-v1/node-{node:03}-proposed-bark.png';mask=Image.open(mask_path).convert('L');box=mask.getbbox();assert box;left,top,right,bottom=box;w,h=right-left,bottom-top;rgba=source_rgb.crop(box).convert('RGBA');rgba.putalpha(mask.crop(box));file=OUT/f'bark-{node:03}.png';rgba.save(file);im=bpy.data.images.load(str(file),check_existing=False);im.pack();im.colorspace_settings.name='sRGB'
   uv=obj.data.uv_layers.new(name=f'Observed bark {node:03}')
   for face in obj.data.polygons:
    for li in face.loop_indices:
     p=obj.matrix_world@obj.data.vertices[obj.data.loops[li].vertex_index].co;sy=-p.y*SIN-p.z*COS;uv.data[li].uv=((p.x-left)/w,1-(sy-top)/h)
   mat=bpy.data.materials.new(f'Tree13 {node:03} reviewed native bark and unknown wood');mat.use_nodes=True;n=mat.node_tree.nodes;n.clear();links=mat.node_tree.links;out=n.new('ShaderNodeOutputMaterial');mix=n.new('ShaderNodeMixShader');unknown=n.new('ShaderNodeBsdfPrincipled');unknown.inputs['Base Color'].default_value=(.18,.18,.18,1);unknown.inputs['Roughness'].default_value=1;known=n.new('ShaderNodeEmission');known.inputs['Strength'].default_value=1;tex=n.new('ShaderNodeTexImage');tex.image=im;tex.interpolation='Closest';tex.extension='CLIP';uvnode=n.new('ShaderNodeUVMap');uvnode.uv_map=uv.name;links.new(uvnode.outputs['UV'],tex.inputs['Vector']);links.new(tex.outputs['Color'],known.inputs['Color']);g=n.new('ShaderNodeNewGeometry');dot=n.new('ShaderNodeVectorMath');dot.operation='DOT_PRODUCT';dot.inputs[1].default_value=RAY;links.new(g.outputs['Normal'],dot.inputs[0]);front=n.new('ShaderNodeMath');front.operation='GREATER_THAN';front.inputs[1].default_value=0;links.new(dot.outputs['Value'],front.inputs[0]);alpha=n.new('ShaderNodeMath');alpha.operation='MULTIPLY';links.new(front.outputs[0],alpha.inputs[0]);links.new(tex.outputs['Alpha'],alpha.inputs[1]);links.new(alpha.outputs[0],mix.inputs[0]);links.new(unknown.outputs[0],mix.inputs[1]);links.new(known.outputs[0],mix.inputs[2]);links.new(mix.outputs[0],out.inputs[0]);slot=len(obj.data.materials);obj.data.materials.append(mat)
   for face in obj.data.polygons:face.material_index=slot
   tree=BVHTree.FromPolygons([obj.matrix_world@v.co for v in obj.data.vertices],[list(f.vertices) for f in obj.data.polygons]);a=np.asarray(mask)>0;good=0;minimum_dot=1
   for y,x in zip(*np.nonzero(a)):
    origin=Vector((x+.5,-(y+.5)/SIN,0))+RAY*10000;p,normal,face,d=tree.ray_cast(origin,-RAY);assert p is not None;(u,v)=(p.x-left,(-p.y*SIN-p.z*COS)-top);assert abs(u-(x+.5-left))<.01 and abs(v-(y+.5-top))<.01;assert normal.dot(RAY)>0;minimum_dot=min(minimum_dot,normal.dot(RAY));assert rgba.getpixel((x-left,y-top))[:3]==source_rgb.getpixel((x,y));good+=1
   records.append(dict(node=node,pixels=good,crop=list(box),mask_sha256=sha(mask_path),image_sha256=sha(file),minimum_native_front_dot=minimum_dot))
  assert before=={o.name:geom(o) for o in objects};old=np.asarray(Image.open(B/'fern-wood-proposal-v1/35-proposed-wood.png'))>0;new=np.asarray(Image.open(B/'tree13-bark-proposal-v1/node-048-proposed-bark.png'))>0;assert np.all(new[old]);assert int(old.sum())==9
  bpy.data.libraries.write(str(OUT/'worker.blend'),{scene},fake_user=True,compress=True);views={f'view-{i}':next(o.name for o in scene.objects if o.type=='CAMERA' and o.name==f'Tree13 view{i}') for i in range(8)};render_views(scene.name,views,OUT/'actual',modes=('textured','solid'),width=384)
  for mode in ('textured','solid'):
   sheet=Image.new('RGB',(1536,768),'#333333')
   for i in range(8):
    im=Image.open(OUT/'actual'/f'view-{i}-{mode}.png').convert('RGBA');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),((i%4)*384,(i//4)*384))
   sheet.save(OUT/'actual'/f'{mode}.png')
  write_json(OUT/'receipt.json',dict(status='PRIVATE source projection; complete wood geometry and context review pending',model_sha256=sha(OUT/'worker.blend'),source_geometry_sha256=sha(source),geometry_exact=True,observed_source_rgb_exact=True,observed_pixels=153,prior_nine_bark_pixels_preserved=True,records=records,actual8_sha256=sha(OUT/'actual/textured.png'),native_view_index=0,limits=['Only reviewed narrow bark cores are observed; remaining mixed mask pixels stay unassigned.','Known RGB uses emission to retain source appearance; unknown wood remains neutral diagnostic material.','No API, inferred bark fill, canopy or wind appearance included.']))
 finally:release()
if __name__=='__main__':main()
