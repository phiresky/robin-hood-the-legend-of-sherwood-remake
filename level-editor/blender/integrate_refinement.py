"""Apply reviewed asset recipes in a disposable integration scene with scope checks.

No save, reprojection, publication or implicit recipe selection occurs here.
Run on a copied scene; a failed recipe may have changed that in-memory scene.
"""
import hashlib
import importlib.util
import json
from pathlib import Path


def apply_recipe(recipe_path, *, asset_id, scene_name, collection_name,
                 function='refine', kwargs=None):
    import bpy
    from refinement_workspace import _geometry

    scene = bpy.data.scenes[scene_name]
    bpy.context.window.scene = scene
    bpy.context.view_layer.update()
    collection = bpy.data.collections[collection_name]

    def owned(obj):
        return obj.type == 'MESH' and (
            obj.get('source_node') == 'ground' if asset_id == '__terrain__'
            else obj.get('asset_group') == asset_id)

    targets = [obj for obj in collection.all_objects if owned(obj)]
    if not targets:
        raise ValueError(f'No owned meshes for {asset_id}')
    before = {obj.name: (_geometry(obj), obj.get('source_node')) for obj in targets}
    outside = {obj.name: _geometry(obj) for obj in scene.objects if not owned(obj)}
    part_ids = {obj.get('source_node') for obj in targets if not obj.hide_render}
    path = Path(recipe_path).resolve(strict=True)
    spec = importlib.util.spec_from_file_location('reviewed_asset_recipe', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    result = getattr(module, function)(**(kwargs or {}))
    bpy.context.view_layer.update()
    after_outside = {obj.name: _geometry(obj) for obj in scene.objects if not owned(obj)}
    if outside != after_outside:
        changed = sorted(name for name in set(outside) | set(after_outside)
                         if outside.get(name) != after_outside.get(name))
        raise ValueError(f'Recipe escaped {asset_id} ownership: {changed}')
    after_targets = [obj for obj in collection.all_objects if owned(obj)]
    after = {obj.name: (_geometry(obj), obj.get('source_node')) for obj in after_targets}
    after_parts = {obj.get('source_node') for obj in after_targets if not obj.hide_render}
    if after_parts != part_ids:
        raise ValueError(f'Recipe changed visible canonical parts: {part_ids ^ after_parts}')
    changed_names = {name for name in set(before) | set(after) if before.get(name) != after.get(name)}
    changed_parts = {value[1] for name in changed_names for value in (before.get(name), after.get(name)) if value}
    return {'asset_id': asset_id, 'recipe': str(path),
            'recipe_sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
            'function': function, 'kwargs': kwargs or {}, 'result': result,
            'protected_objects': len(outside), 'changed_objects': sorted(changed_names),
            'changed_parts': sorted(changed_parts), 'canonical_parts_preserved': True}


def import_asset_textures(blend_path, *, asset_id, collection_name):
    """Import UV/material data only when the saved asset has identical geometry.

    Names and transforms must match the current visible asset. The complete
    copied worker scene is never linked into the working scene.
    """
    import bpy

    targets = {o.name: o for o in bpy.data.collections[collection_name].all_objects
               if o.type == 'MESH' and not o.hide_render and o.get('asset_group') == asset_id}
    if not targets:
        raise ValueError(f'No visible target meshes for {asset_id}')

    def signature(obj):
        record = [obj.get('source_node'), obj.get('asset_group'),
                  [list(row) for row in obj.matrix_world],
                  [list(v.co) for v in obj.data.vertices],
                  [list(p.vertices) for p in obj.data.polygons]]
        return hashlib.sha256(json.dumps(record).encode()).hexdigest()

    before = {name: signature(obj) for name, obj in targets.items()}
    existing = set(bpy.data.objects)
    names = sorted(targets)
    temporary = None
    try:
        with bpy.data.libraries.load(str(Path(blend_path).resolve()), link=False) as (source, destination):
            missing = set(names) - set(source.objects)
            if missing:
                raise ValueError(f'Texture handoff lacks components: {sorted(missing)}')
            # Blender replaces entries in this list with loaded objects.
            destination.objects = list(names)
        loaded = dict(zip(names, destination.objects))
        temporary = bpy.data.collections.new('Texture handoff transform validation')
        bpy.context.scene.collection.children.link(temporary)
        for obj in set(bpy.data.objects) - existing:
            temporary.objects.link(obj)
        bpy.context.view_layer.update()
        for name, obj in loaded.items():
            if obj is None or signature(obj) != before[name]:
                raise ValueError(f'Texture handoff changed geometry or ownership: {name}')
        # Validate every component before replacing any mesh data.
        for name, obj in loaded.items():
            target = targets[name]
            target.data = obj.data
            for key in obj.keys():
                if key.startswith(('source_ownership_', 'reprojection_', 'generated_')):
                    target[key] = obj[key]
        if {name: signature(obj) for name, obj in targets.items()} != before:
            raise RuntimeError('Texture import changed target geometry')
    finally:
        if temporary is not None:
            bpy.data.collections.remove(temporary)
        for obj in set(bpy.data.objects) - existing:
            bpy.data.objects.remove(obj, do_unlink=True)
    return {'asset_id': asset_id, 'source_blend': str(Path(blend_path).resolve()),
            'components': len(targets), 'geometry_unchanged': True,
            'component_geometry': before}
