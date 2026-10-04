"""Read-only comparison of occupancy, animated alpha and selected crown support."""
import argparse
import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'level-editor/refinement/blender'))
from catalog import OUT, TREES, tree_workspace
from evidence_io import sha, write_json


def canvas(path, box, alpha_channel=False):
    image = Image.open(path).convert('RGBA' if alpha_channel else 'L')
    pixels = np.asarray(image)
    alpha = pixels[:, :, 3] > 127 if alpha_channel else pixels > 0
    x, y = map(int, box[:2])
    h, w = alpha.shape
    result = np.zeros((1152, 1792), dtype=bool)
    left, top, right, bottom = max(0, x), max(0, y), min(1792, x+w), min(1152, y+h)
    if right > left and bottom > top:
        result[top:bottom, left:right] = alpha[top-y:bottom-y, left-x:right-x]
    return result


def counts(a, b):
    return dict(first_pixels=int(a.sum()), second_pixels=int(b.sum()),
                shared=int((a & b).sum()), first_only=int((a & ~b).sum()),
                second_only=int((b & ~a).sum()))


def main(destination):
    if destination.exists():
        raise ValueError('Use a fresh audit file')
    animation_manifest = OUT / 'animation-references/manifest.json'
    mask_manifest = OUT / 'baseline/masks/manifest.json'
    animations = json.loads(animation_manifest.read_text())['animations']
    masks = json.loads(mask_manifest.read_text())['masks']
    records, animated = [], {}
    for animation in animations:
        index = animation['index']
        if index >= 8:
            continue
        native = next(row for row in masks if row['index'] == 128+index)
        mask_path = mask_manifest.parent / native['png']
        frame = animation['frames'][0]
        frame_path = Path(frame['image'])
        a = canvas(mask_path, native['box_top_left'])
        b = canvas(frame_path, frame['bbox'], True)
        frame_alpha = np.asarray(Image.open(frame_path).convert('RGBA'))[:, :, 3] > 127
        parity = {f'{y},{x}':int(frame_alpha[y::2, x::2].sum()) for y in (0, 1) for x in (0, 1)}
        animated[index] = b
        records.append(dict(animation=index, profile=animation['profile'], native_mask=128+index,
                            occupancy_png=str(mask_path), occupancy_sha256=sha(mask_path),
                            frame_png=str(frame_path), frame_sha256=sha(frame_path),
                            frame_local_opaque_parity=parity,
                            checkerboard_alpha=(parity['0,0'] == parity['1,1'] == 0 or
                                                parity['0,1'] == parity['1,0'] == 0),
                            comparison=counts(a, b)))
    source_rows = {row['mask']:row for row in json.loads((OUT/'forest-v4-sources/manifest.json').read_text())}
    workers = []
    for mask in TREES:
        worker = tree_workspace(mask)
        report = json.loads((worker/'inspection/refinement.json').read_text())
        packet_path = Path(report.get('source_packet', source_rows[mask]['packet']))
        packet = json.loads(packet_path.read_text())
        index = packet.get('animation')
        source = packet_path.parent/'complete-source.png'
        row = dict(mask=mask, worker=str(worker), model_sha256=sha(worker/'model.blend'),
                   packet=str(packet_path), packet_sha256=sha(packet_path),
                   source_sha256=sha(source), animation=index,
                   coverage_provenance=packet.get('coverage_provenance'),
                   physical_silhouette_authority=packet.get('physical_silhouette_authority'),
                   geometry_version=report.get('crown',{}).get('geometry_version'))
        if index in animated:
            a = canvas(source, packet['native_bbox'], True)
            row['packet_vs_animated_alpha'] = counts(a, animated[index])
        workers.append(row)
    write_json(destination, dict(
        status='Read-only authority comparison; not a visual-correctness or approval decision',
        scope='Visible map pixels only, synchronized phase0, physical RGBA threshold127; per-tree packets can intentionally partition a shared canopy',
        interpretation=['Occupancy-only pixels may contain legitimate static base leaves; they are not automatically fabricated foliage.',
                        'Animated frames use checkerboard alpha; replacing physical support with raw animated alpha can incorrectly discard static leaf pixels beneath the overlay.',
                        'Animated-only pixels may be hidden by native display order or belong to another tree partition; counts alone do not authorize support changes.',
                        'Classify static artwork, animated overlays, ownership and native draw order before changing any geometry/source authority.',
                        'No model, source packet, selector, approval or canonical asset was changed.'],
        animation_manifest_sha256=sha(animation_manifest), mask_manifest_sha256=sha(mask_manifest),
        static_source_sha256=sha(OUT/'baseline/covered.png'),
        composite_sha256=sha(OUT/'animation-references/composite-frame-0.png'),
        construction_recipe_sha256=sha(Path(__file__).with_name('tree_geometry.py')),
        animations=records, workers=workers))
    print(destination)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('destination', type=Path)
    main(parser.parse_args().destination.resolve())
