"""Import only reviewed visible asset meshes, preserving unrelated scene state."""
import hashlib
import json
from pathlib import Path


def import_asset_geometry(blend_path, *, asset_id, object_names, collection_name, source_nodes=None):
    import bpy
    from refinement_workspace import _geometry
    from bake_reviewed_asset import _materials
    collection=bpy.data.collections[collection_name]
    targets=[o for o in collection.all_objects if o.type=='MESH' and
             o.get('asset_group')==asset_id and not o.hide_render and
             (source_nodes is None or o.get('source_node') in source_nodes)]
    roots=[o for o in collection.all_objects if o.type=='EMPTY' and o.get('asset_group')==asset_id]
    if not targets or len(roots)!=1 or not object_names or len(object_names)!=len(set(object_names)):
        raise ValueError('Expected one current asset root and unique reviewed visible meshes')
    root=roots[0]
    expected={o.get('source_node') for o in targets}
    if any(child not in targets for obj in targets for child in obj.children):
        raise ValueError('Cannot replace meshes parenting outside asset children')
    scene=bpy.context.scene
    outside={o.name:(_geometry(o),_materials(o) if o.type=='MESH' else None)
             for o in scene.objects if o not in targets}
    hidden={o.name:_geometry(o) for o in collection.all_objects if o.type=='MESH' and
            o.get('asset_group')==asset_id and o.hide_render}
    existing=set(bpy.data.objects)
    temporary=None
    keep=set()
    try:
        with bpy.data.libraries.load(str(Path(blend_path).resolve(strict=True)),link=False) as (source,destination):
            if set(object_names)-set(source.objects):
                raise ValueError('Reviewed mesh names absent from handoff')
            destination.objects=list(object_names)
        loaded=dict(zip(object_names,destination.objects))
        temporary=bpy.data.collections.new('Reviewed geometry transform validation')
        scene.collection.children.link(temporary)
        for obj in set(bpy.data.objects)-existing:
            temporary.objects.link(obj)
        bpy.context.view_layer.update()
        if any(o is None or o.type!='MESH' or o.hide_render or o.get('asset_group')!=asset_id for o in loaded.values()):
            raise ValueError('Handoff escaped visible asset ownership')
        if {o.get('source_node') for o in loaded.values()}!=expected:
            raise ValueError('Handoff changed canonical part coverage')
        for name in loaded:
            collision=bpy.data.objects.get(name)
            if collision and collision in existing and collision not in targets:
                raise ValueError('Reviewed name collides outside asset: '+name)
        world={name:o.matrix_world.copy() for name,o in loaded.items()}
        parents={name:o.parent for name,o in loaded.items()}
        inverses={name:o.matrix_parent_inverse.copy() for name,o in loaded.items()}
        basis={name:o.matrix_basis.copy() for name,o in loaded.items()}
        for obj in targets:
            bpy.data.objects.remove(obj,do_unlink=True)
        for name,obj in loaded.items():
            obj.name=name
            collection.objects.link(obj)
            old_parent=parents[name]
            obj.parent=root
            if old_parent and all(abs(old_parent.matrix_world[r][c]-root.matrix_world[r][c])<1e-7 for r in range(4) for c in range(4)):
                obj.matrix_parent_inverse=inverses[name]
                obj.matrix_basis=basis[name]
            else:
                obj.matrix_world=world[name]
            keep.add(obj)
        bpy.context.view_layer.update()
        drift=max(abs(o.matrix_world[r][c]-world[name][r][c]) for name,o in loaded.items() for r in range(4) for c in range(4))
        if drift>1e-5:
            raise ValueError('Reviewed world transform drift: '+str(drift))
    finally:
        if temporary:
            bpy.data.collections.remove(temporary)
        for obj in set(bpy.data.objects)-existing-keep:
            bpy.data.objects.remove(obj,do_unlink=True)
    bpy.context.view_layer.update()
    after={o.name:(_geometry(o),_materials(o) if o.type=='MESH' else None)
           for o in scene.objects if o not in keep}
    if outside!=after:
        raise RuntimeError('Geometry import changed outside scene state')
    if hidden!={o.name:_geometry(o) for o in collection.all_objects if o.name in hidden}:
        raise RuntimeError('Hidden retained originals changed')
    return {'asset_id':asset_id,'source_blend':str(Path(blend_path).resolve()),
            'visible_components':len(keep),'canonical_parts':sorted(expected),
            'hidden_originals_preserved':len(hidden),'outside_objects_unchanged':len(outside),
            'maximum_world_transform_drift':drift}
