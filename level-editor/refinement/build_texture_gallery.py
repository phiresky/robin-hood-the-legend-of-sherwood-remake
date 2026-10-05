"""Collect visually checked texture bakes into the shared, stable-ID gallery."""
import argparse
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent / 'blender'))
from build_review_gallery import build
from review_evidence import sha
from texture_decisions import bind as bind_texture_decision
from texture_actual_evidence import actual_sheet


def validate_reconciliation_reference(validation):
    reference = validation.get('reconciliation_reference')
    digest = validation.get('reconciliation_reference_sha256')
    if bool(reference) != bool(digest):
        raise ValueError('Incomplete reconciliation reference evidence')
    if reference:
        path = Path(reference)
        if not path.is_file() or sha(path) != digest:
            raise ValueError('Reconciliation reference changed or is missing')
        guarded = {str(Path(key).resolve()): value for key, value in validation.get('evidence_sha256', {}).items()}
        if guarded.get(str(path.resolve())) != digest:
            raise ValueError('Reconciliation reference absent from guarded bake evidence')


def validate_planar_bake(experiment, validation):
    if validation.get('projection_kind') != 'planar-atlas':
        return
    if validation.get('uv_verified') is not True or validation.get('protected_changes') != 0:
        raise ValueError('Incomplete planar UV/source preservation checks')
    preparation_path = experiment / 'preparation.json'
    if sha(preparation_path) != validation.get('preparation_sha256'):
        raise ValueError('Planar preparation evidence changed')
    preparation = json.loads(preparation_path.read_text())
    for name, digest in preparation['files'].items():
        if sha(experiment / name) != digest:
            raise ValueError('Planar input evidence changed: ' + name)
    manifest = json.loads((experiment / 'views.json').read_text())
    frames = Path(manifest['reviewed_packet']) / 'views.json'
    if sha(frames) != validation.get('frame_manifest_sha256'):
        raise ValueError('Planar original eight-view cameras changed')



def artwork_reference(experiment):
    """Use the same raw context crop as the frozen model review packet.

    This supplementary reference is archived separately from texture-decision
    evidence, so adding it does not invalidate existing texture approvals.
    """
    from PIL import Image
    manifest_path = experiment / 'views.json'
    if not manifest_path.is_file():
        return []  # Legacy packets without frozen camera metadata.
    manifest = json.loads(manifest_path.read_text())
    if not manifest.get('reviewed_packet'):
        return []
    packet = Path(manifest['reviewed_packet'])
    frames_path = packet / 'views.json'
    if sha(frames_path) != manifest.get('reviewed_manifest_sha256'):
        raise ValueError('Original artwork reference camera evidence changed')
    frames = json.loads(frames_path.read_text())
    context = packet / 'context.png'
    if not context.is_file() or not frames.get('source_image') or not frames.get('context_crop'):
        return []
    source = Path(frames['source_image'])
    if sha(source) != frames['source_sha256']:
        raise ValueError('Original artwork reference source changed')
    crop = frames['context_crop']
    box = tuple(crop[key] for key in ('left', 'top', 'right', 'bottom'))
    with Image.open(source) as original, Image.open(context) as reference:
        if (not all(type(v) is int for v in box) or
                not 0 <= box[0] < box[2] <= original.width or
                not 0 <= box[1] < box[3] <= original.height):
            raise ValueError('Original artwork reference crop is invalid')
        expected = original.convert('RGBA').crop(box)
        if reference.size != expected.size or reference.convert('RGBA').tobytes() != expected.tobytes():
            raise ValueError('Original artwork reference differs from the raw source crop')
    return [{'id': 'original', 'label': 'Original artwork with surrounding context',
             'path': str(context), 'sha256': sha(context),
             'source': str(source), 'source_sha256': frames['source_sha256'], 'crop': crop}]


