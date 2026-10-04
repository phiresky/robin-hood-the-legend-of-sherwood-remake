"""Export refined geometry using stable editor part IDs and named asset parents."""
from contextlib import contextmanager
import json
import hashlib
import math
import struct
import tempfile
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from asset_index import write_asset_index

import bpy
from mathutils import Matrix, Vector
from patch_material_export import export_states
from catalog_schema import is_scenery_node, source_for_part
from publication_contract import publication_parts, validate_export_records


@contextmanager
def safe_external_image_names():
    """Packed image display names must not become filesystem paths in glTF."""
    renamed = []
    try:
        for image in bpy.data.images:
            if '/' in image.name or '\\' in image.name or image.name in ('.', '..'):
                original = image.name
                renamed.append((image, original))
                image.name = 'texture-' + hashlib.sha256(original.encode()).hexdigest()[:24]
        yield
    finally:
        for image, original in renamed:
            image.name = original


def projection_metadata(source):
    """Carry surface ownership and projection provenance through glTF export."""
    values = {}
    for key in source.keys():
        if key.startswith(("reprojection_", "reveal_", "sight_patch_", "mission_patch_", "drawbridge_", "source_ownership_", "source_pixel_correction")) or key in (
                "projection_layer", "projection_component", "projection_min_cosine", "step_count", "crenellation_notches", "arch_segments",
                "gate_refinement", "cottage_refinement", "architecture_refinement", "embrasure_count", "refinement_recipe",
                "derby_furniture_floor_clip", "support_floor_source_node", "support_floor_scene_z",
                "projection_subdivision_spacing", "publication_inactive"):
            value = source[key]
            if hasattr(value, "to_list"):
                value = value.to_list()
            if hasattr(value, "to_dict"):
                value = value.to_dict()
            values[key] = value
    return values


def reveal_metadata(working, sources, include_all=False):
    """Export only labels needed by the editor's reveal preview controls."""
    path = working.get("reveal_manifest_path")
    if not path:
        return None
    manifest = json.loads(Path(path).read_text())
    nodes = {source["source_node"] for source in sources}
    patches = []
    for patch in manifest["patches"]:
        associations = {c["source_node"] for c in patch["coverage_candidates"]}
        associations.update(patch["sight_before"] + patch["sight_after"])
        if not include_all and not nodes.intersection(associations):
            continue
        patches.append({"id": patch["id"], "name": patch["name"]})
    return {"patches": patches} if patches else None


def compact_texture_coordinates(doc):
    """Export only UV channels used by each primitive's material, densely numbered.

    Authoring meshes accumulate projection layers; runtime shaders have fewer UV
    inputs. Accessors remain unchanged, so this remapping is lossless.
    """
    mappings = {}
    for index, material in enumerate(doc.get("materials", [])):
        textures = []
        def visit(value):
            if not isinstance(value, dict):
                return
            for key, item in value.items():
                if key.endswith("Texture") and isinstance(item, dict) and "index" in item:
                    textures.append(item)
                elif isinstance(item, dict):
                    visit(item)
        visit(material)
        used = set()
        for texture in textures:
            transform = texture.get("extensions", {}).get("KHR_texture_transform", {})
            used.add(transform.get("texCoord", texture.get("texCoord", 0)))
        mapping = {old: new for new, old in enumerate(sorted(used))}
        if len(mapping) > 4:
            raise ValueError("Material needs more than four simultaneous UV channels")
        mappings[index] = mapping
        for texture in textures:
            transform = texture.get("extensions", {}).get("KHR_texture_transform", {})
            old = transform.get("texCoord", texture.get("texCoord", 0))
            texture["texCoord"] = mapping[old]
            if "texCoord" in transform:
                transform["texCoord"] = mapping[old]
    for mesh in doc.get("meshes", []):
        for primitive in mesh["primitives"]:
            mapping = mappings.get(primitive.get("material"), {})
            attributes = primitive["attributes"]
            replacement = {key: value for key, value in attributes.items()
                           if not key.startswith("TEXCOORD_")}
            for old, new in mapping.items():
                key = f"TEXCOORD_{old}"
                if key not in attributes:
                    raise ValueError(f"Textured primitive missing {key}")
                replacement[f"TEXCOORD_{new}"] = attributes[key]
            primitive["attributes"] = replacement


