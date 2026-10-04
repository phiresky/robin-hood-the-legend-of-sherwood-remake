"""Compare physical scene first hits with a frozen metadata ownership proposal."""
import argparse
import json
import sys
from pathlib import Path
import numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from catalog_schema import source_for_part
from evidence_io import sha,write_json


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('stage',type=Path);args=parser.parse_args();stage=args.stage.resolve();output=stage/'first-hit'
    data=np.load(output/'source-first-hit.npz');labels=data['labels'];names=list(data['names']);catalog=json.loads((stage/'catalog.json').read_text())
    nodes={source_for_part(part):group['id'] for group in catalog['groups'] for part in group['parts']}
    source=OUT/'ground-plant-integration/source-masks.json';manifest=json.loads(source.read_text());ip=source.parent/manifest['mask_inventory'];inventory=json.loads(ip.read_text())
    reference=output/'ownership-reference';reference.mkdir();cache={}
    for row in inventory['masks']:
        if not row.get('png'):continue
        path=(ip.parent/row['png']).resolve();image=Image.open(path).convert('L');x,y=row['box_top_left'];w,h=image.size
        bitmap=np.zeros((1152,1792),bool);bitmap[max(y,0):min(y+h,1152),max(x,0):min(x+w,1792)]=np.asarray(image)[max(-y,0):min(h,1152-y),max(-x,0):min(w,1792-x)]>0
        cache[row['index']]=bitmap;target=reference/f"mask-{row['index']:03}.png";image.save(target);row['png']=str(target)
    write_json(reference/'inventory.json',inventory);manifest['mask_inventory']=str(reference/'inventory.json');write_json(reference/'source-masks.json',manifest)
    expected={};assignments=[]
    for row in manifest['projections']['exterior']['assignments']:
        asset=row.get('asset_group') or nodes.get(row.get('source_node'))
        if not asset:assignments.append(dict(assignment=row,status='source node has no current catalog owner'));continue
        mask=np.logical_or.reduce([cache[index] for index in row['mask_indices']])
        for index in row.get('exclude_mask_indices',[]):mask &= ~cache[index]
        expected[asset]=expected.get(asset,np.zeros_like(mask))|mask
    expected['croisement02-ground-receiver']=np.asarray(Image.open(OUT/'ground-receiver-review-v5/reference/ground-observed-domain.png'))>0
    rows=[];counts=np.zeros((1152,1792),np.uint16)
    for asset,mask in expected.items():
        counts+=mask
        hit=(labels==names.index(asset)) if asset in names else np.zeros_like(mask)
        foreign={str(names[int(label)]) if label>=0 else 'unassigned':int((mask&(labels==label)).sum()) for label in np.unique(labels[mask&~hit])}
        rows.append(dict(asset=asset,metadata_source_pixels=int(mask.sum()),own_first_hit_pixels=int((mask&hit).sum()),other_first_hit=foreign,note='Metadata proposal compared to physical first hit; occlusion and native mask overlap require interpretation.'))
    write_json(output/'metadata-ownership-audit.json',dict(status='measured; not blanket source coverage approval',scene_sha256=sha(stage/'scene.blend'),first_hit_sha256=sha(output/'source-first-hit.npz'),catalog_sha256=sha(stage/'catalog.json'),original_source_manifest_sha256=sha(source),frozen_source_manifest_sha256=sha(reference/'source-masks.json'),rows=rows,visible_receivers_without_global_metadata=[name for name in names if name not in expected],unresolved_assignments=assignments,metadata_pixels_with_multiple_owners=int((counts>1).sum()),limitations=['The93-group manifest is a source ownership proposal, not proof that approved cached materials were rewritten.','Native foreground masks overlap and physical canopy occlusion can legitimately hide parts of another receiver domain.','Receiver-specific candidate domains can be newer than this integrated metadata snapshot; compare their frozen worker evidence before classifying a discrepancy as a model defect.']))
    print(len(rows),'receiver domains;',int((counts>1).sum()),'overlapping metadata pixels')


if __name__=='__main__':main()
