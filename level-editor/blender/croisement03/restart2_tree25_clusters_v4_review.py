"""Render the private rear-patch candidate using the unchanged native-first cameras."""
import hashlib
import json
from pathlib import Path
import shutil
import sys
import bpy
from PIL import Image, ImageChops

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT / 'level-editor/refinement'), str(ROOT / 'level-editor/refinement/blender')]
from render_slots import acquire, release
from render_multiview_asset import render


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def main():
    assert shutil.disk_usage(ROOT).free > 25 * 1024**3
    e = ROOT / 'level-editor/work/croisement03-refinement/restart2/texture-batch-v7/croisement03-tree-25/experiment'
    out = e / 'cluster-geometry-v4'
    report = json.loads((out / 'construction.json').read_text())
    assert sha(out / 'worker.blend') == report['model_sha256']
    assert not (out / 'actual').exists()
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(out / 'worker.blend'))
        render(e / 'views-grid8-v4.json', out / 'actual', modes=('textured', 'solid'), width=384)
        for mode in ('textured', 'solid'):
            sheet = Image.new('RGB', (1536, 768))
            for i in range(8):
                with Image.open(out / 'actual' / f'view-{i}-{mode}.png') as im:
                    sheet.paste(im.convert('RGB'), ((i % 4)*384, (i // 4)*384))
            sheet.save(out / 'actual' / f'{mode}.png')
        old = Image.open(e / 'native-rgb-control-v1/actual/view-0-textured.png').convert('RGBA')
        new = Image.open(out / 'actual/view-0-textured.png').convert('RGBA')
        comparison = Image.new('RGB', (1152,384))
        comparison.paste(old.convert('RGB'), (0,0)); comparison.paste(new.convert('RGB'), (384,0))
        comparison.paste(ImageChops.difference(old,new).convert('RGB'), (768,0))
        comparison.save(out / 'native-before-after-difference.png')
        (out / 'review-render.json').write_text(json.dumps(dict(model_sha256=sha(out / 'worker.blend'),
            native_view_index=0, actual_sheet_sha256=sha(out / 'actual/textured.png'),
            solid_sheet_sha256=sha(out / 'actual/solid.png'),
            native_pixel_difference_bbox=ImageChops.difference(old.convert('RGB'),new.convert('RGB')).getbbox(),
            status='Saved evidence awaiting visual and contact review; private candidate only'),indent=2)+'\n')
    finally:
        release()


if __name__ == '__main__':
    main()
