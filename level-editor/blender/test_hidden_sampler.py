"""Blender fixture: partial occlusion protects source pixels from hidden fills."""
import sys
import tempfile
from pathlib import Path

sys.path[:0] = ['/usr/lib/python3.14', '/usr/lib/python3.14/lib-dynload',
                '/usr/lib/python3.14/site-packages', str(Path(__file__).parent)]
import bpy
import numpy as np
from source_projection_bake import bake

collection = bpy.data.collections.new('Fixture Working')
bpy.context.scene.collection.children.link(collection)
for name, x0, z in [('receiver', 1, 0), ('occluder', 4, 1)]:
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata([(x0,-10,z),(7,-10,z),(7,-2,z),(x0,-2,z)], [], [(0,1,2,3)])
    obj = bpy.data.objects.new(name, mesh)
    obj['source_node'] = name
    collection.objects.link(obj)
bpy.context.view_layer.update()
counts = {'known':0, 'unknown':0}

def safe(obj, normal, positions, accepted, colors):
    assert positions[:,0].min() >= 1-1e-6 and positions[:,0].max() <= 7+1e-6
    assert positions[:,1].min() >= -10-1e-6 and positions[:,1].max() <= -2+1e-6
    counts['known'] += int(accepted.sum())
    counts['unknown'] += int((~accepted).sum())
    colors[~accepted,:3] = (1,0,1)

with tempfile.TemporaryDirectory(dir=Path(__file__).parent) as directory:
    directory = Path(directory)
    image = bpy.data.images.new('Fixture original', width=16, height=16, alpha=True)
    image.pixels.foreach_set(np.tile(np.array((.2,.4,.6,1), dtype=np.float32), 256))
    image.filepath_raw = str(directory/'source.png')
    image.file_format = 'PNG'
    image.save()
    bake('Fixture', image.filepath_raw, directory/'report.json', receiver_nodes=['receiver'],
         preserve_authored=False, hidden_sampler=safe, texels_per_unit=2)
    assert counts['known'] > 0 and counts['unknown'] > 0, counts

    def corrupt(obj, normal, positions, accepted, colors):
        colors[accepted,:3] = (1,0,0)
    receiver = bpy.data.objects['receiver']
    material = receiver.data.materials[receiver.data.polygons[0].material_index]
    material['generated_source_sha256'] = 'fixture-approved-image'
    retained = bake('Fixture', image.filepath_raw, directory/'retained.json', receiver_nodes=['receiver'],
                    preserve_authored=True, hidden_sampler=corrupt, texels_per_unit=2)
    assert retained['objects'][0]['authored_faces_preserved'] == 1
    try:
        bake('Fixture', image.filepath_raw, directory/'bad.json', receiver_nodes=['receiver'],
             preserve_authored=True, reproject_authored_nodes=['receiver'],
             hidden_sampler=corrupt, texels_per_unit=2)
    except ValueError as error:
        assert 'modified protected source pixels' in str(error)
    else:
        raise AssertionError('Corrupting hidden sampler was accepted')
print('PASS: partial source ownership, approved fill preservation, explicit reset and corrupting sampler rejection',counts,flush=True)
