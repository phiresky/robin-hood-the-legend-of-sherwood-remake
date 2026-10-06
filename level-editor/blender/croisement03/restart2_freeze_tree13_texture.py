"""Freeze independently reviewed tree13 appearance for the next grouped review."""
import sys,json,hashlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from build_review_gallery import build
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main():
 b=ROOT/'level-editor/work/croisement03-refinement/restart2';src=b/'tree13-approved-wood-texture-v2/source-restored-fill-v1';out=b/'texture-review-tree13-local75-v1';assert not out.exists();out.mkdir()
 model=src/'worker.blend';assert sha(model)=='ef3f45a211714993959e420ff99fb3b2c709068971861ce6336c9a010f4cc553'
 native=json.loads((src/'native-audit.json').read_text());assert native['accepted_bark_changes']==native['provisional_foliage_changes']==native['known_misses']==0
 notes=['APPEARANCE ONLY: generated inferred wood RGB on user-approved tree13/local75 geometry. Three source-part joins preserve physical triangles; no canopy change.',
 'Original153 bark and3244 foliage native source samples exact. Original packed RGBA and prior UV layers retained. Generated RGB replaces only the original neutral wood shader branch.',
 'Single authorized repository API request used only the Leicester southeast cottage tree and Leicester moat bank tree bark examples. Local composite protected source pixels exactly.',
 'All eight actual saved model views and fern35 contact reviewed by author and root; original game camera is top-left.',
 'Coarse neighboring12/14 columns and diagnostic floor are context only; no final terrain or entire neighborhood completion.',
 'Shared Arbre06 dynamic art and provisional local75 membership remain separate. No animation ownership/runtime ordering approval is requested.']
 evidence=[model,src/'actual/textured.png',src/'native-comparison.png',src/'native-audit.json',src/'transfer.json',src/'joint/sheet.png',src/'joint/receipt.json',src/'self-review.json',b/'geometry-approval-v13-tree13-local75-v7/scope.json']
 write(out/'visual-review.json',dict(status='SCOPED APPEARANCE self and root PASS; user pending',model_sha256=sha(model),native_view_index=0,evidence={str(p):sha(p) for p in evidence},root_review='Root personally reviewed actual8/native source guard and joint8: coherent mottled brown-gray bark, no visible blank areas, fern contact coherent. Neighbor columns/floor only context.',notes=notes))
 item=dict(id='croisement03-tree-13-local75',name='North three-stem tree13 — inferred bark appearance',status='ready-for-user',technical_eligible=True,model=str(model),solid=str(b/'tree13-canopy-prototype-v7/actual/solid.png'),textured=str(src/'actual/textured.png'),stored_material_textured=str(src/'actual/textured.png'),context=str(src/'joint/sheet.png'),source_comparison=str(src/'native-comparison.png'),source_comparison_label='Native expected / saved source samples / differences;153 bark +3244 leaves exact',validation=str(src/'native-audit.json'),ownership=str(src/'transfer.json'),review=str(out/'visual-review.json'),notes=notes)
 manifest=out/'review-candidates.json';write(manifest,dict(map='Croisement03 tree13 appearance',items=[item],scope='Appearance only; geometry approved V13; no dynamic ownership or terrain completion.'));build(manifest,out/'gallery',pending_only=True)
 evidence += [manifest,out/'visual-review.json']+list((out/'gallery').rglob('*'));write(out/'freeze.json',dict(status='Frozen root-reviewed appearance candidate; user decision pending',files={str(p):sha(p) for p in evidence if p.is_file()},model_sha256=sha(model)))
 print(manifest)
if __name__=='__main__':main()
