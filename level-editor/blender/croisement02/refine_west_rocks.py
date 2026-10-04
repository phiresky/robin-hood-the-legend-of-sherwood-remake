"""Restore source-owned rock surfaces and group their small eastern ledge."""
import json
import sys
import argparse
import uuid
import shutil
from pathlib import Path
import bpy

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent))
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT, reviewed_catalog
from evidence_io import sha, write_json
from refinement_workspace import prepare, modified
from render_slots import acquire, release
from revise_relief import sculpt
from audit_candidates import audit
from render_tree import render_workspace


def regroup():
    path = reviewed_catalog()
    catalog = json.loads(path.read_text())
    groups = {g['id']:g for g in catalog['groups']}
    west = groups['croisement02-west-rock-outcrop']
    bank = groups['croisement02-north-woodland-bank']
    if any(p['obstacle']==37 for p in west['parts']): return
    latest = {r['asset_id']:r for r in json.loads((OUT/'user-feedback.json').read_text())['records']}
    for asset in (west['id'],bank['id']):
        if latest.get(asset,{}).get('decision')=='approved': raise ValueError('Approved grouping is frozen')
    original = json.loads(json.dumps(catalog))
    bank['parts'] = [p for p in bank['parts'] if p['obstacle']!=37]
    west['parts'].append(dict(obstacle=37,name='West Rock Outcrop eastern low ledge 037'))
    west['parts'].sort(key=lambda p:p['obstacle'])
    assert sorted(p['obstacle'] for g in catalog['groups'] for p in g['parts'])==list(range(150))
    for group in catalog['groups']:
        if group['id'] not in {west['id'],bank['id']}:
            assert group==next(g for g in original['groups'] if g['id']==group['id'])
    write_json(OUT/'west-rock-source-revision/previous-catalog.json',original)
    write_json(path,catalog)
    write_json(path.parent/'grouping-review.json',dict(status='reviewed',reviewer='Codex',
        catalog_sha256=sha(path),inventory_sha256=sha(OUT/'forest-v4-inventory/inventory.json'),
        evidence=['Native37 projects onto the small rightmost rock at x220–271,y353–383, contiguous with rock38.',
                  'Only the two unapproved rock/bank groups changed; all150native parts still have one owner.']))


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--redo',action='store_true')
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    source_review = OUT/'west-rock-source-revision/ownership-review.json'
    if json.loads(source_review.read_text())['status']!='reviewed':
        raise ValueError('Manually review the source split before projection')
    regroup()
    # Freeze this worker's reviewed native grouping independently of later
    # additions to the whole-map catalog and inventory.
    frozen=OUT/'west-rock-source-revision'
    if not (frozen/'catalog.json').exists():
        shutil.copy2(reviewed_catalog(),frozen/'catalog.json')
        shutil.copy2(reviewed_catalog().parent/'grouping-review.json',frozen/'grouping-review.json')
    asset='croisement02-west-rock-outcrop'
    worker=OUT/'scenery-round-3/assets'/asset
    receipt=worker/'inspection/rock-ownership-revision.json'
    if receipt.exists():
        if not args.redo:raise ValueError('Candidate already exists; do not silently rebuild it')
        latest={r['asset_id']:r for r in json.loads((OUT/'user-feedback.json').read_text())['records']}
        if latest.get(asset,{}).get('decision')=='approved':raise ValueError('Approved geometry is frozen')
        worker.rename(worker.with_name(worker.name+'-archive-'+uuid.uuid4().hex[:8]))
    catalog=json.loads((frozen/'catalog.json').read_text())
    owners={f"building-{p['obstacle']:03}":g for g in catalog['groups'] for p in g['parts'] if 'obstacle' in p}
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(OUT/'forest-v4-input.blend'))
        bpy.context.preferences.filepaths.save_version=0
        for obj in bpy.data.collections['Croisement02 Working'].all_objects:
            if obj.type=='MESH' and obj.get('source_node') in owners:
                group=owners[obj['source_node']]
                obj['asset_group']=group['id'];obj['asset_name']=group['name']
        prepare(worker,asset_id=asset,scene_name='Croisement02 Refinement',
            collection_name='Croisement02 Working',
            source_path=OUT/'animation-references/composite-frame-0.png',
            grouping_manifest=frozen/'catalog.json',inventory_path=OUT/'forest-v4-inventory/inventory.json',
            review_path=frozen/'grouping-review.json',
            source_mask_manifest=OUT/'west-rock-source-revision/assignments.json',
            width=384,height=384,framing_padding=1.3,
            lighting=dict(toward_sun=[-.45,-.55,.70],ambient=.22,diffuse=.78,shadow_epsilon=.05))
        objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects
                 if o.type=='MESH' and o.get('asset_group')==asset]
        parts=[sculpt(o) for o in objects]
        modified(worker)
        (worker/'inspection').mkdir(exist_ok=True)
        report=dict(asset_id=asset,model_sha256=sha(worker/'model.blend'),parts=parts,
            status='geometry/source candidate; visual review pending',
            source_ownership_review=str(source_review),
            limitations=['Hidden surfaces await texture fill after geometry approval.',
                'Crest/source silhouette and the western boundary boulder still require geometry refinement; rejected raster-driven crest deformations are archived.',
                'Foreground native56/61 vegetation is excluded from rock texture; joint coverage still requires its geometry.',
                'Neighboring coarse proxies cannot occlude observed rock pixels; rock self-occlusion remains active.'])
        write_json(worker/'inspection/refinement.json',report)
        audit(worker)
        write_json(receipt,dict(model_sha256=report['model_sha256'],catalog_sha256=sha(frozen/'catalog.json'),
            source_ownership_sha256=sha(source_review)))
        render_workspace(worker,384,release_slot=False)
    finally: release()


if __name__=='__main__': main()
