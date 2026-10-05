"""Reconcile scoped endpoint reviews and verified appearance clips without claiming finished states."""
import json,hashlib,argparse
from pathlib import Path
from catalog import OUT


def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--version',default='v7');args=parser.parse_args()
    source=OUT/'state-completion-checklist-v6/manifest.json';result=json.loads(source.read_text())
    result['prior_checklist_sha256']=sha(source)
    current={
      'log-trap':['restart2-state/root-endpoint-reviews-v3.json','restart2-state/log-trap-native-appearance-v1/manifest.json','restart2-state/log-trap-native-appearance-v1/browser-verification.json','restart2-state/log-trap-native-appearance-v1/self-review.json'],
      'rock-trap':['restart2-state/root-endpoint-reviews-v3.json','rock-trap-state-candidate-v14/manifest.json','rock-trap-state-candidate-v14/contact-audit.json','rock-state-shrub-joint-v7/manifest.json','restart2-state/rock-trap-native-appearance-v1/manifest.json','restart2-state/rock-trap-native-appearance-v1/browser-verification.json','restart2-state/rock-trap-native-appearance-v1/self-review.json'],
      'net-rigging':['restart2-state/root-endpoint-reviews-v3.json','restart2-state/net-attached-v1/manifest.json','restart2-state/net-attached-v1/reopened-audit.json','restart2-state/net-attached-v1/wood-contact-audit-v2.json','restart2-state/net-attached-v1/joint-wood-only-v2/manifest.json','restart2-state/net-attached-v1/self-review.json'],
      'scoped-fence-clearing':['restart2-state/root-endpoint-reviews-v3.json'],
      'north-cart':['north-cart-initial-candidate-v5/manifest.json','north-cart-initial-candidate-v5/self-review.json','north-cart-initial-candidate-v5/support-audit.json'],
    }
    for row in result['items']:
        if row['id']=='rock-trap':
            row['historical_evidence']=[e for e in row['evidence']if 'candidate-v8/'in e['path']or 'shrub-joint-v2/'in e['path']]
            row['evidence']=[e for e in row['evidence']if e not in row['historical_evidence']]
            row['private_candidate_review']='Root scoped endpoint-volume PASS for v14: two initial and five applied complete bodies. Appearance and temporal identity remain incomplete.'
            row['remaining'][0]='Finish bounded 67-pixel rim diagnosis with exact tree03/tree04/shrub62 context; resolve uncovered source samples and rear textures.'
        if row['id']=='log-trap':
            row['remaining'][0]='Endpoint geometry has root scoped PASS. Finish source-view appearance, projected texture distortion and rear/end fill, retaining saved contact checks.'
        if row['id']in ['log-trap','rock-trap']:
            row['verified_milestones']=['Root endpoint geometry review PASS; no user or complete-state approval.','Standalone planar native appearance GLB: every native phase browser-verified, exact alpha, RGB error at most one code value, terminal clamped.']
            row['motion_distinction']='Planar native appearance is a faithful visual fallback. Full rigid-body 3D correspondence remains unproven; the log late-settling proof covers only two upper logs.'
        if row['id']=='net-rigging':
            row['verified_milestones']=['All 10 target/script/body-alternative associations reconciled.','Root scoped geometry PASS for occupied piege01 final phase 0, including corrected reverse camera and native context.']
            row['remaining']=['Finish 109 missing native samples, three gray first hits and unknown rear appearance for the approved partial geometry.','Build remaining empty/initial/family03 endpoints and native appearance phases without inventing captured actor identities.','Bind all mission-scoped activations, leaf effects, placement and visible playback to the final state export.']
        if row['id']=='scoped-fence-clearing':
            row['verified_milestones']=['Root scoped cleared-gap geometry PASS; surviving runs and capped ends reviewed.']
            row['remaining'][0]='Retain scoped root geometry decision and exact outside-domain guards through state integration.'
        if row['id']=='mission-signposts':
            row['verified_milestones']=['Five source instances and native placements bound.','Reusable 97-channel standard STEP GLB and all 32 phases plus wrap verified in browser.']
            row['remaining']=['Finish independent review and exact current physical full-scene foreground/canopy composition.','Integrate mission-only visibility, original script identities and reusable playback into the editor/library state contract.']
        for name in current.get(row['id'],[]):
            path=OUT/name;assert path.is_file(),path
            row['evidence']=[e for e in row['evidence']if e['path']!=name]
            row['evidence'].append(dict(path=name,sha256=sha(path)))
        assert row['status']=='incomplete'
    result['status']='Finite scoped state checklist '+args.version+'; reviewed endpoint geometry and native appearance export milestones are distinct from completed state integration'
    dest=OUT/'restart2-state'/('state-completion-checklist-'+args.version);dest.mkdir(exist_ok=False)
    (dest/'manifest.json').write_text(json.dumps(result,indent=2)+'\n')
    print([(r['id'],len(r.get('verified_milestones',[])),len(r['remaining']))for r in result['items']])


if __name__=='__main__':main()
