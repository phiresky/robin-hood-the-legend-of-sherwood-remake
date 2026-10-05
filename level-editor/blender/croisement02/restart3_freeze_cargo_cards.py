"""Freeze root-reviewed cargo/debris geometry and hash-bound review cards."""
import hashlib
import json
from pathlib import Path
from catalog import OUT


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write(path,data):
    assert not path.exists(),path
    path.write_text(json.dumps(data,indent=2)+'\n')


def main():
    root=OUT/'restart3-south-cart'
    rows=[('barrel-segmented-v2','croisement02-south-cart-terminal-segmented-cargo-v2',
           'Croisement02 south cart — segmented cargo hypothesis',
           'Scoped geometry: two adjoining tapered cargo lobes and physical central join',[
               'Segmented or paired cargo is a bounded hypothesis; the opaque source outline does not prove the object count.',
               'The dark front region is interpreted as an end cap here, but cap-versus-shadow remains uncertain.',
               'Thin persistent vertical strip remains native-only and has no assigned solid role.',
               'Gray hidden surfaces remain unknown texture pending exact geometry approval.',
               'Source silhouette has small bounded contour differences; no motion or whole-state integration is approved.']),
          ('loose-wood-traced-v3','croisement02-south-cart-terminal-loose-wood-v3',
           'Croisement02 south cart — three traced loose wood fragments',
           'Scoped geometry: three finite detached wood fragments with source-traced broken notch',[
               'Only three scoped detached wood regions are included; other particles, body, wheels, fence and shadows remain outside this card.',
               'Thickness and unseen undersides are inferred; small gray edge margins and pixel-scale rough contour remain.',
               'Native source pixels are retained; hidden appearance and whole-state integration are not approved.'])]
    for dirname,identifier,title,scope,limits in rows:
        worker=root/dirname;metadata=json.loads((worker/'manifest.json').read_text());model=worker/'worker.blend'
        assert sha(model)==metadata['model_sha256']
        support=json.loads((worker/'reopened-support-audit.json').read_text());assert support['status']=='PASS'
        assert support['model_sha256']==metadata['model_sha256']
        base=[worker/p for p in ['worker.blend','manifest.json','self-review.json','reopened-support-audit.json','actual.png','solid.png','source.png','native-comparison-v1/comparison.png','native-comparison-v1/report.json']]
        base.extend([root/'cargo-source-role-receipt-v2.json',OUT/'state-target-evidence/south-cart/manifest.json',Path(metadata['source_frame']['image'])])
        if dirname.startswith('barrel'):
            base.extend([root/'barrel-fit-v1/body-domain.png',root/'barrel-axial-profile-v1/report.json',root/'barrel-axial-profile-v1/profile.png'])
        else:base.append(worker/'source-domain.png')
        # Bind construction plus projection/support helpers without changing old evidence.
        recipes=worker/'derived-recipe-v1';recipes.mkdir(exist_ok=False)
        for name in ['restart3_cart_cargo_lobes.py','restart3_cart_cargo_debris.py','restart3_audit_cart_cargo.py','restart3_cargo_native_comparison.py']:
            source=Path(__file__).parent/name;target=recipes/name;target.write_bytes(source.read_bytes());base.append(target)
        review=dict(status='PASS',reviewer='Codex /root',model_sha256=metadata['model_sha256'],scope=scope,
                    inspected=['actual eight views','native source/solid comparison'],limitations=limits,user_approved=False,
                    guard_sha256={str(p):sha(p) for p in base if p.suffix=='.json'})
        write(worker/'root-review.json',review);base.append(worker/'root-review.json')
        presentation=dict(first_view='Original game camera at 35 degrees; top-left of eight-view sheets',
                          main_actual=str(worker/'actual.png'),main_solid=str(worker/'solid.png'),
                          source_comparison=str(worker/'native-comparison-v1/comparison.png'))
        if dirname.startswith('barrel'):presentation['source_profile']=str(root/'barrel-axial-profile-v1/profile.png')
        card=dict(version=1,id=identifier,name=title,asset_ids=[metadata['asset_id']],
                  status='Root geometry PASS; awaiting exact user geometry approval',scope=scope,
                  candidate_model_sha256=metadata['model_sha256'],root_review=review,files={str(p):sha(p) for p in base},
                  presentation=presentation,disclosures=limits,no_api_before_exact_geometry_approval=True,
                  batch_constraint='Following pool unless current batch remains unfrozen and inclusion causes no delay')
        card['review_revision']=hashlib.sha256(json.dumps(card,sort_keys=True,separators=(',',':')).encode()).hexdigest()
        write(worker/'ready-candidate-v1.json',card)
        for name,digest in card['files'].items():assert sha(Path(name))==digest
        print(json.dumps({'card':str(worker/'ready-candidate-v1.json'),'card_sha256':sha(worker/'ready-candidate-v1.json'),'revision':card['review_revision']}))


if __name__=='__main__':main()
