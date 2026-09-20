"""Blender regression: overlapping foreground artwork must never become evidence."""
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import hashlib
import json
import math
import tempfile
import bpy
import numpy as np
from mathutils import Vector
from occlusion_constraints import SourceMaskConstraints
from source_projection_bake import bake
from refinement_review import render_review

out = Path(tempfile.mkdtemp(prefix='review-mask-'))
def png(name, values):
    image = bpy.data.images.new(name, width=16, height=16, alpha=True)
    image.pixels.foreach_set(values.ravel())
    image.filepath_raw = str(out/name)
    image.file_format = 'PNG'
    image.save()
    bpy.data.images.remove(image)

pixels = np.ones((16,16,4), dtype=np.float32)
pixels[:,:,:3] = [1,0,0]
pixels[:,8:,:3] = [0,1,0]  # Foreground artwork where geometry is under-modelled.
png('source.png', pixels)
pixels[:,:,:3] = 1
png('building.png', pixels)
pixels[:,:8,:3] = 0
png('foreground.png', pixels)
inventory = {'masks': [{'index':i, 'box_top_left':[0,0], 'box_size':[16,16], 'png':name}
                       for i,name in [(0,'building.png'),(63,'foreground.png')]]}
(out/'inventory.json').write_text(json.dumps(inventory))
sha = hashlib.sha256((out/'source.png').read_bytes()).hexdigest()
assignment = {'reviewed':True, 'asset_group':'fixture', 'mask_indices':[0],
              'exclude_mask_indices':[63], 'exclusions_reviewed':True,
              'exclusion_reason':'Foreground occupies the right half in this source state.'}
manifest = {'version':1, 'mask_inventory':'inventory.json', 'projections':{
    'exterior': {'source_sha256':sha, 'state':'covered', 'assignments':[assignment]}}}
path = out/'constraints.json'
path.write_text(json.dumps(manifest))
collection = bpy.data.collections.new('MaskFixture Working')
bpy.context.scene.collection.children.link(collection)
up = Vector((0,math.sin(math.radians(35)),math.cos(math.radians(35))))
mesh = bpy.data.meshes.new('Receiver')
mesh.from_pydata([Vector((x,0,0))+up*(y-16) for x,y in [(0,0),(16,0),(16,16),(0,16)]], [], [(0,1,2,3)])
mesh.update()
obj = bpy.data.objects.new('Receiver',mesh)
obj['source_node']='receiver'
obj['asset_group']='fixture'
collection.objects.link(obj)
bpy.context.view_layer.update()
baseline = bake('MaskFixture',out/'source.png',out/'baseline.json',projection_label='exterior')
masked = bake('MaskFixture',out/'source.png',out/'masked.json',projection_label='exterior',source_mask_manifest=path)
assert masked['mask_rejected_texels'] > 100
assert 0 < masked['known_texels'] < baseline['known_texels']
constraints = SourceMaskConstraints(path,'exterior',sha,(16,16))
sx, sy = np.meshgrid(np.arange(16),np.arange(16))
vector = constraints.allowed(constraints.for_object(obj),sx.ravel(),sy.ravel())
scalar = [constraints.allowed_pixel(obj,int(x),15-int(y)) for x,y in zip(sx.ravel(),sy.ravel())]
assert np.array_equal(vector,scalar) and vector.sum()==128
review = render_review(out/'review', scene_name=bpy.context.scene.name,
    collection_name=collection.name, asset_id='fixture', source_path=out/'source.png',
    source_mask_manifest=path, width=32, height=32)
assert review['views'][0]['counts']['mask_rejected'] > 100
assert review['views'][0]['counts']['source'] > 100
image = bpy.data.images.load(str(out/'review/views/view-0-textured.png'))
rgb = np.asarray(image.pixels[:]).reshape(-1,4)[:,:3]
assert not ((rgb[:,1]>.9)&(rgb[:,0]<.01)).any(), 'Foreground green leaked into review'
bpy.data.images.remove(image)
for key,value in [('exclusions_reviewed',False),('exclusion_reason','')]:
    old=assignment[key]; assignment[key]=value; path.write_text(json.dumps(manifest))
    try:
        SourceMaskConstraints(path,'exterior',sha,(16,16))
    except ValueError:
        pass
    else:
        raise AssertionError('Unreviewed foreground exclusion accepted')
    assignment[key]=old
path.write_text(json.dumps(manifest))
try:
    SourceMaskConstraints(path,'exterior','stale',(16,16))
except ValueError:
    pass
else:
    raise AssertionError('Wrong source state accepted')
print(json.dumps({'status':'PASS','output':str(out),'review':review['views'][0]['counts'],
                  'bake_rejected':masked['mask_rejected_texels']}))

# Exercise the same immutable review -> generated fill contract without an API.
import shutil
from project_reviewed_texture import apply, _read
from refinement_review import _save, _tile
packet = out/'generated'
packet.mkdir()
shutil.copy2(out/'review/textured.png', packet/'input.png')
generated = np.ones((64,128,4),dtype=np.float32)
generated[:,:,:3] = [0,0,1]
_save(packet/'generated.png',128,64,generated.ravel())
tile_masks=[]
for view in review['views']:
    i=view['index']
    known=_read(out/f'review/views/view-{i}-known.png')[:,:,0]>.5
    solid=_read(out/f'review/views/view-{i}-solid.png')[:,:,3]>0
    tile=np.ones((32,32,4),dtype=np.float32)
    tile[:,:,3][solid & ~known]=0
    tile_masks.append(tile.ravel())
    view['crop']={'left':i%4*32,'top':i//4*32,'width':32,'height':32}
_tile(tile_masks,32,32,packet/'mask.png')
input_hash=hashlib.sha256((packet/'input.png').read_bytes()).hexdigest()
review.update(reviewed_packet=str(out/'review'),
              reviewed_manifest_sha256=hashlib.sha256((out/'review/views.json').read_bytes()).hexdigest(),
              layout={'width':128,'height':64})
(packet/'views.json').write_text(json.dumps(review))
(packet/'approval.json').write_text(json.dumps({'status':'approved','asset_id':'fixture','input_sha256':input_hash}))
report=apply(packet/'views.json',packet/'generated.png',out/'generated-bake',map_name='MaskFixture')
assert report['counts']['generated_texels_including_padding']>100
assert report['layers'][0]['mask_rejected_texels']>100
mat=mesh.materials[mesh.polygons[0].material_index]
atlas=next(n.image for n in mat.node_tree.nodes if n.type=='TEX_IMAGE')
rgb=np.asarray(atlas.pixels[:]).reshape(-1,4)[:,:3]
assert not ((rgb[:,1]>.9)&(rgb[:,0]<.01)).any(), 'Foreground RGB retained by generated bake'
assert ((rgb[:,0]>.9)&(rgb[:,1]<.01)).any(), 'Protected source missing'
assert ((rgb[:,2]>.1)&(rgb[:,0]<.01)).any(), 'Generated unknown fill missing'
assert mat.get('generated_source_mask_evidence_sha256')
review.pop('source_mask_manifest')
(packet/'views.json').write_text(json.dumps(review))
try:
    apply(packet/'views.json',packet/'generated.png',out/'must-not-exist',map_name='MaskFixture')
except ValueError:
    assert not (out/'must-not-exist').exists()
else:
    raise AssertionError('Dropped ownership contract accepted')
print('PASS: generated bake rejects foreground, protects source and refuses dropped mask contract')
