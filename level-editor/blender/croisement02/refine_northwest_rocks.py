"""Rebuild northwest rocks with source ownership and relief-height constraints."""
import json
import sys
import argparse
import uuid
import shutil
from pathlib import Path
import bpy
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent))
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha, write_json
from refinement_workspace import prepare, modified
from render_slots import acquire, release
from northwest_rock_completion import build
from audit_candidates import audit
from render_tree import render_workspace


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--redo',action='store_true');parser.add_argument('--destination',type=Path,default=OUT/'northwest-profile-round-1')
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    source_review = OUT/'northwest-rock-source-revision/ownership-review.json'
    if json.loads(source_review.read_text())['status']!='reviewed':
        raise ValueError('Manually review the source split before projection')
    # Freeze this worker's reviewed native grouping independently of later
    # additions to the whole-map catalog and inventory.
    frozen=OUT/'northwest-rock-source-revision'
    if not (frozen/'catalog.json').exists():
        reference=OUT/'scenery-round-2/assets/croisement02-northwest-rock-outcrop/reference'
        shutil.copy2(reference/'grouping.json',frozen/'catalog.json')
        shutil.copy2(reference/'grouping-review.json',frozen/'grouping-review.json')
    asset='croisement02-northwest-rock-outcrop'
    worker=args.destination.resolve()/'assets'/asset
    receipt=worker/'inspection/rock-ownership-revision.json'
    if worker.exists():
        if not args.redo:raise ValueError('Candidate already exists; do not silently rebuild it')
        latest={r['asset_id']:r for r in json.loads((OUT/'user-feedback.json').read_text())['records']}
        if latest.get(asset,{}).get('decision')=='approved':raise ValueError('Approved geometry is frozen')
        worker.rename(worker.with_name(worker.name+'-archive-'+uuid.uuid4().hex[:8]))
    catalog=json.loads((frozen/'catalog.json').read_text())
    owners={f"building-{p['obstacle']:03}":g for g in catalog['groups'] for p in g['parts'] if 'obstacle' in p}
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(OUT/'scenery-round-2/assets'/asset/'model.blend'))
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
            source_mask_manifest=OUT/'northwest-rock-source-revision/assignments.json',
            width=384,height=384,framing_padding=1.3,
            lighting=dict(toward_sun=[-.45,-.55,.70],ambient=.22,diffuse=.78,shadow_epsilon=.05))
        objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects
                 if o.type=='MESH' and o.get('asset_group')==asset]
        upper=next(o for o in objects if o.get('source_node')=='building-035')
        original_geometry={o.name:dict(vertices=len(o.data.vertices),faces=len(o.data.polygons)) for o in objects}
        parts=[build(upper)]
        middle=next(o for o in objects if o.get('source_node')=='building-036')
        parts.append(build(middle))
        parts[0]['original_input_mesh_counts']=original_geometry
        trace=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGB').crop((-110,-110,180,250)).resize((870,1080),Image.Resampling.NEAREST)
        draw=ImageDraw.Draw(trace)
        for part,color in zip(parts,['#ff5b5b','#00ffff','#ff66ff','#ffff55']):
            outline=[((x+110)*3,(y+110)*3) for x,y in part['source_profile']]
            draw.line(outline+[outline[0]],fill=color,width=2)
            draw.text(outline[0],part['source_node'],fill=color)
        trace.save(frozen/'profile-traces.png')
        modified(worker)
        (worker/'inspection').mkdir(exist_ok=True)
        report=dict(asset_id=asset,model_sha256=sha(worker/'model.blend'),parts=parts,
            status='geometry/source candidate; visual review pending',
            source_ownership_review=str(source_review),
            limitations=['Hidden surfaces await texture fill after geometry approval.',
                'Observed front ray depths are restored after the closed union. The northern footprint and hidden rear shape are inferred at native crest height; the smallest rock remains unchanged and the middle foot follows its observed source edge.',
                'Foreground native54 and overlapping native0/133 tree domains are excluded; joint coverage requires their geometry.',
                'Neighboring coarse proxies cannot occlude observed rock pixels; rock self-occlusion remains active.'])
        write_json(worker/'inspection/refinement.json',report)
        audit(worker)
        write_json(receipt,dict(model_sha256=report['model_sha256'],catalog_sha256=sha(frozen/'catalog.json'),
            source_ownership_sha256=sha(source_review)))
        render_workspace(worker,384,release_slot=False)
    finally: release()


if __name__=='__main__': main()
