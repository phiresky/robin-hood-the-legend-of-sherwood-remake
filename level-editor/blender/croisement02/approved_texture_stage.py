"""Strict, read-only selection and import fingerprints for approved texture bakes."""
import json
from pathlib import Path
from evidence_io import sha, digest

FLAGS = ('geometry_unchanged', 'foreign_appearance_unchanged', 'physical_alpha_unchanged',
         'known_foliage_rgba_unchanged', 'foliage_uv_and_ownership_unchanged')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def select(decisions_path, models):
    document = json.loads(decisions_path.read_text())
    latest = {row['asset_id']: row for row in document['decisions']}
    result = {}
    for asset, decision in latest.items():
        if decision.get('scope') != 'texture' or decision.get('decision') != 'approved':
            continue
        require(asset in models, 'Approved texture has no current geometry: ' + asset)
        if 'archived_evidence' in decision:
            result[asset] = select_canopy(document, decision, models[asset])
            continue
        paths, hashes = decision['evidence_paths'], decision['evidence_sha256']
        require(set(paths) == set(hashes), 'Incomplete decision evidence: ' + asset)
        for key, path in paths.items():
            require(sha(Path(path)) == hashes[key], 'Stale approved texture evidence: ' + path)
        archive = Path(decision['archive'])
        require(json.loads((archive / 'decision.json').read_text()) == decision,
                'Archived approval differs from decision: ' + asset)
        for key, path in paths.items():
            archived = archive / (key + Path(path).suffix)
            require(sha(archived) == hashes[key], 'Archived evidence changed: ' + str(archived))
        model = Path(paths['model'])
        proof_path = model.parent / 'reopened-preservation.json'
        proof = json.loads(proof_path.read_text())
        require(proof['asset_id'] == asset and proof['status'] == 'PASS'
                and proof['reopened_preservation'] == 'PASS', 'Failed bake proof: ' + asset)
        require(all(proof.get(flag) is True for flag in FLAGS), 'Incomplete preservation proof: ' + asset)
        require(proof['model_sha256'] == sha(models[asset]), 'Texture base differs from current geometry: ' + asset)
        require(proof['candidate_model_sha256'] == hashes['model'], 'Texture model differs from bake proof: ' + asset)
        require(proof['bake_validation_sha256'] == hashes['validation'], 'Bake validation changed: ' + asset)
        for path, expected in proof['evidence_sha256'].items():
            require(sha(Path(path)) == expected, 'Bake evidence changed: ' + path)
        validation = json.loads(Path(paths['validation']).read_text())
        for path, expected in validation['source_mask_evidence'].items():
            require(sha(Path(path)) == expected, 'Source mask evidence changed: ' + path)
        review = json.loads(Path(paths['review']).read_text())
        require(review['status'] == 'ready-for-user' and review['all_eight_actual_views_inspected'] is True,
                'Actual texture review missing: ' + asset)
        require(review['baked_model_sha256'] == hashes['model']
                and review['actual_sheet_sha256'] == hashes['textured'], 'Actual texture review stale: ' + asset)
        result[asset] = dict(decision=decision, model=str(model), proof=str(proof_path),
                             proof_sha256=sha(proof_path), receiver_names=proof['receiver_names'])
    require(bool(result), 'No approved textures selected')
    return result


