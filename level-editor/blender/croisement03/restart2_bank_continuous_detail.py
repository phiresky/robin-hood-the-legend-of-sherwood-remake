"""Saved trace/crest identity guard and native-first close views of continuous strata."""
import sys,math,json
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
R=Path(__file__).resolve().parents[3];sys.path[:0]=[str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from render_slots import acquire,release
from render_views import render_views
from evidence_io import sha,write_json
B=R/'level-editor/work/croisement03-refinement/restart2';O=B/'bank-continuous-strata-v1';S=math.sin(math.radians(35));C=math.cos(math.radians(35))

def shape(o):return dict(vertices=[list(v.co) for v in o.data.vertices],faces=[list(f.vertices) for f in o.data.polygons],world=[list(r) for r in o.matrix_world])

def main():
 out=O/'west-detail';out.mkdir(exist_ok=False);acquire()
 try:
  model=O/'worker.blend';digest=sha(model);bpy.ops.wm.open_mainfile(filepath=str(model));s=bpy.context.scene;current={n:shape(s.objects[f'Candidate bank {n}']) for n in (52,54)};reference=B/'bank-full-prototype-v2/worker.blend';referencehash=sha(reference)
  with bpy.data.libraries.load(str(reference),link=False) as (a,b):b.objects=['Candidate bank 52','Candidate bank 54']
  for n,o in zip((52,54),b.objects):assert shape(o)==current[n],n
  obj=s.objects['Candidate bank 53'];points=[obj.matrix_world@v.co for v in obj.data.vertices];array=np.array([list(v) for v in points]);traces=[]
  for shelf in json.loads((B/'bank-morphology-trace-v1/recipe.json').read_text())['shelves']:
   for ring in shelf['proposed_short_face_rings']:
    for key,source in [('upper_world','source_rim'),('lower_world','source_lower')]:
     target=np.array(ring[key]);i=int(np.argmin(np.linalg.norm(array-target,axis=1)));p=array[i];error=max(abs(p[0]-ring[source][0]),abs(-p[1]*S-p[2]*C-ring[source][1]));assert error<1e-4;traces.append(dict(source=ring[source],vertex=i,max_source_error=float(error)))
  lo=Vector(tuple(min(p[i] for p in points) for i in range(3)));hi=Vector(tuple(max(p[i] for p in points) for i in range(3)));target=(lo+hi)/2;views={};required=[];allpoints=[o.matrix_world@v.co for o in s.objects if o.type=='MESH' for v in o.data.vertices]
  for i in range(8):
   az=i*math.tau/8;direction=Vector((math.sin(az)*C,-math.cos(az)*C,S));data=bpy.data.cameras.new(f'West detail{i}');data.type='ORTHO';camera=bpy.data.objects.new(data.name,data);s.collection.objects.link(camera);camera.location=target+direction*1500;camera.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();bpy.context.view_layer.update();inverse=camera.matrix_world.inverted();local=[inverse@p for p in points];required.append(2*max(max(abs(p.x),abs(p.y)) for p in local)*1.2);depth=[-(inverse@p).z for p in allpoints];data.clip_start=min(depth)-25;data.clip_end=max(depth)+25;views[f'view-{i}']=camera.name
  for name in views.values():s.objects[name].data.ortho_scale=max(required)
  s.render.threads_mode='FIXED';s.render.threads=2;s.cycles.samples=8;render_views(s.name,views,out/'views',modes=('textured','solid'),width=384)
  for mode in ['textured','solid']:
   sheet=Image.new('RGB',(1536,768),'#333333')
   for i in range(8):
    im=Image.open(out/'views'/f'view-{i}-{mode}.png').convert('RGBA');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),((i%4)*384,(i//4)*384))
   sheet.save(out/f'{mode}8.png')
  assert sha(model)==digest and sha(reference)==referencehash;write_json(out/'receipt.json',dict(status='PASS saved trace coordinates and unchanged bank52/fixed crest/ramp54 geometry; close views require self-review',model_sha256=digest,reference_model_sha256=referencehash,unchanged_geometry_nodes=[52,54],source_traces=traces,max_source_error=max(t['max_source_error'] for t in traces),native_view_index=0,images={p.name:sha(p) for p in out.glob('*.png')},limits=['Close views frame the full western body. Surrounding bank and path remain cropped context; full-bank8 sheets retain whole extent.','Neither this guard nor source traces approve inferred shoulder depths or appearance.']))
 finally:release()
if __name__=='__main__':main()
