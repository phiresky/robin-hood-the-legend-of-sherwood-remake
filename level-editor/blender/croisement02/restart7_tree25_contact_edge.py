"""Retain two original native foot centers after ground clipping."""
import sys,json,math
from pathlib import Path
import bpy
from mathutils import Vector
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,OUT,SIN,COS
from evidence_io import sha,write_json
from render_slots import acquire,release
acquire()
try:
 parent=ROOT/'tree25-toe-native-v3/model.blend';out=ROOT/'tree25-toe-native-v4';out.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(parent));bpy.context.view_layer.update();changed=[];item=next(x for x in json.load(open(OUT/'review-mask-inventory.json'))['masks']if x['index']==25);ox,oy=item['box_top_left'];w,h=item['box_size']
 for o in bpy.context.scene.objects:
  if o.type!='MESH'or o.get('projection_component')=='crown':continue
  inv=o.matrix_world.inverted()
  for v in o.data.vertices:
   p=o.matrix_world@v.co;sy=-p.y*SIN-p.z*COS;weight=math.exp(-((p.x-127)/5)**2)*max(0,min(1,(sy-951)/2))*max(0,min(1,(4-p.z)/3))
   if weight>0:p.y-=.85*weight/SIN;v.co=inv@p;changed.append(v.index)
  o.data.update();uv=o.data.uv_layers['Continuous toe native projection']
  for li,loop in enumerate(o.data.loops):
   p=o.matrix_world@o.data.vertices[loop.vertex_index].co;uv.data[li].uv=((p.x-ox)/w,1-(-p.y*SIN-p.z*COS-oy)/h)
 bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True);write_json(out/'contact-edge.json',dict(parent_sha256=sha(parent),model_sha256=sha(out/'model.blend'),changed_vertices=len(changed),max_projected_extension=.85,scope='Bounded same-height contact footprint extension to retain original native centers126/127,954. No new ambiguous foreground target.'))
finally:release()
