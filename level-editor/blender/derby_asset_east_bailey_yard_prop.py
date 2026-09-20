"""Replace obstacle 113's box with its painted chopping block and embedded axe."""
import math
import bpy
import bmesh
from mathutils import Matrix, Vector

ASSET = "derby-east-bailey-yard-prop"


def _plain(name, color):
    material = bpy.data.materials.get(name)
    if material:
        return material
    material = bpy.data.materials.new(name)
    material.use_nodes = True
    nodes = material.node_tree.nodes
    nodes.clear()
    emission = nodes.new("ShaderNodeEmission")
    emission.inputs[0].default_value = (*color, 1)
    output = nodes.new("ShaderNodeOutputMaterial")
    material.node_tree.links.new(emission.outputs[0], output.inputs[0])
    return material


def _component(source, label, role, vertices, faces, material):
    mesh = bpy.data.meshes.new(label)
    mesh.from_pydata(vertices, [], faces)
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    # The axe intersects the stump and handle. Local projection cells keep
    # that contact from forcing an entire otherwise visible surface to fallback.
    bmesh.ops.triangulate(bm, faces=list(bm.faces))
    bmesh.ops.subdivide_edges(bm, edges=list(bm.edges), cuts=3,
                             use_grid_fill=True, smooth=0)
    bm.normal_update()
    bad = sum(not e.is_manifold for e in bm.edges)
    zero = sum(f.calc_area() < 1e-8 for f in bm.faces)
    bm.to_mesh(mesh)
    bm.free()
    if bad or zero:
        raise ValueError(f"Invalid {label} topology: {bad}, {zero}")
    obj = bpy.data.objects.new("East Bailey Chopping Block / " + label, mesh)
    bpy.data.collections["Derby Working"].objects.link(obj)
    obj.parent = source.parent
    obj.matrix_world = Matrix.Identity(4)
    for key in source.keys():
        if not key.startswith("reprojection_"):
            obj[key] = source[key]
    obj["asset_name"] = "East Bailey Chopping Block"
    obj["part_name"] = "Chopping block and axe"
    obj["yard_prop_component"] = role
    obj["projection_min_cosine"] = .15
    mesh.materials.append(material)
    mesh.uv_layers.new(name="UVMap")
    return obj, {"component": role, "nonmanifold_edges":bad,"degenerate_faces":zero}


def _round_bar(start, end, radius, segments=8):
    start, end = Vector(start), Vector(end)
    axis = (end-start).normalized()
    across = axis.cross(Vector((0,1,0))).normalized()
    side = axis.cross(across).normalized()
    vertices = [tuple(center + radius*(across*math.cos(2*math.pi*i/segments)
                                     + side*math.sin(2*math.pi*i/segments)))
                for center in (start,end) for i in range(segments)]
    faces = [tuple(range(segments)),tuple(range(segments,segments*2))]
    faces += [(i,(i+1)%segments,(i+1)%segments+segments,i+segments) for i in range(segments)]
    return vertices, faces


def refine():
    bpy.context.view_layer.update()
    working = bpy.data.collections["Derby Working"]
    source = next(o for o in working.objects if o.get("source_node")=="building-113"
                  and not o.get("yard_prop_component"))
    if source.get("yard_prop_refined"):
        return {"asset":ASSET,"skipped":"already refined"}
    # The stump is inside the measured collision footprint. The axe extends
    # beyond it in the artwork; its render geometry does not alter collision.
    center = Vector((1547.8,-2410.65,0))
    n = 16
    vertices = [tuple(center + Vector((5.35*math.cos(2*math.pi*i/n),
                                      5.65*math.sin(2*math.pi*i/n),z)))
                for z in (0,14.15) for i in range(n)]
    faces = [tuple(range(n)),tuple(range(n,n*2))]
    faces += [(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)]
    stump, stump_report = _component(source,"Round chopping block","stump",vertices,faces,source.data.materials[0])
    # Transfer the box's atlas in each side's own plane. The refreshed artwork
    # then supplies the oval sawn top and the source-facing bark.
    donors=[]
    world=source.matrix_world
    normal_matrix=world.to_3x3().inverted().transposed()
    for face in source.data.polygons:
        p=[world@source.data.vertices[i].co for i in face.vertices]
        if len(p)!=3:
            continue
        a,b=p[1]-p[0],p[2]-p[0]
        matrix=Matrix((a,b,a.cross(b).normalized())).transposed()
        if abs(matrix.determinant())<1e-6:
            continue
        donors.append(((normal_matrix@face.normal).normalized(),p[0],matrix.inverted(),
                       [source.data.uv_layers[0].data[i].uv.copy() for i in face.loop_indices]))
    uv=stump.data.uv_layers[0]
    for face in stump.data.polygons:
        normal,origin,inverse,coords=max(donors,key=lambda d:d[0].dot(face.normal))
        for loop in face.loop_indices:
            point=stump.data.vertices[stump.data.loops[loop].vertex_index].co
            bary=inverse@(point-origin)
            weights=(1-bary.x-bary.y,bary.x,bary.y)
            uv.data[loop].uv=sum((coords[i]*weights[i] for i in range(3)),Vector((0,0)))
    # Screen-traced endpoints: (1530,1358) to (1548,1366), with the
    # thickness running along depth rather than turning the handle into a card.
    sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
    def from_screen(x,y,depth=-2410.65):
        return (x,depth,(-depth*sine-y)/cosine)
    handle_vertices,handle_faces=_round_bar(from_screen(1530,1358),from_screen(1548.1,1366.3),.62)
    handle,handle_report=_component(source,"Wooden axe handle","axe-handle",handle_vertices,handle_faces,
                                   _plain("Derby chopping axe / wood fallback",(.23,.16,.045)))
    outline=[(1547.7,1363.5),(1550,1364.8),(1548.7,1368.5),
             (1548.4,1372.3),(1542.8,1372.1),(1543.6,1368.6),(1546.8,1366.4)]
    head_vertices=[from_screen(x,y,depth) for depth in (-2411.25,-2410.05) for x,y in outline]
    count=len(outline)
    head_faces=[tuple(range(count)),tuple(range(count,count*2))]
    head_faces += [(i,(i+1)%count,(i+1)%count+count,i+count) for i in range(count)]
    head,head_report=_component(source,"Embedded axe head","axe-head",head_vertices,head_faces,
                               _plain("Derby chopping axe / steel fallback",(.13,.16,.17)))
    source["yard_prop_refined"]=True
    source.hide_render=True
    source.hide_set(True)
    return {"asset":ASSET,"name":"East Bailey Chopping Block",
            "changes":[stump_report,handle_report,head_report],
            "ground_contact":0,"collision_preserved":True,
            "fallback_note":"Concealed axe surfaces retain source-matched wood and steel colors."}
