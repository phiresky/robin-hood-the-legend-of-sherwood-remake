"""Trace the decoded cathedral reveal onto the three currently published assemblies."""
from pathlib import Path
import hashlib
import json
import math
import struct
import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/active/york'
OUT = BASE / 'restart2/restart84-cathedral-source-rays-v1'

def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()

def triangles(asset):
    folder = ROOT / 'level-editor/library/3d-assets/york' / asset
    descriptor = json.loads((folder / 'asset.json').read_text())
    raw = (folder / 'model.glb').read_bytes()
    length = struct.unpack_from('<I', raw, 12)[0]
    gltf = json.loads(raw[20:20 + length])
    data = raw[28 + length:]
    def accessor(index):
        a = gltf['accessors'][index]
        view = gltf['bufferViews'][a['bufferView']]
        dtype = np.dtype({5126: '<f4', 5125: '<u4', 5123: '<u2', 5121: 'u1'}[a['componentType']])
        width = {'VEC3': 3, 'SCALAR': 1}[a['type']]
        return np.ndarray((a['count'], width), dtype=dtype, buffer=data, offset=view.get('byteOffset', 0) + a.get('byteOffset', 0), strides=(view.get('byteStride', dtype.itemsize * width), dtype.itemsize)).copy()
    result = []
    for node in gltf['nodes']:
        if 'mesh' not in node:
            continue
        assert not any(k in node for k in ['matrix', 'translation', 'rotation', 'scale']), 'Unexpected mesh transform'
        owner = node['extras'].get('editor_part_node', node['extras']['source_node'])
        for primitive in gltf['meshes'][node['mesh']]['primitives']:
            vertices = accessor(primitive['attributes']['POSITION']).astype(float) + np.array(descriptor['source_origin_scene'])
            result.extend((owner, vertices[index]) for index in accessor(primitive['indices']).reshape(-1, 3))
    return result, {str(folder / name): sha(folder / name) for name in ['asset.json', 'model.glb']}

def main():
    OUT.mkdir(exist_ok=False)
    source = BASE / 'geometry-pass-01/native-state-source-v1/patch-006/row-1/frame-000.png'
    assert sha(source) == 'f6b1b0f37b214a8c10ec917ae9d8fbab1d59e80d5dcd8bc2c3fbfd420c4ef317'
    image = np.array(Image.open(source).convert('RGBA'))
    height, width = image.shape[:2]
    domain = image[:, :, 3] > 0
    depth = np.full((height, width), -np.inf)
    owner_ids = np.full((height, width), -1, np.int32)
    names = []
    pins = {str(source): sha(source)}
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
    camera = np.array([0, -cosine, sine])
    assets = ['york-cathedral-nave-and-aisles', 'york-cathedral-northwest-tower', 'york-cathedral-southwest-tower']
    for asset in assets:
        rows, model_pins = triangles(asset)
        pins.update(model_pins)
        for owner, tri in rows:
            name = asset + '/' + owner
            if name not in names:
                names.append(name)
            oid = names.index(name)
            xy = np.column_stack((tri[:, 0] - 1062, -tri[:, 1] * sine - tri[:, 2] * cosine - 4))
            x0, x1 = max(0, int(np.floor(xy[:, 0].min()))), min(width, int(np.ceil(xy[:, 0].max())))
            y0, y1 = max(0, int(np.floor(xy[:, 1].min()))), min(height, int(np.ceil(xy[:, 1].max())))
            if x1 <= x0 or y1 <= y0:
                continue
            a, b, c = xy
            den = (b[1]-c[1])*(a[0]-c[0])+(c[0]-b[0])*(a[1]-c[1])
            if abs(den) < 1e-10:
                continue
            yy, xx = np.mgrid[y0:y1, x0:x1]
            xx, yy = xx + .5, yy + .5
            u = ((b[1]-c[1])*(xx-c[0])+(c[0]-b[0])*(yy-c[1])) / den
            v = ((c[1]-a[1])*(xx-c[0])+(a[0]-c[0])*(yy-c[1])) / den
            z = u*(tri[0]@camera)+v*(tri[1]@camera)+(1-u-v)*(tri[2]@camera)
            visible = (u >= 0) & (v >= 0) & (u+v <= 1) & (z > depth[y0:y1, x0:x1])
            depth[y0:y1, x0:x1][visible] = z[visible]
            owner_ids[y0:y1, x0:x1][visible] = oid
    counts = {name: int((domain & (owner_ids == i)).sum()) for i, name in enumerate(names)}
    np.savez_compressed(OUT / 'ray-domains.npz', owner_ids=owner_ids, depth=depth.astype(np.float32), source_domain=domain)
    report = dict(status='CURRENT_STATIC_RECEIVER_DIAGNOSTIC_NOT_APPLIED_GEOMETRY', origin=[1062, 4], size=[width, height], source_pixels=int(domain.sum()), no_hit_pixels=int((domain & (owner_ids < 0)).sum()), receivers=counts, owner_names=names, source_pins=pins, next='Split source-supported cover domains from retained three-asset exterior, then reconstruct exposed floor/columns/trusses/altar; this ray map must not be used to project interior artwork onto current roofs.')
    (OUT / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: v for k, v in report.items() if k not in ['source_pins', 'owner_names', 'next']}, indent=2))

if __name__ == '__main__':
    main()
