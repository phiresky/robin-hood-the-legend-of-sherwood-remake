"""Lossy browser models (`lossy.glb`) derived from published asset GLBs (never edits originals).

Publication derives them automatically: `stage_reviewed_publication.py` calls
`refresh_derivatives` on its staged catalog (disable with `--no-lossy` or plan `"lossy": false`).
The same implementation serves these commands (repository root, Blender background):

    blender --background --threads 2 --python-exit-code 1 \
      --python level-editor/refinement/blender/lossy_assets.py -- <command> ...

    library  --root level-editor/library/3d-assets --run <work/...> [--maps ...] [--apply]
             Backfill a live library: dry-run plan by default; --apply derives each stale model,
             then (under the library's `.publication.lock`) writes `<dir>/lossy.glb` + receipt
             and sets `lossy_model`, recording backups for `rollback --run <run>`. Resumable.
    refresh  --root <staged 3d-assets> --work <dir> [--no-lossy] [--no-previews]
             What publication runs: update a staged catalog's lossy models and previews.
    derive   --source-root <3d-assets> --assets ... --output <work/...>
             Scratch comparison outputs only (`index.lossy.json` maps model paths).
    export-worker --worker <blend> --worker-sha256 <sha> --assets ... --output <work/...>
             Development: export worker assets as publication does, then `derive` them.

Placement mirrors the index: `<dir>/model.glb` gets `<dir>/lossy.glb` and
`<dir>/lossy.glb.receipt.json` (`source` = model SHA-256, `output` = lossy SHA-256, settings,
normals policy, tool versions); other `<dir>/X.glb` get `<dir>/X.lossy.glb`.

Each lossy model is derived from the exact published GLB bytes. Nodes, extras, scenes,
materials and vertex attributes are copied, except texture coordinates (textured primitives
receive one shared atlas) and normals of unlit textured primitives: nothing is lit yet, and
lossy assets get rebuilt (with shadows unbaked) once lighting exists (`--keep-normals` keeps
them). Vertices are quantized with KHR_mesh_quantization (`--no-quantize` keeps floats):

1. Every textured glTF mesh becomes a Blender mesh (positions welded for topology only, each loop
   still maps to its glTF vertex). Smart UV Project (66 degrees) finds islands over all meshes;
   islands are normalized to a common surface density, then packed without relaxing them.
2. The atlas is square, a multiple of `--multiple` texels, sized so `--density-coverage` (95%)
   of the surface meets its weakest-direction target: `--density` texels per map pixel
   (the source artwork scale), capped by the source's strongest direction to retain its detail.
   Packed layouts are fitted uniformly into one tile. Collapsed textured triangles get
   separate small charts and are repacked. If the required atlas exceeds `--max-size`,
   or packing remains unsafe, the published layout and resolution are retained and re-encoded.
   Physical-opacity assets (foliage), tiled textures, and an already-filled single-image atlas
   also retain their original layouts. A rebaked atlas may not exceed the source image
   pixel count by more than `--max-atlas-expansion` (4 by default): even BC7 compression
   cannot offset a larger expansion against uncompressed source textures.
   `--min-size` applies to successfully repacked atlases.
3. Cycles EMIT bakes the original textures through their original UVs into the atlas (one joined
   temporary object, so dilation cannot overwrite another mesh's texels). Alpha is baked only
   when a source material is MASK/BLEND; opaque assets ship RGB (the published ownership alpha is
   provenance, not display data).
4. avifenc (lossy, `--quality`) encodes the atlas; the GLB references it through a required
   EXT_texture_avif (three.js GLTFLoader decodes it natively; no fallback image).
5. The written GLB is checked structurally (same meshes, primitives and triangle counts); the
   receipts bind it by hash. `--validate` additionally renders the published and the lossy GLB
   (lossy texture decoded with avifdec) from eight oblique orthographic views at
   `--render-scale` pixels per map pixel and reports mean/p95/max colour differences.
"""
import argparse
import copy
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import uuid

import numpy as np

HERE = Path(__file__).resolve().parent
LEVEL_EDITOR = HERE.parents[1]
ROOT = LEVEL_EDITOR / 'work'  # scratch outputs and run records stay inside the work tree
PIPELINE = LEVEL_EDITOR / 'pipeline'
RENDER_SLOTS = LEVEL_EDITOR / 'refinement'  # render_slots.py: machine-wide Blender render slot pool
sys.path.insert(0, str(RENDER_SLOTS))
from asset_index import write_asset_index, generate_asset_index, discover_asset_index, lossy_problems

SOURCE_UV, NEW_UV = 'Published source', 'Lossy atlas'
COMPONENTS = {5120: np.int8, 5121: np.uint8, 5122: np.int16, 5123: np.uint16, 5125: np.uint32, 5126: np.float32}
WIDTH = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4, 'MAT2': 4, 'MAT3': 9, 'MAT4': 16}
WRAP = {10497: 'REPEAT', 33071: 'EXTEND', 33648: 'MIRROR'}

BACKGROUND = 128  # comparison background gray (0-255) under transparent render pixels


# --- shared bake/measurement helpers ------------------------------------------------------------

def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def require(condition, message):
    if not condition:
        raise ValueError(message)

def weighted_quantile(values, weights, q):
    order = np.argsort(values)
    values, weights = np.asarray(values)[order], np.asarray(weights)[order]
    cumulative = np.cumsum(weights)
    require(cumulative[-1] > 0, 'Weighted quantile of zero total weight')
    return float(values[np.searchsorted(cumulative, q * cumulative[-1])])

def stats(values, weights):
    values = np.asarray(values, dtype=np.float64)
    return {'area_weighted_median': weighted_quantile(values, weights, .5),
            'area_weighted_p10': weighted_quantile(values, weights, .1),
            'area_weighted_p90': weighted_quantile(values, weights, .9),
            'face_median': float(np.median(values)), 'min': float(values.min()), 'max': float(values.max())}

def face_geometry(obj, layer_name):
    """World area and UV area (unit square) of every polygon."""
    mesh, matrix = obj.data, obj.matrix_world
    world = [matrix @ v.co for v in mesh.vertices]
    uv = mesh.uv_layers[layer_name].uv
    areas, uv_areas = [], []
    for poly in mesh.polygons:
        points = [world[i] for i in poly.vertices]
        normal = sum(((points[i] - points[0]).cross(points[i + 1] - points[0])
                      for i in range(1, len(points) - 1)), points[0] * 0)
        areas.append(normal.length / 2)
        coords = [uv[i].vector for i in poly.loop_indices]
        uv_areas.append(abs(sum(coords[i].x * coords[i - 1].y - coords[i - 1].x * coords[i].y
                                for i in range(len(coords)))) / 2)
    return np.array(areas), np.array(uv_areas)

def face_axes(obj, layer_name, width, height):
    """Per-face (min, max) texels per world unit along the principal axes of the UV map.

    Uses the largest fan triangle of each polygon; a stretched layout shows up as min << max.
    """
    mesh, matrix = obj.data, obj.matrix_world
    world = [matrix @ v.co for v in mesh.vertices]
    uv = mesh.uv_layers[layer_name].uv
    rows = []
    for poly in mesh.polygons:
        corners = list(zip(poly.vertices, poly.loop_indices))
        best = max(range(1, len(corners) - 1), key=lambda i: (world[corners[i][0]] - world[corners[0][0]]).cross(
            world[corners[i + 1][0]] - world[corners[0][0]]).length)
        (v0, l0), (v1, l1), (v2, l2) = corners[0], corners[best], corners[best + 1]
        e1, e2 = world[v1] - world[v0], world[v2] - world[v0]
        axis_x = e1.normalized() if e1.length > 0 else e1
        axis_y = e1.cross(e2).cross(e1)
        if e1.length == 0 or axis_y.length == 0:
            rows.append((0.0, 0.0))
            continue
        axis_y.normalize()
        local = np.array([[e1.dot(axis_x), e2.dot(axis_x)], [e1.dot(axis_y), e2.dot(axis_y)]])
        scale = np.array([width, height])
        texel = np.array([(uv[l1].vector - uv[l0].vector)[:], (uv[l2].vector - uv[l0].vector)[:]]).T * scale[:, None]
        singular = np.linalg.svd(texel @ np.linalg.inv(local), compute_uv=False)
        rows.append((float(singular.min()), float(singular.max())))
    return np.array(rows)

def build_material(name, image, layer, interpolation, extension, *, emission_from, target=None):
    """UV Map -> Image -> (Emission) -> Output; optional active bake target node."""
    import bpy
    material = bpy.data.materials.new(name)
    if material.node_tree is None:
        material.use_nodes = True
    tree = material.node_tree
    tree.nodes.clear()
    output = tree.nodes.new('ShaderNodeOutputMaterial')
    uv_node = tree.nodes.new('ShaderNodeUVMap')
    uv_node.uv_map = layer
    texture = tree.nodes.new('ShaderNodeTexImage')
    texture.image, texture.interpolation, texture.extension = image, interpolation, extension
    tree.links.new(uv_node.outputs['UV'], texture.inputs['Vector'])
    if emission_from is None:
        # Same graph shape as the published projection materials (exported as unlit).
        tree.links.new(texture.outputs['Color'], output.inputs['Surface'])
    else:
        emission = tree.nodes.new('ShaderNodeEmission')
        emission.inputs['Strength'].default_value = 1.0
        tree.links.new(texture.outputs[emission_from], emission.inputs['Color'])
        tree.links.new(emission.outputs['Emission'], output.inputs['Surface'])
    if target is not None:
        node = tree.nodes.new('ShaderNodeTexImage')
        node.image = target
        for other in tree.nodes:
            other.select = False
        node.select = True
        tree.nodes.active = node
    return material

def set_slots(obj, materials, face_slots):
    """Replace all slots with `materials` and point every face at its new slot index."""
    mesh = obj.data
    mesh.materials.clear()
    for material in materials:
        mesh.materials.append(material)
    mesh.polygons.foreach_set('material_index', face_slots)
    mesh.update()

def render_views(scene, cameras, out_dir, show, hide):
    import bpy
    out_dir.mkdir(parents=True)
    for obj in hide:
        obj.hide_render = True
    for obj in show:
        obj.hide_render = False
    paths = []
    for index, camera in enumerate(cameras):
        scene.camera = camera
        scene.render.filepath = str(out_dir / f'view-{index}.png')
        bpy.ops.render.render(write_still=True, scene=scene.name)
        paths.append(out_dir / f'view-{index}.png')
    return paths

def composite(path):
    from PIL import Image
    rgba = np.asarray(Image.open(path).convert('RGBA')).astype(np.float64)
    alpha = rgba[..., 3:] / 255
    return rgba[..., :3] * alpha + BACKGROUND * (1 - alpha), rgba[..., 3] > 0

def label(image, text, height=28):
    from PIL import Image, ImageDraw
    canvas = Image.new('RGB', (image.width, image.height + height), (24, 24, 24))
    canvas.paste(image, (0, height))
    ImageDraw.Draw(canvas).text((6, 7), text, fill=(235, 235, 235))
    return canvas

def compare(original_paths, unwrapped_paths, output, labels=('original (per-face source atlases)', 'unwrapped (Smart UV rebake)')):
    """Per-view difference statistics and a labelled original/unwrapped/diff sheet."""
    from PIL import Image
    rows, per_view = [[], [], []], []
    for index, (before_path, after_path) in enumerate(zip(original_paths, unwrapped_paths)):
        before, covered_before = composite(before_path)
        after, covered_after = composite(after_path)
        covered = covered_before | covered_after
        require(covered.any(), f'View {index} renders nothing')
        delta = np.abs(before - after)
        peak = delta.max(axis=2)[covered]
        per_view.append({'view': index, 'covered_pixels': int(covered.sum()),
                         'silhouette_mismatch_pixels': int((covered_before != covered_after).sum()),
                         'mean': float(peak.mean()), 'mean_rgb': float(delta[covered].mean()),
                         'p95': float(np.percentile(peak, 95)), 'max': float(peak.max()),
                         'fraction_over_8': float((peak > 8).mean()), 'fraction_over_24': float((peak > 24).mean())})
        heat = np.zeros(before.shape, dtype=np.float64) + 16
        heat[covered] = np.minimum(255, delta[covered] * 4)
        stats_text = f"v{index} mean {per_view[-1]['mean']:.2f} p95 {per_view[-1]['p95']:.1f} max {per_view[-1]['max']:.0f}"
        for row, pixels, text in ((0, before, f'v{index} {labels[0]}'),
                                  (1, after, f'v{index} {labels[1]}'),
                                  (2, heat, stats_text + ' | |diff| x4')):
            rows[row].append(label(Image.fromarray(pixels.round().astype(np.uint8)), text))
    tile_w, tile_h = rows[0][0].size
    half = (len(original_paths) + 1) // 2
    sheet = Image.new('RGB', (tile_w * half, tile_h * 6), (24, 24, 24))
    for block in range(2):
        for row in range(3):
            for column, tile in enumerate(rows[row][block * half:(block + 1) * half]):
                sheet.paste(tile, (column * tile_w, (block * 3 + row) * tile_h))
    sheet.save(output)
    return {'per_view': per_view,
            'overall': {'mean_of_view_means': float(np.mean([v['mean'] for v in per_view])),
                        'max_p95': float(max(v['p95'] for v in per_view)),
                        'max': float(max(v['max'] for v in per_view))},
            'metric': 'Per pixel max |channel| difference (0-255) after compositing both renders over '
                      f'gray {BACKGROUND}; statistics over the union of both silhouettes.'}


