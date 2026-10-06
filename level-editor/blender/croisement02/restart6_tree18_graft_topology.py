"""Locate nonmanifold Boolean/split residues before any review handoff."""
import sys
from pathlib import Path
import bpy,bmesh
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT
from render_slots import acquire,release
from evidence_io import write_json
acquire()
try:
 bpy.ops.wm.open_mainfile(filepath=str(ROOT/'tree18-continuous-graft-v1/model.blend'));rows=[]
 for o in bpy.context.scene.objects:
  if o.type!='MESH'or'Crown'in o.name:continue
  bm=bmesh.new();bm.from_mesh(o.data);rows.append(dict(object=o.name,edges=[dict(vertices=[list(o.matrix_world@v.co)for v in e.verts],length=e.calc_length(),faces=len(e.link_faces),areas=[f.calc_area()for f in e.link_faces])for e in bm.edges if not e.is_manifold],degenerate_faces=sum(f.calc_area()<1e-9 for f in bm.faces)));bm.free()
 write_json(ROOT/'tree18-continuous-graft-v1/topology-audit.json',rows)
finally:release()
