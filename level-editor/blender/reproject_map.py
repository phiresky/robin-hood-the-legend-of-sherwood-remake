"""Refresh source-visible surfaces after geometry edits; invoke through Blender MCP.

The map uses orthographic pixel coordinates. Concealed and ambiguously visible
faces retain their existing atlas. Visibility is sampled, not a per-pixel proof;
review the exported reference and oblique renders before publishing.
"""
import hashlib
import json
import math
from pathlib import Path


def project_uv(point, width, height, elevation_deg=35.0):
    """Z-up world coordinates to bottom-origin map-image UVs."""
    angle = math.radians(elevation_deg)
    return (point[0] / width,
            1.0 + (point[1] * math.sin(angle) + point[2] * math.cos(angle)) / height)


def reproject_map(map_name, source_path, report_path, elevation_deg=35.0,
                  sample_spacing=12.0, max_subdivisions=24,
                  receiver_nodes=None, occluder_nodes=None, projection_label="source",
                  exclude_occluder_components=None, receiver_components=None):
    """Recompute map projection and visibility from the current working meshes.

    This intentionally leaves ground on its existing cleaned atlas. A separate
    ownership-mask/inpainting pass is needed when structures expose new ground.
    Existing UVs and material assignments are retained as fallback. Calling again
    updates UVs, image pixels and visibility without accumulating material slots.
    Supply explicit stable source-node receiver/occluder lists for each reveal
    layer, and a distinct projection_label for its texture. Closed exterior and
    revealed interior artwork must not share an indiscriminate visibility pass.
    No save or publication is implicit. Export only after this call completes.
    """
    import bpy
    from mathutils import Vector
    from mathutils.bvhtree import BVHTree

    if sample_spacing <= 0 or max_subdivisions < 1:
        raise ValueError("Visibility sample spacing and subdivisions must be positive")
    source_path = Path(source_path).resolve()
    report_path = Path(report_path)
    source_hash = hashlib.sha256(source_path.read_bytes()).hexdigest()
    working = bpy.data.collections[map_name + " Working"]
    sources = [o for o in working.all_objects if o.type == "MESH" and not o.hide_render]
    if not sources:
        raise ValueError("No working meshes")
    for obj in sources:
        if obj.get("source_node") == "ground" or obj.get("source_obstacle") == "ground":
            continue
        if any(mod.show_render or mod.show_viewport for mod in obj.modifiers):
            raise ValueError(f"Bake active modifiers before reprojection: {obj.name}")
        if not obj.data.uv_layers or not obj.data.materials:
            raise ValueError(f"Missing fallback UV/material: {obj.name}")
    receivers = sources if receiver_nodes is None else [
        obj for obj in sources if obj.get("source_node") in set(receiver_nodes)]
    from reveal_components import filter_receivers
    receivers=filter_receivers(receivers,receiver_components,available_objects=working.all_objects)
    occluders = sources if occluder_nodes is None else [
        obj for obj in sources if obj.get("source_node") in set(occluder_nodes)]
    from reveal_components import filter_occluders
    occluders = filter_occluders(occluders, exclude_occluder_components,
        projection_label=projection_label, available_objects=working.all_objects)
    if not receivers or not occluders:
        raise ValueError("Projection receiver and occluder sets must be nonempty")
    present = {obj.get("source_node") for obj in sources}
    for requested in (receiver_nodes, occluder_nodes):
        if requested is not None and set(requested) - present:
            raise ValueError(f"Unknown source nodes: {sorted(set(requested) - present)}")
    bpy.context.view_layer.update()
    depsgraph = bpy.context.evaluated_depsgraph_get()
    vertices, triangles, triangle_owners = [], [], []
    geometry_hash = hashlib.sha256()
    for obj in sorted(sources, key=lambda o: o.name):
        evaluated = obj.evaluated_get(depsgraph)
        mesh = evaluated.to_mesh()
        try:
            mesh.calc_loop_triangles()
            offset = len(vertices)
            world = [obj.matrix_world @ vertex.co for vertex in mesh.vertices]
            if obj in occluders:
                vertices.extend(world)
                triangles.extend(tuple(offset + i for i in face.vertices) for face in mesh.loop_triangles)
                triangle_owners.extend([obj.name] * len(mesh.loop_triangles))
            geometry_hash.update(json.dumps([obj.name, [list(p) for p in world],
                                            [list(f.vertices) for f in mesh.polygons]],
                                           separators=(",", ":")).encode())
        finally:
            evaluated.to_mesh_clear()
    tree = BVHTree.FromPolygons(vertices, triangles, all_triangles=True)
    if tree is None:
        raise ValueError("Working meshes contain no triangles")
    angle = math.radians(elevation_deg)
    toward_camera = Vector((0, -math.cos(angle), math.sin(angle)))
    epsilon = 0.02  # World units are map pixels; do not jump past thin surfaces.
    uv_name = "Refreshed map projection" + (" / " + projection_label if projection_label != "source" else "")
    backup_name = "reprojection_fallback_material"
    material_name = map_name + " / refreshed " + projection_label + " projection"
    material = bpy.data.materials.get(material_name)
    image = next((candidate for candidate in bpy.data.images
                  if candidate.get("reprojection_source_sha256") == source_hash
                  and candidate.get("reprojection_source_path") == str(source_path)), None)
    if image is None:
        image = bpy.data.images.load(str(source_path), check_existing=False)
        image.name = map_name + " / refreshed " + projection_label + " image"
        image["reprojection_source_sha256"] = source_hash
        image["reprojection_source_path"] = str(source_path)
    width, height = image.size
    if not width or not height:
        raise ValueError("Source map has no pixels")
    image.pack()
    if material is None:
        material = bpy.data.materials.new(material_name)
    material.use_nodes = True
    material.node_tree.nodes.clear()
    nodes, links = material.node_tree.nodes, material.node_tree.links
    output = nodes.new("ShaderNodeOutputMaterial")
    emission = nodes.new("ShaderNodeEmission")
    texture = nodes.new("ShaderNodeTexImage")
    texture.image = image
    texture.interpolation = "Linear"
    texture.extension = "CLIP"
    uv_node = nodes.new("ShaderNodeUVMap")
    uv_node.uv_map = uv_name
    links.new(uv_node.outputs["UV"], texture.inputs["Vector"])
    links.new(texture.outputs["Color"], emission.inputs["Color"])
    links.new(emission.outputs[0], output.inputs["Surface"])

    report = {"map": map_name, "source": str(source_path), "source_sha256": source_hash,
              "geometry_sha256": geometry_hash.hexdigest(), "size": [width, height],
              "elevation_deg": elevation_deg, "sample_spacing": sample_spacing,
              "projection_label": projection_label, "receiver_nodes": receiver_nodes,
              "occluder_nodes": occluder_nodes,
              "max_subdivisions": max_subdivisions, "objects": [],
              "limitations": ["Sampled face visibility; sub-sample occluders may remain.",
                              "Partially occluded faces retain the previous atlas.",
                              "Ground cleanup is preserved; newly exposed ground needs separate review."]}
    for obj in receivers:
        if obj.get("source_node") == "ground" or obj.get("source_obstacle") == "ground":
            report["objects"].append({"object": obj.name, "ground_preserved": True})
            continue
        # Evaluated surfaces must coincide with editable faces for safe per-face
        # assignment. Bake modifiers deliberately before requesting projection.
        if any(mod.show_render or mod.show_viewport for mod in obj.modifiers):
            raise ValueError(f"Bake active modifiers before reprojection: {obj.name}")
        mesh = obj.data
        if mesh.users > 1:
            mesh = obj.data = mesh.copy()
        if not mesh.uv_layers or not mesh.materials:
            raise ValueError(f"Missing fallback UV/material: {obj.name}")
        fallback = mesh.attributes.get(backup_name)
        if fallback is None:
            fallback = mesh.attributes.new(backup_name, "INT", "FACE")
            for face in mesh.polygons:
                fallback.data[face.index].value = face.material_index
        elif fallback.domain != "FACE" or fallback.data_type != "INT":
            raise ValueError(f"Invalid fallback material attribute: {obj.name}")
        projection_index = next((i for i, mat in enumerate(mesh.materials) if mat == material), None)
        if projection_index is None:
            projection_index = len(mesh.materials)
            mesh.materials.append(material)
        # New faces should inherit the fallback index attribute from their source
        # faces. Refuse an invalid fallback instead of silently losing its atlas.
        for face in mesh.polygons:
            index = fallback.data[face.index].value
            if index >= len(mesh.materials) or index == projection_index:
                raise ValueError(f"Invalid fallback index on {obj.name} face {face.index}")
            face.material_index = index
        active_uv = mesh.uv_layers.active_index
        render_uv = next((layer.name for layer in mesh.uv_layers if layer.active_render), None)
        uv_layer = mesh.uv_layers.get(uv_name) or mesh.uv_layers.new(name=uv_name)
        mesh.uv_layers.active_index = active_uv
        if render_uv:
            mesh.uv_layers[render_uv].active_render = True
        world = [obj.matrix_world @ v.co for v in mesh.vertices]
        coordinates = [project_uv(p, width, height, elevation_deg) for p in world]
        for loop in mesh.loops:
            uv_layer.data[loop.index].uv = coordinates[loop.vertex_index]
        mesh.calc_loop_triangles()
        eligible = {face.index: True for face in mesh.polygons}
        reasons = {}
        ray_count = 0
        blocking_objects = {}
        for triangle in mesh.loop_triangles:
            fid = triangle.polygon_index
            if not eligible[fid]:
                continue
            baseline_material = mesh.materials[fallback.data[fid].value]
            if baseline_material and baseline_material.get("projection_preserve"):
                eligible[fid] = False
                reasons[fid] = "authored_material"
                continue
            points = [world[i] for i in triangle.vertices]
            normal = (points[1] - points[0]).cross(points[2] - points[0])
            minimum_cosine = float(obj.get("projection_min_cosine", 1e-4))
            if not 0 <= minimum_cosine < 1:
                raise ValueError(f"Invalid projection angle threshold: {obj.name}")
            if normal.length < 1e-8 or normal.normalized().dot(toward_camera) <= minimum_cosine:
                eligible[fid] = False
                reasons[fid] = "grazing_or_backfacing" if minimum_cosine > 1e-4 else "backfacing"
                continue
            uvs = [coordinates[i] for i in triangle.vertices]
            if any(u < 0 or u > 1 or v < 0 or v > 1 for u, v in uvs):
                eligible[fid] = False
                reasons[fid] = "outside_source"
                continue
            span = max(math.hypot((uvs[i][0] - uvs[j][0]) * width,
                                  (uvs[i][1] - uvs[j][1]) * height)
                       for i, j in ((0, 1), (1, 2), (2, 0)))
            divisions = min(max_subdivisions, max(2, math.ceil(span / sample_spacing)))
            center = sum(points, Vector()) / 3
            for a in range(divisions + 1):
                for b in range(divisions + 1 - a):
                    p = (points[0] * a + points[1] * b + points[2] * (divisions - a - b)) / divisions
                    # Inset by a tiny amount to avoid shared-edge ray ambiguity.
                    p = p.lerp(center, 0.002)
                    ray_count += 1
                    hit, _, hit_index, _ = tree.ray_cast(p + toward_camera * epsilon, toward_camera)
                    if hit is not None:
                        blocker = triangle_owners[hit_index]
                        blocking_objects[blocker] = blocking_objects.get(blocker, 0) + 1
                        eligible[fid] = False
                        reasons[fid] = "occluded"
                        break
                if not eligible[fid]:
                    break
        for face in mesh.polygons:
            if eligible[face.index]:
                face.material_index = projection_index
        mesh.update()
        counts = {reason: list(reasons.values()).count(reason) for reason in sorted(set(reasons.values()))}
        report["objects"].append({"object": obj.name, "source_node": obj.get("source_node"),
                                  "projected_faces": sum(eligible.values()),
                                  "fallback_faces": len(reasons), "fallback_reasons": counts,
                                  "visibility_rays": ray_count, "uv_loops": len(mesh.loops),
                                  "blocking_objects": blocking_objects})
        obj["reprojection_geometry_sha256"] = report["geometry_sha256"]
        obj["reprojection_source_sha256"] = source_hash
    report["projected_faces"] = sum(o.get("projected_faces", 0) for o in report["objects"])
    report["fallback_faces"] = sum(o.get("fallback_faces", 0) for o in report["objects"])
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(report, indent=2) + "\n")
    return report


