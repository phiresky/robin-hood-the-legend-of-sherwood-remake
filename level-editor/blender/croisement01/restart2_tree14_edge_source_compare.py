"""Compare unresolved native edge pixels with the exact saved-model render."""
import argparse
import hashlib
import json
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/croisement01-refinement/restart2'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('workspace', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    worker = args.workspace.resolve()
    evidence_path = worker / 'inspection/actual-materials/evidence.json'
    evidence = json.loads(evidence_path.read_text())
    assert sha(worker / 'model.blend') == evidence['model_sha256']
    packet_path = BASE / 'tree14-native-crown-animation-source-v3/remaining-forty-source-pixels.json'
    packet = json.loads(packet_path.read_text())
    # The review input is deliberately explicit; do not derive ownership from
    # rendered alpha, animated overlap, or a color threshold.
    args.output.mkdir(exist_ok=False)
    source = Image.open(BASE.parent / 'baseline/covered.png').convert('RGBA')
    actual_path = worker / 'inspection/native-source/actual.png'
    actual = Image.open(actual_path).convert('RGBA')
    left, top, right, bottom = evidence['source_crop']
    assert actual.size == (right - left, bottom - top)
    crop = (806, 263, 855, 318)
    native = source.crop(crop)
    saved = actual.crop((crop[0] - left, crop[1] - top,
                         crop[2] - left, crop[3] - top))
    alpha = saved.getchannel('A').convert('RGBA')
    views = [('Unmarked native artwork', native),
             ('Saved material, native camera', saved),
             ('Rendered alpha; antialiasing included', alpha),
             ('Artwork with saved-model overlay', Image.alpha_composite(native, saved))]
    scale = 6
    width, height = native.size
    board = Image.new('RGB', (width * scale * 4, height * scale + 30), '#55555d')
    draw = ImageDraw.Draw(board)
    for index, (label, view) in enumerate(views):
        enlarged = view.resize((width * scale, height * scale), Image.Resampling.NEAREST)
        board.paste(enlarged, (index * width * scale, 30), enlarged)
        draw.text((index * width * scale + 4, 5), label, fill='white')
    board.save(args.output / 'source-model-silhouette.png')
    marked = native.resize((width * scale, height * scale), Image.Resampling.NEAREST)
    marking = ImageDraw.Draw(marked)
    for record in packet['pixels']:
        x, y = record['pixel']
        px, py = (x - crop[0]) * scale, (y - crop[1]) * scale
        marking.rectangle((px, py, px + scale - 1, py + scale - 1), outline='#ff00ff')
    marked.save(args.output / 'forty-pixel-locations.png')
    report = dict(status='UNRESOLVED_VISUAL_COMPARISON_NOT_OWNERSHIP_APPROVAL',
                  model_sha256=evidence['model_sha256'],
                  source_crop=crop, native_render_crop=evidence['source_crop'],
                  source_pixel_packet=packet, source_pixel_packet_sha256=sha(packet_path),
                  actual_render_sha256=sha(actual_path),
                  evidence_sha256=sha(evidence_path),
                  limitation='Rendered alpha includes antialiasing and establishes no material ownership. Review the forty native pixel coordinates against unmarked art and the actual saved silhouette.')
    (args.output / 'evidence.json').write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
