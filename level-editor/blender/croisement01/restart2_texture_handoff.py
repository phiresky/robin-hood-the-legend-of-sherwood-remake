"""Translate the existing explicit tree18 texture decision without inventing a new review."""
import hashlib
import json
import sys
from pathlib import Path
ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from texture_decisions import evidence, fields
from texture_staging import validate_texture_handoff
from review_evidence import sha
R = ROOT / 'level-editor/work/croisement01-refinement/restart2'
def read(p): return json.loads(p.read_text())
def write(p, d): p.write_text(json.dumps(d, indent=2) + '\n')
def main():
    case = R / 'approved-tree-fills-v1/croisement01-tree-18'
    approval = read(case / 'user-texture-decision.json')
    ready = R / 'ready-tree18-texture-v2'
    assert sha(ready / 'archive.json') == approval['gallery_archive_sha256']
    for name, digest in read(ready / 'archive.json')['files'].items():
        assert sha(ready / name) == digest, name
    e, b = case / 'experiment-v3', case / 'baked-v1'
    actual = b / 'actual-review-v1'
    assert sha(b / 'worker.blend') == sha(actual / 'model.blend') == approval['model_sha256']
    root_review = read(actual / 'inspection/root-review.json')
    assert root_review['status'] == 'scoped texture appearance PASS'
    for name, digest in root_review['files'].items(): assert sha(actual / name) == digest
    out = case / 'texture-handoff-v1'
    out.mkdir(exist_ok=False)
    review = out / 'review.json'
    write(review, dict(status='ready-for-user', all_eight_actual_views_inspected=True,
        baked_model_sha256=approval['model_sha256'], actual_sheet_sha256=sha(actual / 'inspection/actual-materials/sheet.png'),
        original_user_decision=approval, original_user_decision_sha256=sha(case / 'user-texture-decision.json'),
        root_review=root_review, translation='Schema translation of existing explicit user approval. The original gallery displays labeled derivatives; raw source and generated files below supply technical handoff evidence, not a new user review. Model and displayed actual pixels are unchanged.'))
    gen = e / 'generation-short-no-mask-with-lighting-openrouter-with-auxiliary'
    record = dict(id=approval['asset_id'], solid=str(e/'solid.png'), textured=str(actual/'inspection/actual-materials/sheet.png'),
        source_comparison=str(e/'input.png'), source_comparison_secondary=str(gen/'generated-preserved.png'),
        source_trace=str(gen/'generated-raw.png'), validation=str(b/'validation.json'), review=str(review))
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
if __name__ == '__main__': main()
