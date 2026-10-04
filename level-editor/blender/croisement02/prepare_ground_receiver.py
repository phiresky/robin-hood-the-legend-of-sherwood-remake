"""Freeze a standalone flat-ground receiver review; never generate or publish texture."""
import argparse
import json
import math
import shutil
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT, reviewed_catalog, bank_workspace
from evidence_io import sha,write_json,digest
from render_slots import acquire,release
from review_bank_candidate import camera
from tree_geometry import SIN,RAY
ASSET='croisement02-ground-receiver'
SCENE='Croisement02 Refinement'
COLLECTION='Croisement02 Working'


def material(obj,path):
    image=bpy.data.images.load(str(path),check_existing=False);image.alpha_mode='CHANNEL_PACKED';image.pack()
    mat=bpy.data.materials.new('Ground observed source; unknown neutral');mat.use_nodes=True
    nodes=mat.node_tree.nodes;nodes.clear()
    uv=nodes.new('ShaderNodeUVMap');uv.uv_map=obj.data.uv_layers.active.name
    tex=nodes.new('ShaderNodeTexImage');tex.image=image;tex.interpolation='Closest'
    emission=nodes.new('ShaderNodeEmission');out=nodes.new('ShaderNodeOutputMaterial')
    mat.node_tree.links.new(uv.outputs['UV'],tex.inputs['Vector']);mat.node_tree.links.new(tex.outputs['Color'],emission.inputs['Color']);mat.node_tree.links.new(emission.outputs[0],out.inputs[0])
    obj.data.materials.clear();obj.data.materials.append(mat)
    for polygon in obj.data.polygons:polygon.material_index=0
    return mat


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,default=OUT/'ground-receiver-review-v1')
    parser.add_argument('--catalog',type=Path,default=reviewed_catalog())
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    dest=args.output.resolve()
    if dest.exists():raise FileExistsError(dest)
    dest.mkdir(parents=True);reference=dest/'reference';reference.mkdir()
    proposal=OUT/'ground-texture-preparation';packet=json.loads((proposal/'packet.json').read_text())
    for filename,expected in packet['files'].items():
        if sha(proposal/filename)!=expected:raise ValueError('Changed domain evidence '+filename)
        shutil.copy2(proposal/filename,reference/filename)
    for filename in ['packet.json','state-source-preservation.json']:shutil.copy2(proposal/filename,reference/filename)
    shutil.copy2(args.catalog,reference/'catalog.json')
    catalog=json.loads((reference/'catalog.json').read_text())
    if len(catalog['groups'])!=78:raise ValueError('Expected frozen78 ownership snapshot')
    source=reference/'source.png';shutil.copy2(packet['source'],source)
    if sha(source)!=packet['source_sha256']:raise ValueError('Source changed')
    known=np.asarray(Image.open(reference/'ground-observed-domain.png'))>0
    hidden=np.asarray(Image.open(reference/'ground-hidden-domain.png'))>0
    ground_domain=np.asarray(Image.open(reference/'ground-first-hit.png'))>0
    source_rgb=np.asarray(Image.open(source).convert('RGB'))
    atlas=source_rgb.copy();atlas[~known]=[127,127,127]
    Image.fromarray(atlas).save(reference/'observed-neutral.png')
    diagnostic=np.full_like(source_rgb,[55,65,75]);diagnostic[known]=[30,190,195];diagnostic[hidden]=[185,65,175]
    Image.fromarray(diagnostic).save(reference/'known-unknown.png')
    inventory={'masks':[dict(index=460,layer=None,layer_index=None,png=str(reference/'ground-observed-domain.png'),box_top_left=[0,0],box_size=[1792,1152],synthetic=True,constraint_kind='authored-receiver-domain-proposal')]}
    write_json(dest/'inventory.json',inventory)
    write_json(dest/'source-masks.json',dict(version=1,mask_inventory=str(dest/'inventory.json'),projections=dict(exterior=dict(state='Initial covered source state; ground-only authored domain proposal.',source_sha256=sha(source),assignments=[dict(source_node='ground',mask_indices=[460],reviewed=True,review_note='Source ownership self-review only; user geometry/domain approval pending.')] ))))
    holds=[dict(name='ground plants111–123',domain=str(reference/'pending-ground-plants.png'),pixels=packet['pending_ground_plant_pixels'],effect='Foreground exclusion retained. Geometry is pending separately; underlying hidden ground remains a fill candidate, never a claim that plants are complete.'),dict(name='southwest log102/103 and northeast rootbank370',effect='Entire native log silhouettes and authored rootbank domain already excluded. Missing model coverage cannot transfer their artwork to ground.'),dict(name='mission sprite and terminal background domains',effect='Each state retains independent image hashes/bboxes. Only confirmed barrier terminal background is a ground state assignment; other frame reservations do not imply ground ownership.')]
    plant=OUT/'ground-plant-candidates/ownership/partition-audit.json'
    if plant.exists():holds[0]['ownership_proposal']={'path':str(plant),'sha256':sha(plant)}
    write_json(dest/'domain-review.json',dict(status='self-review pending; user approval required before API',catalog_sha256=sha(reference/'catalog.json'),catalog_groups=78,domain_index=460,known_pixels=int(known.sum()),hidden_pixels=int(hidden.sum()),receiver_pixels=int(ground_domain.sum()),holds=holds,scope='Native flat game plane, separate from bank0–4 and authored scenery relief. Known source excludes every reserved foreground silhouette; unseen plane remains neutral.',state_evidence_sha256=sha(reference/'state-source-preservation.json')))
    acquire()
    try:
        from refinement_workspace import _ownership,_files,_freeze_masks,validate
        from refinement_review import render_review
        frozen=OUT/'forest-v4-input.blend';frozen_hash=sha(frozen)
        bpy.ops.wm.open_mainfile(filepath=str(frozen));scene=bpy.data.scenes[SCENE];bpy.context.window.scene=scene
        collection=bpy.data.collections[COLLECTION]
        grounds=[o for o in collection.all_objects if o.type=='MESH' and o.get('source_node')=='ground']
        if len(grounds)!=1:raise ValueError('Expected one native ground')
        ground=grounds[0];ground['asset_group']=ASSET;ground['asset_name']='Croisement02 flat ground receiver'
        geometry=digest({'vertices':[list(v.co) for v in ground.data.vertices],'faces':[list(p.vertices) for p in ground.data.polygons],'matrix':[list(r) for r in ground.matrix_world]})
        bank=bank_workspace('croisement02-north-woodland-bank');bank_hash=sha(bank/'model.blend')
        for obj in list(collection.all_objects):
            if obj.type=='MESH' and obj.get('source_node') in {f'building-{i:03}' for i in range(5)}:bpy.data.objects.remove(obj,do_unlink=True)
        names=[r['object'] for r in json.loads((bank/'inspection/saved-model-audit.json').read_text())['objects']]
        with bpy.data.libraries.load(str(bank/'model.blend'),link=False) as (src,target):target.objects=names
        for obj in target.objects:collection.objects.link(obj)
        bpy.context.view_layer.update()
        for obj in target.objects:
            matrix=obj.matrix_world.copy();obj.parent=None;obj.matrix_world=matrix;obj.hide_render=False
        bpy.context.view_layer.update()
        bpy.context.preferences.filepaths.save_version=0
        config=dict(version=1,asset_id=ASSET,scene_name=SCENE,collection_name=COLLECTION,map_name='Croisement02',part_ids=['ground'],source_blend=str(frozen),source_blend_sha256=frozen_hash,source_path=str(source),source_mask_manifest=str(dest/'source-masks.json'),projection_manifest=None,width=512,height=384,elevation_degrees=35,context_padding=0,terrain_role='Standalone flat game plane outside scenery catalog; ownership snapshot78, native context plus strict bank geometry.')
        _,config['outside_geometry']=_ownership(config);_freeze_masks(dest,config)
        bpy.ops.wm.save_as_mainfile(filepath=str(dest/'baseline.blend'),copy=True);config['baseline_sha256']=sha(dest/'baseline.blend')
        options=dict(scene_name=SCENE,collection_name=COLLECTION,asset_id=ASSET,source_path=source,source_mask_manifest=dest/'source-masks.json',width=512,height=384,context_padding=0)
        render_review(dest/'input',**options)
        config['input_files']=_files(dest/'input');config['reference_files']=_files(reference);write_json(dest/'workspace.json',config)
        material(ground,reference/'observed-neutral.png')
        bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'))
        render_review(dest/'modified',frame_manifest=dest/'input/views.json',**options)
        validation=validate(dest);validation.update(model_sha256=sha(dest/'model.blend'),geometry_signature=geometry,bank_model_sha256=bank_hash,approval='pending',api='not run',context='Native proxy geometry plus strict bank. Not a frozen78 full geometry assembly; catalog78 records source ownership only.')
        write_json(dest/'validation.json',validation)
        # Actual saved materials, separately from shared reprojection diagnostics.
        actual=bpy.data.scenes.new('Ground actual material review')
        duplicate=ground.copy();duplicate.data=ground.data.copy();duplicate.parent=None;duplicate.matrix_world=ground.matrix_world.copy();actual.collection.objects.link(duplicate)
        review=dest/'inspection';review.mkdir()
        camera(actual,Vector((896,-576/SIN,0)),RAY,1792,1152,1792)
        actual.render.filepath=str(review/'actual-source.png');bpy.ops.render.render(write_still=True,scene=actual.name)
        for label,path in [('actual',reference/'observed-neutral.png'),('domains',reference/'known-unknown.png')]:
            material(duplicate,path)
            for index,angle in enumerate([math.pi/4,5*math.pi/4]):
                direction=Vector((math.sin(angle)*.82,-math.cos(angle)*.82,.57))
                camera(actual,Vector((896,-576/SIN,0)),direction,1200,850,3100)
                actual.render.filepath=str(review/f'{label}-oblique-{index}.png');bpy.ops.render.render(write_still=True,scene=actual.name)
        if sha(frozen)!=frozen_hash or sha(bank/'model.blend')!=bank_hash:raise ValueError('External inputs changed')
        write_json(dest/'receiver-packet.json',dict(status='prepared; visual inspection pending',model_sha256=sha(dest/'model.blend'),workspace_sha256=sha(dest/'workspace.json'),domain_review_sha256=sha(dest/'domain-review.json'),files={str(p.relative_to(dest)):sha(p) for p in sorted(dest.rglob('*.png'))},approval='pending',api='not run'))
    finally:release()


if __name__=='__main__':main()
