"""Split the west curtain into named wall-walk, stair and turret assemblies.

Only grouping and names change. Canonical parts, mesh datablocks, materials,
visibility and world transforms remain intact. A separate reviewed mapping is
provided so the same partition can be applied to the editor's asset catalog.
"""
import json
from pathlib import Path

import bpy

EXPECTED = {f'building-{i:03d}' for i in (23,25,27,28,29,30,31,32,36,37,38,42,45)}


def apply_mapping(mapping_path):
    from group_assets import _renamed_part

    mapping=json.loads(Path(mapping_path).read_text())
    assignments={}
    for group in mapping['groups']:
        for part in group['parts']:
            node=f"building-{part['obstacle']:03d}"
            if node in assignments:
                raise ValueError(f'Duplicate split ownership for {node}')
            assignments[node]=(group,part)
    if set(assignments)!=EXPECTED:
        raise ValueError('Split must preserve all 13 canonical source parts exactly')
    collection=bpy.data.collections['Derby Working']
    objects=list(collection.all_objects)
    meshes=[o for o in objects if o.type=='MESH' and o.get('source_node') in EXPECTED]
    if {o.get('source_node') for o in meshes}!=EXPECTED:
        raise ValueError('Scene lacks expected curtain parts')
    roots={o.get('asset_group'):o for o in objects if o.type=='EMPTY' and o.get('asset_group')}
    original=roots[mapping['replaces_asset_id']]
    allowed_groups={g['id'] for g in mapping['groups']}
    if any(o.get('asset_group') not in allowed_groups for o in meshes):
        raise ValueError('Owned curtain components have unexpected external ownership')
    saved={o:(o.data,o.matrix_world.copy(),o.hide_render,o.hide_viewport,o.get('source_node')) for o in meshes}
    outside={o:(o.parent,o.matrix_world.copy(),o.name,o.get('asset_group'),o.data if o.type=='MESH' else None)
             for o in objects if o not in meshes and o is not original and o.get('asset_group') not in allowed_groups}
    created=[]
    for group in mapping['groups']:
        if group['id'] not in roots:
            obj=bpy.data.objects.new(group['name'],None)
            collection.objects.link(obj)
            obj.parent=original.parent
            obj.matrix_world=original.matrix_world.copy()
            obj.empty_display_type='PLAIN_AXES'
            obj.empty_display_size=15
            roots[group['id']]=obj
            created.append(group['id'])
        parent=roots[group['id']]
        parent.name=group['name']
        parent['asset_group']=group['id']
        parent['asset_name']=group['name']
    bpy.context.view_layer.update()
    for obj in meshes:
        group,part=assignments[obj['source_node']]
        name=_renamed_part(obj,group,part)
        obj.parent=roots[group['id']]
        obj.matrix_world=saved[obj][1]
        obj.name=name
        obj['asset_group']=group['id']
        obj['asset_name']=group['name']
        obj['part_name']=part['name']
    bpy.context.view_layer.update()
    drift=max(abs(o.matrix_world[r][c]-state[1][r][c]) for o,state in saved.items() for r in range(4) for c in range(4))
    if drift>1e-5:
        raise AssertionError(f'Grouping moved geometry: {drift}')
    for obj,state in saved.items():
        if obj.data!=state[0] or (obj.hide_render,obj.hide_viewport,obj.get('source_node'))!=state[2:]:
            raise AssertionError('Grouping changed mesh, visibility or canonical identity')
    for obj,state in outside.items():
        if (obj.parent,obj.matrix_world,obj.name,obj.get('asset_group'),obj.data if obj.type=='MESH' else None)!=state:
            raise AssertionError(f'Grouping changed outside object {obj.name}')
    return {'canonical_parts':len(assignments),'mesh_components':len(meshes),
            'created_groups':created,'groups':[g['id'] for g in mapping['groups']],
            'max_transform_drift':drift,'mesh_datablocks_unchanged':True,
            'outside_objects_unchanged':len(outside),'status':'PASS'}


def write_component_models(mapping_path, output_dir):
    """Write standalone scenes, reopening the saved grouped master each time.

    This deliberately avoids library-writing newly copied scenes: some Blender
    versions crash when traversing their shared render/material dependencies.
    The caller must first save the grouped master. Only disposable in-memory
    copies are pruned; the saved master and original worker remain untouched.
    """
    mapping=json.loads(Path(mapping_path).read_text())
    output=Path(output_dir)
    output.mkdir(parents=True,exist_ok=True)
    records=[]
    master=Path(bpy.data.filepath).resolve()
    if not master.is_file():
        raise ValueError('Save the grouped master before exporting components')
    for group in mapping['groups']:
        bpy.ops.wm.open_mainfile(filepath=str(master))
        collection=bpy.data.collections['Derby Working']
        originals=[o for o in collection.all_objects if o.type=='MESH' and o.get('asset_group')==group['id']]
        parents=[o for o in collection.all_objects if o.type=='EMPTY' and o.get('asset_group')==group['id']]
        if len(parents)!=1:
            raise ValueError('Expected one logical component root')
        parent=parents[0]
        before={o.name:o.matrix_world.copy() for o in originals}
        matrix=parent.matrix_world.copy()
        parent.parent=None
        parent.matrix_world=matrix
        bpy.context.view_layer.update()
        for original in originals:original.matrix_world=before[original.name]
        keep=set(originals+[parent])
        for obj in list(bpy.data.objects):
            if obj not in keep:bpy.data.objects.remove(obj,do_unlink=True)
        bpy.context.view_layer.update()
        drift=max(abs(o.matrix_world[r][c]-before[o.name][r][c])
                  for o in originals for r in range(4) for c in range(4))
        if drift>1e-5:
            raise AssertionError(f'Component isolation moved geometry: {drift}')
        path=output/(group['id']+'.blend')
        bpy.data.orphans_purge(do_local_ids=True,do_linked_ids=True,do_recursive=True)
        bpy.ops.wm.save_as_mainfile(filepath=str(path),compress=True)
        records.append({'asset_id':group['id'],'path':str(path),'mesh_components':len(originals),'max_transform_drift':drift,
                        'source_nodes':sorted({o.get('source_node') for o in originals})})
    bpy.ops.wm.open_mainfile(filepath=str(master))
    return records
