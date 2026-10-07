"""Freeze one representative canopy-motion decision with explicit residuals."""
from pathlib import Path
import hashlib,json,os,shutil
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement/restart14-canopy-animation';TRIAL=BASE/'tree42-motion-v5';DEST=TRIAL/'scoped-motion-card-v1'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 assert not DEST.exists();assert shutil.disk_usage(BASE).free-131072>=10*2**30
 model=TRIAL/'prototype.blend';assert sha(model)=='b569c53628404fd640c582402306ba265fead93a032fd43d1c07d62e03c6eb9b'
 images={
  'stored_material_textured':TRIAL/'review-v3/actual-eight.png',
  'solid_eight':TRIAL/'review-supplement-v1/solid-lit/solid-eight.png',
  'source_physical_fourteen':TRIAL/'native-phases-v1/all-fourteen-source-physical.png',
  'source_physical_cycle':TRIAL/'native-phases-v1/source-physical-cycle.gif',
  'source_phase0_phase7':TRIAL/'native-phases-v1/source-phase0-phase7.png',
  'phase2_alpha_tradeoff':TRIAL/'review-supplement-v1/phase2-new-regression.png',
  'fence_context':TRIAL/'fence-context-v1/comparison.png',
  'fence_source_coordinate':TRIAL/'fence-context-v1/source-coordinate.png'}
 reports={
  'saved_geometry_material_preservation':TRIAL/'report.json',
  'all_phase_actual_coverage':TRIAL/'comparison/report.json',
  'author_trial_review':TRIAL/'self-review-handoff.json',
  'fixed_camera_loop_and_solid_support':TRIAL/'review-v3/report.json',
  'native_phase_resources':TRIAL/'native-phases-v1/report.json',
  'bidirectional_neighbor_comparison':TRIAL/'review-supplement-v1/neighbor-comparison.json',
  'supplement_author_review':TRIAL/'review-supplement-v1/self-review-handoff.json',
  'fence_context_author_review':TRIAL/'fence-context-v1/self-review.json',
  'fence_context_model_camera_pins':TRIAL/'fence-context-v1/report.json',
  'frozen_source_sequence':BASE/'source-reconciliation-v1/report.json',
  'motion_field_provenance':BASE/'tree42-anchor-field-v2/report.json'}
 DEST.mkdir();(DEST/'resources').mkdir();root_review={'status':'ROOT_READY_FOR_GROUPED_REPRESENTATIVE_REVIEW','model_sha256':sha(model),'scope':'Tree42 v5 representative physical canopy motion only. No blanket canopy propagation, exact source parity or runtime/publication approval.','review_relay':['Root personally viewed v5 actual8 and all14 source/physical phases: broad motion coherent, no obvious tearing; preserve16 phase7 residual gaps and new731,880 gap atphases2/12.','Root personally viewed solid8 and phase2 crop: crown volume coherent, no obvious disconnected mass or tearing; solid support unchanged.','Root read bidirectional neighbor supplement and viewed phase7 crop: small source-edge tradeoff proportionate, not a reason for blanket tuning.','Root personally viewed all4 fence-context panels: thin-edge difference does not obviously hide fence; oblique shows clear separation. Ready to present representative Tree42v5 with honest source-alpha tradeoffs.'],'evidence':{str(p):sha(p) for p in list(images.values())+list(reports.values())}}
 rp=DEST/'root-review.json';rp.write_text(json.dumps(root_review,indent=2)+'\n');reports['root_scoped_readiness']=rp
 def freeze(entries):
  result={}
  for label,p in entries.items():
   digest=sha(p);target=DEST/'resources'/f'{digest}{p.suffix.lower()}'
   if not target.exists():os.link(p,target)
   assert sha(target)==digest;result[label]={'file':str(target.relative_to(DEST)),'sha256':digest,'original_source':str(p)}
  return result
 item={'id':'croisement02-tree-42-motion-v5','canonical_asset_id':'croisement02-tree-42','name':'Tree42 — representative physical canopy motion','model':str(model),'model_sha256':sha(model),'status':'ready-for-user','technical_eligible':True,'images':freeze(images),'reports':freeze(reports),'notes':[
 'Approve only this exact Tree42 v5 motion prototype on preserved static geometry/materials. This is not approval to propagate motion to all44 crowns, nor a runtime integration or whole-map completion decision.',
 'Fourteen source phases are represented by physical crown deformation. Wood, UVs, packed images, initial pose and loop are preserved. No billboard or texture-frame substitution.',
 'Phase7 distant source-alpha gaps improve21→16. Phases2/12 add one3.162px gap at(731,880); the preexisting phase7 solid miss(862,775) remains. Exact source-pixel parity is not claimed.',
 'Across14 phases no fixed neighbor blocks a valid Tree42 hit. Relative to v4, v5 adds37 fence-coverage pixel-phase events across21coordinates and resolves16; no new shrub coverage. Eight new events coincide with source leaf alpha,28 lie within1–1.414px, onephase7 event lies3.162px away.',
 'The farthest new fence event is a thin edge: rendered alpha88→120/255, both below128. Native/oblique fence context shows no obvious broad added hiding or physical intersection. These remain disclosed inferred-motion tradeoffs.',
 'Original camera is the top-left view in actual/solid sheets. The fence comparison places native close views left and oblique context right; the source-coordinate marker is a separate panel.'],
 'stored_material_textured_label':'Actual stored materials — all8 phase7 views; native camera top left',
 'solid_eight_label':'Solid geometry — same8 phase7 cameras; native top left',
 'source_physical_fourteen_label':'All14 original-source / physical-motion phase comparisons',
 'source_physical_cycle_label':'Original-source and physical-motion loop preview',
 'source_phase0_phase7_label':'Direct original-source phase0 and phase7 comparison',
 'phase2_alpha_tradeoff_label':'Disclosed phase2/12 alpha regression at(731,880)',
 'fence_context_label':'Fixed fence context — v4 top / v5 bottom; native left / oblique right',
 'fence_source_coordinate_label':'Separate original-source coordinate panel for the fence-edge tradeoff'}
 binding={kind:{k:v['sha256']for k,v in item[kind].items()}for kind in('images','reports')};binding['model']=sha(model);item['review_revision']=hashlib.sha256(json.dumps(binding,sort_keys=True).encode()).hexdigest()
 manifest=DEST/'manifest.json';manifest.write_text(json.dumps({'status':'Frozen pending explicit representative motion decision','items':[item]},indent=2)+'\n')
 config={'title':'Tree42 representative canopy motion','minimum_free_bytes':10*2**30,'sources':[{'kind':'scoped-gallery','evidence':str(manifest),'scope':'representative geometry and physical motion','scope_description':'Tree42 v5 only: physical canopy deformation and its disclosed source-alpha/neighbor edge tradeoffs on unchanged static geometry/materials. No approval for44-crown propagation, runtime integration, or exact source parity.','primary_image':'stored_material_textured'}]}
 (DEST/'compose-source.json').write_text(json.dumps(config,indent=2)+'\n');print(json.dumps({'manifest':str(manifest),'manifest_sha256':sha(manifest),'review_revision':item['review_revision'],'model_sha256':sha(model),'images':len(images),'reports':len(reports)},indent=2))
if __name__=='__main__':main()