def select_canopy(document, decision, base):
    """Validate a gallery archive and both ordinary and retained-wood bake chains."""
    asset = decision['asset_id']
    snapshot = Path(document['snapshot'])
    require(json.loads((snapshot / 'decisions.json').read_text()) == document,
            'Archived canopy decisions changed')
    require(sha(snapshot / 'evidence.json') == decision['gallery_evidence_sha256'],
            'Archived canopy gallery evidence changed')
    hashes = decision['evidence_sha256']
    require(set(hashes) == set(decision['archived_evidence']), 'Incomplete canopy archive')
    for path, expected in hashes.items():
        require(sha(Path(path)) == expected, 'Stale approved canopy evidence: ' + path)
        require(sha(Path(decision['archived_evidence'][path])) == expected,
                'Archived canopy evidence changed: ' + path)
    candidate = Path(decision['candidate'])
    model = candidate / 'worker.blend'
    require(hashes[str(model)] == decision['model_sha256'], 'Canopy model approval mismatch')
    geometry_decision = decision['geometry_approval']
    require(geometry_decision['decision'] == 'approved' and geometry_decision['scope'] == 'geometry'
            and geometry_decision['model_sha256'] == sha(base), 'Canopy current geometry changed')
    retained = (candidate / 'preservation.json').exists()
    restored = (candidate / 'native-boundary-preservation.json').exists()
    require(not (retained and restored), 'Ambiguous texture derivative')
    proof_path = candidate / ('native-boundary-preservation.json' if restored else
                             'preservation.json' if retained else 'reopened-preservation.json')
    proof = json.loads(proof_path.read_text())
    if restored:
        require(str(proof_path) in hashes and sha(proof_path) == hashes[str(proof_path)],
                'Observed boundary restoration must be frozen in the approval')
        require(proof['status'].startswith('PASS scoped native boundary restoration')
                and proof['geometry_uv_alpha_ownership_unchanged'] is True
                and proof['other_materials_unchanged'] is True
                and proof['native_boundary_rgba_exact'] is True,
                'Failed observed boundary restoration')
    else:
        require(proof['asset_id'] == asset and proof['status'] == 'PASS'
                and proof['reopened_preservation'] == 'PASS', 'Failed canopy preservation')
    require(proof['candidate_model_sha256'] == sha(model), 'Canopy proof model changed')
    flags = ('geometry_unchanged', 'foreign_appearance_unchanged',
             'original_wood_appearance_and_uv_unchanged', 'generated_foliage_unchanged',
             'physical_alpha_native_rgba_and_ownership_unchanged') if retained else FLAGS
    require(restored or all(proof.get(flag) is True for flag in flags), 'Incomplete canopy preservation')
    base_key = 'source_model_sha256' if restored else 'original_model_sha256' if retained else 'model_sha256'
    require(proof[base_key] == sha(base),
            'Canopy proof base changed')
    for path, expected in proof['evidence_sha256'].items():
        require(sha(Path(path)) == expected, 'Canopy bake evidence changed: ' + path)
    baked = candidate.parent / 'bake-v1' if retained or restored else candidate
    bake_proof = json.loads((baked / 'reopened-preservation.json').read_text())
    require(bake_proof['status'] == 'PASS' and bake_proof['reopened_preservation'] == 'PASS'
            and all(bake_proof.get(flag) is True for flag in FLAGS), 'Failed underlying canopy bake')
    require(bake_proof['model_sha256'] == sha(base)
            and bake_proof['candidate_model_sha256'] == sha(baked / 'worker.blend'),
            'Underlying canopy bake model changed')
    if retained:
        require(proof['baked_foliage_model_sha256'] == bake_proof['candidate_model_sha256'],
                'Retained wood foliage chain changed')
    if restored:
        require(proof['parent_candidate_sha256'] == bake_proof['candidate_model_sha256'],
                'Observed boundary parent bake changed')
    for path, expected in bake_proof['evidence_sha256'].items():
        require(sha(Path(path)) == expected, 'Underlying canopy bake evidence changed: ' + path)
    validation_path = baked / 'validation.json'
    require(sha(validation_path) == bake_proof['bake_validation_sha256'], 'Canopy validation changed')
    for path, expected in json.loads(validation_path.read_text())['source_mask_evidence'].items():
        require(sha(Path(path)) == expected, 'Canopy source mask changed: ' + path)
    review_path = candidate / 'agent-material-review.json'
    review = json.loads(review_path.read_text()) if review_path.exists() else {}
    # Frozen grouped galleries bind the independent actual review and guards
    # together. Preserve older receipts rather than rewriting their schema.
    root_path = candidate / 'root-review.json'
    grouped = (str(root_path) in hashes and str(proof_path) in hashes)
    if grouped:
        root = json.loads(root_path.read_text())
        grouped = (root.get('status', '').startswith('PASS scoped ')
                   and root.get('all_eight_actual_views_inspected') is True
                   and root.get('candidate_model_sha256') == sha(model)
                   and root.get('actual_sheet_sha256') == sha(candidate / 'actual/textured.png'))
    if grouped:
        if not restored:
            require(review.get('all_eight_actual_views_inspected') is True
                    and review.get('candidate_model_sha256') == sha(model)
                    and review.get('actual_sheet_sha256') == sha(candidate / 'actual/textured.png'),
                    'Grouped saved-model review changed')
            review_proof = review.get('preservation_sha256') or review.get('reopened_preservation_sha256')
            require(review_proof == sha(proof_path), 'Grouped reviewed preservation proof changed')
        return dict(decision=decision, model=str(model), proof=str(proof_path),
                    proof_sha256=sha(proof_path), receiver_names=bake_proof['receiver_names'])
    require(not restored, 'Observed boundary restoration needs frozen independent actual review')
    review_proof = review.get('preservation_report_sha256') or review.get('reopened_preservation_sha256')
    require(review_proof == sha(proof_path), 'Canopy reviewed preservation proof changed')
    ready = review.get('ready_for_coordinator_review') is True
    if not ready and review.get('status') == 'PASS':
        root_path = candidate / 'root-review.json'
        require(str(root_path) in hashes and sha(root_path) == hashes[str(root_path)],
                'Explicit review schema requires archived independent review')
        root = json.loads(root_path.read_text())
        ready = (root.get('status') == 'PASS' and root.get('all_eight_saved_model_views_inspected') is True
                 and root.get('model_sha256') == sha(model)
                 and root.get('actual_sheet_sha256') == sha(candidate / 'actual/textured.png')
                 and root.get('preservation_report_sha256') == sha(proof_path))
    require(ready
            and review['all_eight_saved_model_views_inspected'] is True
            and review['model_sha256'] == sha(model)
            and review['actual_sheet_sha256'] == sha(candidate / 'actual/textured.png'),
            'Canopy saved-model review changed')
    receivers = bake_proof['receiver_names']
    if retained:
        require(set(receivers) == set(proof['wood_objects'] + proof['foliage_objects']),
                'Retained wood receiver scope changed')
    return dict(decision=decision, model=str(model), proof=str(proof_path),
                proof_sha256=sha(proof_path), receiver_names=receivers)


