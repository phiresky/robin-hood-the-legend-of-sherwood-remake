"""Compare retained stone samples with exact animated foreground without reassigning them."""
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

R = Path(__file__).resolve().parents[3]
B = R / 'level-editor/work/active/croisement03'
O = B / 'restart2/bank-foreground-crossings-v1'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    assert not O.exists()
    audit_path = B / 'restart2/bank-foreground-source-audit-v2/receipt.json'
    audit = json.loads(audit_path.read_text())
    for path, digest in audit['pins'].items():
        assert sha(Path(path)) == digest, path
    source = Image.open(B / 'baseline/covered.png').convert('RGBA')
    projection = json.loads((B / 'restart2/bank-convex-source-saved-v1/projection.json').read_text())
    protected = {tuple(p[:2]) for p in projection['protected_firsthits']}
    samples = protected | {tuple(p) for p in projection['accepted_pixels']}
    frames = []
    for frame in audit['frames']:
        canvas = Image.new('RGBA', source.size)
        canvas.paste(Image.open(frame['source']).convert('RGBA'), tuple(frame['bbox'][:2]))
        frames.append(canvas)
    arrays = [np.array(frame) for frame in frames]
    crossings = sorted((x, y) for x, y in samples if any(a[y, x, 3] for a in arrays))
    assert len(crossings) == audit['selected_stone_union_overlap'] == 12
    O.mkdir()
    records = []
    # Native source beside phase-specific foreground composite; no interpolation.
    cell = 128
    sheet = Image.new('RGB', (cell * 5, cell * len(crossings)), '#202020')
    draw = ImageDraw.Draw(sheet)
    for row, (x, y) in enumerate(crossings):
        states = [{'phase': i, 'rgba': a[y, x].tolist()} for i, a in enumerate(arrays) if a[y, x, 3]]
        records.append({'xy': [x, y], 'protected': (x, y) in protected,
                        'base_rgba': list(source.getpixel((x, y))), 'foreground_states': states,
                        'status': 'UNCHANGED_UNDERLAY_PENDING_PHASE_AND_OWNER_REVIEW'})
        views = [('base', source)]
        for state in states[:4]:
            phase = state['phase']
            composite = Image.alpha_composite(source, frames[phase])
            views.append((f'phase {phase}', composite))
        for col, (label, picture) in enumerate(views):
            box = (x - 7, y - 7, x + 8, y + 8)
            crop = picture.crop(box).resize((105, 105), Image.Resampling.NEAREST)
            sheet.paste(crop.convert('RGB'), (col * cell, row * cell + 20))
            draw.text((col * cell + 2, row * cell + 2), f'{x},{y} {label}', fill='white')
            draw.rectangle((col * cell + 49, row * cell + 69, col * cell + 55, row * cell + 75), outline='#ff00ff')
    sheet.save(O / 'source-phase-crossings.png')
    report = {'status': 'READ_ONLY_PHASE_EVIDENCE_NO_OWNERSHIP_CHANGE',
              'audit_sha256': sha(audit_path), 'protected_crossings': sum(r['protected'] for r in records),
              'new_crossings': sum(not r['protected'] for r in records), 'samples': records,
              'limits': ['Base artwork phase is not inferred from similar RGB.',
                         'Native foreground must retain priority; no leaf removal or sample reassignment.',
                         'This source composite is diagnostic artwork, not a rendered model or runtime proof.']}
    (O / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
    print({'crossings': len(records), 'protected': report['protected_crossings']})


if __name__ == '__main__':
    main()
