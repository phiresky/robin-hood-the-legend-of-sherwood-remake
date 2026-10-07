"""Expose sloped leaf artwork against its exact terrain, with matched source markers."""
from pathlib import Path
import sys,json,hashlib,math
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
from bpy_extras.object_utils import world_to_camera_view
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN,COS
from refinement_review import _tree
from restart4_stump_final_contact import frame,sheet
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 root=OUT/'restart9-hiding-scatter';worker=root/'scatter-surfaces-v2';rec=json.loads((worker/'manifest.json').read_text());audit=json.loads((root/'terrain-receivers-v2/report.json').read_text());out=OUT/'restart11-hiding-mound/scatter-slope-contact-v1';out.mkdir(parents=True,exist_ok=False);model=worker/'model.blend';assert sha(model)==rec['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;states=[o for o in scene.objects if o.type=='MESH'];receivers=[]
 for pin in audit['models']:
  path=Path(pin['path']);assert sha(path)==pin['sha256']
  with bpy.data.libraries.load(str(path),link=False)as(src,dst):dst.scenes=src.scenes
  for imported in dst.scenes:
   bpy.context.window.scene=imported;bpy.context.view_layer.update()
   for row in pin['objects']:
    candidates=[o for o in imported.objects if o.name==row['name']or o.name.startswith(row['name']+'.')]
    for o in candidates:
     matrix=o.matrix_world.copy();bpy.context.window.scene=scene;scene.collection.objects.link(o);o.parent=None;o.matrix_world=matrix;receivers.append(o)
 bpy.context.window.scene=scene;bpy.context.view_layer.update();scene.cycles.samples=16;scene.cycles.transparent_max_bounces=512;images=[];cases=[]
 for index in [15,16]:
  row=rec['records'][index];own=[bpy.data.objects[n]for n in row['objects']];tree,owners,_=_tree(own);rgba=np.array(Image.open(row['source']).convert('RGBA'));x0,y0,w,h=row['bbox'];samples=[]
  for y,x in np.argwhere(rgba[:,:,3]>0):
   p,n,ti,d=tree.ray_cast(Vector((x0+x+.5,-(y0+y+.5)/SIN,0))+RAY*6000,-RAY)
   if p is not None:samples.append(dict(pixel=[int(x),int(y)],point=list(p),normal=list(n),surface_area_per_native_area=1/max(1e-8,abs(n.dot(RAY)))))
  selected=[]
  for s in sorted(samples,key=lambda x:x['surface_area_per_native_area'],reverse=True):
   if all(np.linalg.norm(np.array(s['pixel'])-np.array(p['pixel']))>8 for p in selected):selected.append(s)
   if len(selected)==3:break
  source=Image.open(row['source']).convert('RGBA');canvas=Image.new('RGBA',source.size,(65,65,65,255));canvas.alpha_composite(source);canvas=canvas.resize((w*6,h*6),Image.Resampling.NEAREST);draw=ImageDraw.Draw(canvas)
  for letter,s in zip('ABC',selected):x,y=(np.array(s['pixel'])+.5)*6;draw.ellipse((x-7,y-7,x+7,y+7),outline='magenta',width=2);draw.text((x+9,y-6),letter,fill='white')
  file=out/f'{index}-source.png';canvas.save(file);images.append(file)
  for o in scene.objects:
   if o.type=='MESH':o.hide_render=o not in own and o not in receivers
  for label,direction in [('native',RAY),('side',Vector((COS,0,SIN))),('reverse',Vector((0,COS,SIN)))]:
   camera=frame(scene,own,direction,768,1.3);file=out/f'{index}-{label}.png';scene.render.filepath=str(file);bpy.ops.render.render(write_still=True);im=Image.open(file).convert('RGBA');draw=ImageDraw.Draw(im)
   for letter,s in zip('ABC',selected):q=world_to_camera_view(scene,camera,Vector(s['point']));x=q.x*im.width;y=(1-q.y)*im.height;draw.ellipse((x-8,y-8,x+8,y+8),outline='magenta',width=2);draw.text((x+11,y-7),letter,fill='white')
   im.save(out/f'{index}-{label}-annotated.png');images.append(out/f'{index}-{label}-annotated.png')
  cases.append(dict(index=index,instance=row['instances'],samples=selected,maximum_surface_area_per_native_area=max(s['surface_area_per_native_area']for s in samples),minimum_abs_normal_z=min(abs(s['normal'][2])for s in samples)))
 sheet(images,out/'source-contact-sheet.png');(out/'report.json').write_text(json.dumps(dict(status='DIAGNOSTIC_REVIEW',model_sha256=sha(model),terrain_models=audit['models'],cases=cases,scope='Shared scatter mesh unchanged. Exact bank and ground only, no canopy. Source A/B/C identify strongest sampled projection stretch; annotation is diagnostic, not new ownership.'),indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
