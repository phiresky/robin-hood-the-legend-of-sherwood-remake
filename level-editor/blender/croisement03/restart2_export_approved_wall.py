"""Export the exactly approved wall into a private library without publishing."""
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT / 'level-editor/refinement'), str(ROOT / 'level-editor/refinement/blender')]
from render_slots import acquire, release
from texture_staging import verify_baked_geometry
from export_editor import export_asset_library


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    root = ROOT / 'level-editor/work/croisement03-refinement/restart2/wall-tree-integration-preflight'
    handoff_path = root / 'wall-texture-handoff/handoff.json'
    handoff = json.loads(handoff_path.read_text())
    receipt = json.loads((handoff_path.parent / 'receipt.json').read_text())
    assert sha(handoff_path) == receipt['handoff_sha256']
    output = root / 'wall-export-v1'
    assert not output.exists()
    acquire()
    try:
        verified = verify_baked_geometry(handoff)
        assert verified['geometry_verified'] and verified['model_sha256'] == receipt['model_sha256']
        output.mkdir()
        result = export_asset_library('Croisement03', output / '3d-assets',
            ROOT / 'level-editor/work/croisement03-refinement/baseline/Croisement03.rhp.json',
            asset_ids=['croisement03-southeast-stone-wall'])
        assert all(sha(path) == digest for path, digest in handoff['protected_files'].items())
        report = dict(status='Private approved wall export; browser, world-gameplay comparison and live integration pending',
                      handoff_sha256=sha(handoff_path), geometry_check=verified, export=result,
                      files={str(path.relative_to(output)): sha(path) for path in output.rglob('*') if path.is_file()},
                      limitations=['Only stone geometry and texture approved; adjacent ivy and final terrain remain separate unfinished owners.',
                                   'Tree candidate appearance and static/wind integration remain pending.',
                                   'No live index or map file changed.'])
        (output / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps({'output': str(output), 'files': len(report['files'])}))
    finally:
        release()


if __name__ == '__main__':
    main()
