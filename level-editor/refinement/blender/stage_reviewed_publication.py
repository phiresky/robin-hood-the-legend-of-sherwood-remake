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
from publication_contract import canonical_parts, scene_filename, validate_coverage


def stage(plan_path, lossy=True):
    """`lossy` (default on; plan `"lossy": false` or CLI `--no-lossy` disables it) also derives
    lossy.glb models and their previews for the staged catalog once its model bytes are final."""
    plan_path=Path(plan_path).resolve(strict=True)
    plan=json.loads(plan_path.read_text())
    catalog=json.loads(Path(plan['catalog']).read_text())
    expected=canonical_parts(catalog,plan['map_name'])
    scene_file=scene_filename(plan)
    output=Path(plan['output']).resolve()
    output.mkdir(parents=True,exist_ok=False)
    if plan.get('baseline_sha256') and hashlib.sha256(Path(plan['baseline']).read_bytes()).hexdigest()!=plan['baseline_sha256']:
        raise ValueError('Publication baseline changed')
    for path,digest in plan.get('protected_live_files',{}).items():
        if hashlib.sha256(Path(path).read_bytes()).hexdigest()!=digest:raise ValueError('Live publication input changed: '+path)
    texture_checks=[]
    if plan.get('approved_texture_imports'):
        if sorted(plan.get('export_asset_ids',[]))!=sorted(item['asset_id'] for item in plan['imports']):
            raise ValueError('Texture publication must export exactly its approved imports')
        if any(item.get('texture_handoff') for item in plan['imports']) or plan.get('ground_texture_handoff'):
            raise ValueError('Approval-validated bakes cannot be overridden by another texture handoff')
        sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
        from publication_preflight import run_blender
        texture_checks = run_blender(plan, plan_path)
    ground_imports=[item for item in plan['imports'] if item.get('source_nodes')==['ground']]
    if ground_imports and (not plan.get('approved_texture_imports') or len(ground_imports)!=1):
        raise ValueError('Planar ground requires one approval-validated texture handoff')
    for item in plan['imports']:
        if item.get('discover_asset_members'):
            bpy.ops.wm.open_mainfile(filepath=str(Path(item['blend_path']).resolve(strict=True)))
            item['object_names']=sorted(o.name for o in bpy.data.collections[plan['collection_name']].all_objects
                if o.type=='MESH' and not o.hide_render and o.get('asset_group')==item.get('source_asset_id',item['asset_id'])
                and (not item.get('source_nodes') or o.get('source_node') in item['source_nodes']))
    compiled = {}
    if plan.get('approved_texture_imports'):
        from compile_texture_states import compile_states
        for item in plan['imports']:
            result = compile_states(item, output/'state-workers'/(item['asset_id']+'.blend'))
            if result:
                compiled[item['asset_id']] = result
        # Applied drawbridge endpoints are exported independently at the same pivot.
        plan['static_variants'] = [
            {'asset_id': item['asset_id'], 'initial_name': 'Initial',
             'states': {'initial': dict(item, name='Initial'), 'applied': dict(child)}}
            for item in plan['imports'] for child in item.get('texture_states', [])
            if child.get('endpoint_id') == 'applied']
    bpy.ops.wm.open_mainfile(filepath=str(Path(plan['baseline']).resolve(strict=True)))
    bpy.context.window.scene=bpy.data.scenes[plan['scene_name']]
    collection=bpy.data.collections[plan['collection_name']]
    canonical_before={o.get('source_node') for o in collection.all_objects
                      if o.type=='MESH' and o.get('source_node')!='ground'}
    added_nodes={node for item in plan['imports'] if item.get('new_mission_part') or item.get('new_scenery_part')
                 for node in item['source_nodes']}
    if added_nodes & canonical_before:raise ValueError('New supplemental node overlaps existing canonical part')
    validate_coverage(canonical_before,expected-added_nodes)
    from refinement_workspace import _geometry
    from bake_reviewed_asset import _materials
    selected_nodes={node for item in plan['imports'] for node in item.get('source_nodes',[])}
    def outside_state():
        return {o.name:(_geometry(o),_materials(o)) for o in collection.all_objects
            if o.type=='MESH' and o.get('source_node') not in selected_nodes}
    untouched_objects = [o for o in collection.all_objects
                         if o.type == 'MESH' and o.get('source_node') not in selected_nodes
                         and o.get('source_node') != 'ground']
    outside_before=outside_state()
    imports=[]
    binding_objects=[]
    for item in plan['imports']:
        if item in ground_imports:continue
        packet=json.loads(Path(item['review_manifest']).read_text())
        names=item['object_names'] if 'object_names' in item else packet['object_names']
        state_compilation=compiled.get(item['asset_id'])
        if state_compilation:
            names=state_compilation['object_names']
        blend=state_compilation['blend_path'] if state_compilation else item['blend_path']
        blend_hash=hashlib.sha256(Path(blend).read_bytes()).hexdigest()
        expected_blend_hash=state_compilation['blend_sha256'] if state_compilation else item.get('blend_sha256')
        if expected_blend_hash and expected_blend_hash!=blend_hash:
            raise ValueError('Reviewed model changed before staging: '+item['asset_id'])
        if item.get('new_mission_part'):
            from supplemental_parts import import_mission_part
            result=import_mission_part(item,collection.name)
        elif item.get('new_scenery_part'):
            from supplemental_parts import import_scenery_part
            result=import_scenery_part(item,collection.name)
        else:
            result=import_asset_geometry(blend,asset_id=item['asset_id'],object_names=names,
                collection_name=collection.name,source_nodes=item.get('source_nodes'),source_asset_id=item.get('source_asset_id'),
                replace_hidden_source_nodes=item.get('endpoint_id') == 'initial' or bool(state_compilation and state_compilation.get('inactive_object_bindings')),
                inactive_object_bindings=state_compilation.get('inactive_object_bindings') if state_compilation else None)
        if item.get('texture_handoff'):
            result['texture_handoff']=import_asset_textures(item['texture_handoff'],asset_id=item['asset_id'],
                collection_name=collection.name,source_nodes=item.get('source_nodes'))
        if state_compilation:
            result.update(state_compilation)
            binding_objects.append((result, {name: bpy.data.objects[name] for name in names}))
        result['review_manifest']=item['review_manifest']
        result['source_blend_sha256']=blend_hash
        result['review_manifest_sha256']=hashlib.sha256(Path(item['review_manifest']).read_bytes()).hexdigest()
        imports.append(result)
        print('INTEGRATED '+item['asset_id'],flush=True)
    canonical_after={o.get('source_node') for o in collection.all_objects
                     if o.type=='MESH' and o.get('source_node')!='ground'}
    validate_coverage(canonical_after,expected)
    grouping=reconcile_asset_groups(plan['catalog'], preserve_objects=untouched_objects)
    for result, objects in binding_objects:
        for state in result['state_bindings']:
            for row in state['objects']:
                row['staged_name'] = objects[row['staged_name']].name
        for row in result.get('inactive_object_bindings', []):
            row['staged_name'] = objects[row['staged_name']].name
        result['object_names'] = sorted(obj.name for obj in objects.values())
    ground_handoff=None
    if ground_imports:
        item=ground_imports[0]
        grounds=[o for o in collection.all_objects if o.type=='MESH' and o.get('source_node')=='ground' and not o.hide_render]
        if len(grounds)!=1:raise ValueError('Expected one live ground receiver')
        ground=grounds[0]
        root=bpy.data.objects.new(item['asset_id'],None);collection.objects.link(root)
        root['asset_group']=item['asset_id'];root['asset_name']='Ground Background'
        root.parent=ground.parent
        matrix=ground.matrix_world.copy();ground.parent=root;ground.matrix_world=matrix
        ground['asset_group']=item['asset_id'];ground['asset_name']='Ground Background'
        bpy.context.view_layer.update()
        ground_handoff=import_asset_geometry(item['blend_path'],asset_id=item['asset_id'],
            object_names=item['object_names'],collection_name=collection.name,source_nodes=['ground'])
        ground_handoff['projection_kind']=item['projection_kind']
        ground_handoff['source_blend_sha256']=item['blend_sha256']
        imports.append(ground_handoff)
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
    if plan.get('approved_texture_imports'):
        for item in plan['imports']:
            for path,digest in item['protected_files'].items():
                if hashlib.sha256(Path(path).read_bytes()).hexdigest()!=digest:
                    raise ValueError('Approved evidence changed during stage: '+path)
    outside_after=outside_state()
    if outside_before!=outside_after:
        raise ValueError('Staging changed unselected live mesh state')
    generated={}
    for obj in collection.all_objects:
        if obj.type!='MESH' or obj.hide_render:
            continue
        from patch_material_export import state_record
        states=state_record(obj)
        slots=set(states['covered']+states['revealed']) if states else {face.material_index for face in obj.data.polygons}
        for slot in slots:
            mat=obj.data.materials[slot]
            if mat and mat.get('generated_source_sha256'):
                generated.setdefault(mat['generated_source_sha256'],set()).add(mat.name)
    bpy.ops.wm.save_as_mainfile(filepath=str(output/'worker.blend'))
    effective_plan=output/'effective-plan.json'
    effective_plan.write_text(json.dumps(plan,indent=2)+'\n')
    settings_path = Path(plan.get('editor_document', Path('level-editor/library/scenes') / scene_file))
    map_settings = json.loads(settings_path.read_text()) if settings_path.exists() else plan.get('map_settings', {})
    inactive_names = [row['staged_name'] for item in imports for row in item.get('inactive_object_bindings', [])]
    report={'plan':str(effective_plan),'imports':imports,'grouping':grouping,'ground_texture_handoff':ground_handoff,
            'canonical_parts':len(canonical_after),'approved_texture_checks':texture_checks,
            'unselected_meshes_preserved':len(outside_before),'unselected_mesh_state_identical':outside_before==outside_after,
            'generated_materials':{sha:sorted(names) for sha,names in generated.items()},
            'assets':export_asset_library(plan['map_name'],output/'assets',plan['hackable_map'],asset_ids=plan.get('export_asset_ids'),catalog=catalog,include_hidden_objects=inactive_names)}
    if plan.get('static_variants') or any(item.get('texture_state_roles') for item in plan['imports']):
        from export_static_variants import export_variants
        report['static_variants']=export_variants(plan,output)
    report['map']=export_editor(plan['map_name'],output/scene_file,catalog=catalog,
        level=json.loads(Path(plan['hackable_map']).read_text()), include_hidden_objects=inactive_names, map_settings=map_settings)
    # Final map and palette references select the same canonical local models.
    catalog_entries = {entry['id']: entry for entry in json.loads((output/'assets/index.json').read_text())['assets']}
    for row in report.get('static_variants', []):
        descriptor_path = output/'assets'/catalog_entries[row['asset_id']]['descriptor']
        descriptor=json.loads(descriptor_path.read_text())
        variant=(descriptor.get('state_variants') or descriptor['standalone_variants'])[row['state']]
        row.update(model=str(descriptor_path.parent/variant['model']),
                   model_sha256=hashlib.sha256((descriptor_path.parent/variant['model']).read_bytes()).hexdigest(),
                   model_scene=variant['model_scene'], canonical_model=True)
    (output/'stage.json').write_text(json.dumps(report,indent=2)+'\n')
    collection = bpy.data.collections[plan['collection_name']]
    visibility = {obj: obj.hide_render for obj in collection.all_objects}
    try:
        for obj in list(collection.all_objects):
            if obj.get('reveal_show_when_applied'):
                obj.hide_render = True
        render_views(plan['scene_name'],{'reference':plan['reference_camera']},output/'full-map',width=1920)
    finally:
        for obj, hidden in visibility.items():
            obj.hide_render = hidden
    # Lossy models bind exact model bytes, so they are derived last, on the catalog promotion
    # installs (the packed map catalog when the map export produced one). This resets Blender's
    # file; the worker was saved above.
    from lossy_assets import refresh_derivatives
    catalog_root = output/'map-assets/3d-assets' if (output/'map-assets/3d-assets/index.json').exists() else output/'assets'
    enabled = lossy and plan.get('lossy', True)
    report['lossy'] = refresh_derivatives(catalog_root, output/'lossy-work', lossy=enabled, previews=enabled)
    report['lossy']['catalog'] = str(catalog_root)
    (output/'stage.json').write_text(json.dumps(report,indent=2)+'\n')
    return report


if __name__=='__main__':
    arguments=sys.argv[sys.argv.index('--')+1:]
    if any(value not in ('--lossy','--no-lossy') for value in arguments[1:]):
        raise SystemExit('usage: stage_reviewed_publication.py -- PLAN [--lossy|--no-lossy]')
    report=stage(arguments[0], lossy='--no-lossy' not in arguments[1:])
    print(json.dumps({'map':report['map'],'assets':report['assets'],'imports':len(report['imports'])}),flush=True)
