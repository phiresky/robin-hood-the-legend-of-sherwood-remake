"""Diagnose a cap-free continuous exterior and locally blended upper graft."""
import sys,json
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector,Matrix
from PIL import Image,ImageDraw
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,RAY
from render_slots import acquire,release
from evidence_io import sha,write_json
from render_views import render_views

def repair():
 wood=[o for o in bpy.context.scene.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-tree-39' and 'Crown'not in o.name]
 for o in wood:
  bm=bmesh.new();bm.from_mesh(o.data);caps=[f for f in bm.faces if all(abs(v.co.z-70)<.001 for v in f.verts)or all(abs(v.co.x-1705)<.001 for v in f.verts)];bmesh.ops.delete(bm,geom=caps,context='FACES');layer=bm.faces.layers.int.new('source_building_id')
  for f in bm.faces:f[layer]=int(o['source_node'].split('-')[-1])
  bm.to_mesh(o.data);bm.free();o.data.update()
 bpy.ops.object.select_all(action='DESELECT')
 for o in wood:o.select_set(True)
 obj=next(o for o in wood if o['source_node']=='building-095');bpy.context.view_layer.objects.active=obj;bpy.ops.object.join();obj['source_nodes']=['building-095','building-096','building-097'];bm=bmesh.new();bm.from_mesh(obj.data);boundary=[v for v in bm.verts if v.is_boundary];bmesh.ops.remove_doubles(bm,verts=boundary,dist=.001)
 before=dict(nonmanifold=sum(not e.is_manifold for e in bm.edges),verts=len(bm.verts),faces=len(bm.faces));movable=[v for v in bm.verts if 88<v.co.z<110];weights={v:np.sin(np.pi*(v.co.z-88)/22)**2 for v in movable}
 for _ in range(45):
  shifts={v:sum((e.other_vert(v).co for e in v.link_edges),Vector())/len(v.link_edges)-v.co for v in movable}
  for v,d in shifts.items():v.co+=d*(.3*weights[v])
 bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));checks=dict(nonmanifold=sum(not e.is_manifold for e in bm.edges),degenerate=sum(f.calc_area()<1e-9 for f in bm.faces));bm.to_mesh(obj.data);bm.free();obj.data.update()
 # Clear historical split normals; the continuous exterior defines its own normals.
 obj.data.normals_split_custom_set_from_vertices([v.normal for v in obj.data.vertices])
 for f in obj.data.polygons:f.use_smooth=True
 return obj,dict(before=before,after=checks,blended_vertices=len(movable))

def main():
 out=ROOT/'tree39-weld-diagnostic-v1';out.mkdir(exist_ok=False);source=ROOT/'tree39-continuous-graft-v4/model.blend';bpy.ops.wm.open_mainfile(filepath=str(source));obj,checks=repair();scene=bpy.context.scene
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o!=obj
 info=json.load(open(ROOT/'tree39-continuous-solid-delta-v1/evidence.json'));views={}
 for i,data in enumerate(info['same_cameras']):
  cam=bpy.data.objects.new(str(i),bpy.data.cameras.new(str(i)));scene.collection.objects.link(cam);cam.data.type='ORTHO';cam.data.ortho_scale=data['scale'];cam.data.clip_end=20000;cam.matrix_world=Matrix(data['matrix']);views[f'view-{i}']=cam.name
 scene.display.shading.light='STUDIO';scene.display.shading.color_type='SINGLE';scene.display.shading.single_color=(.65,.65,.65);scene.display.shading.show_cavity=True;scene.display.shading.cavity_type='BOTH';render_views(scene.name,views,out,modes=('solid',),width=512)
 im=Image.new('RGB',(2048,512))
 for i in range(4):im.paste(Image.open(out/f'view-{i}-solid.png').convert('RGB'),(i*512,0))
 im.save(out/'sheet.png');write_json(out/'audit.json',dict(source_sha256=sha(source),topology=checks,scope='Diagnostic only, no saved model'))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