def candidate(experiment, map_name, *, supplemental=False):
    experiment = Path(experiment).resolve()
    review_path = experiment / 'texture-review.json'
    review = json.loads(review_path.read_text())
    expected_status = 'supplemental' if supplemental else 'ready-for-user'
    if review.get('status') != expected_status:
        raise ValueError('Texture candidate has unexpected status: ' + str(experiment))
    approval = json.loads((experiment / 'approval.json').read_text())
    bake = (experiment / review['bake']).resolve()
    generation = (experiment / review['generation']).resolve()
    validation = json.loads((bake / 'validation.json').read_text())
    report = json.loads((generation / 'generation.json').read_text())
    validate_reconciliation_reference(validation)
    validate_planar_bake(experiment, validation)
    manifest_path = experiment / 'views.json'
    generation_manifest = json.loads(manifest_path.read_text()) if manifest_path.is_file() else {}
    if 'uv-atlas' in (validation.get('projection_kind'), generation_manifest.get('projection_kind')):
        if validation.get('projection_kind') != generation_manifest.get('projection_kind'):
            raise ValueError('UV atlas generation and bake types differ')
        from uv_atlas import validate_uv_atlas_bake
        validate_uv_atlas_bake(validation, experiment, bake)
    if (review.get('all_eight_actual_views_inspected') is not True
            or review.get('status') != expected_status
            or validation.get('geometry_verified') is not True
            or report.get('changedProtected') != 0):
        raise ValueError(f'Incomplete texture review: {experiment.name}')
    actual = actual_sheet(bake, review)
    if (sha(actual) != review['actual_sheet_sha256']
            or sha(bake / 'worker.blend') != review['baked_model_sha256']
            or sha(generation / 'generated-preserved.png') != validation['generated_sha256']
            or sha(experiment / 'input.png') != approval['input_sha256']):
        raise ValueError(f'Texture review evidence changed: {experiment.name}')
    asset_id = approval['asset_id']
    item = {
        'id': asset_id, 'name': asset_id.removeprefix(map_name.lower() + '-').replace('-', ' ').title(),
        'status': 'ready-for-user', 'user_approval': 'pending',
        'notes': ['Texture approval pending; geometry was previously approved.', *review.get('notes', [])],
        'solid': str(experiment / 'solid.png'),
        'textured': str(actual), 'textured_label': 'Generated texture baked onto the actual mesh — approval candidate',
        'source_comparison': str(experiment / 'input.png'),
        'source_comparison_label': 'Approved source textures before generation',
        'source_comparison_secondary': str(generation / 'generated-preserved.png'),
        'source_comparison_secondary_label': 'Generated sheet with original pixels restored',
        'source_trace': str(generation / 'generated-raw.png'),
        'source_trace_label': ('Raw Sunburst output — inferred-color calibration only' if validation.get('reconciliation_reference') else 'Raw Sunburst output — reference only, not used for this bake'),
        'validation': str(bake / 'validation.json'), 'review': str(review_path),
    }
    if report.get('provider') == 'cached-appearance-reuse':
        item.update(
            textured_label='Existing textures transferred onto approved geometry — approval candidate',
            source_comparison_label='Approved source textures before appearance transfer',
            source_comparison_secondary_label='Cached appearance comparison with original pixels restored',
            source_trace_label='Cached donor appearance rendered in approved views — no new generation')
    references = artwork_reference(experiment)
    if references:
        item['artwork_references'] = references
    return item, review, approval


def attach_states(item, review, approval, experiment, map_name):
    from texture_decisions import IMAGE_FIELDS
    states = []
    for state in review.get('texture_states', []):
        child_path = (experiment / state['experiment']).resolve()
        child, child_review, child_approval = candidate(child_path, map_name, supplemental=True)
        if (child['id'] != item['id'] or child_approval['geometry_revision'] != approval['geometry_revision'] or
                child_review.get('texture_states') or child_review.get('material_states')):
            raise ValueError('Texture state identity/revision differs or states are nested')
        spec = {'id': state['id'], 'name': state.get('name', state['id']),
                'image_fields': list(IMAGE_FIELDS), 'report_fields': ['validation', 'review'],
                'model': str(Path(child['validation']).parent / 'worker.blend')}
        if child.get('source_trace_label', '').startswith('Cached donor appearance'):
            spec['image_labels'] = {key: child[key + '_label'] for key in spec['image_fields']
                                    if key + '_label' in child}
        prefix = 'texture_state_' + spec['id'] + '_'
        for key in (*spec['image_fields'], *spec['report_fields']):
            item[prefix + key] = child[key]
        states.append(spec)
        for reference in child.get('artwork_references', []):
            item.setdefault('artwork_references', []).append({**reference,
                'id': state['id'] + '-' + reference['id'],
                'label': state.get('name', state['id']) + ': original artwork with surrounding context'})
    for state in review.get('material_states', []):
        sheet, validation = (experiment / state['textured']).resolve(), (experiment / state['validation']).resolve()
        if sha(sheet) != state['actual_sheet_sha256'] or sha(validation) != state['validation_sha256']:
            raise ValueError('Preserved material-state evidence changed: ' + state['id'])
        qa = json.loads(validation.read_text())
        if qa.get('baked_model_sha256') != review['baked_model_sha256'] or qa.get('materials_preserved') is not True:
            raise ValueError('Material-state QA must bind baked model and preserved materials')
        spec = {'id': state['id'], 'name': state.get('name', state['id']),
                'image_fields': ['textured'], 'report_fields': ['validation']}
        prefix = 'texture_state_' + spec['id'] + '_'
        item[prefix + 'textured'], item[prefix + 'validation'] = str(sheet), str(validation)
        states.append(spec)
    if states:
        item['texture_states'] = states
        from texture_decisions import fields
        fields(item)  # Reject unsafe or duplicate identifiers before building files.


