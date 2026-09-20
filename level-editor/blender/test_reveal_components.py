"""A split facade excludes only its cover from the revealed source BVH."""
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import hashlib,json,math,tempfile
import bpy,numpy as np
from mathutils import Vector
from refinement_review import render_review,_save
from source_projection_bake import bake
from reveal_components import filter_occluders,filter_receivers
from interior_layers import projection_receivers,validate_projection_reviews,projection_occluder_additions,projection_occluders

out=Path(tempfile.mkdtemp(prefix='reveal-components-'))
def image(name,w,h,rgb):
    pixels=np.ones((h,w,4),dtype=np.float32);pixels[:,:,:3]=rgb
    _save(out/name,w,h,pixels.ravel());return str(out/name)
def sha(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()
covered=image('covered.png',16,16,[1,0,0]);revealed=image('revealed.png',16,16,[0,1,0])
alpha=image('alpha.png',8,16,[1,1,1])
ownership=image('ownership.png',16,16,[1,1,1])
(out/'inventory.json').write_text(json.dumps({'masks':[{'index':0,'png':ownership,'box_top_left':[0,0],'box_size':[16,16]}]}))
mask_manifest=out/'masks.json'
mask_manifest.write_text(json.dumps({'version':1,'mask_inventory':str(out/'inventory.json'),'projections':{
    label:{'source_sha256':sha(path),'state':label,'assignments':[{'reviewed':True,'asset_group':'fixture','mask_indices':[0]}]}
    for label,path in [('exterior',covered),('interior-patch-003',revealed)]}}))
canonical=json.loads(Path(__file__).with_name('derby_upper_gate_projection.json').read_text())
assert projection_receivers({'map':'Derby'})['patch-003']==['building-252','building-253']
manifest={'map':'Derby','sources':{'interior':revealed},'patches':[{'id':'patch-003','graphic':{'alpha':alpha}}],
    'projection_reviews':{'patch-003':{**canonical,'source_sha256':sha(revealed),'alpha_sha256':sha(alpha)}}}
assert projection_receivers(manifest)['patch-003']==canonical['receiver_nodes']
assert projection_occluder_additions({'map':'Derby'})=={}
assert 'building-252' in projection_occluder_additions(manifest)['exterior']
assert 'building-267' in projection_occluders(manifest,canonical['receiver_nodes']+['building-267'])['patch-003']
validate_projection_reviews(manifest,out)
stale={**manifest,'projection_reviews':{'patch-003':{**manifest['projection_reviews']['patch-003'],'alpha_sha256':'stale'}}}
try:validate_projection_reviews(stale,out)
except ValueError:pass
else:raise AssertionError('Changed cover alpha accepted')
collection=bpy.data.collections.new('CoverFixture Working');bpy.context.scene.collection.children.link(collection)
s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
up=Vector((0,s,c));toward=Vector((0,-c,s))
def plane(name,node,left,right,depth,asset):
    mesh=bpy.data.meshes.new(name)
    mesh.from_pydata([Vector((x,0,0))+up*(y-16)+toward*depth
        for x,y in [(left,0),(right,0),(right,16),(left,16)]],[],[(0,1,2,3)])
    mesh.update();obj=bpy.data.objects.new(name,mesh);collection.objects.link(obj)
    obj['source_node']=node;obj['asset_group']=asset
    return obj
receiver=plane('chamber floor','building-249',0,16,0,'fixture')
cover=plane('removable cover','building-257',0,10,2,'context')
parapet=plane('retained parapet','building-257',4,6,3,'context')
outer=plane('outside covered blocker','outer',12,16,2,'context')
cover['projection_component']='upper-chamber-removable-cover'
cover['reveal_component_role']='removable-cover';cover['reveal_component_patch_id']='patch-003'
parapet['projection_component']='upper-chamber-retained-parapet'
parapet['reveal_component_role']='retained-parapet';parapet['reveal_component_patch_id']='patch-003'
selector=[{'source_node':'building-257','projection_component':'upper-chamber-removable-cover','patch_id':'patch-003'}]
objects=[receiver,cover,parapet,outer];nodes=['building-249','building-257','outer']
assert filter_occluders(objects,projection_label='exterior')==objects
assert filter_occluders(objects,selector,projection_label='interior-patch-003')==[receiver,parapet,outer]
for bad_label in ('exterior','interior-other'):
    try:filter_occluders(objects,selector,projection_label=bad_label)
    except ValueError:pass
    else:raise AssertionError('Invalid component projection accepted')
try:filter_occluders([receiver,parapet,outer],selector,projection_label='interior-patch-003')
except ValueError:pass
else:raise AssertionError('Missing split cover accepted')
region={'alpha_path':alpha,'alpha_sha256':sha(alpha),'bbox':[0,0,8,16],
    'source_sha256':sha(revealed),'state':'revealed/patch-003',
    'fallback':{'source_path':covered,'source_sha256':sha(covered),'state':'covered/patch-003',
                'projection_label':'exterior','occluder_nodes':nodes,'include_components':selector}}
bpy.context.view_layer.update()
old=bake('CoverFixture',revealed,out/'old.json',receiver_nodes=['building-249'],
    occluder_nodes=nodes,projection_label='interior-patch-003',projection_region=region)
new=bake('CoverFixture',revealed,out/'new.json',receiver_nodes=['building-249'],
    occluder_nodes=nodes,projection_label='interior-patch-003',projection_region=region,
    exclude_occluder_components=selector)
assert new['known_texels']>old['known_texels']+40
assert new['unknown_texels']>40
assert new['objects'][0]['exterior_fallback_known_texels']==old['objects'][0]['exterior_fallback_known_texels']
definition={'source_path':revealed,'receiver_nodes':['building-249'],'occluder_nodes':nodes,
    'projection_label':'interior-patch-003','projection_region':region,'exclude_occluder_components':selector}
inside=plane('west retained','building-263',30,32,0,'context')
outside=plane('west cover','building-263',30,32,2,'context')
for obj,component in [(inside,'west-retained'),(outside,'west-cover')]:
    obj['projection_component']=component;obj['reveal_component_patch_id']='patch-003'
inner_selector=[{'source_node':'building-263','projection_components':['west-retained'],'patch_id':'patch-003'}]
outer_selector=[{'source_node':'building-263','projection_components':['west-cover'],'patch_id':'patch-003'}]
assert filter_receivers([inside,outside],inner_selector)==[inside]
assert filter_receivers([inside,outside],outer_selector)==[outside]
definition['receiver_nodes'].append('building-263');definition['receiver_components']=inner_selector
exterior_definition={'source_path':covered,'receiver_nodes':['building-263'],'occluder_nodes':nodes+['building-263'],
    'projection_label':'exterior','receiver_components':outer_selector}
before_geometry=[tuple(tuple(v.co) for v in o.data.vertices) for o in objects]
review=render_review(out/'review',scene_name=bpy.context.scene.name,collection_name=collection.name,
    asset_id='fixture',source_path=covered,width=32,height=32,projection_layers=[definition,exterior_definition],source_mask_manifest=mask_manifest)
after_geometry=[tuple(tuple(v.co) for v in o.data.vertices) for o in objects]
assert before_geometry==after_geometry and not cover.hide_render and not parapet.hide_render
assert review['source_constraint_status'][0]['constrained'] is True
old_frozen=json.loads(json.dumps(review))
del old_frozen['projection_layers'][0]['exclude_occluder_components']
try:
    render_review(out/'rejected-migration',scene_name=bpy.context.scene.name,collection_name=collection.name,
        asset_id='fixture',source_path=covered,frame_manifest=old_frozen,projection_layers=[definition,exterior_definition],source_mask_manifest=mask_manifest)
except ValueError:pass
else:raise AssertionError('Frozen component ownership changed silently')
img=bpy.data.images.load(str(out/'review/views/view-0-textured.png'))
rgb=np.asarray(img.pixels[:]).reshape(32,32,4)[:,:,:3];bpy.data.images.remove(img)
assert ((rgb[:,:,1]>.9)&(rgb[:,:,0]<.01)).sum()>150
assert ((rgb[:,:,0]>.9)&(rgb[:,:,1]<.01)).sum()>40
# The retained strip x4..6 and covered-only cover extension x8..10 remain gray.
camera=review['views'][0];scale=camera['ortho_scale']/32
for x in (5,9):
    px=int((x-camera['camera_location'][0])/scale+16)
    color=rgb[16,px]
    assert abs(float(color[0])-float(color[1]))<.01, (x,color)
cover.hide_render=True
revealed_review=render_review(out/'revealed-review',scene_name=bpy.context.scene.name,collection_name=collection.name,
    asset_id='fixture',source_path=covered,width=32,height=32,projection_layers=[definition,exterior_definition],source_mask_manifest=mask_manifest)
assert revealed_review['views'][0]['counts']==review['views'][0]['counts']
cover.hide_render=False
# A regional floor remains a physical blocker for an exterior wall even though
# its source artwork is selected by a different projection pass.
near_floor=plane('nearer same-asset floor','building-252',0,16,5,'fixture')
bpy.context.view_layer.update()
wall_without_floor=bake('CoverFixture',covered,out/'wall-without-floor.json',receiver_nodes=['building-249'],
    occluder_nodes=['building-249'],projection_label='exterior')
wall_with_floor=bake('CoverFixture',covered,out/'wall-with-floor.json',receiver_nodes=['building-249'],
    occluder_nodes=['building-249','building-252'],projection_label='exterior')
assert wall_without_floor['known_texels']>0 and wall_with_floor['known_texels']==0
print(json.dumps({'status':'PASS','output':str(out),'old_known':old['known_texels'],
    'new_known':new['known_texels'],'fallback_known':new['objects'][0]['exterior_fallback_known_texels'],
    'review_counts':review['views'][0]['counts']}))
