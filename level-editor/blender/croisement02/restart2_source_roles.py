"""Bind a post-freeze source-role delta without changing catalog identities."""
import json
import sys
from pathlib import Path
import numpy as np
from PIL import Image
sys.path[:0]=[str(Path(__file__).resolve().parent),str(Path(__file__).resolve().parents[3]/'level-editor/refinement/blender')]
from catalog import OUT,reviewed_catalog
from evidence_io import sha,write_json


def main():
    base=OUT/'restart2-vegetation/registered75-91'
    dest=OUT/'restart2-vegetation/source-role-delta-v1'
    if dest.exists():raise FileExistsError(dest)
    catalog=json.loads((base/'catalog.json').read_text())
    assert catalog==json.loads(reviewed_catalog().read_text()) and len(catalog['groups'])==126
    manifest=json.loads((base/'source-masks.json').read_text())
    prior_inventory=Path(manifest['mask_inventory'])
    inventory=json.loads(prior_inventory.read_text())
    for row in inventory['masks']:row['png']=str((prior_inventory.parent/row['png']).resolve())
    paths={
        6002:OUT/'mixed-wood-audit/boundary-roles76-93-v1/76-foliage76.png',
        6003:OUT/'mixed-wood-audit/boundary-roles76-93-v1/93-foliage93.png',
        6004:OUT/'tree38-root-research/distal-root-inference.png',
        6005:OUT/'understory-candidates/mixed75-91-source-v3/ground-return75.png',
        6006:OUT/'mixed-wood-audit/boundary-roles76-93-v1/76-wattle99.png',
        6008:OUT/'restart2-ground38/cumulative848-v1/ground-observed-domain.png'}
    roles={6002:'Inferred leaf boundary76',6003:'Inferred leaf boundary93',6004:'Inferred ground/root-shadow38',6005:'Native ground returned from mixed75',6006:'Inferred timber boundary99',6007:'Native76 foreground excluding inferred timber99',6008:'Cumulative known ground; includes6004/6005 exactly once'}
    arrays={i:np.asarray(Image.open(p).convert('L'))>0 for i,p in paths.items()}
    for i,count in {6002:22,6003:25,6004:65,6005:783,6006:148,6008:772189}.items():assert int(arrays[i].sum())==count
    assert not np.any(arrays[6004]&arrays[6005])
    assert np.all(arrays[6008][arrays[6004]|arrays[6005]])
    for i in [6002,6003,6006]:assert not np.any(arrays[i]&arrays[6008])
    native=next(r for r in inventory['masks']if r['index']==76)
    raw=np.asarray(Image.open(native['png']).convert('L'))>0
    if raw.shape!=(1152,1792):
        canvas=np.zeros((1152,1792),dtype=bool);x,y=native['box_top_left'];canvas[y:y+raw.shape[0],x:x+raw.shape[1]]=raw;raw=canvas
    assert np.all(raw[arrays[6006]])
    arrays[6007]=raw&~arrays[6006]
    dest.mkdir()
    paths[6007]=dest/'remaining-native76.png';Image.fromarray(arrays[6007].astype('uint8')*255).save(paths[6007])
    for i,path in paths.items():
        assert not any(r['index']==i for r in inventory['masks'])
        inventory['masks'].append(dict(index=i,layer=0,png=str(path),box_top_left=[0,0],box_size=[1792,1152],provenance=roles[i]))
    assignments=manifest['projections']['exterior']['assignments']
    for row in assignments:
        if row.get('source_node')=='ground':
            row['mask_indices']=[6008]
            row['exclude_mask_indices']=sorted(set(row.get('exclude_mask_indices',[])+[6002,6003,6006]))
            row['exclusion_reason']='Exact cumulative known ground; source-role boundary receivers excluded.6004/6005 are provenance subsets, not additional ground permission unions.'
        if row.get('source_node')=='foliage-shrub-076':row['mask_indices']=[502,6002]
        if row.get('source_node')=='foliage-shrub-093':row['mask_indices']=[503,6003]
        if row.get('asset_group')=='croisement02-tree-38':
            row['exclude_mask_indices']=sorted(set(row.get('exclude_mask_indices',[])+[6004]));row['exclusions_reviewed']=True
            row['exclusion_reason']='Explicit65 source ground/root-shadow restoration, independently reviewed.'
    assert not any(r.get('asset_group')=='croisement02-southwest-path-wattle-fence' for r in assignments)
    assignments.append(dict(asset_group='croisement02-southwest-path-wattle-fence',mask_indices=[99,6006],exclude_mask_indices=[6007,42,129],reviewed=True,exclusions_reviewed=True,exclusion_reason='Separate native99 and inferred148 timber; remaining native76 and adjacent canopy foreground retained.'))
    manifest['mask_inventory']=str(dest/'mask-inventory.json')
    write_json(dest/'catalog.json',catalog);write_json(dest/'mask-inventory.json',inventory);write_json(dest/'source-masks.json',manifest)
    rows=[dict(index=i,role=roles[i],path=str(paths[i]),sha256=sha(paths[i]),pixels=int(arrays[i].sum()),certainty='inferred' if i in [6002,6003,6004,6006] else 'source-derived')for i in sorted(paths)]
    write_json(dest/'delta.json',dict(status='Authoritative post-freeze source-role delta; geometry selection and user approval separate',base_catalog_sha256=sha(base/'catalog.json'),catalog_sha256=sha(dest/'catalog.json'),source_manifest_sha256=sha(dest/'source-masks.json'),unchanged_groups=126,domains=rows,ground_permission_pixels=772189,ground_provenance_subset_pixels=848,ground_subsets_added_to_union=False,limitations=['Frozen126 assembly/receipts are unchanged.','Source roles do not assert physical scene coverage.','Shrub93 exact boundary geometry selection remains pending; this records independently accepted inferred source roles.','Tree35 inferred122 timber and other pending wood domains are not included.']))
    assert catalog==json.loads(reviewed_catalog().read_text())
    print(dest)

if __name__=='__main__':main()
