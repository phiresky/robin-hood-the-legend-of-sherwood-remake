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


def _reconcile(generated, manifest, predicted=None):
    from texture_reconciliation import reconcile_tile
    reviewed = Path(manifest['reviewed_packet'])
    source = _read(reviewed/'textured.png')
    predicted = generated if predicted is None else predicted
    if predicted.shape != generated.shape:
        raise ValueError('Raw reconciliation prediction dimensions differ')
    corrected = generated.copy()
    height = len(generated)
    for view in manifest['views']:
        crop = view['crop']
        left, bottom = crop['left'], height-crop['top']-crop['height']
        rows = slice(bottom,bottom+crop['height'])
        cols = slice(left,left+crop['width'])
        known = _read(reviewed/'views'/f"view-{view['index']}-known.png")[:,:,0] > .5
        corrected[rows,cols] = reconcile_tile(generated[rows,cols], source[rows,cols],
                                               predicted[rows,cols], known,
                                               fade_pixels=manifest.get("texture_reconciliation_fade_pixels", 24),
                                               gain_mode=manifest.get("texture_reconciliation_gain_mode", "rgb"),
                                               minimum_gain=manifest.get("texture_reconciliation_minimum_gain", .4))
    return corrected


def apply(manifest_path, image_path, output_dir, *, texels_per_unit=2, map_name=None, reconciliation_reference=None):
    """Recompute layer-aware source ownership, then fill only unowned texels.

    Geometry is unchanged. Existing generated slots do not block a fresh bake:
    all assigned materials are reset by source_projection_bake, while unused
    slots are harmless. A generated view must see the actual surface and its
    approved input mask must designate that projected pixel as unknown.
    """
    manifest_path = Path(manifest_path).resolve()
    manifest = json.loads(manifest_path.read_text())
    if map_name is None:
        map_name = manifest['collection_name'].removesuffix(' Working')
    approval = json.loads((manifest_path.parent/'approval.json').read_text())
    input_hash = hashlib.sha256((manifest_path.parent/'input.png').read_bytes()).hexdigest()
    if approval.get('status') != 'approved' or approval.get('input_sha256') != input_hash:
        raise ValueError('Exact input sheet has not been approved')
    if approval.get('asset_id') != manifest['asset_id']:
        raise ValueError('Approval asset does not match camera manifest')
    reviewed = Path(manifest['reviewed_packet'])
    from reviewed_input_contract import validate as validate_reviewed_input
    validate_reviewed_input(manifest_path, lambda path: _read(path)[::-1])
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
    generated_support = np.ones(generated.shape[:2], dtype=bool)
    support_contract = manifest.get('texture_generated_support_mask')
    if support_contract is not None:
        if set(support_contract) != {'path', 'sha256'}:
            raise ValueError('Invalid generated support mask contract')
        support_path = Path(support_contract['path'])
        if hashlib.sha256(support_path.read_bytes()).hexdigest() != support_contract['sha256']:
            raise ValueError('Generated support mask changed')
        from generated_surface_support import edit_support
        generated_support &= edit_support(_read(support_path), mask)
    background_limit = manifest.get('texture_generated_background_max_rgb')
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
        if background_limit is not None:
            from generated_surface_support import support
            generated_support[rows,cols] &= support(generated[rows,cols], solid, background_limit)
    generated = _reconcile(generated, manifest, _read(reconciliation_reference) if reconciliation_reference else None)
    output = Path(output_dir).resolve()
    output.mkdir(parents=True, exist_ok=False)
    objects = [obj for obj in bpy.data.collections[manifest['collection_name']].all_objects
               if obj.type == 'MESH' and not obj.hide_render and obj.get('asset_group') == manifest['asset_id']]
    if not objects:
        raise ValueError('Approved asset is absent')
    from reviewed_texture_scope import displayed_objects, selected_layers, layer_objects
    objects = displayed_objects(manifest, objects)
    scope = manifest.get('texture_receiver_object_names')
    targets = objects if scope is None else [obj for obj in objects if obj.name in set(scope)]
    if not targets or (scope is not None and set(scope) != {obj.name for obj in targets}):
        raise ValueError('Texture receiver scope is empty or includes absent/foreign meshes')
    repair_policy = manifest.get('texture_inferred_gap_repair')
    if repair_policy is not None:
        from inferred_gap_repair import validate_policy
        validate_policy(repair_policy, [obj.name for obj in targets], {obj.name:len(obj.data.polygons) for obj in targets})
        for obj in targets:
            for face in repair_policy.get('face_bottom_bands',{}).get(obj.name,{}):
                if int(face)>=len(obj.data.polygons):raise ValueError('Basal override names a nonexistent face')
    nodes = {obj.get('source_node') for obj in targets}
    two_sided = set(manifest.get('texture_two_sided_object_names', []))
    if two_sided - {obj.name for obj in targets}:
        raise ValueError('Two-sided texture scoring must be inside explicit receiver scope')
    from texture_face_sampling import face_sampling, eligibility
    face_policy = face_sampling(manifest, {obj.name: len(obj.data.polygons) for obj in targets})
    from generated_visibility import bounded_faces, far_plane, bounded_origin, visible_sample, background_faces
    finite_faces = bounded_faces(manifest, {obj.name: len(obj.data.polygons) for obj in targets})
    filtered_faces = background_faces(manifest, {obj.name: len(obj.data.polygons) for obj in targets})
    from physical_opacity import OpacityRegistry
    opacity = OpacityRegistry()
    vertices, triangles, triangle_owners = [], [], []
    for obj in objects:
        offset = len(vertices)
        vertices.extend(obj.matrix_world @ vertex.co for vertex in obj.data.vertices)
        obj.data.calc_loop_triangles()
        for triangle in obj.data.loop_triangles:
            opacity.add(obj, obj.data, triangle)
        triangles.extend(tuple(offset+i for i in tri.vertices) for tri in obj.data.loop_triangles)
        triangle_owners.extend((obj.name,tri.polygon_index) for tri in obj.data.loop_triangles)
    tree = opacity.wrap(BVHTree.FromPolygons(vertices, triangles, all_triangles=True))
    cameras = []
    for view in manifest['views']:
        matrix = Matrix(view['camera_matrix_world'])
        cameras.append((view, matrix.inverted(), matrix.to_3x3() @ Vector((0,0,1))))
    finite_planes = {view['index']: far_plane(vertices,direction) for view,_,direction in cameras} if finite_faces else {}
    from texture_view_selection import policy, ordered, eligible, preferred_views, SINGLE
    selection = policy(manifest)
    preferred = preferred_views(manifest, {obj.name:len(obj.data.polygons) for obj in targets})
    stats = {'generated_texels_including_padding':0, 'unfilled_texels_including_padding':0,
             'protected_texels_including_padding':0, 'views': {str(v['index']):0 for v,_,_ in cameras}}

    visibility_samples={}

    def sample(obj, normal, positions, accepted, colors, *, face_index, record_statistics=True):
        if record_statistics:
            stats['protected_texels_including_padding'] += int(accepted.sum())
        remaining = ~accepted.copy()
        best_scores = np.full(len(positions), -np.inf)
        weights = np.zeros(len(positions))
        blended = np.zeros((len(positions),3))
        # Selection is per texel: occlusion can vary within a single polygon.
        minimum_cosine, face_two_sided = eligibility(face_policy, obj.name, face_index,
                                                    legacy_two_sided=obj.name in two_sided)
        candidates = ordered((((abs(normal.dot(direction)) if face_two_sided else normal.dot(direction)), view, inverse, direction)
                             for view,inverse,direction in cameras), selection, preferred.get((obj.name,face_index)))
        for score, view, inverse, direction in candidates:
            if score <= minimum_cosine:
                continue
            indices = np.flatnonzero(eligible(accepted, remaining, score, best_scores, selection))
            if not len(indices):
                continue
            local = positions[indices] @ np.asarray(inverse.to_3x3()).T + np.asarray(inverse.translation)
            from texture_camera import orthographic_extents
            crop = view['crop']
            horizontal, vertical = orthographic_extents(view)
            px = crop['left']+(.5+local[:,0]/horizontal)*crop['width']
            py = height-crop['top']-(.5-local[:,1]/vertical)*crop['height']
            ix, iy = np.floor(px).astype(int), np.floor(py).astype(int)
            in_tile = ((ix>=crop['left']) & (ix<crop['left']+crop['width']) &
                       (iy>=height-crop['top']-crop['height']) & (iy<height-crop['top']))
            for k in np.flatnonzero(in_tile):
                x,y = ix[k],iy[k]
                if mask[y,x,3] >= .5:
                    continue
                point = Vector(positions[indices[k]])
                origin = (Vector(bounded_origin(point,direction,finite_planes[view['index']]))
                          if (obj.name,face_index) in finite_faces else point+direction*100000)
                hit,_,hit_index,_ = tree.ray_cast(origin, -direction)
                if (obj.name,face_index) in finite_faces and not visible_sample(
                        hit,point,triangle_owners[hit_index] if hit_index is not None else None,(obj.name,face_index)):
                    continue
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
                if support_contract is not None or (background_limit is not None and (filtered_faces is None or (obj.name,face_index) in filtered_faces)):
                    from generated_surface_support import filtered_color
                    color = filtered_color(
                        [generated[y0,x0,:3], generated[y0,x1,:3], generated[y1,x0,:3], generated[y1,x1,:3]],
                        [(1-ax)*(1-ay), ax*(1-ay), (1-ax)*ay, ax*ay],
                        [generated_support[y0,x0], generated_support[y0,x1], generated_support[y1,x0], generated_support[y1,x1]])
                    if color is None:
                        continue
                if (obj.name,face_index) in finite_faces:
                    key=(obj.name,face_index,view['index'])
                    witnesses=visibility_samples.setdefault(key,[])
                    if len(witnesses)<3:
                        witnesses.append(dict(world=list(point),sheet_pixel=[float(px[k]),float(py[k])],reconciled_rgb=[float(v) for v in color],score=float(score),first_hit_error=float((hit-point).length)))
                sample_index = indices[k]
                if not np.isfinite(best_scores[sample_index]):
                    best_scores[sample_index] = score
                weight = 1.0 if selection == SINGLE else max(0,score-best_scores[sample_index]+.12)**2
                blended[sample_index] += color*weight
                weights[sample_index] += weight
                remaining[indices[k]] = False
                if record_statistics:
                    stats['views'][str(view['index'])] += 1
        filled = weights>0
        colors[filled,:3] = blended[filled]/weights[filled,None]
        if record_statistics:
            stats['generated_texels_including_padding'] += int((~accepted & ~remaining).sum())
            stats['unfilled_texels_including_padding'] += int(remaining.sum())
        return filled

    reports = []; assigned_objects=set()
    for index, layer in enumerate(selected_layers(manifest)):
        layer_targets=layer_objects(layer,targets)
        receivers=sorted({obj.get('source_node') for obj in layer_targets})
        if not layer_targets:
            continue
        assigned_objects.update(obj.name for obj in layer_targets)
        repair_names = [obj.name for obj in layer_targets if repair_policy and obj.name in repair_policy['receiver_objects']]
        layer_repair = {**repair_policy, 'receiver_objects':repair_names} if repair_names else None
        if layer_repair:
            for key in ('receiver_faces','face_bottom_bands','face_distance_limits','face_component_limits','face_coordinate_bands'):
                if key in layer_repair:
                    layer_repair[key]={name:value for name,value in layer_repair[key].items() if name in repair_names}
        if layer_repair is not None and 'face_bottom_bands' in layer_repair:
            layer_repair['face_bottom_bands']={name:faces for name,faces in layer_repair['face_bottom_bands'].items() if name in repair_names}
        reports.append(bake(map_name,layer['source_path'], output/f'layer-{index}.json',
                            collection_name=manifest['collection_name'],
                            receiver_nodes=receivers, occluder_nodes=layer['occluder_nodes'],
                            projection_label=layer['projection_label'] if mask_manifest else f'approved-generated-{index}',
                            texels_per_unit=texels_per_unit, preserve_authored=False,
                            hidden_sampler=sample, hidden_sampler_receives_face=True, source_mask_manifest=mask_manifest,
                            projection_region=layer.get('projection_region'),
                            receiver_components=[selector for selector in layer.get('receiver_components', [])
                                                 if selector['source_node'] in receivers],
                            receiver_object_names=[obj.name for obj in layer_targets],
                            receiver_face_indices=manifest.get('texture_receiver_face_indices'),
                            material_suffix=manifest.get('texture_material_suffix'),
                            exclude_occluder_components=layer.get('exclude_occluder_components'),
                            provenance_directory=output/f'provenance-{index}', inferred_gap_repair=layer_repair))
    assigned = {node for report in reports for node in report['receiver_nodes']}
    if assigned != nodes or assigned_objects != {obj.name for obj in targets}:
        raise ValueError('Not all approved asset receiver objects received a selected source projection layer')
    for obj in targets:
        for face in obj.data.polygons:
            face_scope = manifest.get('texture_receiver_face_indices')
            if face_scope is not None and face.index not in face_scope.get(obj.name, []):
                continue
            mat = obj.data.materials[face.material_index]
            if not mat or not mat.get('source_ownership_bake'):
                continue
            mat['generated_source_sha256'] = image_hash
            mat['generated_camera_manifest'] = str(manifest_path)
            mat['generated_approved_input_sha256'] = input_hash
            if mask_manifest:
                mat['generated_source_mask_manifest'] = mask_manifest
                mat['generated_source_mask_evidence_sha256'] = hashlib.sha256(json.dumps(manifest['source_mask_evidence'],sort_keys=True).encode()).hexdigest()
    from fill_physical_foliage import fill as fill_foliage
    foliage = fill_foliage(targets, sample, manifest.get('texture_receiver_face_indices'), image_hash,
                           subpixels=manifest.get('texture_foliage_subpixel_sampling', False),
                           sample_grid=manifest.get('texture_foliage_sample_grid', 0),
                           edge_fill_radius=manifest.get('texture_foliage_edge_fill_radius', 0))
    # Grid samples include texels absent from the centre-sampled counters.
    # Report them separately; per-material foliage counts are authoritative.
    stats['foliage_extrapolated_texels'] = sum(row['extrapolated'] for row in foliage)
    stats['foliage_grid_generated_texels'] = sum(row['grid_generated'] for row in foliage)
    subpixel_fills = sum(row['subpixel_generated'] for row in foliage)
    stats['generated_texels_including_padding'] += subpixel_fills
    stats['unfilled_texels_including_padding'] -= subpixel_fills
    repaired_count = sum(face['repaired_texels'] for layer in reports for obj in layer['objects'] for face in obj.get('inferred_gap_repairs', []))
    if repair_policy is not None:
        if repaired_count > repair_policy['max_total_texels']:
            raise ValueError('Gap repair exceeds cross-layer texel budget')
        stats['extrapolated_texels_including_padding'] = repaired_count
        stats['unfilled_texels_including_padding'] -= repaired_count
        if stats['unfilled_texels_including_padding'] < 0:raise ValueError('Gap repair provenance count mismatch')
    report = {'asset_id':manifest['asset_id'], 'input_sha256':input_hash,'generated_sha256':image_hash,
              'texture_receiver_object_names':[obj.name for obj in targets],
              'texture_two_sided_object_names':sorted(two_sided),
              'texture_generated_face_sampling':manifest.get('texture_generated_face_sampling', {}),
              'generated_image':str(Path(image_path).resolve()),
              'source_mask_manifest':mask_manifest,
              'source_mask_evidence':manifest.get('source_mask_evidence'),
              'source_constraint_status':reviewed_manifest.get('source_constraint_status'),
              'geometry_changed':False,'source_preservation':'Every protected source atlas texel checked byte-identical before and after hidden sampling',
              'texture_view_selection':selection,
              'generated_background_max_rgb':background_limit,
              'generated_support_mask':support_contract,
              'generated_background_rejected_pixels':int((~generated_support).sum()) if background_limit is not None else 0,
              'texture_preferred_face_views':manifest.get('texture_preferred_face_views',{}),
              'selection':('Highest facing visible single view per unknown texel; ties use projected pixel density then stable view index' if selection == SINGLE else 'Highest facing visible views; near ties blend across a 0.12 cosine band with explicit approved unknown mask'),
              'reconciliation_reference': str(Path(reconciliation_reference).resolve()) if reconciliation_reference else None,
              'reconciliation_reference_sha256': hashlib.sha256(Path(reconciliation_reference).read_bytes()).hexdigest() if reconciliation_reference else None,
              'seam_reconciliation':'Unknown colors only: low frequency RGB gain from explicit observed regions; source atlas texels remain exact',
              'reconciliation_fade_pixels':manifest.get('texture_reconciliation_fade_pixels',24),
              'reconciliation_gain_mode':manifest.get('texture_reconciliation_gain_mode','rgb'),
              'reconciliation_minimum_gain':manifest.get('texture_reconciliation_minimum_gain',.4),
              'counts':stats,'layers':reports, 'physical_foliage':foliage}
    if repair_policy is not None:report['inferred_gap_repair'] = repair_policy
    if filtered_faces is not None:report['generated_background_face_indices'] = manifest['texture_generated_background_face_indices']
    if finite_faces:
        report['generated_bounded_visibility'] = manifest['texture_generated_bounded_visibility']
        report['bounded_visibility_sample_evidence'] = dict(kind='Accepted unknown-only camera samples; near-tie samples may be blended by the recorded selection policy',samples=[dict(object=name,face=face,view=view,witnesses=rows) for (name,face,view),rows in visibility_samples.items()])
    (output/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    return report
