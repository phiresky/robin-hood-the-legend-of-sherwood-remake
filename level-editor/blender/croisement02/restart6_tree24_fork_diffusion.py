"""Remove the inherited narrow sleeve grooves with area-aware local smoothing."""
import sys
from pathlib import Path
import bpy,numpy as np
from scipy.sparse import coo_matrix,diags
from scipy.sparse.linalg import spsolve
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_tree24_contour import ROOT
from evidence_io import sha,write_json
from render_slots import acquire,release
acquire()
try:
 source=ROOT/'tree24-fork-union-v4/model.blend';out=ROOT/'tree24-fork-union-v5';out.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();obj=next(o for o in bpy.context.scene.objects if o.type=='MESH'and'Crown'not in o.name);mesh=obj.data;mesh.calc_loop_triangles();p=np.array([obj.matrix_world@v.co for v in mesh.vertices]);initial=p.copy();faces=np.array([t.vertices[:]for t in mesh.loop_triangles]);edges=np.array([e.vertices[:]for e in mesh.edges]);a,b=edges.T;free=(initial[:,2]>100)&(initial[:,2]<150);indices=np.flatnonzero(free);fixed=np.flatnonzero(~free)
 for _ in range(3):
  area=np.linalg.norm(np.cross(p[faces[:,1]]-p[faces[:,0]],p[faces[:,2]]-p[faces[:,0]]),axis=1)/2;mass=np.zeros(len(p));np.add.at(mass,faces.ravel(),np.repeat(area/3,3));weights=np.ones(len(edges));degree=np.bincount(edges.ravel(),weights=np.repeat(weights,2),minlength=len(p));lap=diags(degree)-coo_matrix((np.r_[weights,weights],(np.r_[a,b],np.r_[b,a])),shape=(len(p),len(p))).tocsr();strength=1.1*np.minimum(1,np.minimum((initial[:,2]-100)/7,(150-initial[:,2])/7));strength=np.maximum(strength,0);matrix=diags(mass)+diags(strength)@lap;rhs=mass[:,None]*p;rhs=rhs[indices]-matrix[indices][:,fixed]@p[fixed];p[indices]=spsolve(matrix[indices][:,indices].tocsc(),rhs)
 inverse=obj.matrix_world.inverted()
 from mathutils import Vector
 for i in indices:mesh.vertices[int(i)].co=inverse@Vector(p[i])
 mesh.update();mesh.normals_split_custom_set([(0,0,0)]*len(mesh.loops));bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True);write_json(out/'diffusion.json',dict(source_sha256=sha(source),model_sha256=sha(out/'model.blend'),max_movement=float(np.linalg.norm(p-initial,axis=1).max()),scope='Local area-aware smoothing insideZ100..150; outside vertices/UV/material assignments unchanged. Guards pending.'))
finally:release()
