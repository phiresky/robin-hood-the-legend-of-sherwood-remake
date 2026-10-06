"""Freeze small native gate endpoint/transition crops before proposing geometry."""
import hashlib
import json
import sys
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
WORK = ROOT / 'level-editor/work/york-refinement'
SOURCE = WORK / 'geometry-pass-01/native-state-source-v1'
DEST = WORK / 'restart2' / (sys.argv[1] if len(sys.argv) > 1 else 'gate-source-study-v1')
if DEST.exists():
    raise FileExistsError(DEST)
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
manifest = json.loads((SOURCE / 'manifest.json').read_text())
level = WORK / 'baseline/york.rhp.json'
assert sha(level) == manifest['source_level_sha256']
level_data = json.loads(level.read_text())
doors = []
for building_index, building in enumerate(level_data['buildings']):
    kind, payload = next(iter(building.items()))
    for local_index, door in enumerate(payload.get('doors', [])):
        doors.append({'index':len(doors),'building_index':building_index,
                      'local_index':local_index,'kind':kind,'record':door})
gate_doors = [doors[i] for i in (13,14)]
assert all(d['kind']=='StandaloneDoors' and d['building_index']==5 for d in gate_doors)
assert all(d['record']['locked_pc'] and not d['record']['locked_pc_after_patch'] for d in gate_doors)
records = {r['id']: r for r in manifest['records']}
crop = (2250, 780, 2470, 1030)
base_path = WORK / 'source-states-complete/revealed.png'
base = Image.open(base_path).convert('RGBA').crop(crop)
DEST.mkdir(parents=True)
sheet = Image.new('RGB', (880, 560), '#182028')
draw = ImageDraw.Draw(sheet)
states = []
for index, frame_index in enumerate((None, 0, 22, 44)):
    composite = base.copy()
    frame_receipts = []
    for patch in ('patch-000', 'patch-004'):
        record = records[patch]
        action = 'PatchInitial' if frame_index is None else 'PatchTransition'
        row = next(r for r in record['rows'] if r['action'] == action)
        frame = row['frames'][0 if frame_index is None else frame_index]
        path = SOURCE / frame['image']
        assert sha(path) == frame['sha256']
        rgba = Image.open(path).convert('RGBA')
        composite.alpha_composite(rgba, (frame['bbox'][0]-crop[0], frame['bbox'][1]-crop[1]))
        frame_receipts.append({'patch': patch, **frame})
    name = 'initial' if frame_index is None else f'transition-{frame_index:02d}'
    path = DEST / f'{name}.png'
    composite.save(path)
    sheet.paste(composite.convert('RGB'), (index*220, 25))
    draw.text((index*220+4, 6), f'Native {name}', fill='white')
    # Enlarged raw frames use alpha over neutral gray, never interpret key RGB as paint.
    tile = Image.new('RGBA', (110, 130), '#444444')
    for j, fr in enumerate(frame_receipts):
        tile.alpha_composite(Image.open(SOURCE/fr['image']).convert('RGBA'), (j*50+4, 5))
    sheet.paste(tile.resize((220,260), Image.Resampling.NEAREST).convert('RGB'), (index*220,300))
    states.append({'name':name,'image':path.name,'sha256':sha(path),'frames':frame_receipts})
sheet.save(DEST/'native-source-sheet.png')
report = {
    'status':'Source study only; no geometry approval or live changes',
    'crop':crop,'source_projection':'Native original artwork direction, identical crop in every column',
    'source_level_sha256':sha(level),'source_manifest_sha256':sha(SOURCE/'manifest.json'),
    'base_image':str(base_path.relative_to(ROOT)),'base_sha256':sha(base_path),
    'composition':'Frozen revealed background plus only gate/mechanism frames at recorded offsets; independent patches are not assumed synchronized or jointly reachable.',
    'native_records':{k:records[k]['record'] for k in ('patch-000','patch-004')},
    'door_index_convention':'Sequential non-lift doors in native building record order',
    'gate_doors':gate_doors,
    'states':states,
    'behavior':[
        'Applying a background-integrated patch paints its last transition frame into the background before disabling an invalid final animation.',
        'Patch000 therefore retains its 11x14 final transition remnant; no final animation does not mean invisible final appearance.',
        'Patch000 swaps native door rights 13 and 14, but changes no listed masks or sight obstacles.',
        'Patch004 starts with a transparent 1x1 frame and ends with visible chain/winch artwork; it is an independent definitive patch.'
    ],
    'pending':[
        'Validate gatehouse geometry depth/contact against native door endpoints before inferring the largely hidden gate plane.',
        'Determine which mechanism surfaces belong to permanent background versus the animated replacement.',
        'Transition timing, mission reachability and gameplay integration remain separate from endpoint geometry.'
    ]
}
(DEST/'manifest.json').write_text(json.dumps(report,indent=2)+'\n')
print(DEST)
