"""Bake an approved generated sheet only where original source ownership fails."""
import hashlib
import json
from pathlib import Path

import bpy
import numpy as np
from mathutils import Matrix, Vector
from mathutils.bvhtree import BVHTree

from source_projection_bake import bake


def _read(path):
    image = bpy.data.images.load(str(path), check_existing=False)
    try:
        pixels = np.empty(len(image.pixels), dtype=np.float32)
        image.pixels.foreach_get(pixels)
        return pixels.reshape(image.size[1], image.size[0], 4)
    finally:
        bpy.data.images.remove(image)


def _reconcile(generated, manifest):
    """Match low-frequency tone near source boundaries, only in inferred color.

    Explicit known masks provide evidence; colors are never used to classify
    ownership. Corrections fade out away from observed artwork so distant unseen
    walls do not inherit the brightness of an unrelated piece of source roof.
    """
    from scipy.ndimage import gaussian_filter, distance_transform_edt
    reviewed = Path(manifest['reviewed_packet'])
    source = _read(reviewed/'textured.png')
    corrected = generated.copy()
    height = len(generated)
    for view in manifest['views']:
        crop = view['crop']
        left, bottom = crop['left'], height-crop['top']-crop['height']
        rows = slice(bottom,bottom+crop['height'])
        cols = slice(left,left+crop['width'])
        known = _read(reviewed/'views'/f"view-{view['index']}-known.png")[:,:,0] > .5
        if not known.any():
            continue
        support = gaussian_filter(known.astype(float), 6)
        ratios = np.ones((*known.shape,3))
        for channel in range(3):
            observed = gaussian_filter(source[rows,cols,channel]*known, 6)
            predicted = gaussian_filter(generated[rows,cols,channel]*known, 6)
            ratios[:,:,channel] = np.clip(observed/np.maximum(predicted,.015*support+1e-8),.4,1.8)
        distance, nearest = distance_transform_edt(~known, return_indices=True)
        nearby = ratios[nearest[0],nearest[1]]
        strength = np.exp(-distance/24)[:,:,None]
        corrected[rows,cols,:3] *= 1+(nearby-1)*strength
    return np.clip(corrected,0,1)


