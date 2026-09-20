"""Bake camera-owned source pixels, with explicitly unknown shaded surfaces.

Run once per reveal layer, passing that layer's source image and occluder nodes.
Geometry and existing UV layers are retained. No save or publication is implicit.
"""
import hashlib
import json
import math
from pathlib import Path


def bake(map_name, source_path, report_path, receiver_nodes=None,
         occluder_nodes=None, projection_label="source", texels_per_unit=1,
         elevation_deg=35.0, preserve_authored=True):
    import bpy
    import numpy as np
    from mathutils import Vector
    from mathutils.bvhtree import BVHTree

    if texels_per_unit <= 0:
        raise ValueError("Texture density must be positive")
    objects = [o for o in bpy.data.collections[map_name + " Working"].all_objects
               if o.type == "MESH" and not o.hide_render]
    present = {o.get("source_node") for o in objects}
    for requested in (receiver_nodes, occluder_nodes):
        if requested is not None and set(requested) - present:
            raise ValueError("Unknown projection nodes: " + str(set(requested) - present))
    receivers = [o for o in objects if receiver_nodes is None or o.get("source_node") in receiver_nodes]
    occluders = [o for o in objects if occluder_nodes is None or o.get("source_node") in occluder_nodes]
    if not receivers or not occluders:
        raise ValueError("Projection requires receivers and occluders")
    if any(m.show_render or m.show_viewport for o in set(receivers + occluders) for m in o.modifiers):
        raise ValueError("Apply geometry modifiers before ownership projection")
    bpy.context.view_layer.update()
    vertices, triangles = [], []
    for obj in occluders:
        obj.data.calc_loop_triangles()
        offset = len(vertices)
        vertices.extend(obj.matrix_world @ v.co for v in obj.data.vertices)
        for triangle in obj.data.loop_triangles:
            triangles.append(tuple(offset + i for i in triangle.vertices))
    tree = BVHTree.FromPolygons(vertices, triangles, all_triangles=True)
    if tree is None:
        raise ValueError("No occluder triangles")
    angle = math.radians(elevation_deg)
    toward = Vector((0, -math.cos(angle), math.sin(angle)))
    camera_depth = max(v.dot(toward) for v in vertices) + 10
    source_path = Path(source_path).resolve()
    source_hash = hashlib.sha256(source_path.read_bytes()).hexdigest()
    source = bpy.data.images.load(str(source_path), check_existing=False)
    sw, sh = source.size
    pixels = np.empty(sw * sh * 4, dtype=np.float32)
    source.pixels.foreach_get(pixels)
    pixels = pixels.reshape(sh, sw, 4)
    bpy.data.images.remove(source)
    ray_count = 0

    def visible_at(position):
        nonlocal ray_count
        ray_count += 1
        point = Vector(position)
        origin = point + toward * (camera_depth - point.dot(toward))
        hit, normal, index, distance = tree.ray_cast(origin, -toward)
        # Compare depth at the continuous projected sample, not polygon identity
        # at a rounded pixel center: finely subdivided coplanar faces share pixels.
        return hit is not None and (hit - point).length <= .01

    uv_name = "Owned source / " + projection_label
    report = {"source": str(source_path), "source_sha256": source_hash,
              "projection_label": projection_label, "receiver_nodes": receiver_nodes,
              "occluder_nodes": occluder_nodes, "objects": [], "geometry_changed": False,
              "ownership": "First hit depth at continuous projected texel center; tolerance 0.01 world units",
              "unknown": "Neutral shaded color; no previous projected texture retained",
              "limitations": ["Geometry outside the artwork silhouette must still be corrected geometrically.",
                              "Reveal layers require explicit retained occluders matching their source artwork.",
                              "Ground cleanup and explicit projection_preserve materials are retained."]}
    light = Vector((-.35, -.45, .82)).normalized()
    for obj in receivers:
        if obj.get("source_node") == "ground" or obj.get("source_obstacle") == "ground":
            continue
        if obj.data.users > 1:
            obj.data = obj.data.copy()
        mesh = obj.data
        mesh.calc_loop_triangles()
        world = [obj.matrix_world @ v.co for v in mesh.vertices]
        by_face = {}
        for triangle in mesh.loop_triangles:
            by_face.setdefault(triangle.polygon_index, []).append(triangle)
        islands = []
        preserved = 0
        degenerate = 0
        for face in mesh.polygons:
            mat = mesh.materials[face.material_index] if mesh.materials else None
            if preserve_authored and mat and mat.get("projection_preserve") and not mat.get("source_ownership_bake"):
                preserved += 1
                continue
            points = [world[i] for i in face.vertices]
            normal = (obj.matrix_world.to_3x3().inverted().transposed() @ face.normal).normalized()
            origin = points[0]
            axis = max((p - origin for p in points), key=lambda v: v.length).normalized()
            vertical = normal.cross(axis).normalized()
            coords = [Vector(((p-origin).dot(axis), (p-origin).dot(vertical))) for p in points]
            low = Vector((min(p.x for p in coords), min(p.y for p in coords)))
            size = Vector((max(p.x for p in coords), max(p.y for p in coords))) - low
            if min(size) < 1e-7:
                degenerate += 1
                continue
            w, h = [min(1024, max(2, math.ceil(v * texels_per_unit))) for v in size]
            islands.append((face.index, origin, axis, vertical, normal, low, size, w, h))
        if not islands:
            report["objects"].append({"object": obj.name, "authored_faces_preserved": preserved,
                                      "degenerate_faces_unchanged": degenerate})
            continue
        area = sum((island[7]+4)*(island[8]+4) for island in islands)
        desired = max(64, max(island[7]+4 for island in islands), math.sqrt(area))
        width = min(2048, 2**math.ceil(math.log2(desired)))
        x = y = 2
        row = 0
        packed = []
        for island in islands:
            w, h = island[7:9]
            if x + w + 2 > width:
                x = 2
                y += row + 4
                row = 0
            packed.append((*island, x, y))
            x += w + 4
            row = max(row, h)
        islands = packed
        height = y + row + 2
        if height > 16384:
            raise ValueError(f"Ownership atlas for {obj.name} exceeds 16384 pixels; lower texels_per_unit or split the mesh")
        atlas = np.zeros((height, width, 4), dtype=np.float32)
        atlas[:, :, 3] = 1
        known = unknown = 0
        for fid, origin, axis, vertical, normal, low, size, w, h, left, bottom in islands:
            yy, xx = np.mgrid[-2:h+2, -2:w+2]
            qx = low.x + (xx.ravel()+.5)*size.x/w
            qy = low.y + (yy.ravel()+.5)*size.y/h
            positions = np.zeros((len(qx), 3))
            best = np.full(len(qx), -np.inf)
            for triangle in by_face[fid]:
                ps = [world[i] for i in triangle.vertices]
                a, b, c = [Vector(((p-origin).dot(axis), (p-origin).dot(vertical))) for p in ps]
                det = (b.y-c.y)*(a.x-c.x)+(c.x-b.x)*(a.y-c.y)
                if abs(det) < 1e-12:
                    continue
                wa = ((b.y-c.y)*(qx-c.x)+(c.x-b.x)*(qy-c.y))/det
                wb = ((c.y-a.y)*(qx-c.x)+(a.x-c.x)*(qy-c.y))/det
                weights = np.stack((wa, wb, 1-wa-wb), axis=1)
                margin = weights.min(axis=1)
                take = margin > best
                positions[take] = weights[take] @ np.asarray(ps)
                best[take] = margin[take]
            colors = np.ones((len(qx), 4), dtype=np.float32)
            colors[:, :3] = .16 + .16*max(0, normal.dot(light))
            sx = np.floor(positions[:, 0]).astype(int)
            sy = np.floor(sh + positions[:, 1]*math.sin(angle) + positions[:, 2]*math.cos(angle)).astype(int)
            front = normal.dot(toward) > float(obj.get("projection_min_cosine", .0001))
            accepted = np.zeros(len(qx), dtype=bool)
            if front:
                for i in np.flatnonzero((sx >= 0) & (sx < sw) & (sy >= 0) & (sy < sh)):
                    accepted[i] = visible_at(positions[i])
            colors[accepted] = pixels[sy[accepted], sx[accepted]]
            inside = best >= 0
            known += int(np.count_nonzero(accepted & inside))
            unknown += int(np.count_nonzero(~accepted & inside))
            atlas[bottom-2:bottom+h+2, left-2:left+w+2] = colors.reshape(h+4, w+4, 4)
        name = obj.name + " / owned " + projection_label
        existing = next(((i, m) for i, m in enumerate(mesh.materials)
                         if m and m.get("source_ownership_label") == projection_label), None)
        old_image = next((n.image for n in existing[1].node_tree.nodes
                          if n.type == "TEX_IMAGE" and n.image), None) if existing else None
        image = old_image or bpy.data.images.new(name, width=width, height=height, alpha=True)
        if tuple(image.size) != (width, height):
            image.scale(width, height)
        image.pixels.foreach_set(atlas.ravel())
        image.update()
        image.pack()
        mat = existing[1] if existing else bpy.data.materials.new(name)
        mat.use_nodes = True
        mat["source_ownership_bake"] = True
        mat["source_ownership_label"] = projection_label
        mat["projection_preserve"] = True
        mat["reprojection_source_sha256"] = source_hash
        nodes, links = mat.node_tree.nodes, mat.node_tree.links
        nodes.clear()
        uv = nodes.new("ShaderNodeUVMap")
        uv.uv_map = uv_name
        texture = nodes.new("ShaderNodeTexImage")
        texture.image = image
        texture.interpolation = "Closest"
        emission = nodes.new("ShaderNodeEmission")
        output = nodes.new("ShaderNodeOutputMaterial")
        links.new(uv.outputs["UV"], texture.inputs["Vector"])
        links.new(texture.outputs["Color"], emission.inputs["Color"])
        links.new(emission.outputs[0], output.inputs["Surface"])
        slot = existing[0] if existing else len(mesh.materials)
        if not existing:
            mesh.materials.append(mat)
        layer = mesh.uv_layers.get(uv_name) or mesh.uv_layers.new(name=uv_name)
        fallback = mesh.attributes.get("reprojection_fallback_material")
        for fid, origin, axis, vertical, normal, low, size, w, h, left, bottom in islands:
            face = mesh.polygons[fid]
            face.material_index = slot
            if fallback:
                fallback.data[fid].value = slot
            for lid in face.loop_indices:
                p = world[mesh.loops[lid].vertex_index] - origin
                q = Vector((p.dot(axis), p.dot(vertical))) - low
                layer.data[lid].uv = ((left+q.x/size.x*w)/width, (bottom+q.y/size.y*h)/height)
        report["objects"].append({"object": obj.name, "faces": len(islands),
                                  "known_texels": known, "unknown_texels": unknown,
                                  "atlas_size": [width, height], "authored_faces_preserved": preserved})
        obj['reprojection_ownership_label'] = projection_label
        obj['reprojection_ownership_source_sha256'] = source_hash
        obj['reprojection_known_texels'] = known
        obj['reprojection_unknown_texels'] = unknown
        report["objects"][-1]["degenerate_faces_unchanged"] = degenerate
    report["visibility_rays"] = ray_count
    report["known_texels"] = sum(o.get("known_texels", 0) for o in report["objects"])
    report["unknown_texels"] = sum(o.get("unknown_texels", 0) for o in report["objects"])
    Path(report_path).parent.mkdir(parents=True, exist_ok=True)
    Path(report_path).write_text(json.dumps(report, indent=2)+"\n")
    return report