def superseded_reviews(records, map_name):
    """Hide explicitly replaced revisions without editing archived decisions."""
    excluded = {}
    for record in records:
        old = Path(record['review']).resolve()
        new = Path(record['replacement']).resolve()
        if old == new or old in excluded:
            raise ValueError('Duplicate or self-replacing texture revision')
        if sha(old) != record['review_sha256'] or sha(new) != record['replacement_sha256']:
            raise ValueError('Texture supersession evidence changed')
        old_item, _, _ = candidate(old.parent, map_name)
        new_item, _, _ = candidate(new.parent, map_name)
        if old_item['id'] != new_item['id'] or old_item['id'] != record['asset_id']:
            raise ValueError('Texture supersession must preserve asset identity')
        excluded[old] = new
    if set(excluded) & set(excluded.values()):
        raise ValueError('Texture supersession chains require an explicit final replacement')
    return excluded


def collect(experiments, output, map_name, additional_experiments=(), *, supersessions=()):
    experiments, output = Path(experiments).resolve(), Path(output).resolve()
    items = []
    decisions_path = output / 'decisions.json'
    decisions = json.loads(decisions_path.read_text())['decisions'] if decisions_path.exists() else []
    roots = {experiments, *(Path(path).resolve() for path in additional_experiments)}
    excluded = superseded_reviews(supersessions, map_name)
    for experiment in sorted({path.resolve() for root in roots for path in root.iterdir() if path.is_dir()}):
        review_path = experiment / 'texture-review.json'
        if not review_path.is_file():
            continue
        if review_path in excluded:
            continue
        review = json.loads(review_path.read_text())
        if review.get('status') in {'held', 'fix-needed', 'rejected', 'supplemental',
                                  'in-progress', 'prepared', 'generation-pending', 'bake-pending'}:
            continue
        item, review, approval = candidate(experiment, map_name)
        attach_states(item, review, approval, experiment, map_name)
        if any(existing['id'] == item['id'] for existing in items):
            raise ValueError('Multiple ready texture candidates for asset: ' + item['id'])
        items.append(item)
    included_reviews = {Path(item['review']).resolve() for item in items}
    if not set(excluded.values()).issubset(included_reviews):
        raise ValueError('Replacement texture review is absent from collected candidates')
    output.mkdir(parents=True, exist_ok=True)
    for item in items:
        bind_texture_decision(item, decisions)
        if item['user_approval'] == 'approved':
            item['notes'][0] = 'Texture explicitly approved for this baked revision.'
    manifest = output / 'texture-candidates.json'
    manifest.write_text(json.dumps({'map': map_name + ' texture', 'review_kind':'texture', 'items': items}, indent=2) + '\n')
    build(manifest, output / 'gallery', map_name=map_name + ' texture', pending_only=True)
    return {'gallery': str(output / 'gallery/index.html'),
            'candidates': sum(item['user_approval'] != 'approved' for item in items),
            'approved': sum(item['user_approval'] == 'approved' for item in items)}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('experiments', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--map-name', required=True)
    parser.add_argument('--additional-experiments', type=Path, action='append', default=[],
                        help='Include another immutable experiment root without relocating its evidence')
    args = parser.parse_args()
    print(json.dumps(collect(args.experiments, args.output, args.map_name, args.additional_experiments)))
