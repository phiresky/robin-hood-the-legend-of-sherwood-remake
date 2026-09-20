"""Close the gate tower roofs while retaining stable roof-sector ownership."""
import math
import bpy
import bmesh
from mathutils import Matrix, Vector

TAG = "south-gate-roof-sectors-v1"


def refine():
    working = bpy.data.collections["Derby Working"]
    existing = [o for o in working.objects if o.get("south_gate_roof") == TAG and not o.hide_render]
    if existing:
        if len(existing) != 10:
            raise RuntimeError("Incomplete south gate roof refinement")
        return {"reused": True, "pieces": len(existing), "profile": refine_profile()}
    bpy.context.view_layer.update()
    report = []
    for ids in (range(13, 17), range(17, 23)):
        sources = []
        for i in ids:
            matches = [o for o in working.objects if o.get("source_node") == f"building-{i:03d}"
                       and o.type == "MESH" and not o.hide_render]
            if len(matches) != 1:
                raise RuntimeError(f"Ambiguous roof source {i}")
            sources.append(matches[0])
        # Each reconstruction ends with its actual sloping roof triangle;
        # preceding polygons are vertical collision curtains, not roof surfaces.
        triangles = []
        for source in sources:
            polygon = source.data.polygons[-1]
            points = [source.matrix_world @ source.data.vertices[i].co for i in polygon.vertices]
            if len(points) != 3 or max(p.z for p in points) < 350:
                raise RuntimeError("Roof anchor topology changed")
            triangles.append(points)
        apex = sum((max(t, key=lambda p: p.z) for t in triangles), Vector()) / len(triangles)
        endpoints = [p for t in triangles for p in t if p.z < 300]
        body_id = "building-004" if ids.start == 13 else "building-006"
        body = next(o for o in working.objects if o.get("source_node") == body_id
                    and o.type == "MESH" and not o.hide_render)
        # Hidden eaves are evidenced by the top contour of the supporting tower.
        # Keep the observed roof contour where it exists; add only missing corners.
        for vertex in body.data.vertices:
            point = body.matrix_world @ vertex.co
            if point.z > 260 and all((point-other).length > 4 for other in endpoints):
                endpoints.append(point)
        clusters = []
        for point in endpoints:
            match = next((c for c in clusters if (point - c[0]).length < 4), None)
            if match is None:
                clusters.append([point])
            else:
                match.append(point)
        ring = [sum(c, Vector()) / len(c) for c in clusters]
        ring.sort(key=lambda p: math.atan2(p.y-apex.y, p.x-apex.x))
        center = Vector((apex.x, apex.y, min(p.z for p in ring)))
        sectors = {o: [] for o in sources}
        for a, b in zip(ring, ring[1:]+ring[:1]):
            midpoint = (a+b)/2
            owner = min(range(len(sources)), key=lambda j:
                        ((sum((p for p in triangles[j] if p.z < 300), Vector())/2)-midpoint).length)
            sectors[sources[owner]].append((a,b))
        for source, triangle in zip(sources, triangles):
            vertices, faces = [], []
            for a,b in sectors[source]:
                offset = len(vertices)
                vertices.extend((apex,a,b,center))
                faces.extend(tuple(offset+i for i in face) for face in
                             ((0,1,2),(0,3,1),(0,2,3),(1,3,2)))
            mesh = bpy.data.meshes.new(source.name + " / closed roof mesh")
            mesh.from_pydata(vertices, [], faces)
            bm = bmesh.new(); bm.from_mesh(mesh)
            bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
            bad = sum(not e.is_manifold for e in bm.edges)
            degenerate = sum(f.calc_area() < 1e-7 for f in bm.faces)
            bm.to_mesh(mesh); bm.free()
            if bad or degenerate:
                raise RuntimeError("Invalid closed gate roof sector")
            polygon = source.data.polygons[-1]
            # Extend each original layer independently, including atlas fallback
            # and reference projection. Final visibility reprojection runs later.
            axis = max(range(3), key=lambda i: abs((triangle[1]-triangle[0]).cross(triangle[2]-triangle[0])[i]))
            axes = [i for i in range(3) if i != axis]
            inverse = Matrix([(p[axes[0]],p[axes[1]],1) for p in triangle]).transposed().inverted()
            original_midpoint=sum((p for p in triangle if p.z < 300),Vector())/2
            def projected_area(points):
                sine, cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
                projected=[Vector((p.x,-p.y*sine-p.z*cosine)) for p in points]
                return abs((projected[1]-projected[0]).cross(projected[2]-projected[0]))
            donor_index=max(range(len(sources)),key=lambda i:projected_area(triangles[i]))
            donor=sources[donor_index]
            donor_polygon=donor.data.polygons[-1]
            donor_points=triangles[donor_index]
            for old in source.data.uv_layers:
                uv = mesh.uv_layers.new(name=old.name)
                anchors = [old.data[i].uv.copy() for i in polygon.loop_indices]
                donor_uv=donor.data.uv_layers.get(old.name)
                donor_anchors=[donor_uv.data[i].uv.copy() for i in donor_polygon.loop_indices]
                top_index=max(range(3),key=lambda i:donor_points[i].z)
                sides=[i for i in range(3) if i != top_index]
                mapped=(donor_anchors[top_index],donor_anchors[sides[0]],donor_anchors[sides[1]],
                        (donor_anchors[sides[0]]+donor_anchors[sides[1]])/2)
                for loop in mesh.loops:
                    a,b=sectors[source][loop.vertex_index//4]
                    if ((a+b)/2-original_midpoint).length < 4:
                        p=mesh.vertices[loop.vertex_index].co
                        weights=inverse @ Vector((p[axes[0]],p[axes[1]],1))
                        uv.data[loop.index].uv=sum((anchors[i]*weights[i] for i in range(3)),Vector((0,0)))
                    else:
                        uv.data[loop.index].uv=mapped[loop.vertex_index%4]
            for material in source.data.materials: mesh.materials.append(material)
            covered=next((image for image in bpy.data.images if image.filepath.endswith("covered.png")),None)
            if covered is None:
                raise RuntimeError("Covered projection image must be loaded before roof refinement")
            material=bpy.data.materials.get("South gate roof concealed shingles")
            if material is None:
                material=bpy.data.materials.new("South gate roof concealed shingles")
                material.use_nodes=True
                nodes=material.node_tree.nodes;nodes.clear()
                output=nodes.new("ShaderNodeOutputMaterial")
                shader=nodes.new("ShaderNodeEmission")
                texture=nodes.new("ShaderNodeTexImage");texture.image=covered
                uvnode=nodes.new("ShaderNodeUVMap");uvnode.uv_map="South gate roof donor"
                links=material.node_tree.links
                links.new(uvnode.outputs["UV"],texture.inputs["Vector"])
                links.new(texture.outputs["Color"],shader.inputs["Color"])
                links.new(shader.outputs[0],output.inputs["Surface"])
            fallback_index=len(mesh.materials);mesh.materials.append(material)
            donor_layer=mesh.uv_layers.new(name="South gate roof donor")
            sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
            direction=Vector((0,-cosine,sine))
            donor_apex=max(donor_points,key=lambda p:p.z)
            donor_eaves=[p for p in donor_points if p.z<300]
            donor_positions=(donor_apex,*donor_eaves,(donor_eaves[0]+donor_eaves[1])/2)
            for face in mesh.polygons:
                for loop_id in face.loop_indices:
                    vertex_id=mesh.loops[loop_id].vertex_index
                    point=(mesh.vertices[vertex_id].co if face.index%4==0 and face.normal.dot(direction)>0.25
                           else donor_positions[vertex_id%4])
                    donor_layer.data[loop_id].uv=(point.x/1920,1-(-point.y*sine-point.z*cosine)/2752)
            fallback=mesh.attributes.new("reprojection_fallback_material","INT","FACE")
            for face in mesh.polygons:
                face.material_index=polygon.material_index
                fallback.data[face.index].value=fallback_index
            obj=bpy.data.objects.new(source.name+" / closed roof",mesh)
            working.objects.link(obj); obj.parent=source.parent; obj.matrix_world=Matrix.Identity(4)
            for key in source.keys():obj[key]=source[key]
            obj["south_gate_roof"]=TAG
            obj["projection_min_cosine"]=0.25
            obj["todo"]="Rear roof artwork is inferred from adjacent roof sectors; no independent rear reference exists."
            source.hide_render=True;source.hide_set(True);source["south_gate_roof_baseline"]=TAG
            report.append({"source_node":obj["source_node"],"sectors":len(sectors[source]),
                           "nonmanifold_edges":bad,"degenerate_faces":degenerate})
    bpy.context.view_layer.update()
    return {"reused":False,"pieces":report,"profile":refine_profile()}


# Traced flank samples in the unscaled 1920x2752 covered source image.
# Crown finials are excluded from the flank fit; their height is fitted jointly.
PROFILE_SAMPLES = {
    "west": [(2160,745,761),(2170,741,767),(2180,736,774),
             (2190,731,781),(2200,722,788),(2210,715,797),(2220,707,805)],
    "east": [(2120,937,953),(2130,928,958),(2140,923,965),
             (2150,917,973),(2160,910,980),(2170,903,986),(2180,894,992)],
}


def refine_profile():
    """Fit curved taper to measured source flanks, keeping closed sector ownership."""
    working=bpy.data.collections["Derby Working"]
    profile_tag="south-gate-curved-roofs-v1"
    current=[o for o in working.objects if o.get("south_gate_roof")==TAG and not o.hide_render]
    if len(current)!=10:raise RuntimeError("Expected all ten gate roof sectors")
    if all(o.get("south_gate_profile")==profile_tag for o in current):
        return {"reused":True,"pieces":10}
    if any(o.get("south_gate_profile") for o in current):raise RuntimeError("Mixed gate roof profile versions")
    bpy.context.view_layer.update()
    sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
    project=lambda p:(p.x,-p.y*sine-p.z*cosine)
    report=[]
    for tower, ids in (("east",range(13,17)),("west",range(17,23))):
        sources=[o for o in current if int(o["source_node"].split('-')[-1]) in ids]
        sectors=[]
        for source in sources:
            world=[source.matrix_world@v.co for v in source.data.vertices]
            if len(world)%4:raise RuntimeError("Expected audited tetrahedral roof sectors")
            sectors.extend((source,i,world[i],world[i+1],world[i+2],world[i+3]) for i in range(0,len(world),4))

        constrain=False
        cap_samples={"east":[(2088,944,946),(2100,941,947),(2110,938,949)],
                     "west":[(2128,753,755),(2140,751,757),(2150,748,759)]}[tower]
        silhouette=cap_samples+PROFILE_SAMPLES[tower]

        def point(apex,eave,t,power,lift):
            radius=(1-t)**power
            result=Vector((apex.x+(eave.x-apex.x)*radius,
                           apex.y+(eave.y-apex.y)*radius,
                           eave.z+(apex.z+lift-eave.z)*t))
            if constrain:
                _,y=project(result)
                for low,high in zip(silhouette,silhouette[1:]):
                    if low[0]<=y<=high[0]:
                        amount=(y-low[0])/(high[0]-low[0])
                        left=low[1]+(high[1]-low[1])*amount
                        right=low[2]+(high[2]-low[2])*amount
                        result.x=min(right,max(left,result.x))
                        break
            return result

        def error(power,lift,return_errors=False):
            segments=[]
            for _,_,apex,a,b,_ in sectors:
                for eave in (a,b):
                    curve=[project(point(apex,eave,i/24,power,lift)) for i in range(25)]
                    segments.extend(zip(curve,curve[1:]))
                segments.append((project(point(apex,a,0,power,lift)),project(point(apex,b,0,power,lift))))
            errors=[]
            for y,left,right in PROFILE_SAMPLES[tower]:
                xs=[a[0]+(b[0]-a[0])*(y-a[1])/(b[1]-a[1]) for a,b in segments
                    if min(a[1],b[1])<=y<=max(a[1],b[1]) and abs(b[1]-a[1])>1e-8]
                if not xs:return 1e6
                errors.extend((min(xs)-left,max(xs)-right))
            return errors if return_errors else sum(e*e for e in errors)/len(errors)

        # Anchor the observed finial tip separately so fitting the broad roof
        # flanks cannot trade an excessively tall needle for a stronger taper.
        tip_y={"east":2088.0,"west":2128.0}[tower]
        lift=max(0,(project(sectors[0][2])[1]-tip_y)/cosine)
        power=min((1+i*.02 for i in range(81)),key=lambda p:error(p,lift))
        before=error(1,0)**.5
        baseline_errors=error(1,0,True)
        unconstrained=error(power,lift)**.5
        constrain=True
        after=error(power,lift)**.5
        fitted_errors=error(power,lift,True)
        overshoot=lambda values:max([0]+[-value if i%2==0 else value for i,value in enumerate(values)])
        for source in sources:
            vertices,faces,weights=[],[],[]
            # Curve rings are closely spaced near the flared eave.
            levels=(0,.06,.12,.20,.30,.42,.55,.68,.80,.90,.96)
            for _,offset,apex,a,b,center in [s for s in sectors if s[0]==source]:
                start=len(vertices)
                for t in levels:
                    for side,eave in enumerate((a,b)):
                        vertices.append(point(apex,eave,t,power,lift))
                        weights.append((offset,(t,(1-t)*(1-side),(1-t)*side,0)))
                top=len(vertices);vertices.append(point(apex,a,1,power,lift));weights.append((offset,(1,0,0,0)))
                bottom=len(vertices);vertices.append(center);weights.append((offset,(0,0,0,1)))
                for i in range(len(levels)-1):faces.append((start+2*i,start+2*i+1,start+2*i+3,start+2*i+2))
                faces.extend(((start+2*(len(levels)-1),start+2*(len(levels)-1)+1,top),
                              tuple([bottom]+[start+2*i for i in range(len(levels))]+[top]),
                              tuple([bottom,top]+[start+2*i+1 for i in reversed(range(len(levels)))]),
                              (bottom,start+1,start)))
            mesh=bpy.data.meshes.new(source.name+" / curved profile mesh");mesh.from_pydata(vertices,[],faces)
            bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
            bad=sum(not e.is_manifold for e in bm.edges);degenerate=sum(f.calc_area()<1e-7 for f in bm.faces)
            bm.to_mesh(mesh);bm.free()
            if bad or degenerate:raise RuntimeError("Curved gate roof is not closed")
            for old in source.data.uv_layers:
                uv=mesh.uv_layers.new(name=old.name)
                original={loop.vertex_index:old.data[loop.index].uv.copy() for loop in source.data.loops}
                for polygon in mesh.polygons:
                    for loop_id in polygon.loop_indices:
                        vertex=mesh.loops[loop_id].vertex_index;offset,w=weights[vertex]
                        uv.data[loop_id].uv=sum((original[offset+i]*w[i] for i in range(4)),Vector((0,0)))
                        if old.name=="South gate roof donor" and polygon.normal.dot(Vector((0,-cosine,sine)))>.25:
                            x,y=project(mesh.vertices[vertex].co);uv.data[loop_id].uv=(x/1920,1-y/2752)
            for material in source.data.materials:mesh.materials.append(material)
            fallback_index=source.data.attributes["reprojection_fallback_material"].data[0].value
            fallback=mesh.attributes.new("reprojection_fallback_material","INT","FACE")
            for polygon in mesh.polygons:
                polygon.material_index=fallback_index;fallback.data[polygon.index].value=fallback_index
            obj=bpy.data.objects.new(source.name+" / curved taper",mesh);working.objects.link(obj)
            obj.parent=source.parent;obj.matrix_world=Matrix.Identity(4)
            for key in source.keys():obj[key]=source[key]
            obj["south_gate_profile"]=profile_tag;obj["roof_profile_power"]=power;obj["roof_apex_lift"]=lift
            source.hide_render=True;source.hide_set(True);source["south_gate_profile_baseline"]=profile_tag
        report.append({"tower":tower,"power":power,"apex_lift":lift,"source_finial_tip_y":tip_y,
                       "traced_flank_rmse_before_px":before,"traced_flank_rmse_after_px":after,
                       "uniform_profile_rmse_px":unconstrained,
                       "max_traced_overshoot_before_px":overshoot(baseline_errors),
                       "max_traced_overshoot_after_px":overshoot(fitted_errors),
                       "silhouette_constraint":"Roof vertices contained within traced source flanks; cap and eave ownership retained.",
                       "samples":PROFILE_SAMPLES[tower],"nonmanifold_edges":0,"degenerate_faces":0})
    bpy.context.view_layer.update()
    return {"reused":False,"towers":report}

