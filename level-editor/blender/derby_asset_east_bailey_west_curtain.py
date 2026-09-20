"""Source-audited crenellations for the east bailey's western curtain.

Walk platforms and their masonry buttresses remain separate canonical parts.
The descending parapet follows its measured slope; its slots are measured in
reference-image coordinates. Refresh layered projection after integration.
"""
from pathlib import Path
import runpy

import bpy
from mathutils import Matrix, Vector

TAG = 'east-bailey-west-curtain-v1'


def _repair_fallback(obj, source, project):
    """Prefer nearby matching walls when several parallel atlas donors exist."""
    points=[source.matrix_world@v.co for v in source.data.vertices]
    normal_matrix=source.matrix_world.to_3x3().inverted().transposed()
    donors=[]
    for face in source.data.polygons:
        if len(face.vertices)!=3:continue
        matrix=Matrix([project(points[i]) for i in face.vertices]).transposed()
        if abs(matrix.determinant())<1e-7:continue
        donors.append(((normal_matrix@face.normal).normalized(),matrix.inverted(),
                       [source.data.uv_layers[0].data[i].uv.copy() for i in face.loop_indices],
                       sum((points[i] for i in face.vertices),Vector())/3))
    uv=obj.data.uv_layers[0]
    top=max(p.z for p in points)
    for face in obj.data.polygons:
        alignment=max(d[0].dot(face.normal) for d in donors)
        candidates=[d for d in donors if d[0].dot(face.normal)>alignment-.025]
        cut=min(obj.data.vertices[i].co.z for i in face.vertices)>top-27
        for li in face.loop_indices:
            point=obj.data.vertices[obj.data.loops[li].vertex_index].co.copy()
            if cut:
                point=face.center+(point-face.center)*.12
                point.z=top-55
                candidates=[d for d in donors if abs(d[0].z)<.2
                            and d[0].dot(Vector((0,-.819,.574)))>.15]
            donor=min(candidates,key=lambda d:(d[3]-point).length_squared)
            normal,inverse,coords,center=donor
            weights=inverse@project(point)
            if cut:
                weights=Vector(tuple(max(.03,min(.94,w)) for w in weights))
                weights/=sum(weights)
            value=sum((coords[k]*weights[k] for k in range(3)),Vector((0,0)))
            # Keep extrapolated fallback inside this donor's atlas rectangle.
            # Final visible faces receive direct map UVs in the projection pass.
            uv.data[li].uv=(max(min(v.x for v in coords),min(max(v.x for v in coords),value.x)),
                            max(min(v.y for v in coords),min(max(v.y for v in coords),value.y)))


def refine():
    working = bpy.data.collections['Derby Working']
    existing = [o for o in working.objects if o.get('east_bailey_west_refinement') == TAG]
    if existing:
        if len(existing) != 2:
            raise RuntimeError('Incomplete east bailey western curtain refinement')
        return {'reused': True, 'objects': [o.name for o in existing]}
    bpy.context.view_layer.update()
    directory = Path(__file__).resolve().parent
    helpers = runpy.run_path(str(directory / 'derby_asset_lower_west_curtain.py'))
    architecture = runpy.run_path(str(directory / 'derby_architecture.py'))
    sources = {}
    for node in ('building-093', 'building-094'):
        matches = [o for o in working.objects if o.type == 'MESH'
                   and o.get('source_node') == node and not o.hide_render]
        if len(matches) != 1:
            raise RuntimeError('Expected one visible baseline for ' + node)
        sources[node] = matches[0]
    source = sources['building-093']
    proxy = source.copy()
    proxy.data = source.data.copy()
    proxy.name = source.name + ' / audited parapet'
    working.objects.link(proxy)
    proxy.matrix_world = source.matrix_world.copy()
    # The generated wall and roof differ by 0.014 scene units; normalize only
    # these coincident top vertices so the roof defines one closed footprint.
    world = proxy.matrix_world
    inverse = world.inverted()
    top = max((world @ v.co).z for v in proxy.data.vertices)
    for vertex in proxy.data.vertices:
        point = world @ vertex.co
        if abs(point.z - top) < .1:
            point.z = top
            vertex.co = inverse @ point
    cuts = [
        (83,82,0,((977,983),(994,1001),(1012,1018),(1030,1036),(1047,1053))),
        (82,92,0,((1069,1073),(1078,1082),(1087,1091),(1096,1100),(1105,1109))),
        (96,92,0,((1095,1106),)),
        (99,96,0,((1074,1081),)),
        (99,95,0,((1074,1080),)),
        (95,91,0,((1097,1108),)),
        (91,90,0,((1134,1145),)),
        (90,89,0,((1167,1176),)),
    ]
    report = helpers['_wall'](proxy, cuts)
    obj = bpy.data.objects[report['object']]
    _repair_fallback(obj,source,helpers['_project'])
    obj['projection_min_cosine'] = .02
    obj['east_bailey_west_refinement'] = TAG
    del obj['lower_west_curtain_refinement']
    source['east_bailey_west_baseline'] = TAG
    source.hide_render = True
    source.hide_set(True)
    proxy_mesh = proxy.data
    bpy.data.objects.remove(proxy, do_unlink=True)
    bpy.data.meshes.remove(proxy_mesh)
    source = sources['building-094']
    points = [source.matrix_world @ v.co for v in source.data.vertices]
    project = helpers['_project']
    a, b = points[16], points[19]
    qa, qb = project(a), project(b)
    gaps = []
    for y1, y2 in ((1427,1435),(1447,1455),(1465,1471)):
        interval = sorted(a.x+(b.x-a.x)*(y-qa.y)/(qb.y-qa.y) for y in (y1,y2))
        gaps.append(tuple(interval))
    gaps.sort()
    previous_active = source.data.uv_layers.active_index
    source.data.uv_layers.active_index = 0
    slope_report = architecture['crenellate'](source, (16,19,17,18), gaps, 26)
    source.data.uv_layers.active_index = previous_active
    slope = bpy.data.objects[slope_report['object']]
    for key in source.keys():
        if key != 'architecture_refined':
            slope[key] = source[key]
    slope['east_bailey_west_refinement'] = TAG
    slope.data.uv_layers[0].name = source.data.uv_layers[0].name
    slope.data.attributes.new('reprojection_fallback_material', 'INT', 'FACE')
    return {'objects': [report, slope_report],
            'notches': report['notches']+slope_report['notches'],
            'retained_parts': ['building-%03d' % n for n in list(range(86,93))+list(range(102,106))+[121]],
            'remaining': 'Arrow slits and small masonry relief remain texture-only; concealed facade artwork is unavailable.'}
