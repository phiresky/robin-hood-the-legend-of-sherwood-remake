"""Package independently reviewed final floor appearance for a grouped decision."""
import sys,json
from pathlib import Path
from PIL import Image,ImageDraw
HERE=Path(__file__).resolve().parent;sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from build_review_gallery import build

def main():
 d=OUT/'restart4-final-floor-bake-v1';v=json.loads((d/'validation.json').read_text());r=json.loads((d/'root-review.json').read_text());assert r['status']=='ready-for-user' and r['model_sha256']==sha(d/'model.blend')==v['model_sha256']
 sheet=Image.new('RGB',(1792*2,1184),'#292929');draw=ImageDraw.Draw(sheet)
 for i,(p,label)in enumerate([(OUT/'restart4-remaining-floor-bake-v1/composite.png','Approved76bba floor atlas'),(d/'composite.png','Saved206c7c56 floor atlas')]):sheet.paste(Image.open(p).convert('RGB'),(i*1792,32));draw.text((i*1792+8,8),label,fill='white')
 sheet.save(d/'atlas-before-after.png')
 item=dict(id='croisement02-complete-flat-floor-appearance',name='Flat ground — completed approved floor reuse',status='ready-for-user',technical_eligible=True,user_approval=None,review_scope='saved flat-ground texture appearance only',model=str(d/'model.blend'),solid=str(d/'saved-review/actual8.png'),solid_label='Reopened saved floor actual materials — original camera top-left',textured=str(d/'saved-review/bank-contact-before-after.png'),textured_label='Unchanged bank: original and reverse camera before/after floor fill',source_comparison=str(d/'atlas-before-after.png'),source_comparison_label='Approved previous atlas / exact saved candidate atlas',source_comparison_secondary=str(OUT/'restart4-floor-closure-input-v1/native13-close.png'),source_comparison_secondary_label='Thirteen exact native returns retained in saved model',source_trace=str(OUT/'restart4-floor-closure-input-v1/source-reuse-contexts-0.png'),source_trace_label='Approved source/underlying-floor scope; pink foreground gaps remain separate',projection_errors=str(OUT/'restart4-bank-underlay-input-v1/full-domain-regions-0.png'),projection_errors_label='Approved inferred bank-underlay input; bank itself unchanged',validation=str(d/'validation.json'),review=str(d/'root-review.json'),notes=[
 'Saved appearance only: exact575810-pixel composite of both Batch11-approved inputs, including575797 inferred reuse pixels and13 exact native background returns. No synthesis ran.',
 'All772238 previously known pixels,1488574 outside pixels, previous approved ground changes, alpha, UVs and ground geometry remain exact. Bank geometry/materials and dynamic state overlays are unchanged.',
 'Reopened saved atlas has zero neutral-gray texels. This establishes flat-floor appearance coverage only, not complete foreground geometry, native source ownership or arbitrary-angle bank closure.',
 'Original camera is top-left. The reverse bank-edge gray floor triangle is filled; native bank appearance remains unchanged. Inferred floor is softer and carries existing inferred shadows.',
 'The487 native foreground coverage candidates remain a separate geometry/source-role audit. In particular fence95 ambiguous39 pixels are not claimed solved by inferred floor. No native wood/vegetation source artwork was reassigned to ground.'
 ])
 write_json(d/'review-candidates.json',dict(map='Croisement02 completed flat-floor appearance',items=[item],without_packets=[],status_counts={'pending saved appearance':1}));build(d/'review-candidates.json',d/'gallery');write_json(d/'ready-for-next-batch.json',dict(status='ready-for-user',scope=item['review_scope'],model_sha256=v['model_sha256'],candidates=str(d/'review-candidates.json'),gallery=str(d/'gallery/index.html'),user_approval=None))
if __name__=='__main__':main()
