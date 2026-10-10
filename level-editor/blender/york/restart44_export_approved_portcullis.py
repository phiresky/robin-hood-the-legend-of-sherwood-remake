"""Export the two approved portcullis appearances into a private staging area."""
from pathlib import Path
import hashlib
import json
import shutil
import sys

ROOT = Path(__file__).resolve().parents[3]
WORK = ROOT / 'level-editor/work/york-refinement'
BASE = WORK / 'restart2'
OUT = BASE / 'restart44-approved-portcullis-export-v1'
sys.path[:0] = [str(ROOT / 'level-editor/refinement'), str(ROOT / 'level-editor/refinement/blender')]
from render_slots import acquire, release


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main(state):
    assert state in ['covered', 'raised']
    authority = BASE / 'approved-gate-staging-plan-v1/plan.json'
    assert sha(authority) == 'f2d79b2c98bc913c8b584fdd5fe1a2ba4c84113f102fbbe9751fd69a1e240c44'
    plan = json.loads(authority.read_text())
    member = next(m for m in plan['approved_members'] if m['asset_id'] == 'york-castle-portcullis--' + state)
    source = Path(member['model'])
    assert sha(source) == member['model_sha256']
    assert sha(member['source_evidence']) == member['source_evidence_sha256']
    assert shutil.disk_usage(ROOT).free >= 10 * 1024**3
    assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines()
                    if x.startswith('MemAvailable:'))) * 1024 >= 6 * 1024**3
    destination = OUT / state
    assert not destination.exists()
    import bpy
    from export_editor import export_asset_library
    from refinement_workspace import _geometry
    bpy.ops.wm.open_mainfile(filepath=str(source))
    scene = bpy.context.scene
    scene.name = 'york Refinement'
    scene.render.threads_mode = 'FIXED'
    scene.render.threads = 2
    gate = bpy.data.objects['scenery-york-castle-portcullis']
    before = _geometry(gate, protect_appearance=True)
    aid = 'york-castle-portcullis'
    audit = json.loads((BASE / 'gate-saved-contact-audit-v1/report.json').read_text())
    vs = audit['geometry_world']['gate_vertices']
    pivot = [(min(p[i] for p in vs) + max(p[i] for p in vs)) / 2 for i in range(2)] + [min(p[2] for p in vs)]
    collection = bpy.data.collections.new('york Working')
    scene.collection.children.link(collection)
    collection.objects.link(gate)
    old = {k: gate[k] for k in ['asset_group', 'asset_name', 'part_name'] if k in gate}
    gate['asset_group'] = aid
    gate['asset_name'] = 'Castle portcullis'
    gate['part_name'] = 'portcullis'
    assert gate['source_node'] == 'scenery-york-castle-portcullis'
    report = export_asset_library('york', destination / '3d-assets', WORK / 'baseline/york.rhp.json',
                                  standalone_pivots={aid: pivot}, asset_ids=[aid],
                                  include_hidden_objects=[gate.name])
    for key in ['asset_group', 'asset_name', 'part_name']:
        if key in old:
            gate[key] = old[key]
        else:
            del gate[key]
    assert _geometry(gate, protect_appearance=True) == before
    assert sha(source) == member['model_sha256']
    asset = destination / '3d-assets' / aid
    result = {'status': 'PRIVATE_EXPORT_PENDING_ROUNDTRIP_AND_BROWSER', 'state': state,
              'asset_id': aid, 'source_model': str(source), 'source_model_sha256': sha(source),
              'approval_authority': str(authority), 'approval_authority_sha256': sha(authority),
              'model_sha256': sha(asset / 'model.glb'), 'descriptor_sha256': sha(asset / 'asset.json'),
              'source_origin_scene': pivot, 'source_geometry_uv_materials_exact': True,
              'export_report': report, 'canonical_writes': False,
              'limitations': ['Static endpoint appearance only. No native timing or collision changes.',
                             'Roundtrip source surface and all 45 approved poses remain to verify.',
                             'Native artwork compositor and full editor integration remain separate.']}
    (destination / 'export-report.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result), flush=True)


if __name__ == '__main__':
    acquire(slots=2)
    try:
        main(sys.argv[sys.argv.index('--') + 1])
    finally:
        release()
