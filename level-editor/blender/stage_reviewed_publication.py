"""Stage selected asset handoffs, reconcile grouping, and export one coherent map."""
import hashlib
import json
from pathlib import Path
import sys

sys.path.insert(0,str(Path(__file__).resolve().parent))
import bpy
from import_reviewed_geometry import import_asset_geometry
from integrate_refinement import import_asset_textures
from group_assets import reconcile_asset_groups
from export_editor import export_editor, export_asset_library
from render_views import render_views


def stage(plan_path):
    plan_path=Path(plan_path).resolve(strict=True)
    plan=json.loads(plan_path.read_text())
    output=Path(plan['output']).resolve()
    output.mkdir(parents=True,exist_ok=False)
    for item in plan['imports']:
        if item.get('discover_asset_members'):
            bpy.ops.wm.open_mainfile(filepath=str(Path(item['blend_path']).resolve(strict=True)))
            item['object_names']=sorted(o.name for o in bpy.data.collections[plan['collection_name']].all_objects
                if o.type=='MESH' and not o.hide_render and o.get('asset_group')==item['asset_id'])
    bpy.ops.wm.open_mainfile(filepath=str(Path(plan['baseline']).resolve(strict=True)))
    bpy.context.window.scene=bpy.data.scenes[plan['scene_name']]
    collection=bpy.data.collections[plan['collection_name']]
    canonical_before={o.get('source_node') for o in collection.all_objects
                      if o.type=='MESH' and o.get('source_node')!='ground'}
    imports=[]
    for item in plan['imports']:
        packet=json.loads(Path(item['review_manifest']).read_text())
        names=item.get('object_names',packet['object_names'])
        blend=item['blend_path']
        blend_hash=hashlib.sha256(Path(blend).read_bytes()).hexdigest()
        if item.get('blend_sha256') and item['blend_sha256']!=blend_hash:
            raise ValueError('Reviewed model changed before staging: '+item['asset_id'])
        result=import_asset_geometry(blend,asset_id=item['asset_id'],object_names=names,
            collection_name=collection.name,source_nodes=item.get('source_nodes'))
        if item.get('texture_handoff'):
            result['texture_handoff']=import_asset_textures(item['texture_handoff'],asset_id=item['asset_id'],
                collection_name=collection.name,source_nodes=item.get('source_nodes'))
        result['review_manifest']=item['review_manifest']
        result['source_blend_sha256']=blend_hash
        result['review_manifest_sha256']=hashlib.sha256(Path(item['review_manifest']).read_bytes()).hexdigest()
        imports.append(result)
        print('INTEGRATED '+item['asset_id'],flush=True)
    canonical_after={o.get('source_node') for o in collection.all_objects
                     if o.type=='MESH' and o.get('source_node')!='ground'}
    if canonical_before!=canonical_after or len(canonical_after)!=270:
        raise ValueError('Publication changed canonical part coverage')
    grouping=reconcile_asset_groups(plan['catalog'])
    ground_handoff=None
    if plan.get('ground_texture_handoff'):
        item=plan['ground_texture_handoff']
        proof=json.loads(Path(item['proof']).read_text())
        if proof.get('status')!='PASS' or not all(proof.get(key) for key in (
                'outside_mask_pixels_identical','alpha_identical','all_object_geometry_identical',
                'terrain_uv_identical','outside_material_assignments_identical')):
            raise ValueError('Ground cleanup has incomplete scope evidence')
        if proof['original_atlas_sha256']!=item['baseline_atlas_sha256']:
            raise ValueError('Ground cleanup evidence names a different baseline')
        if item['requires_asset_id'] not in {entry['asset_id'] for entry in plan['imports']}:
            raise ValueError('Ground cleanup requires its replacement geometry')
        grounds=[o for o in collection.all_objects if o.type=='MESH' and not o.hide_render and o.get('source_node')=='ground']
        if len(grounds)!=1:
            raise ValueError('Expected exactly one ground receiver')
        ground=grounds[0]
        image=next(n.image for n in ground.data.materials[0].node_tree.nodes if n.type=='TEX_IMAGE' and n.image)
        if hashlib.sha256(image.packed_file.data).hexdigest()!=item['baseline_atlas_sha256']:
            raise ValueError('Ground cleanup baseline changed')
        ground_handoff=import_asset_textures(item['blend_path'],asset_id=ground.get('asset_group'),
            collection_name=collection.name,source_nodes=['ground'])
        image=next(n.image for n in ground.data.materials[0].node_tree.nodes if n.type=='TEX_IMAGE' and n.image)
        if hashlib.sha256(image.packed_file.data).hexdigest()!=proof['output_atlas_sha256']:
            raise ValueError('Ground cleanup handoff differs from reviewed atlas')
        ground['ground_cleanup_report']=item['proof']
    generated={}
    for obj in collection.all_objects:
        if obj.type!='MESH' or obj.hide_render:
            continue
        for face in obj.data.polygons:
            mat=obj.data.materials[face.material_index]
            if mat and mat.get('generated_source_sha256'):
                generated.setdefault(mat['generated_source_sha256'],set()).add(mat.name)
    bpy.ops.wm.save_as_mainfile(filepath=str(output/'worker.blend'))
    report={'plan':str(plan_path),'imports':imports,'grouping':grouping,'ground_texture_handoff':ground_handoff,
            'canonical_parts':len(canonical_after),
            'generated_materials':{sha:sorted(names) for sha,names in generated.items()},
            'map':export_editor(plan['map_name'],output/'derby.scene.glb'),
            'assets':export_asset_library(plan['map_name'],output/'assets',plan['hackable_map'])}
    (output/'stage.json').write_text(json.dumps(report,indent=2)+'\n')
    render_views(plan['scene_name'],{'reference':plan['reference_camera']},output/'full-map',width=1920)
    return report


if __name__=='__main__':
    report=stage(sys.argv[sys.argv.index('--')+1])
    print(json.dumps({'map':report['map'],'assets':report['assets'],'imports':len(report['imports'])}),flush=True)