def apply(manifest_path, image_path, output_dir, *, texels_per_unit=2, map_name=None):
    """Recompute layer-aware source ownership, then fill only unowned texels.

    Geometry is unchanged. Existing generated slots do not block a fresh bake:
    all assigned materials are reset by source_projection_bake, while unused
    slots are harmless. A generated view must see the actual surface and its
    approved input mask must designate that projected pixel as unknown.
    """
    manifest_path = Path(manifest_path).resolve()
    manifest = json.loads(manifest_path.read_text())
    if map_name is None:
        if not manifest['collection_name'].endswith(' Working'):
            raise ValueError('Supply map_name for a nonstandard working collection')
        map_name = manifest['collection_name'].removesuffix(' Working')
    approval = json.loads((manifest_path.parent/'approval.json').read_text())
    input_hash = hashlib.sha256((manifest_path.parent/'input.png').read_bytes()).hexdigest()
    if approval.get('status') != 'approved' or approval.get('input_sha256') != input_hash:
        raise ValueError('Exact input sheet has not been approved')
    if approval.get('asset_id') != manifest['asset_id']:
        raise ValueError('Approval asset does not match camera manifest')
    reviewed = Path(manifest['reviewed_packet'])
    if hashlib.sha256((reviewed/'textured.png').read_bytes()).hexdigest() != input_hash:
        raise ValueError('Reviewed source sheet changed after approval')
    if hashlib.sha256((reviewed/'views.json').read_bytes()).hexdigest() != manifest['reviewed_manifest_sha256']:
        raise ValueError('Reviewed cameras or lighting changed after approval')
    reviewed_manifest = json.loads((reviewed/'views.json').read_text())
    for key in ('source_mask_manifest', 'source_mask_evidence', 'projection_layers'):
        if reviewed_manifest.get(key) != manifest.get(key):
            raise ValueError('Approved source ownership contract dropped or changed: ' + key)
    mask_manifest = manifest.get('source_mask_manifest')
    if manifest.get('source_mask_evidence'):
        from occlusion_constraints import evidence_record
        if not mask_manifest or evidence_record(mask_manifest) != manifest['source_mask_evidence']:
            raise ValueError('Reviewed source-mask evidence changed or was dropped')
    elif mask_manifest:
        raise ValueError('Source masks require frozen evidence hashes')
    for layer in manifest['projection_layers']:
        if hashlib.sha256(Path(layer['source_path']).read_bytes()).hexdigest() != layer['source_sha256']:
            raise ValueError('Original projection artwork changed after approval')
    image_hash = hashlib.sha256(Path(image_path).read_bytes()).hexdigest()
    generated, mask = _read(image_path), _read(manifest_path.parent/'mask.png')
    height, width = generated.shape[:2]
    if generated.shape != mask.shape or [width,height] != [manifest['layout']['width'],manifest['layout']['height']]:
        raise ValueError('Generated image, mask and approved camera dimensions differ')
    for view in manifest['views']:
        known_path = reviewed/'views'/f"view-{view['index']}-known.png"
        if view.get('ownership_sha256'):
            if hashlib.sha256(known_path.read_bytes()).hexdigest() != view['ownership_sha256']:
                raise ValueError('Reviewed ownership pixels changed after approval')
        elif mask_manifest:
            raise ValueError('Masked review requires immutable ownership buffer hashes')
        known = _read(known_path)[:,:,0] > .5
        solid = _read(reviewed/'views'/f"view-{view['index']}-solid.png")[:,:,3] > 0
        crop = view['crop']
        rows = slice(height-crop['top']-crop['height'],height-crop['top'])
        cols = slice(crop['left'],crop['left']+crop['width'])
        if not np.array_equal(mask[rows,cols,3]<.5, solid & ~known):
            raise ValueError('Generated fill mask differs from reviewed source ownership')
    generated = _reconcile(generated, manifest)
    output = Path(output_dir).resolve()
    output.mkdir(parents=True, exist_ok=False)
    objects = [obj for obj in bpy.data.collections[manifest['collection_name']].all_objects
               if obj.type == 'MESH' and not obj.hide_render and obj.get('asset_group') == manifest['asset_id']]
    if not objects:
        raise ValueError('Approved asset is absent')
    nodes = {obj.get('source_node') for obj in objects}
    vertices, triangles = [], []
    for obj in objects:
        offset = len(vertices)
        vertices.extend(obj.matrix_world @ vertex.co for vertex in obj.data.vertices)
        obj.data.calc_loop_triangles()
        triangles.extend(tuple(offset+i for i in tri.vertices) for tri in obj.data.loop_triangles)
    tree = BVHTree.FromPolygons(vertices, triangles, all_triangles=True)
    cameras = []
    for view in manifest['views']:
        matrix = Matrix(view['camera_matrix_world'])
        cameras.append((view, matrix.inverted(), matrix.to_3x3() @ Vector((0,0,1))))
    stats = {'generated_texels_including_padding':0, 'unfilled_texels_including_padding':0,
             'protected_texels_including_padding':0, 'views': {str(v['index']):0 for v,_,_ in cameras}}

    def sample(obj, normal, positions, accepted, colors):
        stats['protected_texels_including_padding'] += int(accepted.sum())
        remaining = ~accepted.copy()
        best_scores = np.full(len(positions), -np.inf)
        weights = np.zeros(len(positions))
        blended = np.zeros((len(positions),3))
        # Selection is per texel: occlusion can vary within a single polygon.
        candidates = sorted(((normal.dot(direction), view, inverse, direction)
                             for view,inverse,direction in cameras), key=lambda item:item[0], reverse=True)
        for score, view, inverse, direction in candidates:
            if score <= .12:
                continue
            indices = np.flatnonzero(~accepted & (score >= best_scores-.12))
            if not len(indices):
                continue
            local = positions[indices] @ np.asarray(inverse.to_3x3()).T + np.asarray(inverse.translation)
            crop, scale = view['crop'], view['ortho_scale']
            px = crop['left']+(.5+local[:,0]/(scale*crop['width']/crop['height']))*crop['width']
            py = height-crop['top']-(.5-local[:,1]/scale)*crop['height']
            ix, iy = np.floor(px).astype(int), np.floor(py).astype(int)
            in_tile = ((ix>=crop['left']) & (ix<crop['left']+crop['width']) &
                       (iy>=height-crop['top']-crop['height']) & (iy<height-crop['top']))
            for k in np.flatnonzero(in_tile):
                x,y = ix[k],iy[k]
                if mask[y,x,3] >= .5:
                    continue
                point = Vector(positions[indices[k]])
                hit,_,_,_ = tree.ray_cast(point+direction*100000, -direction)
                if hit is None or (hit-point).length > .02:
                    continue
                # Bilinear filtering stays inside the candidate camera tile.
                fx,fy = px[k]-.5,py[k]-.5
                x0,y0 = int(np.floor(fx)),int(np.floor(fy))
                ax,ay = fx-x0,fy-y0
                x0 = max(crop['left'],min(crop['left']+crop['width']-1,x0))
                y0 = max(height-crop['top']-crop['height'],min(height-crop['top']-1,y0))
                x1 = min(crop['left']+crop['width']-1,x0+1)
                y1 = min(height-crop['top']-1,y0+1)
                color = ((generated[y0,x0,:3]*(1-ax)+generated[y0,x1,:3]*ax)*(1-ay)+
                         (generated[y1,x0,:3]*(1-ax)+generated[y1,x1,:3]*ax)*ay)
                sample_index = indices[k]
                if not np.isfinite(best_scores[sample_index]):
                    best_scores[sample_index] = score
                weight = max(0,score-best_scores[sample_index]+.12)**2
                blended[sample_index] += color*weight
                weights[sample_index] += weight
                remaining[indices[k]] = False
                stats['views'][str(view['index'])] += 1
        filled = weights>0
        colors[filled,:3] = blended[filled]/weights[filled,None]
        stats['generated_texels_including_padding'] += int((~accepted & ~remaining).sum())
        stats['unfilled_texels_including_padding'] += int(remaining.sum())

    reports = []
    for index, layer in enumerate(manifest['projection_layers']):
        receivers = sorted(nodes & set(layer['receiver_nodes']))
        if not receivers:
            continue
        reports.append(bake(map_name,layer['source_path'], output/f'layer-{index}.json',
                            receiver_nodes=receivers, occluder_nodes=layer['occluder_nodes'],
                            projection_label=layer['projection_label'] if mask_manifest else f'approved-generated-{index}',
                            texels_per_unit=texels_per_unit, preserve_authored=False,
                            hidden_sampler=sample, source_mask_manifest=mask_manifest,
                            projection_region=layer.get('projection_region')))
    assigned = {node for report in reports for node in report['receiver_nodes']}
    if assigned != nodes:
        raise ValueError('Not all approved asset nodes received a source projection layer')
    for obj in objects:
        for face in obj.data.polygons:
            mat = obj.data.materials[face.material_index]
            if not mat or not mat.get('source_ownership_bake'):
                continue
            mat['generated_source_sha256'] = image_hash
            mat['generated_camera_manifest'] = str(manifest_path)
            mat['generated_approved_input_sha256'] = input_hash
            if mask_manifest:
                mat['generated_source_mask_manifest'] = mask_manifest
                mat['generated_source_mask_evidence_sha256'] = hashlib.sha256(json.dumps(manifest['source_mask_evidence'],sort_keys=True).encode()).hexdigest()
    report = {'asset_id':manifest['asset_id'], 'input_sha256':input_hash,'generated_sha256':image_hash,
              'generated_image':str(Path(image_path).resolve()),
              'source_mask_manifest':mask_manifest,
              'source_mask_evidence':manifest.get('source_mask_evidence'),
              'source_constraint_status':reviewed_manifest.get('source_constraint_status'),
              'geometry_changed':False,'source_preservation':'Every protected source atlas texel checked byte-identical before and after hidden sampling',
              'selection':'Highest facing visible views; near ties blend across a 0.12 cosine band with explicit approved unknown mask',
              'seam_reconciliation':'Unknown colors only: low frequency RGB gain from explicit observed regions, fades over 24 image pixels; source atlas texels remain exact',
              'counts':stats,'layers':reports}
    (output/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    return report
