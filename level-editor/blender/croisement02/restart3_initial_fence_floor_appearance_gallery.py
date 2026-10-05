"""Freeze the reviewed initial floor appearance for a later grouped user batch."""
import sys,json
from pathlib import Path
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from build_review_gallery import build

def main():
 e=OUT/'restart3-initial-fence/floor-fill-v1';d=e/'bake-v1';p=OUT/'restart3-initial-fence/floor-proposal-v2';review=json.loads((d/'root-review.json').read_text());validation=json.loads((d/'validation.json').read_text())
 assert review['status']=='ready-for-user' and review['model_sha256']==sha(d/'model.blend')==validation['model_sha256']
 contacts=[Image.open(d/f'contact-{i}.png').convert('RGB')for i in range(2)];sheet=Image.new('RGB',(contacts[0].width*2,contacts[0].height));sheet.paste(contacts[0],(0,0));sheet.paste(contacts[1],(contacts[0].width,0));sheet.save(d/'both-contacts.png')
 combined=dict(validation);combined['contact_validation']=json.loads((d/'contact-validation.json').read_text());combined['root_review_sha256']=sha(d/'root-review.json');combined['source_input_user_approval_sha256']=sha(p/'user-input-approval.json');combined['scope']='Initial inferred floor appearance only';write_json(d/'review-validation.json',combined)
 item=dict(id='croisement02-initial-fence-floor-appearance',name='Initial fence floor — inferred grass and soil',status='ready-for-user',technical_eligible=True,user_approval=None,review_scope='texture appearance only',model=str(d/'model.blend'),solid=str(d/'native-before-after.png'),solid_label='Original game camera: previous floor / inferred floor',textured=str(d/'actual8.png'),textured_label='Ground with approved fence — original camera first',source_comparison=str(e/'generation-close-review.png'),source_comparison_label='Approved input / raw synthesis / protected composite',source_comparison_secondary=str(d/'both-contacts.png'),source_comparison_secondary_label='Both oblique ground contacts',source_trace=str(p/'inputs-v1/references.png'),source_trace_label='Four exact native ground references',projection_errors=str(p/'source-role-proposal.png'),projection_errors_label='Approved inferred floor domain',validation=str(d/'review-validation.json'),review=str(d/'root-review.json'),notes=[
 'Appearance only: 5,419 inferred initial-state floor pixels now continue nearby grass, soil and diffuse shadow beneath the fence. No fence artwork was transferred to the floor.',
 'All 772,189 known pixels and every pixel outside the approved floor region remain exact. The separate approved 14-pixel trap correction is retained. Ground geometry, UVs and alpha are unchanged.',
 'The fill is softer than the sharp original ground art. The gray foliage reservation at the right remains protected and belongs to separate work; this card does not claim that gap is complete.',
 'The approved cleared-fence terminal artwork remains applied-only and unchanged. The shown fence geometry is separately approved; its material approval is not part of this card.',
 'OpenRouter received the ordinary region guide and four native references without an edit mask. The authoritative mask was enforced locally; the untouched raw response remains archived.'
 ])
 index=d/'review-candidates.json';write_json(index,dict(map='Croisement02 initial fence floor appearance',items=[item],without_packets=[],status_counts={'pending appearance':1}));build(index,d/'gallery')
 page=d/'gallery/index.html';page.write_text(page.read_text().replace('Croisement02 initial fence floor appearance model review','Croisement02 initial fence floor appearance review').replace('Geometry candidates, not generated textures. Gray means no accepted original texture.','Scoped inferred floor appearance. Original camera is first; unrelated gray reservations remain protected.'))
 write_json(d/'ready-for-next-batch.json',dict(status='ready-for-user',scope='appearance only',model_sha256=sha(d/'model.blend'),candidates=str(index),gallery=str(d/'gallery/index.html'),evidence=str(d/'gallery/evidence.json'),user_approval=None))
if __name__=='__main__':main()
