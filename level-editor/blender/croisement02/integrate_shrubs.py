"""Register only manually reviewed authored shrub candidates in the map catalog."""
import argparse
import json
import sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT,reviewed_catalog
from catalog_schema import parse_catalog
from evidence_io import sha,write_json


def main(directory,workers_directory):
    directory=directory.resolve()
    if sha(reviewed_catalog())!=sha(directory/'previous-catalog.json'):
        raise ValueError('Current ownership changed; merge the new shrub groups explicitly')
    catalog=json.loads((directory/'catalog.json').read_text())
    inv=json.loads((directory/'inventory/inventory.json').read_text())
    parse_catalog(catalog,{r['source_node'] for r in inv['objects']}-{'ground'})
    previous=json.loads((directory/'previous-catalog.json').read_text())
    old_groups={g['id']:g for g in previous['groups']}
    groups={g['id']:g for g in catalog['groups']}
    if any(groups.get(key)!=value for key,value in old_groups.items()):
        raise ValueError('Shrub integration must preserve every existing group')
    if any(catalog['canonical_owners'].get(key)!=value for key,value in previous['canonical_owners'].items()):
        raise ValueError('Shrub integration must preserve existing source owners')
    added=[g for key,g in groups.items() if key not in old_groups]
    if not added or any(not g.get('authored_scenery') or 'native_foliage_mask' not in g for g in added):
        raise ValueError('Only new authored shrub groups may be registered')
    grouping=json.loads((directory/'grouping-review.json').read_text())
    if grouping['catalog_sha256']!=sha(directory/'catalog.json') or grouping['inventory_sha256']!=sha(directory/'inventory/inventory.json'):
        raise ValueError('Candidate source inventory binding changed')
    records=[]
    for group in added:
        worker=workers_directory/group['id']
        model_hash=sha(worker/'model.blend')
        review=json.loads((worker/'inspection/visual-review.json').read_text())
        audit=json.loads((worker/'inspection/saved-model-audit.json').read_text())
        coverage=json.loads((worker/'inspection/source-coverage/report.json').read_text())
        bounds=json.loads((worker/'inspection/actual-materials/opacity-bounds.json').read_text())
        if not (review.get('ready_for_geometry_review') and review['model_sha256']==model_hash
                and audit['status']=='PASS' and audit['model_sha256']==model_hash
                and coverage['model_sha256']==model_hash and coverage['intersection_over_union']>=.95
                and bounds['model_sha256']==model_hash and min(r['depth_width_ratio'] for r in bounds['crowns'])>=1.
                and review['sheet_sha256']==sha(worker/'inspection/actual-materials/sheet.png')):
            raise ValueError('Shrub review/geometry checks incomplete: '+worker.name)
        preservation=Path(review['preservation_evidence'])
        if sha(preservation)!=review['preservation_evidence_sha256']:
            raise ValueError('Observed source preservation evidence changed')
        joint_path=worker/'inspection/joint-neighbourhood.json'
        if sha(joint_path)!=review['joint_neighbourhood_sha256']:
            raise ValueError('Joint review binding changed')
        joint=json.loads(joint_path.read_text())
        if (joint['model_sha256']!=model_hash or sha(Path(joint['evidence']))!=joint['evidence_sha256']
                or sha(Path(joint['sheet']))!=joint['sheet_sha256']):
            raise ValueError('Joint review packet changed')
        evidence=json.loads(Path(joint['evidence']).read_text())
        for dependency in evidence['workers']:
            if sha(Path(dependency['path'])/'model.blend')!=dependency['model_sha256']:
                raise ValueError('Joint neighbour changed since review')
        record=dict(model_sha256=model_hash,catalog_sha256=sha(directory/'catalog.json'),
                    status='reviewed geometry candidate; no user approval implied')
        records.append(dict(asset_id=worker.name,**record))
    if sha(reviewed_catalog())!=sha(directory/'previous-catalog.json'):
        raise ValueError('Concurrent canonical catalog change')
    for record in records:
        write_json(workers_directory/record['asset_id']/'inspection/shrub-candidate.json',{k:v for k,v in record.items() if k!='asset_id'})
    write_json(OUT/'ownership-revision/catalog.json',catalog)
    write_json(OUT/'ownership-revision/grouping-review.json',json.loads((directory/'grouping-review.json').read_text()))
    write_json(directory/'integration.json',dict(status='reviewed shrub candidates integrated; user approval pending',groups=len(catalog['groups']),native_parts=sum(key.startswith('building-') for key in catalog['canonical_owners']),authored_parts=sum(key.startswith(('foliage-','scenery-')) for key in catalog['canonical_owners']),
        inventory=str(directory/'inventory/inventory.json'),source_masks=str(directory/'source-masks.json'),records=records))
    print('Integrated',len(records),'shrub candidates:',len(catalog['groups']),'groups')

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory',type=Path,default=OUT/'understory-candidates/clumps-v1')
    parser.add_argument('--workers-directory',type=Path,default=OUT/'understory-round-1/assets')
    args=parser.parse_args();main(args.directory,args.workers_directory)
