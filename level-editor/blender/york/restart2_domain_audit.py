"""Compare paired architecture against independently inspected source domains."""
import argparse
from collections import Counter
import hashlib
import json
import math
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/york-refinement'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('model', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--furniture-only', action='store_true',
                        help='Independent native furniture-mask fit; omits room context explicitly')
    args = parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    if args.output.exists():
        raise FileExistsError(args.output)
    sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
    from render_slots import acquire
    acquire()
    import bpy
    import numpy as np
    from PIL import Image
    from mathutils import Vector
    from mathutils.bvhtree import BVHTree
    bpy.ops.wm.open_mainfile(filepath=str(args.model.resolve()))
    scene = bpy.data.scenes['york Refinement']
    vertices, triangles, owners = [], [], []
    furniture={824:631,825:634,826:633,827:640,828:632,829:636}
    nodes={f'building-{n}' for n in furniture}
    for obj in scene.objects:
        if obj.type != 'MESH' or obj.hide_render:
            continue
        if args.furniture_only and obj.get('source_node') not in nodes:
            continue
        start = len(vertices)
        vertices += [obj.matrix_world @ v.co for v in obj.data.vertices]
        obj.data.calc_loop_triangles()
        triangles += [tuple(start+i for i in triangle.vertices) for triangle in obj.data.loop_triangles]
        owners += [(obj.get('source_node') if args.furniture_only else obj.get('asset_group'),obj.get('source_node',obj.name))] * len(obj.data.loop_triangles)
    tree = BVHTree.FromPolygons(vertices, triangles, all_triangles=True)
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
    ray = Vector((0,-cosine,sine))
    source = Image.open(OUT/'baseline'/('revealed.png' if args.furniture_only else 'covered.png')).convert('RGB')
    args.output.mkdir(parents=True)
    results = []
    domains=[
            ('york-market-southeast-tall-narrow-house', OUT/'geometry-pass-01/narrow-house-domain-mask.png', (550,1205)),
            ('york-southwest-square-west-house', OUT/'restart2/main-house-domain.png', (550,1100))]
    if args.furniture_only:
        masks={m['index']:m for m in json.loads((OUT/'baseline/masks/manifest.json').read_text())['masks']}
        domains=[(f'building-{n}',OUT/'baseline/masks'/masks[index]['png'],masks[index]['box_top_left']) for n,index in furniture.items()]
    for asset, image, origin in domains:
        domain = np.array(Image.open(image).convert('L')) > 0
        size = domain.shape[::-1]
        crop = (*origin,origin[0]+size[0],origin[1]+size[1])
        picture = np.array(source.crop(crop)).astype(float)
        counts = Counter()
        bad = np.zeros_like(domain)
        for y,x in zip(*np.nonzero(domain)):
            start = Vector((origin[0]+float(x)+.5,-(origin[1]+float(y)+.5)/sine,0))+ray*10000
            hit,normal,index,distance = tree.ray_cast(start,-ray)
            if index is None:
                counts['NO_GEOMETRY'] += 1
                bad[y,x] = True
                color = np.array([255,0,0])
            elif owners[index][0] == asset:
                counts['OWNED_FIRST_HIT'] += 1
                color = np.array([0,255,0])
            else:
                counts[owners[index][1]] += 1
                bad[y,x] = True
                color = np.array([255,0,128])
            picture[y,x] = picture[y,x]*.5+color*.5
        Image.fromarray(picture.astype('uint8')).resize((size[0]*3,size[1]*3),Image.Resampling.NEAREST).save(args.output/f'{asset}.png')
        Image.fromarray(bad.astype('uint8')*255).save(args.output/f'{asset}-rejected.png')
        results.append(dict(asset=asset,domain_pixels=int(domain.sum()),domain_sha256=hashlib.sha256(image.read_bytes()).hexdigest(),
                            counts=dict(counts),owned_fraction=counts['OWNED_FIRST_HIT']/int(domain.sum())))
    (args.output/'report.json').write_text(json.dumps(dict(model_sha256=hashlib.sha256(args.model.read_bytes()).hexdigest(),
        scope=('Opaque saved geometry first-hit preflight; native domains reviewed independently. Missing pixels are not waived or subtracted.'+
               (' Furniture-only diagnostic excludes all room context; not a complete revealed state.' if args.furniture_only else '')),assets=results),indent=2)+'\n')
    print(json.dumps(results))


if __name__ == '__main__':
    main()
