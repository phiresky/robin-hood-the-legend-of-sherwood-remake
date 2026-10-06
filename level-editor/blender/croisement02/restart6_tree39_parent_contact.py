"""Separate inherited reverse appearance from lower-contour changes."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,SPECS,RAY
from restart4_stump_final_contact import frame,sheet
from render_slots import acquire,release
from evidence_io import write_json,sha
def own():return [o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')=='croisement02-tree-39']
def main():
 source=SPECS[39][0];candidate=ROOT/'tree39-contour-v3/model.blend';out=ROOT/'tree39-parent-contact-v1';out.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();positions={o.name:np.array([o.matrix_world@v.co for v in o.data.vertices])for o in own()};bpy.ops.wm.open_mainfile(filepath=str(candidate));bpy.context.view_layer.update();moved=[]
 for o in own():
  p=np.array([o.matrix_world@v.co for v in o.data.vertices]);moved.extend(p[np.linalg.norm(p-positions[o.name],axis=1)>1e-5])
 bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o.get('asset_group')!='croisement02-tree-39'or'Crown'in o.name
 scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=512;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None';mesh=bpy.data.meshes.new('Same candidate contact framing');mesh.from_pydata(moved,[],[]);proxy=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(proxy);proxy.hide_render=True;center=np.array(moved).mean(0);bpy.ops.mesh.primitive_plane_add(size=220,location=(center[0],center[1],0));floor=bpy.context.object;mat=bpy.data.materials.new('Neutral contact guide');mat.diffuse_color=(.12,.13,.10,1);floor.data.materials.append(mat)
 for i,d in enumerate([RAY,Vector((.6124,-.6124,.5)),Vector((.6124,.6124,.5)),Vector((-.866,0,.5))]):frame(scene,[proxy],d.normalized(),512,1.25);scene.render.filepath=str(out/f'view-{i}.png');bpy.ops.render.render(write_still=True)
 sheet([out/f'view-{i}.png'for i in range(4)],out/'sheet.png');write_json(out/'evidence.json',dict(approved_parent_sha256=sha(source),candidate_framing_sha256=sha(candidate),scope='Unchanged approved parent at exact candidate contact cameras. No model saved.'))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
