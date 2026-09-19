"""Render eight views with source pixels, shaded unknown geometry, and edit masks."""
import hashlib
import json
import math
from pathlib import Path

import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree

import setup_map


def _solid_views(scene, cameras, objects, output):
    """Render untextured structure using the exact sampling cameras."""
    shading = scene.display.shading
    shading_values = {
        "light": "STUDIO", "color_type": "SINGLE", "single_color": (0.55, 0.55, 0.55),
        "show_shadows": True, "show_cavity": True, "cavity_type": "BOTH",
        "show_specular_highlight": False,
    }
    previous_shading = {key: getattr(shading, key) for key in shading_values}
    previous_render = {key: getattr(scene.render, key) for key in
                       ("engine", "filepath", "film_transparent", "resolution_percentage", "use_border")}
    previous_camera = scene.camera
    previous_format = scene.render.image_settings.file_format
    hidden = [(obj, obj.hide_render) for obj in scene.objects if obj.type == "MESH"]
    markers = [(marker, marker.camera) for marker in scene.timeline_markers]
    buffers = []
    try:
        for obj, _ in hidden:
            obj.hide_render = obj not in objects
        for marker, _ in markers:
            marker.camera = None
        for key, value in shading_values.items():
            setattr(shading, key, value)
        scene.render.engine = "BLENDER_WORKBENCH"
        scene.render.film_transparent = True
        scene.render.resolution_percentage = 100
        scene.render.use_border = False
        scene.render.image_settings.file_format = "PNG"
        for index, camera in enumerate(cameras):
            scene.camera = camera
            scene.render.filepath = str(output / f"view-{index}-solid.png")
            bpy.ops.render.render(write_still=True, scene=scene.name)
            rendered = bpy.data.images.load(scene.render.filepath, check_existing=False)
            buffers.append(list(rendered.pixels))
            bpy.data.images.remove(rendered)
    finally:
        for obj, value in hidden:
            obj.hide_render = value
        for marker, camera in markers:
            marker.camera = camera
        for key, value in previous_shading.items():
            setattr(shading, key, value)
        for key, value in previous_render.items():
            setattr(scene.render, key, value)
        scene.render.image_settings.file_format = previous_format
        scene.camera = previous_camera
    return buffers


def _tree(objects):
    vertices, triangles, owners = [], [], []
    for obj in objects:
        start = len(vertices)
        vertices.extend(obj.matrix_world @ vertex.co for vertex in obj.data.vertices)
        obj.data.calc_loop_triangles()
        triangles.extend(tuple(start+i for i in triangle.vertices) for triangle in obj.data.loop_triangles)
        owners.extend((obj,triangle.polygon_index) for triangle in obj.data.loop_triangles)
    return BVHTree.FromPolygons(vertices, triangles, all_triangles=True), owners


