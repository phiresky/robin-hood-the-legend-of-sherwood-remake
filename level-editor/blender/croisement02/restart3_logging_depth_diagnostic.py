"""Measure the three newly hidden native foliage samples without model edits."""
import json,sys
from pathlib import Path
import bpy
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,RAY
from refinement_review import _tree
from stage_review_scene import signature

def main():
    directory=OUT/'leaf-clump-joint-review/restart2-logging-root-4ff3e937'
    data=json.loads((directory/'evidence.json').read_text())
    bpy.ops.wm.read_factory_settings(use_empty=True)
    for row in data['inputs']:
        model=Path(row['workspace'])/'model.blend';assert sha(model)==row['model_sha256']
        with bpy.data.libraries.load(str(model),link=False) as (_,loaded): loaded.objects=[o['name'] for o in row['objects']]
        for obj in loaded.objects:
            bpy.context.scene.collection.objects.link(obj);parent=obj.parent
            while parent:
                if not parent.users_collection:bpy.context.scene.collection.objects.link(parent)
                parent=parent.parent
        bpy.context.view_layer.update()
        for obj,expected in zip(loaded.objects,row['objects']):
            assert signature(obj)==expected['signature']
            mat=obj.matrix_world.copy();obj.parent=None;obj.matrix_world=mat
    objects=[o for o in bpy.context.scene.objects if o.type=='MESH']
    groups={asset:[o for o in objects if o.get('asset_group')==asset] for asset in ['croisement02-logging-clearing-log','croisement02-shrub-86']}
    trees={a:_tree(obs)[:2] for a,obs in groups.items()}
    combined,combined_owners,_=_tree(objects)
    rows=[]
    for x,y in [[1404,306],[1390,314],[1444,317],[1437,304]]:
        hits={}
        origin=Vector((x+.5,-(y+.5)/SIN,0))+RAY*5000
        for asset,(tree,owners) in trees.items():
            hit,normal,index,distance=tree.ray_cast(origin,-RAY)
            hits[asset]=None if hit is None else dict(object=owners[index].name,location=list(hit),distance=distance,triangle=index)
        ch,cn,ci,cd=combined.ray_cast(origin,-RAY)
        winner=min(((a,h['distance']) for a,h in hits.items() if h),key=lambda r:r[1])[0]
        rows.append(dict(pixel=[x,y],hits=hits,independent_nearest=winner,combined=None if ch is None else dict(object=combined_owners[ci].name,location=list(ch),distance=cd)))
    target=OUT/'restart3-logging/neighbor-depth-v2';target.mkdir(parents=True,exist_ok=True)
    write_json(target/'report.json',dict(source_evidence_sha256=sha(directory/'evidence.json'),rows=rows,ray=list(RAY),status='Read-only exact participant depth diagnostic'))
    print(json.dumps(rows))

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
