"""Audit reachable crown content against saved actual-editor evidence."""
import hashlib
import json
import struct
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/active/croisement03/restart2'
BROWSER = BASE / 'combined-trio-full-editor-browser-v2/browser-v1'
OUT = BASE / 'combined-trio-stage-v2/browser-crown-diagnosis-v1'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    assert not OUT.exists()
    state = json.loads((BROWSER / 'failure-browser-state.json').read_text())['c3']
    http = json.loads((BROWSER / 'actual-asset-http-responses.json').read_text())
    rows = []
    for number in (12, 13, 14):
        identity = f'croisement03-tree-{number}'
        model = BASE / f'combined-trio-stage-v2/library/3d-assets/croisement03/{identity}/model.glb'
        blob = model.read_bytes()
        doc = json.loads(blob[20:20 + struct.unpack_from('<I', blob, 12)[0]])
        reached = set()
        def visit(index):
            assert index not in reached
            reached.add(index)
            for child in doc['nodes'][index].get('children', []):
                visit(child)
        for index in doc['scenes'][doc.get('scene', 0)]['nodes']:
            visit(index)
        def triangles(index):
            return sum(doc['accessors'][p['indices']]['count'] // 3 for p in doc['meshes'][index]['primitives'])
        reachable = sum(triangles(node['mesh']) for index, node in enumerate(doc['nodes']) if index in reached and 'mesh' in node)
        owner = next(row for row in state['owners'] if row['id'] == identity)
        actual = sum(mesh['triangles'] for mesh in owner['meshes'])
        assert actual == reachable and all(mesh['visible'] for mesh in owner['meshes'])
        digest = sha(model)
        served = next(row for row in http if row['path'].endswith(f'/{identity}/model.glb'))
        assert served['sha256'] == digest
        removed = [{'name': node['name'], 'triangles': triangles(node['mesh'])} for index, node in enumerate(doc['nodes']) if index not in reached and 'mesh' in node]
        rows.append({'tree': number, 'model_sha256': digest, 'http_exact': True, 'runtime_triangles': actual,
                     'reachable_glb_triangles': reachable, 'all_runtime_meshes_visible': True, 'unreachable_meshes': removed})
    evidence = [BROWSER / 'c3-trio-native-initial.png', BROWSER / 'c3-trio-oblique-90.png',
                BROWSER / 'failure-browser-state.json', BROWSER / 'actual-asset-http-responses.json',
                BASE / 'static-pair-publication-audit-v1/static-body-v1/receipt.json',
                BASE / 'static-crown-pair-review-v3/root-review.json',
                BASE / 'approved-hub-textures-v1/static-pair-export-v1/root-review.json',
                BASE / 'trio-tree-integration-v1/static-leaf-source-proposal-v2/native-wide.png']
    for number in (12, 14):
        evidence.extend([BASE / f'approved-hub-textures-v1/static-pair-export-v1/tree{number}/webgl-v1/actual8.png',
                         BASE / f'approved-hub-textures-v1/static-pair-export-v1/tree{number}/report.json'])
    report = {
        'status': 'HOLD_COMBINED_CROWN_APPEARANCE', 'rows': rows,
        'visual_review': 'Independently viewed native-initial and oblique90, both approved full-export actual8 sheets, and native-wide artwork. Current Tree12/14 have conspicuously exposed upper branches and much less side crown volume than full approved exports; Tree13 retains its static Local75 crown.',
        'diagnosis': 'Runtime triangle counts exactly equal reachable staged GLB content and HTTP model bytes match. No retained static mesh omission is evidenced. Static-body separation intentionally disconnected whole Arbre06 fragment subtrees; those subtrees contain inferred crown volume as well as observed frame0 proxy faces.',
        'disconnected_roles': {'tree12': {'native_triangles': 1900, 'inferred_triangles': 7192},
                               'tree14': {'native_triangles': 868, 'inferred_triangles': 4146}},
        'approval_scope': 'Full-export review explicitly says both crown roles visible and excludes animation-role approval. Static-body receipt is structural PASS with publication HOLD. Full static geometry approval does not prove appearance equivalence of the separated combined derivative.',
        'next_step': 'Separate proven observed dynamic-frame0 faces from inferred volume with exact source provenance. Establish an explicit retained-volume role that cannot duplicate or occlude native animation over all14 phases, then recheck native first hits, static source ownership and actual obliques. Do not reconnect the whole frame0 subtree or invent animation.',
        'limitations': ['No pixel first-hit audit or replacement geometry produced in this CPU diagnosis.',
                        'Pale branch appearance is consistent with newly exposed retained bark; no claim of texture-byte loss is made.'],
        'pins': {str(path): sha(path) for path in evidence}, 'canonical_written': False,
    }
    OUT.mkdir()
    (OUT / 'diagnosis.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'status': report['status'], 'rows': rows, 'report': str(OUT / 'diagnosis.json')}))


if __name__ == '__main__':
    main()
