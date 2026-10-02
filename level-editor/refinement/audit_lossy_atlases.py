"""Measure runtime atlas dimensions and approximate UV coverage for a saved map.

python3 refinement/audit_lossy_atlases.py library/scenes/Wychford.rhlos-map.json \
    --output work/atlas-audit.json

Coverage is a conservative low-resolution triangle raster, not a quality score.
The summed UV triangle area can exceed one when surfaces reuse texture pixels.
"""
import argparse
import io
import json
from pathlib import Path
import sys

import numpy as np
from PIL import Image, ImageDraw

sys.path.insert(0, str(Path(__file__).resolve().parent / 'blender'))
import lossy_assets as lossy


def image_payload(document, buffers, model, image):
    if 'uri' in image:
        return (model.parent / image['uri']).read_bytes()
    view = document['bufferViews'][image['bufferView']]
    start = view.get('byteOffset', 0)
    return buffers[view.get('buffer', 0)][start:start + view['byteLength']]


def coverage(document, buffers, image_index, edge):
    mask = Image.new('1', (edge, edge))
    draw = ImageDraw.Draw(mask)
    area, triangles, collapsed = 0., 0, 0
    coordinates = []
    for mesh in document.get('meshes', []):
        for primitive in mesh['primitives']:
            found = lossy.display_texture(document, primitive)
            if not found or found[1] != image_index:
                continue
            info = found[0]
            uv = lossy.accessor_array(document, buffers,
                primitive['attributes'][f'TEXCOORD_{info.get("texCoord", 0)}'], True)
            indices = (lossy.accessor_array(document, buffers, primitive['indices'])
                       if 'indices' in primitive else np.arange(len(uv)))
            faces = uv[indices].reshape(-1, 3, 2)
            if not np.isfinite(faces).all():
                raise ValueError('Non-finite texture coordinates')
            coordinates.append(faces.reshape(-1, 2))
            edges = faces[:, 1:] - faces[:, :1]
            double_area = np.abs(edges[:, 0, 0] * edges[:, 1, 1] - edges[:, 0, 1] * edges[:, 1, 0])
            area += float(double_area.sum() / 2)
            triangles += len(faces)
            collapsed += int((double_area < 1e-12).sum())
            for face in faces:
                draw.polygon([tuple(point) for point in face * (edge - 1)], fill=1)
    if not coordinates:
        return None
    uv = np.concatenate(coordinates)
    outside = bool((uv < 0).any() or (uv > 1).any())
    return {'triangles': triangles, 'collapsed_uv_triangles': collapsed,
            'uv_min': uv.min(axis=0).tolist(), 'uv_max': uv.max(axis=0).tolist(),
            'uv_area_sum': area, 'out_of_range': outside,
            # Clipping a repeated texture to one tile would give a misleading result.
            'raster_coverage': None if outside else float(np.asarray(mask).mean()),
            'raster_edge': edge}


def audit(manifest, library, edge):
    saved = json.loads(manifest.read_text())
    paths = dict.fromkeys(ref['model'] for ref in saved['sceneAssets'] + saved.get('assetSources', []))
    rows, missing = [], []
    for model in paths:
        source = library / model
        runtime = source.with_name(lossy.lossy_name(source))
        if not runtime.exists():
            missing.append(model)
            continue
        receipt = json.loads(Path(str(runtime) + '.receipt.json').read_text())
        row = {'model': model, 'algorithm_version': receipt['settings']['algorithm_version'],
               'source_sha256': lossy.sha(source), 'lossy_sha256': lossy.sha(runtime)}
        for kind, path in [('source', source), ('lossy', runtime)]:
            doc, buffers, _ = lossy.read_glb(path)
            images = []
            for index, image in enumerate(doc.get('images', [])):
                payload = image_payload(doc, buffers, path, image)
                with Image.open(io.BytesIO(payload)) as decoded:
                    size = list(decoded.size)
                item = {'index': index, 'size': size, 'bytes': len(payload)}
                if kind == 'lossy':
                    item['uv'] = coverage(doc, buffers, index, edge)
                images.append(item)
            row[kind] = images
        rows.append(row)
        print(f'{source.parent.name}: {sum(i["size"][0] * i["size"][1] for i in row["lossy"]):,} runtime pixels', flush=True)
    return {'manifest': str(manifest), 'rows': rows, 'missing_lossy': missing,
            'note': 'Image entries are counted individually, before browser texture/source deduplication; no mipmaps.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path)
    parser.add_argument('--library', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--raster-edge', type=int, default=512)
    args = parser.parse_args()
    if not 16 <= args.raster_edge <= 4096:
        parser.error('raster edge must be between 16 and 4096')
    result = audit(args.manifest.resolve(), (args.library or args.manifest.parent.parent).resolve(), args.raster_edge)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    main()
