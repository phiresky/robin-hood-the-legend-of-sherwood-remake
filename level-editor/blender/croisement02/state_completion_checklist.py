"""Bind the finite remaining state deliverables without treating source files as models."""
import argparse
import json
from pathlib import Path
from catalog import OUT
from native_log_foreground_reference import sha


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output',default='state-completion-checklist-v1');args=parser.parse_args()
    root=OUT/'state-target-evidence'; dest=OUT/args.output;dest.mkdir(exist_ok=True);assert not (dest/'manifest.json').exists()
    visual=json.loads((root/'visual-completeness.json').read_text())
    rows=[]
    def add(identifier,scope,evidence,remaining):
        bindings=[]
        for name in evidence:
            p=OUT/name;assert p.is_file(),p;bindings.append(dict(path=name,sha256=sha(p)))
        rows.append(dict(id=identifier,scope=scope,status='incomplete',evidence=bindings,remaining=remaining))
    add('log-trap','Mission target wood, independent metadata swap and shadow receiver', ['log-trap-state-candidate-v14/manifest.json','log-trap-state-candidate-v14/dense-contact-audit.json','log-state-appearance-proof-v5/manifest.json','state-target-evidence/log-trap/manifest.json'],['Approve covered/applied complete bodies and source-camera state presentation.','Establish source-supported moving body correspondence for all30unique phases through tick87; preserve native25Hz and separate script/metadata clocks.','Bind reviewed per-frame bank/ground shadow receivers and background endpoint.','Implement supported isolated export/runtime contract only after review; validate actual visible motion.'])
    add('rock-trap','Mission target boulders, metadata swap and shadow receiver',['rock-trap-state-candidate-v8/manifest.json','rock-trap-state-candidate-v8/contact-audit.json','rock-state-shrub-joint-v2/manifest.json','state-target-evidence/rock-trap/manifest.json'],['Resolve exposed inferred rim against exact source context and approve endpoints.','Establish moving boulder correspondence, breakage/reveal semantics and complete native timing through terminaltick102.','Bind bank/ground shadows; validate actual state and motion export.'])
    for identifier in ['south-cart','north-cart']:
        add(identifier,'Mission-specific cart/wheels/debris plus distinct horse actor source; never permanent all-mission scenery',[f'state-target-evidence/{identifier}/manifest.json','state-target-evidence/visual-completeness.json'],['Partition source cart, wheels, harness and living horse-team pixels with explicit ownership.','Build and review covered/applied scenery solids; preserve actor representation separately.','Preserve sequential target starts, mobile paths, sprite offsets, sounds and frozen endpoints.','Build actual visible motion and validate mission-scoped activation/background changes.'])
    add('net-rigging','10mission target instances, two profiles',['state-target-evidence/manifest.json'],['Inventory each mission instance and script activation.','Model visible rigging/net geometry and all changed poses; retain masks and source timings.','Review and validate mission-scoped placement and animation.'])
    add('arrow-interaction-markers','45mission-specific Bow Target instances; interaction markers, not furniture',['state-target-evidence/manifest.json','state-target-evidence/visual-completeness.json'],['Retain animated marker actions and trigger semantics in mission presentation.','Verify each instance remains addressable by script and activates only in its mission.','Do not fabricate45static archery props.'])
    add('mission-signposts','Five visible Panneau instances in S03_FoB_MP only',['state-target-evidence/manifest.json','state-target-evidence/visual-completeness.json'],['Build/review sign geometry and rotating poses from native artwork.','Place five distinct mission instances; preserve per-action timing and mission visibility.'])
    add('ground-and-shadow-states','All native and mission patch visibility/background transitions',['state-ground-receivers-v2/receiver-transitions.json','ground-texture-preparation/state-source-preservation.json'],['Apply approved frame domains to exact bank/ground receivers; metadata is not visible geometry.','Preserve whole fence terminal patch and integrate_in_background semantics.','Verify all3353mission frame records plus9native patch records remain bound after state export.'])
    add('scoped-fence-clearing','Source-scoped south-field fence subsection; static approval separate from new state geometry',['fence-state-candidate-v2/validation.json'],['Obtain independent covered/cleared state geometry decision.','Integrate exact terminal ground patch and native transition with unchanged surrounding fence geometry.','Reopen/export state and verify unchanged outside-domain geometry and source.'])
    add('map-animations','15native animation sequences; canopy motion cannot be claimed from frozen cards',['animation-references/manifest.json'],['Map every animation component to refined physical geometry and owned source RGBA.','Preserve native phase timing and static-plus-animated composition, including target overlays.','Demonstrate supported animated export and actual viewer/runtime playback; expand tree41 local proof only after review.'])
    add('gameplay-state-metadata','Four groups/eight invisible obstacle records142..149, linked to visible assemblies',['state-target-evidence/visual-completeness.json'],['Retain metadata state activation/collision/sight semantics separately from modeled targets.','Reconcile native9patches and129mission patch instances plus all visibility/background transitions.','Verify no invisible metadata group is counted as a completed visible model.'])
    optional={
        'log-trap': ['log-trap-state-candidate-v14/endpoint-orbit/self-review.json','log-late-settling-proof-v1/self-review.json'],
        'rock-trap': ['rock-trap-state-candidate-v10/self-review.json','rock-trap-state-candidate-v11/pair-interpenetration.json','rock-trap-state-candidate-v12/self-review.json','rock-trap-state-candidate-v14/manifest.json','rock-trap-state-candidate-v14/contact-audit.json','rock-trap-state-candidate-v14/pair-volume-audit.json','rock-trap-state-candidate-v14/self-review.json','rock-trap-state-candidate-v14/peer-review-missing-fences.json','state-target-evidence/rock-trap/unknown-rim-context-v5/manifest.json','state-target-evidence/rock-trap/full-motion/manifest.json','rock-motion-correspondence-v1/manifest.json'],
        'net-rigging': ['net-state-source-review-v1/manifest.json','net-state-source-review-v1/script-call-bindings.json','net-endpoint-candidate-v3/self-review.json','net-native-order-v1/manifest.json'],
        'arrow-interaction-markers': ['net-state-source-review-v1/script-call-bindings.json','net-state-source-review-v1/marker-presentation-contract.json'],
        'mission-signposts': ['state-sign-candidate/candidate-v2/validation.json','state-sign-candidate/candidate-v2/self-review.json','state-sign-candidate/placement-audit.json','state-sign-candidate/five-instances-v3/assembly.json','state-sign-candidate/phase-appearance-v1/self-review.json','state-sign-candidate/animation-export-v2/verification.json','state-sign-candidate/animation-export-v2/browser-verification.json','state-sign-candidate/animation-export-v2/self-review.json'],
        'map-animations': ['canopy-phase-receiver-inventory-v1/manifest.json'],
    }
    for row in rows:
        for name in optional.get(row['id'],[]):
            path=OUT/name
            if path.is_file():row['evidence'].append(dict(path=name,sha256=sha(path)))
        if row['id']=='arrow-interaction-markers':
            row['geometry_requirement']='No invented solid required: native animated UI bullseye/pointer, not archery furniture.'
            row['source_binding_progress']='All 45 actor indices and script self-hide calls reconcile; actual mission export/playback verification remains pending.'
        if row['id']=='net-rigging':
            row['source_binding_progress']='All 10 target/marker associations reconcile to script-selected e/i body patch alternatives plus g leaf effect; target action160 is blank. Private v3 covers only occupied piege01 final phase 0; attachments, other poses, appearance and integration remain open.'
            row['remaining'][0]='Preserve the reconciled mission target/patch identities through actual state export.'
        if row['id']=='rock-trap':
            row['private_candidate_review']='v10 rejected for folded shape; v11 has intersecting initial volumes; v12 rejected for implausible stack balance. v14 has self-reviewed two-body covered and five-body applied endpoints with contact/volume checks; peer geometry review found no new blocker, while root review remains pending. Appearance holds include 67 rim pixels, 24 covered and 53 applied missing native pixels, and unfilled backs. Bank contact is sub-unit clearance rather than exact zero contact. Full transition identity is unproven.'
        if row['id']=='map-animations':
            animations=json.loads((OUT/'animation-references/manifest.json').read_text())['animations']
            row['source_sequences']=[dict(index=a['index'],profile=a['profile'],frames=len(a['frames']),cycle_ticks=sum(f['delay']+1 for f in a['frames']),role='tree overlay requiring owned crown appearance' if 'Arbre' in a['profile'] else 'native butterfly visual effect; preserve effect presentation',source_frames=[dict(image=f['image'],sha256=sha(Path(f['image'])))for f in a['frames']]) for a in animations]
            assert sum('Arbre' in a['profile']for a in animations)==8
            assert sum('papillon' in a['profile']for a in animations)==7
    result=dict(status='finite scoped state checklist; source preservation and private proofs do not imply completion',items=rows,counts=dict(items=len(rows),assemblies=4,net_instances=10,arrow_markers=45,mission_signposts=5,map_animation_sequences=15),acceptance='Each item needs source/provenance guards, independent geometry or appearance decisions where applicable, actual visible state/motion verification, and reviewed integration. No runtime publication is authorized by this checklist.')
    (dest/'manifest.json').write_text(json.dumps(result,indent=2)+'\n');print([(r['id'],len(r['remaining']))for r in rows])

if __name__=='__main__':main()
