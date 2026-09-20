"""Rebuild the cottage roof with level eaves and an authored silhouette fit.

Run on its isolated refinement workspace, then regenerate the modified packet.
The earlier variable-height eaves bent the roof along its entire length.
"""
import math
import bpy
import bmesh
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from derby_asset_lower_west_cottage import _roof_shell, _world

TAG = 'round2-common-battered-facades'
SIN, COS = math.sin(math.radians(35)), math.cos(math.radians(35))
RIDGE, EAVE = 121.53, 70.0


def point(x, y, z):
    return Vector((x, -(y + z * COS) / SIN, z))


def replace(obj, mesh):
    # Source projection is rebuilt afterwards; retain one valid material slot
    # only so the source-only baker can assign its own fresh atlas.
    old = obj.data
    if old.materials:
        mesh.materials.append(old.materials[0])
    mesh.uv_layers.new(name='Source projection placeholder')
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bmesh.ops.triangulate(bm, faces=list(bm.faces))
    defects = {'nonmanifold_edges':sum(not e.is_manifold for e in bm.edges),
               'degenerate_faces':sum(f.calc_area()<1e-7 for f in bm.faces)}
    if any(defects.values()):
        raise ValueError((obj.name, defects))
    bm.to_mesh(mesh)
    bm.free()
    mesh.transform(obj.matrix_world.inverted())
    obj.data = mesh
    obj['cottage_round2'] = TAG
    obj['projection_min_cosine'] = .25
    return {'object':obj.name,'source_node':obj['source_node'],
            'vertices':len(mesh.vertices),'faces':len(mesh.polygons),**defects}


def refine():
    objects = [o for o in bpy.data.objects if o.type == 'MESH'
               and o.get('asset_group') == 'derby-lower-west-cottage'
               and not o.hide_render]
    reports = []
    for node in ('building-055','building-056'):
        roof = next(o for o in objects if o.get('source_node') == node
                    and o.get('cottage_component_role') == 'thatch roof')
        wall = next(o for o in objects if o.get('source_node') == node
                    and o.get('cottage_component_role') == 'walls')
        apex = Vector((607,1919))
        corner = Vector((652,1968)) if node.endswith('055') else Vector((563,1961))
        front_ridge = point(apex.x,apex.y,RIDGE)
        rear_ridge = point(623,1845,RIDGE)
        axis = (rear_ridge-front_ridge).normalized()
        lateral = Vector((axis.y,-axis.x,0))
        half_width = 45 if node.endswith('055') else -44
        vertices = []
        for row in range(17):
            for col in range(9):
                t = col/8
                z = RIDGE*(1-t)+EAVE*t+.8*math.sin(math.pi*t)
                pixel = apex.lerp(corner,t)
                front = point(pixel.x,pixel.y,z)
                # One ordinary rear gable plane. Never independently solve the
                # depth of each rear silhouette pixel: that creates horns and
                # a concave rear edge despite a numerically level ridge.
                rear = rear_ridge+lateral*(half_width*t)
                rear.z = z
                vertices.append(front.lerp(rear,row/16))
        faces = [(r*9+c,r*9+c+1,(r+1)*9+c+1,(r+1)*9+c)
                 for r in range(16) for c in range(8)]
        lip = point(607,1965,EAVE)
        vertices.append(lip)
        faces.append(tuple([len(vertices)-1]+list(reversed(range(9)))))
        mesh = _roof_shell('Level cottage thatch shell',vertices,faces,4)
        reports.append(replace(roof,mesh))
        tree = BVHTree.FromPolygons([roof.matrix_world@v.co for v in mesh.vertices],
                                   [tuple(p.vertices) for p in mesh.polygons],epsilon=.001)
        # One shared footprint and common inward-battered end planes.
        length = (rear_ridge-front_ridge).length
        front_s, back_s = -2.0, length-5.0
        middle_s = (front_s+back_s)/2
        half_length = (back_s-front_s)/2
        width = 37.5 if node.endswith('055') else -37.5
        footprint = [(front_s,0),(front_s,width),(back_s,width),(back_s,0)]
        top, bottom = [], []
        for a,b in zip(footprint,footprint[1:]+footprint[:1]):
            for step in range(64):
                u = step/64
                s = a[0]*(1-u)+b[0]*u
                t = a[1]*(1-u)+b[1]*u
                base = front_ridge+axis*s+lateral*t
                base.z = 0
                bottom.append(base)
                height = EAVE
                for iteration in range(32):
                    inward = -(s-middle_s)/half_length*.05*height
                    p = base+axis*inward
                    probe = p+lateral*(.002 if width>0 else -.002)
                    hit = tree.ray_cast(Vector((probe.x,probe.y,1000)),Vector((0,0,-1)))[0]
                    if hit is None:
                        raise ValueError('Battered wall escaped roof')
                    target = hit.z-1
                    if abs(target-height)<.0001:
                        break
                    height = target
                p.z = target
                top.append(p)
        n = len(top)
        center_top = sum(top,Vector())/n
        center_top.z = min(p.z for p in top)-2
        center_bottom = sum(bottom,Vector())/n
        all_vertices = top+bottom+[center_top,center_bottom]
        faces=[]
        for i in range(n):
            j=(i+1)%n
            faces.extend([(i,j,2*n),(j+n,i+n,2*n+1),(i,i+n,j+n,j)])
        body=bpy.data.meshes.new('Cottage common battered facade shell')
        body.from_pydata(all_vertices,[],faces);body.update()
        reports.append(replace(wall,body))
        wall['facade_batter']=.05
        wall['front_facade_axis_position']=front_s
        wall['rear_facade_axis_position']=back_s
    annex = next(o for o in objects if o.get('source_node') == 'building-054'
                 and o.get('cottage_component_role') == 'extension roof')
    baseline = next(o for o in bpy.data.objects if o.type == 'MESH'
                    and o.get('source_node') == 'building-054'
                    and len(o.data.vertices) == 20)
    _, original = _world(baseline)
    center = sum((original[i] for i in (16,17,18,19)),Vector())/4
    corners = []
    for i in (16,19,18,17):
        p = original[i].copy()
        outward = p-center
        outward.z = 0
        corners.append(p+outward.normalized()*2)
    outline = []
    for i, corner in enumerate(corners):
        a = corner.lerp(corners[(i-1)%4],.14)
        b = corner.lerp(corners[(i+1)%4],.14)
        for step in range(7):
            t = step/6
            outline.append(a*(1-t)**2+corner*2*t*(1-t)+b*t*t)
    reports.append(replace(annex,_roof_shell('Rounded annex thatch eaves',
        outline,[tuple(range(len(outline)))],4)))
    bpy.context.view_layer.update()
    return {'version':TAG,'parts':reports,'ridge_height':RIDGE,'eave_height':EAVE,
            'mask_reference':10,'remaining':['Hidden walls lack observed details.',
                'Fine thatch fringe remains a texture detail, not individual strands.']}
