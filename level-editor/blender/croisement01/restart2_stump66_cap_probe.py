"""Read-only face partition evidence for retaining both native stump references."""
import sys,json
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2';case=R/'approved-stump66-wood-fill-v1/croisement01-south-cut-stump'
model=case/'baked-v1-luminance/worker.blend';out=R/'stump66-cap-partition-probe-v1.json';assert not out.exists()
acquire();bpy.ops.wm.open_mainfile(filepath=str(model));cfg=json.loads((case/'approved-workspace/workspace.json').read_text());obj=next(o for o in bpy.data.collections[cfg['collection_name']].all_objects if o.type=='MESH' and o.get('asset_group')==cfg['asset_id'])
faces=[]
for polygon in obj.data.polygons:
    points=[obj.matrix_world@obj.data.vertices[i].co for i in polygon.vertices]
    if len(points)>4:faces.append(dict(index=polygon.index,vertices=len(points),normal=list((obj.matrix_world.to_3x3().inverted().transposed()@polygon.normal).normalized()),world_z=[min(p.z for p in points),max(p.z for p in points)],area=polygon.area))
assert len(faces)==2
cap=max(faces,key=lambda f:sum(f['world_z']));assert cap['normal'][2]>.5
out.write_text(json.dumps(dict(status='read-only planned export partition; no approved geometry changed',model_sha256=sha(model),source_node=obj.get('source_node'),vertices=len(obj.data.vertices),polygons=len(obj.data.polygons),large_faces=faces,planned_cap_face=cap['index'],planned_body_node='building-056',planned_cap_node='building-067',requirements=['Separate this exact existing top cap face into part067 without moving vertices or UV/material samples.','All other existing faces remain part056; preserve both original native gameplay references and cap walking/projection semantics.','Require exact union-of-surfaces proof, reopened GLB appearance, and complete editor checks before publication.']),indent=2)+'\n')
print(out)
