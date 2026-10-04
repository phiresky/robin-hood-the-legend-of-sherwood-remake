"""Merge reviewed central foliage proposals without changing prior scene geometry."""
import argparse
import json
import sys
from pathlib import Path
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT,reviewed_catalog
from catalog_schema import parse_catalog
from evidence_io import sha,write_json
from central_candidates import validate_record


def main(base,proposal_path,destination):
    proposal=json.loads(proposal_path.read_text())
    if not proposal['root_review_ready'] or proposal.get('user_approval') is not None:raise ValueError('Independent geometry readiness required; no inherited approval')
    baseline=sha(reviewed_catalog())
    if baseline!=sha(base/'catalog.json'):raise ValueError('Source base and canonical catalog differ')
    inventory_path=base/'inventory.json'
    if not inventory_path.exists():inventory_path=base/'inventory/inventory.json'
    catalog=json.loads(reviewed_catalog().read_text());previous=json.loads(json.dumps(catalog));inventory=json.loads(inventory_path.read_text())
    masks=json.loads((base/'source-masks.json').read_text());mask_path=Path(masks['mask_inventory']);native=json.loads(mask_path.read_text())
    for row in native['masks']:row['png']=str((mask_path.parent/row['png']).resolve())
    indices={r['index'] for r in native['masks']};nodes={r['source_node'] for r in inventory['objects']};records={};domains={}
    for record in proposal['records']:
        group=record['group'];worker=validate_record(record,group);node=group['parts'][0]['node'];domain=record['domain']
        if node in nodes or domain in indices or group['id'] in {g['id'] for g in catalog['groups']}:raise ValueError('New central source/domain collides')
        batch=Path(record['batch']);data=json.loads((batch/'inventory/inventory.json').read_text())
        additions=[r for r in data['objects'] if r['source_node']==node]
        if len(additions)!=1:raise ValueError('Expected one exact authored source object')
        catalog['groups'].append(group);catalog['canonical_owners'][node]=group['id'];inventory['objects']+=additions;nodes.add(node);indices.add(domain)
        domain_path=batch/f'domain-{domain}.png';domains[domain]=np.asarray(Image.open(domain_path).convert('L'))>0
        if sha(domain_path)!=record['files'][str(domain_path)]:raise ValueError('Observed domain not frozen in review')
        native['masks'].append(dict(index=domain,layer=0,png=str(domain_path),box_top_left=[0,0],box_size=[1792,1152],provenance='Reviewed native foliage observed source role; cached old appearances unchanged'))
        records[group['id']]=record
    if np.stack(list(domains.values())).sum(axis=0).max()>1:raise ValueError('New observed foliage domains overlap; explicit partition required')
    rows={r['index']:r for r in native['masks']};cache=dict(domains)
    def bitmap(index):
        if index not in cache:
            row=rows[index];x,y=row['box_top_left'];w,h=row['box_size'];a=np.zeros((1152,1792),bool)
            a[y:y+h,x:x+w]=np.asarray(Image.open(row['png']).convert('L'))>0;cache[index]=a
        return cache[index]
    exterior=masks['projections']['exterior'];changes=[]
    for assignment in exterior['assignments']:
        accepted=np.logical_or.reduce([bitmap(i) for i in assignment['mask_indices']])
        for index in assignment.get('exclude_mask_indices',[]):accepted&=~bitmap(index)
        for domain,array in domains.items():
            overlap=int((accepted&array).sum())
            if overlap:
                assignment.setdefault('exclude_mask_indices',[]).append(domain)
                assignment.update(exclusions_reviewed=True,exclusion_reason='Reviewed observed central foliage domains transfer source role to authored clumps; cached old model/materials unchanged')
                changes.append(dict(receiver=assignment.get('source_node',assignment.get('asset_group')),component=assignment.get('projection_component'),domain=domain,pixels=overlap,cached_geometry_or_material_changed=False));accepted&=~array
    receivers={'ground',*catalog['canonical_owners']}
    for record in records.values():
        node=record['group']['parts'][0]['node'];domain=record['domain']
        exterior['assignments'].append(dict(source_node=node,mask_indices=[domain],reviewed=True))
        exterior['occluder_constraints'].append(dict(source_node=node,mask_indices=[domain],reviewed=True,receiver_nodes=sorted(receivers-{node}),reason='Inferred volume may obscure foreign source receivers only inside own observed domain'))
    for rule in exterior['occluder_constraints']:
        if rule['source_node'].startswith(('foliage-','scenery-')):rule['receiver_nodes']=sorted(receivers-{rule['source_node']})
    parse_catalog(catalog,nodes-{'ground'})
    if catalog['groups'][:len(previous['groups'])]!=previous['groups'] or any(catalog['canonical_owners'][k]!=v for k,v in previous['canonical_owners'].items()):raise ValueError('Existing grouping changed')
    destination.mkdir(exist_ok=False)
    write_json(destination/'previous-catalog.json',previous);write_json(destination/'catalog.json',catalog);write_json(destination/'inventory.json',inventory);write_json(destination/'mask-inventory.json',native)
    masks['mask_inventory']=str(destination/'mask-inventory.json');write_json(destination/'source-masks.json',masks)
    grouping=dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(destination/'catalog.json'),inventory_sha256=sha(destination/'inventory.json'),evidence='Independent source/contact geometry reviews; existing groups unchanged; no user approval inherited')
    write_json(destination/'grouping-review.json',grouping)
    selector=OUT/'central-foliage-integration/selection.json';selected=json.loads(selector.read_text());selected['records'].update(records)
    if sha(reviewed_catalog())!=baseline:raise ValueError('Concurrent catalog modification')
    write_json(selector,selected);write_json(OUT/'ownership-revision/catalog.json',catalog);write_json(OUT/'ownership-revision/grouping-review.json',grouping)
    write_json(destination/'integration.json',dict(groups=len(catalog['groups']),previous_catalog_sha256=baseline,source_masks=str(destination/'source-masks.json'),inventory=str(destination/'inventory.json'),added=list(records),proposal_sha256=sha(proposal_path),source_role_transfers=changes,limitation='Source ownership metadata changed only; cached previous materials remain unchanged. Fresh stage/first-hit ownership review required.'))
    print('Registered',len(records),'central groups; total',len(catalog['groups']))

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--base',type=Path,required=True);parser.add_argument('--proposal',type=Path,required=True);parser.add_argument('--destination',type=Path,required=True)
    args=parser.parse_args();main(args.base.resolve(),args.proposal.resolve(),args.destination.resolve())
