"""Render authored material traces; do not mutate source ownership or models."""
import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/active/croisement01'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('packet', type=Path)
    args = parser.parse_args()
    packet = args.packet.resolve()
    traces_path = packet / 'manual-traces.json'
    document = json.loads(traces_path.read_text())
    source_path = BASE / 'baseline/covered.png'
    assert hashlib.sha256(source_path.read_bytes()).hexdigest() == document['source_sha256']
    source = Image.open(source_path).convert('RGB')
    origin = (689, 122)
    native = np.asarray(Image.open(BASE / 'restart2/tree14-source-restoration-proposal-v1/wood-domain.png')) > 0
    masks = {key: np.zeros_like(native) for key in ('wood-candidate', 'foliage-candidate')}
    rows = []
    for trace in document['traces']:
        mask = Image.new('L', (native.shape[1], native.shape[0]))
        points = [(x-origin[0], y-origin[1]) for x, y in trace['points']]
        ImageDraw.Draw(mask).polygon(points, fill=255)
        selected = (np.asarray(mask) > 0) & native
        masks[trace['material']] |= selected
        rows.append(dict(id=trace['id'], material=trace['material'], selected_pixels=int(selected.sum())))
    for key, mask in masks.items():
        Image.fromarray(mask.astype(np.uint8)*255).save(packet / f'{key}-proposal.png')
    overlap = masks['wood-candidate'] & masks['foliage-candidate']
    assert not overlap.any(), 'Authored material traces overlap; resolve the conflict explicitly.'
    box = (689, 122, 916, 306)
    width, height, scale = box[2]-box[0], box[3]-box[1], 5
    plain = source.crop(box).resize((width*scale, height*scale), Image.Resampling.NEAREST)
    marked = plain.copy()
    draw = ImageDraw.Draw(marked)
    for index, trace in enumerate(document['traces']):
        points = [((x-box[0])*scale, (y-box[1])*scale) for x, y in trace['points']]
        color = '#55cfff' if trace['material'] == 'wood-candidate' else '#ed66ee'
        draw.line(points+[points[0]], fill=color, width=2)
        draw.text((points[0][0]+4, points[0][1]+2), str(index+1), fill='white')
    board = Image.new('RGB', (width*scale*2, height*scale+35), '#33333a')
    board.paste(plain, (0, 35))
    board.paste(marked, (width*scale, 35))
    draw = ImageDraw.Draw(board)
    draw.text((5, 5), 'Unmarked native source', fill='white')
    draw.text((width*scale+5, 5), 'Authored proposal traces: cyan wood / magenta foreground candidates', fill='white')
    board.save(packet / 'manual-material-proposals.png')
    report = dict(status='PRIVATE_SOURCE_PROPOSAL_NO_TRANSFER_OR_FILL',
                  trace_sha256=hashlib.sha256(traces_path.read_bytes()).hexdigest(),
                  source_sha256=document['source_sha256'], mask_origin=origin,
                  counts={key: int(mask.sum()) for key, mask in masks.items()},
                  conflicts=int(overlap.sum()), traces=rows)
    (packet / 'trace-review.json').write_text(json.dumps(report, indent=2)+'\n')
    print(report['counts'])


if __name__ == '__main__':
    main()
