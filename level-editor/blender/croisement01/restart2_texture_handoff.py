"""Translate the existing explicit tree18 texture decision without inventing a new review."""
import hashlib
import json
import sys
import shutil
from pathlib import Path
ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from texture_decisions import evidence, fields
from texture_staging import validate_texture_handoff
from review_evidence import sha
R = ROOT / 'level-editor/work/croisement01-refinement/restart2'
def read(p): return json.loads(p.read_text())
def write(p, d): p.write_text(json.dumps(d, indent=2) + '\n')
def main(kind='tree18'):
    cases = dict(tree18=('approved-tree-fills-v1/croisement01-tree-18','ready-tree18-texture-v2','experiment-v3','baked-v1'), rock29=('approved-rock29-fill-v2/croisement01-small-bank-stones','ready-rock29-texture-v1','experiment','baked-v1-luminance'))
    cases.update(tree20=('approved-tree-fills-v1/croisement01-tree-20','ready-tree20-texture-v1','experiment-v2-dark-bark','baked-v4-dark-bark'), stump68=('approved-stump68-wood-fill-v1/croisement01-southeast-small-stump','ready-stump68-wood-texture-v1','experiment','baked-v1-luminance'))
    cases.update(tree21=('approved-tree21-fill-v1/croisement01-tree-21','ready-tree21-texture-v1','experiment','baked-v1-luminance'), stump65=('approved-stump65-wood-fill-v1/croisement01-southwest-cut-stump','ready-stump65-wood-texture-v1','experiment','baked-v1-luminance'))
    folder, ready_name, experiment, bake = cases[kind]
    case = R / folder
    approval = read(case / 'user-texture-decision.json')
    ready = R / ready_name
    assert sha(ready / 'archive.json') == approval['gallery_archive_sha256']
    for name, digest in read(ready / 'archive.json')['files'].items():
        assert sha(ready / name) == digest, name
    e, b = case / experiment, case / bake
    actual = b / 'actual-review-v1'
    assert sha(b / 'worker.blend') == sha(actual / 'model.blend') == approval['model_sha256']
    root_review = read(actual / 'inspection/root-review.json')
    assert root_review['status'] == 'scoped texture appearance PASS'
    for name, digest in root_review['files'].items(): assert sha(actual / name) == digest
    out = case / ('texture-handoff-v2' if kind == 'rock29' else 'texture-handoff-v1')
    out.mkdir(exist_ok=False)
    validation = b / 'validation.json'
    if kind == 'rock29':
        from texture_transport import validate_crop
        original = read(validation)
        raw = e / 'generation-short-no-mask-with-lighting-openrouter-with-auxiliary/generated-raw.png'
        crop = raw.parent / 'bake-reconciliation-content.png'
        old_crop = Path(original['reconciliation_reference'])
        assert sha(old_crop) == original['reconciliation_reference_sha256']
        if not crop.exists(): shutil.copy2(old_crop, crop)
        assert sha(crop) == sha(old_crop)
        validate_crop(raw, crop, raw.parent / 'generated-preserved.png', read(e / 'views.json')['transport_padding'])
        derived = out / 'bake-evidence'
        derived.mkdir()
        shutil.copy2(b / 'worker.blend', derived / 'worker.blend')
        assert sha(derived / 'worker.blend') == approval['model_sha256']
        translated = dict(original, reconciliation_reference=str(crop),
            evidence_location_translation=dict(original_validation=str(validation), original_validation_sha256=sha(validation),
                original_reference=str(old_crop), reason='Exact unchanged crop copied alongside its generation to satisfy transport provenance location guard. Approved model, pixels, and original validation remain unchanged.'))
        validation = derived / 'validation.json'
        write(validation, translated)
    review = out / 'review.json'
    write(review, dict(status='ready-for-user', all_eight_actual_views_inspected=True,
        baked_model_sha256=approval['model_sha256'], actual_sheet_sha256=sha(actual / 'inspection/actual-materials/sheet.png'),
        original_user_decision=approval, original_user_decision_sha256=sha(case / 'user-texture-decision.json'),
        root_review=root_review, translation='Schema translation of existing explicit user approval. The original gallery displays labeled derivatives; raw source and generated files below supply technical handoff evidence, not a new user review. Model and displayed actual pixels are unchanged.'))
    gen = e / 'generation-short-no-mask-with-lighting-openrouter-with-auxiliary'
    record = dict(id=approval['asset_id'], solid=str(e/'solid.png'), textured=str(actual/'inspection/actual-materials/sheet.png'),
        source_comparison=str(e/'input.png'), source_comparison_secondary=str(gen/'generated-preserved.png'),
        source_trace=str(gen/'generated-raw.png'), validation=str(validation), review=str(review))
    paths, hashes = evidence(record)
    images, reports = fields(record)
    binding = dict(images={k:hashes[k] for k in images}, reports={k:hashes[k] for k in reports})
    decision = dict(asset_id=approval['asset_id'], scope='texture', decision='approved', exact_user_text=approval['exact_user_text'],
        review_revision=hashlib.sha256(json.dumps(binding, sort_keys=True).encode()).hexdigest(),
        evidence_paths={k:str(v) for k,v in paths.items()}, evidence_sha256=hashes,
        original_gallery_decision=approval, translation='Existing approval, unchanged model and image content, technical evidence schema translation only.')
    write(out/'decisions.json', dict(version=1, decisions=[decision]))
    handoff = validate_texture_handoff(case/'review-manifest.json', approval['asset_id'], out/'decisions.json', case/'decisions.json')
    for p in [ready/'archive.json',case/'user-texture-decision.json']:
        handoff['protected_files'][str(p)] = sha(p)
    write(out/'handoff.json', handoff)
    print(out/'handoff.json')
if __name__ == '__main__': main(sys.argv[1] if len(sys.argv)>1 else 'tree18')
