"""Prepare five approved imports without changing the canonical catalog or scene."""
import json
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from texture_staging import validate_texture_handoff
from review_evidence import sha


def write(path, data):
    path.write_text(json.dumps(data, indent=2) + '\n')


def main():
    out = ROOT / 'level-editor/work/croisement03-refinement'
    root = out / 'restart2'
    destination = root / 'integration-round1'
    destination.mkdir(exist_ok=True)
    catalog = json.loads((out / 'catalog.json').read_text())
    handoffs, metadata = [], []
    for suffix in ['timber-bridge', 'stream-fallen-log', 'fern-35', 'fern-76', 'southwest-firewood-stack']:
        asset = 'croisement03-' + suffix
        archived = root / 'approval-round1/assets' / asset
        item = json.loads((archived / 'gallery-item.json').read_text())
        worker = Path(item['model']).parent
        assert sha(worker / 'model.blend') == sha(archived / 'model.blend')
        target = archived / 'workspace.json'
        if target.exists():
            assert sha(target) == sha(worker / 'workspace.json')
        else:
            shutil.copy2(worker / 'workspace.json', target)
        metadata.append(dict(asset_id=asset, source=str(worker / 'workspace.json'),
                             destination=str(target), sha256=sha(target)))
        packet = root / 'texture-round1' / asset
        number = 2 if suffix == 'southwest-firewood-stack' else 1
        handoff = validate_texture_handoff(packet / 'review-manifest.json', asset,
                     root / f'texture-review-round{number}/decisions.json', packet / 'decisions.json')
        if asset not in {group['id'] for group in catalog['groups']}:
            original = json.loads((worker / 'reference/grouping.json').read_text())
            matches = [group for group in original['groups'] if group['id'] == asset]
            assert len(matches) == 1
            catalog['groups'].append(matches[0])
            handoff['new_scenery_part'] = True
        handoffs.append(handoff)
    write(destination / 'workspace-metadata.json', dict(scope='Exact existing original worker metadata; approved model/camera/mask/displayed bytes unchanged', files=metadata))
    write(destination / 'catalog.json', catalog)
    write(destination / 'approved-handoffs.json', handoffs)
    write(destination / 'plan.json', dict(baseline=str(out / 'croisement03-grouped.blend'),
         baseline_sha256=sha(out / 'croisement03-grouped.blend'),
         catalog=str(destination / 'catalog.json'), catalog_sha256=sha(destination / 'catalog.json'),
         handoffs=str(destination / 'approved-handoffs.json'),
         handoffs_sha256=sha(destination / 'approved-handoffs.json'),
         source_level=str(out / 'baseline/Croisement03.rhp.json'),
         source_level_sha256=sha(out / 'baseline/Croisement03.rhp.json'),
         output=str(destination / 'stage-v1'),
         scope='Private partial scene and five standalone exports. Full scene integration remains incomplete; no publication.'))
    print(json.dumps(dict(approved_imports=len(handoffs), catalog_groups=len(catalog['groups']))))


if __name__ == '__main__':
    main()
