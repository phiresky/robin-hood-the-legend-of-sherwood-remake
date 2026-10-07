"""Render native-first shape evidence for every site and complete exceptional orbits."""
from pathlib import Path
import sys,json,hashlib,math
import shutil
import bpy
from mathutils import Vector
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,SIN,COS
from restart4_stump_final_contact import frame,sheet
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 worker=OUT/'restart15-hiding-mounds/all-placements-v1';r=json.loads((worker/'validation.json').read_text());guard=json.loads((worker/'saved-native-guard.json').read_text());assert guard['status']=='PASS';model=worker/'model.blend';assert sha(model)==r['model_sha256']==guard['model_sha256'];out=worker/'shape-review-v1';out.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;objects=[o for o in scene.objects if o.type=='MESH'];gray=bpy.data.materials.new('Opaque geometry review');gray.use_nodes=True;gray.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.65,.65,.65,1);gray.node_tree.nodes['Principled BSDF'].inputs['Roughness'].default_value=.85;light=bpy.data.objects.new('Review area',bpy.data.lights.new('Review area','AREA'));scene.collection.objects.link(light);light.data.energy=6000;light.data.size=35;scene.cycles.samples=16;rows=[];native=[]
 for row in r['records']:
  own=[bpy.data.objects[n]for n in row['objects']];points=[o.matrix_world@Vector(v)for o in own for v in o.bound_box];center=sum(points,Vector())/len(points);extent=[max(p[i]for p in points)-min(p[i]for p in points)for i in range(3)];light.location=center+Vector((30,-55,85));light.rotation_euler=(center-light.location).to_track_quat('-Z','Y').to_euler()
  for obj in objects:obj.hide_render=obj not in own
  heights=[s['height_gain']for s in row['support']];exceptional=max(heights)>1 or max(heights)-min(heights)>.25;indices=range(8)if exceptional else[0,4];images=[]
  for mode in ['actual','coverage-gray']:
   scene.view_layers[0].material_override=gray if mode=='coverage-gray'else None;files=[]
   for i in indices:
    angle=i*math.pi/4;frame(scene,own,Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN)),384,1.2);file=out/f'{row["tag"]}-{mode}-{i:02}.png';scene.render.filepath=str(file);assert shutil.disk_usage(out).free>25*1024**3,'Disk reserve reached before render';bpy.ops.render.render(write_still=True);files.append(file);images.append(dict(mode=mode,view=i,path=file.name,sha256=sha(file)))
    if mode=='actual'and i==0:native.append(file)
   sheet(files,out/f'{row["tag"]}-{mode}-sheet.png')
  rows.append(dict(tag=row['tag'],instance=row['instance'],exceptional=exceptional,world_extent=extent,height_width_ratio=extent[2]/extent[0],support_height_range=[min(heights),max(heights)],images=images))
 sheet(native,out/'all-native.png');(out/'report.json').write_text(json.dumps(dict(model_sha256=sha(model),records=rows,scope='All sites native and reverse actual/opaque gray; full8 for elevated or differential supports. Camera0 is native. No inferred cap texture or user approval.'),indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
