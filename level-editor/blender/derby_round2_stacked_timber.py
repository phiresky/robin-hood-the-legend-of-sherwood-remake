"""Audit timber without cutting logs hidden behind the foreground bucket.

The composite native mask includes both timber and bucket. Shortening logs to
eliminate projected bucket pixels would manufacture unsupported geometry.
"""
import math
import bpy
import bmesh
import numpy as np
from mathutils import Vector

ASSET = "derby-east-bailey-stacked-timber"
BUCKET_ROLE = "round2_timber_yard_bucket_review"


def bucket_proposal():
    """Create a review accessory, without allocating a new canonical source ID.

    The vessel is visible in front of the log ends. Its exact hidden base height
    and attachment are not recovered, so no hanging post is invented. The root
    reviewer must decide whether this becomes an independent editor asset.
    """
    working = bpy.data.collections["Derby Working"]
    existing = [o for o in working.all_objects if o.get("component_role") == BUCKET_ROLE]
    if existing:
        if len(existing) != 1:
            raise ValueError("Duplicate bucket review components")
        return {"status": "existing", "object": existing[0].name}
    sources = [o for o in working.all_objects if o.type == "MESH" and not o.hide_render
               and o.get("asset_group") == ASSET and o.get("source_node") == "building-107"]
    if len(sources) != 1:
        raise ValueError("Expected one visible source107 timber component")
    source = sources[0]
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
    center = Vector((1593.0, -1310.0 / sine, 0))
    vertices, faces = [], []

    def vessel(levels, close_section=False):
        start = len(vertices)
        count = 32
        for z, radius in levels:
            vertices.extend(center + Vector((radius*math.cos(i*math.tau/count),
                                             radius*math.sin(i*math.tau/count), z))
                            for i in range(count))
        if not close_section:
            faces.append(tuple(start+i for i in reversed(range(count))))
        for ring in range(len(levels)-1):
            base = start+ring*count
            for i in range(count):
                j = (i+1) % count
                faces.append((base+i, base+j, base+count+j, base+count+i))
        if close_section:
            last = start+(len(levels)-1)*count
            for i in range(count):
                j = (i+1) % count
                faces.append((last+i,last+j,start+j,start+i))
        else:
            faces.append(tuple(start+(len(levels)-1)*count+i for i in range(count)))

    # Solid wooden bottom, thick cylindrical wall and genuine open cavity.
    vessel([(1.0,4.7), (18.8,5.8), (20.0,5.8), (20.0,4.8), (2.5,3.9)])
    # Thin continuous raised rim; no decorative hoops inferred on unseen sides.
    vessel([(18.0,5.85), (20.1,5.95), (20.1,5.65), (18.0,5.55)], close_section=True)

    def source_point(x,y,z):
        return Vector((x, -(y+z*cosine)/sine, z))

    def beam(a,b,width):
        axis = (b-a).normalized()
        side = axis.cross(Vector((0,0,1)))
        if side.length < .001:
            side = axis.cross(Vector((0,1,0)))
        side.normalize()
        other = axis.cross(side).normalized()
        offsets = [(side*x+other*y)*width/2 for x,y in ((-1,-1),(1,-1),(1,1),(-1,1))]
        start = len(vertices)
        vertices.extend(p+offset for p in (a,b) for offset in offsets)
        faces.extend(tuple(start+i for i in f) for f in
                     ((3,2,1,0),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)))

    # Two short protrusions trace the visible hardware; no unseen support frame.
    beam(source_point(1585.5,1282.0,31.0), source_point(1590.0,1293.5,19.5), 1.3)
    beam(source_point(1599.0,1283.5,30.0), source_point(1596.0,1294.0,18.0), 1.5)
    mesh = bpy.data.meshes.new("Timber-yard bucket / review vessel and hardware")
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bmesh.ops.triangulate(bm, faces=list(bm.faces))
    defects = {"nonmanifold_edges": sum(not e.is_manifold for e in bm.edges),
               "degenerate_faces": sum(f.calc_area() < 1e-7 for f in bm.faces)}
    if any(defects.values()):
        bm.free()
        raise ValueError(defects)
    bm.to_mesh(mesh)
    bm.free()
    material = bpy.data.materials.get("Timber-yard bucket review neutral")
    if material is None:
        material = bpy.data.materials.new("Timber-yard bucket review neutral")
        material.diffuse_color = (.3,.3,.3,1)
    mesh.materials.append(material)
    mesh.uv_layers.new(name="UVMap")
    obj = bpy.data.objects.new("Timber-yard bucket and tools / ownership proposal", mesh)
    working.objects.link(obj)
    obj.parent = source.parent
    from mathutils import Matrix
    obj.matrix_world = Matrix.Identity(4)
    for key in source.keys():
        if key.startswith("reprojection_") or key in ("log_count", "timber_log_refinement"):
            continue
        obj[key] = source[key]
    obj["component_role"] = BUCKET_ROLE
    obj["projection_component"] = BUCKET_ROLE
    obj["part_name"] = "Timber-yard bucket and tools / review accessory"
    obj["proposed_asset_group"] = "derby-east-bailey-timber-bucket"
    obj["ownership_review_required"] = True
    obj["mask_evidence"] = "Composite mask30, exterior layer0; bucket and pile share mask"
    bpy.context.view_layer.update()
    return {"status": "proposal_created", "object": obj.name, "source_node": obj["source_node"],
            "proposed_asset_group": obj["proposed_asset_group"], **defects,
            "remaining": "Validate native-pixel silhouette and root grouping before publication"}


