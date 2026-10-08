"""Freeze paired climbing geometry and shed texture for independent review scopes."""
import hashlib,json,subprocess,sys
from pathlib import Path
HERE=Path(__file__).resolve().parent
REPO=HERE.parents[1]
OUT=REPO/'work/croisement02-refinement'
CLIMB=OUT/'restart14-hidden-archer/climbing-v21-edge'
YORK=REPO/'work/york-refinement/restart2/shed-texture-baked-v1'
BATCH=OUT/'restart3-review-batches'
INPUT=BATCH/'next-climbing-shed-v1-inputs'
DEST=BATCH/'next-climbing-shed-v1'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def write(p,data):p.write_text(json.dumps(data,indent=2)+'\n')
def main():
    assert not INPUT.exists() and not DEST.exists()
    review=CLIMB/'self-review-v1.json';self_review=json.loads(review.read_text())
    for rel,digest in self_review['evidence_sha256'].items():assert sha(CLIMB/rel)==digest
    root=CLIMB/'root-review-v1.json';assert not root.exists()
    write(root,dict(status='PASS_FOR_GROUPED_GEOMETRY_REVIEW',reviewer='root',authority='Explicit root message: personally viewed both actual/solid eight sheets, both six-panel contacts and both source comparisons; rehashed all 64 self-review evidence pins PASS.',self_review=str(review),self_review_sha256=sha(review),evidence=self_review['evidence_sha256'],scope='Both climbing05 endpoint geometries only. Inferred surfaces remain gray; unchanged rock strip is separate context. No full-composite parity or texture/runtime approval.',user_approval='PENDING'))
    handoff=YORK/'grouped-review-handoff.json';york=json.loads(handoff.read_text())
    for item in york['items']:
        for filename,digest in item['evidence'].items():assert sha(filename)==digest
    york_root=YORK/'root-review.json';yrr=json.loads(york_root.read_text());assert yrr['status']=='PASS_FOR_GROUPED_TEXTURE_REVIEW'
    assert yrr['model_sha256']==york['items'][0]['model_sha256']
    for rel,digest in yrr['images'].items():assert sha(YORK/rel)==digest
    INPUT.mkdir()
    items=[]
    for state in ['initial','applied']:
        worker=CLIMB/f'profile-05-{state}';model=worker/'model.blend';asset=f'croisement02-hidden-archer05-climbing-{state}'
        reports=[model,review,root,worker/'construction.json',worker/'preservation.json',worker/'review-v2/evidence.json',worker/'review-v2/native-coverage.json']
        images=[]
        for label,path in [('Saved materials: original camera top-left',worker/'review-v2/actual-eight.png'),('Solid geometry: original camera top-left',worker/'review-v2/solid-eight.png'),('Rock and bank contacts: original camera top-left',CLIMB/f'rock-bank-contact-v1/{state}/sheet.png'),('Native source and isolated endpoint context; not full-composite parity',CLIMB/f'rock-bank-contact-v1/{state}/source-comparison.png')]:
            images.append(dict(label=label,file=str(path),sha256=sha(path)));reports.append(path)
        evidence={str(p):sha(p) for p in reports};revision=hashlib.sha256(json.dumps(dict(asset_id=asset,scope='geometry',evidence=evidence),sort_keys=True).encode()).hexdigest()
        items.append(dict(asset_id=asset,name=f'Climbing vegetation — {state} endpoint',state=state,model=str(model),model_sha256=sha(model),review_revision=revision,scope='Geometry only for this endpoint. Gray inferred leaves/supports still need texture fill. Neither texture appearance nor runtime behavior is approved here.',decision='pending',evidence=evidence,displayed_images=images,notes=['Both endpoints share this geometry decision; inspect both before choosing.','All 7,073 native source pixels pass across the pair; original v18 faces, UVs, materials and ownership are unchanged.','The inferred off-map continuation has three unequal connected clumps; the former edge gap is closed.','Gray cap and reverse surfaces are deliberately untextured inference.','The broad diagonal gray strip belongs to the unchanged rock context, not this vegetation.','Source comparisons show endpoint plus rock/bank only; surrounding crowns are omitted, so they do not claim full-composite parity.','No user approval has been recorded.']))
    climbing=INPUT/'climbing-bound.json';write(climbing,dict(status='Frozen pending grouped user geometry review',cards=[dict(card_id='croisement02-hidden-archer05-climbing-pair',title='Croisement02 climbing vegetation — both endpoints',asset_ids=[i['asset_id'] for i in items])],items=items))
    for item in york['items']:
        item['name']='York riverside timber shed texture';item['scope']=item['scope_description'];item['evidence'][str(york_root)]=sha(york_root)
    york_packet=INPUT/'shed-bound.json';write(york_packet,york)
    config=INPUT/'config.json';write(config,dict(title='Private grouped review — climbing vegetation and York shed',sources=[dict(kind='bound-members',scope='geometry',evidence=str(climbing)),dict(kind='bound-members',scope='texture',evidence=str(york_packet))],supplementary_evidence=[dict(file=str(handoff),sha256=sha(handoff))],minimum_free_bytes=10*1024**3,max_resource_bytes=32*1024**2))
    subprocess.run([sys.executable,str(HERE/'compose_review_batch.py'),str(config),str(DEST)],check=True)
    evidence=json.loads((DEST/'evidence.json').read_text());assert evidence['card_count']==2 and evidence['decision_count']==3
    assert [len(c['members']) for c in evidence['cards']]==[2,1]
    for entry in evidence['resources'].values():assert sha(entry['source'])==entry['sha256']
    print(DEST/'index.html')
if __name__=='__main__':main()
