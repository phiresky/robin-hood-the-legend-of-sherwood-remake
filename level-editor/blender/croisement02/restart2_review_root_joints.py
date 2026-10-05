"""Current-neighbor root completion review and finite physical source guards."""
import argparse
import json
import sys
from pathlib import Path
from collections import Counter
import bpy
import numpy as np
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
import inspect_leaf_clump_joint as joint
from catalog import OUT,scenery_workspace
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,RAY
from refinement_review import _tree
from audit_scene_first_hit import full_mask


def main(kind):
    logging=kind=='logging';asset='croisement02-logging-clearing-log' if logging else 'croisement02-southwest-stumps'
    worker=OUT/'restart2-vegetation'/(kind+'-root-package-v1')/'assets'/asset
    expected='4ff3e9373996f57e01cbb1f03a80398bccab89bf0d1af93410f66216c9adf91f' if logging else 'b650c6daa46be56eb58390a9c407a39153eca5a950044bf9e990f4554cbeea7a'
    assert sha(worker/'model.blend')==expected
    names=['shrub-86','logging-clearing-stumps','north-kindling-bundle'] if logging else ['shrub-79','shrub-81','southwest-rock-outcrop','southwest-field-wattle-fence','southwest-log-pile']
    contexts=[scenery_workspace('croisement02-'+name) for name in names]
    joint.worker=lambda index:worker
    label='restart2-'+kind+'-root-'+expected[:8]
    joint.run(label,[28 if logging else 123],False,contexts,transparent_bounces=256)
    directory=OUT/'leaf-clump-joint-review'/label
    objects=[o for o in bpy.context.scene.objects if o.type=='MESH' and not o.hide_render]
    after,owners,_=_tree(objects)
    before,before_owners,_=_tree([o for o in objects if o.get('asset_group')!=asset])

    def owner(tree,names,x,y):
        hit,normal,index,distance=tree.ray_cast(Vector((float(x)+.5,-(float(y)+.5)/SIN,0))+RAY*5000,-RAY)
        return names[index].get('asset_group') if hit is not None else None

    prior=OUT/'restart2-vegetation'/('logging-branches-v5' if logging else 'southwest-branches-v1')/'proposal.json'
    samples=json.loads(prior.read_text())['hits'];target_rows=[]
    for row in samples:
        x,y=row['pixel'];target_rows.append(dict(pixel=[x,y],first_asset=owner(after,owners,x,y),previous_private_gap=row['object'] is None))
    guards=[]
    for context in contexts:
        cfg=json.loads((context/'source-masks.json').read_text());inventory_path=Path(cfg['mask_inventory']);inventory=json.loads(inventory_path.read_text());lookup={r['index']:r for r in inventory['masks']}
        assignment=next(r for r in cfg['projections']['exterior']['assignments'] if r.get('asset_group')==context.name)
        domain=np.zeros((1152,1792),bool)
        for index in assignment['mask_indices']:domain|=full_mask(lookup[index],inventory_path)
        for index in assignment.get('exclude_mask_indices',[]):domain&=~full_mask(lookup[index],inventory_path)
        visible=0;blocked=[]
        for y,x in zip(*np.nonzero(domain)):
            original_owner=owner(before,before_owners,x,y)
            if original_owner!=context.name:continue
            visible+=1;new_owner=owner(after,owners,x,y)
            if new_owner==asset:blocked.append([int(x),int(y)])
        guards.append(dict(asset=context.name,source_manifest_sha256=sha(context/'source-masks.json'),native_domain_pixels=int(domain.sum()),baseline_visible=visible,new_root_blocks=blocked))
    points=[o.matrix_world@v.co for o in objects if o.get('asset_group')==asset for v in o.data.vertices]
    counts=Counter(r['first_asset'] for r in target_rows)
    write_json(directory/'physical-source-guards.json',dict(status='Measured finite physical-alpha source comparisons; manual review required',model_sha256=expected,joint_evidence_sha256=sha(directory/'evidence.json'),target_counts=dict(counts),target_rows=target_rows,neighbors=guards,minimum_world_z=min(p.z for p in points),floor='Neutral z0 contact guide, not painted source coverage',limitations=['Baseline neighbor visibility is measured with new root asset absent; preserved original prop is included in candidate.','Only native source centers are sampled; not an exhaustive collision proof.','Unknown rear bark remains pending texture completion.']))
    print(dict(target_counts=dict(counts),new_neighbor_blocks={r['asset']:len(r['new_root_blocks']) for r in guards}))


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('kind',choices=['logging','southwest']);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    acquire()
    try:main(args.kind)
    finally:release()
