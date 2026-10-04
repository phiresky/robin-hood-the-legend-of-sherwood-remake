"""Measure saved-scene visibility against each snapshotted worker's own domains."""
import argparse
import json
import sys
from pathlib import Path
import numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog_schema import source_for_part
from evidence_io import sha,write_json


def bitmap(row,root):
    a=np.asarray(Image.open(root/row['png']).convert('L'))>0;x,y=row['box_top_left'];h,w=a.shape
    result=np.zeros((1152,1792),bool)
    result[max(y,0):min(y+h,1152),max(x,0):min(x+w,1792)]=a[max(-y,0):min(h,1152-y),max(-x,0):min(w,1792-x)]
    return result


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('snapshot',type=Path);parser.add_argument('stage',type=Path);args=parser.parse_args()
    snapshot=args.snapshot.resolve();stage=args.stage.resolve();selected=json.loads((snapshot/'snapshot.json').read_text())
    groups={r['id']:r for r in json.loads((snapshot/'catalog.json').read_text())['groups']}
    data=np.load(stage/'first-hit/source-first-hit.npz');labels=data['labels'];names=list(data['names']);records=[]
    dest=stage/'first-hit/worker-domains';dest.mkdir()
    for item in selected['workers']:
        if 'worker' not in item:continue
        asset=item['id'];worker=Path(item['worker']);path=worker/'source-masks.json'
        if sha(worker/'model.blend')!=item['model_sha256'] or not path.exists():
            records.append(dict(asset=asset,status='held: worker changed or source manifest absent'));continue
        manifest=json.loads(path.read_text());ip=(path.parent/manifest['mask_inventory']).resolve();inventory={r['index']:r for r in json.loads(ip.read_text())['masks']}
        nodes={source_for_part(p) for p in groups[asset]['parts']};domain=np.zeros((1152,1792),bool);used=[]
        for row in manifest.get('projections',{}).get('exterior',{}).get('assignments',[]):
            if row.get('asset_group')!=asset and row.get('source_node') not in nodes:continue
            mask=np.logical_or.reduce([bitmap(inventory[index],ip.parent) for index in row['mask_indices']])
            for index in row.get('exclude_mask_indices',[]):mask &= ~bitmap(inventory[index],ip.parent)
            domain |= mask;used.append(row)
        if not used:records.append(dict(asset=asset,status='held: no matching exterior assignment'));continue
        output=dest/f'{asset}.png';Image.fromarray(domain.astype('uint8')*255).save(output)
        own=labels==names.index(asset) if asset in names else np.zeros_like(domain)
        others={str(names[int(i)]) if i>=0 else 'unassigned':int((domain&(labels==i)).sum()) for i in np.unique(labels[domain&~own])}
        records.append(dict(asset=asset,status='measured',worker_model_sha256=item['model_sha256'],source_manifest=str(path),source_manifest_sha256=sha(path),inventory_sha256=sha(ip),assignments=used,domain=str(output),domain_sha256=sha(output),source_pixels=int(domain.sum()),own_first_hit_pixels=int((domain&own).sum()),other_first_hit=others))
    write_json(stage/'first-hit/worker-domain-audit.json',dict(scene_sha256=sha(stage/'scene.blend'),snapshot_sha256=sha(snapshot/'snapshot.json'),first_hit_sha256=sha(stage/'first-hit/source-first-hit.npz'),records=records,limitations=['Worker source domains are measured independently, not silently reassigned based on visibility.','Global93 metadata may include later exclusions such as fern119/tree23 that do not rewrite cached worker material.','A foreground first hit can be legitimate native overlap; this is not automatic coverage approval.','Shared canopy source permissions can span several trees; a low hit ratio is not a missing-geometry count.']))
    print(len(records),'workers;',sum(r['status']=='measured' for r in records),'domains measured')


if __name__=='__main__':main()
