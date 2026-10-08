"""CPU source guard and immutable staging recipe for approved static crown additions."""
import hashlib,json
from pathlib import Path
import numpy as np
from PIL import Image
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement/restart2';O=B/'approved-hub-textures-v1/static-pair-staging-v2'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def read(p):return json.loads(Path(p).read_text())
def main():
 assert not (O/'source-guards-and-recipes.json').exists();O.mkdir(exist_ok=True);archive=read(B/'approved-hub-v17-v23-plus-two-v1/verified-scope.json');source=B.parent/'baseline/covered.png';src=np.array(Image.open(source).convert('RGBA'));proposal=B/'trio-tree-integration-v1/static-leaf-source-proposal-v2';scope=read(proposal/'scope.json');pins={}
 def pin(p,expected=None):
  p=Path(p);digest=sha(p)
  if expected:assert digest==expected,str(p)
  pins[str(p)]=digest;return p
 for p,d in scope['source_guards'].items():pin(p,d)
 inventorypath=pin(B.parent/'baseline/masks/manifest.json');inventory=read(inventorypath);reserved=np.zeros(src.shape[:2],bool)
 for index in (35,76,107):
  row=next(r for r in inventory['masks'] if r['index']==index);patch=np.array(Image.open(pin(B.parent/'baseline/masks'/row['png'])))>0;x,y=row['box_top_left'];reserved[y:y+patch.shape[0],x:x+patch.shape[1]]|=patch
 prior13=pin(B/'tree13-canopy-context-v1/provisional75-excluding-known-bark.png');reserved[:57,1006:1132]|=np.array(Image.open(prior13))[:,:,3]>0
 rows=[]
 for n in (12,14):
  asset=f'croisement03-tree-{n}';member=archive['effective_assets'][asset];model=pin(member['model'],member['model_sha256']);folder=model.parent;receipt=read(pin(folder/'receipt.json',archive['verified_files'][str(folder/'receipt.json')]));audit=read(pin(folder/'native-audit.json',archive['verified_files'][str(folder/'native-audit.json')]));assert audit['model_sha256']==sha(model)
  for key in ['accepted_bark_changes','provisional_foliage_changes','new_static_foliage_changes','known_misses','ray_exhaustions']:assert audit[key]==0
  prior=read(pin(B/f'user-approval-v15/{asset}-shared-crown-fragment.json'));assert prior['status']=='approved' and prior['scope']=='texture';pin(prior['model'],prior['model_sha256']);assert receipt['source_guards'][prior['model']]==prior['model_sha256']
  for p,d in receipt['source_guards'].items():pin(p,d)
  barkpath=pin(B/f'tree{n}-bark-proposal-v1/proposed-bark.png');bark=np.array(Image.open(barkpath))>0
  maskpath=pin(proposal/f'tree{n}-candidate-static-foliage.png',archive['verified_files'][str(proposal/f'tree{n}-candidate-static-foliage.png')]);staticmask=np.array(Image.open(maskpath))>0
  staticpath=pin(proposal/f'tree{n}-proposed-source-rgba.png');static=np.array(Image.open(staticpath).convert('RGBA'));assert np.array_equal(static[:,:,3]>0,staticmask);assert np.array_equal(static[staticmask,:3],src[staticmask,:3]);assert not (staticmask&bark).any();assert not (staticmask&reserved).any()
  dynamicpath=pin(B/f'tree{n}-canopy-fragment-source-v1/000.png');leaf=np.array(Image.open(dynamicpath).convert('RGBA'));dynamic=np.zeros_like(src);x=946 if n==12 else 1084;dynamic[:leaf.shape[0],x:x+leaf.shape[1]]=leaf;assert not ((dynamic[:,:,3]>0)&staticmask).any()
  box=(930,0,1055,200) if n==12 else (1070,0,1190,175);left,top,right,bottom=box;expected=np.zeros_like(src);expected[bark]=src[bark];m=dynamic[:,:,3]>0;expected[m]=dynamic[m];expected[staticmask]=static[staticmask];expected=expected[top:bottom,left:right]
  samplespath=pin(folder/'native-samples.npz');samples=np.load(samplespath);assert np.array_equal(expected,samples['expected']);known=expected[:,:,3]>0;assert np.array_equal(samples['actual'][known],expected[known]);assert int(staticmask.sum())==receipt['new_static_faces']==audit['new_static_foliage_pixels'];assert int(bark.sum())==audit['accepted_bark_pixels']
  for neighbor in (10,11,12,13,14):
   path=next(Path(p) for p in scope['neighbor_exclusions'] if f'tree{neighbor}-bark-proposal' in p);assert not (staticmask&(np.array(Image.open(path))>0)).any()
  Image.fromarray(expected).save(O/f'tree{n}-expected-native.png')
  rows.append(dict(asset_id=asset,approved_model=str(model),approved_model_sha256=sha(model),prior_approved_bark_model=prior['model'],prior_approved_bark_sha256=prior['model_sha256'],bark_pixels=int(bark.sum()),static_pixels=int(staticmask.sum()),dynamic_frame0_pixels=int(m.sum()),native_known_pixels=int(known.sum()),native_box=list(box),source_rgba_exact=True,saved_expected_and_actual_known_rgba_exact=True,static_bark_and_frame0_overlap=0,reserved_fern_root_tree13_overlap=0,api_request=None,recipe=dict(mode='Reuse approved source-restored appearance; no new texture synthesis',source_model=str(model),source_scene='Croisement03 Refinement',wood_asset_group=asset,foliage_group=f'croisement03-arbre06-fragment-tree{n}-provisional',static_object=f'Tree{n} proposed static canopy leaf samples',required_distinct_foliage_roles=['Existing frame0 source/inferred foliage','New approved STATIC native sample surfaces'],native_old_foliage_faces=950 if n==12 else 434,new_static_faces=int(staticmask.sum()),export_rules=['Start from this approved static-crown v3 model, not the older tree exact-export-v2.','Preserve approved bark composite and all packed source RGB/alpha and UV layers.','Select both foliage objects explicitly; shared asset_group is not proof of shared animation ownership.','Keep new static cells separate from dynamic frame provenance; source-ownership red is independent of physical alpha.','Do not apply old native-face counts to the new static object.','Re-run saved-model native first-hit, exact packed material and geometry comparison before runtime review.','Root alone decides canonical publication; ground fill appearance is still HOLD.'])))
 report=dict(status='PASS CPU source/mask reconstruction; bounded saved-model/export work not run',rows=rows,pins=pins,hub_receipt_sha256=archive['receipt_sha256'],requirements=['No Blender or API was run.','Two models retain prior approved wood fill; do not generate redundant bark.','No new source ownership or animation membership is inferred.','Future render lane must be granted; >=6GiB memory, >=10GiB disk, two threads, shared FIFO, <=32MiB round.'])
 (O/'source-guards-and-recipes.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(dict(status=report['status'],rows=[{k:r[k] for k in ['asset_id','bark_pixels','static_pixels','dynamic_frame0_pixels','native_known_pixels']} for r in rows],pins=len(pins))))
if __name__=='__main__':main()
