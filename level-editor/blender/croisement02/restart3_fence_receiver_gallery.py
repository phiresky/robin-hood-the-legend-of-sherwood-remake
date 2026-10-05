"""Freeze the applied fence ground receiver for a combined state review batch."""
import json
from pathlib import Path
import sys
sys.path[:0] = [str(Path(__file__).parent),str(Path(__file__).resolve().parents[2]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha, write_json
from build_review_gallery import build


def main():
    folder = OUT/'restart3-fence-receiver/terminal-v3'
    model = sha(folder/'model.blend')
    validation = json.loads((folder/'validation.json').read_text())
    review = json.loads((folder/'root-review.json').read_text())
    if validation['model_sha256'] != model or validation['outside_changed'] != 0 or not validation['geometry_uv_unchanged']:
        raise ValueError('Exact preservation proof required')
    if review['model_sha256'] != model or review['status'] != 'ready-for-user':
        raise ValueError('Exact root readiness required')
    item = dict(id='croisement02-cleared-fence-ground-receiver',
        name='Cleared fence — applied-state ground artwork',status='ready-for-user',
        technical_eligible=True,user_approval=None,model=str(folder/'model.blend'),
        textured=str(folder/'actual8.png'),textured_label='Applied ground and approved cleared fence — original game camera first',
        source_comparison=str(folder/'base-applied-source.png'),source_comparison_label='Base ground / applied terminal artwork',
        source_comparison_secondary=str(folder/'terminal-source.png'),source_comparison_secondary_label='Complete retained terminal background source',
        projection_errors=str(folder/'view-1.png'),projection_errors_label='Oblique physical ground and remaining fence context',
        validation=str(folder/'validation.json'),review=str(folder/'root-review.json'),
        notes=[
            'The complete native 152×152 terminal background is applied to the existing real ground receiver only while the fence is cleared. The original base-state ground stays archived unchanged.',
            'All 23,104 source pixels are retained exactly. Every atlas pixel outside the rectangle, along with ground geometry, UVs and transform, is unchanged after reopening.',
            'The approved cleared-fence geometry is included for physical context. Its new cut-end textures remain pending; this card does not approve a generated fence fill.',
            'This is a separate applied-state receiver candidate, not a permanent ground source reassignment. Runtime/state integration is a separate step.',
        ])
    index = folder/'review-candidates.json'
    write_json(index,dict(map='Croisement02 cleared-fence receiver',items=[item],without_packets=[],status_counts={'pending state receiver review':1}))
    build(index,folder/'gallery')


if __name__ == '__main__':
    main()
