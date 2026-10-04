"""Joint mask68 stump and separately identified native foreground grass80."""
import argparse
import json
import math
import sys
from pathlib import Path
import bpy
from mathutils import Matrix
from PIL import Image

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent))
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha
from prepare_props import stump
from render_slots import acquire
from refinement_workspace import prepare, modified, validate
from refinement_inventory import inventory


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--continuous-front',action='store_true');parser.add_argument('--radial-front',action='store_true');parser.add_argument('--leaf-front',action='store_true');parser.add_argument('--leaf-volume',action='store_true');parser.add_argument('--wood-review',action='store_true');parser.add_argument('--native-leaves',action='store_true')
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    if args.wood_review or args.native_leaves:args.leaf_volume=True
    if args.leaf_volume:args.leaf_front=True
    acquire()
    destination=OUT/('stump68-native-grass-v6' if args.native_leaves else 'stump68-wood-review-v1' if args.wood_review else 'stump68-native-grass-v5' if args.leaf_volume else 'stump68-native-grass-v4' if args.leaf_front else 'stump68-native-grass-v3' if args.radial_front else 'stump68-native-grass-v2' if args.continuous_front else 'stump68-native-grass-v1');destination.mkdir(exist_ok=False)
    source=destination/'source';source.mkdir()
    manifest=json.loads((OUT/'baseline/masks/manifest.json').read_text())['masks']
    wood_native=next(r for r in manifest if r['index']==68)
    grass_native=next(r for r in manifest if r['index']==80)
    native_image=Image.open(OUT/'baseline/covered.png').convert('RGBA')
    x,y,w,h=grass_native['box_top_left']+grass_native['box_size']
    grass_mask=Image.open(OUT/'baseline/masks'/grass_native['png']).convert('L')
    rgba=native_image.crop((x,y,x+w,y+h));rgba.putalpha(grass_mask)
    rgba.save(source/'complete-source.png');rgba.save(source/'observed-source.png');grass_mask.save(source/'grass-domain.png')
    from PIL import ImageChops
    wood_mask=Image.open(OUT/'prop-domains/small-stump-wood-domain.png').convert('L')
    foreign=Image.new('L',wood_mask.size)
    foreign.paste(grass_mask,(x-wood_native['box_top_left'][0],y-wood_native['box_top_left'][1]))
    wood_mask=ImageChops.subtract(wood_mask,foreign);wood_mask.save(source/'wood-domain.png')
    packet=dict(directory=str(source),native_bbox=[x,y,w,h],bbox=[x,y,w,h],native_mask=80,ground_z=0,
                source_node='foliage-stump68-grass80',observed_pixels=sum(v>0 for v in grass_mask.getdata()))
    partition=dict(status='private native-domain assembly',wood_native_mask=68,foreground_native_mask=80,
                   wood_domain_sha256=sha(source/'wood-domain.png'),grass_domain_sha256=sha(source/'grass-domain.png'),
                   evidence='Native grass80 blade silhouette overlaps the stump68 lower stem; independent native grass pixels retained.')
    (source/'packet.json').write_text(json.dumps(packet,indent=2)+'\n')
    (source/'ownership.json').write_text(json.dumps(partition,indent=2)+'\n')
    asset='croisement01-southeast-small-stump';node=packet['source_node']
    bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement01-grouped.blend'))
    bpy.context.preferences.filepaths.save_version=0
    working=bpy.data.collections['Croisement01 Working']
    wood=next(o for o in working.all_objects if o.type=='MESH' and o.get('asset_group')==asset)
    grass=bpy.data.objects.new('Southeast Small Stump / Basal Grass',bpy.data.meshes.new('Basal Grass'))
    working.objects.link(grass);grass.parent=wood.parent;grass.matrix_world=Matrix.Identity(4)
    grass_asset='croisement01-grass-80' if args.wood_review else asset
    grass['source_node']=node;grass['asset_group']=grass_asset;grass['asset_name']='Foreground Grass 80' if args.wood_review else 'Southeast Small Stump';grass['part_name']='Basal Grass'
    if args.wood_review:
        parent=bpy.data.objects.new('Foreground Grass 80',None);working.objects.link(parent);parent['asset_group']=grass_asset
        grass.parent=parent;grass.matrix_world=Matrix.Identity(4)
    bpy.context.view_layer.update()
    # Reuse the rooted-blade construction algorithm with this
    # map's own pixels; no other map's plant asset or appearance is imported.
    sys.path.append(str(ROOT/'level-editor/blender/croisement02'))
    from ground_plant_geometry import build
    grass_report=build(grass,packet)
    if args.continuous_front or args.radial_front or args.leaf_front:
        # Keep adjacent source pixels on a continuous curved surface. Independent
        # nearest-blade depths made otherwise connected grass look like confetti.
        from mathutils import Vector
        sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
        root_y=-grass_report['root_world'][1]*sine
        center_x=grass_report['root_world'][0]
        count=packet['observed_pixels']*18
        inverse=grass.matrix_world.inverted();drift=0.
        if args.leaf_front:
            import random
            leaf_path=OUT/('grass-080-leaf-paths-v3/paths.json' if args.native_leaves else 'grass-080-leaf-paths-v2/paths.json')
            leaves=json.loads(leaf_path.read_text())
            if len(leaves['observed_pixels'])!=packet['observed_pixels']:raise ValueError('Leaf trace alpha count changed')
            factors=[random.Random(8100+i).uniform(.35,1.45) if args.leaf_volume else random.Random(8100+i).uniform(.4,1.) for i in range(leaves['leaf_count'])]
            leans=[random.Random(8300+i).uniform(-.25,.25) if args.leaf_volume else 0 for i in range(leaves['leaf_count'])]
        for index,vertex in enumerate(grass.data.vertices):
            if index>=count:break
            point=grass.matrix_world@vertex.co;source_y=-point.y*sine-point.z*cosine
            fraction=.7+.22*math.cos((point.x-center_x)/w*math.pi)
            if args.radial_front:
                angle=math.atan2(max(0,root_y-source_y),point.x-center_x)
                fraction=.7+.4*math.sin(angle*13+.7)
            if args.leaf_front:fraction=factors[leaves['observed_pixels'][index//18]['leaf']]
            lean=leans[leaves['observed_pixels'][index//18]['leaf']] if args.leaf_front else 0
            z=max(.08,(root_y-source_y)/cosine*fraction+(point.x-center_x)*lean)
            layer_offset=[.02,0,-.01][(index//3)%3]
            corrected=Vector((point.x,(-source_y-z*cosine)/sine,z))+Vector((0,-cosine,sine))*layer_offset
            drift=max(drift,abs(-corrected.y*sine-corrected.z*cosine-source_y))
            vertex.co=inverse@corrected
        grass.data.update()
        if args.leaf_front:
            import bmesh
            bm=bmesh.new();bm.from_mesh(grass.data);bm.faces.ensure_lookup_table()
            bmesh.ops.delete(bm,geom=[f for f in bm.faces if f.index>=packet['observed_pixels']*6],context='FACES')
            bm.to_mesh(grass.data);bm.free()
            from opacity_bounds import measure
            grass_report.update(blade_count=leaves['leaf_count'],leaf_paths_sha256=sha(leaf_path),minimum_z=min(v.co.z for v in grass.data.vertices),opacity_bounds=measure(grass),
                                interpretation='Native leaf-path patches at coherent per-leaf depths; inferred reverse surfaces use the same own-plant source colors, not observed rear evidence.')
        grass_report.update(geometry_version='native-grass-leaf-volume-v1' if args.leaf_volume else 'native-grass-leaf-depth-v1' if args.leaf_front else 'native-grass-radial-source-front-v1' if args.radial_front else 'native-grass-continuous-source-front-v1',source_projection_max_drift=drift,
            limitation='Continuous curved native front plus inferred rooted blades; side appearance still requires review.')
    for material in grass.data.materials:
        material['texture_provenance']='observed Croisement01 grass front' if material.get('foliage_observed') else 'inferred rear/side using only this Croisement01 grass native palette'
    catalog=json.loads((OUT/'catalog.json').read_text());catalog['version']=2
    catalog['canonical_owners']={f"building-{p['obstacle']:03}":g['id'] for g in catalog['groups'] for p in g['parts']}
    group=next(g for g in catalog['groups'] if g['id']==asset)
    if args.wood_review:
        catalog['groups'].append(dict(id=grass_asset,name='Foreground Grass 80',parts=[dict(node=node,name='Basal Grass')],source_mask=80,status='private geometry; side appearance refinement pending'))
    else:group['parts'].append(dict(node=node,name='Basal Grass'))
    catalog['canonical_owners'][node]=grass_asset
    catalog_path=destination/'catalog.json';catalog_path.write_text(json.dumps(catalog,indent=2)+'\n')
    bpy.ops.wm.save_as_mainfile(filepath=str(destination/'input.blend'))
    inventory(destination/'inventory',collection_name=working.name,map_name='Croisement01',source_path=OUT/'baseline/covered.png',patch_manifest=OUT/'source-states/layers.json')
    native=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    for row in native['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
    box=packet['native_bbox']
    for index,name in [(200,'wood'),(201,'grass')]:
        native['masks'].append(dict(index=index,layer=0,png=str(source/(name+'-domain.png')),box_top_left=wood_native['box_top_left'] if name=='wood' else box[:2],box_size=wood_native['box_size'] if name=='wood' else box[2:],
            provenance='Native stump68 observed wood and foreground grass80; separate domains exclude foreign grass from wood.'))
    ground=destination/'ground-domain.png';Image.new('L',(1408,960),255).save(ground)
    native['masks'].append(dict(index=202,layer=0,png=str(ground),box_top_left=[0,0],box_size=[1408,960],provenance='Ground context domain; grass exclusion is explicit, whole-map ownership remains unfinished.'))
    mask_inventory=destination/'masks.json';mask_inventory.write_text(json.dumps(native,indent=2)+'\n')
    assignments=[dict(reviewed=True,source_node='building-059',mask_indices=[200]),
                 dict(reviewed=True,source_node=node,mask_indices=[201]),
                 dict(reviewed=True,source_node='ground',mask_indices=[202],exclude_mask_indices=[201],
                      exclusions_reviewed=True,exclusion_reason='Native foreground grass belongs to its authored scenery receiver.')]
    masks=destination/'source-masks.json';masks.write_text(json.dumps(dict(version=1,mask_inventory=str(mask_inventory),
        projections=dict(exterior=dict(state='Initial static stump with separately owned foreground grass',
            source_sha256=sha(OUT/'baseline/covered.png'),assignments=assignments,
            occluder_constraints=[dict(reviewed=True,source_node=node,
                receiver_nodes=sorted({'ground',*catalog['canonical_owners']}-{node}),mask_indices=[201],
                reason='Inferred grass volume cannot hide foreign native artwork outside its observed domain.')]))),indent=2)+'\n')
    grouping=destination/'grouping-review.json';grouping.write_text(json.dumps(dict(status='reviewed',reviewer='Codex',
        catalog_sha256=sha(catalog_path),inventory_sha256=sha(destination/'inventory/inventory.json'),
        evidence='Native stump68 and separate foreground grass80 share a joint review assembly; both untouched source contexts and mask80 blade silhouette inspected. Other groups remain provisional.'),indent=2)+'\n')
    worker=destination/'assets'/asset
    prepare(worker,asset_id=asset,scene_name='Croisement01 Refinement',collection_name=working.name,
        source_path=OUT/'baseline/covered.png',grouping_manifest=catalog_path,
        inventory_path=destination/'inventory/inventory.json',review_path=grouping,
        source_mask_manifest=masks,width=256,height=256,framing_padding=1.2,
        lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05))
    level=json.loads((OUT/'baseline/Croisement01.rhp.json').read_text())
    wood_report=stump(wood,level['sight_obstacles'][59],59)
    validate(worker);modified(worker)
    inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
    (inspection/'refinement.json').write_text(json.dumps(dict(asset_id=asset,model_sha256=sha(worker/'model.blend'),
        wood=wood_report,grass=grass_report,partition=partition,status='private candidate; saved-material and native joint inspection pending',
        limitations=['Observed cap and bare stem retain their pixels; grass80 has its independent native domain. Other peripheral field grass is not claimed as wood.',
                     'Hidden grass blades and unseen wood depth are inferred.',
                     'Unobserved bark remains gray until reviewed geometry and texture approval.']),indent=2)+'\n')
    import render_candidate
    previous=sys.argv;sys.argv=['render_candidate.py','--',str(worker)]
    if args.wood_review:sys.argv+=['--native-context-node',node]
    try:render_candidate.main()
    finally:sys.argv=previous
    print(worker)


if __name__=='__main__':main()
