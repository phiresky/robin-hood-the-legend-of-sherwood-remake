"""Comparable, source-only review packets for isolated Blender asset workers.

``render_review(..., frame_manifest='input/views.json')`` reuses the baseline
framing for a modified model. Existing UVs/materials are deliberately ignored:
only pixels from declared source artwork can appear in the textured review.
"""
from array import array
import hashlib
import json
import math
from pathlib import Path

import bpy
from mathutils import Matrix, Vector
from mathutils.bvhtree import BVHTree

from experiment_multiview_texture import _solid_views
from setup_map import fit_camera


def _tree(objects):
    vertices, triangles, owners = [], [], []
    depsgraph = bpy.context.evaluated_depsgraph_get()
    for obj in objects:
        evaluated = obj.evaluated_get(depsgraph)
        mesh = evaluated.to_mesh()
        try:
            mesh.calc_loop_triangles()
            start = len(vertices)
            vertices.extend(obj.matrix_world @ vertex.co for vertex in mesh.vertices)
            triangles.extend(tuple(start + v for v in tri.vertices) for tri in mesh.loop_triangles)
            owners.extend(obj for _ in mesh.loop_triangles)
        finally:
            evaluated.to_mesh_clear()
    if not triangles:
        raise ValueError("Review geometry contains no triangles")
    return BVHTree.FromPolygons(vertices, triangles, all_triangles=True), owners, vertices


def _save(path, width, height, pixels):
    image = bpy.data.images.new("Refinement review output", width=width, height=height, alpha=True)
    try:
        image.pixels.foreach_set(pixels)
        image.file_format = "PNG"
        image.filepath_raw = str(path)
        image.save()
    finally:
        bpy.data.images.remove(image)


