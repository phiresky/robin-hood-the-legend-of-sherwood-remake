import sys
from pathlib import Path
import bpy,numpy as np
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_tree24_contour import ROOT
from render_slots import acquire,release
acquire()
try:
 bpy.ops.wm.open_mainfile(filepath=str(ROOT/'tree24-fork-union-v6/model.blend'));bpy.context.view_layer.update();o=next(o for o in bpy.context.scene.objects if o.type=='MESH'and'Crown'not in o.name);p=np.array([o.matrix_world@v.co for v in o.data.vertices])
 for z in [100,104,108,110,112,114,118,125,135,140,145,150,155]:
  q=p[abs(p[:,2]-z)<.5]
  if len(q):print(z,len(q),q.min(0).tolist(),q.max(0).tolist(),flush=True)
finally:release()
