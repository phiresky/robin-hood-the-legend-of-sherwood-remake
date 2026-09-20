"""Stage an approved texture fill with complete geometry and outside-asset guards.

Run in an isolated loaded worker blend. Nothing is published or copied into the
main scene; the resulting worker is suitable for geometry-checked texture import.
"""
from array import array
import hashlib
import json
from pathlib import Path

import bpy

from project_reviewed_texture import apply
from refinement_workspace import _geometry
from refinement_review import _tile
from render_multiview_asset import render


def _materials(obj):
    records = []
    for material in obj.data.materials:
        if material is None:
            records.append(None)
            continue
        records.append([material.name, repr(dict(material)), [
            [node.name, node.type, getattr(node,'uv_map',None),
             node.image.name if node.type=='TEX_IMAGE' and node.image else None]
            for node in material.node_tree.nodes] if material.use_nodes else None])
    record = {'slots':records, 'faces':[face.material_index for face in obj.data.polygons],
              'uv':{layer.name:[list(entry.uv) for entry in layer.data] for layer in obj.data.uv_layers}}
    return hashlib.sha256(json.dumps(record,sort_keys=True).encode()).hexdigest()


def stage(manifest_path, image_path, output_dir, *, texels_per_unit=2):
    manifest_path, image_path, output = Path(manifest_path), Path(image_path), Path(output_dir)
    manifest = json.loads(manifest_path.read_text())
    scene = bpy.data.scenes[manifest['scene_name']]
    bpy.context.window.scene = scene
    bpy.context.view_layer.update()
    asset_id = manifest['asset_id']
    geometry = {obj.name:_geometry(obj) for obj in scene.objects}
    outside = {obj.name:_materials(obj) for obj in scene.objects
               if obj.type=='MESH' and obj.get('asset_group') != asset_id}
    evidence = [manifest_path, image_path, manifest_path.parent/'input.png',manifest_path.parent/'mask.png']
    hashes = {str(path):hashlib.sha256(path.read_bytes()).hexdigest() for path in evidence}
    report = apply(manifest_path,image_path,output,texels_per_unit=texels_per_unit)
    if geometry != {obj.name:_geometry(obj) for obj in scene.objects}:
        raise RuntimeError('Texture stage changed geometry or scene membership')
    if outside != {obj.name:_materials(obj) for obj in scene.objects if obj.name in outside}:
        raise RuntimeError('Texture stage changed another asset material or UV')
    if hashes != {str(path):hashlib.sha256(path.read_bytes()).hexdigest() for path in evidence}:
        raise RuntimeError('Approved evidence changed during texture stage')
    report.update(geometry_verified=True, outside_objects_unchanged=len(outside), evidence_sha256=hashes)
    (output/'validation.json').write_text(json.dumps(report,indent=2)+'\n')
    bpy.ops.wm.save_as_mainfile(filepath=str(output/'worker.blend'))
    width,height = manifest['tile_size']
    render(manifest_path,output/'actual',width=width)
    buffers=[]
    for index in range(8):
        image=bpy.data.images.load(str(output/'actual'/f'view-{index}-textured.png'),check_existing=False)
        try:
            buffer=array('f',[0])*len(image.pixels)
            image.pixels.foreach_get(buffer)
            buffers.append(buffer)
        finally:
            bpy.data.images.remove(image)
    _tile(buffers,width,height,output/'actual'/'textured.png')
    return report


if __name__=='__main__':
    import sys
    args=sys.argv[sys.argv.index('--')+1:]
    if len(args) not in (3,4):
        raise ValueError('Expected -- manifest.json generated.png new_output_dir [texels_per_unit]')
    result=stage(*args[:3],texels_per_unit=float(args[3]) if len(args)==4 else 2)
    print(json.dumps({'asset':result['asset_id'],'counts':result['counts'],
                      'geometry_verified':result['geometry_verified'],
                      'outside_objects_unchanged':result['outside_objects_unchanged']}),flush=True)
