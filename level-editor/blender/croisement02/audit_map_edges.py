"""Find native tree domains touching the image boundary for completion review.

Contact is evidence to inspect, not proof that the saved geometry is cropped.
Never change a geometry approval or infer completion from an image-only audit.
"""
import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image

from catalog import OUT, reviewed_catalog, tree_workspace

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'refinement/blender'))
from evidence_io import sha, write_json


def main():
    catalog = reviewed_catalog()
    source = OUT / 'animation-references/composite-frame-0.png'
    width, height = Image.open(source).size
    approvals = {r['asset_id']: r for r in json.loads((OUT / 'user-feedback.json').read_text())['records']}
    packets = {r['mask']: r['packet'] for r in json.loads((OUT / 'forest-v4-sources/manifest.json').read_text())}
    rows = []
    for group in json.loads(catalog.read_text())['groups']:
        if 'wood_mask' not in group:
            continue
        worker = tree_workspace(group['wood_mask'])
        report = json.loads((worker / 'inspection/refinement.json').read_text())
        packet_path = Path(report['source_packet'] if 'source_packet' in report else packets[group['wood_mask']])
        packet = json.loads(packet_path.read_text())
        image_path = packet_path.parent / 'complete-source.png'
        alpha = np.asarray(Image.open(image_path).convert('RGBA'))[:, :, 3] > 127
        x, y, w, h = packet['native_bbox']
        if alpha.shape != (h, w):
            raise ValueError(f'Source packet dimensions disagree: {packet_path}')
        edges = {}
        for edge, index, axis in [('north', -y, 0), ('south', height - 1 - y, 0),
                                 ('west', -x, 1), ('east', width - 1 - x, 1)]:
            if 0 <= index < alpha.shape[axis]:
                count = int((alpha[index, :] if axis == 0 else alpha[:, index]).sum())
                if count:
                    edges[edge] = count
        receipts = []
        for name in ['boundary-completion.json', 'source-domain-revision.json']:
            path = worker / 'inspection' / name
            if path.exists():
                receipts.append(dict(path=str(path), sha256=sha(path)))
        rows.append(dict(asset_id=group['id'], worker=str(worker), model_sha256=sha(worker / 'model.blend'),
            source_packet_sha256=sha(packet_path), source_image_sha256=sha(image_path),
            native_foliage_edge_pixels=edges, revision_receipts=receipts,
            latest_user_decision=approvals.get(group['id'], {}).get('decision'),
            status='Inspect complete saved geometry and transition' if edges or receipts else 'No foliage contact in this source domain',
            limitation='Does not inspect wood, hidden crown continuation, or other scenery; source contact alone does not prove a flat model cutoff.'))
    destination = OUT / 'map-edge-review'
    destination.mkdir(exist_ok=True)
    write_json(destination / 'inventory.json', dict(catalog_sha256=sha(catalog), source_sha256=sha(source),
        map_size=[width, height], trees=rows))
    print(f'{sum(bool(row["native_foliage_edge_pixels"]) for row in rows)} of {len(rows)} tree domains touch the image edge')


if __name__ == '__main__':
    main()
