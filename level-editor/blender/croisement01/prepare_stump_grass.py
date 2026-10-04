"""Rebuild the stump/grass assembly with distinct bark and foliage ownership."""
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
    parser=argparse.ArgumentParser();parser.add_argument('--low-field',action='store_true')
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    acquire()
    destination=OUT/('stump65-assembly-v4' if args.low_field else 'stump65-assembly-v3');destination.mkdir(exist_ok=False)
    source=OUT/'stump65-source-split-v3'
    partition=json.loads((source/'partition.json').read_text())
    packet=json.loads((source/'grass-packet.json').read_text())
    asset='croisement01-southwest-cut-stump';node=packet['source_node']
    bpy.ops.wm.open_mainfile(filepath=str(OUT/'croisement01-grouped.blend'))
    bpy.context.preferences.filepaths.save_version=0
    working=bpy.data.collections['Croisement01 Working']
    wood=next(o for o in working.all_objects if o.type=='MESH' and o.get('asset_group')==asset)
    grass=bpy.data.objects.new('Southwest Cut Stump / Basal Grass',bpy.data.meshes.new('Basal Grass'))
    working.objects.link(grass);grass.parent=wood.parent;grass.matrix_world=Matrix.Identity(4)
    grass['source_node']=node;grass['asset_group']=asset;grass['asset_name']='Southwest Cut Stump';grass['part_name']='Basal Grass'
    bpy.context.view_layer.update()
    # Reuse the already reviewed rooted-blade construction algorithm with this
    # map's own pixels; no other map's plant asset or appearance is imported.
    sys.path.append(str(ROOT/'level-editor/blender/croisement02'))
    from ground_plant_geometry import build
    grass_report=build(grass,packet)
    if args.low_field:
        # The native residual is field grass around the stump, not a separate
        # tall fountain-shaped tuft. Keep every source projection and UV while
        # lowering its ground cover and retaining the stems nearest the wood.
        from mathutils import Vector
        sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
        inverse=grass.matrix_world.inverted();drift=0.
        for vertex in grass.data.vertices:
            point=grass.matrix_world@vertex.co
            projected_y=-point.y*sine-point.z*cosine
            tall_fraction=.3+.7/(1+math.exp(max(-50,min(50,(point.x-578)/2))))
            z=max(.05,(701-projected_y)/cosine*tall_fraction)
            corrected=Vector((point.x,(-projected_y-z*cosine)/sine,z))
            drift=max(drift,abs((-corrected.y*sine-corrected.z*cosine)-projected_y))
            vertex.co=inverse@corrected
        grass.data.update()
        grass_report.update(geometry_version='native-source-field-grass-v1',source_projection_max_drift=drift,
            interpretation='Low field grass with a taller fringe beside the stump, not an isolated tall tuft.')
    for material in grass.data.materials:
        material['texture_provenance']='observed Croisement01 grass front' if material.get('foliage_observed') else 'inferred rear/side using only this Croisement01 grass native palette'
    catalog=json.loads((OUT/'catalog.json').read_text());catalog['version']=2
    catalog['canonical_owners']={f"building-{p['obstacle']:03}":g['id'] for g in catalog['groups'] for p in g['parts']}
    group=next(g for g in catalog['groups'] if g['id']==asset)
    group['parts'].append(dict(node=node,name='Basal Grass'))
    catalog['canonical_owners'][node]=asset
    catalog_path=destination/'catalog.json';catalog_path.write_text(json.dumps(catalog,indent=2)+'\n')
    bpy.ops.wm.save_as_mainfile(filepath=str(destination/'input.blend'))
    inventory(destination/'inventory',collection_name=working.name,map_name='Croisement01',source_path=OUT/'baseline/covered.png',patch_manifest=OUT/'source-states/layers.json')
    native=json.loads((OUT/'baseline/masks/manifest.json').read_text())
    for row in native['masks']:row['png']=str(OUT/'baseline/masks'/row['png'])
    box=packet['native_bbox']
    for index,name in [(200,'wood'),(201,'grass')]:
        native['masks'].append(dict(index=index,layer=0,png=str(source/(name+'-domain.png')),box_top_left=box[:2],box_size=box[2:],
            provenance='Native mask 65 partition: visible bark/cap versus foreground grass. No overlap or discarded pixels.'))
    ground=destination/'ground-domain.png';Image.new('L',(1408,960),255).save(ground)
    native['masks'].append(dict(index=202,layer=0,png=str(ground),box_top_left=[0,0],box_size=[1408,960],provenance='Ground context domain; grass exclusion is explicit, whole-map ownership remains unfinished.'))
    mask_inventory=destination/'masks.json';mask_inventory.write_text(json.dumps(native,indent=2)+'\n')
    assignments=[dict(reviewed=True,source_node='building-055',mask_indices=[200]),
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
        evidence='One native stump plus its surrounding grass share a logical assembly; source partition v3 visually inspected. Other groups remain provisional.'),indent=2)+'\n')
    worker=destination/'assets'/asset
    prepare(worker,asset_id=asset,scene_name='Croisement01 Refinement',collection_name=working.name,
        source_path=OUT/'baseline/covered.png',grouping_manifest=catalog_path,
        inventory_path=destination/'inventory/inventory.json',review_path=grouping,
        source_mask_manifest=masks,width=256,height=256,framing_padding=1.2,
        lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05))
    level=json.loads((OUT/'baseline/Croisement01.rhp.json').read_text())
    profile=None
    if args.low_field:
        profile=json.loads((OUT/'stump65-profile-fit-v1/fit.json').read_text())['rows'][1]['parameters']
    wood_report=stump(wood,level['sight_obstacles'][55],55,profile=profile)
    validate(worker);modified(worker)
    inspection=worker/'inspection';inspection.mkdir(exist_ok=True)
    (inspection/'refinement.json').write_text(json.dumps(dict(asset_id=asset,model_sha256=sha(worker/'model.blend'),
        wood=wood_report,grass=grass_report,partition=partition,status='private candidate; saved-material and native joint inspection pending',
        limitations=['Source split follows the cap/bark contour; a few ivy-edge pixels remain uncertain.',
                     'Hidden grass blades and unseen wood depth are inferred.',
                     'Unobserved bark remains gray until reviewed geometry and texture approval.']),indent=2)+'\n')
    import render_candidate
    previous=sys.argv;sys.argv=['render_candidate.py','--',str(worker)]
    try:render_candidate.main()
    finally:sys.argv=previous
    print(worker)


if __name__=='__main__':main()
