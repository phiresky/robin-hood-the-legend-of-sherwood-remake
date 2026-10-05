"""Project scoped boundary derivatives with frozen crown/outside appearances."""
import argparse,json,sys
from pathlib import Path
import bpy,numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import prepare,modified,validate,_geometry
from audit_candidates import audit
from render_tree import render_workspace


def main():
    parser=argparse.ArgumentParser();parser.add_argument('index',type=int,choices=[35,43,45,46]);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);index=args.index
    old=tree_workspace(index);prototype=OUT/f'restart2-wood/tree{index}-boundary-v1';worker=OUT/'restart2-wood/projected/assets'/old.name;domain=OUT/f'restart2-wood/tree{index}-domain-v{2 if index==46 else 1}'
    if worker.exists() or domain.exists():raise FileExistsError('Preserve previous packet')
    domain.mkdir(parents=True);proto=json.loads((prototype/'evidence.json').read_text());base=Path(proto['previous_worker']);base_hash=sha(base/'model.blend');prototype_hash=sha(prototype/'model.blend')
    if base_hash!=proto['previous_model_sha256'] or prototype_hash!=proto['model_sha256']:raise ValueError('Prototype input changed')
    cfg=json.loads((old/'workspace.json').read_text());source_manifest=OUT/'tree35-root-research/source-domain-v1/source-masks.json' if index==35 else old/'source-masks.json';manifest=json.loads(source_manifest.read_text());inventory=json.loads(Path(manifest['mask_inventory']).read_text());boundary=Path(proto['boundary_mask']);pixels=np.asarray(Image.open(boundary).convert('L'))>0
    if sha(boundary)!=proto['boundary_mask_sha256']:raise ValueError('Boundary mask changed')
    owner=next(r for r in manifest['projections']['exterior']['assignments'] if r.get('asset_group')==old.name);private_index=9000+index
    if any(r['index']==private_index for r in inventory['masks']):raise ValueError('Private index already exists')
    for exclusion in owner.get('exclude_mask_indices',[]):
        row=next(r for r in inventory['masks'] if r['index']==exclusion);image=Image.new('L',(1792,1152));image.paste(Image.open(row['png']).convert('L'),tuple(row['box_top_left']))
        overlap=pixels&(np.asarray(image)>0)
        if np.any(overlap):
            if index!=46 or exclusion!=91 or int(overlap.sum())!=7:raise ValueError('Boundary conflicts with protected foreground mask')
            adjusted=domain/'foreground91-minus-reviewed-wood46.png';Image.fromarray(((np.asarray(image)>0)&~pixels).astype('uint8')*255).save(adjusted)
            if any(r['index']==19046 for r in inventory['masks']):raise ValueError('Private foreground derivative index exists')
            inventory['masks'].append(dict(index=19046,layer=row['layer'],png=str(adjusted),box_top_left=[0,0],box_size=[1792,1152],provenance='Native91 mixed foreground minus exactly7 separately reviewed inferred wood46 contour pixels.'))
            owner['exclude_mask_indices']=[19046 if x==91 else x for x in owner['exclude_mask_indices']]
            owner['exclusion_reason']+=' Native91 exclusion narrowed only by the7 reviewed inferred wood46 boundary pixels; all other91 and native128 unchanged.'
    mask=domain/'inferred-contour.png';Image.fromarray(pixels.astype('uint8')*255).save(mask);inventory['masks'].append(dict(index=private_index,layer=0,png=str(mask),box_top_left=[0,0],box_size=[1792,1152],provenance='Private explicitly reviewed contextual wood-contour inference; preserve exact source RGB.'))
    write_json(domain/'inventory.json',inventory);manifest['mask_inventory']=str(domain/'inventory.json');owner['mask_indices'].append(private_index);write_json(domain/'source-masks.json',manifest)
    write_json(domain/'source-review.json',dict(boundary_mask=str(boundary),boundary_mask_sha256=sha(boundary),private_domain=private_index,pixels=int(pixels.sum()),status='Private inferred contextual wood boundary; no canonical mask change',source_manifest_sha256=sha(domain/'source-masks.json')))
    bpy.ops.wm.open_mainfile(filepath=str(base/'model.blend'));bpy.context.preferences.filepaths.save_version=0
    prepare(worker,asset_id=old.name,scene_name=cfg['scene_name'],collection_name=cfg['collection_name'],source_path=old/'reference/source.png',grouping_manifest=old/'reference/grouping.json',inventory_path=old/'reference/inventory.json',review_path=old/'reference/grouping-review.json',source_mask_manifest=domain/'source-masks.json',width=384,height=384,framing_padding=cfg['framing_padding'],lighting=cfg['lighting'])
    bpy.ops.wm.open_mainfile(filepath=str(prototype/'model.blend'));bpy.context.preferences.filepaths.save_version=0
    bpy.ops.wm.save_as_mainfile(filepath=str(worker/'model.blend'));modified(worker)
    import restart2_resume_boundaries
    restart2_resume_boundaries.main()
    print(worker)

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
