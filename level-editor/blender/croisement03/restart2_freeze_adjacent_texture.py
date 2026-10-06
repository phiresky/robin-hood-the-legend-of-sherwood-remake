"""Freeze independently reviewed adjacent-tree appearance for the next grouped review."""
import sys,json,hashlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from build_review_gallery import build
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main():
 tree=int(sys.argv[1]);assert tree in (12,14);b=ROOT/'level-editor/work/croisement03-refinement/restart2';src=b/f'tree{tree}-approved-wood-texture-v1/source-restored-fill-v1';out=b/f'texture-review-tree{tree}-crownfragment-v1';assert not out.exists();out.mkdir()
 model=src/'worker.blend';assert sha(model)=={12:'30e93ac1b8ae794a1c5ff38e75bd4841b3f1bff8f7bbb662eeac91679c196777',14:'935ca2ac333f4d253a9660f02098d1e63bf8aab92de4258024d7aac97353a993'}[tree]
 native=json.loads((src/'native-audit.json').read_text());assert native['accepted_bark_changes']==native['provisional_foliage_changes']==native['known_misses']==0
 notes=['APPEARANCE ONLY: generated inferred wood RGB on user-approved V14 adjacent-tree geometry. Source-part joins preserve physical triangles; no canopy change.',
 'Tree12 retains505 bark samples (500visible,5same frame0-occluded) and2975 leaf; Tree14 retains358 bark and1339 leaf. All visible native source samples exact. Original packed RGBA and prior UV layers retained. Generated RGB replaces only the original neutral wood shader branch.',
 'Single authorized repository API request used only the Leicester southeast cottage tree and Leicester moat bank tree bark examples. Local composite protected source pixels exactly.',
 'All eight actual saved model views and combined trees12/13/14 and ferns35/76 contact reviewed by author and root; original game camera is top-left.',
 'Approved13 and ferns35/76 are joint context; diagnostic floor remains unfinished terrain; no final terrain or entire neighborhood completion.',
 'Shared Arbre06 dynamic art and provisional crown-fragment membership remain separate. No animation ownership/runtime ordering approval is requested.']
 evidence=[model,src/'actual/textured.png',src/'native-comparison.png',src/'native-audit.json',src/'transfer.json',b/'tree12-14-filled-joint-v1/sheet.png',b/'tree12-14-filled-joint-v1/receipt.json',src/'self-review.json',b/f'user-approval-v14/croisement03-tree-{tree}-shared-crown-fragment.json']
 write(out/'visual-review.json',dict(status='SCOPED APPEARANCE self and root PASS; user pending',model_sha256=sha(model),native_view_index=0,evidence={str(p):sha(p) for p in evidence},root_review='Root personally reviewed actual8/native source guard and joint8: coherent mottled brown-gray bark, no visible blank areas, fern contact coherent. Approved adjacent tree and ferns contextual; diagnostic floor unfinished.',notes=notes))
 item=dict(id=f'croisement03-tree-{tree}-shared-crown-fragment',name=f'North tree{tree} — inferred bark appearance',status='ready-for-user',technical_eligible=True,model=str(model),solid=str(b/f'tree{tree}-crownfragment-v{7 if tree==12 else 5}/actual/solid.png'),textured=str(src/'actual/textured.png'),stored_material_textured=str(src/'actual/textured.png'),context=str(b/'tree12-14-filled-joint-v1/sheet.png'),source_comparison=str(src/'native-comparison.png'),source_comparison_label='Native expected / saved source samples / differences;observed bark and leaves exact',validation=str(src/'native-audit.json'),ownership=str(src/'transfer.json'),review=str(out/'visual-review.json'),notes=notes)
 manifest=out/'review-candidates.json';write(manifest,dict(map='Croisement03 adjacent-tree appearance',items=[item],scope='Appearance only; geometry approved V14; no dynamic ownership or terrain completion.'));build(manifest,out/'gallery',pending_only=True)
 evidence += [manifest,out/'visual-review.json']+list((out/'gallery').rglob('*'));write(out/'freeze.json',dict(status='Frozen root-reviewed appearance candidate; user decision pending',files={str(p):sha(p) for p in evidence if p.is_file()},model_sha256=sha(model)))
 print(manifest)
if __name__=='__main__':main()
