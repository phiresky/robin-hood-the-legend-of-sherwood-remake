"""Combine exact-hash independently reviewed private Croisement03 geometry."""
import json,hashlib,sys,math
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from build_review_gallery import build
OUT=ROOT/'level-editor/work/croisement03-refinement';RESTART=OUT/'restart2'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    if (RESTART/'approval-round1/decision.json').exists():
        raise RuntimeError('Round1 is approved and archived; create a separate next-round gallery instead of rewriting its receipts')
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
    w=RESTART/'firewood-v7/assets/croisement03-southwest-firewood-stack'
    expected='9d5f468e181805a422b85c4c4ce7ff673d23331f13ee5556798157fdf01bb237'
    assert sha(w/'model.blend')==expected
    limitations=['Isolated compact three-billet geometry only; exact concealed shapes and billet count are inferred.','Native foreground foliage45/54 remains required in the final joint scene to conceal rear and excluded source areas. Current coarse contact context does not establish that completion.','Known timber and the two pale end patches are preserved by an authored positive trace within native mask114. Twelve trace-boundary pixels remain uncovered.','Gray rear surfaces remain unknown pending user geometry approval and texture fill. No user approval or publication implied.']
    evidence={str(p.relative_to(OUT)):sha(p) for p in [w/'model.blend',w/'validation.json',w/'inspection/actual-materials/sheet.png',w/'inspection/source-comparison/comparison.png',w/'inspection/ground-contact/sheet.png',w/'inspection/ground-contact/evidence.json']}
    review=w/'inspection/visual-review.json'
    review.write_text(json.dumps(dict(status='ready-for-geometry-review',model_sha256=expected,user_approved=False,evidence=evidence,self_review='Actual eight, native comparison and coarse ground contact eight inspected.',independent_review='Root inspected exact v7 actual eight/native/contact eight. Scoped isolated geometry PASS: shorter hidden continuation proportionate, visible three-billet/pale-end arrangement maintained, no floating base.',limitations=limitations),indent=2)+'\n')
    ownership=[p for p in (w/'projection').glob('*/ownership.json') if p.parent.name!='input'];assert len(ownership)==1
    items.append(dict(id=w.name,name='Southwest compact firewood stack',status='ready-for-user',technical_eligible=True,model=str(w/'model.blend'),solid=str(w/'modified/solid.png'),textured=str(w/'modified/textured.png'),context=str(w/'modified/context.png'),stored_material_textured=str(w/'inspection/actual-materials/sheet.png'),source_comparison=str(w/'inspection/source-comparison/comparison.png'),source_comparison_label='Native source and compact three-billet geometry',source_comparison_secondary=str(w/'inspection/ground-contact/sheet.png'),source_comparison_secondary_label='Ground contact; native foreground foliage remains unfinished',validation=str(w/'validation.json'),ownership=str(ownership[0]),review=str(review),notes=limitations))
    camera_audit=[]
    for item in items:
        worker=Path(item['model']).parent
        packet=json.loads((worker/'modified/views.json').read_text())
        actual=json.loads((worker/'inspection/actual-camera-manifest.json').read_text())
        native=[0.,-math.cos(math.radians(35)),math.sin(math.radians(35))]
        for manifest in [packet,actual]:
            v=manifest['views'][0];assert v['index']==0
            matrix=v['camera_matrix_world'];direction=[matrix[i][2] for i in range(3)]
            assert max(abs(a-b) for a,b in zip(direction,native))<1e-6
        camera_audit.append(dict(asset=item['id'],model_sha256=sha(worker/'model.blend'),native_view_index=0,camera_direction=native,packet_sha256=sha(worker/'modified/views.json'),actual_camera_sha256=sha(worker/'inspection/actual-camera-manifest.json'),contact='Contact recipes use unchanged modified packet cameras, orthographic projection, ordered view0..7; wider framing only.'))
        item['notes']=['Top-left tile is the original game-art camera: orthographic, 35 degree elevation. All solid, textured, actual-material and contact sheets keep this order.']+item['notes']
        item['source_comparison_secondary_label']+='; original game view at top left'
    (RESTART/'native-camera-gallery-audit.json').write_text(json.dumps(camera_audit,indent=2)+'\n')
    for item in items:
        worker=Path(item['model']).parent;r=json.loads(Path(item['review']).read_text());assert r['status']=='ready-for-geometry-review';assert sha(worker/'model.blend')==r['model_sha256'];assert json.loads(Path(item['validation']).read_text())['status']=='PASS'
        for relative,expected_hash in r['evidence'].items():
            path=OUT/relative if relative.startswith('restart2/') else worker/relative
            if sha(path)!=expected_hash:raise ValueError('Stale review evidence: '+str(path))
    manifest=RESTART/'review-candidates.json';manifest.write_text(json.dumps(dict(map='Croisement03 reviewed geometry',items=items,status_counts={'ready for geometry review':len(items),'user approved':0},scope='Five independently reviewed isolated geometry candidates. Full-map environment, receivers, texture approval and integration remain unfinished.'),indent=2)+'\n')
    build(manifest,RESTART/'gallery',pending_only=True);print(RESTART/'gallery/index.html')
if __name__=='__main__':main()
