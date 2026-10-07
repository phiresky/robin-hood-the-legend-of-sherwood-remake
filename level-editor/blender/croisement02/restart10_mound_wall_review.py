"""Native-first actual and lit-solid views of flat and sloped leaf-cover volumes."""
from pathlib import Path
import sys,json,hashlib,math
import bpy
from mathutils import Vector
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,SIN,COS
from restart4_stump_final_contact import frame,sheet
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 worker=OUT/'restart9-hiding-scatter/mound-support-variants-v3';out=worker/'eight-view-review-v1';out.mkdir(exist_ok=False);r=json.loads((worker/'validation.json').read_text());model=worker/'model.blend';assert sha(model)==r['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;scene.cycles.samples=16;scene.cycles.transparent_max_bounces=512;solid=bpy.data.materials.new('Lit neutral geometry');solid.use_nodes=True;solid.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.65,.65,.65,1);solid.node_tree.nodes['Principled BSDF'].inputs['Roughness'].default_value=.85;light=bpy.data.objects.new('Geometry key',bpy.data.lights.new('Geometry key','AREA'));scene.collection.objects.link(light);light.data.energy=6000;light.data.size=35;images=[]
 for tag,instance in [('wall-toe','mission-Tac02_FoB_EC-patch-022')]:
  record=next(x for x in r['records']if instance in x['instances']);obj=bpy.data.objects[record['object']]
  for o in scene.objects:
   if o.type=='MESH':o.hide_render=o!=obj
  center=sum((obj.matrix_world@Vector(p)for p in obj.bound_box),Vector())/8;light.location=center+Vector((30,-55,85));light.rotation_euler=(center-light.location).to_track_quat('-Z','Y').to_euler()
  for mode in ['actual','solid']:
   scene.view_layers[0].material_override=solid if mode=='solid'else None;paths=[]
   for index in range(8):
    angle=index*math.pi/4;camera=frame(scene,[obj],Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN)),384,1.3);file=out/f'{tag}-{mode}-{index:02}.png';scene.render.filepath=str(file);bpy.ops.render.render(write_still=True);paths.append(file);images.append(dict(variant=tag,instance=instance,mode=mode,index=index,path=file.name,sha256=sha(file),camera_matrix=[list(x)for x in camera.matrix_world]))
   sheet(paths,out/f'{tag}-{mode}-eight.png')
 (out/'report.json').write_text(json.dumps(dict(model_sha256=sha(model),images=images,scope='Only the corrected wall-toe variant. First camera native in each sheet. All other19 variants remain exactly unchanged.'),indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