def enforce_foliage_contract(doc):
    """Physical coverage and source evidence use distinct, explicit channels."""
    foliage = set()
    for index, material in enumerate(doc.get("materials", [])):
        extras = material.get("extras", {})
        if extras.get("foliage_physical_opacity") is not True:
            continue
        required = {"opacity_semantics": "physical-coverage",
                    "source_ownership_semantics": "separate-mask",
                    "source_ownership_channel": "vertex-color-r"}
        if any(extras.get(key) != value for key, value in required.items()):
            raise ValueError("Foliage material lacks explicit independent opacity/ownership channels")
        if "baseColorTexture" not in material.get("pbrMetallicRoughness", {}):
            raise ValueError("Foliage material requires its physical RGBA texture")
        material.update(alphaMode="MASK", alphaCutoff=0.5,
                        doubleSided=extras.get("foliage_card_sides") != "paired-one-sided")
        if extras.get("foliage_unlit") is True:
            material.setdefault("extensions", {})["KHR_materials_unlit"] = {}
            used = doc.setdefault("extensionsUsed", [])
            if "KHR_materials_unlit" not in used:
                used.append("KHR_materials_unlit")
        foliage.add(index)
    for mesh in doc.get("meshes", []):
        for primitive in mesh.get("primitives", []):
            if primitive.get("material") in foliage and "COLOR_0" not in primitive["attributes"]:
                raise ValueError("Foliage primitive lacks separate COLOR_0 source ownership")


def export_foliage_material(material, mesh):
    """A temporary graph requests COLOR_0 without changing the authored material.

    The editor consumes the exported vertex color as evidence, never RGB tint.
    Ownership boundaries must be split vertices; a triangle cannot represent an
    independent per-pixel evidence boundary with this vertex channel.
    """
    attribute = mesh.color_attributes.active_color
    if attribute is None:
        raise ValueError("Foliage mesh requires an active source ownership color attribute")
    temporary = material.copy()
    temporary.use_nodes = True
    nodes = temporary.node_tree.nodes
    principled = next((node for node in nodes if node.type == "BSDF_PRINCIPLED"), None)
    if principled is None or not principled.inputs['Base Color'].is_linked:
        bpy.data.materials.remove(temporary)
        raise ValueError("Foliage export requires a linked Principled base-color texture")
    # The authored Cycles graph may explicitly discard reverse faces. glTF
    # expresses that behavior with doubleSided=false, so export the physical
    # Principled surface directly instead of an unsupported Backfacing mix.
    output = next((node for node in nodes if node.type == 'OUTPUT_MATERIAL' and node.is_active_output), None)
    if output is None:
        bpy.data.materials.remove(temporary)
        raise ValueError("Foliage material lacks an active output")
    temporary.node_tree.links.new(principled.outputs['BSDF'], output.inputs['Surface'])
    source = principled.inputs['Base Color'].links[0].from_socket
    color = nodes.new('ShaderNodeVertexColor')
    color.layer_name = attribute.name
    multiply = nodes.new('ShaderNodeMix')
    multiply.data_type = 'RGBA'
    multiply.blend_type = 'MULTIPLY'
    multiply.inputs[0].default_value = 1.0
    temporary.node_tree.links.new(source, multiply.inputs[6])
    temporary.node_tree.links.new(color.outputs['Color'], multiply.inputs[7])
    temporary.node_tree.links.new(multiply.outputs[2], principled.inputs['Base Color'])
    return temporary


