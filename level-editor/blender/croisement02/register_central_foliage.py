"""Register independently reviewed native92 without inheriting approval."""
import json
import sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT,reviewed_catalog
from evidence_io import sha,write_json
from catalog_schema import parse_catalog
from central_candidates import validate_record


def main():
    base=OUT/'ground-plant-integration';destination=OUT/'central-foliage-integration'
    batch=OUT/'understory-candidates/native-92-v1';worker=OUT/'understory-round-1/assets/croisement02-shrub-92'
    joint=OUT/'ground-plant-candidates/joint-v2/native-shrub-92'
    baseline=sha(reviewed_catalog())
    if baseline!=sha(base/'catalog.json'):raise ValueError('Canonical base changed; explicitly rebase registration')
    destination.mkdir(exist_ok=False)
    catalog=json.loads((base/'catalog.json').read_text());inventory=json.loads((base/'inventory.json').read_text())
    masks=json.loads((base/'source-masks.json').read_text());native=json.loads((base/'mask-inventory.json').read_text())
    asset=worker.name;node='foliage-shrub-092';domain=471
    group=next(g for g in json.loads((batch/'catalog.json').read_text())['groups'] if g['id']==asset)
    model=sha(worker/'model.blend');inspection=worker/'inspection';sheet=inspection/'actual-materials/sheet.png'
    root_review=destination/'root-review-92.json'
    write_json(root_review,dict(reviewer='root',ready=True,user_approval=None,model_sha256=model,
        evidence='Root reviewed92 actual8 +bankjoint5 +native-scale context. Rounded low clump appropriate, native source/placement aligns; no large opaque squares or clipped volume. Ready for new geometry gallery registration with strict support/source guards, no approval.',
        files={str(p):sha(p) for p in (sheet,joint/'sheet.png',joint/'source-overlay.png',joint/'native-scale-context.png',joint/'evidence.json')}))
    write_json(inspection/'joint-neighbourhood.json',dict(model_sha256=model,evidence=str(joint/'evidence.json'),evidence_sha256=sha(joint/'evidence.json'),sheet=str(joint/'sheet.png'),sheet_sha256=sha(joint/'sheet.png')))
    preservation=batch/'source-rgb-validation.json'
    write_json(inspection/'visual-review.json',dict(model_sha256=model,sheet_sha256=sha(sheet),ready_for_geometry_review=True,user_approval=None,
        review='Self and independent root reviewed all eight actual views and bank joint. Irregular native leaf clusters; hidden arrangement inferred. Full-scene interactions remain separate.',
        preservation_evidence=str(preservation),preservation_evidence_sha256=sha(preservation),joint_neighbourhood_sha256=sha(inspection/'joint-neighbourhood.json'),root_review=str(root_review)))
    files=[worker/'model.blend',worker/'workspace.json',worker/'validation.json',sheet,inspection/'refinement.json',inspection/'saved-model-audit.json',inspection/'source-coverage/report.json',inspection/'visual-review.json',inspection/'joint-neighbourhood.json',preservation,root_review,joint/'evidence.json',joint/'sheet.png',joint/'native-scale-context.png',batch/'domain-471.png',batch/'shrub-92/support.json']
    for dependency in json.loads((joint/'evidence.json').read_text())['inputs']:
        path=Path(dependency['workspace'])/'model.blend'
        if sha(path)!=dependency['model_sha256']:raise ValueError('Joint dependency changed')
        files.append(path)
    record=dict(native_mask=92,domain=471,group=group,workspace=str(worker),model_sha256=model,user_approval=None,files={str(p):sha(p) for p in files})
    validate_record(record,group)
    if node in catalog['canonical_owners'] or any(r['index']==domain for r in native['masks']):raise ValueError('Source/domain collision')
    previous=json.loads(json.dumps(catalog));catalog['groups'].append(group);catalog['canonical_owners'][node]=asset
    additions=[r for r in json.loads((batch/'inventory/inventory.json').read_text())['objects'] if r['source_node']==node]
    if len(additions)!=1:raise ValueError('Expected single shrub mesh')
    inventory['objects']+=additions
    native['masks'].append(dict(index=domain,layer=0,png=str(batch/'domain-471.png'),box_top_left=[0,0],box_size=[1792,1152],provenance='Exact own native92 observed foliage'))
    exterior=masks['projections']['exterior'];exterior['assignments'].append(dict(source_node=node,mask_indices=[domain],reviewed=True))
    ground=next(a for a in exterior['assignments'] if a.get('source_node')=='ground')
    ground.setdefault('exclude_mask_indices',[]).append(domain)
    ground.update(exclusions_reviewed=True,exclusion_reason='Exact native92 leaves the ground source receiver for its new authored shrub; no cached material mutation claimed')
    receivers={'ground',*catalog['canonical_owners']}
    exterior['occluder_constraints'].append(dict(source_node=node,reviewed=True,receiver_nodes=sorted(receivers-{node}),mask_indices=[domain],reason='Inferred shrub volume blocks foreign sources only inside native92 observed pixels'))
    for rule in exterior['occluder_constraints']:
        if rule['source_node'].startswith(('foliage-','scenery-')):rule['receiver_nodes']=sorted(receivers-{rule['source_node']})
    parse_catalog(catalog,{r['source_node'] for r in inventory['objects']}-{'ground'})
    write_json(destination/'previous-catalog.json',previous);write_json(destination/'catalog.json',catalog);write_json(destination/'inventory.json',inventory);write_json(destination/'mask-inventory.json',native)
    masks['mask_inventory']=str(destination/'mask-inventory.json');write_json(destination/'source-masks.json',masks)
    write_json(destination/'selection.json',dict(status='New geometry review; no user approval inherited',records={asset:record}))
    grouping=dict(status='reviewed',reviewer='Codex',catalog_sha256=sha(destination/'catalog.json'),inventory_sha256=sha(destination/'inventory.json'),evidence='93 existing groups retained; native92 registered after independent bank joint/source review')
    write_json(destination/'grouping-review.json',grouping)
    if sha(reviewed_catalog())!=baseline:raise ValueError('Concurrent catalog update')
    write_json(OUT/'ownership-revision/catalog.json',catalog);write_json(OUT/'ownership-revision/grouping-review.json',grouping)
    write_json(inspection/'shrub-candidate.json',dict(workspace=str(worker),model_sha256=model,group=group,status='New geometry awaiting user approval'))
    write_json(destination/'integration.json',dict(groups=len(catalog['groups']),source_masks=str(destination/'source-masks.json'),inventory=str(destination/'inventory.json'),geometry_base=str(base/'input.blend'),append_worker=str(worker),status='Native92 registered; full physical scene is base93 plus exact92 worker; no full-scene duplicate written'))
    print('Registered native92:',len(catalog['groups']),'groups')

if __name__=='__main__':main()
