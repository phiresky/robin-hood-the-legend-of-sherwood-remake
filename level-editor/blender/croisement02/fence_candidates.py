"""Register reviewed authored rail-fence candidates without creating approvals."""
import argparse
import copy
import json
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog_schema import parse_catalog,source_for_part
from evidence_io import sha,write_json


def selected_workspace(out,asset,catalog_path):
    receipt_path=out/'fence-integration/selection.json'
    if not receipt_path.exists():return None
    records=json.loads(receipt_path.read_text())['records']
    if asset not in records:return None
    record=records[asset];worker=Path(record['workspace'])
    catalog=json.loads(catalog_path.read_text())
    group=next((g for g in catalog['groups'] if g['id']==asset),None)
    if group!=record['group']:raise ValueError('Registered fence ownership changed: '+asset)
    for path,digest in record['files'].items():
        if sha(Path(path))!=digest:raise ValueError('Registered fence evidence changed: '+path)
    if sha(worker/'model.blend')!=record['model_sha256']:raise ValueError('Registered fence model changed')
    for path,digest in json.loads((worker/'modified/views.json').read_text()).get('source_mask_evidence',{}).items():
        if sha(Path(path))!=digest:raise ValueError('Fence source evidence changed: '+path)
    return worker


def register(directory,base,out):
    from catalog import reviewed_catalog
    canonical=reviewed_catalog();previous_hash=sha(canonical)
    previous=json.loads(canonical.read_text());base_catalog=json.loads((base/'catalog.json').read_text())
    if previous!=base_catalog:raise ValueError('Current catalog differs from explicit integration base')
    grouping=json.loads((out/'ownership-revision/grouping-review.json').read_text())
    if grouping['inventory_sha256']!=sha(base/'inventory/inventory.json') or grouping['catalog_sha256']!=previous_hash:
        raise ValueError('Current canonical catalog/inventory binding differs')
    inventory=copy.deepcopy(json.loads((base/'inventory/inventory.json').read_text()))
    catalog=copy.deepcopy(previous);proposal=json.loads((directory/'catalog.json').read_text())
    proposed_inventory=json.loads((directory/'inventory/inventory.json').read_text())
    manifest=copy.deepcopy(json.loads((base/'source-masks.json').read_text()))
    inventory_path=Path(manifest['mask_inventory']);masks=copy.deepcopy(json.loads(inventory_path.read_text()))
    for row in masks['masks']:row['png']=str((inventory_path.parent/row['png']).resolve())
    fence_manifest=json.loads((directory/'source-masks.json').read_text())
    fence_inventory_path=Path(fence_manifest['mask_inventory']);fence_masks=json.loads(fence_inventory_path.read_text())
    destination=out/'fence-integration'
    if destination.exists():raise FileExistsError(destination)
    records={};new_nodes=[]
    for index,domain in [(94,430),(95,431)]:
        asset=f'croisement02-east-upright-rail-fence-{index}';node=f'scenery-upright-fence-{index:03}';worker=directory/'assets'/asset
        group=copy.deepcopy(next(g for g in proposal['groups'] if g['id']==asset));group['native_scenery_mask']=index
        if not group.get('authored_scenery') or group['parts']!=[dict(node=node,name=f'Open wooden rail fence {index}',wood_domain_mask=domain)]:raise ValueError('Unexpected authored fence ownership')
        if asset in {g['id'] for g in catalog['groups']} or node in catalog['canonical_owners']:raise ValueError('Fence already registered')
        model_hash=sha(worker/'model.blend');review=json.loads((worker/'inspection/visual-review.json').read_text());audit=json.loads((worker/'inspection/saved-model-audit.json').read_text());topology=json.loads((worker/'inspection/fence-topology.json').read_text());coverage=topology['coverage'];validation=json.loads((worker/'validation.json').read_text())
        if not (review.get('ready_for_geometry_review') and review.get('all_eight_actual_views_inspected') and audit['status']==topology['status']==validation['status']=='PASS' and all(r['model_sha256']==model_hash for r in (review,audit,topology,coverage)) and review['sheet_sha256']==sha(worker/'inspection/actual-materials/sheet.png') and 1-coverage['missing_pixels']/coverage['expected_pixels']>=.98):raise ValueError('Fence review or coverage incomplete: '+asset)
        if any(r['nonmanifold_edges'] or r['degenerate_faces'] for r in topology['objects']):raise ValueError('Open/degenerate timber')
        if sorted(o['source_node'] for o in audit['objects'])!=[node]:raise ValueError('Foreign model objects in fence scope')
        scoped=json.loads((worker/'workspace.json').read_text())
        if scoped['part_ids']!=[node]:raise ValueError('Wrong worker ownership')
        added_objects=[r for r in proposed_inventory['objects'] if r['source_node']==node]
        if len(added_objects)!=1:raise ValueError('Missing or duplicate authored mesh')
        catalog['groups'].append(group);catalog['canonical_owners'][node]=asset;inventory['objects']+=added_objects;new_nodes.append(node)
        row=next(r for r in fence_masks['masks'] if r['index']==domain)
        if any(r['index']==domain for r in masks['masks']):raise ValueError('Authored domain index collision')
        row=copy.deepcopy(row);row['png']=str((fence_inventory_path.parent/row['png']).resolve());masks['masks'].append(row)
        assignment=next(r for r in fence_manifest['projections']['exterior']['assignments'] if r.get('source_node')==node)
        manifest['projections']['exterior']['assignments'].append(assignment)
        constraint=next(r for r in fence_manifest['projections']['exterior']['occluder_constraints'] if r['source_node']==node)
        manifest['projections']['exterior'].setdefault('occluder_constraints',[]).append(copy.deepcopy(constraint))
        files=[worker/'model.blend',worker/'workspace.json',worker/'source-masks.json',worker/'validation.json',worker/'inspection/visual-review.json',worker/'inspection/saved-model-audit.json',worker/'inspection/fence-topology.json',worker/'inspection/source-coverage/report.json',worker/'inspection/refinement.json',worker/'inspection/actual-materials/evidence.json',worker/'inspection/actual-materials/sheet.png',worker/'modified/solid.png',worker/'modified/textured.png',worker/'modified/views.json',directory/'source-review.json',Path(row['png'])]
        records[asset]=dict(workspace=str(worker),model_sha256=model_hash,group=group,files={str(p):sha(p) for p in files},approval='pending; registration does not imply user approval')
    parse_catalog(catalog,{r['source_node'] for r in inventory['objects']}-{'ground'})
    receivers={'ground',*catalog['canonical_owners']}
    for constraint in manifest['projections']['exterior']['occluder_constraints']:
        if constraint['source_node'].startswith(('foliage-','scenery-')):constraint['receiver_nodes']=sorted(receivers-{constraint['source_node']})
    ground=next(r for r in manifest['projections']['exterior']['assignments'] if r.get('source_node')=='ground')
    ground['exclude_mask_indices']+= [430,431];ground['exclusions_reviewed']=True;ground['exclusion_reason']+=' Source-traced authored open fences430/431 leave ground ownership.'
    if sha(canonical)!=previous_hash:raise ValueError('Concurrent canonical catalog change; retry against current base')
    destination.mkdir()
    write_json(destination/'previous-catalog.json',previous);write_json(destination/'catalog.json',catalog);write_json(destination/'inventory.json',inventory);write_json(destination/'mask-inventory.json',masks)
    manifest['mask_inventory']=str(destination/'mask-inventory.json');write_json(destination/'source-masks.json',manifest)
    review=dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(destination/'catalog.json'),inventory_sha256=sha(destination/'inventory.json'),previous_catalog_sha256=previous_hash,evidence='Add only two self-reviewed authored open rail fences. Existing groups and source owners are unchanged. Candidate models remain exact reviewed bytes; source assignments for new fences are unchanged. Full scene integration remains separate.')
    write_json(destination/'grouping-review.json',review)
    write_json(destination/'selection.json',dict(version=1,status='reviewed candidates exposed for user decisions; no approvals added',records=records))
    write_json(canonical,catalog);write_json(out/'ownership-revision/grouping-review.json',review)
    print('Registered fences:',len(catalog['groups']),'groups;',len(catalog['canonical_owners']),'sources')


if __name__=='__main__':
    from catalog import OUT
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--directory',type=Path,default=OUT/'missing-fence-candidates/v9');parser.add_argument('--base',type=Path,default=OUT/'understory-candidates/west-bank-v4')
    args=parser.parse_args();register(args.directory.resolve(),args.base.resolve(),OUT)