@contextmanager
def foliage_export_meshes(objects):
    """Normalize foliage on disposable copies for any glTF export entry point."""
    originals, meshes, materials = [], [], []
    try:
        for obj in objects:
            if not any(mat and mat.get("foliage_physical_opacity") is True for mat in obj.data.materials):
                continue
            originals.append((obj, obj.data))
            mesh = obj.data.copy()
            meshes.append(mesh)
            obj.data = mesh
            for slot, material in enumerate(mesh.materials):
                if material and material.get("foliage_physical_opacity") is True:
                    temporary = export_foliage_material(material, mesh)
                    materials.append(temporary)
                    mesh.materials[slot] = temporary
        yield
    finally:
        for obj, mesh in originals:
            obj.data = mesh
        for mesh in meshes:
            bpy.data.meshes.remove(mesh)
        for material in materials:
            bpy.data.materials.remove(material)


def finalize_foliage_glb(path):
    """Apply the same explicit physical-material contract to an audit export."""
    path = Path(path)
    data = path.read_bytes()
    length, kind = struct.unpack_from("<II", data, 12)
    if kind != 0x4E4F534A:
        raise ValueError("Expected a GLB JSON chunk")
    doc = json.loads(data[20:20 + length])
    enforce_foliage_contract(doc)
    chunk = json.dumps(doc, separators=(",", ":")).encode()
    chunk += b" " * (-len(chunk) % 4)
    binary = data[20 + length:]
    path.write_bytes(struct.pack("<4sII", b"glTF", 2, 20 + len(chunk) + len(binary)) +
                     struct.pack("<II", len(chunk), kind) + chunk + binary)


def mission_editor_footprint(sources, pivot):
    """Editor-only bounds; never a fabricated sight-obstacle table entry."""
    points=[obj.matrix_world @ Vector(corner) for obj in sources for corner in obj.bound_box]
    if not points:raise ValueError('Missing supplemental footprint geometry')
    low=[min(v[i] for v in points) for i in range(3)]
    high=[max(v[i] for v in points) for i in range(3)]
    px,py,pz=pivot;sin,cos=math.sin(math.radians(35)),math.cos(math.radians(35))
    return {'points':[{'x':x-px,'y':(-y+py)*sin,'z_bottom':(low[2]-pz)*cos,'z_top':(high[2]-pz)*cos}
        for x,y in ((low[0],low[1]),(high[0],low[1]),(high[0],high[1]),(low[0],high[1]))],
        'projection_area':[0,0],'opaque':False,'solid':False,'mouse':True,
        'show_shadow_polygon':False,'default_material':0,'material_indices':[]}


def component_editor_footprint(sources, pivot, obstacle):
    """Scope collision metadata to the reviewed component mesh's convex footprint."""
    depsgraph = bpy.context.evaluated_depsgraph_get()
    evaluated = [obj.evaluated_get(depsgraph) for obj in sources]
    vertices = [obj.matrix_world @ vertex.co for obj in evaluated for vertex in obj.data.vertices]
    points = sorted(set((float(v.x), float(v.y)) for v in vertices))
    def cross(a, b, c):
        return (b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])
    lower, upper = [], []
    for point in points:
        while len(lower) >= 2 and cross(lower[-2], lower[-1], point) <= 0:
            lower.pop()
        lower.append(point)
    for point in reversed(points):
        while len(upper) >= 2 and cross(upper[-2], upper[-1], point) <= 0:
            upper.pop()
        upper.append(point)
    hull = lower[:-1] + upper[:-1]
    if len(hull) < 3:
        raise ValueError('Component requires a nondegenerate collision footprint')
    px, py, pz = pivot
    sin, cos = math.sin(math.radians(35)), math.cos(math.radians(35))
    bottom, top = min(v.z for v in vertices), max(v.z for v in vertices)
    result = json.loads(json.dumps(obstacle))
    result['points'] = [{'x': x-px, 'y': (-y+py)*sin,
                         'z_bottom': (bottom-pz)*cos, 'z_top': (top-pz)*cos} for x, y in hull]
    return result


