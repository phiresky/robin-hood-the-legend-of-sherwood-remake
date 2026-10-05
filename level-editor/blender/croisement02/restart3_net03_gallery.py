"""Freeze independently reviewed net03 endpoint cards for a later grouped batch."""
import json,sys
from pathlib import Path
sys.path[:0]=[str(Path(__file__).parent),str(Path(__file__).resolve().parents[2]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from build_review_gallery import build


def main():
 base=OUT/'restart3-net03/endpoints-v4';items=[]
 for suffix,label in [('e','Empty'),('i','Occupied')]:
  p=base/suffix;root=json.loads((p/'root-review.json').read_text());model=sha(p/'model.blend');audit=json.loads((p/'reopened-audit.json').read_text());context=json.loads((p/'context-budget-v1/validation.json').read_text())
  if root['status']!='ready-for-user' or root['model_sha256']!=model:raise ValueError('Exact root review missing')
  if audit['model_sha256']!=model or not audit['native_rgba_unchanged'] or audit['bag_wood_overlap_volume']>.001:raise ValueError('Exact guards failed')
  if audit['source_air_blocked'] or context['final_changed_pixels'] or not context['all_imported_geometry_uv_world_exact']:raise ValueError('Air or neighbor context guard failed')
  write_json(p/'review-validation.json',dict(status='PASS',model_sha256=model,source_audit=audit,context_validation=context,source_classification=json.loads((p/'source-classification.json').read_text())))
  items.append(dict(id='croisement02-net-piege03-'+suffix+'-endpoint',name='Northeast net trap — '+label.lower()+' endpoint and inferred support',status='ready-for-user',technical_eligible=True,user_approval=None,model=str(p/'model.blend'),review_scope='geometry only',solid=str(p/'solid8.png'),solid_label='Closed endpoint solids — original game camera first',textured=str(p/'actual8.png'),textured_label='Exact native artwork and gray unknown surfaces — original game camera first',source_comparison=str(p/'source-native.png'),source_comparison_label='Original endpoint artwork / source-camera model',source_comparison_secondary=str(p/'context-budget-v1'/f"native-{context['final_budget']}.png"),source_comparison_secondary_label='Current tree context — original camera',projection_errors=str(p/'context-budget-v1/wood-only.png'),projection_errors_label='Attachment diagnostic — canopy hidden to show unchanged tree wood and separate support',source_trace=str(p/'context-budget-v1/contact-1.png'),source_trace_label='All four current neighboring trees — converged oblique context',validation=str(p/'review-validation.json'),ownership=str(p/'manifest.json'),review=str(p/'root-review.json'),notes=[
   'Geometry review only: final phase 0 of this net endpoint. Initial rigging, animation, captured actors, effects and runtime integration remain separate.',
   'The bag follows the connected native silhouette, with larger observed strap openings kept as real air. Its unseen depth and local deformation around the wooden counterweight are conservative inferences.',
   'Existing tree39 geometry is unchanged. A separate, small branch extension supplies continuous physical support under its canopy; this new hidden support is explicitly inferred rather than observed in the sprite.',
   'All admitted native RGBA pixels are exact. Unobserved backs, rope extensions and support remain gray; no generated texture or appearance approval is requested.',
   'Source tracing has approximately two native pixels of uncertainty. The source-coverage report distinguishes unresolved boundary pixels from a detached native fragment with unassigned physical role.',
  ]))
 index=base/'review-candidates.json';write_json(index,dict(map='Croisement02 northeast net endpoints',items=items,without_packets=[],status_counts={'pending geometry review':2}));build(index,base/'gallery')
if __name__=='__main__':main()