def geometry(obj):
    """Ignore only appearance and Blender's import-generated datablock names."""
    require(obj.type == 'MESH', 'Texture receiver is not a mesh')
    require(not any(m.show_render or m.show_viewport for m in obj.modifiers),
            'Texture staging requires explicit mesh geometry')
    return digest(dict(vertices=[list(v.co) for v in obj.data.vertices],
                       faces=[list(p.vertices) for p in obj.data.polygons],
                       edges=[list(e.vertices) for e in obj.data.edges],
                       matrix=[list(row) for row in obj.matrix_world],
                       source_node=obj.get('source_node'), asset_group=obj.get('asset_group'),
                       projection_component=obj.get('projection_component')))


def appearance(obj):
    from workspace_components import appearance_state
    state = appearance_state(obj)
    for material in state['materials']:
        if material is None:
            continue
        material.pop('name')
        for node in material.get('nodes', []):
            image = node.get('image')
            if image:
                require(image['packed_sha256'] is not None, 'Unpacked image cannot be staged immutably')
                for key in ('name', 'filepath', 'dirty'):
                    image.pop(key)
    return digest(state)


def inspect(selected, models):
    """Read original baked files before importing, and compare actual base meshes."""
    import bpy
    for asset, record in selected.items():
        bpy.ops.wm.open_mainfile(filepath=record['model'])
        refs = {}
        for name in record['receiver_names']:
            obj = bpy.data.objects[name]
            require(obj.get('asset_group') == asset, 'Foreign texture receiver')
            refs[name] = dict(geometry=geometry(obj), appearance=appearance(obj))
        bpy.ops.wm.open_mainfile(filepath=str(models[asset]))
        for name, ref in refs.items():
            require(geometry(bpy.data.objects[name]) == ref['geometry'], 'Baked geometry changed: ' + name)
        record['objects'] = refs
