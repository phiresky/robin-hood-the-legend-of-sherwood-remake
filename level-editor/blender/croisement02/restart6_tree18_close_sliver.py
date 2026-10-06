"""Close a microscopic floor-cap boundary in the private continuous tree18."""
import sys
from pathlib import Path
import bpy,bmesh
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT
from render_slots import acquire,release
from evidence_io import sha,write_json

def main():
 source=ROOT/'tree18-continuous-graft-v1/model.blend';out=ROOT/'tree18-continuous-graft-v2';out.mkdir(exist_ok=False)
 bpy.ops.wm.open_mainfile(filepath=str(source));o=next(o for o in bpy.context.scene.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-tree-18' and 'Crown'not in o.name)
 bm=bmesh.new();bm.from_mesh(o.data);edges=[e for e in bm.edges if e.is_boundary];assert len(edges)==3
 tiny=min(edges,key=lambda e:e.calc_length());assert tiny.calc_length()<.0001 and max(v.co.z for v in tiny.verts)<.151
 before=[list(v.co)for v in tiny.verts];bmesh.ops.remove_doubles(bm,verts=list(tiny.verts),dist=.0001);bm.normal_update();checks=dict(nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-9 for f in bm.faces));assert checks['nonmanifold_edges']==0 and checks['degenerate_faces']==0,checks
 bm.to_mesh(o.data);bm.free();o.data.update();bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True);write_json(out/'construction.json',dict(parent_sha256=sha(source),model_sha256=sha(out/'model.blend'),merged_floor_vertices=before,topology=checks,scope='Only microscopic ground cap duplicate welded. No upper/crown edits.'))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