def connected_components(mesh):
    adjacent = [set() for _ in mesh.vertices]
    for edge in mesh.edges:
        a, b = edge.vertices
        adjacent[a].add(b)
        adjacent[b].add(a)
    unseen = set(range(len(adjacent)))
    groups = []
    while unseen:
        pending = [min(unseen)]
        unseen.remove(pending[0])
        group = []
        while pending:
            current = pending.pop()
            group.append(current)
            new = adjacent[current] & unseen
            unseen.difference_update(new)
            pending.extend(sorted(new))
        groups.append(sorted(group))
    return groups


def fit_axis(points):
    coordinates = np.array([tuple(point) for point in points], dtype=float)
    center = coordinates.mean(axis=0)
    _, vectors = np.linalg.eigh((coordinates-center).T @ (coordinates-center))
    axis = Vector(vectors[:, -1])
    if axis.x < 0:
        axis.negate()
    if abs(axis.z) > .001 or axis.x < .1:
        raise ValueError("Expected horizontal timber cylinder")
    return Vector(center), axis


def refine():
    """Return geometry evidence, leaving scene geometry and identities intact."""
    objects = [o for o in bpy.data.collections["Derby Working"].all_objects
               if o.type == "MESH" and not o.hide_render and o.get("asset_group") == ASSET
               and o.get("component_role") != BUCKET_ROLE]
    if len(objects) != 2 or {o.get("source_node") for o in objects} != {"building-107", "building-108"}:
        raise ValueError("Expected exactly the two owned timber meshes")
    report, cylinders = [], []
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
    for obj in sorted(objects, key=lambda o: o["source_node"]):
        world = [obj.matrix_world @ v.co for v in obj.data.vertices]
        groups = connected_components(obj.data)
        if len(groups) != int(obj.get("log_count", -1)):
            raise ValueError("Connected components differ from recorded log count")
        ends = []
        for indices in groups:
            points = [world[index] for index in indices]
            center, axis = fit_axis(points)
            along = [(p-center).dot(axis) for p in points]
            radius = max((p-center-axis*(p-center).dot(axis)).length for p in points)
            cylinders.append((center, axis, radius))
            caps = [center+axis*t for t in (min(along), max(along))]
            ends.append({"source_caps": [[p.x, -p.y*sine-p.z*cosine] for p in caps],
                         "length": max(along)-min(along), "radius": radius})
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        defects = {"nonmanifold_edges": sum(not e.is_manifold for e in bm.edges),
                   "degenerate_faces": sum(f.calc_area() < 1e-7 for f in bm.faces)}
        bm.free()
        if any(defects.values()):
            raise ValueError(defects)
        report.append({"source_node": obj["source_node"], "logs": len(groups),
                       "caps": ends, "min_world_z": min(p.z for p in world), **defects})
    gaps = []
    for i, (center, axis, radius) in enumerate(cylinders):
        nearby = []
        for j, (other, other_axis, other_radius) in enumerate(cylinders):
            if i == j:
                continue
            if abs(axis.dot(other_axis)) < .999:
                raise ValueError("Unexpected nonparallel cylinders")
            delta = other-center
            nearby.append((delta-axis*delta.dot(axis)).length-radius-other_radius)
        gaps.append(min(nearby))
    return {"status": "reviewed_no_geometry_change", "parts": report,
            "nearest_neighbor_gap_range": [min(gaps), max(gaps)],
            "rejected_change": "Source-x truncation removes potentially valid logs behind bucket",
            "remaining": ["Foreground bucket needs separate geometry/ownership review",
                          "Composite mask 30 is not an exclusive timber mask",
                          "Exact exposed end correspondence and hidden count remain uncertain"]}
