"""Private source-ray crown articulation and inferred closure; no texture synthesis."""
import json
import math
from pathlib import Path
import shutil
import sys

import bpy
import numpy as np
from PIL import Image
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT/'level-editor/refinement'), str(ROOT/'level-editor/refinement/blender'), str(ROOT/'level-editor/blender/leicester')]
from render_slots import acquire
from evidence_io import sha
from foliage_trees import geometry_hash
from render_multiview_asset import render

BASE = ROOT/'level-editor/work/croisement01-refinement/restart2'
OLD = BASE/'tree14-whole-tree-v1'
OUT = BASE/'tree14-whole-tree-v2'
EXPECTED = '0903bce89abb49d6c1418c06ebefda21bfe488899b2047d7988441c9fc070765'


def main():
    acquire(slots=2)
    assert shutil.disk_usage(ROOT).free >= 10*1024**3
    assert int(next(s.split()[1] for s in Path('/proc/meminfo').read_text().splitlines() if s.startswith('MemAvailable:')))*1024 >= 6*1024**3
    assert sha(OLD/'model.blend') == EXPECTED
    OUT.mkdir(exist_ok=False)
    for name in ('source-packet.json','source-conservation.json','wood-role.png','crown-static-role.png','neighbor-wood-context.png','neighbor-foreground-context.png'):
        shutil.copy2(OLD/name, OUT/name)
    bpy.ops.wm.open_mainfile(filepath=str(OLD/'model.blend'))
    bpy.context.window.scene = bpy.data.scenes['Croisement01 Refinement']
    scene = bpy.context.scene
    bpy.context.preferences.filepaths.save_version = 0
    wood = next(o for o in scene.objects if o.get('source_node') == 'scenery-tree14-wood')
    protected = geometry_hash(wood)
    crowns = [o for o in scene.objects if o.get('source_node') in ('foliage-tree14-crown94','foliage-tree14-crown94-interior')]
    assert len(crowns) == 2
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
    ray = Vector((0,-cosine,sine))
    center = Vector((814.5,-412/sine,(412-193)/cosine))
    seeds = [(780,90,50),(825,130,43),(884,155,43),(725,165,47),(795,205,49),(915,215,44),(720,235,45),(850,285,57)]
    maximum_drift = 0.
    for obj in crowns:
        assert obj.matrix_world.is_identity
        for vertex in obj.data.vertices:
            p = vertex.co.copy()
            x, y = p.x, -p.y*sine-p.z*cosine
            plane = Vector((x,-412/sine,(412-y)/cosine))
            depth = (p-plane).dot(ray)
            delta = plane-center
            radii = (185.,195.,185.)
            aa = sum((ray[i]/radii[i])**2 for i in range(3))
            bb = 2*sum(delta[i]*ray[i]/radii[i]**2 for i in range(3))
            middle = -bb/(2*aa)
            peak = max(math.exp(-((x-sx)**2+(y-sy)**2)/(2*radius**2)) for sx,sy,radius in seeds)
            articulation = .62+.52*peak
            # Full volumetric lobes remain joined through the existing interior;
            # variation changes hidden depth without displacing observed texels.
            updated = middle+(depth-middle)*articulation
            # A measured source-leaf pixel intersects the wood by less than one
            # unit. This local forward offset gives the leaf its traced order.
            if depth > middle:
                updated += 8*math.exp(-((x-901)**2+(y-227)**2)/(2*24**2))
            vertex.co = plane+ray*updated
            maximum_drift = max(maximum_drift,abs(vertex.co.x-x),abs(-vertex.co.y*sine-vertex.co.z*cosine-y))
        obj.data.update()
    wood_domain = np.asarray(Image.open(OUT/'wood-role.png').convert('L')) > 0
    packet = json.loads((OLD/'source-packet.json').read_text())
    closure_images = []
    for obj in crowns:
        if obj.get('source_node') != 'foliage-tree14-crown94':
            continue
        for slot in obj.material_slots:
            original = slot.material
            if original.get('foliage_observed'):
                continue
            tex = next(n for n in original.node_tree.nodes if n.type == 'TEX_IMAGE')
            # The native lobe image identifies its source bbox; neither texture
            # color nor another tree's appearance is borrowed.
            image_name = Path(tex.image.filepath).name
            index = int(image_name.split('-')[1])
            lobe = packet['lobes'][index]
            x0,y0,x1,y1 = lobe['bbox_source']
            rgba = np.array(Image.open(OLD/f'lobe-{index:02}-unknown.png').convert('RGBA'))
            rgba[wood_domain[y0:y1,x0:x1],3] = 0
            path = OUT/f'lobe-{index:02}-closure.png'
            Image.fromarray(rgba).save(path)
            closure_images.append({'lobe':index,'path':str(path),'sha256':sha(path),'protected_wood_excluded':True})
            mat = original.copy()
            slot.material = mat
            texture = next(n for n in mat.node_tree.nodes if n.type == 'TEX_IMAGE')
            texture.image = bpy.data.images.load(str(path),check_existing=False)
            texture.image.pack()
            shader = next(n for n in mat.node_tree.nodes if n.type == 'BSDF_PRINCIPLED')
            output = next(n for n in mat.node_tree.nodes if n.type == 'OUTPUT_MATERIAL')
            for link in list(output.inputs['Surface'].links):
                mat.node_tree.links.remove(link)
            mat.node_tree.links.new(shader.outputs['BSDF'],output.inputs['Surface'])
            mat.use_backface_culling = False
            mat['foliage_card_sides'] = 'double-sided'
            mat['closure_scope'] = 'Inferred crown behind neighboring source occluders; protected own wood excluded.'
    assert geometry_hash(wood) == protected and maximum_drift < .0002
    model = OUT/'model.blend'
    bpy.ops.wm.save_as_mainfile(filepath=str(model))
    previous = json.loads((OLD/'construction.json').read_text())
    points = [obj.matrix_world@v.co for obj in crowns for v in obj.data.vertices]
    spans = [max(p[a] for p in points)-min(p[a] for p in points) for a in range(3)]
    previous.update(status='PRIVATE_CLOSURE_REQUIRES_JOINT_AUDIT',model_sha256=sha(model),crown_geometry_hashes={o.name:geometry_hash(o) for o in crowns},source_projection_max_drift=maximum_drift,previous_model_sha256=EXPECTED,inferred_closure=True,neighbor_geometry_not_proven=True,closure_images=closure_images,crown_world_spans_xyz=spans,crown_depth_at_least_width=spans[1]>=spans[0])
    (OUT/'construction.json').write_text(json.dumps(previous,indent=2)+'\n')
    shutil.copy2(OLD/'views.json',OUT/'views.json')
    render(OUT/'views.json',OUT/'views',modes=('textured',),width=384)
    images = [Image.open(OUT/f'views/view-{i}-textured.png').convert('RGB') for i in range(8)]
    w,h = images[0].size
    sheet = Image.new('RGB',(w*4,h*2))
    for i,im in enumerate(images):
        sheet.paste(im,((i%4)*w,(i//4)*h))
    sheet.save(OUT/'material-eight.png')
    print('Saved inferred closure candidate',sha(model),flush=True)


if __name__ == '__main__':
    main()