def export_editor(map_name, output_path, asset_id=None, *, standalone_pivot=None,
                  include_hidden_objects=None, catalog=None, level=None, map_settings=None):
    """Evaluate explicitly retained hidden parts without enabling their render visibility."""
    working = bpy.data.collections[map_name + ' Working']
    names = include_hidden_objects or []
    evaluated = {obj: obj.hide_viewport for obj in working.objects if obj.name in names}
    try:
        for obj in evaluated:
            obj.hide_viewport = False
        bpy.context.view_layer.update()
        return _export_editor(map_name, output_path, asset_id, standalone_pivot=standalone_pivot,
                              include_hidden_objects=include_hidden_objects, catalog=catalog, level=level, map_settings=map_settings)
    finally:
        for obj, hidden in evaluated.items():
            obj.hide_viewport = hidden
        bpy.context.view_layer.update()


def _export_editor(map_name, output_path, asset_id=None, *, standalone_pivot=None,
                   include_hidden_objects=None, catalog=None, level=None, map_settings=None):
    """Export visible meshes plus explicitly named inactive reviewed components.

    ``include_hidden_objects`` contains exact object names, never source-node
    selectors. Retained hidden originals stay excluded unless named explicitly.
    ``standalone_pivot`` is an optional finite scene-space XYZ anchor for an
    asset export; callers can use one reviewed pivot for every state variant.
    Hidden sources are exported as geometry with ``default_hidden`` metadata;
    downstream document creation must apply that metadata to part visibility.
    """
    working = bpy.data.collections[map_name + " Working"]
    output = Path(output_path)
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.exists():
        raise FileExistsError(output)
    if asset_id is None and (not output.name.endswith('.rhlos-map.json') or level is None):
        raise ValueError('Map export requires a .rhlos-map.json output and source level data')
    export_directory = tempfile.TemporaryDirectory(prefix='map-export-') if asset_id is None else None
    export_path = Path(export_directory.name) / 'map.gltf' if export_directory else output
    previous_scene = bpy.context.window.scene
    bpy.context.view_layer.update()
    depsgraph = bpy.context.evaluated_depsgraph_get()
    requested = [] if include_hidden_objects is None else include_hidden_objects
    if not isinstance(requested, (list, tuple)) or any(not isinstance(name, str) or not name for name in requested):
        raise ValueError("Inactive inclusion requires a list of exact object names")
    if len(set(requested)) != len(requested):
        raise ValueError("Duplicate inactive object name")
    named = {obj.name: obj for obj in working.objects}
    for name in requested:
        obj = named.get(name)
        if obj is None or obj.type != 'MESH':
            raise ValueError(f"Requested inactive mesh is absent from working collection: {name}")
        if asset_id and obj.get('asset_group') != asset_id:
            raise ValueError(f"Requested inactive mesh belongs to another asset: {name}")
    sources = [o for o in working.objects if o.type == "MESH" and
               (not o.hide_render or o.name in requested)]
    if asset_id:
        sources = [o for o in sources if o.get("asset_group") == asset_id]
    if not sources or any(not o.get("source_node") for o in sources):
        raise ValueError("Every exported mesh must retain its editor source node")
    declared = publication_parts(catalog) if catalog is not None else {}
    part_keys = validate_export_records(catalog, [dict(name=o.name, source_node=o['source_node'],
        asset_group=o.get('asset_group'), projection_component=o.get('projection_component')) for o in sources],
        [asset_id] if asset_id else None) if catalog is not None else {o.name:o['source_node'] for o in sources}
    part_keys.update({o.name:'ground' for o in sources if o['source_node']=='ground'})
    if any(value['source_components'] for value in declared.values()) and level is None:
        raise ValueError('Partitioned publication requires original obstacle metadata')
    visibility = {}
    ownership = {}
    for source in sources:
        key = part_keys[source.name]
        original_key = source['source_node']
        if not isinstance(key, str):
            raise ValueError('Source node must be a stable string')
        if key != 'ground':
            if key.startswith('mission-'):
                source_for_part({'node':key,'mission_profile':source.get('mission_patch_profile')})
                if source.get('source_obstacle') is not None:
                    raise ValueError('Mission part must not alias a sight obstacle: '+key)
            elif is_scenery_node(key):
                source_for_part({'node':key})
                if (source.get('source_obstacle') is not None or source.get('mission_patch_profile') is not None
                        or source.get('obstacle_local_game') is not None):
                    raise ValueError('Scenery part must not claim an obstacle or mission profile: '+key)
            elif not original_key.startswith('building-') or not original_key[9:].isdigit():
                raise ValueError('Invalid canonical editor source node: '+key)
            if any(not isinstance(source.get(field), str) or not source[field].strip()
                   for field in ('asset_group', 'asset_name', 'part_name')):
                raise ValueError('Exported component lacks explicit asset/source ownership: '+source.name)
            if key in ownership and ownership[key] != source['asset_group']:
                raise ValueError('Split asset ownership for '+key)
            ownership[key] = source['asset_group']
        hidden = bool(source.hide_render)
        if key in visibility and visibility[key] != hidden:
            raise ValueError('Mixed default visibility within canonical part '+key)
        visibility[key] = hidden
    explicit_pivot = None
    if standalone_pivot is not None:
        if not asset_id:
            raise ValueError('An explicit standalone pivot requires asset_id')
        try:
            values = list(standalone_pivot)
        except TypeError as error:
            raise ValueError('Standalone pivot must contain three finite numbers') from error
        if len(values) != 3 or any(isinstance(v, bool) or not isinstance(v, (int, float))
                                   or not math.isfinite(v) for v in values):
            raise ValueError('Standalone pivot must contain three finite numbers')
        explicit_pivot = Vector(values)
    reveal = reveal_metadata(working, sources, include_all=asset_id is None)
    bounds = [o.matrix_world @ Vector(corner) for o in sources for corner in o.evaluated_get(depsgraph).bound_box]
    lo = Vector(tuple(min(p[i] for p in bounds) for i in range(3)))
    hi = Vector(tuple(max(p[i] for p in bounds) for i in range(3)))
    pivot = (explicit_pivot if explicit_pivot is not None else
             Vector(((lo.x + hi.x) / 2, (lo.y + hi.y) / 2, lo.z)) if asset_id else Vector())
    scene = bpy.data.scenes.new(map_name + " Editor Export")
    objects, meshes, foliage_materials = [], [], []

    def node(name, parent=None, mesh=None):
        obj = bpy.data.objects.new("Export / " + name, mesh)
        scene.collection.objects.link(obj)
        obj.parent = parent
        obj["editor_node_name"] = name
        objects.append(obj)
        return obj

    try:
        root = node("map")
        # The editor reads Z-up part meshes below a glTF Y-up map wrapper.
        root.rotation_euler.x = -math.pi / 2
        root['default_hidden_source_nodes'] = sorted(key for key, hidden in visibility.items() if hidden)
        groups, parts = {}, {}
        for source in sources:
            key = part_keys[source.name]
            original_key = source['source_node']
            mesh = bpy.data.meshes.new_from_object(source.evaluated_get(depsgraph),
                preserve_all_data_layers=True, depsgraph=depsgraph)
            mesh.transform(Matrix.Translation(-pivot) @ source.matrix_world)
            mesh.update()
            meshes.append(mesh)
            for slot, material in enumerate(mesh.materials):
                if material and material.get("foliage_physical_opacity") is True:
                    temporary = export_foliage_material(material, mesh)
                    foliage_materials.append(temporary)
                    mesh.materials[slot] = temporary
            if key == "ground":
                ground = node("ground", root, mesh)
                ground['default_hidden'] = visibility[key]
                continue
            group_id = source["asset_group"]
            if group_id not in groups:
                group = node(source["asset_name"], root)
                group["asset_group"] = group_id
                if asset_id is None and source.parent and 'asset_origin_scene' in source.parent:
                    values = list(source.parent['asset_origin_scene'])
                    if len(values) != 3 or any(not math.isfinite(v) for v in values):
                        raise ValueError('Invalid authored asset origin: '+group_id)
                    group['asset_origin_scene'] = values
                groups[group_id] = group
            if key not in parts:
                part = node(key, groups[group_id])
                if key.startswith('mission-'):
                    part['mission_patch_profile']=source['mission_patch_profile']
                    if asset_id is None and source.get('native_patch_preview'):
                        part['native_patch_preview']=source['native_patch_preview'].to_dict()
                    part['obstacle_local_game']=mission_editor_footprint([o for o in sources if o['source_node']==key],pivot)
                elif is_scenery_node(key):
                    # Visual only: the game has no obstacle, so none is exported.
                    part['scenery']=True
                else:
                    part["source_obstacle"] = int(original_key.split("-")[1])
                part['source_node'] = original_key
                selectors = declared.get(key, {}).get('source_components', [])
                if selectors:
                    part['source_components'] = selectors
                    part['obstacle_local_game'] = component_editor_footprint(
                        [o for o in sources if part_keys[o.name] == key], pivot,
                        level['sight_obstacles'][part['source_obstacle']])
                    part['footprint_basis'] = 'Reviewed component mesh convex footprint; original obstacle flags retained.'
                part["part_name"] = source["part_name"]
                part['default_hidden'] = visibility[key]
                parts[key] = part
            elif parts[key].parent != groups[group_id]:
                raise ValueError(f"Split asset ownership for {key}")
            elif key.startswith('mission-') and parts[key]['mission_patch_profile']!=source['mission_patch_profile']:
                raise ValueError('Mixed mission profiles in one supplemental part: '+key)
            for state, variant_mesh, state_metadata in export_states(source, mesh):
                if variant_mesh != mesh:
                    meshes.append(variant_mesh)
                piece = node(source.name if state == 'default' else source.name + ' / ' + state, parts[key], variant_mesh)
                piece["source_node"] = original_key
                piece['editor_part_node'] = key
                piece['default_hidden'] = bool(source.hide_render)
                for metadata_key, value in projection_metadata(source).items():
                    piece[metadata_key] = value
                for metadata_key, value in state_metadata.items():
                    piece[metadata_key] = value
        bpy.context.window.scene = scene
        bpy.context.view_layer.update()
        with safe_external_image_names():
            bpy.ops.export_scene.gltf(filepath=str(export_path), export_format="GLTF_SEPARATE" if export_directory else "GLB",
                use_active_scene=True, export_yup=False, export_extras=True,
                export_animations=False, export_cameras=False, export_lights=False,
                export_image_format="AUTO")
        # Blender names are globally unique, even across scenes. Strip only our
        # export aliases in the JSON chunk; binary accessor offsets stay intact.
        if export_directory:
            doc = json.loads(export_path.read_text())
        else:
            data = output.read_bytes()
            length, kind = struct.unpack_from("<II", data, 12)
            if kind != 0x4E4F534A:
                raise ValueError("Expected a GLB JSON chunk")
            doc = json.loads(data[20:20 + length])
        for item in doc["nodes"]:
            extras = item.get("extras", {})
            if "editor_node_name" in extras:
                item["name"] = extras.pop("editor_node_name")
            if item.get("name") == "map" and reveal:
                item.setdefault("extras", {})["reveal"] = reveal
        enforce_foliage_contract(doc)
        compact_texture_coordinates(doc)
        if asset_id:
            sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
            from unify_map_assets import local_states
            doc = local_states(doc)
        manifest = None
        if export_directory:
            export_path.write_text(json.dumps(doc))
            sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
            from scene_manifest import export_document
            settings = map_settings or {}
            manifest = export_document(export_path, output, map_name, level,
                                       size=settings.get('size'), camera=settings.get('camera'),
                                       export_bounds=settings.get('exportBounds'))
        else:
            chunk = json.dumps(doc, separators=(",", ":")).encode()
            chunk += b" " * (-len(chunk) % 4)
            binary = data[20 + length:]
            output.write_bytes(struct.pack("<4sII", b"glTF", 2, 20 + len(chunk) + len(binary)) + struct.pack("<II", len(chunk), kind) + chunk + binary)
        report = {"file": str(output), "assets": len(groups), "parts": len(parts),
                  "meshes": len(meshes), "steps": sum(o.get("step_count", 0) for o in sources),
                  "included_hidden_objects": [o.name for o in sources if o.hide_render],
                  "default_hidden_source_nodes": sorted(key for key, hidden in visibility.items() if hidden)}
        if manifest:
            report["library"] = manifest["library"]
            report["verified_assets"] = manifest["report"]["verified_assets"]
        if asset_id:
            descriptor = {"version": 1, "kind": "projection-mapped-asset", "id": asset_id,
                "name": sources[0]["asset_name"], "source_map": map_name, "model": output.name,
                "coordinates": "Z-up mesh children; Y-up glTF map wrapper; units are map pixels",
                "anchor": "explicit common scene-space pivot" if explicit_pivot is not None else "horizontal bounds center at lowest geometry point",
                "bounds_local_scene": {"min": list(lo - pivot), "max": list(hi - pivot)},
                "components": [{"name": source.name, "source_node": source["source_node"],
                                "editor_part_node": part_keys[source.name],
                                "default_hidden": bool(source.hide_render), **projection_metadata(source)} for source in sources],
                "parts": [{"node": key, "name": obj["part_name"],
                           **({'mission_profile':obj['mission_patch_profile'],
                               'obstacle_local_game':obj['obstacle_local_game'].to_dict(),
                               'footprint_basis':'Editor bounds only; no sight-obstacle association or animation inferred.'}
                              if key.startswith('mission-') else {'scenery':True} if is_scenery_node(key)
                              else {'source_obstacle':obj['source_obstacle']}),
                           **({'source_node':obj['source_node'], 'source_components':list(obj['source_components']),
                               'obstacle_local_game':obj['obstacle_local_game'].to_dict(),
                               'footprint_basis':obj['footprint_basis']} if obj.get('source_components') else {}),
                           "default_hidden": visibility[key]} for key, obj in parts.items()]}
            if {source['source_node'] for source in sources} == {'ground'}:
                descriptor['editor_usage'] = 'map-background'
            from unify_map_assets import local_descriptor
            descriptor = local_descriptor(descriptor)
            output.with_name("asset.json").write_text(json.dumps(descriptor, indent=2) + "\n")
            report["asset"] = descriptor
            report["placement_origin_scene"] = list(pivot)
        return report
    finally:
        bpy.context.window.scene = previous_scene
        for obj in objects:
            bpy.data.objects.remove(obj, do_unlink=True)
        for mesh in meshes:
            bpy.data.meshes.remove(mesh)
        for material in foliage_materials:
            bpy.data.materials.remove(material)
        bpy.data.scenes.remove(scene)
        if export_directory:
            export_directory.cleanup()


