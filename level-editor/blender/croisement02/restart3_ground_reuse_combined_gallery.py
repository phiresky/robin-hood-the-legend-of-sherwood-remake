"""Freeze one cumulative saved-ground appearance card with both approved reuse scopes."""
import sys,json
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from build_review_gallery import build

def main():
 d=OUT/'restart3-ground-reuse-combined-v1';state=OUT/'restart2-state/underlay-saved-context-v1';rock=OUT/'restart2-state/underlay-saved-rock-revealed-v1';r=json.loads((d/'root-review.json').read_text());h=sha(d/'model.blend');assert r['status']=='ready-for-user' and r['model_sha256']==h
 sm=json.loads((state/'manifest.json').read_text());rm=json.loads((rock/'manifest.json').read_text());assert sm['saved_model_sha256']==rm['saved_model_sha256']==h
 v=json.loads((d/'validation.json').read_text());v['state_context_manifest_sha256']=sha(state/'manifest.json');v['revealed_rock_manifest_sha256']=sha(rock/'manifest.json');v['full_resolution_views']=[dict(path=str(folder/row['file']),sha256=row['sha256'])for folder,m in [(state,sm),(rock,rm)]for row in m['records']];write_json(d/'review-validation.json',v)
 item=dict(id='croisement02-cumulative-ground-reuse-appearance',name='Cumulative ground — tree45 floor and trap/cart underlays',status='ready-for-user',technical_eligible=True,user_approval=None,review_scope='saved-model texture appearance only',model=str(d/'model.blend'),solid=str(d/'tree45-context/source-context.png'),solid_label='Original camera: source, previous floor and current tree45 context',textured=str(state/'labeled-context.png'),textured_label='Saved ground in four state contexts — original camera left, oblique right',source_comparison=str(d/'tree45-context/reverse-before-after.png'),source_comparison_label='Tree45 underlying floor: approved base / saved cumulative result',source_comparison_secondary=str(rock/'comparison.png'),source_comparison_secondary_label='Rock-trap floor revealed with vegetation hidden — diagnostic only',source_trace=str(d/'composite.png'),source_trace_label='Exact cumulative saved atlas; unrelated gray retained',projection_errors=str(d/'combined-domain.png'),projection_errors_label='Exact disjoint 10,642-pixel update domain',validation=str(d/'review-validation.json'),review=str(d/'root-review.json'),notes=[
 'Saved-model appearance only:2,441 tree45 floor pixels and8,201 trap/cart underlay pixels use the exact material reuse inputs approved in v8. No new synthesis was run.',
 'All772,189 known pixels, the approved5,419 fence-floor pixels and14 earlier trap pixels remain exact. Every pixel outside the10,642-pixel union is unchanged; ground geometry, UVs and alpha are unchanged.',
 'The fill is softer/coarser than the original ground. Remaining gray patches belong to the separate14,925-pixel completion proposal and are excluded from this model.',
 'State overlays and terminal cleared-fence artwork stay separate and unchanged. Context uses frozen approved endpoint exports and neighbors; later neighbor-only refinements are not silently substituted.',
 'The revealed rock view hides vegetation to expose the81-pixel floor update. This is a diagnostic, not a delivered vegetation state. Original camera is first in all view comparisons.'
 ])
 index=d/'review-candidates.json';write_json(index,dict(map='Croisement02 cumulative ground reuse appearance',items=[item],without_packets=[],status_counts={'pending appearance':1}));build(index,d/'gallery');page=d/'gallery/index.html';page.write_text(page.read_text().replace('Geometry candidates, not generated textures. Gray means no accepted original texture.','Saved cumulative floor appearance. Exact approved input domains only; unrelated gray remains unchanged.'));write_json(d/'ready-for-next-batch.json',dict(status='ready-for-user',scope='saved-model appearance only',model_sha256=h,candidates=str(index),gallery=str(d/'gallery/index.html'),user_approval=None))
if __name__=='__main__':main()