def prepare(output_dir, asset_id="derby-south-gatehouse", width=384, height=512):
    output = Path(output_dir)
    output.mkdir(parents=True, exist_ok=False)
    scene = bpy.data.scenes["Derby Refinement"]; bpy.context.window.scene = scene
    bpy.context.view_layer.update()
    all_objects = [o for o in bpy.data.collections["Derby Working"].objects
                   if o.type == "MESH" and not o.hide_render]
    objects = [o for o in all_objects if o.get("asset_group") == asset_id]
    if not objects:
        raise ValueError("Asset has no visible geometry")
    scene.render.resolution_x = width; scene.render.resolution_y = height
    (asset_tree,asset_owners), (source_tree,source_owners) = _tree(objects), _tree(all_objects)
    source_path = Path(next(i.filepath for i in bpy.data.images if i.filepath.endswith("covered.png"))).resolve()
    source_hash = hashlib.sha256(source_path.read_bytes()).hexdigest()
    # Reload exact file bytes, never select a possibly stale packed image.
    source_image = bpy.data.images.load(str(source_path),check_existing=False)
    source_pixels = list(source_image.pixels)
    source_width, source_height = source_image.size
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
    toward_source = Vector((0, -cosine, sine))
    points = [obj.matrix_world @ vertex.co for obj in objects for vertex in obj.data.vertices]
    target = (Vector(tuple(min(p[a] for p in points) for a in range(3))) +
              Vector(tuple(max(p[a] for p in points) for a in range(3)))) / 2
    cameras = []
    for index in range(8):
        camera = bpy.data.objects["Derby reference"].copy(); camera.data = camera.data.copy()
        scene.collection.objects.link(camera); camera.name = f"Multiview source audit {index}"
        yaw = math.radians(index * 45)
        camera.location = target + Vector((math.sin(yaw)*cosine,-math.cos(yaw)*cosine,sine))*1600
        camera.rotation_euler = (target-camera.location).to_track_quat("-Z","Y").to_euler()
        setup_map.fit_camera(camera, objects, width/height)
        cameras.append(camera)
    shared_scale = max(camera.data.ortho_scale for camera in cameras)
    for camera in cameras: camera.data.ortho_scale = shared_scale
    bpy.context.view_layer.update()
    solid_views = _solid_views(scene, cameras, objects, output)
    records = []
    audits = []
    for index, camera in enumerate(cameras):
        frame = camera.data.view_frame(scene=scene)
        left,right = min(p.x for p in frame),max(p.x for p in frame)
        bottom,top = min(p.y for p in frame),max(p.y for p in frame)
        direction = camera.matrix_world.to_3x3() @ Vector((0,0,-1))
        pixels, mask = [], []
        counts = {"known":0,"unknown":0,"background":0}
        for y in range(height):
            for x in range(width):
                origin = camera.matrix_world @ Vector((left+(x+0.5)*(right-left)/width,
                                                       bottom+(y+0.5)*(top-bottom)/height,0))
                hit, normal, triangle_id, _ = asset_tree.ray_cast(origin,direction)
                if hit is None:
                    pixels.extend((0,0,0,1)); mask.extend((1,1,1,1)); counts["background"]+=1
                    continue
                sx = hit.x
                sy = -hit.y*sine-hit.z*cosine
                owner, face_id = asset_owners[triangle_id]
                minimum_cosine = max(0.05,float(owner.get("projection_min_cosine",0.05)))
                verified = (owner.get("reprojection_source_sha256")==source_hash
                            and normal.dot(toward_source)>minimum_cosine and 0<=sx<source_width and 0<=sy<source_height
                            and source_tree.ray_cast(hit+toward_source*0.1,toward_source)[0] is None)
                # A point can be visible while its nearest source texel belongs
                # to a neighbor/background. Verify the actual sampled texel's ray.
                if verified and index != 0:
                    sample = hit + Vector((math.floor(sx)+0.5-sx,0,0))
                    sample -= Vector((0,sine,cosine))*(math.floor(sy)+0.5-sy)
                    _,_,sample_triangle,_ = source_tree.ray_cast(sample+toward_source*10000,-toward_source)
                    verified = (sample_triangle is not None and source_owners[sample_triangle][0] == owner)
                # The reference tile is the original artwork clipped to the
                # projected asset silhouette, not a surface-confidence mask.
                # Keep oblique-view rejection out of this direct source view.
                if index == 0:
                    verified = 0 <= sx < source_width and 0 <= sy < source_height
                    if not verified:
                        raise ValueError("Reference silhouette extends outside source artwork")
                if verified:
                    px = min(source_width-1,max(0,int(sx)))
                    py = min(source_height-1,max(0,source_height-1-int(sy)))
                    offset = (py*source_width+px)*4
                    pixels.extend((*source_pixels[offset:offset+3],1))
                    mask.extend((1,1,1,1)); counts["known"]+=1
                    if index==0 and len(audits)<100 and counts["known"]%500==0:
                        audits.append({"output_xy":[x,height-1-y],"source_xy":[px,source_height-1-py],
                                       "source_node":owner.get("source_node"),"face":face_id,
                                       "source_rgb":[round(value*255) for value in source_pixels[offset:offset+3]]})
                else:
                    offset = (y*width+x)*4
                    pixels.extend((*solid_views[index][offset:offset+3],1))
                    mask.extend((1,1,1,0)); counts["unknown"]+=1
        for suffix, values in (("input",pixels),("mask",mask)):
            image = bpy.data.images.new(f"Multiview {index} {suffix}",width=width,height=height,alpha=True)
            image.pixels.foreach_set(values); image.file_format="PNG"
            image.filepath_raw=str(output/f"view-{index}-{suffix}.png"); image.save()
            bpy.data.images.remove(image)
        records.append({"index":index,"azimuth_degrees":index*45,"elevation_degrees":35,
                        "camera_matrix_world":[list(row) for row in camera.matrix_world],
                        "ortho_scale":shared_scale,"counts":counts,
                        "input":f"view-{index}-input.png","mask":f"view-{index}-mask.png",
                        "crop":{"left":index%4*width,"top":index//4*height,"width":width,"height":height}})
    manifest = {"version":1,"asset_id":asset_id,"layout":{"columns":4,"rows":2,"width":4*width,"height":2*height},
                "source_image":str(source_path),"source_sha256":hashlib.sha256(source_path.read_bytes()).hexdigest(),
                "source_blend":bpy.data.filepath,"source_blend_sha256":hashlib.sha256(Path(bpy.data.filepath).read_bytes()).hexdigest(),
                "source_projection":"x=X, y=-Y*sin(35)-Z*cos(35)",
                "known_rule":"View 0: original artwork clipped to projected asset silhouette, fully protected; may include scene occluders. Other views: fresh source bytes, matching reprojection SHA, per-object grazing cutoff, unoccluded world ray, sampled source texel belongs to same object.",
                "source_pixel_audit":audits,
                "unknown_appearance":"Neutral untextured Blender Workbench shading with shadows and cavity; exact same cameras.",
                "mask_rule":"Transparent=unknown shaded geometry; opaque=original source texel or background.",
                "views":records}
    (output/"views.json").write_text(json.dumps(manifest,indent=2))
    return manifest
