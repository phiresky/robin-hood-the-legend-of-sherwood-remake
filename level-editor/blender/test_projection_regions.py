"""Mixed receiver regression: revealed pixels, covered pixels, and hidden pixels."""
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import hashlib
import json
import math
import tempfile
import bpy
import numpy as np
from mathutils import Vector
from refinement_review import render_review, _save
from source_projection_bake import bake
from projection_regions import ProjectionRegion

out=Path(tempfile.mkdtemp(prefix='projection-regions-'))
def image(name,width,height,rgb):
    pixels=np.ones((height,width,4),dtype=np.float32)
    pixels[:,:,:3]=rgb
    _save(out/name,width,height,pixels.ravel())
    return str(out/name)
covered=image('covered.png',16,16,[1,0,0])
revealed=image('revealed.png',16,16,[0,1,0])
alpha=image('alpha.png',8,16,[.2,.2,.2])
def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()
collection=bpy.data.collections.new('RegionFixture Working')
bpy.context.scene.collection.children.link(collection)
sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
up=Vector((0,sine,cosine)); toward=Vector((0,-cosine,sine))
def plane(name,left,right,depth,asset):
    mesh=bpy.data.meshes.new(name)
    mesh.from_pydata([Vector((x,0,0))+up*(y-16)+toward*depth
                     for x,y in [(left,0),(right,0),(right,16),(left,16)]],[],[(0,1,2,3)])
    mesh.update()
    obj=bpy.data.objects.new(name,mesh)
    obj['source_node']=name;obj['asset_group']=asset
    collection.objects.link(obj)
    return obj
receiver=plane('receiver',0,16,0,'fixture')
blocker=plane('covered-shell',12,16,2,'context')
bpy.context.view_layer.update()
region={'alpha_path':alpha,'alpha_sha256':sha(alpha),'bbox':[0,0,8,16],
        'source_sha256':sha(revealed),'state':'revealed/fixture',
        'fallback':{'source_path':covered,'source_sha256':sha(covered),
                    'state':'covered/fixture','projection_label':'exterior',
                    'occluder_nodes':['receiver','covered-shell']}}
report=bake('RegionFixture',revealed,out/'bake.json',receiver_nodes=['receiver'],
            occluder_nodes=['receiver'],projection_label='interior-fixture',projection_region=region)
assert report['objects'][0]['exterior_fallback_known_texels']>20
assert report['known_texels']>100 and report['unknown_texels']>20
review=render_review(out/'review',scene_name=bpy.context.scene.name,collection_name=collection.name,
    asset_id='fixture',source_path=covered,width=32,height=32,projection_layers=[
        {'source_path':revealed,'receiver_nodes':['receiver'],'occluder_nodes':['receiver'],
         'projection_label':'interior-fixture','projection_region':region}])
counts=review['views'][0]['counts']
assert counts['source']>500 and counts['exterior_fallback_source']>100 and counts['unknown']>100
img=bpy.data.images.load(str(out/'review/views/view-0-textured.png'))
rgb=np.asarray(img.pixels[:]).reshape(-1,4)[:,:3]
bpy.data.images.remove(img)
assert ((rgb[:,0]>.9)&(rgb[:,1]<.01)).sum()>100
assert ((rgb[:,1]>.9)&(rgb[:,0]<.01)).sum()>100
for bad in ({**region,'source_sha256':'stale'}, {**region,'state':''},
            {**region,'alpha_sha256':'stale'}):
    try:
        ProjectionRegion(bad,sha(revealed),(16,16),[receiver,blocker])
    except ValueError:
        pass
    else:
        raise AssertionError('Stale region evidence accepted')
print(json.dumps({'status':'PASS','output':str(out),'review_counts':counts,
                  'bake':report['objects'][0]}))
