"""Bind the cleared-state decision independently from approved covered geometry."""
import sys,json,hashlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from build_review_gallery import build

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 root=Path(sys.argv[1]).resolve();packet=root/'geometry-review';evidence=json.loads((packet/'evidence.json').read_text());review=json.loads((packet/'manual-review.json').read_text())
 assert evidence['model_sha256']==sha(root/'worker.blend')==review['model_sha256']
 assert review['all_eight_views_inspected'] and review['ready_for_geometry_review']
 for file,expected in evidence['artifacts'].items():assert sha(root/file)==expected, file
 for mode in ('solid','textured'):assert sha(packet/f'{mode}.png')==review[mode+'_sha256']
 topology=json.loads((root/'reopened-topology.json').read_text());assert len(topology)==2 and all(r['nonmanifold_edges']==r['zero_area_faces']==0 for r in topology)
 item=dict(id=evidence['state_id'],name='South field wattle fence — cleared mission state',status='ready-for-user',technical_eligible=True,
  model=str(root/'worker.blend'),solid=str(packet/'solid.png'),textured=str(packet/'textured.png'),stored_material_textured=str(packet/'textured.png'),
  context=str(root/'source-comparison.png'),source_comparison=str(root/'comparison.png'),source_comparison_label='Covered endpoint reference above; new scoped cleared geometry below',
  validation=str(root/'validation.json'),ownership=str(packet/'evidence.json'),review=str(packet/'manual-review.json'),stored_material_audit=str(root/'reopened-topology.json'),
  notes=['Decision concerns only new cleared fence geometry; existing covered geometry approval remains separate.',
         'Exactly one native transition frame: transition and applied fence geometry coincide.',
         'Both long fence remainders remain. Only source patch subsection across parts19/20 is removed.',
         'Terminal cleared-ground artwork still needs actual terrain receiver integration; ground is excluded from this geometry decision.',
         'Displayed texture is a pending texture candidate requiring independent review and approval. Geometry approval does not approve texture.'])
 index=root/'state-review-candidates.json';index.write_text(json.dumps({'map':'Croisement02 cleared fence state','items':[item]},indent=2)+'\n');build(index,root/'gallery',map_name='Croisement02 cleared fence state')
 print(root/'gallery/index.html')
if __name__=='__main__':main()