def export_asset_library(map_name, output_dir, level_path, *, standalone_pivots=None,
                         include_hidden_objects=None, asset_ids=None, catalog=None):
    """Export every named asset, local collision volumes, and merge the library index.

    Run into a fresh staging directory for each revision, then publish reviewed
    asset directories. Other maps in an existing index remain intact.
    Optional ``standalone_pivots`` maps exported asset IDs to common scene-space
    XYZ anchors. Exact inactive object names are validated before any export and
    forwarded only to their owning asset, including groups with no visible mesh.
    """
    output_dir = Path(output_dir)
    level = json.loads(Path(level_path).read_text())
    working = bpy.data.collections[map_name + " Working"]
    requested = [] if include_hidden_objects is None else include_hidden_objects
    if not isinstance(requested, (list, tuple)) or any(not isinstance(name, str) or not name for name in requested):
        raise ValueError('Inactive inclusion requires a list of exact object names')
    if len(set(requested)) != len(requested):
        raise ValueError('Duplicate inactive object name')
    named = {obj.name: obj for obj in working.objects}
    for name in requested:
        obj = named.get(name)
        if obj is None or obj.type != 'MESH' or not obj.get('asset_group'):
            raise ValueError('Requested inactive mesh lacks working-map asset ownership: '+name)
        key = obj.get('source_node')
        if is_scenery_node(key):
            continue
        if not isinstance(key, str) or not key.startswith('building-') or not key[9:].isdigit():
            raise ValueError('Requested inactive mesh lacks canonical source ownership: '+name)
        if int(key[9:]) >= len(level['sight_obstacles']):
            raise ValueError('Requested inactive mesh source is absent from level catalog: '+name)
    ids = sorted({o["asset_group"] for o in working.objects if o.type == "MESH" and
                  (not o.hide_render or o.name in requested) and o.get("asset_group")})
    if asset_ids is not None:
        if (not isinstance(asset_ids, (list, tuple)) or not asset_ids
                or any(not isinstance(key, str) or not key for key in asset_ids)
                or len(set(asset_ids)) != len(asset_ids)):
            raise ValueError('Asset subset requires unique nonempty asset IDs')
        if set(asset_ids) - set(ids):
            raise ValueError('Asset subset references unknown or inactive asset groups')
        if any(named[name]['asset_group'] not in asset_ids for name in requested):
            raise ValueError('Inactive mesh request belongs to an unselected asset')
        ids = sorted(asset_ids)
    pivots = {} if standalone_pivots is None else standalone_pivots
    if not isinstance(pivots, dict) or set(pivots) - set(ids):
        raise ValueError('Standalone pivots reference unexported asset groups')
    for key, values in pivots.items():
        if not isinstance(values, (list, tuple, Vector)) or len(values) != 3 or any(
                isinstance(v, bool) or not isinstance(v, (int, float)) or not math.isfinite(v) for v in values):
            raise ValueError('Standalone pivot must contain three finite numbers: '+key)
    if not ids:
        raise ValueError("No named assets to export")
    if any((output_dir / key / "model.glb").exists() for key in ids):
        raise FileExistsError("Asset output exists; use a fresh staging directory")
    for key in ids:
        report = export_editor(map_name, output_dir / key / "model.glb", asset_id=key,
            standalone_pivot=pivots.get(key),
            include_hidden_objects=[name for name in requested if named[name]['asset_group'] == key], catalog=catalog, level=level)
        descriptor = report["asset"]
        px, py, pz = report["placement_origin_scene"]
        sin, cos = math.sin(math.radians(35)), math.cos(math.radians(35))
        descriptor["source_origin_scene"] = [px, py, pz]
        for part in descriptor["parts"]:
            if 'mission_profile' in part or part.get('source_components'):
                # export_editor has already applied the common variant pivot.
                continue
            elif part.get('scenery'):
                # Authored scenery has no sight obstacle to localize.
                continue
            else:
                obstacle = json.loads(json.dumps(level["sight_obstacles"][part["source_obstacle"]]))
            for point in obstacle["points"]:
                point["x"] -= px
                point["y"] += py * sin
                point["z_bottom"] -= pz * cos
                point["z_top"] -= pz * cos
            part["obstacle_local_game"] = obstacle
        (output_dir / key / "asset.json").write_text(json.dumps(descriptor, indent=2) + "\n")
    write_asset_index(output_dir)
    return {"assets": len(ids), "index": str(output_dir / "index.json")}


def exported_pivot(directory, asset_id):
    """Read the pivot pinned to this exported asset revision."""
    directory = Path(directory)
    index = json.loads((directory/'index.json').read_text())
    entry = next(entry for entry in index['assets'] if entry['id'] == asset_id)
    return json.loads((directory/entry['descriptor']).read_text())['source_origin_scene']
