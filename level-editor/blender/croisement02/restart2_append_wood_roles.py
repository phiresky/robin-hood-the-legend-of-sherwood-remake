"""Append reviewed wood-boundary authority to the immutable source delta."""
import json,sys
from pathlib import Path
import numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT,reviewed_catalog
from evidence_io import sha,write_json
from restart2_wood_revisions import selected_workspace


def main():
    base=OUT/'restart2-vegetation/source-role-delta-v1';dest=OUT/'restart2-vegetation/source-role-delta-v2'
    if dest.exists():raise FileExistsError(dest)
    catalog=json.loads((base/'catalog.json').read_text());assert catalog==json.loads(reviewed_catalog().read_text())
    masks=json.loads((base/'mask-inventory.json').read_text());manifest=json.loads((base/'source-masks.json').read_text())
    existing={r['index']:r for r in masks['masks']};records=[]
    for n,count in [(35,122),(43,20),(45,2),(46,7)]:
        worker=selected_workspace(OUT,n,reviewed_catalog());assert worker is not None
        own=json.loads((worker/'source-masks.json').read_text());invpath=Path(own['mask_inventory']);inv=json.loads(invpath.read_text())
        row=next(r for r in own['projections']['exterior']['assignments']if r.get('asset_group')==f'croisement02-tree-{n}')
        wanted=set(row['mask_indices']+row.get('exclude_mask_indices',[]))
        for r in inv['masks']:
            if r['index'] not in wanted:continue
            r=dict(r,png=str((invpath.parent/r['png']).resolve()))
            if r['index'] in existing:
                old=existing[r['index']]
                if (old['box_top_left']!=r['box_top_left'] or old['box_size']!=r['box_size'] or sha(Path(old['png']))!=sha(Path(r['png']))):raise ValueError('Conflicting mask ID '+str(r['index']))
            else:masks['masks'].append(r);existing[r['index']]=r
        boundary=existing[9000+n];a=np.asarray(Image.open(boundary['png']).convert('L'))>0;assert int(a.sum())==count
        for j,old in enumerate(manifest['projections']['exterior']['assignments']):
            if old.get('asset_group')==row['asset_group']:manifest['projections']['exterior']['assignments'][j]=row;break
        else:raise ValueError('Missing current tree assignment')
        ground=next(r for r in manifest['projections']['exterior']['assignments']if r.get('source_node')=='ground')
        ground['exclude_mask_indices']=sorted(set(ground['exclude_mask_indices']+[9000+n]))
        receipt=OUT/f'restart2-wood/selections/tree-{n}.json'
        records.append(dict(asset_id=row['asset_group'],worker=str(worker),model_sha256=sha(worker/'model.blend'),selection=str(receipt),selection_sha256=sha(receipt),inferred_domain=9000+n,pixels=count,mask_sha256=sha(Path(boundary['png']))))
    dest.mkdir();manifest['mask_inventory']=str(dest/'mask-inventory.json')
    write_json(dest/'catalog.json',catalog);write_json(dest/'mask-inventory.json',masks);write_json(dest/'source-masks.json',manifest)
    write_json(dest/'delta.json',dict(status='Authoritative additional post-freeze scoped wood-source delta',base_delta=str(base/'delta.json'),base_delta_sha256=sha(base/'delta.json'),base_source_sha256=sha(base/'source-masks.json'),source_sha256=sha(dest/'source-masks.json'),unchanged_groups=126,records=records,limitations=['Scopes inferred wood only; whole-tree crowns and rear texture remain HOLD.','No user approval or full-scene completion implied.','All earlier snapshots remain unchanged.']))
    print(dest)

if __name__=='__main__':main()
