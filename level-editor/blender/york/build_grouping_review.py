"""Collect York ownership evidence into the shared interactive review gallery."""
import hashlib
import json
import sys
from pathlib import Path

from PIL import Image

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/active/york'


def sha(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def build():
    sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
    from build_review_gallery import build as shared_build
    review=OUT/'review'
    manifest=json.loads((review/'manifest.json').read_text())
    geometry=json.loads((review/'geometry.json').read_text())
    grounding=json.loads((OUT/'grounding/report.json').read_text()) if manifest['scene']['grounded'] else None
    if grounding and grounding['output_sha256'] != manifest['scene']['sha256']:
        raise ValueError('Stale grounding evidence')
    catalog=json.loads((ROOT/'level-editor/refinement/catalogs/york.json').read_text())
    checked=json.loads((OUT/'catalog-validation.json').read_text())
    exported=json.loads((OUT/'stage/export-report.json').read_text())
    if (checked['status']!='PASS' or checked['catalog_sha256']!=manifest['catalog_sha256'] or
            manifest['catalog_sha256']!=sha(ROOT/'level-editor/refinement/catalogs/york.json') or
            exported['source_sha256']!=manifest['scene']['sha256'] or
            exported['verified_assets']!=len(catalog['groups'])+1):
        raise ValueError('Stale or unverified grouping/export evidence')
    groups={g['id']:g for g in catalog['groups']}
    packets=review/'packets';packets.mkdir(exist_ok=True)
    artwork=Image.open(OUT/'baseline/covered.png').convert('RGB')
    items=[]
    for record in manifest['groups']:
        identity=record['id'];folder=packets/identity;folder.mkdir(exist_ok=True)
        model=OUT/'stage/map-assets/3d-assets/york'/identity/'model.glb'
        descriptor=model.with_name('asset.json')
        if not model.is_file():raise FileNotFoundError(model)
        paths={}
        for key,suffix in [('solid','game'),('east_solid','east'),('projection_errors','source')]:
            target=folder/(key+'.png')
            Image.open(review/'assets'/f'{identity}-{suffix}.jpg').save(target)
            paths[key]=str(target)
        context=folder/'context.png';artwork.crop(record['source_bounds']).save(context)
        ownership={'version':1,'scope':'grouping-only','asset_id':identity,'name':record['name'],
            'parts':groups.get(identity,{}).get('parts',[]),'source_nodes':record['sources'],
            'model_sha256':sha(model),'descriptor_sha256':sha(descriptor),
            'geometry_sha256':hashlib.sha256(json.dumps(geometry[identity],sort_keys=True).encode()).hexdigest()}
        floor=grounding.get('floor_extensions',{}).get(identity) if grounding else None
        if floor:
            ownership['floor_continuation']=floor
        own=folder/'ownership.json';own.write_text(json.dumps(ownership,indent=2)+'\n')
        validation=folder/'validation.json';validation.write_text(json.dumps({'status':'PASS','scope':'grouping-only',
            'asset_id':identity,'unique_source_ownership':True,'source_parts':len(record['sources']),
            'model_sha256':ownership['model_sha256']},indent=2)+'\n')
        items.append({'id':identity,'name':record['name'],'status':'ready-for-user','technical_eligible':True,
            'approval_scope':'grouping-only',**paths,'context':str(context),'model':str(model),
            'solid_label':'Selected geometry — game camera (35° orthographic)',
            'east_solid_label':record.get('east_solid_label','Selected geometry — east oblique'),
            'projection_errors_label':'Selected source parts (cyan overlay)',
            'ownership':str(own),'validation':str(validation),
            'notes':record['notes']+(' Foundation trimmed to its adjoining floor. '+floor['reason'] if floor else '')+' Source parts: '+', '.join(record['sources'])+'. '+
                'Review the grouping and name. Missing walls, rough proxy geometry and unfinished textures remain separate refinement work.'})
    path=review/'candidates.json'
    path.write_text(json.dumps({'map':'York','review_kind':'grouping','total_groups':len(groups),
        'solid_view_label':'Game camera','east_view_label':'East oblique',
        'supplemental_count':1,'items':items},indent=2)+'\n')
    # Build the exact revision first; only matching explicit grouping decisions
    # may hide a card. Browser drafts are never treated as submitted approvals.
    shared_build(path,review,pending_only=True)
    decisions=OUT/'grouping-decisions.json'
    if decisions.exists():
        from record_grouping_feedback import preview_only_update
        latest={row['asset_id']:row for row in json.loads(decisions.read_text())['decisions']}
        displayed=json.loads((review/'evidence.json').read_text())['items']
        archived={}
        for evidence in sorted((review/'history').glob('*/evidence.json')):
            for row in json.loads(evidence.read_text())['items']:
                archived[(row['id'],row['review_revision'])]=row
        approved=set();transfers=[]
        for row in displayed:
            decision=latest.get(row['id'],{})
            if decision.get('decision')!='approved':continue
            previous=decision['review_revision']
            if previous==row['review_revision']:
                approved.add(row['id'])
            elif preview_only_update(archived.get((row['id'],previous),{}),row):
                approved.add(row['id'])
                transfers.append({'asset_id':row['id'],'approved_review_revision':previous,
                    'display_review_revision':row['review_revision'],
                    'basis':'Only solid previews changed; ownership, model, geometry, validation, source images and notes match the explicit grouping approval.'})
        (review/'grouping-preview-updates.json').write_text(json.dumps(transfers,indent=2)+'\n')
        if approved:
            data=json.loads(path.read_text())
            for item in data['items']:
                if item['id'] in approved:item['user_approval']='approved grouping'
            path.write_text(json.dumps(data,indent=2)+'\n')
            shared_build(path,review,pending_only=True)


if __name__=='__main__':build()
