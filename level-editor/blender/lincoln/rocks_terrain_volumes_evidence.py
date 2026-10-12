"""Write mask-review evidence sheets for the rocks/terrain lane (system python + PIL).

    python3 level-editor/blender/lincoln/rocks_terrain_volumes_evidence.py <asset-id> [round-1|round-2]

For every owned node with a reviewed row in rocks_terrain_volumes_masks.MASKS
it writes inspection/mask-review-<node>.png: the unmarked covered crop, the
accepted include-minus-exclusions domain (cyan) and the reviewed exclusions
(magenta), with the node's projected native outline (yellow).
"""
import json
import math
import sys
from pathlib import Path

from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from rocks_terrain_volumes_masks import MASKS  # noqa: E402

ROOT = HERE.parents[1] / 'work/needs-review/lincoln-refinement'
S35, C35 = math.sin(math.radians(35)), math.cos(math.radians(35))


def main():
    asset = sys.argv[1]
    rnd = sys.argv[2] if len(sys.argv) > 2 else 'round-1'
    workspace = ROOT / rnd / 'assets' / asset
    config = json.loads((workspace / 'workspace.json').read_text())
    working = json.loads(Path(config['source_mask_manifest']).read_text())
    inventory_path = (Path(config['source_mask_manifest']).parent / working['mask_inventory']).resolve()
    records = {r['index']: r for r in json.loads(inventory_path.read_text())['masks']}
    rows = {r['source_node']: r for r in working['projections']['exterior']['assignments'] if 'source_node' in r}
    inv = {o['source_node']: o for o in json.loads((ROOT / 'inventory/inventory.json').read_text())['objects']}
    source = Image.open(ROOT / 'source-states/covered.png').convert('RGB')
    out = workspace / 'inspection'
    out.mkdir(exist_ok=True)
    written = []
    for node in config['part_ids']:
        n = int(node.split('-')[-1])
        if n not in MASKS:
            continue
        row = rows[node]

        def layer(indices):
            m = Image.new('L', source.size, 0)
            for i in indices:
                r = records[i]
                bitmap = Image.open(inventory_path.parent / r['png']).convert('L').point(lambda v: 255 if v else 0)
                m.paste(bitmap, tuple(r['box_top_left']), bitmap)
            return m
        include = layer([i for i in row['mask_indices'] if i != 428])
        exclude = layer(row.get('exclude_mask_indices', []))
        box = include.getbbox() or (0, 0, 1, 1)
        rec = inv[node]
        m = rec['matrix_world']
        pts = []
        for v in rec['geometry']['vertices']:
            w = [sum(m[r][c] * v[c] for c in range(3)) + m[r][3] for r in range(3)]
            pts.append((w[0], -w[1] * S35 - w[2] * C35))
        xs = [p[0] for p in pts] + [box[0], box[2]]
        ys = [p[1] for p in pts] + [box[1], box[3]]
        crop = (max(0, int(min(xs)) - 20), max(0, int(min(ys)) - 20),
                min(source.width, int(max(xs)) + 20), min(source.height, int(max(ys)) + 20))
        base = source.crop(crop)
        over = base.copy()
        inc = include.crop(crop)
        exc = exclude.crop(crop)
        accepted = Image.composite(inc, Image.new('L', inc.size, 0), exc.point(lambda v: 0 if v else 255))
        over.paste(Image.new('RGB', base.size, (0, 255, 255)), (0, 0), accepted.point(lambda v: 90 if v else 0))
        over.paste(Image.new('RGB', base.size, (255, 0, 255)), (0, 0), exc.point(lambda v: 120 if v else 0))
        d = ImageDraw.Draw(over)
        for face in rec['geometry']['faces']:
            q = [(pts[i][0] - crop[0], pts[i][1] - crop[1]) for i in face]
            d.line(q + [q[0]], fill=(255, 255, 0), width=1)
        sheet = Image.new('RGB', (base.width * 2 + 8, base.height + 34), (24, 24, 24))
        sheet.paste(base, (0, 34))
        sheet.paste(over, (base.width + 8, 34))
        t = ImageDraw.Draw(sheet)
        t.text((4, 2), f'{node}: include {row["mask_indices"]} exclude {row.get("exclude_mask_indices", [])} '
                       f'crop {crop}', fill=(255, 255, 0))
        t.text((4, 16), 'left: unmarked covered.png | right: cyan accepted, magenta reviewed exclusions, '
                        'yellow native outline', fill=(200, 200, 200))
        scale = min(1.0, 1800 / sheet.width)
        if scale < 1:
            sheet = sheet.resize((int(sheet.width * scale), int(sheet.height * scale)))
        path = out / f'mask-review-{node}.png'
        sheet.save(path)
        written.append(str(path))
    print(json.dumps(written))


if __name__ == '__main__':
    main()
