"""Inspect existing foliage ownership export metadata without changing the worker."""
import sys,json
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
B=ROOT/'level-editor/work/croisement03-refinement/restart2'
acquire()
try:
 bpy.ops.wm.open_mainfile(filepath=str(B/'texture-batch-v7/croisement03-tree-25/experiment/cluster-geometry-v6/worker.blend'));o=next(o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement03-tree-25' and not o.hide_render);d=dict(name=o.name,attributes=[dict(name=a.name,domain=a.domain,data_type=a.data_type) for a in o.data.attributes],active_color=o.data.color_attributes.active_color.name if o.data.color_attributes.active_color else None,active_color_index=o.data.color_attributes.active_color_index,render_color_index=o.data.color_attributes.render_color_index,materials=[dict(name=m.name,foliage=bool(m.get('foliage_physical_opacity')),nodes=[n.type for n in m.node_tree.nodes] if m.use_nodes else []) for m in o.data.materials]);(B/'tree25-cluster-integration-v1/metadata-probe.json').write_text(json.dumps(d,indent=2)+'\n');print(json.dumps(d))
finally:release()
