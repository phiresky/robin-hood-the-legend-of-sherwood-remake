"""Apply York's complete catalog while preserving the assembled scene surfaces."""
import hashlib
import json
import math
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/active/york'
CATALOG = ROOT / 'level-editor/refinement/catalogs/york.json'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def snapshot(obj):
    return {
        'vertices': [list(v.co) for v in obj.data.vertices],
        'polygons': [list(p.vertices) for p in obj.data.polygons],
        'uvs': {u.name:[list(p.uv) for p in u.data] for u in obj.data.uv_layers},
        'materials': [m.name if m else None for m in obj.data.materials],
        'slots': [p.material_index for p in obj.data.polygons],
        'matrix': [list(r) for r in obj.matrix_world],
    }


def surface_area(obj):
    obj.data.calc_loop_triangles()
    return sum(((obj.matrix_world @ obj.data.vertices[t.vertices[1]].co - obj.matrix_world @ obj.data.vertices[t.vertices[0]].co).cross(
        obj.matrix_world @ obj.data.vertices[t.vertices[2]].co - obj.matrix_world @ obj.data.vertices[t.vertices[0]].co)).length / 2
        for t in obj.data.loop_triangles)


def main():
    import bpy
    import bmesh
    from mathutils import Vector
    sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
    from render_slots import acquire
    acquire()
    sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
    from refinement_inventory import validate_catalog
    from group_assets import group_assets
    source = OUT / 'baseline/york-baseline.blend'
    output = OUT / 'grouped/york-grouped.blend'
    if output.exists():
        raise FileExistsError(output)
    catalog = json.loads(CATALOG.read_text())
    validation = validate_catalog(OUT / 'inventory/inventory.json', CATALOG)
    frozen_sha = sha(source)
    bpy.ops.wm.open_mainfile(filepath=str(source))
    bpy.context.window.scene = bpy.data.scenes['york Refinement']
    working = bpy.data.collections['york Working']
    before = {o:snapshot(o) for o in working.objects if o.type=='MESH'}
    partitions = []
    for spec in catalog.get('partitions', []):
        original = next(o for o in working.objects if o.get('source_node')==spec['source_node'])
        bounds = [-math.inf,*spec.get('boundaries', []),math.inf]
        area_before = surface_area(original)
        axis_x, axis_y = spec.get('axis_coefficients', [-.325,1.0])
        if not axis_y:
            raise ValueError('Partition axis must have a nonzero game-y coefficient')
        component_cuts = spec.get('component_cuts')
        if component_cuts is None:
            component_cuts = {component:[{'axis_coefficients':[axis_x,axis_y],
                'boundary':boundary,'keep_above':keep_above}
                for boundary,keep_above in [(bounds[i],True),(bounds[i+1],False)]
                if math.isfinite(boundary)] for i,component in enumerate(spec['components_ascending'])}
        components = []
        for component, cuts in component_cuts.items():
            mesh = original.data.copy()
            mesh.transform(original.matrix_world)
            bm = bmesh.new()
            bm.from_mesh(mesh)
            for cut in cuts:
                boundary,keep_above = cut['boundary'],cut['keep_above']
                axis_x,axis_y = cut['axis_coefficients']
                if not axis_y:
                    raise ValueError('Partition axis must have a nonzero game-y coefficient')
                bmesh.ops.bisect_plane(bm, geom=list(bm.verts)+list(bm.edges)+list(bm.faces),
                    dist=0.000001, plane_co=Vector((0,-boundary/(axis_y*math.sin(math.radians(35))),0)),
                    plane_no=Vector((axis_x,-axis_y*math.sin(math.radians(35)),0)),
                    clear_inner=keep_above, clear_outer=not keep_above)
            bm.to_mesh(mesh)
            bm.free()
            mesh.update()
            if not mesh.polygons:
                raise ValueError('Empty frontage component: '+component)
            obj = bpy.data.objects.new(original.name+' / '+component,mesh)
            working.objects.link(obj)
            obj['source_node']=spec['source_node']
            obj['source_obstacle']=spec['source_node']
            obj['projection_component']=component
            components.append(obj)
        area_after = sum(surface_area(o) for o in components)
        if abs(area_after-area_before)>max(.1,area_before*1e-6):
            raise ValueError(f'Partition changed surface coverage: {area_before} -> {area_after}')
        original.hide_render=True
        original.hide_viewport=True
        partitions.append({'source_node':spec['source_node'],'area_before':area_before,'area_after':area_after,
                           'relative_area_error':abs(area_after-area_before)/area_before,
                           'components':[o['projection_component'] for o in components],
                           'new_caps':False,'hidden_original_retained':True})
    output.parent.mkdir(parents=True,exist_ok=True)
    bpy.context.preferences.filepaths.save_version=0
    bpy.ops.wm.save_as_mainfile(filepath=str(output))
    result=group_assets(CATALOG)
    for obj,old in before.items():
        current=snapshot(obj)
        if current != old:
            raise ValueError('Grouping changed retained geometry, UVs, materials or transforms: '+obj.name)
    if sha(source)!=frozen_sha:
        raise ValueError('Frozen baseline changed')
    report={'status':'PASS','catalog_validation':validation,'grouping':result,'partitions':partitions,
            'unchanged_original_meshes':len(before),'baseline_sha256':frozen_sha,
            'catalog_sha256':sha(CATALOG),'grouped_sha256':sha(output),'recipe_sha256':sha(__file__)}
    (output.parent/'validation.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report))


if __name__=='__main__':
    main()
