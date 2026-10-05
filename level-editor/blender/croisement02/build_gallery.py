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
from supplemental_reviews import records as supplemental_records, append_section as append_supplemental_section
from catalog_schema import source_for_part
from audit_native_mask_backlog import audit as audit_native_masks


def main():
    backlog=audit_native_masks(OUT)
    backlog_path=OUT/'understory-review/native-mask-backlog.json'
    backlog_path.write_text(json.dumps(backlog,indent=2)+'\n')
    if backlog['inputs']['ownership-revision/catalog.json']!=sha(reviewed_catalog()):raise ValueError('Catalog changed after native-mask audit')
    catalog=json.loads(reviewed_catalog().read_text());items=[];missing=[]
    for group in catalog['groups']:
        if group.get('state_only'):
            missing.append(dict(id=group['id'],name=group['name'],status='state integration pending',
                reason='Classified as patch-controlled obstacle metadata. Visible mission sprites and state integration remain under review; these volumes are not permanent scenery.'))
            continue
        tree='wood_mask' in group
        stem=bool(group.get('authored_scenery') and 'native_wood_mask' in group)
        shrub=bool(group.get('authored_scenery') and 'native_foliage_mask' in group)
        ground_plant=bool(group.get('authored_scenery') and 'native_ground_plant_mask' in group)
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
        if tree and report['crown'].get('geometry_version') not in ('native-leaf-clusters-v5','native-leaf-clusters-v6','microfragment-curved-envelope-irregular-volume-v2','microfragment-volume-paired-front-v3','branch-clump-fragments-v1','native-fragment-envelope-v1','inferred-boundary-density-v5'):
            missing.append(dict(id=group['id'],name=group['name'],status='in progress',reason='Replacing the rejected large-shell prototype with small, full-depth leaf clusters.'));continue
        shrub_version=report.get('crown',{}).get('geometry_version')
        reviewed_shrub_version=(shrub_version=='native-shrub-leaf-volume-v2' or
                                group.get('native_foliage_mask')==73 and shrub_version=='native-conifer-leaf-volume-v1')
        if shrub and group.get('native_foliage_mask')==76:
            from wattle_flower_candidates import selected_workspace as reviewed_pair_workspace
            reviewed_shrub_version=reviewed_shrub_version or reviewed_pair_workspace(OUT,group['id'],reviewed_catalog())==workspace
        if shrub and not reviewed_shrub_version:
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
            if (foliage or stem or ground_plant) and coverage.exists() and json.loads(coverage.read_text()).get('model_sha256')==model_hash:
                metrics=json.loads(coverage.read_text())
                bounds=json.loads((actual/'opacity-bounds.json').read_text()) if foliage else None
                item['projection_errors']=str(coverage.parent/'difference.png');item['projection_errors_label']='Native-mask comparison: red missing, cyan extra'
                item['source_comparison']=str(coverage.parent/'render.png');item['source_comparison_label']='Saved geometry rendered from the original map camera'
                if foliage:
                    ratio=min(r['depth_width_ratio'] for r in bounds['crowns'])
                    item['notes'].append(f"Visible depth/width {ratio:.3f}; source silhouette IoU {metrics['intersection_over_union']:.3f}.")
                else:item['notes'].append(f"Authored {'ground plant' if ground_plant else 'standalone stem'}; source silhouette IoU {metrics['intersection_over_union']:.3f}.")
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
        elif stem or ground_plant:
            technical=technical and coverage.exists() and json.loads(coverage.read_text()).get('model_sha256')==model_hash and json.loads(coverage.read_text())['intersection_over_union']>=.95
        elif 'state' in group['id']:technical=False
        if ground_plant:
            geometry=report['crown']
            technical=technical and geometry['geometry_version']=='native-rooted-ground-plants-v4' and abs(geometry['minimum_z']-geometry['ground_z']-.05)<.002
            selection=json.loads((OUT/'ground-plant-integration/selection.json').read_text())['records'][group['id']]
            joint=Path(selection['joint_review_directory'])
            item['source_comparison_secondary']=str(joint/'sheet.png')
            item['source_comparison_secondary_label']='Grouped source and oblique ground-contact review'
            item['source_trace']=str(joint/'native-scale-context.png')
            item['source_trace_label']='Exact native map scale: coarse source fragments assessed in context'
            item['notes'].extend(['Principal inferred blades/fronds are rooted; some source-pixel fragments look scattered or detached under magnification. Not every fragment is connected.',
                                  'Hidden backs reuse only this plant native palette. No AI-generated texture or observed rear artwork is claimed.'])
        joint_path=workspace/'inspection/joint-neighbourhood.json'
        if joint_path.exists():
            joint=json.loads(joint_path.read_text())
            joint_evidence=json.loads(Path(joint['evidence']).read_text())
            neighbours=joint_evidence.get('inputs') or joint_evidence['workers']
            if (joint['model_sha256']==model_hash
                    and sha(Path(joint['evidence']))==joint['evidence_sha256']
                    and sha(Path(joint['sheet']))==joint['sheet_sha256']
                    and all((Path(r.get('worker',r.get('path',r.get('workspace'))))/'model.blend').exists()
                            and sha(Path(r.get('worker',r.get('path',r.get('workspace'))))/'model.blend')==r['model_sha256']
                            for r in neighbours)):
                item['source_comparison_secondary']=joint['sheet']
                item['source_comparison_secondary_label']=joint.get('label','Source and oblique views with approved neighbouring trees; lower row hides foliage' if stem else 'Joint source and oblique review with neighbouring geometry')
                if group.get('native_foliage_mask') in (67,68,69,70,71,72,73,75,79,80,82,91,92):
                    native_context=Path(joint['sheet']).parent/'native-scale-context.png'
                    if not native_context.exists():raise ValueError('Central foliage native-scale context missing')
                    item['source_trace']=str(native_context)
                    item['source_trace_label']='Isolated plant material over native artwork; covered fronts may overpaint foreground'
                    item['notes'].append('Hidden leaf arrangement reuses only this plant native palette; no observed rear artwork or AI texture is claimed. Small native-pixel fragments remain coarse when magnified.')
                    item['notes'].append(f"Observed source leaf pixels: {report['crown']['observed_leaf_pixels']}; covered and rear arrangement is inferred.")
                    if group.get('native_foliage_mask')!=73 and not report['crown'].get('inferred_branch_support'):
                        item['notes'].append('Leaf-only shrub hypothesis: lower leaves can clear the terrain; roots and connecting stems are unobserved and are not proven by transparent mesh corners.')
                    if report['crown'].get('inferred_branch_support'):
                        item['notes'].append('Small branched supports are inferred from the leaf arrangement. Existing observed leaf geometry and UVs are unchanged; support backs use only this plant native warm palette.')
                    if group.get('native_foliage_mask')==80:item['notes'].append('West map-edge continuation is inferred from this clump own native edge artwork; off-map pixels are not observed evidence.')
                    if group.get('native_foliage_mask') in (75,91):
                        item['notes'].append('Leaf ownership follows a reviewed source partition. Other mixed wood/fence/ground receivers have separate evidence and coverage checks; the original mixed mask is not treated as all foliage or all bark.')
                        source_packet=json.loads(Path(report['source_packet']).read_text())
                        inferred_owners=source_packet.get('source_role_review',{}).get('record',{}).get('inferred_owner_boundary_pixels',0)
                        if inferred_owners:item['notes'].append(f'{inferred_owners} source pixels have explicitly inferred leaf ownership; their dark native RGB is preserved, not brightened or generated.')
                    if group.get('native_foliage_mask')==68:item['notes'].append('The shown tree07 neighbour retains an earlier floating-wood limitation. This leaf-group candidate does not claim that trunk correction.')
                    if group.get('native_foliage_mask') in (69,70,71):item['notes'].append('Shown mature tree31/32 canopy curtains and wood support remain separate corrections. Isolated native-scale overlays are context views, not full-scene parity evidence.')

        log_comparison=workspace/'inspection/approved-baseline-comparison/comparison.json'
        log_residual=workspace/'inspection/source-coverage/residual-review.json'
        if group['id']=='croisement02-southwest-log-pile' and log_comparison.exists() and log_residual.exists():
            compared=json.loads(log_comparison.read_text());residual=json.loads(log_residual.read_text())
            if compared['model_sha256']!=model_hash or residual['model_sha256']!=model_hash or compared['comparison_sha256']!=sha(log_comparison.parent/'comparison.png'):
                raise ValueError('Stale additive log comparison')
            item['source_comparison_secondary']=str(log_comparison.parent/'comparison.png')
            item['source_comparison_secondary_label']='Original source, approved baseline, additive candidate, and aligned source overlays'
            item['projection_errors']=str(log_residual.parent/'residual-review.png')
            item['projection_errors_label']='Residuals: red timber outlines; magenta fine twig region; yellow ambiguous plant/wood boundary'
            item['notes'].append('Earlier approved geometry and fill remain unchanged. New hidden wood faces are neutral pending fill; residual fine wood and plant boundaries remain documented, not 100% silhouette coverage.')
        wood_review=workspace/'inspection/independent-wood-review.json'
        if wood_review.exists():
            wood=json.loads(wood_review.read_text())
            if (wood['model_sha256']==model_hash and wood['status'].startswith('PASS')
                    and all(sha(Path(path))==expected for path,expected in wood['files'].items())):
                from wood_revision_candidates import validate_worker as validate_wood_revision
                validate_wood_revision(workspace)
                if technical:
                    item['status']='ready-for-user';item['technical_eligible']=True
                item['source_trace']=str(workspace/'inspection/lower-stem/textured.png')
                item['source_trace_label']='NEW continuous lower trunk: actual materials from eight close-up views; gray rear wood awaits texture fill'
                item['notes'].append('Lower wood058 and junction062 revised; approved crown and wood060/061 preserved. This wood revision needs new geometry approval.')
        cleanup_comparison=workspace/'inspection/baseline-comparison'
        if (cleanup_comparison/'evidence.json').exists():
            compared=json.loads((cleanup_comparison/'evidence.json').read_text())
            if (compared['model_sha256']==model_hash and
                    compared['baseline_sha256']==sha(workspace/'baseline.blend') and
                    compared['camera_manifest_sha256']==sha(workspace/'inspection/actual-camera-manifest.json') and
                    all(sha(cleanup_comparison/name)==expected for name,expected in compared['sheets'].items())):
                item['source_comparison_secondary']=str(cleanup_comparison/'comparison-0-3.png')
                preservation=json.loads((workspace/'inspection/prototype-preservation.json').read_text())
                baseline_label=('Reviewed root addition with earlier crown' if preservation.get('root_completion_base')
                                else 'Earlier approved geometry')
                item['source_comparison_secondary_label']=baseline_label+' above; NEW cleanup candidate below, identical cameras (views0–3)'
                item['source_trace']=str(cleanup_comparison/'comparison-4-7.png')
                item['source_trace_label']=baseline_label+' above; NEW cleanup candidate below, identical cameras (views4–7)'
        review=workspace/'inspection/visual-review.json'
        complete_volume=workspace/'inspection/complete-volume'
        if (complete_volume/'evidence.json').exists():
            supplemental=json.loads((complete_volume/'evidence.json').read_text())
            if (supplemental['model_sha256']!=model_hash
                    or supplemental['fixed_manifest_sha256']!=sha(workspace/'modified/views.json')
                    or supplemental['fitted_manifest_sha256']!=sha(complete_volume/'views.json')
                    or supplemental['actual_sheet_sha256']!=sha(complete_volume/'textured-sheet.png')
                    or supplemental['solid_sheet_sha256']!=sha(complete_volume/'solid-sheet.png')):
                raise ValueError('Stale complete-volume review packet')
            item.setdefault('stored_material_states',[]).extend([
                dict(id='complete-volume-actual',name='Complete inferred volume: actual materials, wider framing',sheet=str(complete_volume/'textured-sheet.png'),audit=str(complete_volume/'evidence.json')),
                dict(id='complete-volume-solid',name='Complete inferred volume: solid geometry, wider framing',sheet=str(complete_volume/'solid-sheet.png'),audit=str(complete_volume/'evidence.json'))])
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
        composed_gallery=workspace/'inspection/composed-gallery.json'
        if composed_gallery.exists():
            composed=json.loads(composed_gallery.read_text())
            native_joint=Path(composed['native_joint'])
            if (composed['model_sha256']!=model_hash
                    or not native_joint.is_file()
                    or sha(native_joint)!=composed['native_joint_sha256']):
                raise ValueError('Stale composed native joint evidence')
            item['source_trace']=str(native_joint)
            item['source_trace_label']=composed['label']
        disclosure_path=workspace/'inspection/composed-disclosure.json'
        if disclosure_path.exists():
            disclosure=json.loads(disclosure_path.read_text())
            if (disclosure.get('model_sha256')!=model_hash
                    or not isinstance(disclosure.get('notes'),list)
                    or not all(isinstance(note,str) for note in disclosure['notes'])):
                raise ValueError('Stale or invalid composed candidate disclosure')
            item['notes']+=disclosure['notes']
            item['disclosure']=str(disclosure_path)
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
        from fence_cap_candidate import geometry_review_ready
        fence_ready=geometry_review_ready(OUT,group['id'],model_hash)
        if fence_ready:
            joint=Path(fence_ready['joint'])
            item['source_trace']=str(joint/'source-comparison.png')
            item['source_trace_label']='Current tree38 / shrub75 / fence95 native source comparison'
            item['source_comparison_secondary']=str(joint/'contact-sheet.png')
            item['source_comparison_secondary_label']='Current neighboring geometry: four contact views'
            item['notes']+=fence_ready['notes']
            if technical:
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
            decision=next((r for r in reversed(decisions) if r['model_sha256']==model_hash),decisions[-1])
            current=decision['model_sha256']==model_hash
            correction=workspace/'inspection/feedback-revision-1.json'
            if not current and correction.exists():
                corrected=json.loads(correction.read_text())
                current=(corrected['before_model_sha256']==decision['model_sha256'] and corrected['model_sha256']==model_hash and corrected['before_geometry_sha256']==corrected['geometry_sha256'])
            item['notes'].append(('Current model user review: ' if current else 'Earlier model user review: ')+decision['exact_user_text'])
            if decision['decision']=='approved' and current:
                item['user_approval']='approved geometry: '+decision['exact_user_text']
            elif decision['model_sha256']==model_hash:
                item['status']='refinement-in-progress';item['technical_eligible']=False
        holds_path=OUT/'restart2-vegetation/gallery-holds.json'
        if holds_path.exists():
            for hold in json.loads(holds_path.read_text())['holds']:
                if hold['asset_id']==group['id'] and hold['model_sha256']==model_hash:
                    item['status']='refinement-in-progress';item['technical_eligible']=False
                    item['notes'].append(hold['reason'])
        item['notes']=list(dict.fromkeys(item['notes']))
        items.append(item)
    missing.extend([
        dict(id='croisement02-terrain-integration',name='Terrain integration',status='pending',reason='Full-scene gap audit, foreground-domain removal and terrain texture completion remain required before publication.'),
        dict(id='croisement02-mask-only-scenery',name='Mask-only scenery',status='pending',reason=f"Current ownership audit: {backlog['summary']['missing_foliage_domains']} native foliage source domains remain outside the {len(catalog['groups'])}-group catalog. This counts source domains, not future assets: some combine into clumps or need mixed-material splits. Every native mask is classified in understory-review/native-mask-backlog.json. Mask22 is a foliage fragment; applied-only masks138–141 remain state assets.",evidence=str(backlog_path),evidence_sha256=sha(backlog_path)),
        dict(id='croisement02-animation-and-mission-states',name='Animation and mission states',status='pending',reason='All 15 animation sequences and 129 mission patches are preserved as source evidence. Candidates show synchronized first-frame foliage; full state/animation integration is pending.')])
    data=dict(map='Croisement02',items=items,without_packets=missing,status_counts=dict(Counter(i['status'] for i in items)),
              policy='No geometry or texture approval is implied. Only the two explicitly selected Leicester trees are reference assets.')
    data['supplemental_reviews']=supplemental_records(OUT)
    path=OUT/'review-candidates.json';path.write_text(json.dumps(data,indent=2)+'\n');build(path,OUT/'gallery',pending_only=True,map_name='Croisement02')
    append_supplemental_section(OUT/'gallery',data['supplemental_reviews'])
    print(OUT/'gallery/index.html',len(items),'candidates')

if __name__=='__main__':main()
