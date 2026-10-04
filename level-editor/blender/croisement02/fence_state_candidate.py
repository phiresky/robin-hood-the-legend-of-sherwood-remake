"""Isolated source-domain fence clearing, retaining the complete covered meshes."""
import sys,json,math,hashlib
from pathlib import Path
import bpy,bmesh
import numpy as np
from mathutils import Vector
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender'),str(Path(__file__).parent)]
from render_slots import acquire,release
from refinement_workspace import _geometry
from render_multiview_asset import render
from catalog import OUT
S=math.sin(math.radians(35));C=math.cos(math.radians(35))
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def write(p,v):p.write_text(json.dumps(v,indent=2)+'\n')
def main():
 dest=Path(sys.argv[sys.argv.index('--')+1]);dest.mkdir(exist_ok=False)
 exp=OUT/'texture-fill-round-1/croisement02-south-field-wattle-fence/experiment';base=exp/'bake-v1/worker.blend'
 frames=json.loads((exp/'views.json').read_text());layers=json.loads((OUT/'source-states/layers.json').read_text())
 patches=[p for p in layers['mission_patches'] if p['name']=='chariot02_barriere'];f=patches[0]['states']['transition']['frames'][0]
 assert all(len(p['states']['transition']['frames'])==1 and not p['state']['end_animation_valid'] and p['state']['integrate_in_background'] for p in patches)
 img=OUT/'source-states'/f['image'];rgba=np.array(Image.open(img).convert('RGBA'));assert np.all(rgba[:,:,3]==255)
 assert all(sha(OUT/'source-states'/p['states']['transition']['frames'][0]['image'])==sha(img) for p in patches)
 x,y,w,h=f['bbox'];acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(base));scene=bpy.data.scenes[frames['scene_name']];bpy.context.window.scene=scene
  before={o.name:_geometry(o,protect_appearance=True) for o in scene.objects};names=frames['object_names'];objs=[scene.objects[n] for n in names]
  copies=[];audits=[]
  def signature(o,face):
   uv=o.data.uv_layers.active
   return tuple(sorted(tuple(round(v,4) for v in (list(o.matrix_world@o.data.vertices[o.data.loops[i].vertex_index].co)+list(uv.data[i].uv))) for i in face.loop_indices))
  for original in objs:
   obj=original.copy();obj.data=original.data.copy();scene.collection.objects.link(obj);obj.name=original.name+' / cleared';copies.append(obj)
   external=[]
   for face in original.data.polygons:
    ps=[original.matrix_world@original.data.vertices[i].co for i in face.vertices];q=[(p.x,-S*p.y-C*p.z) for p in ps]
    if max(p[0] for p in q)<x or min(p[0] for p in q)>x+w or max(p[1] for p in q)<y or min(p[1] for p in q)>y+h:external.append(signature(original,face))
   points=[obj.matrix_world@v.co for v in obj.data.vertices]
   middle=[p for p in points if x<=p.x<=x+w]
   assert all(y<=-S*p.y-C*p.z<=y+h for p in middle), 'Rectangle vertical edges need explicit cuts'
   xs=[p.x for p in points]
   assert min(xs)>=x or max(xs)<=x+w, 'Two retained pieces require separate clipping'
   keep_right=min(xs)>=x;cut=x+w if keep_right else x
   inv=obj.matrix_world.inverted();co=inv@Vector((cut,0,0));normal=obj.matrix_world.transposed().to_3x3()@Vector((1,0,0))
   bm=bmesh.new();bm.from_mesh(obj.data)
   result=bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),dist=1e-6,plane_co=co,plane_no=normal,clear_inner=keep_right,clear_outer=not keep_right)
   cut_edges=[e for e in result['geom_cut'] if isinstance(e,bmesh.types.BMEdge) and e.is_boundary]
   if cut_edges:bmesh.ops.holes_fill(bm,edges=cut_edges,sides=0)
   bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));topology=dict(nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),zero_area_faces=sum(f.calc_area()<1e-8 for f in bm.faces));bm.to_mesh(obj.data);bm.free()

   actual={signature(obj,p) for p in obj.data.polygons};missing=sum(s not in actual for s in external)
   if missing:raise ValueError(f'Changed {missing} protected outside faces in {original.name}')
   audits.append(dict(source=original.name,covered_faces=len(original.data.polygons),cleared_faces=len(obj.data.polygons),protected_outside_faces=len(external),missing_protected_faces=missing,topology=topology))
   obj['state_recipe']='croisement02-chariot02-barriere-source-domain';obj['reveal_show_when_applied']=[p['id'] for p in patches]
  if before!={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name in before}:raise ValueError('Base changed')
  # Reversible visibility metadata is added only after proving saved base appearance untouched.
  for original in objs:original['reveal_hide_when_applied']=[p['id'] for p in patches]
  for obj in copies:obj.hide_render=True
  bpy.ops.wm.save_as_mainfile(filepath=str(dest/'worker.blend'))
  scene.render.engine='CYCLES';scene.cycles.samples=24
  packet={**frames,'views':[frames['views'][i] for i in (0,2)],'tile_size':[512,512]}
  write(dest/'covered-views.json',packet);render(dest/'covered-views.json',dest/'covered',width=512)
  for obj in objs:obj.hide_render=True
  for obj in copies:obj.hide_render=False
  packet['object_names']=[o.name for o in copies];packet.pop('texture_receiver_object_names',None)
  write(dest/'applied-views.json',packet);render(dest/'applied-views.json',dest/'applied',width=512)
  sheet=Image.new('RGB',(1024,1064),'#ddd');draw=ImageDraw.Draw(sheet)
  for row,state in enumerate(('covered','applied')):
   for col,index in enumerate((0,2)):
    image=Image.open(dest/state/f'view-{index}-textured.png').convert('RGBA')
    sheet.paste(image,(col*512,row*532+20),image)
    draw.text((col*512+8,row*532+5),f'{state}: '+('source camera' if index==0 else 'oblique'),fill='black')
  sheet.save(dest/'comparison.png')
  source=Image.open(OUT/'baseline/covered.png').convert('RGBA');applied=source.copy();applied.alpha_composite(Image.open(img).convert('RGBA'),(x,y))
  context=Image.new('RGB',(800,340),'#ddd');draw=ImageDraw.Draw(context)
  for col,(label,im) in enumerate((('Covered source',source),('Transition / applied source',applied))):
   crop=im.crop((x-80,y-60,x+w+80,y+h+60));crop.thumbnail((400,300));context.paste(crop,(col*400,30));draw.text((col*400+8,8),label,fill='black')
  context.save(dest/'source-comparison.png')
  write(dest/'validation.json',dict(status='GEOMETRY_CANDIDATE',base=str(base),base_sha256=sha(base),source_patch_sha256=sha(img),source_bbox=f['bbox'],patches=[p['id'] for p in patches],outside_faces=audits,approved_base_preserved=True,transition_frames=1,transition_equals_applied_geometry=True,texture_approval='pending',holds=['Ground artwork terminal patch must project onto actual reviewed terrain receiver; no flat substitute added.','New cut end surfaces require appearance review.','Candidate state geometry requires visual review and integration.']))
 finally:release()
if __name__=='__main__':main()