def restore_projection(map_name):
    """Restore saved atlas assignments before changing a map's layer partition.

    Geometry, UV layers and the saved fallback attribute remain intact. Unused
    generated projection materials are left available for rerunning a layer.
    """
    import bpy
    restored = 0
    for obj in bpy.data.collections[map_name + " Working"].all_objects:
        if obj.type != "MESH":
            continue
        fallback = obj.data.attributes.get("reprojection_fallback_material")
        if fallback is None:
            continue
        for face in obj.data.polygons:
            index = fallback.data[face.index].value
            if not 0 <= index < len(obj.data.materials):
                raise ValueError(f"Invalid fallback index on {obj.name} face {face.index}")
            face.material_index = index
            restored += 1
        obj.data.update()
    return {"restored_faces": restored}


def reproject_layers(manifest_path, report_dir=None, sample_spacing=12.0,
                     max_subdivisions=24, ownership_nodes=None, texels_per_unit=1,
                     preserve_authored=True, exterior_source=None,
                     hidden_fill="neutral", source_mask_manifest=None,
                     reproject_authored_nodes=None):
    """Refresh audited exterior/interior layers without changing geometry visibility.

    Receiver ownership comes from the map-specific reviewed recipe, never from
    bounding-box overlap or sight activation. Ambiguous overlap candidates use
    covered artwork and retain ambiguous metadata; they are not promoted to an
    interior receiver or treated as an authored removable shell.
    """
    import importlib.util
    import struct
    import bpy

    manifest_path = Path(manifest_path).resolve()
    manifest = json.loads(manifest_path.read_text())
    if manifest.get("version") != 1:
        raise ValueError("Unsupported projection layer manifest")
    roles_path = Path(__file__).with_name("interior_layers.py")
    spec = importlib.util.spec_from_file_location("projection_interior_roles", roles_path)
    roles = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(roles)
    roles.validate_projection_reviews(manifest, manifest_path.parent)
    interiors = roles.projection_receivers(manifest)
    map_name = manifest["map"]
    working = bpy.data.collections[map_name + " Working"]
    sources = [obj for obj in working.all_objects if obj.type == "MESH" and not obj.hide_render]
    if any(not obj.get("source_node") for obj in sources):
        raise ValueError("Every visible working mesh must retain a stable source_node")
    available = {obj["source_node"] for obj in sources}
    reproject_authored_nodes = set(reproject_authored_nodes or ())
    if reproject_authored_nodes - available:
        raise ValueError("Unknown authored texture reset nodes")
    if ownership_nodes is not None and reproject_authored_nodes - set(ownership_nodes):
        raise ValueError("Authored texture reset nodes must be ownership-baked")
    patches = {patch["id"] for patch in manifest["patches"]}
    if len(patches) != len(manifest["patches"]):
        raise ValueError("Duplicate patch identifiers")
    ownership = {}
    for patch, nodes in interiors.items():
        if patch not in patches:
            raise ValueError(f"Authored interior patch is absent: {patch}")
        if not nodes or len(nodes) != len(set(nodes)):
            raise ValueError(f"Empty or duplicate receiver list for {patch}")
        for node in nodes:
            if node == "ground" or node in ownership:
                raise ValueError(f"Mixed or duplicate receiver classification: {node}")
            if node not in available:
                raise ValueError(f"Authored interior receiver is absent or hidden: {node}")
            ownership[node] = patch
    receiver_components=roles.projection_receiver_components(manifest)
    exterior = sorted((available - ownership.keys()) | {
        selector['source_node'] for selector in receiver_components.get('exterior',[])})
    if not exterior:
        raise ValueError("No exterior receiver nodes")
    paths = {layer: (manifest_path.parent / manifest["sources"][layer]).resolve()
             for layer in ("exterior", "interior")}
    if exterior_source:
        paths['exterior'] = Path(exterior_source).resolve()
    if paths["exterior"] == paths["interior"]:
        raise ValueError("Covered and revealed source paths must be distinct")
    for path in paths.values():
        with path.open("rb") as file:
            header = file.read(24)
        if header[:8] != b"\x89PNG\r\n\x1a\n" or list(struct.unpack(">II", header[16:24])) != manifest["size"]:
            raise ValueError(f"Source image dimensions differ from layer manifest: {path}")
    for obj in sources:
        if obj["source_node"] == "ground":
            continue
        if any(mod.show_render or mod.show_viewport for mod in obj.modifiers):
            raise ValueError(f"Bake active modifiers before reprojection: {obj.name}")
        if not obj.data.materials or not obj.data.uv_layers:
            raise ValueError(f"Missing fallback UV/material: {obj.name}")
    report_dir = Path(report_dir) if report_dir else manifest_path.parent / "reprojection"
    report_dir.mkdir(parents=True, exist_ok=True)
    # Restore every previous pass before applying the new disjoint partition.
    # This includes receivers omitted from a revised interior recipe.
    restored = restore_projection(map_name)
    annotation = roles.annotate_layers(manifest_path)
    manifest_hash = hashlib.sha256(manifest_path.read_bytes()).hexdigest()
    recipe_hash = hashlib.sha256(json.dumps(interiors, sort_keys=True).encode()).hexdigest()
    retained = roles.projection_occluders(manifest, available)
    component_exclusions = roles.projection_component_exclusions(manifest)
    additions=roles.projection_occluder_additions(manifest)
    exterior_occluders=sorted(set(exterior) | (set(additions.get('exterior',[])) & available))
    passes = [("exterior", paths["exterior"], exterior, exterior_occluders)]
    passes.extend(("interior-" + patch, paths["interior"], sorted(nodes), retained[patch])
                  for patch, nodes in sorted(interiors.items()))
    reports = []
    ownership_reports = []
    import importlib.util
    bake_spec = importlib.util.spec_from_file_location('source_projection_bake', Path(__file__).with_name('source_projection_bake.py'))
    baking = importlib.util.module_from_spec(bake_spec)
    bake_spec.loader.exec_module(baking)
    if ownership_nodes is not None and set(ownership_nodes) - available:
        raise ValueError('Unknown ownership bake receiver nodes')
    for label, source, receivers, occluders in passes:
        exclusions=component_exclusions.get(label.removeprefix('interior-')) if label.startswith('interior-') else None
        receiver_selectors=receiver_components.get(label)
        from projection_regions import region_record
        region = (region_record(manifest,manifest_path.parent,label.removeprefix('interior-'),
                  source,paths['exterior'],set(exterior_occluders)|set(receivers),covered_components=exclusions) if label != 'exterior' else None)
        report = reproject_map(map_name, source, report_dir / (label + ".json"),
                               elevation_deg=manifest["elevation_degrees"],
                               sample_spacing=sample_spacing,
                               max_subdivisions=max_subdivisions,
                               receiver_nodes=receivers, occluder_nodes=occluders,
                               projection_label=label, exclude_occluder_components=exclusions,
                               receiver_components=receiver_selectors)
        reports.append(report)
        bake_receivers = receivers if ownership_nodes is None else sorted(set(receivers) & set(ownership_nodes))
        if bake_receivers:
            ownership_reports.append(baking.bake(
                map_name, source, report_dir / (label + '-ownership.json'),
                receiver_nodes=bake_receivers, occluder_nodes=occluders,
                projection_label=label, elevation_deg=manifest['elevation_degrees'],
                texels_per_unit=texels_per_unit, preserve_authored=preserve_authored,
                hidden_fill=hidden_fill, source_mask_manifest=source_mask_manifest,
                projection_region=region,
                exclude_occluder_components=exclusions,
                receiver_components=receiver_selectors,
                reproject_authored_nodes=sorted(reproject_authored_nodes & set(bake_receivers))))
        per_object = {entry["object"]: entry for entry in report["objects"]}
        for obj in sources:
            if obj.name not in per_object:
                continue
            entry = per_object[obj.name]
            obj["reprojection_receiver_layer"] = label
            obj["reprojection_manifest_sha256"] = manifest_hash
            obj["reprojection_recipe_sha256"] = recipe_hash
            obj["reprojection_source_path"] = str(source)
            obj["reprojection_projected_faces"] = entry.get("projected_faces", 0)
            obj["reprojection_fallback_faces"] = entry.get("fallback_faces", 0)
            obj["reprojection_ground_preserved"] = bool(entry.get("ground_preserved"))
    report = {"map": map_name, "manifest": str(manifest_path),
              "manifest_sha256": manifest_hash, "receiver_recipe_sha256": recipe_hash,
              "restored": restored, "interior_receivers": interiors,
              "exterior_receivers": exterior, "passes": reports,
              "interior_occluders": retained,
              "occluder_audit": roles.projection_occluder_audit(manifest),
              "ownership_bakes": ownership_reports,
              "ownership_scope": 'all non-ground receivers' if ownership_nodes is None else list(ownership_nodes),
              "reproject_authored_nodes": sorted(reproject_authored_nodes),
              "projected_faces": sum(item["projected_faces"] for item in reports),
              "fallback_faces": sum(item["fallback_faces"] for item in reports),
              "ambiguous_receivers": sorted({obj["source_node"] for obj in sources
                                              if obj.get("reveal_role") == "ambiguous"}),
              "limitations": [
                  "Ambiguous overlap candidates receive covered artwork, not an inferred interior state.",
                  "Interior visibility includes audited retained shells; partially cut-away facade geometry still requires further review.",
                  "Ground remains on its existing cleaned atlas; no ground synthesis performed.",
                  "Hidden texels use the requested fill; authored materials survive unless their nodes explicitly require fresh projection.",
                  "Face assignment counts describe the preliminary projection; ownership bake reports describe final texel visibility."]}
    (report_dir / "layers-report.json").write_text(json.dumps(report, indent=2) + "\n")
    return report
