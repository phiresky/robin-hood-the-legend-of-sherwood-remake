"""Freeze the bounded initial fence geometry card after exact independent review."""
import sys,json
from pathlib import Path
from PIL import Image,ImageDraw
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from build_review_gallery import build

def main():
 d=OUT/'restart3-initial-fence/geometry-v6';model=sha(d/'model.blend');root=json.loads((d/'root-review.json').read_text());assert root['status']=='ready-for-user' and root['model_sha256']==model
 validation=json.loads((d/'validation.json').read_text());contact=json.loads((d/'contact-v1/validation.json').read_text());cuts=json.loads((d/'cut-plane-guard.json').read_text());packed=json.loads((d/'packed-appearance-guard.json').read_text());close=json.loads((d/'close8-v1/receipt.json').read_text());applied=json.loads((d/'applied-survivor-compatibility.json').read_text())
 assert all(v['model_sha256']==model for v in [validation,contact,cuts,packed,close]);assert cuts['status']=='PASS' and applied['status'].startswith('PASS');assert contact['ground_RGBA_exact']
 write_json(d/'review-validation.json',dict(status='PASS',model_sha256=model,geometry=validation,contact=contact,cut_boundaries=cuts,packed_appearance=packed,close_views=close,applied_survivors=applied))
 panel=Image.new('RGB',(1408,544),'#303030');draw=ImageDraw.Draw(panel)
 for i,title in enumerate(['Front oblique: unchanged approved ground','Reverse contact: unchanged approved ground']):
  panel.paste(Image.open(d/f'contact-v1/oblique-{i}.png').convert('RGB'),(704*i,32));draw.text((704*i+8,10),title,fill='white')
 panel.save(d/'both-ground-contacts.png')
 item=dict(id='croisement02-south-field-wattle-fence',name='South field fence — bounded initial-state geometry correction',status='ready-for-user',technical_eligible=True,user_approval=None,review_scope='initial fence geometry only',model=str(d/'model.blend'),solid=str(d/'close8-v1/solid8.png'),solid_label='Changed section: eight close views — original game camera first',textured=str(d/'close8-v1/actual8.png'),textured_label='Changed section actual materials — original game camera first',source_comparison=str(d/'contact-v1/source-baseline-candidate.png'),source_comparison_label='Native source / approved initial / candidate on unchanged ground',source_comparison_secondary=str(d/'both-ground-contacts.png'),source_comparison_secondary_label='Both physical ground contacts; underlay remains gray',source_trace=str(d/'contact-v1/solid8.png'),source_trace_label='Complete fence: eight solid views',projection_errors=str(d/'actual8.png'),projection_errors_label='Complete fence: eight textured views',validation=str(d/'review-validation.json'),review=str(d/'root-review.json'),notes=[
 'Initial-state geometry only: wider source-matched posts, the observed leaning post, and taller individual weave rows. Open gaps remain; no opaque panel was added.',
 'The 22 boundary-crossing members and the approved cleared-state caps remain unchanged. Surviving geometry, UVs and material assignments match the previous model exactly.',
 'Original material images and UVs are preserved. Changed front surfaces use exact native artwork; rear appearance remains provisional on the new geometry.',
 'Gray ground beneath the fence is unchanged and has a separate pending fill proposal. Neighboring foliage reservations remain separate. This card does not approve ground fill or new textures.',
 'All sheets begin with the original game camera. Close views show the changed section; the complete fence and both ground contacts are included below.'

 ])
 index=d/'review-candidates.json';write_json(index,dict(map='Croisement02 initial fence correction',items=[item],without_packets=[],status_counts={'pending geometry':1}));build(index,d/'gallery')
if __name__=='__main__':main()
