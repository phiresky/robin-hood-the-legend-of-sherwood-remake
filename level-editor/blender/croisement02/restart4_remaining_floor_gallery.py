"""Freeze saved residual-floor appearance evidence after independent review."""
import sys,json
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from build_review_gallery import build

def main():
 d=OUT/'restart4-remaining-floor-bake-v1';p=OUT/'restart3-remaining-floor-input-v1';context=OUT/'restart2-state/remaining-floor-saved-context-v1';r=json.loads((d/'root-review.json').read_text());h=sha(d/'model.blend');assert r['status']=='ready-for-user' and r['model_sha256']==h
 manifest=json.loads((context/'manifest.json').read_text());assert manifest['saved_model_sha256']==h
 v=json.loads((d/'validation.json').read_text());v['context_manifest_sha256']=sha(context/'manifest.json');v['full_resolution_views']=[dict(path=str(context/row['file']),sha256=row['sha256'])for row in manifest['records']];v['remaining_gray_census_sha256']=sha(d/'remaining-gray-census.json');write_json(d/'review-validation.json',v)
 item=dict(id='croisement02-remaining-ground-appearance',name='Ground — reviewed residual floor and native background returns',status='ready-for-user',technical_eligible=True,user_approval=None,review_scope='saved-model appearance only',model=str(d/'model.blend'),solid=str(d/'tree45-context/source-context.png'),solid_label='Original camera: original art and saved floor/tree context',textured=str(context/'labeled-context.png'),textured_label='Saved ground in four state contexts — original camera left, oblique right',source_comparison=str(d/'tree45-context/reverse-before-after.png'),source_comparison_label='Approved868 base / saved residual-floor candidate',source_comparison_secondary=str(p/'native49-comparison.png'),source_comparison_secondary_label='Exact49 native background returns — packed RGBA independently verified',source_trace=str(p/'major-reuse-crops.png'),source_trace_label='Approved bounded source/material reuse inputs',projection_errors=str(p/'remaining-reuse-crops.png'),projection_errors_label='Remaining approved source/material crops',validation=str(d/'review-validation.json'),review=str(d/'root-review.json'),notes=[
 'Saved-model appearance only:14,876 inferred floor pixels reuse the exact existing material approved in v9;49 transient-reservation pixels restore exact native background. No new synthesis ran.',
 'Every previously known pixel and prior5,419/2,441/8,201/14-pixel update remains exact. All2,049,459 pixels outside this update, alpha, ground geometry and UVs are unchanged. Native ground domain increases from772,189 to772,238.',
 'Inferred floor remains softer/coarser than original art. The49 native pixels are brighter/sharper than nearby inferred floor because their original values are preserved.',
 'Dynamic transitioning overlays and terminal patches remain separate and unchanged. Frozen local endpoint/neighbor context is not a new whole-scene assembly.',
 'This closes the finite59-component set shown in the approved input. A read-only census still tracks5,930 gray non-relief floor pixels elsewhere; they were not included or silently filled. Separate bank/rock relief remains unchanged.'
 ])
 index=d/'review-candidates.json';write_json(index,dict(map='Croisement02 residual ground appearance',items=[item],without_packets=[],status_counts={'pending appearance':1}));build(index,d/'gallery');page=d/'gallery/index.html';page.write_text(page.read_text().replace('Geometry candidates, not generated textures. Gray means no accepted original texture.','Saved residual-floor appearance. Exact approved domain; remaining unrelated gray is tracked separately.'));write_json(d/'ready-for-next-batch.json',dict(status='ready-for-user',scope='saved-model appearance only',model_sha256=h,candidates=str(index),gallery=str(d/'gallery/index.html'),user_approval=None))
if __name__=='__main__':main()
