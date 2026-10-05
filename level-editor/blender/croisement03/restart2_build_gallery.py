"""Combine exact-hash independently reviewed private Croisement03 geometry."""
import json,hashlib,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from build_review_gallery import build
OUT=ROOT/'level-editor/work/croisement03-refinement';RESTART=OUT/'restart2'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    w=RESTART/'fallen-log-v3/assets/croisement03-stream-fallen-log';joint=RESTART/'fallen-log-contact-v3';expected='a8e4d1fd808f70ee17182d302a8c690e1eeca647a80672b80cda39fa70e54d1e'
    assert sha(w/'model.blend')==expected
    limitations=['Isolated log geometry only; no user approval or texture fill yet.','Left hidden continuation is complete geometry with gray unknown texture; fill follows geometry approval.','Twenty-six authored source-boundary pixels remain uncovered, at most one native pixel deep. The source trace is authored, not a native mask.','Contact sheet uses inferred west bank and support rock seated to the riverbed; complete native surrounding terrain remains unfinished.','Water and riverbed depths, hidden log back and concealed end shapes are inferred.','The dark feature below the central trunk is treated as water shadow, not an invented branch.']
    evidence={str(p.relative_to(OUT)):sha(p) for p in [w/'model.blend',w/'validation.json',w/'inspection/actual-materials/sheet.png',w/'inspection/source-comparison/comparison.png',joint/'sheet.png',joint/'joint.blend',joint/'evidence.json']}
    review=w/'inspection/visual-review.json';review.write_text(json.dumps(dict(status='ready-for-geometry-review',model_sha256=expected,user_approved=False,evidence=evidence,self_review='Saved actual eight views, native source and seated bank/rock/water eight views inspected.',independent_review='Root independently inspected actual eight, source overlay and contact eight. Scoped log geometry PASS: continuous bent taper, plausible seated ends and retained moss/bark.',limitations=limitations),indent=2)+'\n')
    items=[]
    for name in ['bridge-v6','fern-ownership-v1']:
        items.extend(json.loads((RESTART/name/'review-candidates.json').read_text())['items'])
    ownership=[p for p in (w/'projection').glob('*/ownership.json') if p.parent.name!='input'];assert len(ownership)==1
    items.append(dict(id=w.name,name='Lower stream fallen log',status='ready-for-user',technical_eligible=True,model=str(w/'model.blend'),solid=str(w/'modified/solid.png'),textured=str(w/'modified/textured.png'),context=str(w/'modified/context.png'),stored_material_textured=str(w/'inspection/actual-materials/sheet.png'),source_comparison=str(w/'inspection/source-comparison/comparison.png'),source_comparison_label='Native artwork, saved geometry and authored source-boundary differences',source_comparison_secondary=str(joint/'sheet.png'),source_comparison_secondary_label='Actual log with provisional bank, water and bed-seated support rock',validation=str(w/'validation.json'),ownership=str(ownership[0]),review=str(review),notes=limitations))
    for item in items:
        worker=Path(item['model']).parent;r=json.loads(Path(item['review']).read_text());assert r['status']=='ready-for-geometry-review';assert sha(worker/'model.blend')==r['model_sha256'];assert json.loads(Path(item['validation']).read_text())['status']=='PASS'
        for relative,expected_hash in r['evidence'].items():
            path=OUT/relative if relative.startswith('restart2/') else worker/relative
            if sha(path)!=expected_hash:raise ValueError('Stale review evidence: '+str(path))
    manifest=RESTART/'review-candidates.json';manifest.write_text(json.dumps(dict(map='Croisement03 reviewed geometry',items=items,status_counts={'ready for geometry review':len(items),'user approved':0},scope='Four independently reviewed isolated geometry candidates. Full-map environment, receivers, texture approval and integration remain unfinished.'),indent=2)+'\n')
    build(manifest,RESTART/'gallery',pending_only=True);print(RESTART/'gallery/index.html')
if __name__=='__main__':main()
