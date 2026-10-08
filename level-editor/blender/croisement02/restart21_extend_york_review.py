"""Extend the frozen review with separately scoped, root-reviewed York candidates."""
import hashlib,json,subprocess,sys
from pathlib import Path
HERE=Path(__file__).resolve().parent
EDITOR=HERE.parents[1]
BATCH=EDITOR/'work/croisement02-refinement/restart3-review-batches'
YORK=EDITOR/'work/york-refinement/restart2/jamb-clearance-candidate-v1'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main():
    inputs=BATCH/'next-climbing-shed-v2-inputs';dest=BATCH/'next-climbing-shed-v2';assert not dest.exists()
    root=YORK/'root-review.json';rr=json.loads(root.read_text());assert rr['status']=='PASS_FOR_GROUPED_USER_REVIEW'
    handoff=YORK/'grouped-review-handoff.json';h=json.loads(handoff.read_text());freeze=Path(h['freeze']);f=json.loads(freeze.read_text())
    assert sha(h['model'])==h['model_sha256']==rr['model_sha256']
    for rel,digest in f['files'].items():assert sha(YORK/rel)==digest
    assert len(f['files'])==rr['frozen_files_verified']==234
    common=[Path(h['model']),root,handoff,freeze,YORK/'self-review.json',YORK/'motion-proposal.json']
    specs=[('york-portcullis-jamb-hidden-clearance','York portcullis jamb — hidden clearance','geometry','Changed inferred jamb back geometry only. Thirteen back vertices move; front vertices, source textures and UVs are retained. Prior approval does not cover this change.', [('Saved materials; original camera top-left','actual/textured-eight.png'),('Solid geometry; original camera top-left','actual/solid-eight.png'),('Native shaded view before the change','native-before/native-textured.png'),('Native shaded view after; not byte-identical shading','native-after/native-textured.png')]),('york-portcullis-inferred-45-pose-motion','York portcullis — proposed 45-pose motion','physical motion','Proposed 45-pose gate lift and contact with corrected jamb only. Frame 36 is inferred at 57 source pixels within its 57–65 uncertainty. No complete gatehouse support or runtime approval.', [('All 45 poses in original camera order','motion-review/native-45-poses.png'),('Source feature comparison and inferred frame disclosure','motion-review/source-feature-comparison.png')]+[(f'Pose {pose}: {kind}, original camera top-left',f'motion-review/contact-{pose:02d}/{kind}-eight.png') for pose in [0,22,34,36,38,44] for kind in ['textured','solid']])]
    sources=[]
    for asset,title,scope,description,labels in specs:
        images=[dict(label=label,file=str(YORK/rel),sha256=sha(YORK/rel)) for label,rel in labels]
        evidence={str(p):sha(p) for p in common+[YORK/rel for _,rel in labels]}
        identity=dict(asset_id=asset,scope=scope,source_revision=h['revision'],evidence=evidence)
        revision=hashlib.sha256(json.dumps(identity,sort_keys=True).encode()).hexdigest()
        item=dict(asset_id=asset,name=title,model=h['model'],model_sha256=h['model_sha256'],review_revision=revision,scope=description,decision='pending',evidence=evidence,displayed_images=images,notes=['Root and worker reviews passed; explicit user decision is still pending.','Native shaded before/after contains 105 changed subpixels in the sampled known region (maximum channel difference 86). UVs/source images are retained; shaded frames are not byte-identical.','Saved mesh rays retain 660/660 triangle/material owners; UV drift is at most 4.31e-6.','All 45 contact poses have zero triangle intersections, with minimum separation 0.04997 world units.','Frame 36 is unobservable in the source and explicitly inferred; the slight one-pixel settle at frame 38 is a weak feature-supported inference.','The gate rises above this isolated jamb assembly. Complete gatehouse mechanics, surrounding support, runtime integration and publication are not proven here.'])
        packet=inputs/(asset+'-bound.json');assert not packet.exists();write(packet,dict(cards=[dict(card_id=asset,title=title,asset_ids=[asset])],items=[item]))
        sources.append(dict(kind='bound-members',scope=scope,evidence=str(packet)))
    config=json.loads((inputs/'config.skeleton.json').read_text());config.pop('preparation_status');config['title']='Grouped refinement review — climbing vegetation and York';config['sources']+=sources
    config_path=inputs/'config.json';assert not config_path.exists();write(config_path,config)
    subprocess.run([sys.executable,str(HERE/'compose_review_batch.py'),str(config_path),str(dest)],check=True)
    old=json.loads((BATCH/'next-climbing-shed-v1/evidence.json').read_text());new=json.loads((dest/'evidence.json').read_text());assert new['cards'][:2]==old['cards'];assert new['card_count']==4 and new['decision_count']==5
    assert all(m['decision']=='pending' for c in new['cards'] for m in c['members'])
    assert all('tree32' not in m['asset_id'] for c in new['cards'] for m in c['members'])
    for rel,binding in new['resources'].items():assert sha(dest/rel)==binding['sha256']
    write(dest/'static-verification.json',dict(status='PASS',cards=4,exact_decisions=5,prior_cards_byte_equivalent=True,tree32_excluded=True,all_resources_hash_verified=True,york_frozen_files_verified=234,browser='Not launched; waiting for runtime-proof owner release',approvals_recorded=False))
    print(dest/'index.html')
if __name__=='__main__':main()
