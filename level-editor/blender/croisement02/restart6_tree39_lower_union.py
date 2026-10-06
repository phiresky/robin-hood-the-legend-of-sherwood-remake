"""Private lower union experiment retaining the upper support and crown."""
import sys,json,shutil
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector,Matrix
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,OUT,RAY,SIN,COS
from restart6_tree39_contour import covered
from restart4_stump_final_contact import frame,sheet
from render_views import render_views
from render_slots import acquire,release
from evidence_io import sha,write_json

def main():
 out=ROOT/'tree39-lower-union-v1';out.mkdir(exist_ok=False);source=ROOT/'tree39-contour-v6/model.blend';bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();scene=bpy.context.scene;wood=[o for o in scene.objects if o.type=='MESH'and 'Crown'not in o.name];assert len(wood)==3;bm=bmesh.new()
 for o in wood:
  mesh=o.data.copy();mesh.transform(o.matrix_world);bm.from_mesh(mesh);bpy.data.meshes.remove(mesh)
 bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),plane_co=(0,0,110),plane_no=(0,0,1),dist=.0001,clear_outer=True);bmesh.ops.holes_fill(bm,edges=[e for e in bm.edges if e.is_boundary],sides=0);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));mesh=bpy.data.meshes.new('Closed lower original union');bm.to_mesh(mesh);bm.free();obj=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(obj);bpy.ops.object.select_all(action='DESELECT');obj.select_set(True);bpy.context.view_layer.objects.active=obj
 modifier=obj.modifiers.new('Resolve stacked lower collars','REMESH');modifier.mode='VOXEL';modifier.voxel_size=.55;modifier.use_smooth_shade=True;bpy.ops.object.modifier_apply(modifier=modifier.name)
 modifier=obj.modifiers.new('Relax lower collar ridges','SMOOTH');modifier.factor=.8;modifier.iterations=45;bpy.ops.object.modifier_apply(modifier=modifier.name)
 bm=bmesh.new();bm.from_mesh(obj.data);bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),plane_co=(0,0,104),plane_no=(0,0,1),dist=.0001,clear_outer=True);bmesh.ops.holes_fill(bm,edges=[e for e in bm.edges if e.is_boundary],sides=0);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(obj.data);bm.free();obj.data.update();verts=np.array([v.co[:]for v in obj.data.vertices]);faces=np.array([list(p.vertices)for p in obj.data.polygons],dtype=object);np.savez_compressed(out/'lower.npz',vertices=verts,faces=faces)
 for o in scene.objects:
  if o.type=='MESH'and o!=obj:o.hide_render=True
 # Keep the original upper surface intact in the diagnostic, with a small
 # overlap reserved for a later exact union, not an accepted final seam.
 upper=wood[0].copy();upper.data=wood[0].data.copy();upper.parent=None;upper.matrix_world=wood[0].matrix_world.copy();scene.collection.objects.link(upper);upper.hide_render=False;bm=bmesh.new();bm.from_mesh(upper.data);bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),plane_co=upper.matrix_world.inverted()@Vector((0,0,100)),plane_no=(0,0,1),dist=.0001,clear_inner=True);bmesh.ops.holes_fill(bm,edges=[e for e in bm.edges if e.is_boundary],sides=0);bm.to_mesh(upper.data);bm.free();upper.data.update()
 info=json.loads((ROOT/'tree39-solid-delta-v1/evidence.json').read_text());views={};scene.render.resolution_x=512;scene.render.resolution_y=512;scene.display.shading.light='STUDIO';scene.display.shading.color_type='SINGLE';scene.display.shading.single_color=(.65,.65,.65);scene.display.shading.show_cavity=True;scene.display.shading.cavity_type='BOTH'
 for i,c in enumerate(info['same_cameras']):
  cam=bpy.data.objects.new(f'Lower union {i}',bpy.data.cameras.new(f'Lower union {i}'));scene.collection.objects.link(cam);cam.data.type='ORTHO';cam.data.ortho_scale=c['scale'];cam.data.clip_end=20000;cam.matrix_world=Matrix(c['matrix']);views[f'view-{i}']=cam.name
 render_views(scene.name,views,out/'solid',modes=('solid',),width=512);sheet([out/f'solid/view-{i}-solid.png'for i in range(4)],out/'sheet.png');bm=bmesh.new();bm.from_mesh(obj.data);edges={str(n):sum(len(e.link_faces)==n for e in bm.edges)for n in range(5)};bm.free();write_json(out/'evidence.json',dict(parent_sha256=sha(source),vertices=len(verts),polygons=len(faces),edge_face_counts=edges,scope='Private lower union shape experiment only; upper join/source coverage/material transfer pending. No selected model saved.'))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
