"""Build a hash-bound review gallery; never manufacture approval or readiness."""
import json
import sys
from pathlib import Path
from collections import Counter
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT,tree_workspace,scenery_workspace,reviewed_catalog
from evidence_io import sha
from build_review_gallery import build
from catalog_schema import source_for_part


def main():
    catalog=json.loads(reviewed_catalog().read_text());items=[];missing=[]
    for group in catalog['groups']:
        if group.get('state_only'):
            missing.append(dict(id=group['id'],name=group['name'],status='state integration pending',
                reason='Classified as patch-controlled obstacle metadata. Visible mission sprites and state integration remain under review; these volumes are not permanent scenery.'))
            continue
        tree='wood_mask' in group
        stem=bool(group.get('authored_scenery') and 'native_wood_mask' in group)
        shrub=bool(group.get('authored_scenery') and 'native_foliage_mask' in group)
        foliage=tree or shrub
        workspace=OUT/('forest-v4-round-1' if tree else 'scenery-round-1')/'assets'/group['id']
        if tree:workspace=tree_workspace(group['wood_mask'])
        if not tree:workspace=scenery_workspace(group['id'])
        report_path=workspace/'inspection/refinement.json'
        if not report_path.exists():
            missing.append(dict(id=group['id'],name=group['name'],status='in progress',reason='Worker geometry/review packet is still being built.'));continue
        scope=json.loads((workspace/'workspace.json').read_text())
        if set(scope['part_ids'])!={source_for_part(p) for p in group['parts']}:
            missing.append(dict(id=group['id'],name=group['name'],status='ownership revision pending',reason='Previous worker owns a different source-part set. A fresh workspace is required; the old geometry packet is withheld.'));continue
        report=json.loads(report_path.read_text())
        if tree and report['crown'].get('geometry_version') not in ('native-leaf-clusters-v5','native-leaf-clusters-v6'):
            missing.append(dict(id=group['id'],name=group['name'],status='in progress',reason='Replacing the rejected large-shell prototype with small, full-depth leaf clusters.'));continue
        if shrub and report['crown'].get('geometry_version')!='native-shrub-leaf-volume-v2':
            missing.append(dict(id=group['id'],name=group['name'],status='in progress',reason='Current round-volume shrub geometry is not prepared.'));continue
        model=workspace/'model.blend';model_hash=sha(model)
        if report['model_sha256']!=model_hash:
            missing.append(dict(id=group['id'],name=group['name'],status='in progress',reason='Candidate is being revised; previous evidence is withheld.'));continue
        validation=json.loads((workspace/'validation.json').read_text())
        if validation['status']!='PASS':raise ValueError('Failed workspace validation: '+group['id'])
        item=dict(id=group['id'],name=group['name'],status='refinement-in-progress',technical_eligible=False,
                  solid=str(workspace/'modified/solid.png'),textured=str(workspace/'modified/textured.png'),context=str(workspace/'modified/context.png'),
                  validation=str(workspace/'validation.json'),review=str(report_path),model=str(model),notes=list(report['limitations']))
        if group['id'].endswith('east-rail-fence'):item['name']='Southeast Stone Wall Returns'
        actual=workspace/'inspection/actual-materials';evidence=actual/'evidence.json';coverage=workspace/'inspection/source-coverage/report.json'
        if evidence.exists() and json.loads(evidence.read_text())['model_sha256']==model_hash:
            item['stored_material_textured']=str(actual/'sheet.png');item['stored_material_audit']=str(evidence)
            if (foliage or stem) and coverage.exists() and json.loads(coverage.read_text()).get('model_sha256')==model_hash:
                metrics=json.loads(coverage.read_text())
                bounds=json.loads((actual/'opacity-bounds.json').read_text()) if foliage else None
                item['projection_errors']=str(coverage.parent/'difference.png');item['projection_errors_label']='Native-mask comparison: red missing, cyan extra'
                item['source_comparison']=str(coverage.parent/'render.png');item['source_comparison_label']='Saved geometry rendered from the original map camera'
                if foliage:
                    ratio=min(r['depth_width_ratio'] for r in bounds['crowns'])
                    item['notes'].append(f"Visible depth/width {ratio:.3f}; source silhouette IoU {metrics['intersection_over_union']:.3f}.")
                else:item['notes'].append(f"Authored standalone stem; source silhouette IoU {metrics['intersection_over_union']:.3f}.")
        audit_path=workspace/'inspection/saved-model-audit.json'
        audited=False
        if audit_path.exists():
            saved=json.loads(audit_path.read_text());recipe=saved['inspection_recipe']
            if sha(workspace/recipe['recipe'])!=recipe['recipe_sha256']:raise ValueError('Frozen inspection recipe changed')
            audited=saved['status']=='PASS' and saved['model_sha256']==model_hash
            if audited:item['stored_material_audit']=str(audit_path)
        current_actual=bool(evidence.exists() and json.loads(evidence.read_text())['model_sha256']==model_hash)
        technical=audited and current_actual
        if foliage:
            technical=technical and coverage.exists() and json.loads(coverage.read_text()).get('model_sha256')==model_hash
            if technical:
                values=json.loads(coverage.read_text());bounds=json.loads((actual/'opacity-bounds.json').read_text())
                technical=values['intersection_over_union']>=.95 and min(v['depth_width_ratio'] for v in bounds['crowns'])>=1.
        elif stem:
            technical=technical and coverage.exists() and json.loads(coverage.read_text()).get('model_sha256')==model_hash and json.loads(coverage.read_text())['intersection_over_union']>=.95
        elif 'state' in group['id']:technical=False
        joint_path=workspace/'inspection/joint-neighbourhood.json'
        if joint_path.exists():
            joint=json.loads(joint_path.read_text())
            joint_evidence=json.loads(Path(joint['evidence']).read_text())
            neighbours=joint_evidence.get('inputs') or joint_evidence['workers']
            if (joint['model_sha256']==model_hash
                    and sha(Path(joint['evidence']))==joint['evidence_sha256']
                    and sha(Path(joint['sheet']))==joint['sheet_sha256']
                    and all((Path(r.get('worker',r.get('path')))/'model.blend').exists()
                            and sha(Path(r.get('worker',r.get('path')))/'model.blend')==r['model_sha256']
                            for r in neighbours)):
                item['source_comparison_secondary']=joint['sheet']
                item['source_comparison_secondary_label']=joint.get('label','Source and oblique views with approved neighbouring trees; lower row hides foliage' if stem else 'Joint source and oblique review with neighbouring geometry')
        review=workspace/'inspection/visual-review.json'
        full_crown=workspace/'inspection/full-crown'
        if (full_crown/'evidence.json').exists():
            supplemental=json.loads((full_crown/'evidence.json').read_text())
            if (supplemental['model_sha256']==model_hash
                    and supplemental['original_cameras_sha256']==sha(workspace/'modified/views.json')
                    and supplemental['supplemental_cameras_sha256']==sha(full_crown/'cameras.json')
                    and supplemental['solid_sha256']==sha(full_crown/'solid.png')
                    and supplemental['textured_sha256']==sha(full_crown/'textured.png')):
                item['source_comparison_secondary']=str(full_crown/'solid.png')
                item['source_comparison_secondary_label']='Supplemental full-crown solid mesh (opaque cards; wider framing)'
                item['stored_material_states']=[dict(id='full-crown',name='Supplemental full-crown views (wider framing)',sheet=str(full_crown/'textured.png'),audit=str(full_crown/'evidence.json'))]
        if review.exists():
            reviewed=json.loads(review.read_text())
            current_review=reviewed.get('model_sha256')==model_hash
            if reviewed.get('sheet_sha256'):
                current_review=current_review and reviewed['sheet_sha256']==sha(actual/'sheet.png')
            if reviewed.get('self_review_packet'):
                packet=Path(reviewed['self_review_packet'])
                current_review=current_review and packet.exists() and sha(packet)==reviewed['self_review_packet_sha256']
            if reviewed.get('full_crown_evidence_sha256'):
                current_review=current_review and 'stored_material_states' in item and sha(full_crown/'evidence.json')==reviewed['full_crown_evidence_sha256']
            if reviewed.get('joint_neighbourhood_sha256'):
                current_review=current_review and joint_path.exists() and sha(joint_path)==reviewed['joint_neighbourhood_sha256'] and 'source_comparison_secondary' in item
            if reviewed.get('preservation_evidence'):
                preservation=Path(reviewed['preservation_evidence'])
                current_review=current_review and preservation.exists() and sha(preservation)==reviewed['preservation_evidence_sha256']
            if current_review:
                item['notes']+=reviewed.get('notes',[])
                if reviewed.get('ready_for_geometry_review') and technical:
                    item['status']='ready-for-user';item['technical_eligible']=True
        bank_receipt=workspace/'inspection/bank-candidate.json'
        if bank_receipt.exists():
            bank=json.loads(bank_receipt.read_text())
            item['source_comparison_secondary']=bank['ramp_detail']
            item['source_comparison_secondary_label']='Northeast ramp: source, side and rear; actual texture left, solid geometry right'
            item['projection_errors']=bank['source_difference']
            item['projection_errors_label']=f"Independent bank source coverage: red missed pixels ({bank['source_coverage']['missing_known_pixels']} remaining pixels)"
        ownership=workspace/'inspection/bark-ownership.json'
        if not ownership.exists():
            reports=[p for p in (workspace/'projection').glob('*/ownership.json') if p.parent.name!='input']
            if reports:ownership=max(reports,key=lambda p:p.stat().st_mtime)
        comparison=workspace/'inspection/source-comparison'
        if (comparison/'report.json').exists():
            check=json.loads((comparison/'report.json').read_text())
            if check['model_sha256']==model_hash and check['comparison_sha256']==sha(comparison/'comparison.png'):
                item['source_comparison']=str(comparison/'comparison.png');item['source_comparison_label']='Original source / exact-camera geometry / overlay'
        feedback_ownership=workspace/'inspection/feedback-source-ownership.json'
        if feedback_ownership.exists():ownership=feedback_ownership
        if ownership.exists():item['ownership']=str(ownership)
        feedback_path=OUT/'user-feedback.json'
        decisions=[r for r in json.loads(feedback_path.read_text())['records'] if r['asset_id']==group['id']] if feedback_path.exists() else []
        if decisions:
            decision=decisions[-1];item['notes'].append('User review: '+decision['exact_user_text'])
            current=decision['model_sha256']==model_hash
            correction=workspace/'inspection/feedback-revision-1.json'
            if not current and correction.exists():
                corrected=json.loads(correction.read_text())
                current=(corrected['before_model_sha256']==decision['model_sha256'] and corrected['model_sha256']==model_hash and corrected['before_geometry_sha256']==corrected['geometry_sha256'])
            if decision['decision']=='approved' and current:
                item['user_approval']='approved geometry: '+decision['exact_user_text']
            elif decision['model_sha256']==model_hash:
                item['status']='refinement-in-progress';item['technical_eligible']=False
        item['notes']=list(dict.fromkeys(item['notes']))
        items.append(item)
    missing.extend([
        dict(id='croisement02-terrain-integration',name='Terrain integration',status='pending',reason='Full-scene gap audit, foreground-domain removal and terrain texture completion remain required before publication.'),
        dict(id='croisement02-mask-only-scenery',name='Mask-only scenery',status='pending',reason='Most undergrowth and small grass sprites remain pending. Authored stems09/44, shrubs55/58/59 and the overlapping western56/61 bank have separate review candidates. Mask22 is a northern foliage fragment, not automatically a missing trunk; its ownership remains under review. Mask21 belongs to native obstacle132.'),
        dict(id='croisement02-animation-and-mission-states',name='Animation and mission states',status='pending',reason='All 15 animation sequences and 129 mission patches are preserved as source evidence. Candidates show synchronized first-frame foliage; full state/animation integration is pending.')])
    data=dict(map='Croisement02',items=items,without_packets=missing,status_counts=dict(Counter(i['status'] for i in items)),
              policy='No geometry or texture approval is implied. Only the two explicitly selected Leicester trees are reference assets.')
    path=OUT/'review-candidates.json';path.write_text(json.dumps(data,indent=2)+'\n');build(path,OUT/'gallery',pending_only=True,map_name='Croisement02')
    print(OUT/'gallery/index.html',len(items),'candidates')

if __name__=='__main__':main()
