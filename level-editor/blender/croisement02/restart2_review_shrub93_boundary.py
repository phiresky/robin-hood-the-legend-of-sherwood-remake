"""Inspect exact additive shrub93 and measure its separate inferred boundary."""
import json
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT/'level-editor/refinement'),
               str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT, scenery_workspace
from evidence_io import sha, write_json
from tree_geometry import SIN, RAY
from render_slots import acquire, release
from render_multiview_asset import render


def sheet(folder, output):
    images = [Image.open(folder/f'view-{i}-textured.png').convert('RGBA') for i in range(8)]
    w, h = images[0].size; result = Image.new('RGB', (w*4, h*2), '#444444')
    for i, image in enumerate(images): result.paste(image, ((i%4)*w, (i//4)*h), image)
    result.save(output)


def main():
    worker = OUT/'restart2-vegetation/shrub93-boundary-v2'
    proof = json.loads((worker/'preservation.json').read_text())
    digest = sha(worker/'model.blend'); assert digest == proof['model_sha256']
    target = worker/'review-v3'; target.mkdir(exist_ok=False)
    bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
    scene = bpy.data.scenes['Croisement02 Refinement']
    bpy.context.window.scene = scene
    scene.render.engine = 'CYCLES'; scene.cycles.samples = 8
    scene.cycles.transparent_max_bounces = 256
    packet = json.loads((worker/'cameras.json').read_text())
    for field in ('object_names', 'render_object_names'):
        if packet.get(field) is not None and proof['added_object'] not in packet[field]:
            packet[field].append(proof['added_object'])
    write_json(target/'cameras.json', packet)
    render(target/'cameras.json', target/'actual', width=384)
    sheet(target/'actual', target/'actual/sheet.png')
    selected = [o for o in scene.objects if o.type == 'MESH' and o.get('asset_group') == 'croisement02-shrub-93']
    assert len(selected) == 4
    for obj in scene.objects:
        if obj.type == 'MESH': obj.hide_render = obj not in selected
    crop = [1320, 707, 1540, 879]; left, top, right, bottom = crop
    data = bpy.data.cameras.new('Exact native boundary camera'); data.type = 'ORTHO'; data.ortho_scale = right-left; data.clip_end=10000
    camera = bpy.data.objects.new(data.name, data); scene.collection.objects.link(camera); scene.camera = camera
    center = Vector(((left+right)/2, -(top+bottom)/2/SIN, 0))
    camera.location = center+RAY*5000; camera.rotation_euler = (center-camera.location).to_track_quat('-Z', 'Y').to_euler()
    scene.render.resolution_x = (right-left)*3; scene.render.resolution_y = (bottom-top)*3; scene.render.resolution_percentage=100
    scene.render.film_transparent=True; scene.render.image_settings.color_mode='RGBA'
    scene.render.filepath=str(target/'source.png'); bpy.ops.render.render(write_still=True)
    alpha=np.asarray(Image.open(target/'source.png').convert('RGBA'))[1::3,1::3,3]>127
    boundary_path=OUT/'mixed-wood-audit/boundary-roles76-93-v1/93-foliage93.png'
    boundary=np.asarray(Image.open(boundary_path).convert('L'))[top:bottom,left:right]>0
    known=np.asarray(Image.open(OUT/'mixed-wood-audit/domain-503.png').convert('L'))[top:bottom,left:right]>0
    assert boundary.sum()==25
    original=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA').crop(crop)
    original.resize(((right-left)*3,(bottom-top)*3),Image.Resampling.NEAREST).save(target/'native-source.png')
    rendered=Image.open(target/'source.png').convert('RGBA')
    backdrop=original.resize(rendered.size,Image.Resampling.NEAREST)
    Image.alpha_composite(backdrop,rendered).save(target/'source-overlay.png')
    write_json(target/'source-coverage.json',dict(model_sha256=digest,crop=crop,
        known503_pixels=int(known.sum()),known503_covered=int((known&alpha).sum()),
        separately_inferred_boundary_pixels=int(boundary.sum()),boundary_covered=int((boundary&alpha).sum()),
        limitation='Isolated physical alpha coverage; actual neighbor first hit and appearance remain separate.'))
    assert int((boundary&alpha).sum())==25
    assert sha(worker/'model.blend')==digest
    write_json(target/'evidence.json',dict(model_sha256=digest,source_crop=crop,
        cameras_sha256=sha(target/'cameras.json'),actual_sheet_sha256=sha(target/'actual/sheet.png'),
        source_coverage_sha256=sha(target/'source-coverage.json'),ready_for_geometry_review=False,
        status='Private actual saved model; independent visual and joint review pending'))
    print(target)


if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
