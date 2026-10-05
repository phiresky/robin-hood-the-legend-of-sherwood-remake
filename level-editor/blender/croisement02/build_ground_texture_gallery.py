"""Expose the exact protected planar fill only after independent texture review."""
import json
from pathlib import Path
import sys
sys.path[:0]=[str(Path(__file__).parent),str(Path(__file__).resolve().parents[2]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from build_review_gallery import build


def main():
    experiment=OUT/'restart2-ground-completion/approved-fill-retry-v2'
    baked=experiment/'bake-v1';source=OUT/'restart2-ground38/cumulative848-v1'
    generated=experiment/'generation-short-no-mask-with-lighting-openrouter-with-auxiliary'
    review=json.loads((baked/'root-review.json').read_text())
    if review['status']!='PASS' or review['model_sha256']!=sha(baked/'model.blend') or review['actual_sheet_sha256']!=sha(baked/'actual/textured.png'):
        raise ValueError('Exact independent texture review required')
    validation=json.loads((baked/'validation.json').read_text())
    if validation['model_sha256']!=sha(baked/'model.blend') or validation['protected_pixels_changed']!=0 or not validation['geometry_uv_unchanged']:
        raise ValueError('Ground candidate preservation failed')
    approval=json.loads((experiment/'approval.json').read_text())
    if approval['status']!='approved' or approval['approved_by']!='user' or approval['model_sha256']!=sha(source/'model.blend'):
        raise ValueError('Exact receiver/input approval required')
    item=dict(id='croisement02-ground-receiver',name='Ground texture — protected native floor and inferred hidden floor',
        status='ready-for-user',technical_eligible=True,user_approval=None,
        model=str(baked/'model.blend'),solid=str(source/'actual/textured.png'),solid_label='Approved native ground before fill — eight actual views',
        textured=str(baked/'actual/textured.png'),textured_label='Proposed ground fill — eight actual saved-material views',
        source_comparison=str(experiment/'input.png'),source_comparison_label='Exact approved native source atlas',
        source_comparison_secondary=str(generated/'generated-preserved.png'),source_comparison_secondary_label='Proposed filled atlas: all known and state pixels restored exactly',
        source_trace=str(experiment/'domain-review.png'),source_trace_label='Cyan known, ochre filled floor, magenta reserved state floor, slate separate relief',
        validation=str(baked/'validation.json'),review=str(baked/'root-review.json'),ownership=str(experiment/'approval.json'),
        notes=['Texture decision only. Receiver geometry and source ownership were approved separately; this is the exact unchanged ground model with 685,385 hidden floor pixels filled.',
               '772,189 known native pixels, all 226,475 state reservation pixels (including 36,930 unknown floor pixels), and 569,880 separate relief pixels remain exact. Gray regions are deliberate holds for separate terrain or state work.',
               'The first attempt recreated fences/logs and was rejected. This candidate continues flat leaf litter, soil and grass; no standing fences or logs are newly painted.',
               'Some inferred forest-floor patches are darker and coarser than native context. Review these joins before approval.',
               'OpenRouter received no edit mask. The approved mask was enforced locally during compositing and verified after the saved model reopened.'])
    index=experiment/'texture-review-candidates.json';write_json(index,dict(map='Croisement02 ground texture',review_kind='texture',items=[item],without_packets=[],status_counts={'pending texture review':1}))
    build(index,experiment/'texture-gallery')


if __name__=='__main__':main()
