"""Freeze the reviewed initial cart scope without claiming state completeness."""
import json,hashlib
from pathlib import Path
from catalog import OUT


def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def write(p,value):
    assert not p.exists(),p
    p.write_text(json.dumps(value,indent=2)+'\n')


def main():
    root=OUT/'restart3-north-cart';w=root/'front-closure-v4';model=sha(w/'worker.blend');assert model=='b28b4334c66c8dc0d819aaeee811a3b2805bb7703bbf93729b238d21fcb2cb15'
    support=json.loads((w/'current-support-v1/report.json').read_text());native=json.loads((w/'native-review-v1/report.json').read_text())
    assert support['status']=='PASS' and support['model_sha256']==model
    assert native['model_sha256']==model and native['canopy_connected_to_running_gear']
    wheels=[r for r in support['objects'] if r['object'].startswith('Wheel rim')];assert len(wheels)==4 and all(r['contacts'] for r in wheels)
    limits=['Only the shown initial cabin and running gear are included; no whole north-cart or state completion claim.',
            'Fore-platform, drawbar, harness and under-wheel source regions remain unassigned by this refinement.',
            'Horse actors, approach, wheel motion, breakup and broken terminal state remain separate.',
            'Hidden canopy depth, screen/cloth material, four-wheel construction and rear appearance include disclosed inference; gray hidden texture awaits exact geometry approval.',
            'Historical manual source domain is preserved, not asserted to prove complete semantic ownership of its dark residual pixels.']
    files=[w/p for p in ['worker.blend','manifest.json','source.png','source-domain.png','source-preservation.json','actual.png','solid.png','closure-preservation.json','current-support-v1/report.json','native-review-v1/comparison.png','native-review-v1/report.json','derived-recipe.json']]
    files.extend([root/'post-clearance-v3/wheel-clearance.json',root/'roof-fit-v1/fit.json',root/'roof-fit-v1/survey-fit.png',OUT/'state-target-evidence/north-cart/manifest.json',Path(json.loads((w/'manifest.json').read_text())['source_frame']['image'])])
    own=dict(status='PASS bounded initial cabin/running-gear geometry',model_sha256=model,
             inspected=['Actual all eight views','Solid all eight views','Original-native/baseline/current comparison','Current evaluated ground/bank contact and component connection graph','Native roof-edge survey and exact source preservation'],
             findings=['Surveyed roof edges remove broad excess canopy margins.','Native opaque brown front has finite supported closure without the former horizontal gap.','Post/rim intersection corrected from7.99989 to0 cubic units; remaining wheel geometry unchanged.','All four wheels contact approved current receivers; canopy and screen connect to bed/frame.'],limits=limits,
             evidence={str(p):sha(p) for p in files})
    write(w/'self-review.json',own);files.append(w/'self-review.json')
    review=dict(status='PASS',reviewer='Codex /root',model_sha256=model,scope='Shown initial cabin and running gear; surveyed canopy and physical front closure',
                inspected=['actual eight views','solid eight views','native source comparison'],
                condition='Final saved-model contact audit must pass current evaluated bank/ground; now satisfied',
                condition_evidence={str(w/'current-support-v1/report.json'):sha(w/'current-support-v1/report.json'),str(w/'native-review-v1/report.json'):sha(w/'native-review-v1/report.json')},limitations=limits,user_approved=False)
    write(w/'root-review.json',review);files.append(w/'root-review.json')
    card=dict(version=1,id='croisement02-north-cart-initial-cabin-running-gear-v1',name='Croisement02 north cart — initial cabin and running gear',asset_ids=['croisement02-north-cart-initial-physical'],status='Root scoped geometry PASS; exact user geometry approval pending',scope=review['scope'],candidate_model_sha256=model,root_review=review,files={str(p):sha(p) for p in files},presentation=dict(first_view='Original game camera at35degrees; top-left',main_actual=str(w/'actual.png'),main_solid=str(w/'solid.png'),source_comparison=str(w/'native-review-v1/comparison.png'),roof_survey=str(root/'roof-fit-v1/survey-fit.png')),disclosures=limits,no_api_before_exact_geometry_approval=True,batch_constraint='Following review pool; do not mutate frozen batches')
    card['review_revision']=hashlib.sha256(json.dumps(card,sort_keys=True,separators=(',',':')).encode()).hexdigest();write(w/'ready-candidate-v1.json',card)
    for path,digest in card['files'].items():assert sha(Path(path))==digest
    print(json.dumps(dict(card=str(w/'ready-candidate-v1.json'),sha256=sha(w/'ready-candidate-v1.json'),revision=card['review_revision'])))

if __name__=='__main__':main()
