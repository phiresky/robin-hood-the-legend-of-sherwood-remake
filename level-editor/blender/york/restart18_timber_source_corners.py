"""Record source observations before authoring the overlapping loose boards."""
import hashlib
import json
from pathlib import Path
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
SOURCE = ROOT / 'level-editor/work/york-refinement/restart2/pair-v14/assets/york-southwest-square-west-house/reference/source.png'
OUT = ROOT / 'level-editor/work/york-refinement/restart2/timber-props-source-corners-v2'
if OUT.exists():
    raise FileExistsError(OUT)
digest = hashlib.sha256(SOURCE.read_bytes()).hexdigest()
assert digest == '3d83bfab6b8ff1c27f87c82c0c034dc854df44b4e2322a09e7e083db57fb409e'
# Native pixel observations, not a solved mesh or claims about buried joints.
observations = [
    ('A', (1253, 949), 'leftmost pale board, lower exposed corner'),
    ('B', (1258, 952), 'leftmost pale board, lower opposite corner'),
    ('C', (1261, 952), 'next pale board, exposed end'),
    ('D', (1265, 956), 'next pale board, opposite exposed end'),
    ('E', (1267, 957), 'third pale board, exposed end'),
    ('F', (1272, 960), 'third pale board, opposite exposed end'),
    ('G', (1282, 959), 'rightmost pale board, exposed end'),
    ('H', (1287, 964), 'rightmost pale board, opposite exposed end'),
    ('I', (1307, 945), 'rightmost pale board, far end'),
    ('J', (1309, 948), 'rightmost pale board, far opposite corner'),
    ('K', (1265, 930), 'long crossing brown timber, upper end'),
    ('L', (1269, 930), 'long crossing brown timber, upper opposite end'),
    ('M', (1294, 955), 'long crossing brown timber, lower top corner'),
    ('N', (1292, 959), 'long crossing brown timber, lower side corner'),
    ('O', (1251, 946), 'short crossing timber, left exposed end'),
    ('P', (1274, 956), 'short crossing timber, right exposed tip'),
]
OUT.mkdir()
crop_box = (1243, 922, 1317, 973)
crop = Image.open(SOURCE).convert('RGB').crop(crop_box)
crop.save(OUT / 'original.png')
scale = 10
canvas = crop.resize((crop.width * scale, crop.height * scale), Image.Resampling.NEAREST)
draw = ImageDraw.Draw(canvas)
for label, (x, y), role in observations:
    px, py = ((x - crop_box[0]) * scale + scale // 2,
              (y - crop_box[1]) * scale + scale // 2)
    draw.ellipse((px-3, py-3, px+3, py+3), fill='red', outline='white')
    draw.text((px+5, py-10), label, fill='white', stroke_width=2, stroke_fill='black')
canvas.save(OUT / 'numbered-observations.png')
report = {
    'status': 'PRIVATE_SOURCE_OBSERVATIONS_NOT_GEOMETRY_READY',
    'source': str(SOURCE), 'source_sha256': digest,
    'asset': 'york-riverside-loose-planks', 'source_node': 'building-008',
    'crop': crop_box, 'display_scale': scale, 'native_pixel_uncertainty': 2,
    'corners': [{'id': label, 'pixel': xy, 'role': role, 'confidence': 'manual candidate'}
                for label, xy, role in observations],
    'visible_lower_bound': 'Four separated pale board-end faces plus two crossing brown timbers; buried board count not established.',
    'constraints': [
        'Preserve visible gaps between pale boards; do not replace the pile with a solid slab.',
        'Crossing timbers obscure intermediate board edges; do not score inferred continuations as observations.',
        'No visible fasteners establish a fixed pallet or attachment to a crane.',
        'Native mask005 includes foreign pixels; use actual observed faces for source ownership.',
        'Existing building086 support is near z109.7502; it constrains feet, not all top surfaces.',
    ],
    'next': ['Review each mark against the native art before using it as a fit target.',
             'Trace complete exposed edge segments and solve heights/contact order.',
             'Build separate board volumes, then inspect source projection and all eight views.'],
}
(OUT / 'observations.json').write_text(json.dumps(report, indent=2) + '\n')
print(OUT)
