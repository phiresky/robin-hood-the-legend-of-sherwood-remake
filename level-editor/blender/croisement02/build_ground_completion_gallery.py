"""Package separately scoped ground geometry and fill-input decisions without approval."""
import json
from pathlib import Path
import sys

sys.path[:0] = [str(Path(__file__).parent), str(Path(__file__).resolve().parents[2] / 'refinement/blender')]
from catalog import OUT
from evidence_io import sha, write_json
from build_review_gallery import build


def main():
    prep = OUT / 'restart2-ground-completion/preparation-v1'
    worker = OUT / 'restart2-ground38/cumulative848-v1'
    inventory = json.loads((prep / 'inventory.json').read_text())
    root = json.loads((worker / 'root-review.json').read_text())
    model = sha(worker / 'model.blend')
    if model != inventory['model_sha256'] or model != root['model_sha256']:
        raise ValueError('Ground model changed since review')
    for name, digest in inventory['files'].items():
        if sha(prep / name) != digest:
            raise ValueError('Ground input changed: ' + name)
    for name, key in [('actual/textured.png', 'actual_sheet_sha256'),
                      ('restored-source-context.png', 'source_comparison_sha256'),
                      ('validation.json', 'validation_sha256')]:
        if sha(worker / name) != root[key]:
            raise ValueError('Root-reviewed ground evidence changed: ' + name)
    for reference in inventory['supplementary_references']:
        if sha(Path(reference['file'])) != reference['sha256']:
            raise ValueError('Supplementary source changed')
    # All prospective request bytes are bound in a report included in the decision
    # fingerprint, including the four original-size supplementary crops.
    request = dict(status='Proposed request only; API off pending both user decisions',
        model_sha256=model, dimensions=inventory['dimensions'],
        editable_pixels=inventory['counts']['eligible_hidden_floor'],
        files={name: dict(path=str(prep / name), sha256=sha(prep / name)) for name in
               ('input.png', 'mask.png', 'solid.png', 'auxiliary-references.json', 'material-prompt.txt')},
        references=inventory['supplementary_references'],
        approval_scope='Authorize source-preserving repository texture fill on this exact proposed domain after receiver/domain approval. Generated output requires separate texture review.',
        user_approval=None, api_calls=0)
    write_json(prep / 'proposed-request.json', request)
    shared = dict(status='ready-for-user', technical_eligible=True, model=str(worker / 'model.blend'),
                  user_approval=None, validation=str(worker / 'validation.json'))
    geometry = dict(shared, id='croisement02-ground-receiver', name='Ground receiver and cumulative source ownership',
        solid=str(prep / 'solid.png'), solid_label='Native planar footprint; no out-of-map extension',
        textured=str(worker / 'actual/textured.png'), textured_label='Actual saved receiver — eight views',
        source_comparison=str(prep / 'input.png'), source_comparison_label='772,189 known native pixels; gray is unknown',
        source_comparison_secondary=str(worker / 'restored-source-context.png'), source_comparison_secondary_label='Latest 65-pixel ground/shadow restoration, preserving prior 783',
        projection_errors=str(prep / 'domain-review.png'), projection_errors_label='Cyan: known; ochre: proposed hidden floor; magenta: state-reserved floor; slate: separate relief',
        review=str(worker / 'root-review.json'), ownership=str(worker / 'source-domain-manifest.json'),
        notes=['Decision 1 approves the unchanged flat receiver geometry and cumulative 848-pixel source ownership only. It does not authorize texture generation.',
               '848 returned native pixels comprise 783 reviewed ground pixels plus 65 inferred ground/root-shadow pixels; no speculative root volume.',
               'Legend: cyan 772,189 known; ochre 685,385 proposed hidden floor; magenta 36,930 state-reserved hidden floor; slate 569,880 separately owned relief.',
               'The footprint ends at the native map rectangle. Separate bank/asset geometry and final assembled-scene contact/visibility checks remain independent.'])
    fill = dict(shared, id='croisement02-ground-fill-input', name='Proposed ground texture-fill input and authorization',
        solid=str(prep / 'solid.png'), solid_label='Exact planar lighting reference — 1792 × 1152',
        textured=str(prep / 'input.png'), textured_label='Exact API input — 1792 × 1152',
        source_comparison=str(prep / 'mask.png'), source_comparison_label='Exact API mask: transparent editable; opaque white protected',
        source_comparison_secondary=str(prep / 'domain-review.png'), source_comparison_secondary_label='Editable ochre only; cyan known, magenta state floor and slate relief protected',
        source_trace=str(prep / 'reference-review.png'), source_trace_label='All four supplementary native crops: forest floor, path, grass, shaded floor',
        ownership=str(prep / 'inventory.json'), review=str(prep / 'proposed-request.json'),
        artwork_references=[dict(id=r['role'], label=r['role'] + ' — exact original-size supplementary crop',
                                 path=r['file'], sha256=r['sha256']) for r in inventory['supplementary_references']],
        notes=['Decision 2 authorizes the repository texture API for this exact input, mask, prompt and four supplementary references, conditional on Decision 1 approval.',
               'Fill only 685,385 unknown in-map floor pixels. Preserve known RGB exactly; never fill state reservations or separate relief. Mask transparency denotes editable pixels.',
               'Generated output and its baked model require a later texture review. No API call has run. This approval does not approve a generated result, state integration or publication.'])
    index = prep / 'review-candidates.json'
    write_json(index, dict(map='Croisement02 ground preparation', items=[geometry, fill], without_packets=[], status_counts={'pending decisions': 2}))
    build(index, prep / 'gallery', map_name='Croisement02 ground preparation')
    print(prep / 'gallery/index.html')


if __name__ == '__main__':
    main()
