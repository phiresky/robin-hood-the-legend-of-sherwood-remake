"""Rebuild the cottage roof with level eaves and an authored silhouette fit.

Run on its isolated refinement workspace, then regenerate the modified packet.
The earlier variable-height eaves bent the roof along its entire length.
"""
import math
import bpy
import bmesh
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from derby_asset_lower_west_cottage import _roof_shell, _solid, _world

TAG = 'round2-level-eaves-mask-fit'
SIN, COS = math.sin(math.radians(35)), math.cos(math.radians(35))
RIDGE, EAVE = 121.53, 70.0
# Main roof outline read against the authored main-cottage occlusion bitmap.
REAR = {
    'building-055': [(623,1845),(629,1855),(635,1861),(641,1865),
                     (647,1869),(653,1873),(659,1877),(665,1881),(670,1886)],
    'building-056': [(623,1845),(617.5,1854),(612,1859),(606.5,1863),
                     (601,1866),(595.5,1869),(590,1872),(584.5,1875),(579,1880)],
}


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
        vertices = []
        for row in range(17):
            for col in range(9):
                t = col/8
                z = RIDGE*(1-t)+EAVE*t + 1.4*math.sin(math.pi*t)
                pixel = apex.lerp(corner,t)
                front = point(pixel.x,pixel.y,z)
                rear = point(*REAR[node][col],z)
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
        # The straight foundation is inset from the eaves, while the back gable
        # and short front hip meet the actual underside rather than stretching.
        outline = [lip, vertices[8], vertices[16*9+8], vertices[16*9], vertices[0]]
        center = sum(outline,Vector())/len(outline)
        # Set the straight rear wall behind the scalloped thatch overhang. A
        # chord drawn directly across its outer edge escapes the curved shell.
        rear_outer = vertices[16*9+8].lerp(center,.18)
        rear_outer += (vertices[8]-vertices[16*9+8])*.08
        rear_center = vertices[0].lerp(vertices[16*9],.80)
        front_inset = .30 if node.endswith('055') else .18
        corners = [lip,vertices[8].lerp(center,front_inset),rear_outer,
                   rear_center,vertices[0]]
        top = []
        roof_points = [roof.matrix_world@v.co for v in mesh.vertices]
        for a,b in zip(corners,corners[1:]+corners[:1]):
            cuts = {step/24 for step in range(24)}
            direction = b-a
            for edge in mesh.edges:
                c,d = (roof_points[i] for i in edge.vertices)
                other = d-c
                determinant = direction.x*other.y-direction.y*other.x
                if abs(determinant)<1e-7:
                    continue
                offset = c-a
                t = (offset.x*other.y-offset.y*other.x)/determinant
                u = (offset.x*direction.y-offset.y*direction.x)/determinant
                if 1e-5<t<1-1e-5 and 0<=u<=1:
                    cuts.add(round(t,6))
            # Include the projected triangle boundaries, so each wall segment
            # follows one planar roof face rather than bridging across folds.
            previous = -1
            for t in sorted(cuts):
                if t-previous<1e-4:
                    continue
                previous = t
                p = a.lerp(b,t)
                # Shared centerline vertices remain coincident between halves.
                probe = p.lerp(center,.0001)
                hit = tree.ray_cast(Vector((probe.x,probe.y,1000)),Vector((0,0,-1)))[0]
                if hit is None:
                    raise ValueError('Wall escaped thatch shell')
                # Seat the wall inside the four-unit roof thickness. A shallow
                # overlap closes interpolation differences along triangulation
                # boundaries instead of leaving a fragile tangent contact.
                p.z = hit.z-1.0
                top.append(p)
        reports.append(replace(wall,_solid('Straight cottage wall shell',top)))
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
