"""Merge only the thirteen independently reviewed ground-plant candidates."""
import json
import sys
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT,reviewed_catalog
from evidence_io import sha,write_json
from catalog_schema import parse_catalog
from ground_plant_candidates import validate_record


def main():
    source=OUT/'understory-candidates/ready-north65-66-v2'
    destination=OUT/'ground-plant-integration'
    proposal_path=OUT/'ground-plant-candidates/proposal.json';proposal=json.loads(proposal_path.read_text())
    root_path=OUT/'ground-plant-candidates/root-joint-review.json';review=json.loads(root_path.read_text())
    if not review['ready'] or review['proposal_sha256']!=sha(proposal_path):raise ValueError('Independent root review absent/stale')
    for name,digest in review['files'].items():
        if sha(Path(name))!=digest:raise ValueError('Root joint evidence changed')
    baseline=sha(reviewed_catalog())
    if baseline!=sha(source/'catalog.json'):raise ValueError('Canonical ownership changed since coordinated80-group base')
    destination.mkdir(exist_ok=False)
    catalog=json.loads(reviewed_catalog().read_text());old_groups=list(catalog['groups']);old_owners=dict(catalog['canonical_owners'])
    write_json(destination/'previous-catalog.json',catalog)
    inventory=json.loads((source/'inventory/inventory.json').read_text())
    masks=json.loads((source/'source-masks.json').read_text());inventory_path=Path(masks['mask_inventory'])
    native=json.loads(inventory_path.read_text())
    for row in native['masks']:row['png']=str((inventory_path.parent/row['png']).resolve())
    indices={r['index'] for r in native['masks']};nodes={r['source_node'] for r in inventory['objects']}
    records={};arrays={}
    for original in proposal['records']:
        record=dict(original);group=dict(original['group']);index=record['native_mask'];domain=record['domain'];node=record['source_node'];worker=Path(record['workspace'])
        group.pop('native_foliage_mask');group['native_ground_plant_mask']=index
        group['name']=('East dry grass tuft ' if index<=115 else 'West dry grass tuft ' if index==116 else 'North plateau fern ')+str(index)
        record['group']=group;record['files']=dict(record['files'],**{str(root_path):sha(root_path),str(proposal_path):sha(proposal_path)})
        validate_record(record,group)
        if node in nodes or domain in indices or group['id'] in {g['id'] for g in catalog['groups']}:raise ValueError('Plant source/domain collision')
        catalog['groups'].append(group);catalog['canonical_owners'][node]=group['id']
        batch=worker.parents[1];data=json.loads((batch/'inventory/inventory.json').read_text())
        additions=[r for r in data['objects'] if r['source_node']==node]
        if len(additions)!=1:raise ValueError('Expected one exact plant mesh')
        inventory['objects'].extend(additions);nodes.add(node);indices.add(domain)
        native['masks'].append(dict(index=domain,layer=0,png=record['domain_path'],box_top_left=[0,0],box_size=[1792,1152],provenance='Unique native plant pixel ownership; explicit overlap priority recorded in proposal'))
        arrays[domain]=np.asarray(Image.open(record['domain_path']).convert('L'))>0
        masks['projections']['exterior']['assignments'].append(dict(source_node=node,mask_indices=[domain],reviewed=True))
        records[group['id']]=record
    combined=np.stack(list(arrays.values())).sum(axis=0)
    reserved=np.asarray(Image.open(OUT/'ground-texture-preparation/pending-ground-plants.png').convert('L'))>0
    if combined.max()!=1 or not np.array_equal(combined>0,reserved) or int(reserved.sum())!=6237:raise ValueError('Reserved plant ownership lost/duplicated')
    rows={r['index']:r for r in native['masks']}
    def bitmap(index):
        if index in arrays:return arrays[index]
        r=rows[index];x,y=r['box_top_left'];w,h=r['box_size'];a=np.zeros((1152,1792),bool)
        a[y:y+h,x:x+w]=np.asarray(Image.open(r['png']).convert('L'))>0
        return a
    receivers={'ground',*catalog['canonical_owners']};changes=[]
    exterior=masks['projections']['exterior']
    for assignment in exterior['assignments']:
        target=assignment.get('source_node');asset=assignment.get('asset_group')
        if target=='ground' or asset in ['croisement02-north-woodland-bank','croisement02-tree-23']:
            accepted=np.logical_or.reduce([bitmap(i) for i in assignment['mask_indices']])
            for i in assignment.get('exclude_mask_indices',[]):accepted&=~bitmap(i)
            for domain,array in arrays.items():
                overlap=int((accepted&array).sum())
                if overlap:
                    assignment.setdefault('exclude_mask_indices',[]).append(domain)
                    assignment.update(exclusions_reviewed=True,exclusion_reason='Observed native ground-plant domains belong to their authored scenery receivers; exact overlap pixels recorded.')
                    changes.append(dict(receiver=target or asset,domain=domain,pixels=overlap));accepted&=~array
    for record in records.values():
        exterior.setdefault('occluder_constraints',[]).append(dict(reviewed=True,source_node=record['source_node'],receiver_nodes=sorted(receivers-{record['source_node']}),mask_indices=[record['domain']],reason='Plant volume cannot obscure foreign source receivers outside its exact observed domain.'))
    for rule in exterior['occluder_constraints']:
        if rule['source_node'].startswith(('foliage-','scenery-')):rule['receiver_nodes']=sorted(receivers-{rule['source_node']})
    parse_catalog(catalog,nodes-{'ground'})
    assert catalog['groups'][:len(old_groups)]==old_groups and all(catalog['canonical_owners'][k]==v for k,v in old_owners.items())
    write_json(destination/'catalog.json',catalog);write_json(destination/'inventory.json',inventory);write_json(destination/'mask-inventory.json',native)
    masks['mask_inventory']=str(destination/'mask-inventory.json');write_json(destination/'source-masks.json',masks)
    grouping=dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(destination/'catalog.json'),inventory_sha256=sha(destination/'inventory.json'),evidence='Existing80 groups and owners preserved;13 native ground-plant candidates added after independent source/terrain joint review. No user approval inherited.')
    write_json(destination/'grouping-review.json',grouping)
    write_json(destination/'selection.json',dict(status='New geometry pending user approval; source-native inferred backs, no AI generation',records=records))
    if sha(reviewed_catalog())!=baseline:raise ValueError('Concurrent canonical writer changed catalog')
    write_json(OUT/'ownership-revision/catalog.json',catalog);write_json(OUT/'ownership-revision/grouping-review.json',grouping)
    write_json(destination/'integration.json',dict(status='13 new geometry candidates registered; user approval pending',groups=len(catalog['groups']),records=list(records),source_masks=str(destination/'source-masks.json'),inventory=str(destination/'inventory.json'),ownership_changes=changes,previous_catalog_sha256=baseline,catalog_sha256=sha(destination/'catalog.json')))
    print('Registered',len(records),'plants;',len(catalog['groups']),'groups')

if __name__=='__main__':main()
