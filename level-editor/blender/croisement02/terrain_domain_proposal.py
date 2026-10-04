"""Private source-space terrain ownership proposal and same-map fill pilot.

This is not a first-hit receiver domain: northern relief and missing vegetation
must be reconciled against final geometry before source projection/publication.
"""
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
from scipy.ndimage import distance_transform_edt, gaussian_filter
from scipy.sparse import coo_matrix
from scipy.sparse.linalg import spsolve

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/croisement02-refinement'
DEST = OUT / 'terrain-review'
# Source-artwork reservations, deliberately broad pending relief ownership.
BANK = [(0, 0), (1792, 0), (1792, 295), (1680, 280), (1580, 250),
        (1450, 265), (1350, 307), (1220, 350), (1130, 405), (1040, 444),
        (940, 455), (840, 590), (768, 645), (670, 658), (632, 560),
        (480, 500), (350, 430), (0, 240)]


def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def main():
    DEST.mkdir(exist_ok=True)
    source = OUT / 'baseline/covered.png'
    inventory_path = OUT / 'review-mask-inventory.json'
    layers_path = OUT / 'source-states/layers.json'
    source_rgb = np.asarray(Image.open(source).convert('RGB'))
    height, width = source_rgb.shape[:2]
    inventory = json.loads(inventory_path.read_text())['masks']
    layers = json.loads(layers_path.read_text())
    lookup = {(m.get('layer'), m.get('layer_index')): m['index'] for m in inventory if 'layer_index' in m}
    initial, applied = set(), set()
    for patch in layers['patches']:
        initial.update(lookup[(m['layer'], m['index'])] for m in patch['state']['old_masks'])
        applied.update(lookup[(m['layer'], m['index'])] for m in patch['state']['new_masks'])
    removed = np.zeros((height, width), bool)
    bitmaps, mask_evidence = {}, []
    for mask in inventory:
        if mask['index'] in applied - initial or not mask.get('png'):
            continue
        path = Path(mask['png'])
        bitmap = np.asarray(Image.open(path).convert('L')) > 0
        x, y = mask['box_top_left']
        h, w = bitmap.shape
        full = np.zeros_like(removed)
        full[max(0, y):min(height, y+h), max(0, x):min(width, x+w)] = bitmap[max(0, -y):min(h, height-y), max(0, -x):min(w, width-x)]
        bitmaps[mask['index']] = full
        removed |= full
        mask_evidence.append(dict(index=mask['index'], path=str(path), sha256=sha(path), pixels=int(full.sum())))
    reserve_image = Image.new('L', (width, height))
    ImageDraw.Draw(reserve_image).polygon(BANK, fill=255)
    reserve = np.asarray(reserve_image) > 0
    ground = ~removed & ~reserve
    domains = {'scenery-exclusion': removed, 'relief-reservation': reserve & ~removed,
               'proposed-observed-ground': ground}
    for name, bitmap in domains.items():
        Image.fromarray(bitmap.astype('uint8')*255).save(DEST / f'{name}.png')
    overlay = source_rgb.copy().astype(float)
    for bitmap, colour in [(removed, (230, 20, 220)), (reserve & ~removed, (255, 155, 0)), (ground, (0, 210, 230))]:
        overlay[bitmap] = overlay[bitmap]*.55 + np.array(colour)*.45
    Image.fromarray(overlay.astype('uint8')).save(DEST / 'ownership-overlay.png')
    # A bounded meadow pilot under the approved haystack. Select only intact
    # ground patches from this same meadow, with native exclusions enforced.
    target = bitmaps[124]
    candidates = []
    size = 24
    for y in range(985, height-size, 4):
        for x in range(820, 1120-size, 4):
            if ground[y:y+size, x:x+size].all():
                candidates.append((x, y, source_rgb[y:y+size, x:x+size].astype(float)))
    if not candidates:
        raise ValueError('No observed meadow donors')
    yy, xx = np.nonzero(target)
    result = source_rgb.copy().astype(float)
    # Harmonic continuation carries surrounding meadow lighting through the hole.
    labels = np.full(target.shape, -1, dtype=int)
    labels[yy, xx] = np.arange(len(yy))
    rows, columns, values = [], [], []
    rhs = np.zeros((len(yy), 3))
    for i, (y, x) in enumerate(zip(yy, xx)):
        rows.append(i); columns.append(i); values.append(4.)
        for ny, nx in [(y-1,x),(y+1,x),(y,x-1),(y,x+1)]:
            if target[ny,nx]:
                rows.append(i); columns.append(labels[ny,nx]); values.append(-1.)
            else:
                rhs[i] += source_rgb[ny,nx]
    matrix = coo_matrix((values,(rows,columns)),shape=(len(yy),len(yy))).tocsc()
    result[yy,xx] = spsolve(matrix,rhs)
    texture, weights = np.zeros_like(result), np.zeros(target.shape)
    selected = []
    window = np.outer(np.hanning(size+2)[1:-1],np.hanning(size+2)[1:-1])
    rng = np.random.default_rng(124)
    for y in range(int(yy.min())-size//2, int(yy.max())+1, size//2):
        for x in range(int(xx.min())-size//2, int(xx.max())+1, size//2):
            if not target[y:y+size, x:x+size].any():
                continue
            dx,dy,tile = candidates[int(rng.integers(len(candidates)))]
            detail = tile-gaussian_filter(tile,(2.5,2.5,0))
            texture[y:y+size,x:x+size] += detail*window[...,None]
            weights[y:y+size,x:x+size] += window
            selected.append(dict(target=[x,y,size,size],donor=[dx,dy,size,size]))
    texture /= np.maximum(weights[...,None],1e-8)
    result[target] += texture[target]*1.3
    result = np.clip(result,0,255)
    # Blend inside a 3px strip only; all observed ground stays byte-for-byte.
    weight = np.minimum(distance_transform_edt(target)/3, 1)[...,None]
    result = np.where(target[...,None], result*weight + source_rgb*(1-weight), source_rgb).astype('uint8')
    assert np.array_equal(result[~target], source_rgb[~target])
    Image.fromarray(result).save(DEST / 'haystack-hidden-ground-pilot.png')
    crop=(830,930,1100,1152)
    comparison=Image.new('RGB',(810,222))
    for i,im in enumerate([Image.fromarray(source_rgb),Image.fromarray(result),Image.fromarray(overlay.astype('uint8'))]):
        comparison.paste(im.crop(crop),(i*270,0))
    comparison.resize((1620,444)).save(DEST/'haystack-pilot-comparison.png')
    record = dict(version=1,status='proposal; not a certified receiver domain or published texture',
        source=str(source),source_sha256=sha(source),inventory_sha256=sha(inventory_path),layers_sha256=sha(layers_path),
        masks=mask_evidence,excluded_applied_only_masks=sorted(applied-initial),
        reservations={'northern_relief_polygon':BANK,'meaning':'Broad source-artwork reservation, not proposed geometry or a new exclusion from bank ownership.'},
        statistics={key:int(value.sum()) for key,value in domains.items()},
        shadows='Unmasked painted ground and shadows remain unchanged. Masked shadow overlap requires source review.',
        unresolved=['First-hit gating against final terrain and scenery geometry is absent.',
            'Northern relief reservation is broad; bank/ground ownership must be split using reviewed relief geometry.',
            'Mixed vegetation masks may include rock, fence and wood; excluded from ground regardless of final scenery owner.',
            'Mask complement can contain unmasked foliage or props; this proposal does not certify those pixels.',
            'Other mission states require separate revealed-state domains.',
            'Pilot is source-space only; integration with elevated ground UVs remains pending.'],
        pilot=dict(target_mask=124,method='Harmonic boundary lighting, overlapping 24px same-meadow high-frequency texture, 3px internal blend',
                   observed_outside_target_preserved=True,donors=selected,known_donor_count=len(candidates)),
        evidence={p.name:sha(p) for p in DEST.glob('*.png') if p.name!='haystack-source.png'},
        reference_evidence={str(p):sha(p) for p in [
            OUT/'integration-review/sheet.png', OUT/'integration-review/assembly.json',
            OUT/'scenery-domains/north-woodland-bank.png',
            OUT.parent/'map-compile/occlusion-depth-all-layers/croisement02.layer-0.occlusion-depth.mask-ids.png']},
        bank_scope_note='Current bank owns native parts 0–4; part 37 belongs to west rock. Historical bank domain 200 is reference only, not accepted receiver ownership.')
    (DEST/'proposal.json').write_text(json.dumps(record,indent=2)+'\n')
    print(json.dumps(record['statistics']))


if __name__ == '__main__':
    main()