# --- GLB access -------------------------------------------------------------------------------

def read_glb(path):
    """(document, buffer byte strings, GLB bytes). External buffers (shared library blobs)
    resolve relative to the GLB, like the loader's resource pins."""
    data = Path(path).read_bytes()
    magic, version, length = struct.unpack_from('<4sII', data, 0)
    require(magic == b'glTF' and version == 2 and length == len(data), f'Not a glTF 2 GLB: {path}')
    offset, doc, binary = 12, None, None
    while offset < length:
        size, kind = struct.unpack_from('<II', data, offset)
        chunk = data[offset + 8:offset + 8 + size]
        if kind == 0x4E4F534A:
            doc = json.loads(chunk)
        elif kind == 0x004E4942:
            binary = chunk
        offset += 8 + size
    require(doc is not None, f'GLB without JSON: {path}')
    buffers = []
    for index, buffer in enumerate(doc.get('buffers', [])):
        if 'uri' in buffer:
            require(not buffer['uri'].startswith('data:'), f'Data URI buffers are not supported: {path}')
            buffers.append((Path(path).parent / buffer['uri']).resolve(strict=True).read_bytes())
        else:
            require(index == 0 and binary is not None, f'Buffer {index} has no data: {path}')
            buffers.append(binary)
    for key in ('animations', 'skins'):
        require(not doc.get(key), f'{key} are not supported: {path}')
    return doc, buffers, data


def accessor_array(doc, binary, index, dequantize=False):
    accessor = doc['accessors'][index]
    require('sparse' not in accessor and 'bufferView' in accessor, f'Sparse/empty accessor {index}')
    view = doc['bufferViews'][accessor['bufferView']]
    dtype = np.dtype(COMPONENTS[accessor['componentType']])
    width = WIDTH[accessor['type']]
    start = view.get('byteOffset', 0) + accessor.get('byteOffset', 0)
    stride = view.get('byteStride') or dtype.itemsize * width
    count = accessor['count']
    raw = np.ndarray((count, width), dtype=dtype, buffer=binary[view.get('buffer', 0)], offset=start, strides=(stride, dtype.itemsize)).copy()
    if dequantize and accessor.get('normalized'):
        # glTF normalized integers: unsigned v / max, signed max(v / max, -1).
        limit = float(np.iinfo(dtype).max)
        raw = np.maximum(raw.astype(np.float64) / limit, -1.0)
    elif dequantize:
        raw = raw.astype(np.float64)
    return raw if width > 1 else raw[:, 0]


