"""Freeze reviewed artwork ownership for the paired market houses."""
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageFilter

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/york-refinement'
DEST = OUT / 'restart2/pair-v16/source-authority-v1'
if DEST.exists():
    raise FileExistsError(DEST)
source_path = OUT / 'baseline/covered.png'
source = Image.open(source_path).convert('RGB')
inventory_path = OUT / 'baseline/masks/manifest.json'
inventory = json.loads(inventory_path.read_text())
records = {r['index']: r for r in inventory['masks']}
inputs = [source_path, inventory_path]

def native(index):
    record = records[index]
    path = inventory_path.parent / record['png']
    inputs.append(path)
    result = Image.new('L', source.size)
    result.paste(Image.open(path).convert('L'), record['box_top_left'])
    return np.array(result) > 0

# The native bay mask ends at the timber sill. Its already reviewed extension
# follows the visible two lower wall faces down to the original painted foot.
# These pixels are source traces, never candidate render or first-hit selections.
bay_path = OUT / 'geometry-pass-01/narrow-house-domain-mask.png'
inputs.append(bay_path)
bay = Image.new('L', source.size)
bay.paste(Image.open(bay_path).convert('L'), (550, 1205))
bay = np.array(bay) > 0
house = native(216) & ~bay
exclusions = {
    198: 'Foreground shop lower facade, distinct from the recessed adjoining doorway.',
    199: 'Foreground shop component overlapping the main house occlusion gate.',
    200: 'Foreground shop tiled roof and upper facade.',
    204: 'Small projecting shop awning beam across the recessed doorway.',
    214: 'Separate upper-right neighboring roof.'}
for index in exclusions:
    house &= ~native(index)

DEST.mkdir(parents=True)
crop = (550, 1100, 790, 1460)
entries = []
assignments = []
review = []
colors = [(255, 0, 255), (0, 255, 255)]
picture = np.array(source).astype(float)
for index, (asset, domain) in enumerate([
        ('york-market-southeast-tall-narrow-house', bay),
        ('york-southwest-square-west-house', house)]):
    # A single source pixel at the trace edge is ambiguous at the artwork's
    # antialias transition. Keep it separately as uncertain evidence rather
    # than treating background mixture as known facade texture.
    traced = Image.fromarray(domain.astype('uint8') * 255)
    confident = traced.filter(ImageFilter.MinFilter(3))
    boundary = np.array(traced) > np.array(confident)
    confident.crop(crop).save(DEST / f'{asset}.png')
    Image.fromarray(boundary.astype('uint8') * 255).crop(crop).save(DEST / f'{asset}-uncertain-boundary.png')
    traced.crop(crop).save(DEST / f'{asset}-full-trace.png')
    owned = np.array(confident) > 0
    picture[owned] = picture[owned] * .55 + np.array(colors[index]) * .45
    entries.append({'index': index, 'kind': 'authored-artwork-domain',
                    'box_top_left': [550, 1100], 'box_size': [240, 360],
                    'png': f'{asset}.png'})
    assignments.append({'reviewed': True, 'asset_group': asset,
                        'mask_indices': [index],
                        'review_evidence': 'Original artwork, native semantic-mask study and final source-domain overlay; no candidate visibility used.'})
    review.append({'asset_id': asset, 'full_trace_pixels': int(domain.sum()),
                   'confident_pixels': int(owned.sum()),
                   'uncertain_boundary_pixels': int(boundary.sum())})
Image.fromarray(picture.astype('uint8')).crop(crop).resize((720, 1080), Image.Resampling.NEAREST).save(DEST / 'source-domain-overlay.png')
sha = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
(DEST / 'inventory.json').write_text(json.dumps({'version': 1,
    'index_namespace': 'Local authored domain indices, NOT native mask IDs.',
    'masks': entries}, indent=2) + '\n')
(DEST / 'source-masks.json').write_text(json.dumps({'version': 1,
    'mask_inventory': str(DEST / 'inventory.json'),
    'projections': {'pair-reviewed-source': {
        'source_sha256': sha(source_path), 'state': 'Original covered native artwork',
        'assignments': assignments}}}, indent=2) + '\n')
(DEST / 'review.json').write_text(json.dumps({
    'status': 'Source ownership reviewed; geometry and user approval are separate.',
    'native_references': [216, 217], 'foreground_exclusions': exclusions,
    'input_hashes': {str(path.relative_to(ROOT)): sha(path) for path in inputs},
    'domains': review,
    'method': 'Artwork-reviewed native masks and the existing source-traced bay lower storeys. One-pixel uncertain trace boundary retained separately. No candidate geometry or first-hit selection.',
    'caveat': 'Occlusion gates overlap. Canopy masks were not blindly subtracted because they extend behind real roof pixels. Broad prior domain-audit scores remain diagnostics, not texture authority.',
    'geometry_unchanged': True}, indent=2) + '\n')
print(DEST)
