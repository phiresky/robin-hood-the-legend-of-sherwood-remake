"""Freeze the exact fourteen-pixel contact-shade appearance candidate."""
import sys,json
from pathlib import Path
HERE=Path(__file__).resolve().parent;sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from build_review_gallery import build

def main():
 d=OUT/'restart4-fence14-ground-candidate-v1';v=json.loads((d/'validation.json').read_text());assert sha(d/'model.blend')==v['model_sha256']=='4f4875bc62b5602417830eb8b458bfbe8dcc9244095699096616fdb4dfb58bf8'
 write_json(d/'root-review.json',dict(status='ready-for-user',reviewer='root',model_sha256=v['model_sha256'],validation_sha256=sha(d/'validation.json'),scope='Exact14 conservative contact-shade saved appearance; not physical wood proof or broad ground ownership',viewed={'before-after-close.png':sha(d/'before-after-close.png'),'paired-review/two-state-close.png':sha(d/'paired-review/two-state-close.png')},findings=['Root personally reviewed native/oblique before-after and both initial/applied states; scoped appearance PASS.','Approved206c remains unchanged; this private appearance derivative needs grouped user approval.'],user_appearance_approval=None))
 item=dict(id='croisement02-fence14-contact-shade',name='Fence contact — exact14 source-color fallback',status='ready-for-user',technical_eligible=True,user_approval=None,review_scope='saved14-pixel contact-shade appearance only',model=str(d/'model.blend'),solid=str(d/'before-after-close.png'),solid_label='Original camera top-left: approved206c / exact14 candidate; oblique below',textured=str(d/'paired-review/two-state-close.png'),textured_label='Initial and applied states: source contact remains static in both',source_comparison=str(OUT/'restart4-fence14-role-audit-v1/source-versus-underlay.png'),source_comparison_label='Original source / marked14 centers / previous generated floor',source_comparison_secondary=str(d/'paired-review/source-versus-underlay.png'),source_comparison_secondary_label='Exact source colors restored on current receiver',validation=str(d/'validation.json'),review=str(d/'root-review.json'),notes=[
 'Appearance decision only: restore exactly14 dark native source pixels as a conservative2D contact-shade fallback. Wood versus painted shadow remains ambiguous; this is not proof of physical wood.',
 'All2064370 outside pixels, prior known source pixels, alpha, UVs and ground geometry remain exact. No fence geometry changed and no below-ground strand was added.',
 'Both initial and applied states hit ground at all14 centers. They lie outside the cleared152x152 terminal patch, so the source-context treatment remains static in both states.',
 'No synthesis ran. The approved206c ground remains unchanged; integration may select this derivative only after its own appearance approval. No broad source-role reassignment is implied.'
 ])
 write_json(d/'review-candidates.json',dict(map='Croisement02 fence source contact appearance',items=[item],without_packets=[],status_counts={'pending appearance':1}));build(d/'review-candidates.json',d/'gallery');write_json(d/'ready-for-next-batch.json',dict(status='ready-for-user',scope=item['review_scope'],model_sha256=v['model_sha256'],candidates=str(d/'review-candidates.json'),gallery=str(d/'gallery/index.html'),user_approval=None))
if __name__=='__main__':main()