def node_matrix(node):
    if 'matrix' in node:
        return np.array(node['matrix'], dtype=np.float64).reshape(4, 4).T
    t = node.get('translation', [0, 0, 0])
    x, y, z, w = node.get('rotation', [0, 0, 0, 1])
    s = node.get('scale', [1, 1, 1])
    rotation = np.array([[1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
                         [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
                         [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)]])
    matrix = np.eye(4)
    matrix[:3, :3] = rotation * np.array(s)[None, :]
    matrix[:3, 3] = t
    return matrix


def mesh_instances(doc):
    """First Z-up map-space matrix of every mesh referenced from any scene."""
    found = {}

    def visit(index, parent):
        node = doc['nodes'][index]
        # The editor's `map` wrapper only turns Z-up map space into glTF Y-up; dropping it keeps
        # every file (wrapped assets and unwrapped map backgrounds) in Z-up map space.
        world = parent if node.get('name') == 'map' and 'mesh' not in node else parent @ node_matrix(node)
        if 'mesh' in node:
            found.setdefault(node['mesh'], world)
        for child in node.get('children', []):
            visit(child, world)

    for scene in doc.get('scenes', []):
        for root in scene.get('nodes', []):
            visit(root, np.eye(4))
    return found


def has_pbr_maps(doc):
    """Multi-map materials must retain their UV layout and independent texture channels."""
    return any('normalTexture' in m or 'occlusionTexture' in m
               or 'metallicRoughnessTexture' in m.get('pbrMetallicRoughness', {})
               for m in doc.get('materials', []))


def display_texture(doc, primitive):
    """(texture info, image index, sampler) of the texture a primitive displays, or None.

    Published materials show one image: as base colour (unlit, or foliage whose emissive
    texture is the same texture), or as emissive over a black base colour (lit background
    planes). Anything else is refused rather than approximated.
    """
    if 'material' not in primitive:
        return None
    material = doc['materials'][primitive['material']]
    name = material.get('name')
    for key in ('normalTexture', 'occlusionTexture'):
        require(key not in material, f'Unsupported {key} in material {name}')
    pbr = material.get('pbrMetallicRoughness', {})
    require('metallicRoughnessTexture' not in pbr, f'Unsupported metallicRoughnessTexture: {name}')
    base, emissive = pbr.get('baseColorTexture'), material.get('emissiveTexture')
    if base is None and emissive is None:
        return None
    factor = pbr.get('baseColorFactor', [1, 1, 1, 1])
    if emissive is not None:
        require(material.get('emissiveFactor') == [1, 1, 1], f'Emissive texture needs emissiveFactor 1: {name}')
    if base is not None:
        require(all(abs(v - 1) < 1e-9 for v in factor), f'baseColorFactor with texture is not supported: {name}')
        require(emissive is None or (emissive['index'] == base['index'] and emissive.get('texCoord', 0) == base.get('texCoord', 0)),
                f'Different base colour and emissive textures: {name}')
    else:
        require(all(abs(v) < 1e-9 for v in factor[:3]), f'Emissive-only texture over a non-black base colour: {name}')
    info = base if base is not None else emissive
    require(not info.get('extensions'), f'Texture transforms are not supported: {name}')
    texture = doc['textures'][info['index']]
    sampler = doc['samplers'][texture['sampler']] if 'sampler' in texture else {}
    return info, avif_source(texture), sampler


# --- Blender scene from a GLB --------------------------------------------------------------------

def load_images(doc, binary, directory, base=None, decode_avif=False):
    """Decode every image; `uri` images (shared library blobs) resolve against `base`."""
    import bpy
    directory.mkdir(parents=True, exist_ok=True)
    images = {}
    decoded_images = {}
    physical_alpha = {found[1] for mesh in doc.get('meshes', []) for primitive in mesh['primitives']
                      if (found := display_texture(doc, primitive))
                      and doc['materials'][primitive['material']].get('alphaMode', 'OPAQUE') != 'OPAQUE'}
    for index, image in enumerate(doc.get('images', [])):
        if 'uri' in image:
            require(base is not None and not image['uri'].startswith('data:'), f'Unsupported image uri: {image["uri"]}')
            data = (base / image['uri']).resolve(strict=True).read_bytes()
        else:
            view = doc['bufferViews'][image['bufferView']]
            chunk = binary[view.get('buffer', 0)]
            data = chunk[view.get('byteOffset', 0):view.get('byteOffset', 0) + view['byteLength']]
        extension = {'image/png': 'png', 'image/jpeg': 'jpg', 'image/avif': 'avif'}[image['mimeType']]
        key = (hashlib.sha256(data).digest(), index in physical_alpha)
        if key in decoded_images:
            images[index] = decoded_images[key]
            continue
        path = directory / f'image-{index}.{extension}'
        path.write_bytes(data)
        if extension == 'avif':
            require(decode_avif, 'AVIF source images are not expected')
            decoded = directory / f'image-{index}.decoded.png'
            subprocess.run(['avifdec', str(path), str(decoded)], check=True, capture_output=True)
            path = decoded
        loaded = bpy.data.images.load(str(path))
        # Opaque atlases can use alpha for source ownership. Cycles must not
        # premultiply away their synthesized RGB while sampling for the bake.
        loaded.alpha_mode = 'STRAIGHT' if index in physical_alpha else 'NONE'
        images[index] = loaded
        decoded_images[key] = loaded
    return images


def avif_source(texture):
    extensions = texture.get('extensions', {})
    require(set(extensions) <= {'EXT_texture_avif'}, f'Unsupported texture extensions: {sorted(extensions)}')
    if 'EXT_texture_avif' in extensions:
        return extensions['EXT_texture_avif']['source']
    return texture['source']


def build_objects(doc, binary, images, collection, label):
    """One Blender object per textured glTF mesh, plus per-object corner records for the writer."""
    import bpy
    objects, records = [], []
    instances = mesh_instances(doc)
    # Retained, uninstanced meshes still belong to the document and need valid
    # atlas coordinates if an editor later attaches them to a scene node.
    for mesh_index in range(len(doc.get('meshes', []))):
        world = instances.get(mesh_index, np.eye(4))
        mesh_doc = doc['meshes'][mesh_index]
        transform = world
        positions, faces, corners, uvs, slots, materials = [], [], [], [], [], []
        weld = {}
        for prim_index, primitive in enumerate(mesh_doc['primitives']):
            require(primitive.get('mode', 4) == 4, f'Only triangle primitives are supported (mesh {mesh_index})')
            require(not primitive.get('targets'), f'Morph targets are not supported (mesh {mesh_index})')
            found = display_texture(doc, primitive)
            if found is None:
                continue
            info, image_index, sampler = found
            attributes = primitive['attributes']
            position = accessor_array(doc, binary, attributes['POSITION'], dequantize=True)
            uv = accessor_array(doc, binary, attributes[f'TEXCOORD_{info.get("texCoord", 0)}'], dequantize=True)
            count = len(position)
            indices = accessor_array(doc, binary, primitive['indices']) if 'indices' in primitive else np.arange(count)
            world_position = (np.c_[position, np.ones(count)] @ transform.T)[:, :3]
            keys = [tuple(np.round(p, 5)) for p in world_position]
            slot = len(materials)
            materials.append((images[image_index], 'Closest' if sampler.get('magFilter') == 9728 else 'Linear',
                              WRAP[sampler.get('wrapS', 10497)], primitive.get('material')))
            for triangle in indices.reshape(-1, 3):
                verts = []
                for corner in triangle:
                    key = keys[corner]
                    if key not in weld:
                        weld[key] = len(positions)
                        positions.append(world_position[corner])
                    verts.append(weld[key])
                if len(set(verts)) < 3:
                    # Degenerate after welding: keep it with private vertices so it still maps 1:1.
                    verts = []
                    for corner in triangle:
                        verts.append(len(positions))
                        positions.append(world_position[corner])
                faces.append(verts)
                corners.append([(prim_index, int(c)) for c in triangle])
                uvs.append([(float(uv[c][0]), 1.0 - float(uv[c][1])) for c in triangle])
                slots.append(slot)
        if not faces:
            continue
        name = f'{label} / mesh {mesh_index} / {mesh_doc.get("name", "")}'
        mesh = bpy.data.meshes.new(name)
        mesh.from_pydata([tuple(p) for p in positions], [], faces)
        require(len(mesh.polygons) == len(faces), f'Blender merged or dropped faces of mesh {mesh_index}')
        layer = mesh.uv_layers.new(name=SOURCE_UV)
        layer.uv.foreach_set('vector', np.array(uvs, dtype=np.float32).ravel())
        obj = bpy.data.objects.new(name, mesh)
        collection.objects.link(obj)
        obj['gltf_mesh'] = mesh_index
        render_materials = [build_material(f'{label} / {mesh_index} / {slot}', image, SOURCE_UV, interpolation,
                                           extension, emission_from='Color')
                            for slot, (image, interpolation, extension, _) in enumerate(materials)]
        set_slots(obj, render_materials, np.array(slots, dtype=np.int32))
        objects.append(obj)
        records.append({'object': obj, 'mesh': mesh_index, 'corners': corners, 'materials': materials})
    return objects, records


# --- unwrap, bake, encode ----------------------------------------------------------------------

class UnsafeAtlasError(ValueError):
    """Packing returned coordinates that cannot be baked into one atlas."""


def check_atlas_uvs(source, packed):
    """Reject lost UV triangles even when Blender reports a successful pack."""
    source = np.asarray(source, dtype=np.float64).reshape(-1, 3, 2)
    packed = np.asarray(packed, dtype=np.float64).reshape(-1, 3, 2)
    if not np.isfinite(packed).all():
        raise UnsafeAtlasError('Packed atlas contains non-finite UVs')

    def twice_area(triangles):
        edges = triangles[:, 1:] - triangles[:, :1]
        return np.abs(edges[:, 0, 0] * edges[:, 1, 1] - edges[:, 0, 1] * edges[:, 1, 0])

    collapsed = (twice_area(source) > 0) & (twice_area(packed) == 0)
    if collapsed.any():
        raise UnsafeAtlasError(f'Packed atlas collapsed {int(collapsed.sum())} previously textured triangles')
    if packed.min() < -1e-6 or packed.max() > 1 + 1e-6:
        raise UnsafeAtlasError('Packed atlas lies outside the single texture tile')


def check_packed_objects(objects):
    for obj in objects:
        arrays = []
        for name in (SOURCE_UV, NEW_UV):
            uv = np.empty(len(obj.data.loops) * 2, dtype=np.float32)
            obj.data.uv_layers[name].uv.foreach_get('vector', uv)
            arrays.append(uv)
        check_atlas_uvs(*arrays)


def rescue_collapsed_charts(source, packed):
    """Give collapsed textured triangles separate charts for another packing attempt.

    Very thin triangles can disappear at float32 UV precision. Baking a small
    independent chart preserves their texture without changing their geometry.
    """
    source = np.asarray(source, dtype=np.float64).reshape(-1, 3, 2)
    result = np.asarray(packed, dtype=np.float32).reshape(-1, 3, 2).copy()
    def area(values):
        edges = values[:, 1:].astype(np.float64) - values[:, :1]
        return np.abs(edges[:, 0, 0] * edges[:, 1, 1] - edges[:, 0, 1] * edges[:, 1, 0])
    collapsed = np.flatnonzero((area(source) > 0) & (area(result) == 0))
    for slot, face in enumerate(collapsed):
        # Distinct coordinates disconnect these charts from adjacent faces.
        result[face] = np.array([[0, 0], [.01, 0], [0, .01]]) + [2 + slot * .02, 2]
    return result.ravel(), len(collapsed)


def rescue_packed_objects(objects):
    count = 0
    for obj in objects:
        arrays = []
        for name in (SOURCE_UV, NEW_UV):
            uv = np.empty(len(obj.data.loops) * 2, dtype=np.float32)
            obj.data.uv_layers[name].uv.foreach_get('vector', uv)
            arrays.append(uv)
        repaired, changed = rescue_collapsed_charts(*arrays)
        if changed:
            obj.data.uv_layers[NEW_UV].uv.foreach_set('vector', repaired)
        count += changed
    return count


def fit_atlas_tile(objects, margin):
    """Fit the entire packed layout into tile 1001 with one uniform transform."""
    arrays = []
    for obj in objects:
        uv = np.empty(len(obj.data.loops) * 2, dtype=np.float32)
        obj.data.uv_layers[NEW_UV].uv.foreach_get('vector', uv)
        arrays.append(uv.reshape(-1, 2))
    values = np.concatenate(arrays).astype(np.float64)
    if not np.isfinite(values).all():
        raise UnsafeAtlasError('Packed atlas contains non-finite UVs')
    low, high = values.min(axis=0), values.max(axis=0)
    if values.min() >= 0 and values.max() <= 1:
        return 1.0
    scale = (1 - 2 * margin) / max(float((high - low).max()), 1e-12)
    for obj, uv in zip(objects, arrays):
        uv = ((uv.astype(np.float64) - low) * scale + margin).astype(np.float32)
        obj.data.uv_layers[NEW_UV].uv.foreach_set('vector', uv.ravel())
    return scale


def choose_size(args, targets, weakest, area):
    """Size meeting target density over the requested fraction of surface area."""
    require(0 < args.density_coverage <= 1, 'density coverage must be in (0, 1]')
    required = weighted_quantile(targets / np.maximum(weakest, 1e-12), area, args.density_coverage)
    size = max(args.min_size, args.multiple * math.ceil(required / args.multiple))
    return min(size, args.max_size), required


def atlas_expansion_exceeded(size, source_sizes, maximum):
    """Avoid spending more pixels on a rebake than its delivery/runtime benefit can justify."""
    require(math.isfinite(maximum) and maximum >= 1, 'max atlas expansion must be finite and >= 1')
    source_pixels = sum(width * height for width, height in source_sizes)
    require(source_pixels > 0, 'Source textures have no pixels')
    return size * size > source_pixels * maximum


def unwrap(objects, args, targets):
    import bpy
    for obj in objects:
        obj.data.uv_layers.active = obj.data.uv_layers.new(name=NEW_UV)
    bpy.ops.object.select_all(action='DESELECT')
    for obj in objects:
        obj.select_set(True)
    bpy.context.view_layer.objects.active = objects[0]
    bpy.ops.object.mode_set(mode='EDIT')
    bpy.ops.mesh.select_all(action='SELECT')
    bpy.ops.uv.select_all(action='SELECT')
    bpy.ops.uv.smart_project(angle_limit=math.radians(args.angle_limit), island_margin=0.003,
                             area_weight=1.0, correct_aspect=True, scale_to_bounds=False)
    # Relaxing the projected charts can fold/collapse finely tessellated curved roofs.
    # Keep the projection and normalize island scale before packing.
    bpy.ops.uv.average_islands_scale()
    bpy.ops.object.mode_set(mode='OBJECT')
    rescue_packed_objects(objects)
    margin, history = 0.003, []
    for _ in range(8):
        bpy.ops.object.mode_set(mode='EDIT')
        bpy.ops.mesh.select_all(action='SELECT')
        bpy.ops.uv.select_all(action='SELECT')
        bpy.ops.uv.pack_islands(udim_source='ACTIVE_UDIM', rotate=True, rotate_method='ANY', scale=True,
                                margin_method='FRACTION', margin=margin, shape_method=args.pack_shape)
        bpy.ops.object.mode_set(mode='OBJECT')
        tile_scale = fit_atlas_tile(objects, margin)
        if rescue_packed_objects(objects):
            continue
        check_packed_objects(objects)
        area = np.concatenate([face_geometry(o, NEW_UV)[0] for o in objects])
        weakest = np.concatenate([face_axes(o, NEW_UV, 1, 1)[:, 0] for o in objects])
        size, required = choose_size(args, targets, weakest, area)
        # A very small candidate atlas can request huge fractional gutters.
        # That squeezes thin charts almost flat; grow the image instead once
        # padding reaches five percent of the tile.
        if margin >= .05:
            required = max(required, args.pack_margin_px / (margin * tile_scale))
            size = min(args.max_size, max(size, args.multiple * math.ceil(required / args.multiple)))
        history.append({'margin_fraction': margin, 'size': size, 'required_size': required})
        if margin * tile_scale * size >= args.pack_margin_px - 1e-9:
            return size, required, history
        margin = min(.05, args.pack_margin_px / (size * tile_scale))
    raise RuntimeError(f'Pack margin did not converge: {history}')


def bake(objects, size, need_alpha, args, work):
    """Bake RGB (and alpha) of the render materials into the lossy layer; returns uint8 array."""
    import bpy
    from PIL import Image
    scene = bpy.context.scene
    scene.render.engine = 'CYCLES'
    scene.cycles.device = 'CPU'
    scene.cycles.samples = 4
    scene.cycles.use_denoising = False
    scene.render.dither_intensity = 0.0
    scene.render.bake.use_selected_to_active = False
    result = []
    for pass_name, socket, colorspace in [('rgb', 'Color', 'sRGB')] + ([('alpha', 'Alpha', 'Non-Color')] if need_alpha else []):
        target = bpy.data.images.new(f'lossy bake {pass_name}', size, size, alpha=True, float_buffer=False)
        target.colorspace_settings.name = colorspace
        joined = []
        for obj in objects:
            temporary = obj.copy()
            temporary.data = obj.data.copy()
            bpy.context.scene.collection.objects.link(temporary)
            materials = []
            for slot in obj.material_slots:
                node = next(n for n in slot.material.node_tree.nodes if n.bl_idname == 'ShaderNodeTexImage')
                materials.append(build_material(f'lossy bake {pass_name} / {slot.material.name}', node.image, SOURCE_UV,
                                                node.interpolation, node.extension, emission_from=socket, target=target))
            indices = np.empty(len(obj.data.polygons), dtype=np.int32)
            obj.data.polygons.foreach_get('material_index', indices)
            set_slots(temporary, materials, indices)
            joined.append(temporary)
        bpy.ops.object.select_all(action='DESELECT')
        for temporary in joined:
            temporary.select_set(True)
        bpy.context.view_layer.objects.active = joined[0]
        if len(joined) > 1:
            bpy.ops.object.join()
        baked = bpy.context.view_layer.objects.active
        # Published charts can have discontinuous UVs/materials across a mesh edge.
        # Extend actual baked edge texels; adjacent-face sampling can pull atlas background
        # into the gutter and produce dark seams on otherwise continuous roofs.
        bpy.ops.object.bake(type='EMIT', margin=args.bake_margin, margin_type='EXTEND',
                            use_clear=True, target='IMAGE_TEXTURES', uv_layer=NEW_UV)
        mesh = baked.data
        bpy.data.objects.remove(baked, do_unlink=True)
        bpy.data.meshes.remove(mesh)
        path = work / f'bake-{pass_name}.png'
        target.filepath_raw = str(path)
        target.file_format = 'PNG'
        target.save()
        bpy.data.images.remove(target)
        result.append(np.asarray(Image.open(path).convert('RGB' if pass_name == 'rgb' else 'L')))
        path.unlink()
    return np.dstack(result) if need_alpha else result[0]


def encode_avif(pixels, work, args, name='atlas'):
    from PIL import Image
    png = work / f'{name}.png'
    Image.fromarray(pixels).save(png)
    avif = work / f'{name}.avif'
    command = ['avifenc', '-q', str(args.quality), '-s', str(args.speed), '-j', '2', str(png), str(avif)]
    if pixels.ndim == 3 and pixels.shape[2] == 4:
        command[1:1] = ['--qalpha', str(args.quality)]
    subprocess.run(command, check=True, capture_output=True)
    return avif.read_bytes(), command[:-2], png


# --- lossy GLB writer --------------------------------------------------------------------------

class BufferBuilder:
    def __init__(self):
        self.views, self.accessors, self.chunks, self.length = [], [], [], 0

    def view(self, data, target=None):
        pad = (-self.length) % 4
        if pad:
            self.chunks.append(b'\0' * pad)
            self.length += pad
        view = {'buffer': 0, 'byteOffset': self.length, 'byteLength': len(data)}
        if target:
            view['target'] = target
        self.chunks.append(data)
        self.length += len(data)
        self.views.append(view)
        return len(self.views) - 1

    def accessor(self, array, template, target):
        array = np.ascontiguousarray(array)
        record = {key: value for key, value in template.items()
                  if key not in ('bufferView', 'byteOffset', 'count', 'min', 'max', 'sparse')}
        rows = array.reshape(len(array), -1)
        element = rows.shape[1] * rows.dtype.itemsize
        if target == 34962 and element % 4:
            # Vertex attributes need 4-byte aligned elements: pad rows (e.g. int16 VEC3 -> stride 8).
            stride = element + (-element) % 4
            padded = np.zeros((len(rows), stride), dtype=np.uint8)
            padded[:, :element] = rows.view(np.uint8).reshape(len(rows), element)
            record['bufferView'] = self.view(padded.tobytes(), target)
            self.views[-1]['byteStride'] = stride
        else:
            record['bufferView'] = self.view(array.tobytes(), target)
        record['count'] = len(array)
        if 'min' in template or record.get('type') == 'VEC3' and template.get('_position'):
            values = array.reshape(len(array), -1)
            record['min'] = values.min(axis=0).tolist()
            record['max'] = values.max(axis=0).tolist()
        record.pop('_position', None)
        self.accessors.append(record)
        return len(self.accessors) - 1


def triangle_precision_safe(before, after, triangles):
    """Reject collapsed/flipped triangles or >10% area error, including tiny details."""
    triangles = np.asarray(triangles).reshape(-1, 3)
    def normals(values):
        points = values.astype(np.float64)[triangles]
        edges = points[:, 1:] - points[:, :1]
        if values.shape[1] == 2:
            return (edges[:, 0, 0] * edges[:, 1, 1] - edges[:, 0, 1] * edges[:, 1, 0])[:, None]
        return np.cross(edges[:, 0], edges[:, 1])
    original, rounded = normals(before), normals(after)
    squared = np.sum(original * original, axis=1)
    valid = squared > 0
    # Vector error also catches orientation changes that area alone would miss.
    return bool(np.all(np.sum((rounded[valid] - original[valid]) ** 2, axis=1) <= .01 * squared[valid]))


class Quantizer:
    """KHR_mesh_quantization for every mesh of the document.

    Positions become normalized uint16 on one asset-wide grid (identical source positions stay
    identical across meshes) dequantized by a uniform scale + translation on each mesh node, so
    normals transform unchanged. Normals become normalized int8/int16; lossy UVs normalized
    uint16. If rounding damages a triangle's area or orientation, positions stay float across
    the asset, or UVs stay float for that accessor. Only float source attributes are quantized.
    """
    def __init__(self, doc, binary, normal_bits):
        positions = [accessor_array(doc, binary, p['attributes']['POSITION'])
                     for m in doc['meshes'] for p in m['primitives']]
        low = np.min([p.min(axis=0) for p in positions], axis=0).astype(np.float64)
        high = np.max([p.max(axis=0) for p in positions], axis=0).astype(np.float64)
        self.offset = low
        self.scale = float(max((high - low).max(), 1e-6))
        self.normal_type = {8: (np.int8, 5120), 16: (np.int16, 5122)}[normal_bits]
        self.quantize_positions = True
        self.source_triangles = {}
        for mesh in doc['meshes']:
            for primitive in mesh['primitives']:
                attributes = primitive['attributes']
                positions = accessor_array(doc, binary, attributes['POSITION'], dequantize=True)
                indices = (accessor_array(doc, binary, primitive['indices']) if 'indices' in primitive
                           else np.arange(len(positions)))
                for name, index in attributes.items():
                    if name.startswith('TEXCOORD_'):
                        self.source_triangles.setdefault(index, []).extend(indices.reshape(-1, 3))
                rounded = np.round((positions - self.offset) / self.scale * 65535) / 65535 * self.scale + self.offset
                if not triangle_precision_safe(positions, rounded, indices):
                    # One common policy preserves shared boundaries between meshes.
                    self.quantize_positions = False
        for node in doc['nodes']:
            if 'mesh' in node:
                require(not any(k in node for k in ('matrix', 'translation', 'rotation', 'scale')),
                        f'Quantization needs untransformed mesh nodes: {node.get("name")}')

    def convert(self, name, array, template, triangles=None):
        if template.get('componentType') != 5126 or template.get('normalized'):
            return array, template
        record = {k: v for k, v in template.items() if k not in ('componentType', 'normalized', 'min', 'max')}
        if name == 'POSITION':
            if not self.quantize_positions:
                return array, template
            values = np.round((array.astype(np.float64) - self.offset) / self.scale * 65535)
            require(values.min() >= 0 and values.max() <= 65535, 'Position outside the quantization grid')
            return values.astype(np.uint16), {**record, 'componentType': 5123, 'normalized': True, '_position': True}
        if name == 'NORMAL':
            dtype, component = self.normal_type
            limit = np.iinfo(dtype).max
            return (np.round(np.clip(array, -1, 1) * limit).astype(dtype),
                    {**record, 'componentType': component, 'normalized': True})
        if name.startswith('TEXCOORD_') and array.min() >= 0 and array.max() <= 1:
            rounded = np.round(array.astype(np.float64) * 65535)
            if triangles is None or not triangle_precision_safe(array, rounded / 65535, triangles):
                return array, template
            return (rounded.astype(np.uint16),
                    {**record, 'componentType': 5123, 'normalized': True})
        return array, template

    def apply_nodes(self, out):
        for node in out['nodes']:
            if 'mesh' in node and self.quantize_positions:
                node['translation'] = self.offset.tolist()
                node['scale'] = [self.scale] * 3
        for key in ('extensionsUsed', 'extensionsRequired'):
            out[key] = sorted(set(out.get(key, [])) | {'KHR_mesh_quantization'})


def write_lossy(doc, binary, records, atlas_bytes, output, drop_normals=True, texture_file=False, normal_bits=None,
                reencoded=None):
    """Copy the published document; replace textured primitives' vertices and all textures.

    With `drop_normals`, NORMAL is omitted on rebuilt primitives whose material is unlit (their
    display never reads normals); lit materials keep them. Returns (bytes, normal statistics).
    """
    out = copy.deepcopy(doc)
    normals = {'dropped_primitives': 0, 'kept_lit_primitives': 0, 'copied_untextured_primitives': 0}
    builder = BufferBuilder()
    quantizer = Quantizer(doc, binary, normal_bits) if normal_bits else None
    remap = {}

    def emit(name, array, template, triangles=None):
        if quantizer and name:
            array, template = quantizer.convert(name, array, template, triangles)
        return builder.accessor(array, template, 34962 if name else 34963)

    def copy_accessor(index, name):
        if (index, name) not in remap:
            remap[(index, name)] = emit(name, accessor_array(doc, binary, index), dict(doc['accessors'][index]),
                                        quantizer.source_triangles.get(index) if quantizer else None)
        return remap[(index, name)]

    rebuilt = {}
    for record in records or []:
        obj, mesh_index = record['object'], record['mesh']
        uv = obj.data.uv_layers[NEW_UV].uv
        values = np.empty(len(uv) * 2, dtype=np.float32)
        uv.foreach_get('vector', values)
        values = values.reshape(-1, 3, 2)
        per_prim = {}
        for face, (corner_list, new_uv) in enumerate(zip(record['corners'], values)):
            for (prim_index, vertex), (u, v) in zip(corner_list, new_uv):
                per_prim.setdefault(prim_index, []).append((vertex, float(u), float(1.0 - v)))
        rebuilt[mesh_index] = per_prim
    for mesh_index, mesh in enumerate(out['meshes']):
        for prim_index, primitive in enumerate(mesh['primitives']):
            source = doc['meshes'][mesh_index]['primitives'][prim_index]
            corners = rebuilt.get(mesh_index, {}).get(prim_index)
            if corners is None:
                require(reencoded is not None or display_texture(doc, source) is None,
                        f'Textured primitive {mesh_index}/{prim_index} was not rebuilt')
                # Untouched primitive, or keep-layout mode (original UVs and images, re-encoded).
                textured = reencoded is not None and ('material' in source and (has_pbr_maps(doc) or display_texture(doc, source) is not None))
                unlit = textured and 'KHR_materials_unlit' in doc['materials'][source['material']].get('extensions', {})
                drop = drop_normals and unlit and 'NORMAL' in source['attributes']
                primitive['attributes'] = {name: copy_accessor(index, name) for name, index in source['attributes'].items()
                                           if not (drop and name == 'NORMAL')}
                if 'indices' in source:
                    primitive['indices'] = copy_accessor(source['indices'], None)
                if not textured:
                    normals['copied_untextured_primitives'] += 1
                else:
                    normals['dropped_primitives' if drop else 'kept_lit_primitives'] += drop or not unlit
                continue
            unlit = 'KHR_materials_unlit' in doc['materials'][source['material']].get('extensions', {})
            drop = drop_normals and unlit and 'NORMAL' in source['attributes']
            normals['dropped_primitives' if drop else 'kept_lit_primitives'] += drop or not unlit
            kept = {name: index for name, index in source['attributes'].items()
                    if not name.startswith('TEXCOORD_') and not (drop and name == 'NORMAL')}
            arrays = {name: accessor_array(doc, binary, index) for name, index in kept.items()}
            # Identical copied attributes + identical new UV = one vertex (the published per-face
            # charts split every vertex; most of that splitting disappears with the new layout).
            signature = np.concatenate([np.ascontiguousarray(a).reshape(len(a), -1).view(np.uint8).reshape(len(a), -1)
                                        for a in arrays.values()], axis=1)
            _, signature_id = np.unique(signature, axis=0, return_inverse=True)
            unique, index_list, order = {}, [], []
            for vertex, u, v in corners:
                key = (int(signature_id[vertex]), u, v)
                if key not in unique:
                    unique[key] = len(unique)
                    order.append(vertex)
                index_list.append(unique[key])
            order = np.array(order, dtype=np.int64)
            attributes = {}
            for name, index in kept.items():
                template = dict(doc['accessors'][index])
                if name == 'POSITION':
                    template['_position'] = True
                attributes[name] = emit(name, arrays[name][order], template)
            texcoords = np.array([(key[1], key[2]) for key in unique], dtype=np.float32)
            attributes['TEXCOORD_0'] = emit('TEXCOORD_0', texcoords, {'componentType': 5126, 'type': 'VEC2'}, index_list)
            primitive['attributes'] = attributes
            indices = np.array(index_list, dtype=np.uint16 if len(unique) < 65536 else np.uint32)
            primitive['indices'] = emit(None, indices, {'componentType': 5123 if indices.dtype == np.uint16 else 5125,
                                                        'type': 'SCALAR'})
    if reencoded is not None:
        # Keep-layout mode: same images, textures, samplers and materials; AVIF image data.
        require(not texture_file, 'Sibling texture files are only supported for a single atlas')
        image_views = {}
        def image_view(index):
            data = reencoded[index]
            checksum = hashlib.sha256(data).digest()
            if checksum not in image_views:
                image_views[checksum] = builder.view(data)
            return image_views[checksum]
        out['images'] = [{**({'name': image['name']} if 'name' in image else {}), 'mimeType': 'image/avif',
                          'bufferView': image_view(index)} for index, image in enumerate(doc.get('images', []))]
        out['textures'] = [{**{k: v for k, v in texture.items() if k in ('sampler', 'name')},
                            'extensions': {'EXT_texture_avif': {'source': avif_source(texture)}}}
                           for texture in doc.get('textures', [])]
        textured = set()
        samplers = None
    else:
        textured = {record_material for record in records for *_, record_material in record['materials']}
    # One texture per distinct source sampler, all on the one atlas image: materials keep their
    # filtering (e.g. nearest-neighbour source sampling) and wrap modes.
    if reencoded is None:
        material_sampler = {material_index: json.dumps(sampler, sort_keys=True) for record in records
                            for *_, material_index in record['materials']
                            for _, _, sampler in [display_texture(doc, {'material': material_index})]}
        samplers = sorted(set(material_sampler.values()))
    if reencoded is not None:
        pass
    elif texture_file:
        # Sibling file (same stem), referenced relative to the GLB.
        texture_path = output.with_suffix('.avif')
        texture_path.write_bytes(atlas_bytes)
        out['images'] = [{'uri': texture_path.name, 'mimeType': 'image/avif', 'name': 'lossy atlas'}]
    else:
        out['images'] = [{'bufferView': builder.view(atlas_bytes), 'mimeType': 'image/avif', 'name': 'lossy atlas'}]
    if reencoded is None:
        out['samplers'] = [json.loads(sampler) for sampler in samplers]
        out['textures'] = [{'sampler': i, 'extensions': {'EXT_texture_avif': {'source': 0}}} for i in range(len(samplers))]
    used_materials = {primitive['material'] for mesh in doc['meshes'] for primitive in mesh['primitives']
                      if 'material' in primitive}
    for index, material in enumerate(out.get('materials', []) if reencoded is None else []):
        pbr = material.get('pbrMetallicRoughness', {})
        if index in textured:
            texture = samplers.index(material_sampler[index])
            if 'baseColorTexture' in pbr:
                pbr['baseColorTexture'] = {'index': texture}
            if 'emissiveTexture' in material:
                material['emissiveTexture'] = {'index': texture}
        elif 'baseColorTexture' in pbr or 'emissiveTexture' in material:
            # A textured material no primitive references is legal glTF (e.g. left over by the
            # spline tool). Its images are not part of the atlas, so it stays, untextured.
            require(index not in used_materials, f'Textured material {index} is used by a primitive that was not rebuilt')
            pbr.pop('baseColorTexture', None)
            material.pop('emissiveTexture', None)
            material.pop('emissiveFactor', None)
    if out.get('images'):
        for key in ('extensionsUsed', 'extensionsRequired'):
            out[key] = sorted(set(out.get(key, [])) | {'EXT_texture_avif'})
    if quantizer:
        quantizer.apply_nodes(out)
    out['accessors'] = builder.accessors
    out['bufferViews'] = builder.views
    body = b''.join(builder.chunks)
    body += b'\0' * ((-len(body)) % 4)
    out['buffers'] = [{'byteLength': len(body)}]
    out.setdefault('asset', {})['extras'] = {**out['asset'].get('extras', {}), 'lossy_derivation': True}
    chunk = json.dumps(out, separators=(',', ':')).encode()
    chunk += b' ' * ((-len(chunk)) % 4)
    data = (struct.pack('<4sII', b'glTF', 2, 12 + 8 + len(chunk) + 8 + len(body))
            + struct.pack('<II', len(chunk), 0x4E4F534A) + chunk + struct.pack('<II', len(body), 0x004E4942) + body)
    output.write_bytes(data)
    return data, normals


# --- validation ----------------------------------------------------------------------------------

def cameras_for(scene, objects, args):
    """Eight 35-degree oblique orthographic views sharing one frame and resolution."""
    import bpy
    from mathutils import Vector
    points = [o.matrix_world @ v.co for o in objects for v in o.data.vertices]
    lo = Vector([min(p[i] for p in points) for i in range(3)])
    hi = Vector([max(p[i] for p in points) for i in range(3)])
    centre = (lo + hi) / 2
    corners = [Vector((x, y, z)) for x in (lo.x, hi.x) for y in (lo.y, hi.y) for z in (lo.z, hi.z)]
    elevation = math.radians(35)
    frames, width, height = [], 0.0, 0.0
    for index in range(8):
        yaw = math.radians(index * 45)
        direction = Vector((math.sin(yaw) * math.cos(elevation), -math.cos(yaw) * math.cos(elevation), math.sin(elevation)))
        rotation = (-direction).to_track_quat('-Z', 'Y')
        inverse = rotation.to_matrix().inverted()
        local = [inverse @ (c - centre) for c in corners]
        width = max(width, 2 * max(abs(p.x) for p in local))
        height = max(height, 2 * max(abs(p.y) for p in local))
        frames.append((direction, rotation))
    width, height = width * 1.04, height * 1.04
    scale = min(args.render_scale, args.render_max / max(width, height))
    scene.render.resolution_x = max(16, round(width * scale))
    scene.render.resolution_y = max(16, round(height * scale))
    cameras = []
    for direction, rotation in frames:
        data = bpy.data.cameras.new('lossy review camera')
        data.type = 'ORTHO'
        data.ortho_scale = max(width, height)
        data.clip_start = 0.1
        data.clip_end = 1e6
        camera = bpy.data.objects.new(data.name, data)
        camera.location = centre + direction * (max(width, height) * 4 + 1000)
        camera.rotation_euler = rotation.to_euler()
        scene.collection.objects.link(camera)
        cameras.append(camera)
    return cameras, scale, {'min': list(lo), 'max': list(hi)}


def validate(original_objects, lossy_objects, work, args):
    import bpy
    scene = bpy.context.scene
    scene.render.engine = 'CYCLES'
    scene.cycles.samples = 16
    scene.render.film_transparent = True
    scene.render.image_settings.file_format = 'PNG'
    scene.render.image_settings.color_mode = 'RGBA'
    scene.render.image_settings.color_depth = '8'
    scene.view_settings.view_transform = 'Standard'
    scene.view_settings.look = 'None'
    scene.world = None
    cameras, scale, bounds = cameras_for(scene, original_objects, args)
    bpy.context.view_layer.update()
    before = render_views(scene, cameras, work / 'renders/published', original_objects, lossy_objects)
    after = render_views(scene, cameras, work / 'renders/lossy', lossy_objects, original_objects)
    differences = compare(before, after, work / 'compare-sheet.png',
                          labels=('published', f'lossy (AVIF q{args.quality})'))
    for camera in cameras:
        bpy.data.objects.remove(camera, do_unlink=True)
    return differences, scale, bounds, [scene.render.resolution_x, scene.render.resolution_y]


# --- per asset -----------------------------------------------------------------------------------

def tool_versions():
    import bpy
    avif = subprocess.run(['avifenc', '--version'], check=True, capture_output=True, text=True).stdout.splitlines()[0]
    return {'blender': bpy.app.version_string, 'avifenc': avif,
            'script': sha(Path(__file__))}


def texel_reuse(objects, records):
    """Per source image: summed face texel area / image texels (>1 = texels shared by faces or
    tiled), and whether any published UV leaves [0, 1] (repeat tiling)."""
    covered, out_of_range = {}, False
    for obj, record in zip(objects, records):
        _, uv_area = face_geometry(obj, SOURCE_UV)
        slots = np.empty(len(obj.data.polygons), dtype=np.int32)
        obj.data.polygons.foreach_get('material_index', slots)
        values = np.empty(len(obj.data.uv_layers[SOURCE_UV].uv) * 2, dtype=np.float32)
        obj.data.uv_layers[SOURCE_UV].uv.foreach_get('vector', values)
        out_of_range |= bool(values.size and (values.min() < -1e-3 or values.max() > 1 + 1e-3))
        for slot, (image, *_rest) in enumerate(record['materials']):
            covered[image.name] = covered.get(image.name, 0.0) + float(uv_area[slots == slot].sum()) * image.size[0] * image.size[1]
    sizes = {record_image.name: record_image.size[0] * record_image.size[1]
             for record in records for record_image, *_ in record['materials']}
    return {name: covered[name] / max(sizes[name], 1) for name in covered}, out_of_range


def derive_pbr(asset_id, model_path, lossy_path, args, work, doc, binary, source_bytes):
    """Compress independent PBR images, preserving geometry, UVs and material parameters.

    Identity matrix and 4:4:4 avoid mixing the independent channels of data maps.
    Keep their quality higher than colour artwork, especially for normal vectors.
    """
    import io
    from PIL import Image
    data_images = set()
    for material in doc.get('materials', []):
        for info in (material.get('normalTexture'), material.get('occlusionTexture'),
                     material.get('pbrMetallicRoughness', {}).get('metallicRoughnessTexture')):
            if info:
                data_images.add(avif_source(doc['textures'][info['index']]))
    encoded = {}
    for index, image in enumerate(doc.get('images', [])):
        if 'uri' in image:
            require(not image['uri'].startswith('data:'), 'Data URI images are not supported')
            payload = (model_path.parent / image['uri']).read_bytes()
        else:
            view = doc['bufferViews'][image['bufferView']]
            start = view.get('byteOffset', 0)
            payload = binary[view.get('buffer', 0)][start:start + view['byteLength']]
        png, avif = work / f'pbr-{index}.png', work / f'pbr-{index}.avif'
        with Image.open(io.BytesIO(payload)) as source:
            source.convert('RGBA' if 'A' in source.getbands() else 'RGB').save(png)
        quality = max(90, args.quality) if index in data_images else args.quality
        command = ['avifenc', '-q', str(quality), '--qalpha', '100', '-s', str(args.speed), '-j', '2']
        if index in data_images:
            command += ['--yuv', '444', '--cicp', '1/13/0']
        subprocess.run(command + [str(png), str(avif)], check=True, capture_output=True)
        encoded[index] = avif.read_bytes()
    lossy_path.parent.mkdir(parents=True, exist_ok=True)
    data, normals = write_lossy(doc, binary, [], b'', lossy_path, drop_normals=False,
                               reencoded=encoded)
    report = {'asset_id': asset_id, 'mode': 'PBR texture re-encode',
              'source_sha256': sha(model_path), 'lossy_sha256': sha(lossy_path),
              'bytes': {'source_glb': len(source_bytes), 'lossy_glb': len(data)}, 'normals': normals}
    receipt = {'source': report['source_sha256'], 'output': report['lossy_sha256'],
               'settings': settings(args), 'tools': tool_versions()}
    Path(str(lossy_path) + '.receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    (work / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def derive(asset_id, model_path, lossy_path, args, work):
    import bpy
    bpy.ops.wm.read_homefile(use_empty=True, use_factory_startup=True)
    doc, binary, source_bytes = read_glb(model_path)
    work.mkdir(parents=True)
    if has_pbr_maps(doc):
        return derive_pbr(asset_id, model_path, lossy_path, args, work, doc, binary, source_bytes)
    if not any(display_texture(doc, primitive) for mesh in doc['meshes'] for primitive in mesh['primitives']):
        require(not doc.get('images') and not doc.get('textures'), 'Untextured model has unused texture resources')
        lossy_path.parent.mkdir(parents=True, exist_ok=True)
        data, normals = write_lossy(doc, binary, [], b'', lossy_path, drop_normals=False,
                                   normal_bits=None if args.no_quantize else args.normal_bits, reencoded={})
        report = {'asset_id': asset_id, 'mode': 'untextured geometry',
                  'source_sha256': sha(model_path), 'lossy_sha256': sha(lossy_path),
                  'bytes': {'source_glb': len(source_bytes), 'lossy_glb': len(data)}, 'normals': normals}
        receipt = {'source': report['source_sha256'], 'output': report['lossy_sha256'],
                   'settings': settings(args), 'tools': tool_versions()}
        Path(str(lossy_path) + '.receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
        (work / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
        return report
    images = load_images(doc, binary, work / 'source-images', base=model_path.parent)
    need_alpha = any(doc['materials'][p['material']].get('alphaMode', 'OPAQUE') != 'OPAQUE'
                     for m in doc['meshes'] for p in m['primitives'] if display_texture(doc, p))
    collection = bpy.data.collections.new('Published')
    bpy.context.scene.collection.children.link(collection)
    objects, records = build_objects(doc, binary, images, collection, 'published')
    require(objects, f'No textured meshes in {model_path}')
    area = np.concatenate([face_geometry(o, SOURCE_UV)[0] for o in objects])
    source_axes, nearest = [], []
    for obj, record in zip(objects, records):
        slots = np.empty(len(obj.data.polygons), dtype=np.int32)
        obj.data.polygons.foreach_get('material_index', slots)
        axes = np.zeros((len(slots), 2))
        for slot, material_slot in enumerate(obj.material_slots):
            image = next(n for n in material_slot.material.node_tree.nodes if n.bl_idname == 'ShaderNodeTexImage').image
            mask = slots == slot
            axes[mask] = face_axes(obj, SOURCE_UV, *image.size)[mask]
        source_axes.append(axes)
        nearest.append(np.array([record['materials'][slot][1] == 'Closest' for slot in slots], dtype=bool))
    source_axes, nearest = np.concatenate(source_axes), np.concatenate(nearest)
    # Preserve the source's detailed direction, rather than propagating its blurriest axis
    # into every direction of the new atlas.
    # Nearest-filtered ("source pixel sampling") faces are baked at --nearest-density instead,
    # so their source pixel blocks stay sharp after resampling.
    targets = np.where(nearest, args.nearest_density, np.minimum(args.density, source_axes[:, 1]))
    uv_area = np.concatenate([face_geometry(o, SOURCE_UV)[1] for o in objects])
    reuse, out_of_range = texel_reuse(objects, records)
    # Keep the published layout when a unique-texel atlas cannot help: one image the UVs already
    # fill (map background planes), or texels reused by many faces / tiled (foliage cards).
    # Foliage's many tiny opacity-cutout charts can collapse during smart packing.
    # Preserve its authored UVs and physical coverage rather than rebaking cards.
    reencode = (need_alpha or (len(images) == 1 and uv_area.sum() >= args.reencode_utilization)
                or max(reuse.values(), default=0) > args.reuse_ratio or out_of_range)
    packing_failure = None
    expansion_limited = False
    if not reencode:
        try:
            size, required, history = unwrap(objects, args, targets)
            new_axes = np.concatenate([face_axes(o, NEW_UV, size, size) for o in objects])
            # A capped atlas cannot meet the requested surface density. Keep the source
            # layout instead of silently publishing undersampled or collapsed charts.
            reencode = required > args.max_size
            expansion_limited = atlas_expansion_exceeded(
                size, [image.size for image in images.values()], args.max_atlas_expansion)
            reencode |= expansion_limited
        except UnsafeAtlasError as error:
            packing_failure = str(error)
            print(f'RETAIN SOURCE UVS {asset_id}: {error}', flush=True)
            reencode = True
    if reencode:
        from PIL import Image
        reencoded, avif_command, size = {}, None, []
        alpha_images = {image for m in doc['meshes'] for p in m['primitives'] for found in [display_texture(doc, p)]
                        if found and doc['materials'][p['material']].get('alphaMode', 'OPAQUE') != 'OPAQUE'
                        for image in [found[1]]}
        encoded_images = {}
        for index, image in images.items():
            source = Image.open(image.filepath_raw)
            keep_alpha = index in alpha_images and 'A' in source.getbands()
            key = (sha(Path(image.filepath_raw)), keep_alpha)
            if key not in encoded_images:
                encoded_images[key], avif_command, png = encode_avif(np.asarray(source.convert('RGBA' if keep_alpha else 'RGB')),
                                                                   work, args, name=f'image-{index}')
                png.unlink()
            reencoded[index] = encoded_images[key]
            size.append(list(image.size))
        for index in range(len(doc.get('images', []))):
            require(index in reencoded, f'Image {index} was not decoded')
        atlas_bytes = b''.join(reencoded.values())
        required, history, new_axes, atlas_png = None, [], source_axes, None
    else:
        reencoded = None
        pixels = bake(objects, size, need_alpha, args, work)
        size = [size, size]
        atlas_bytes, avif_command, atlas_png = encode_avif(pixels, work, args)
    lossy_path.parent.mkdir(parents=True, exist_ok=True)
    lossy_bytes, normals = write_lossy(doc, binary, None if reencode else records, atlas_bytes, lossy_path,
                                       not args.keep_normals, args.texture_file,
                                       None if args.no_quantize else args.normal_bits, reencoded=reencoded)

    # Structural check from the written bytes: same meshes, primitives and triangle counts.
    lossy_doc, lossy_binary, _ = read_glb(lossy_path)
    for mesh, lossy_mesh in zip(doc['meshes'], lossy_doc['meshes'], strict=True):
        for primitive, lossy_primitive in zip(mesh['primitives'], lossy_mesh['primitives'], strict=True):
            counts = [p_doc['accessors'][p['indices']]['count'] if 'indices' in p else p_doc['accessors'][p['attributes']['POSITION']]['count']
                      for p_doc, p in ((doc, primitive), (lossy_doc, lossy_primitive))]
            require(counts[0] == counts[1], f'Triangle count changed in mesh {mesh.get("name")}')
    points = [o.matrix_world @ v.co for o in objects for v in o.data.vertices]
    bounds = {'min': [min(p[i] for p in points) for i in range(3)], 'max': [max(p[i] for p in points) for i in range(3)]}
    differences = render_scale = resolution = None
    if args.validate:
        # Optional evidence: render published vs lossy (decoded from its own bytes) from 8 views.
        lossy_images = load_images(lossy_doc, lossy_binary, work / 'lossy-images', base=lossy_path.parent, decode_avif=True)
        lossy_collection = bpy.data.collections.new('Lossy')
        bpy.context.scene.collection.children.link(lossy_collection)
        lossy_objects, _ = build_objects(lossy_doc, lossy_binary, lossy_images, lossy_collection, 'lossy')
        require(len(lossy_objects) == len(objects), 'Lossy lost textured meshes')
        for before, after in zip(objects, lossy_objects):
            require(len(before.data.polygons) == len(after.data.polygons), f'Face count changed: {before.name}')
        differences, render_scale, bounds, resolution = validate(objects, lossy_objects, work, args)

    source_images = [{'size': list(img.size), 'pixels': img.size[0] * img.size[1],
                      'bytes': ((model_path.parent / doc['images'][i]['uri']).stat().st_size if 'uri' in doc['images'][i]
                                else doc['bufferViews'][doc['images'][i]['bufferView']]['byteLength']),
                      'shared_blob': doc['images'][i].get('uri'),
                      'mime': doc['images'][i]['mimeType']} for i, img in images.items()]
    extent = [bounds['max'][i] - bounds['min'][i] for i in range(3)]
    lossy_pixels = sum(w * h for w, h in size) if reencode else size[0] * size[1]
    report = {
        'asset_id': asset_id, 'source_model': str(model_path), 'source_sha256': hashlib.sha256(source_bytes).hexdigest(),
        'lossy_model': str(lossy_path), 'lossy_sha256': hashlib.sha256(lossy_bytes).hexdigest(),
        'on_map': {'extent_map_px': extent, 'surface_area_map_px2': float(area.sum()),
                   'faces': int(len(area)), 'textured_meshes': len(objects)},
        'bytes': {'source_glb': len(source_bytes), 'lossy_glb': len(lossy_bytes),
                  'source_textures': sum(i['bytes'] for i in source_images), 'lossy_texture_avif': len(atlas_bytes)},
        'textures': {'source': source_images, 'source_pixels': sum(i['pixels'] for i in source_images),
                     'lossy_size': size, 'lossy_pixels': lossy_pixels,
                     'lossy_channels': 4 if need_alpha else 3,
                     'gpu_rgba8_bytes': {'source': sum(i['pixels'] for i in source_images) * 4,
                                         'lossy': lossy_pixels * 4}},
        'atlas_size': {'mode': 're-encode published layout' if reencode else 'smart-uv + normalized island scale',
                       'packing_failure': packing_failure,
                       'expansion_limited': expansion_limited,
                       'texel_reuse': reuse, 'uv_out_of_range': out_of_range,
                       'size': size, 'required': required, 'multiple': args.multiple, 'min': args.min_size,
                       'max': args.max_size, 'clamped': None if required is None else 'max' if required > args.max_size
                       else 'min' if required < args.min_size else None, 'pack_history': history},
        'density_texels_per_map_px': {
            'source_weakest_axis': stats(source_axes[:, 0], area), 'lossy_weakest_axis': stats(new_axes[:, 0], area),
            'source_sqrt_area': stats(np.sqrt(source_axes[:, 0] * source_axes[:, 1]), area),
            'lossy_sqrt_area': stats(np.sqrt(new_axes[:, 0] * new_axes[:, 1]), area),
            'surface_fraction_lossy_weakest_below_target': float(
                area[new_axes[:, 0] < targets - 1e-6].sum() / area.sum()),
            'target': 'min(--density, source strongest axis) per face; --nearest-density on nearest-filtered faces',
            'nearest_filtered_surface_fraction': float(area[nearest].sum() / area.sum())},
        'encoding': {'avifenc': avif_command, 'alpha': need_alpha},
        'normals': normals,
        'validation': None if differences is None else {
            'render_px_per_map_px': render_scale, 'resolution': resolution,
            'differences': differences, 'sheet': str(work / 'compare-sheet.png')},
    }
    texture_path = lossy_path.with_suffix('.avif')
    receipt = {'source': report['source_sha256'], 'output': report['lossy_sha256'],
               **({'texture': {'path': texture_path.name, 'sha256': sha(texture_path)}} if args.texture_file else {}),
               'settings': settings(args),
               'validation': None if differences is None else differences['overall'],
               'normals': {**normals, 'policy': 'kept (--keep-normals)' if args.keep_normals else
                           'dropped on unlit textured primitives: no lighting yet; lossy assets are rebuilt '
                           '(and shadows unbaked) when lighting is added. Published models keep their normals.'},
               'tools': tool_versions()}
    Path(str(lossy_path) + '.receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    (work / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    if atlas_png is not None:
        shutil.copyfile(atlas_png, work / 'atlas-preview.png')
        atlas_png.unlink()
    if not args.validate:
        # Without --validate only the report is kept; the receipts carry the binding hashes.
        for child in work.iterdir():
            if child.name != 'report.json':
                shutil.rmtree(child) if child.is_dir() else child.unlink()
    return report


def library_models(source_root, ids):
    """(asset id, model path relative to the index root) for every model file an entry uses.

    Paths come only from the library index and descriptors, so the lossy placement follows
    whatever layout the library uses (`<id>/model.glb`, `<level>/<id>/model.glb`, ...).
    """
    index_path = source_root / 'index.json'
    entries = {e['id']: e for e in json.loads(index_path.read_text())['assets']}
    missing = sorted(set(ids) - set(entries))
    require(not missing, f'Assets not in {index_path}: {missing}')
    models = []
    for key in ids:
        entry = entries[key]
        paths = [entry['model']]
        descriptor_path = source_root / entry['descriptor']
        if descriptor_path.exists():
            descriptor = json.loads(descriptor_path.read_text())
            parent = Path(entry['descriptor']).parent
            for variants in (descriptor.get('state_variants'), descriptor.get('standalone_variants')):
                for variant in (variants or {}).values():
                    paths.append(str(parent / variant['model']))
        for path in dict.fromkeys(paths):
            models.append((key, path))
    return models


def lossy_name(model_path):
    return 'lossy.glb' if model_path.name == 'model.glb' else model_path.stem + '.lossy.glb'


def preview_name(model_path):
    return 'preview.glb' if model_path.name == 'model.glb' else model_path.stem + '.preview.glb'


PREVIEW_CLI = PIPELINE / 'src/preview-model.ts'
_preview_fingerprint = None


def preview_fingerprint():
    """Preview settings + installed tool versions (pipeline/src/preview-model.ts), cached per run."""
    global _preview_fingerprint
    if _preview_fingerprint is None:
        _preview_fingerprint = subprocess.run(['node', str(PREVIEW_CLI), '--fingerprint'], cwd=PIPELINE, check=True,
                                              capture_output=True, text=True).stdout.strip().splitlines()[-1]
    return _preview_fingerprint


def preview_current(root, source, preview):
    """True when `preview` and its receipt bind the current `source` bytes and preview settings."""
    receipt_path = root / (preview + '.receipt.json')
    if not (root / preview).exists() or not receipt_path.exists():
        return False
    receipt = json.loads(receipt_path.read_text())
    return (receipt.get('source') == sha(root / source) and receipt.get('source_model') == source
            and receipt.get('fingerprint') == preview_fingerprint() and receipt.get('output') == sha(root / preview))


def write_preview(source_path, source_relative, output):
    """Preview GLB (simplified, meshopt, AVIF at the size rule) + receipt chained to `source`."""
    source_path, output = Path(source_path).resolve(strict=True), Path(output).resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_name(f'.{output.name}.{uuid.uuid4().hex}.tmp')
    try:
        result = subprocess.run(['node', str(PREVIEW_CLI), str(source_path), str(temporary)], cwd=PIPELINE,
                                capture_output=True, text=True)
        if result.returncode:
            raise RuntimeError(f'Preview failed for {source_relative}: {(result.stderr or result.stdout).strip()[-800:]}')
        info = json.loads(result.stdout.strip().splitlines()[-1])
        os.replace(temporary, output)
    finally:
        temporary.unlink(missing_ok=True)
    receipt = {'source': sha(source_path), 'source_model': source_relative, 'fingerprint': preview_fingerprint(),
               'output': sha(output), 'texture_edge': info['edge']}
    Path(str(output) + '.receipt.json').write_text(json.dumps(receipt) + '\n')
    return info


def main_derive(args):
    source_root = args.source_root.resolve(strict=True)
    output = args.output.resolve()
    require(ROOT.resolve() in output.parents, f'Output must stay under {ROOT}')
    sys.path.insert(0, str(RENDER_SLOTS))
    from render_slots import acquire
    acquire()
    reports, failures, lossy_models = [], {}, {}
    for asset_id, model in library_models(source_root, args.assets):
        relative = Path(model)
        lossy_relative = relative.parent / lossy_name(relative)
        lossy = output / lossy_relative
        work = output.parent / 'work' / relative.with_suffix('')
        require(not lossy.exists() and not work.exists(), f'Output exists: {lossy} / {work}')
        try:
            reports.append(derive(asset_id, (source_root / relative).resolve(strict=True), lossy, args, work))
            lossy_models[model] = {'asset_id': asset_id, 'lossy_model': str(lossy_relative),
                               **({'lossy_texture': str(lossy_relative.with_suffix('.avif'))}
                                  if args.texture_file else {})}
            print(f'LOSSY {model}: {json.dumps(summary_row(reports[-1]))}', flush=True)
        except Exception as error:  # Report and continue with the other assets.
            failures[model] = f'{type(error).__name__}: {error}'
            print(f'LOSSY FAILED {model}: {failures[model]}', flush=True)
            if args.fail_fast:
                raise
    # Keyed by the index's own model paths; merging into the library index sets
    # `lossy_model` on the entry whose `model` matches.
    (output / 'index.lossy.json').write_text(json.dumps({'version': 1, 'lossy_models': lossy_models}, indent=2) + '\n')
    (output.parent / 'summary.json').write_text(json.dumps(
        {'rows': [summary_row(r) for r in reports], 'failures': failures}, indent=2) + '\n')
    require(not failures, f'Failed assets: {failures}')


ALGORITHM_VERSION = 5

SETTING_KEYS = ('density_coverage', 'density', 'nearest_density', 'pack_shape', 'multiple', 'min_size', 'max_size', 'max_atlas_expansion', 'quality', 'reencode_utilization', 'reuse_ratio', 'keep_normals',
                'texture_file', 'no_quantize', 'normal_bits', 'speed', 'angle_limit', 'pack_margin_px', 'bake_margin')


def settings(args):
    """Derivation settings recorded in receipts; a receipt with other settings is out of date."""
    return {'algorithm_version': ALGORITHM_VERSION, **{key: getattr(args, key) for key in SETTING_KEYS}}


def summary_row(report):
    if report.get('mode') in ('untextured geometry', 'PBR texture re-encode'):
        return {'asset': report['asset_id'], 'mode': report['mode'],
                'glb_bytes': [report['bytes']['source_glb'], report['bytes']['lossy_glb']]}
    validation, b, t = report['validation'], report['bytes'], report['textures']
    d = validation['differences']['overall'] if validation else None
    return {'asset': report['asset_id'], 'extent_map_px': [round(v) for v in report['on_map']['extent_map_px']],
            'faces': report['on_map']['faces'],
            'source_textures': [f'{s["size"][0]}x{s["size"][1]}' for s in t['source']],
            'lossy_atlas': (f'{t["lossy_size"][0]}x{t["lossy_size"][1]}' if isinstance(t['lossy_size'][0], int)
                            else [f'{w}x{h}' for w, h in t['lossy_size']]), 'clamped': report['atlas_size']['clamped'],
            'mode': report['atlas_size']['mode'],
            'weakest_axis_median': round(report['density_texels_per_map_px']['lossy_weakest_axis']['area_weighted_median'], 3),
            'glb_bytes': [b['source_glb'], b['lossy_glb']], 'texture_bytes': [b['source_textures'], b['lossy_texture_avif']],
            'gpu_bytes': [t['gpu_rgba8_bytes']['source'], t['gpu_rgba8_bytes']['lossy']],
            **({'diff_mean': round(d['mean_of_view_means'], 2), 'diff_p95_max': d['max_p95'], 'diff_max': d['max'],
                'render_px_per_map_px': round(validation['render_px_per_map_px'], 3)} if validation else {})}


def main_export_worker(args):
    """Development: export worker assets exactly as publication does, into a scratch layout."""
    worker = args.worker.resolve(strict=True)
    require(sha(worker) == args.worker_sha256, 'Worker hash mismatch')
    output = args.output.resolve()
    require(ROOT.resolve() in output.parents, f'Output must stay under {ROOT}')
    sys.path.insert(0, str(RENDER_SLOTS))
    sys.path.insert(0, str(HERE))
    from render_slots import acquire
    acquire()
    import bpy
    from export_editor import export_editor
    bpy.ops.wm.open_mainfile(filepath=str(worker))
    for asset_id in args.assets:
        target = output / asset_id / 'model.glb'
        target.parent.mkdir(parents=True, exist_ok=False)
        export_editor(args.map_name, target, asset_id)
        print(f'EXPORTED {asset_id} {target.stat().st_size}', flush=True)
    write_asset_index(output)
    (output / 'export.json').write_text(json.dumps({'worker': str(worker), 'worker_sha256': args.worker_sha256,
                                                    'assets': args.assets}, indent=2) + '\n')


# --- live library ------------------------------------------------------------------------------

def static_check(root, model, *, quantize=True):
    """Reasons this model cannot be derived, found without Blender (empty list = derivable)."""
    reasons = []
    if not model.endswith('.glb'):
        return ['not a GLB']
    try:
        doc, buffers, _ = read_glb(root / model)
        textured = 0
        for mesh in doc['meshes']:
            for primitive in mesh['primitives']:
                require(primitive.get('mode', 4) == 4, 'non-triangle primitive')
                require(not primitive.get('targets'), 'morph targets')
                textured += has_pbr_maps(doc) or display_texture(doc, primitive) is not None
        require(doc['meshes'], 'no meshes')
        if not textured:
            require(not doc.get('images') and not doc.get('textures'), 'Untextured model has unused texture resources')
        for node in doc['nodes']:
            require(not quantize or 'mesh' not in node or not any(k in node for k in ('matrix', 'translation', 'rotation', 'scale')),
                    'transformed mesh node')
    except (ValueError, KeyError, FileNotFoundError) as error:
        reasons.append(f'{type(error).__name__}: {error}')
    return reasons


def receipt_current(root, model, lossy, args):
    """True when `lossy` and its receipt match the current model bytes and settings."""
    receipt_path = root / (lossy + '.receipt.json')
    if not (root / lossy).exists() or not receipt_path.exists():
        return False
    receipt = json.loads(receipt_path.read_text())
    return (receipt.get('source') == sha(root / model) and receipt.get('output') == sha(root / lossy)
            and receipt.get('settings') == settings(args))


class LibraryLock:
    """The library publisher's advisory lock (`flock` on `.publication.lock`)."""
    def __init__(self, root):
        self.path = root / '.publication.lock'

    def __enter__(self):
        import fcntl
        self.handle = self.path.open('a+')
        fcntl.flock(self.handle, fcntl.LOCK_EX)
        return self

    def __exit__(self, *_):
        self.handle.close()


def atomic_write(path, data):
    temporary = path.with_name(f'.{path.name}.{uuid.uuid4().hex}.tmp')
    try:
        temporary.write_bytes(data)
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def publish_one(root, run, entry_id, model, files, fields, source_sha, record):
    """Copy staged derivative files into the library and set index fields, under the lock.

    `files` maps library-relative paths to staged files; `fields` maps index keys (lossy_model,
    preview_model) to values. Previous files are kept in the run backup for `rollback`.
    """
    with LibraryLock(root):
        index_path = root / 'index.json'
        index = discover_asset_index(root)
        entries = [e for e in index['assets'] if e['id'] == entry_id]
        require(len(entries) == 1, f'Asset left the index during derivation: {entry_id}')
        entry = entries[0]
        require(sha(root / model) == source_sha, f'Model changed during derivation; rerun: {model}')
        generate_asset_index(root, files=files)
        backup = run / 'backup'
        for name, staged in files.items():
            target = root / name
            if target.exists():
                saved = backup / name
                saved.parent.mkdir(parents=True, exist_ok=True)
                if not saved.exists():
                    shutil.copyfile(target, saved)
            record['files'].append({'path': name, 'previous_sha256': sha(target) if target.exists() else None})
            target.parent.mkdir(parents=True, exist_ok=True)
            atomic_write(target, Path(staged).read_bytes())
            record['files'][-1]['sha256'] = sha(target)
        for key, value in fields.items():
            if model == entry['model'] and entry.get(key) != value:
                record['index'].append({'id': entry_id, 'field': key, 'previous': entry.get(key), 'value': value})
                entry[key] = value
        write_asset_index(root)
        record['index_sha256'] = sha(index_path)
        (run / 'record.json').write_text(json.dumps(record, indent=2) + '\n')


def main_library(args):
    """Derive and publish lossy models and previews for live library assets (dry run unless --apply)."""
    root = args.root.resolve(strict=True)
    index = discover_asset_index(root)
    maps = {m.lower() for m in args.maps} if args.maps else None
    selected = [e for e in index['assets'] if (maps is None or e['source_map'].lower() in maps)
                and (not args.assets or e['id'] in args.assets)]
    require(selected, 'No assets selected')
    plan, by_map = [], {}
    for entry in selected:
        model = entry['model']
        lossy = str(Path(model).parent / lossy_name(Path(model)))
        preview = str(Path(model).parent / preview_name(Path(model)))
        reasons = static_check(root, model, quantize=not args.no_quantize)
        lossy_state = 'refused' if reasons else 'current' if not args.force and receipt_current(root, model, lossy, args) else 'derive'
        # A re-derived lossy model always gets a new preview; a refused one previews the model.
        preview_source = model if reasons else lossy
        preview_state = ('derive' if lossy_state == 'derive' or args.force or not preview_current(root, preview_source, preview)
                         else 'current')
        plan.append({'id': entry['id'], 'map': entry['source_map'], 'model': model, 'lossy_model': lossy,
                     'preview_model': preview, 'preview_source': preview_source, 'state': lossy_state,
                     'preview_state': preview_state, 'reasons': reasons, 'model_bytes': (root / model).stat().st_size,
                     'preview_bytes_before': (root / entry['preview_model']).stat().st_size
                     if entry.get('preview_model') and (root / entry['preview_model']).exists() else None})
        counts = by_map.setdefault(entry['source_map'], {'derive': 0, 'current': 0, 'refused': 0, 'preview_derive': 0,
                                                          'preview_current': 0, 'previews_before': 0,
                                                          'model_bytes': 0, 'preview_bytes_before': 0})
        counts[lossy_state] += 1
        counts['preview_' + preview_state] += 1
        counts['model_bytes'] += plan[-1]['model_bytes']
        if plan[-1]['preview_bytes_before'] is not None:
            counts['previews_before'] += 1
            counts['preview_bytes_before'] += plan[-1]['preview_bytes_before']
    run = args.run.resolve()
    require(ROOT.resolve() in run.parents, f'Run records must stay under {ROOT}')
    run.mkdir(parents=True, exist_ok=True)
    (run / 'plan.json').write_text(json.dumps({'root': str(root), 'settings': settings(args), 'by_map': by_map,
                                               'plan': plan}, indent=2) + '\n')
    for source_map, counts in sorted(by_map.items()):
        print(f'PLAN {source_map}: {counts}', flush=True)
    for item in plan:
        if item['state'] == 'refused':
            print(f'REFUSED {item["model"]}: {"; ".join(item["reasons"])}', flush=True)
    if not args.apply:
        print(f'Dry run only; plan written to {run / "plan.json"}', flush=True)
        return
    sys.path.insert(0, str(RENDER_SLOTS))
    from render_slots import acquire
    acquire()
    record_path = run / 'record.json'
    record = json.loads(record_path.read_text()) if record_path.exists() else {
        'root': str(root), 'files': [], 'index': [], 'reports': {}, 'failures': {}}
    if not (run / 'backup/index.json').exists():
        with LibraryLock(root):
            (run / 'backup').mkdir(parents=True, exist_ok=True)
            shutil.copyfile(root / 'index.json', run / 'backup/index.json')
    for item in plan:
        if item['state'] != 'derive' and item['preview_state'] != 'derive':
            continue
        lossy_done = item['state'] != 'derive' or (receipt_current(root, item['model'], item['lossy_model'], args)
                                                     and not args.force)
        if lossy_done and preview_current(root, item['preview_source'], item['preview_model']) and not args.force:
            continue  # Resumed run: already published.
        stage = run / 'stage'
        work = run / 'work' / Path(item['model']).with_suffix('')
        for path in (stage / Path(item['model']).parent, work):
            if path.exists():
                shutil.rmtree(path)
        try:
            source_sha = sha(root / item['model'])
            files, fields, row = {}, {}, {}
            if not lossy_done:
                report = derive(item['id'], (root / item['model']).resolve(strict=True), stage / item['lossy_model'], args, work)
                require(report['source_sha256'] == source_sha, 'Model changed while deriving')
                row = summary_row(report)
                for name in (item['lossy_model'], item['lossy_model'] + '.receipt.json'):
                    files[name] = stage / name
                fields['lossy_model'] = item['lossy_model']
            # The preview source is the staged lossy model when one was just derived.
            source_path = files.get(item['preview_source'], root / item['preview_source'])
            info = write_preview(source_path, item['preview_source'], stage / item['preview_model'])
            for name in (item['preview_model'], item['preview_model'] + '.receipt.json'):
                files[name] = stage / name
            fields['preview_model'] = item['preview_model']
            row['preview'] = {'bytes': info['bytes'], 'edge': info['edge'], 'before': item['preview_bytes_before']}
            publish_one(root, run, item['id'], item['model'], files, fields, source_sha, record)
            record['reports'][item['model']] = row
            record['failures'].pop(item['model'], None)
            print(f'LOSSY {item["model"]}: {json.dumps(row)}', flush=True)
        except Exception as error:  # Record and continue; a rerun resumes.
            record['failures'][item['model']] = f'{type(error).__name__}: {error}'
            print(f'LOSSY FAILED {item["model"]}: {record["failures"][item["model"]]}', flush=True)
            if args.fail_fast:
                raise
        finally:
            record_path.write_text(json.dumps(record, indent=2) + '\n')
    require(not record['failures'], f'Failed models: {sorted(record["failures"])}')


def main_rollback(args):
    """Restore the index and lossy files recorded by a library run."""
    run = args.run.resolve(strict=True)
    record = json.loads((run / 'record.json').read_text())
    root = Path(record['root'])
    with LibraryLock(root):
        index_path = root / 'index.json'
        require(sha(index_path) == record.get('index_sha256'), 'Index changed after the run; refusing blind rollback')
        index = discover_asset_index(root)
        entries = {e['id']: e for e in index['assets']}
        for change in reversed(record['index']):
            entry = entries[change['id']]
            key = change['field']
            require(entry.get(key) == change['value'], f'{key} changed since the run: {change["id"]}')
            if change['previous'] is None:
                entry.pop(key)
            else:
                entry[key] = change['previous']
        restored_files = {change['path']: run / 'backup' / change['path']
                          for change in record['files'] if change['previous_sha256'] is not None}
        restored_files.update({change['path']: None for change in record['files']
                               if change['previous_sha256'] is None})
        generate_asset_index(root, files=restored_files)
        for change in reversed(record['files']):
            target = root / change['path']
            require(not target.exists() or sha(target) == change['sha256'], f'File changed since the run: {change["path"]}')
            if change['previous_sha256'] is None:
                target.unlink(missing_ok=True)
            else:
                atomic_write(target, (run / 'backup' / change['path']).read_bytes())
        write_asset_index(root)
    print(f'Rolled back {len(record["index"])} index entries and {len(record["files"])} files', flush=True)


def default_settings(**overrides):
    """Settings namespace with the CLI defaults (publisher calls use these)."""
    parser = argparse.ArgumentParser()
    add_settings(parser)
    args = parser.parse_args([])
    for key, value in overrides.items():
        require(hasattr(args, key), f'Unknown lossy setting: {key}')
        setattr(args, key, value)
    return args


def verify_derivatives(root, *, index=None):
    """Problems with the lossy/preview derivatives an index declares (empty list = consistent)."""
    root = Path(root)
    index = discover_asset_index(root) if index is None else index
    problems = lossy_problems(root, index)
    for entry in index['assets']:
        model = entry['model']
        if entry.get('preview_model'):
            receipt_path = root / (entry['preview_model'] + '.receipt.json')
            if not (root / entry['preview_model']).is_file() or not receipt_path.is_file():
                problems.append(f'{entry["id"]}: preview or receipt missing')
            else:
                receipt = json.loads(receipt_path.read_text())
                source = receipt.get('source_model', model)
                if source not in (model, entry.get('lossy_model')) or receipt.get('source') != sha(root / source):
                    problems.append(f'{entry["id"]}: preview receipt does not bind the current model or lossy model')
                if receipt.get('output') != sha(root / entry['preview_model']):
                    problems.append(f'{entry["id"]}: preview bytes differ from its receipt')
    return problems


def refresh_derivatives(root, work, *, lossy=True, previews=True, ids=None, settings_args=None, log=print):
    """Bring a staged catalog's lossy models and previews up to date with its final model bytes.

    Runs inside Blender after every model byte is final. For each selected entry: keep a
    current lossy model (receipt binds the model bytes and settings), otherwise derive it into
    `<dir>/lossy.glb` + receipt and set `lossy_model`; entries refused by `static_check` keep no
    lossy model and are reported. `lossy=False` removes `lossy_model` fields instead, so nothing
    points at a stale derivative. Then each entry's `<dir>/preview.glb` is rebuilt when stale
    from its lossy model (or its model without one) and `preview_model` set; the preview receipt
    binds that source's bytes. Returns a report; index.json is rewritten atomically.
    """
    root, work = Path(root).resolve(strict=True), Path(work)
    args = settings_args or default_settings()
    index_path = root / 'index.json'
    index = discover_asset_index(root)
    report = {'derived': [], 'current': [], 'refused': {}, 'removed': [], 'lossy': lossy, 'previews': previews}
    for entry in index['assets']:
        if ids is not None and entry['id'] not in ids:
            continue
        model = entry['model']
        target = str(Path(model).parent / lossy_name(Path(model)))
        if not lossy:
            previous = entry.pop('lossy_model', None)
            if previous is not None:
                (root/previous).unlink(missing_ok=True)
                (root/(previous+'.receipt.json')).unlink(missing_ok=True)
                report['removed'].append(entry['id'])
            continue
        reasons = static_check(root, model, quantize=not args.no_quantize)
        if reasons:
            previous = entry.pop('lossy_model', None)
            if previous is not None:
                (root/previous).unlink(missing_ok=True)
                (root/(previous+'.receipt.json')).unlink(missing_ok=True)
            report['refused'][entry['id']] = reasons
            log(f'LOSSY REFUSED {entry["id"]}: {"; ".join(reasons)}')
            continue
        if receipt_current(root, model, target, args):
            report['current'].append(entry['id'])
        else:
            asset_work = work / Path(model).with_suffix('')
            if asset_work.exists():
                shutil.rmtree(asset_work)
            row = summary_row(derive(entry['id'], (root / model).resolve(strict=True), root / target, args, asset_work))
            report['derived'].append(row)
            log(f'LOSSY {model}: {json.dumps(row)}')
        entry['lossy_model'] = target
    if previews:
        for entry in index['assets']:
            if ids is not None and entry['id'] not in ids:
                continue
            source = entry.get('lossy_model', entry['model'])
            preview = str(Path(entry['model']).parent / preview_name(Path(entry['model'])))
            if not preview_current(root, source, preview):
                info = write_preview(root / source, source, root / preview)
                report.setdefault('previews_built', []).append({'id': entry['id'], 'bytes': info['bytes'], 'edge': info['edge']})
            entry['preview_model'] = preview
    problems = verify_derivatives(root, index=index)
    require(not problems, f'Derivatives inconsistent after refresh: {problems}')
    write_asset_index(root)
    return report


def main_refresh(args):
    """Refresh a staged (non-live) catalog, e.g. a publication stage, from the command line."""
    root = args.root.resolve(strict=True)
    require(ROOT.resolve() in root.parents, f'refresh only edits staged catalogs under {ROOT}; use `library` for live ones')
    sys.path.insert(0, str(RENDER_SLOTS))
    from render_slots import acquire
    acquire()
    report = refresh_derivatives(root, args.work, lossy=not args.no_lossy, previews=not args.no_previews,
                                 ids=args.assets, settings_args=args)
    print(json.dumps({k: v for k, v in report.items() if k != 'derived'} | {'derived': len(report['derived'])}), flush=True)


def add_settings(parser):
    parser.add_argument('--density-coverage', type=float, default=0.95,
                        help='Fraction of surface area that should meet the texture density target (0 < value <= 1)')
    parser.add_argument('--density', type=float, default=1.0, help='Target weakest-axis texels per map pixel')
    parser.add_argument('--nearest-density', type=float, default=2.0,
                        help='Target texels per map pixel on nearest-filtered (source pixel sampling) materials')
    parser.add_argument('--multiple', type=int, default=16)
    parser.add_argument('--min-size', type=int, default=32)
    parser.add_argument('--max-size', type=int, default=4096)
    parser.add_argument('--max-atlas-expansion', type=float, default=4.0,
                        help='Retain source textures when a rebake would exceed this multiple of their total pixels')
    parser.add_argument('--quality', type=int, default=80)
    parser.add_argument('--speed', type=int, default=6)
    parser.add_argument('--angle-limit', type=float, default=66.0)
    parser.add_argument('--pack-margin-px', type=float, default=8.0)
    parser.add_argument('--pack-shape', choices=('AABB', 'CONVEX', 'CONCAVE'), default='AABB',
                        help='Pack Islands shape; CONCAVE packs ~2%% tighter but takes ~30-100 s per pack')
    parser.add_argument('--bake-margin', type=int, default=8)
    parser.add_argument('--validate', action='store_true',
                        help='Also render published vs lossy from 8 views and report colour differences (evidence only)')
    parser.add_argument('--render-scale', type=float, default=2.0, help='Validation pixels per map pixel')
    parser.add_argument('--render-max', type=int, default=1024, help='Validation tile size cap')
    parser.add_argument('--reencode-utilization', type=float, default=0.9,
                        help='Keep the published layout when one image is at least this full')
    parser.add_argument('--reuse-ratio', type=float, default=1.25,
                        help='Keep the published layout when faces cover an image this many times over (tiling)')
    parser.add_argument('--keep-normals', action='store_true',
                        help='Keep NORMAL (by default it is dropped on unlit textured primitives)')
    parser.add_argument('--texture-file', action='store_true',
                        help='Write the AVIF as a sibling <lossy>.avif referenced by URI instead of embedding it')
    parser.add_argument('--no-quantize', action='store_true', help='Keep float vertex attributes')
    parser.add_argument('--normal-bits', type=int, choices=(8, 16), default=8,
                        help='KHR_mesh_quantization normal precision')
    parser.add_argument('--fail-fast', action='store_true')


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest='command', required=True)
    export = commands.add_parser('export-worker')
    export.add_argument('--worker', type=Path, required=True)
    export.add_argument('--worker-sha256', required=True)
    export.add_argument('--assets', nargs='+', required=True)
    export.add_argument('--output', type=Path, required=True)
    export.add_argument('--map-name', default='lincoln', help='Map whose "<map> Working" collection is exported')
    derive_parser = commands.add_parser('derive')
    derive_parser.add_argument('--source-root', type=Path, required=True)
    derive_parser.add_argument('--assets', nargs='+', required=True)
    derive_parser.add_argument('--output', type=Path, required=True, help='Lossy root (library-like <id>/ layout)')
    add_settings(derive_parser)
    library = commands.add_parser('library', help='Derive + publish into a live library (dry run unless --apply)')
    library.add_argument('--root', type=Path, required=True, help='Library 3d-assets directory')
    library.add_argument('--maps', nargs='+', help='source_map names (default: all)')
    library.add_argument('--assets', nargs='+', help='Restrict to these asset ids')
    library.add_argument('--run', type=Path, required=True, help='Run record directory (plan, backups, reports)')
    library.add_argument('--apply', action='store_true')
    library.add_argument('--force', action='store_true', help='Rebuild even when receipts are current')
    add_settings(library)
    rollback = commands.add_parser('rollback')
    rollback.add_argument('--run', type=Path, required=True)
    refresh = commands.add_parser('refresh', help='Update lossy models and previews of a staged catalog')
    refresh.add_argument('--root', type=Path, required=True, help='Staged 3d-assets directory (contains index.json)')
    refresh.add_argument('--work', type=Path, required=True, help='Validation reports directory')
    refresh.add_argument('--assets', nargs='+', help='Restrict to these asset ids')
    refresh.add_argument('--no-lossy', action='store_true', help='Remove lossy_model fields instead of deriving')
    refresh.add_argument('--no-previews', action='store_true')
    add_settings(refresh)
    args = parser.parse_args(argv)
    {'export-worker': main_export_worker, 'derive': main_derive, 'library': main_library,
     'rollback': main_rollback, 'refresh': main_refresh}[args.command](args)


if __name__ == '__main__':
    main(sys.argv[sys.argv.index('--') + 1:])
