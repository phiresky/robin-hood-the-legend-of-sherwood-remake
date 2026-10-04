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
    latest = {row['asset_id']: row for row in json.loads(decisions_path.read_text())['decisions']}
    result = {}
    for asset, decision in latest.items():
        if decision.get('scope') != 'texture' or decision.get('decision') != 'approved':
            continue
        require(asset in models, 'Approved texture has no current geometry: ' + asset)
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
