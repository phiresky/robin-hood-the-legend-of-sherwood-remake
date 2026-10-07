"""Reopen private market well for native-first geometry, appearance and contact review."""
import sys,json,math,hashlib
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from refinement_review import _tree
B=ROOT/'level-editor/work/york-refinement';D=B/'restart7-market-well-v4';S=math.sin(math.radians(35));C=math.cos(math.radians(35));RAY=Vector((0,-C,S));ASSET='york-market-roofed-stone-well'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def main():
 model=D/'model.blend';digest=sha(model);bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;bpy.context.view_layer.update();own=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')==ASSET];context=[o for o in scene.objects if o.type=='MESH'and o not in own];cam=bpy.data.objects.new('Private native-first camera',bpy.data.cameras.new('Private native-first camera'));scene.collection.objects.link(cam);cam.data.type='ORTHO';cam.data.clip_end=20000;scene.camera=cam;scene.cycles.samples=8
 def camera(center,angle,scale):
  direction=Vector((math.sin(angle)*C,-math.cos(angle)*C,S));cam.location=Vector(center)+direction*5000;cam.rotation_euler=(Vector(center)-cam.location).to_track_quat('-Z','Y').to_euler();cam.data.ortho_scale=scale;bpy.context.view_layer.update()
 def render(path):scene.render.filepath=str(path);bpy.ops.render.render(write_still=True);return Image.open(path).convert('RGBA')
 folder=D/'review';folder.mkdir(exist_ok=False);center=(232,-2245,138);records=[]
 for mode in ['actual','solid']:
  sheet=Image.new('RGB',(1536,816),(42,42,42));draw=ImageDraw.Draw(sheet)
  for o in context:o.hide_render=True
  if mode=='solid':
   scene.render.engine='BLENDER_WORKBENCH';scene.display.shading.light='STUDIO';scene.display.shading.studiolight_rotate_z=0;scene.display.shading.color_type='SINGLE';scene.display.shading.single_color=(.65,.65,.65);scene.display.shading.show_shadows=True;scene.display.shading.show_cavity=True;scene.display.shading.cavity_type='BOTH'
  for i in range(8):
   camera(center,i*math.pi/4,100);pic=render(folder/f'{mode}-{i}.png');sheet.paste(pic,(i%4*384,i//4*408),pic.getchannel('A'));draw.text((i%4*384+6,i//4*408+390),f'{mode} {i}; native first',fill='white')
  sheet.save(folder/f'{mode}-eight.png')
 scene.render.engine='CYCLES'
 for o in context:o.hide_render=False
 sheet=Image.new('RGB',(1536,408),(42,42,42));draw=ImageDraw.Draw(sheet)
 for i,az in enumerate([0,math.pi/2,math.pi,3*math.pi/2]):
  camera(center,az,130);pic=render(folder/f'contact-{i}.png');sheet.paste(pic,(i*384,0),pic.getchannel('A'));draw.text((i*384+6,390),f'Exact context contact {i}',fill='white')
 sheet.save(folder/'contact-four.png')
 # Native source camera maps the 160-square original region exactly.
 box=(180,1125,280,1225);camera((230,-1175/S,0),0,100);render(folder/'native-context.png');source=Image.open(B/'baseline/covered.png').convert('RGBA').crop(box).resize((384,384),Image.Resampling.NEAREST);sheet=Image.new('RGB',(768,410),(42,42,42));sheet.paste(source,(0,0));pic=Image.open(folder/'native-context.png').convert('RGBA');sheet.paste(pic,(384,0),pic.getchannel('A'));ImageDraw.Draw(sheet).text((6,390),'Native artwork / exact saved candidate and context',fill='white');sheet.save(folder/'source-comparison.png')
 bpy.context.view_layer.update();tree,owners,_=_tree(own+context);mask=np.array(Image.open(D/'native-domain.png').convert('L'))>0;hits=[];miss=[];foreign=[]
 for yy,xx in np.argwhere(mask):
  x=int(xx)+202;y=int(yy)+1138;p,_,i,_=tree.ray_cast(Vector((x+.5,-(y+.5)/S,0))+RAY*5000,-RAY)
  if p is None:miss.append([x,y])
  elif owners[i]in own:hits.append([x,y])
  else:foreign.append(dict(pixel=[x,y],owner=owners[i].name))
 outside=[]
 for yy,xx in np.argwhere(~mask):
  x=int(xx)+202;y=int(yy)+1138;p,_,i,_=tree.ray_cast(Vector((x+.5,-(y+.5)/S,0))+RAY*5000,-RAY)
  if p is not None and owners[i]in own:outside.append([x,y])
 overlay=Image.open(B/'baseline/covered.png').convert('RGB');draw=ImageDraw.Draw(overlay)
 for row in foreign:draw.point(tuple(row['pixel']),fill=(255,0,255))
 for point in outside:draw.point(tuple(point),fill=(0,255,255))
 overlay.crop((200,1135,262,1215)).resize((620,800),Image.Resampling.NEAREST).save(folder/'source-coverage-overlay.png')
 floor=next(o for o in context if o.get('source_node')=='building-086');ft,_,_=_tree([floor]);contacts=[]
 for x,y in [(231+17.5*math.cos(a*math.pi/4),-1288.5/S+15.5*math.sin(a*math.pi/4))for a in range(8)]+[(246.7+2.9*math.cos(a*math.pi/4),-2264.6+2.9*math.sin(a*math.pi/4))for a in range(8)]:
  p,_,_,_=ft.ray_cast(Vector((x,y,500)),Vector((0,0,-1)));contacts.append(dict(xy=[x,y],ground_z=p.z if p else None,base_z=109.751,clearance=109.751-p.z if p else None))
 for o in own:
  bm=bmesh.new();bm.from_mesh(o.data);records.append(dict(object=o.name,vertices=len(bm.verts),faces=len(bm.faces),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges)));bm.free()
 report=dict(status='Private candidate review rendered; self-review pending',model_sha256=digest,mask_domain_pixels=int(mask.sum()),own_first_hit=len(hits),misses=miss,foreign_first_hits=foreign,outside_domain_first_hits=outside,closed_meshes=records,contacts=contacts,limitations=['Native mask49 maps ring artwork by source evidence; mask50 links roof036/037 explicitly.','Unknown back/underside remains gray; no texture synthesis or approval.','Solid studio lighting is diagnostic, not a calibrated source sun claim.','Well is a separate worker; exact raised terrain086 is read-only context. Small pail uses a separately traced inferred source domain; surrounding source shadow is not modeled.']);(D/'review.json').write_text(json.dumps(report,indent=2)+'\n');assert sha(model)==digest;print('COVERAGE',len(hits),int(mask.sum()),'foreign',len(foreign),'miss',len(miss),flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
