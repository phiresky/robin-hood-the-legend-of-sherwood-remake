"""Reopen exact root joint participants; audit source-node and group domains."""
import argparse,json,sys
from pathlib import Path
from collections import Counter
import bpy
import numpy as np
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,RAY
from refinement_review import _tree
from audit_scene_first_hit import full_mask
from stage_review_scene import signature


def main(kind):
    logging=kind=='logging';asset='croisement02-logging-clearing-log' if logging else 'croisement02-southwest-stumps'
    prefix='4ff3e937' if logging else 'b650c6da';added='Logging root and branch tangle' if logging else 'Southwest stump root and branch tangle'
    directory=OUT/'leaf-clump-joint-review'/('restart2-'+kind+'-root-'+prefix);evidence=directory/'evidence.json';data=json.loads(evidence.read_text())
    bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene
    for row in data['inputs']:
        model=Path(row['workspace'])/'model.blend';assert sha(model)==row['model_sha256']
        with bpy.data.libraries.load(str(model),link=False)as(_,loaded):loaded.objects=[o['name']for o in row['objects']]
        for obj in loaded.objects:
            assert obj is not None;scene.collection.objects.link(obj);parent=obj.parent
            while parent:
                if not parent.users_collection:scene.collection.objects.link(parent)
                parent=parent.parent
        bpy.context.view_layer.update()
        for obj,expected in zip(loaded.objects,row['objects']):
            assert signature(obj)==expected['signature'],obj.name
            matrix=obj.matrix_world.copy();obj.parent=None;obj.matrix_world=matrix;obj.hide_render=False
    objects=[o for o in scene.objects if o.type=='MESH'];assert sum(o.name==added for o in objects)==1
    after,owners,_=_tree(objects);before,before_owners,_=_tree([o for o in objects if o.name!=added])
    def owner(tree,names,x,y):
        hit,normal,index,distance=tree.ray_cast(Vector((float(x)+.5,-(float(y)+.5)/SIN,0))+RAY*5000,-RAY)
        return names[index].get('asset_group') if hit is not None else None
    prior=OUT/'restart2-vegetation'/('logging-branches-v5'if logging else'southwest-branches-v1')/'proposal.json';samples=json.loads(prior.read_text())['hits'];target_rows=[]
    for row in samples:
        x,y=row['pixel'];target_rows.append(dict(pixel=[x,y],first_asset=owner(after,owners,x,y),previous_private_gap=row['object']is None))
    guards=[]
    for row in data['inputs'][1:]:
        context=Path(row['workspace']);cfg=json.loads((context/'source-masks.json').read_text());parts=set(json.loads((context/'workspace.json').read_text())['part_ids']);inventory_path=Path(cfg['mask_inventory']);lookup={r['index']:r for r in json.loads(inventory_path.read_text())['masks']}
        assignments=[r for r in cfg['projections']['exterior']['assignments']if r.get('asset_group')==context.name or r.get('source_node')in parts]
        if not assignments:raise ValueError('Missing own source domains: '+str(context))
        domain=np.zeros((1152,1792),bool)
        for assignment in assignments:
            own=np.zeros_like(domain)
            for index in assignment['mask_indices']:own|=full_mask(lookup[index],inventory_path)
            for index in assignment.get('exclude_mask_indices',[]):own&=~full_mask(lookup[index],inventory_path)
            domain|=own
        visible=0;blocked=[]
        for y,x in zip(*np.nonzero(domain)):
            if owner(before,before_owners,x,y)!=context.name:continue
            visible+=1
            if owner(after,owners,x,y)==asset:blocked.append([int(x),int(y)])
        guards.append(dict(asset=context.name,source_manifest_sha256=sha(context/'source-masks.json'),native_domain_pixels=int(domain.sum()),baseline_visible=visible,new_root_blocks=blocked))
    points=[o.matrix_world@v.co for o in objects if o.get('asset_group')==asset for v in o.data.vertices]
    counts=Counter(r['first_asset']for r in target_rows)
    write_json(directory/'physical-source-guards-v2.json',dict(status='Measured finite source visibility; exact original prop retained in baseline',joint_evidence_sha256=sha(evidence),inputs=data['inputs'],target_counts=dict(counts),target_rows=target_rows,neighbors=guards,minimum_world_z=min(p.z for p in points),limitations=['Only additive root mesh is absent from baseline; original approved log/stumps remain.', 'Source center rays and declared opacity/one-sided materials are sampled; not an exhaustive collision proof.']))
    print(dict(target_counts=dict(counts),new_neighbor_blocks={r['asset']:len(r['new_root_blocks'])for r in guards}))


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('kind',choices=['logging','southwest']);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);acquire()
    try:main(args.kind)
    finally:release()
