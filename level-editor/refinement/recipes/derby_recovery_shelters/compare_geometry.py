import bpy, json, sys, hashlib
from pathlib import Path
ROOT=Path.cwd()
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from refinement_workspace import _geometry
base=ROOT/'level-editor/work/needs-review/derby-refinement/round-2'
jobs=['sunburst-south-curtain-lean-to-approved','sunburst-east-wall-lean-to-approved','sunburst-derby-lower-northwest-cottage-barrel-approved']
out={}
for job in jobs:
 m=json.loads((base/job/'views.json').read_text())
 live=[o for o in bpy.data.collections['Derby Working'].all_objects if o.type=='MESH' and o.get('asset_group')==m['asset_id']]
 with bpy.data.libraries.load(m['source_blend'],link=False) as (a,b): b.objects=m['object_names']
 old=b.objects
 for o in old:bpy.context.scene.collection.objects.link(o)
 bpy.context.view_layer.update()
 def record(o):
  return {'name':o.name,'node':o.get('source_node'),'hidden':o.hide_render,'component':o.get('projection_component'),'geometry':hashlib.sha256(json.dumps({'vertices':[list(v.co) for v in o.data.vertices],'faces':[list(p.vertices) for p in o.data.polygons],'matrix':[list(r) for r in o.matrix_world]},sort_keys=True).encode()).hexdigest()}
 out[m['asset_id']]={'live':[record(o) for o in live],'approved':[record(o) for o in old]}
 for o in old:bpy.data.objects.remove(o,do_unlink=True)
(base/'recovery-shelters-20260923/geometry-comparison.json').write_text(json.dumps(out,indent=2))
print(json.dumps(out))
