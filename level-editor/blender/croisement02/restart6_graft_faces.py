import sys,json
from pathlib import Path
import bpy,bmesh
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT
from render_slots import acquire,release
from evidence_io import write_json
acquire()
try:
 out={}
 for tree,version in [(18,2),(39,4)]:
  bpy.ops.wm.open_mainfile(filepath=str(ROOT/f'tree{tree}-continuous-graft-v{version}/model.blend'));rows=[]
  for o in bpy.context.scene.objects:
   if o.type!='MESH'or o.get('asset_group')!=f'croisement02-tree-{tree}'or'Crown'in o.name:continue
   bm=bmesh.new();bm.from_mesh(o.data);unseen=set(bm.verts);components=[]
   while unseen:
    stack=[unseen.pop()];seen=[]
    while stack:
     v=stack.pop();seen.append(v)
     for e in v.link_edges:
      q=e.other_vert(v)
      if q in unseen:unseen.remove(q);stack.append(q)
    components.append(dict(vertices=len(seen),z=[min(v.co.z for v in seen),max(v.co.z for v in seen)]))
   faces=sorted([f for f in bm.faces if 98<f.calc_center_median().z<106],key=lambda f:abs(f.normal.z),reverse=True);rows.append(dict(object=o.name,components=components,graft_faces=[dict(z=f.calc_center_median().z,area=f.calc_area(),normal=list(f.normal),n=len(f.verts),bounds=[min(v.co.z for v in f.verts),max(v.co.z for v in f.verts)])for f in faces[:20]]));bm.free()
  out[tree]=rows
 write_json(ROOT/'graft-face-diagnostic.json',out)
finally:release()
