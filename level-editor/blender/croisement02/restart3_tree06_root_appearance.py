"""Project observed native pixels onto front-hit root faces; keep unknown volume gray."""
import sys,json,math
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from tree_geometry import SIN,COS,RAY
from evidence_io import sha,write_json
from render_slots import acquire,release
from sign_context_import import append_verified
from restart3_tree06_root_review import configure
from restart2_sign_neighbors import camera_to,render

def main():
 base=OUT/'restart3-tree06-root/research-roots-v5';dest=base/'appearance-v3';dest.mkdir(exist_ok=False)
 model=base/'root.blend';audit=json.loads((base/'full-neighborhood-audit.json').read_text());assert sha(model)==audit['model_sha256']
 bpy.ops.wm.open_mainfile(filepath=str(model));obj=next(o for o in bpy.context.scene.objects if o.type=='MESH');obj.data.calc_loop_triangles()
 tree=BVHTree.FromPolygons([tuple(v.co)for v in obj.data.vertices],[tuple(t.vertices)for t in obj.data.loop_triangles],all_triangles=True)
 box=audit['crop'];w,h=box[2]-box[0],box[3]-box[1];mask=np.zeros((h,w,4),np.uint8);mask[:,:,3]=255;polys=set()
 for row in audit['pixels']:
  if not row['wood_domain']:continue
  x,y=row['pixel'];origin=Vector((x+.5,-(y+.5)/SIN,0))+RAY*5000;hit,normal,face,d=tree.ray_cast(origin,-RAY,10000)
  assert face is not None
  polys.add(obj.data.loop_triangles[face].polygon_index);mask[y-box[1],x-box[0],:3]=255
 # Resolve subpixel mesh faces independently, so fine remeshing does not leave gray holes.
 for polygon in obj.data.polygons:
  center=polygon.center;x=center.x;y=-SIN*center.y-COS*center.z;ix,iy=math.floor(x)-box[0],math.floor(y)-box[1]
  if not(0<=ix<w and 0<=iy<h):continue
  hit,normal,face,d=tree.ray_cast(center+RAY*5000,-RAY,10000)
  if hit is not None and (hit-center).length<.08 and polygon.normal.dot(RAY)>0:polys.add(polygon.index)
 Image.fromarray(mask).save(dest/'observed-mask.png');Image.open(OUT/'baseline/covered.png').crop(box).save(dest/'source.png')
 uv=obj.data.uv_layers.new(name='Native observed projection')
 for face in obj.data.polygons:
  for loop in face.loop_indices:
   v=obj.data.vertices[obj.data.loops[loop].vertex_index].co;uv.data[loop].uv=((v.x-box[0])/w,1-(-SIN*v.y-COS*v.z-box[1])/h)
 def material(name,observed):
  mat=bpy.data.materials.new(name);mat.use_nodes=True;n=mat.node_tree.nodes;n.clear();out=n.new('ShaderNodeOutputMaterial');em=n.new('ShaderNodeEmission');em.inputs['Color'].default_value=(.22,.22,.22,1);mat.node_tree.links.new(em.outputs[0],out.inputs['Surface'])
  if observed:
   src=n.new('ShaderNodeTexImage');src.image=bpy.data.images.load(str(dest/'source.png'));src.interpolation='Closest';src.extension='CLIP';src.image.pack()
   own=n.new('ShaderNodeTexImage');own.image=bpy.data.images.load(str(dest/'observed-mask.png'));own.image.colorspace_settings.name='Non-Color';own.interpolation='Closest';own.extension='CLIP';own.image.pack()
   mix=n.new('ShaderNodeMixRGB');mix.inputs[1].default_value=(.22,.22,.22,1);mat.node_tree.links.new(own.outputs['Color'],mix.inputs[0]);mat.node_tree.links.new(src.outputs['Color'],mix.inputs[2]);mat.node_tree.links.new(mix.outputs[0],em.inputs['Color'])
  return mat
 obj.data.materials.clear();obj.data.materials.append(material('Unknown inferred root volume',False));obj.data.materials.append(material('Observed first-hit native root front',True))
 for face in obj.data.polygons:face.material_index=int(face.index in polys)
 saved=dest/'root.blend';bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(saved),compress=True);name=obj.name;expected={name:dict(matrix_world=[list(r)for r in obj.matrix_world])}
 bank=OUT/'restart2-bank321/packaged-v1/assets/croisement02-north-woodland-bank/model.blend';bpy.ops.wm.open_mainfile(filepath=str(bank));bpy.context.view_layer.update();names=[o.name for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')=='croisement02-north-woodland-bank'];refs={n:dict(matrix_world=[list(r)for r in bpy.data.objects[n].matrix_world])for n in names}
 original=Path(json.loads((OUT/'restart3-tree06-root/probe.json').read_text())['model']);bpy.ops.wm.open_mainfile(filepath=str(original));bpy.context.view_layer.update();scene=bpy.context.scene
 from restart3_tree06_root_correction import fingerprint
 originals=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')=='croisement02-tree-06'];fingerprints={o.name:fingerprint(o)for o in originals}
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o not in originals or 'Crown'in o.name
 roots,root_receipt=append_verified(scene,saved,[name],expected);banks,bank_receipt=append_verified(scene,bank,names,refs);camera=configure(scene);camera.data.ortho_scale=112;camera_to(camera,Vector((650,-520/SIN,0)),RAY)
 roots[0].hide_render=True;before=render(scene,dest/'native-before.png');roots[0].hide_render=False;after=render(scene,dest/'native-after.png')
 source=Image.open(dest/'source.png').convert('RGBA').resize((384,384),Image.Resampling.NEAREST);sheet=Image.new('RGB',(1152,408),(60,60,60))
 for i,pic in enumerate([source,before,after]):sheet.paste(pic,(i*384,0),pic.getchannel('A'));ImageDraw.Draw(sheet).text((i*384+5,388),['Source art','Approved geometry','Private smooth roots'][i],fill='white')
 sheet.save(dest/'source-comparison.png');sheet=Image.new('RGB',(1536,816),(60,60,60));camera.data.ortho_scale=145;center=Vector((650,(-535-COS*47)/SIN,47))
 for i in range(8):
  a=i*math.pi/4;camera_to(camera,center,Vector((math.sin(a)*COS,-math.cos(a)*COS,SIN)));pic=render(scene,dest/f'contact-{i}.png');sheet.paste(pic,(i%4*384,i//4*408),pic.getchannel('A'));ImageDraw.Draw(sheet).text((i%4*384+5,i//4*408+388),f'Actual material / contact {i}',fill='white')
 sheet.save(dest/'contact-eight.png');assert fingerprints=={o.name:fingerprint(o)for o in originals}
 data=json.loads((base/'report.json').read_text());overlay=source.copy();draw=ImageDraw.Draw(overlay)
 for row in data['samples']:
  color=(255,40,40)if row['wood']and not row['visible']else((40,120,255)if not row['wood']and row['visible']else None)
  if color:
   x,y=row['pixel'];x=(x-box[0])*384/w;y=(y-box[1])*384/h;draw.rectangle((x,y,x+3,y+3),outline=color)
 overlay.save(dest/'source-residual-overlay.png')
 write_json(dest/'report.json',dict(model_sha256=sha(saved),geometry_sha256=sha(model),original_sha256=sha(original),original_fingerprints_unchanged=True,observed_pixels=int((mask[:,:,0]>0).sum()),observed_front_polygons=len(polys),context_imports=[root_receipt,bank_receipt],target313_covered=data['target313_covered'],bank131_overreach=data['bank131_overreach'],limitations=['Opaque unknown volume remains gray; native grass is never mapped to wood.','Only source-ray first-hit polygons at owned samples receive native projection; inferred rear faces are gray.','Bounded residual source and bank overlaps remain; no geometry approval or native parity claim.','Original crown hidden only for contact inspection.']))

if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