def _tile(buffers, width, height, path):
    pixels = array("f", [0.0]) * (width * height * 8 * 4)
    for index, buffer in enumerate(buffers):
        for row in range(height):
            destination = (((1 - index // 4) * height + row) * width * 4 + index % 4 * width) * 4
            pixels[destination:destination + width * 4] = array("f", buffer[row * width * 4:(row + 1) * width * 4])
    _save(path, width * 4, height * 2, pixels)


def render_review(output_dir, *, scene_name, collection_name, asset_id,
                  source_path, frame_manifest=None, width=384, height=512,
                  elevation_degrees=35.0, context_padding=24, projection_layers=None,
                  lighting=None, source_mask_manifest=None):
    """Render context.png, solid.png, textured.png, views.json and individual views.

    Coordinates use the map's orthographic projection: source x=X,
    source y=-Y*sin(elevation)-Z*cos(elevation), measured from image top-left.
    Optional projection_layers is a list of {source_path, receiver_nodes,
    occluder_nodes}; each receiver belongs to exactly one layer. This lets an
    interior use revealed artwork with only its declared occluders. Pass full
    scene context geometry in collection_name even in an isolated worker file.
    The crop is raw source artwork including background, not a silhouette cutout.
    This function does not mutate materials or save the blend file.
    source_mask_manifest optionally restricts evidence per receiver/layer to
    reviewed silhouettes minus explicitly reviewed foreground masks.
    """
    if width <= 0 or height <= 0 or context_padding < 0:
        raise ValueError("Positive render dimensions and nonnegative padding required")
    output = Path(output_dir).resolve()
    if output.exists():
        raise FileExistsError(f"Review directory already exists: {output}")
    baseline = (json.loads(Path(frame_manifest).read_text()) if isinstance(frame_manifest, (str, Path))
                else frame_manifest)
    if baseline:
        if baseline.get("version") != 1 or baseline["asset_id"] != asset_id:
            raise ValueError("Framing manifest version or asset does not match")
        width, height = baseline["tile_size"]
        elevation_degrees = baseline["elevation_degrees"]
    scene = bpy.data.scenes[scene_name]
    old_scene = bpy.context.window.scene
    bpy.context.window.scene = scene
    old_resolution = (scene.render.resolution_x, scene.render.resolution_y)
    old_aspect = (scene.render.pixel_aspect_x, scene.render.pixel_aspect_y)
    cameras, images = [], []
    try:
        bpy.context.view_layer.update()
        all_objects = [o for o in bpy.data.collections[collection_name].all_objects
                       if o.type == "MESH" and not o.hide_render]
        objects = [o for o in all_objects if o.get("asset_group") == asset_id]
        if not objects:
            raise ValueError(f"No visible mesh objects for {asset_id}")
        asset_tree, owners, points = _tree(objects)
        source_path = Path(source_path).resolve()
        source_hash = hashlib.sha256(source_path.read_bytes()).hexdigest()
        if baseline and baseline["source_sha256"] != source_hash:
            raise ValueError("Source artwork changed since baseline review")

        def read_image(path):
            image = bpy.data.images.load(str(Path(path).resolve()), check_existing=False)
            images.append(image)
            pixels = array("f", [0.0]) * (image.size[0] * image.size[1] * 4)
            image.pixels.foreach_get(pixels)
            return pixels, int(image.size[0]), int(image.size[1])

        original, source_width, source_height = read_image(source_path)
        angle = math.radians(elevation_degrees)
        sine, cosine = math.sin(angle), math.cos(angle)
        toward_source = Vector((0, -cosine, sine))
        source_down = Vector((0, -sine, -cosine))
        present = {obj.get("source_node") for obj in all_objects}
        definitions = projection_layers or [{"source_path": str(source_path),
                                           **({'projection_label':'exterior'} if source_mask_manifest else {}),
                                           "receiver_nodes": sorted(present, key=str),
                                           "occluder_nodes": sorted(present, key=str)}]
        layers, receiver_layers, layer_records = [], {}, []
        mask_record = None
        if source_mask_manifest:
            from occlusion_constraints import SourceMaskConstraints, evidence_record
            mask_record = evidence_record(source_mask_manifest)
            if baseline and baseline.get('source_mask_evidence') not in (None, mask_record):
                raise ValueError('Reviewed source-mask evidence changed since baseline')
        elif baseline and baseline.get('source_mask_evidence'):
            raise ValueError('Cannot drop source-mask evidence from a frozen review')
        for definition in definitions:
            receivers, occluders = set(definition["receiver_nodes"]), set(definition["occluder_nodes"])
            if (receivers | occluders) - present:
                raise ValueError("Projection layer refers to absent source nodes")
            if receivers & receiver_layers.keys():
                raise ValueError("Projection layers have overlapping receivers")
            path = Path(definition["source_path"]).resolve()
            pixels, sw, sh = read_image(path)
            if (sw, sh) != (source_width, source_height):
                raise ValueError("Projection layers must share source image dimensions")
            tree, layer_owners, _ = _tree([o for o in all_objects if o.get("source_node") in occluders])
            constraints = (SourceMaskConstraints(source_mask_manifest,
                           definition.get('projection_label', 'exterior'),
                           hashlib.sha256(path.read_bytes()).hexdigest(), (sw, sh))
                           if source_mask_manifest else None)
            region = None
            if definition.get('projection_region'):
                from projection_regions import ProjectionRegion
                region = ProjectionRegion(definition['projection_region'],
                    hashlib.sha256(path.read_bytes()).hexdigest(),(sw,sh),all_objects,source_mask_manifest)
            layer = (pixels, tree, layer_owners, constraints, region)
            layers.append(layer)
            receiver_layers.update({node: layer for node in receivers})
            layer_records.append({**definition, "source_path": str(path),
                                  "source_sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
        if any(o.get("source_node") not in receiver_layers for o in objects):
            raise ValueError("Every review object needs an explicit projection receiver layer")
        def without_labels(records):
            return [{k: v for k, v in row.items() if k not in ('projection_label','projection_region')} for row in records]
        if baseline and without_labels(baseline["projection_layers"]) != without_labels(layer_records):
            raise ValueError("Projection layer sources or membership changed since baseline")
        if baseline and (baseline.get('source_mask_evidence') or any(r.get('projection_region') for r in baseline['projection_layers'])) and baseline['projection_layers'] != layer_records:
            raise ValueError('Source-mask projection labels changed since baseline')
        scene.render.resolution_x, scene.render.resolution_y = width, height
        scene.render.pixel_aspect_x = scene.render.pixel_aspect_y = 1
        target = (Vector(tuple(min(p[a] for p in points) for a in range(3))) +
                  Vector(tuple(max(p[a] for p in points) for a in range(3)))) / 2
        for index in range(8):
            data = bpy.data.cameras.new("Refinement review camera")
            camera = bpy.data.objects.new(data.name, data)
            scene.collection.objects.link(camera)
            cameras.append(camera)
            data.type = "ORTHO"
            data.clip_end = 100000
            if baseline:
                record = baseline["views"][index]
                if "camera_location" in record:
                    camera.location = record["camera_location"]
                    camera.rotation_euler = record["camera_rotation_euler"]
                else:
                    camera.matrix_world = Matrix(record["camera_matrix_world"])
                data.ortho_scale = record["ortho_scale"]
            else:
                yaw = math.radians(index * 45)
                camera.location = target + Vector((math.sin(yaw) * cosine, -math.cos(yaw) * cosine, sine)) * 10000
                camera.rotation_euler = (target - camera.location).to_track_quat("-Z", "Y").to_euler()
                # Fit actual evaluated vertices separately at each angle. Box
                # corners and a worst-angle shared scale waste much of the tile.
                fit_camera(camera, objects, width / height, points=points, padding=1.04)
        bpy.context.view_layer.update()
        crop = baseline["context_crop"] if baseline else {
            "left": max(0, math.floor(min(p.x for p in points)) - context_padding),
            "top": max(0, math.floor(min(p.dot(source_down) for p in points)) - context_padding),
            "right": min(source_width, math.ceil(max(p.x for p in points)) + context_padding),
            "bottom": min(source_height, math.ceil(max(p.dot(source_down) for p in points)) + context_padding)}
        cw, ch = crop["right"] - crop["left"], crop["bottom"] - crop["top"]
        if cw <= 0 or ch <= 0:
            raise ValueError("Asset projection does not overlap source artwork")
        output.mkdir(parents=True)
        views_dir = output / "views"
        views_dir.mkdir()
        context = array("f")
        for y in range(source_height - crop["bottom"], source_height - crop["top"]):
            start = (y * source_width + crop["left"]) * 4
            context.extend(original[start:start + cw * 4])
        _save(output / "context.png", cw, ch, context)
        # A frozen input keeps its lighting as well as its camera framing.
        from review_sunlight import configuration
        lighting = configuration(lighting or (baseline or {}).get("lighting"))
        if baseline and not baseline.get("lighting"):
            from experiment_multiview_texture import _legacy_studio_views
            solids = _legacy_studio_views(scene, cameras, objects, views_dir)
            lighting_record = None
        else:
            solids = _solid_views(scene, cameras, objects, views_dir, lighting=lighting)
            lighting_record = lighting
        textured, records = [], []
        for index, camera in enumerate(cameras):
            frame = camera.data.view_frame(scene=scene)
            left, right = min(p.x for p in frame), max(p.x for p in frame)
            bottom, top = min(p.y for p in frame), max(p.y for p in frame)
            direction = camera.matrix_world.to_3x3() @ Vector((0, 0, -1))
            values, known = array("f"), array("f")
            counts = {"source": 0, "unknown": 0, "background": 0, "mask_rejected": 0,
                      "exterior_fallback_source": 0}
            for y in range(height):
                for x in range(width):
                    origin = camera.matrix_world @ Vector((left + (x + .5) * (right - left) / width,
                                                           bottom + (y + .5) * (top - bottom) / height, 0))
                    hit, normal, triangle, _ = asset_tree.ray_cast(origin, direction)
                    verified = False
                    if hit is not None:
                        owner = owners[triangle]
                        pixels, tree, layer_owners, constraints, region = receiver_layers[owner.get("source_node")]
                        sx, sy = hit.x, hit.dot(source_down)
                        fallback = bool(region and not region.contains(math.floor(sx),math.floor(sy)))
                        if fallback:
                            pixels,tree,layer_owners,constraints = region.pixels.reshape(-1),region.tree,region.owners,region.constraints
                        verified = (normal.dot(toward_source) > max(.05, float(owner.get("projection_min_cosine", .05)))
                                    and 0 <= sx < source_width and 0 <= sy < source_height
                                    and tree.ray_cast(hit + toward_source * .02, toward_source)[0] is None)
                        if verified:
                            sample = hit + Vector((math.floor(sx) + .5 - sx, 0, 0)) + source_down * (math.floor(sy) + .5 - sy)
                            _, _, sampled, _ = tree.ray_cast(sample + toward_source * 100000, -toward_source)
                            verified = sampled is not None and layer_owners[sampled] == owner
                        if verified and constraints and not constraints.allowed_pixel(owner, int(sx), int(sy)):
                            verified = False
                            counts['mask_rejected'] += 1
                    if verified:
                        offset = ((source_height - 1 - int(sy)) * source_width + int(sx)) * 4
                        values.extend((*pixels[offset:offset + 3], 1))
                        counts["source"] += 1
                        counts['exterior_fallback_source'] += int(fallback)
                    elif hit is not None:
                        offset = (y * width + x) * 4
                        values.extend((*solids[index][offset:offset + 3], 1))
                        counts["unknown"] += 1
                    else:
                        values.extend((0, 0, 0, 1))
                        counts["background"] += 1
                    known.extend((int(verified), int(verified), int(verified), 1))
            _save(views_dir / f"view-{index}-textured.png", width, height, values)
            _save(views_dir / f"view-{index}-known.png", width, height, known)
            textured.append(values)
            records.append({"index": index, "azimuth_degrees": index * 45,
                            "camera_matrix_world": [list(row) for row in camera.matrix_world],
                            "camera_location": list(camera.location),
                            "camera_rotation_euler": list(camera.rotation_euler),
                            "ortho_scale": camera.data.ortho_scale, "counts": counts,
                            "ownership_sha256": hashlib.sha256((views_dir / f'view-{index}-known.png').read_bytes()).hexdigest()})
        _tile(solids, width, height, output / "solid.png")
        _tile(textured, width, height, output / "textured.png")
        manifest = {"version": 1, "asset_id": asset_id, "scene_name": scene_name,
                    "collection_name": collection_name, "tile_size": [width, height],
                    "layout": {"columns": 4, "rows": 2}, "elevation_degrees": elevation_degrees,
                    "framing": baseline.get("framing", "legacy shared scale") if baseline else "Per-view evaluated geometry, 4 percent padding; frozen for modified comparison",
                    "context_crop": crop, "source_image": str(source_path), "source_sha256": source_hash,
                    "projection_layers": layer_records, "views": records,
                    "source_mask_evidence": mask_record,
                    "source_mask_manifest": str(Path(source_mask_manifest).resolve()) if source_mask_manifest else None,
                    "source_constraint_status": [
                        {"source_node": obj.get('source_node'),
                         "constrained": bool(receiver_layers[obj.get('source_node')][3] and
                                             receiver_layers[obj.get('source_node')][3].for_object(obj) is not None),
                         "state": receiver_layers[obj.get('source_node')][3].state if receiver_layers[obj.get('source_node')][3] else None,
                         "regional_source_selection": bool(receiver_layers[obj.get('source_node')][4])}
                        for obj in objects],
                    "lighting": lighting_record,
                    "lighting_basis": "World-space direction inferred from upper-left reference illumination; not recovered metadata" if lighting_record else "Historical camera-relative Workbench studio",
                    "source_blend": bpy.data.filepath, "object_names": sorted(o.name for o in objects),
                    "known_rule": "Fresh source pixels, facing source, unoccluded in declared layer, sampled texel ray belongs to the same mesh, inside any reviewed receiver mask and outside its reviewed foreground exclusions. Regional receivers use revealed pixels only inside positive native patch alpha; outside uses covered artwork and exterior visibility. Identical rule in all eight views.",
                    "limitations": ["Artwork ownership is constrained only where explicit reviewed masks exist. Unassigned oversized models can still project background onto themselves; compare context and solid silhouettes.",
                                    "Visibility is checked per output pixel; nearest source-texel ownership is conservative at boundaries."]}
        (output / "views.json").write_text(json.dumps(manifest, indent=2) + "\n")
        return manifest
    finally:
        for camera in cameras:
            data = camera.data
            bpy.data.objects.remove(camera, do_unlink=True)
            bpy.data.cameras.remove(data)
        for image in images:
            bpy.data.images.remove(image)
        scene.render.resolution_x, scene.render.resolution_y = old_resolution
        scene.render.pixel_aspect_x, scene.render.pixel_aspect_y = old_aspect
        bpy.context.window.scene = old_scene
