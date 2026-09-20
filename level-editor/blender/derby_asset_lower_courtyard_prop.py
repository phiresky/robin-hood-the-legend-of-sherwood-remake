"""Source-traced lower bailey well, replacing its solid square proxy.

Audited source crop: covered.png pixels (765,1640)-(860,1780). The stone well
has an open shaft and an iron lifting frame. Frame anchors below are artwork
pixels, backprojected onto the well's central vertical plane.
"""
import math

import bpy
import bmesh
from mathutils import Matrix, Vector

TAG = "lower-bailey-open-well-v1"


def refine():
    working = bpy.data.collections["Derby Working"]
    old = next((o for o in working.objects if o.get("well_refinement") == TAG), None)
    if old:
        return {"reused": True, "object": old.name}
    sources = [o for o in working.objects if o.get("source_node") == "building-046"
               and o.type == "MESH" and not o.hide_render]
    if len(sources) != 1:
        raise ValueError("Expected one lower courtyard well proxy")
    source = sources[0]
    bpy.context.view_layer.update()
    corners = [source.matrix_world @ source.data.vertices[i].co for i in (16,17,18,19)]
    center = sum(corners, Vector()) / 4
    vertices, faces, slots = [], [], []

    def face(indices, slot):
        faces.append(indices); slots.append(slot)

    def ring(rings, segments, slot):
        start = len(vertices)
        for radius, height in rings:
            for i in range(segments):
                a = i * math.tau / segments
                vertices.append(Vector((center.x + radius * math.cos(a),
                                        center.y + radius * math.sin(a), height)))
        for j in range(len(rings)):
            k = (j + 1) % len(rings)
            for i in range(segments):
                n = (i + 1) % segments
                face((start+j*segments+i,start+j*segments+n,
                      start+k*segments+n,start+k*segments+i),slot)

    def rod(a, b, radius, slot, segments=6):
        direction=(b-a).normalized()
        side=direction.cross(Vector((0,0,1)))
        if side.length < .01: side=direction.cross(Vector((0,1,0)))
        side.normalize(); other=direction.cross(side).normalized()
        start=len(vertices)
        for p in (a,b):
            for i in range(segments):
                angle=i*math.tau/segments
                vertices.append(p+radius*(side*math.cos(angle)+other*math.sin(angle)))
        face(tuple(start+i for i in range(segments-1,-1,-1)),slot)
        face(tuple(start+segments+i for i in range(segments)),slot)
        for i in range(segments):
            n=(i+1)%segments
            face((start+i,start+n,start+segments+n,start+segments+i),slot)

    # Stone shaft with a thick coping and a lowered, visibly hollow centre.
    stone_profile=[(11.8,.1),(11.9,4),(12,8),(12.1,12),(12.2,17),
                   (15.2,18.5),(15.2,center.z),(9.8,center.z),
                   (9.8,5),(8.8,5),(8.8,.1)]
    ring(stone_profile,12,0)
    # Dark inner walls make the recessed well shaft readable from oblique views.
    for i in range(7*12,10*12):slots[i]=1
    # Closed dark inset disk: hides no source geometry and reads as depth.
    ring([(8.8,4.5),(8.8,5),(0.15,5),(0.15,4.5)],12,1)
    sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
    def pixel(x,y):
        return Vector((x,center.y,(-center.y*sine-y)/cosine))
    paths=[[(800,1734),(801,1716),(798,1710),(802,1708),(805,1695),(811,1687)],
           [(823,1734),(823,1701),(820,1703),(817,1692),(811,1687)],
           [(798,1710),(793,1704)],[(823,1701),(828,1692)]]
    for path in paths:
        for a,b in zip(path,path[1:]): rod(pixel(*a),pixel(*b),.52,2)
    # Pulley housing, hanging rope and the small bucket visible behind the rim.
    rod(pixel(811,1691),pixel(811,1697),1.1,2,8)
    rod(pixel(811,1697),pixel(812,1731),.19,3,6)
    bucket_center = pixel(807,1726)
    saved=center.copy()
    center.x,center.y=bucket_center.x,bucket_center.y
    base=bucket_center.z
    ring([(2.3,base),(3.1,base+5),(2.3,base+5),(1.6,base)],8,2)
    center=saved
    mesh=bpy.data.meshes.new("Lower bailey stone well and iron lifting frame")
    mesh.from_pydata(vertices,[],faces);mesh.update()
    for label,color in (("masonry",(.22,.20,.15,1)),("shaft darkness",(.012,.009,.006,1)),
                        ("weathered iron",(.018,.016,.012,1)),("hemp rope",(.30,.25,.16,1))):
        name="Lower well fallback / "+label
        material=bpy.data.materials.get(name)
        if material is None:
            material=bpy.data.materials.new(name);material.use_nodes=True
            nodes=material.node_tree.nodes;nodes.clear()
            output=nodes.new("ShaderNodeOutputMaterial");shader=nodes.new("ShaderNodeEmission")
            shader.inputs["Color"].default_value=color
            material.node_tree.links.new(shader.outputs[0],output.inputs["Surface"])
            material.diffuse_color=color
        mesh.materials.append(material)
    for polygon,slot in zip(mesh.polygons,slots):polygon.material_index=slot
    uv=mesh.uv_layers.new(name="UVMap")
    for loop in mesh.loops:
        p=mesh.vertices[loop.vertex_index].co
        uv.data[loop.index].uv=(p.x/1920,1+(p.y*sine+p.z*cosine)/2752)
    # Retain the square proxy's masonry atlas as fallback on the rebuilt shaft.
    # Its visibility can be partial at ground contact; losing every wall to a
    # single occluded corner must not turn stone into an untextured solid colour.
    source_uv=source.data.uv_layers[0]
    source_world=[source.matrix_world @ v.co for v in source.data.vertices]
    def screen(p):return Vector((p.x,-p.y*sine-p.z*cosine,1))
    backup=source.data.attributes.get("reprojection_fallback_material")
    offset=len(mesh.materials)
    for material in source.data.materials:mesh.materials.append(material)
    for polygon in mesh.polygons:
        if polygon.material_index != 0:continue
        candidates=[source.data.polygons[i] for i in (0,2,4,6,8)]
        chosen=max(candidates,key=lambda f: polygon.normal.dot(
            (source.matrix_world.to_3x3() @ f.normal).normalized()))
        mapping=Matrix([screen(source_world[i]) for i in chosen.vertices]).transposed().inverted()
        triangle=[source_uv.data[i].uv.copy() for i in chosen.loop_indices]
        index=backup.data[chosen.index].value if backup else chosen.material_index
        polygon.material_index=offset+index
        for loop_id in polygon.loop_indices:
            weights=mapping @ screen(mesh.vertices[mesh.loops[loop_id].vertex_index].co)
            uv.data[loop_id].uv=sum((triangle[i]*weights[i] for i in range(3)),Vector((0,0)))
    bm=bmesh.new();bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    validation={"nonmanifold_edges":sum(not e.is_manifold for e in bm.edges),
                "degenerate_faces":sum(f.calc_area()<1e-8 for f in bm.faces)}
    bm.to_mesh(mesh);bm.free()
    if any(validation.values()):raise ValueError(validation)
    obj=bpy.data.objects.new("Lower Bailey Well / Stone well and lifting frame",mesh)
    working.objects.link(obj);obj.parent=source.parent;obj.matrix_world=Matrix.Identity(4)
    for key in source.keys():
        if not key.startswith("reprojection_"):obj[key]=source[key]
    obj["part_name"]="Stone well and lifting frame"
    obj["asset_name"]="Lower Bailey Well"
    obj["well_refinement"]=TAG
    obj["refinement_note"]="Iron frame traced from source pixels; concealed surfaces use neutral material fallback."
    source.hide_render=source.hide_viewport=True
    source["well_refinement_baseline"]=TAG
    bpy.context.view_layer.update()
    return {"reused":False,"source_node":"building-046","object":obj.name,
            "faces":len(mesh.polygons),"validation":validation}
