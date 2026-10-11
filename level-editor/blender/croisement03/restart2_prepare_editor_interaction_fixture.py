"""Freeze CPU-only incremental editor checks for the exact combined trio stage."""
import hashlib,json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/croisement03-refinement/restart2'
C=BASE/'combined-trio-inferred-crown-candidate-v1'
OUT=C/'editor-interaction-fixture-v3'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main():
 assert not OUT.exists();request=json.loads((C/'browser-library-v1/browser-request.json').read_text());lib=Path(request['library']);doc=json.loads(Path(request['map']).read_text());handoff=BASE/'combined-trio-full-editor-browser-v5/handoff.json';proof=json.loads(handoff.read_text())
 assert sha(Path(request['map']))==proof['stage_map_sha256']
 assets={r['id']:r for r in doc['assetSources']+doc['sceneAssets']if r['id']in proof['approved_models']}
 parts={}
 for identity,ref in assets.items():
  assert sha(lib/ref['model'])==proof['approved_models'][identity] and sha(lib/ref['descriptor'])==ref['descriptor_sha256']
  parts[identity]=[p['node']for p in json.loads((lib/ref['descriptor']).read_text())['parts']]
 insertions=[{'asset':f'croisement03-tree-{n}','x':1600+i*260}for i,n in enumerate((12,13,14))]+[{'asset':'croisement03-native-arbre06','x':2380}]
 OUT.mkdir();cfg={'library':str(lib),'map':request['map'],'map_sha256':proof['stage_map_sha256'],'catalog_sha256':sha(lib/'3d-assets/index.json'),'stage_map':doc,'assets':assets,'parts':parts,'insertions':insertions,'expected_counts':{'groups':86,'objects':127},'prior_clock_appearance_proof':{'path':str(handoff),'sha256':sha(handoff)}}
 write(OUT/'config.json',cfg)
 for source,target in [('restart2_trio_editor_interactions.js','check.js'),('restart2_trio_editor_reload.js','reload-check.js')]:
  (OUT/target).write_bytes((ROOT/'level-editor/blender/croisement03'/source).read_bytes())
 plan={'status':'CPU_PREPARED_NOT_EXECUTED','library':str(lib),'browser_request':str(C/'browser-library-v1/browser-request.json'),'globals':{'reviewViewport':'actual editor viewport','reviewThree':'same Three module','c3InteractionConfig':'config.json','scatterCaptures':'wrapper screenshot acknowledgments'},'check':'Evaluate check.js after loading exact original staged map in disposable fresh editor profile. Return has .evidence.reloadExpectation. Preserve it outside page before full reload.','reload':'Reload actual page without reseeding or injecting a document. Reopen its saved OPFS map through normal editor flow. Inject only window.c3InteractionReloadExpectation from saved evidence, then evaluate reload-check.js.','expected_globals':['c3InteractionEvidence','c3InteractionReloadExpectation'],'http':'Wrapper intercept actual catalog and five source GLBs plus all15native resources; exact hash pins from config.assets. Do not assert separate descriptor HTTP because catalog supplies metadata.','scope':'Four group/part selections and palette insertions at separate X poses; ground is retained sceneAsset with exactpins, not an extra terrain insertion. Original127parts/86groups and alloriginal refs/resources remain unchanged. Two native effects after one extra native placement. No repeat14phase loop or geometry/texture edits.','capture_names':['c3-trio-editor-selected-parts']+['c3-palette-'+x['asset']for x in insertions]+['c3-trio-editor-saved-insertions','c3-trio-editor-reloaded-insertions'],'pins':{str(OUT/n):sha(OUT/n)for n in ('config.json','check.js','reload-check.js')},'normalization':'Stored serializer deliberately omits descriptor-owned model_scene/resources; pins permit omission only in stored JSON, then require hydrated reload equality, actual catalog descriptor semantics and every fetched native resource hash.','limits':['Only disposable OPFS map may change; hardlinked library blobs stay read-only.','Final publication current-index rebase remains root-owned; this fixture exercises the frozen appearance stage catalog.','Automatic mission clock wiring remains outside scope.']}
 write(OUT/'hook-plan.json',plan);print(json.dumps({'output':str(OUT),'pins':plan['pins'],'library':str(lib)}))
if __name__=='__main__':main()
