"""Freeze the separately reviewed initial-fence floor input proposal, without API use."""
import sys,json
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from build_review_gallery import build

def main():
 d=OUT/'restart3-initial-fence/floor-proposal-v2';inputs=d/'inputs-v1';proposal=json.loads((d/'proposal.json').read_text());request=json.loads((inputs/'request.json').read_text());review=json.loads((d/'root-review.json').read_text());assert review['status']=='ready-for-user' and review['proposal_sha256']==sha(d/'proposal.json') and review['request_sha256']==sha(inputs/'request.json')
 ground=OUT/'restart2-ground-completion/approved-fill-retry-v2/bake-v1/model.blend';assert sha(ground)==proposal['ground_model_sha256']
 editable=np.array(Image.open(d/'inferred-hidden-floor.png').convert('L'))>0;returned=np.array(Image.open(d/'native-background-return.png').convert('L'))>0;mask=np.array(Image.open(inputs/'mask.png').convert('RGBA'));guide=np.array(Image.open(inputs/'region-guide.png').convert('RGBA'));input_rgba=np.array(Image.open(inputs/'input.png').convert('RGBA'));base=np.array(Image.open(OUT/'restart3-fence-receiver/terminal-v3/base-atlas.png').convert('RGBA'));source=np.array(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA'));known=np.array(Image.open(OUT/'restart2-ground-completion/preparation-v1/known.png').convert('L'))>0
 assert editable.sum()==5419 and returned.sum()==0 and not (editable&returned).any();assert np.array_equal(mask[:,:,3]==0,editable);assert np.array_equal(guide[~editable],input_rgba[~editable]);assert (guide[editable,:3]==[235,140,35]).all();assert np.array_equal(input_rgba[~returned],base[~returned]);assert np.array_equal(input_rgba[returned],source[returned]);assert np.array_equal(input_rgba[known],base[known]) and not(known&editable).any()
 guard=json.loads((d/'review-validation.json').read_text());assert guard['editable_inferred_floor_pixels']==5419 and guard['native_return_pixels']==0 and guard['input_entire_atlas_exact']
 item=dict(id='croisement02-initial-fence-floor-input',name='Initial fence floor — source region and fill input',status='ready-for-user',technical_eligible=True,user_approval=None,review_scope='source-region and texture-fill input approval only',model=str(ground),solid=str(inputs/'region-guide.png'),solid_label='Orange only: 5,419 proposed editable floor pixels',textured=str(inputs/'input.png'),textured_label='Proposed input: unchanged approved ground atlas',source_comparison=str(d/'source-role-proposal.png'),source_comparison_label='Native source and proposed floor domain',source_comparison_secondary=str(d/'post-edge-source-close.png'),source_comparison_secondary_label='Native post-edge context: no source reassignment',source_trace=str(inputs/'references.png'),source_trace_label='Four exact native ground references',projection_errors=str(OUT/'restart3-initial-fence/geometry-v6/contact-v1/source-baseline-candidate.png'),projection_errors_label='Separate fence geometry candidate on unchanged ground',validation=str(d/'review-validation.json'),review=str(d/'root-review.json'),notes=[
 'Approve this source region and input for a later texture-fill request, not generated appearance or new ground geometry. The fence geometry has its own separate card.',
 'The proposal fills 5,419 inferred floor pixels beneath the initial fence. There are no native-pixel returns or source ownership transfers. Original fence artwork stays on the fence; it will not be painted flat onto the floor.',
 'All 772,189 previously known ground pixels remain exact. Only the orange region is editable. Every other gray gap, neighboring foliage reservation and unrelated state region stays protected by the local mask.',
 'Four exact same-map ground crops supply grass, soil and shadow context. OpenRouter receives an ordinary region guide; the authoritative mask is enforced locally because OpenRouter drops edit masks.',
 'The approved cleared-state terminal artwork remains applied-only and unchanged. The API has not run; a generated result will need its own visual review.'
 ])
 index=d/'review-candidates.json';write_json(index,dict(map='Croisement02 initial fence floor input',items=[item],without_packets=[],status_counts={'pending source region and fill input':1}));build(index,d/'gallery')
 page=d/'gallery/index.html';text=page.read_text().replace('Croisement02 initial fence floor input model review','Croisement02 initial fence floor — input review').replace('Geometry candidates, not generated textures. Gray means no accepted original texture.','Proposed source region and fill input only. Orange identifies the editable floor; all other gray regions remain protected.');page.write_text(text)
if __name__=='__main__':main()
