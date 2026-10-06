"""Separate source-space and assembled world-space tree24 geometry differences."""
import sys,json
from pathlib import Path
import bpy,numpy as np
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,OUT
from render_slots import acquire,release
from evidence_io import sha,write_json

def snapshot(path):
 bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update();o=next(o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')=='croisement02-tree-24'and'Crown'not in o.name)
 return dict(local=np.array([v.co[:]for v in o.data.vertices]),world=np.array([o.matrix_world@v.co for v in o.data.vertices]),matrix=[list(r)for r in o.matrix_world],faces=[list(p.vertices)for p in o.data.polygons],edges=[list(e.vertices)for e in o.data.edges],source_node=o.get('source_node'),asset_group=o.get('asset_group'),projection_component=o.get('projection_component'))
acquire()
try:
 source=OUT/'forest-v4-round-1/assets/croisement02-tree-24/model.blend';scene=OUT/'restart2-textures/batch-v3-coherent-scene-v1/scene.blend';a=snapshot(source);b=snapshot(scene);rows={k:a[k]==b[k]for k in ['faces','edges','source_node','asset_group','projection_component']};write_json(ROOT/'tree24-transform-reconciliation-v1.json',dict(source_sha256=sha(source),scene_sha256=sha(scene),local_vertices_exact=np.array_equal(a['local'],b['local']),world_vertices_exact=np.array_equal(a['world'],b['world']),max_local_difference=float(np.linalg.norm(a['local']-b['local'],axis=1).max()),max_world_difference=float(np.linalg.norm(a['world']-b['world'],axis=1).max()),source_matrix=a['matrix'],assembled_matrix=b['matrix'],identities=rows,scope='Read-only reopened source versus frozen scene; no conversion or geometry changes.'))
finally:release()
