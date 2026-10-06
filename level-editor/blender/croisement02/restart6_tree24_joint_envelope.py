"""Remove residual sleeve undercuts with an interpolated own-trunk envelope."""
import sys
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_tree24_contour import ROOT
from evidence_io import sha,write_json
from render_slots import acquire,release
acquire()
try:
 source=ROOT/'tree24-fork-union-v6/model.blend';out=ROOT/'tree24-fork-union-v7';out.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();obj=next(o for o in bpy.context.scene.objects if o.type=='MESH'and'Crown'not in o.name);p=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);old=p.copy()
 # A conservative circular envelope follows the adjacent source-supported shaft.
 z=p[:,2];center=np.column_stack((8.1+(z-104)*.11,np.full(len(p),-1652.65)));delta=p[:,:2]-center;radius=np.linalg.norm(delta,axis=1);zone=(z>104)&(z<124);weight=np.maximum(0,np.sin(np.pi*np.clip((z-104)/20,0,1)))**.6;minimum=7.7-.035*(z-104);increase=np.maximum(minimum-radius,0)*weight;valid=zone&(radius>.0001);p[valid,:2]+=delta[valid]/radius[valid,None]*increase[valid,None]
 # Retain an actual supporting twig thickness through the former cap junction.
 center=np.array([2.1,-1652.5]);delta=p[:,:2]-center;radius=np.linalg.norm(delta,axis=1);zone=(z>130)&(z<150)&(p[:,0]<5.5);weight=np.maximum(0,np.sin(np.pi*np.clip((z-130)/20,0,1)))**.5;increase=np.maximum(1.7-radius,0)*weight;valid=zone&(radius>.0001);p[valid,:2]+=delta[valid]/radius[valid,None]*increase[valid,None]
 inverse=obj.matrix_world.inverted();moved=np.linalg.norm(p-old,axis=1)>1e-8
 for i in np.flatnonzero(moved):obj.data.vertices[int(i)].co=inverse@Vector(p[i])
 obj.data.update();obj.data.normals_split_custom_set([(0,0,0)]*len(obj.data.loops));bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True);write_json(out/'envelope.json',dict(source_sha256=sha(source),model_sha256=sha(out/'model.blend'),moved=int(moved.sum()),max_movement=float(np.linalg.norm(p-old,axis=1).max()),scope='Own adjacent trunk/branch envelope only; local inferred join thickness, source guard pending.'))
finally:release()
