"""Exclude native foreground foliage from a private oak35 wood projection."""
import copy,json,sys
from pathlib import Path
import numpy as np
from PIL import Image,ImageFilter
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'level-editor/refinement/blender'))
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json


def main():
    destination=OUT/'tree35-root-research/source-domain-v1';destination.mkdir(parents=True,exist_ok=False)
    original=tree_workspace(35);manifest=copy.deepcopy(json.loads((original/'source-masks.json').read_text()));path=Path(manifest['mask_inventory']);inventory=copy.deepcopy(json.loads(path.read_text()));by_index={r['index']:r for r in inventory['masks']}
    for record in inventory['masks']:record['png']=str((path.parent/record['png']).resolve())
    def mask(index):
        r=by_index[index];x,y=r['box_top_left'];w,h=r['box_size'];a=np.zeros((1152,1792),bool);a[y:y+h,x:x+w]=np.asarray(Image.open(r['png']).convert('L'))>0;return a
    wood=mask(35);foliage={i:mask(i) for i in [85,128,131]};confirmed=np.logical_or.reduce(list(foliage.values()));near=np.asarray(Image.fromarray(confirmed.astype('uint8')*255).filter(ImageFilter.MaxFilter(3)))>0;uncertain=wood&near&~confirmed;known=wood&~near
    if 505 in by_index:raise ValueError('Ambiguous domain505 already allocated')
    Image.fromarray(uncertain.astype('uint8')*255).save(destination/'domain-505-ambiguous.png');Image.fromarray(known.astype('uint8')*255).save(destination/'confirmed-wood.png');Image.fromarray((wood&confirmed).astype('uint8')*255).save(destination/'confirmed-foreground-foliage.png')
    inventory['masks'].append(dict(index=505,layer=0,layer_index=0,png=str(destination/'domain-505-ambiguous.png'),mask_type=7,box_top_left=[0,0],box_size=[1792,1152],character_polyline=[],projectile_polyline=[],obstacle_indices=[],provenance='One-pixel uncertainty band around native foreground foliage; excluded from wood, not asserted foliage or ground.'))
    assignment=next(r for r in manifest['projections']['exterior']['assignments'] if r.get('asset_group')=='croisement02-tree-35');assignment.update(exclude_mask_indices=[85,128,131,505],exclusions_reviewed=True,exclusion_reason='Native85 shrub and native128 foreground crown visibly cross the oak base; native131 is separate canopy ownership. Their native alpha/shape plus occlusion depth identify foreground foliage. One-pixel ambiguity excluded separately505.')
    write_json(destination/'mask-inventory.json',inventory);manifest['mask_inventory']=str(destination/'mask-inventory.json');write_json(destination/'source-masks.json',manifest)
    evidence=dict(status='private source-domain correction; no canonical ownership changed',model_sha256=sha(original/'model.blend'),original_source_manifest_sha256=sha(original/'source-masks.json'),source_sha256=sha(OUT/'animation-references/composite-frame-0.png'),native_masks={str(i):sha(Path(by_index[i]['png'])) for i in [35,85,128,131]},source_mask_manifest_sha256=sha(destination/'source-masks.json'),mask_inventory_sha256=sha(destination/'mask-inventory.json'),wood_native_pixels=int(wood.sum()),confirmed_foreground_pixels=int((wood&confirmed).sum()),confirmed_foreground_overlap_counts={str(i):int((wood&a).sum()) for i,a in foliage.items()},ambiguous_boundary_pixels=int(uncertain.sum()),remaining_observed_wood_pixels=int(known.sum()),foreground_owners={'85':'pending shrub domain495','128':'existing native foreground canopy owner(s)','131':'existing canopy component'},domain505='Unassigned uncertainty; do not repaint as wood, add new foliage, or ground.',evidence=['mixed-wood-audit/root-mask-85.png','mixed-wood-audit/root-mask-128.png','mixed-wood-audit/root-mask-131.png','mixed-wood-audit/93-depth.png','mixed-wood-audit/root-source-grid.png'])
    write_json(destination/'source-review.json',evidence);print(json.dumps(evidence,indent=2))

if __name__=='__main__':main()
